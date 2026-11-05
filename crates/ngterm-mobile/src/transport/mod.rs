//! How the core reaches a machine: a [`Transport`] opens one [`Connection`] per machine, and a connection runs
//! commands on exec [`Channel`]s (the tree and control lines with no pty, `ngtermctl live present` per attached
//! session, `ngterm-session attach` per shown pane with a pty sized by the view).
//!
//! The traits are the seam between the actor and ssh: the russh implementation ([`russh`]) is the app's, the scripted
//! `fake` (feature `fakes`) the tests'. Both run on the actor's `LocalSet`, so neither a transport's futures nor its
//! connections need to be `Send`.
//!
//! A [`Channel`] is the same concrete pair of queues whatever the transport: the transport drives its far end, a
//! [`ChannelPeer`], and the core reads and writes the near one. Its [`ChannelReader`] can be read on the actor
//! ([`ChannelReader::read`]) or on a pane's feeder thread ([`ChannelReader::blocking_read`]), and its
//! [`ChannelWriter`] is cloned onto whichever thread writes. The output queue is bounded ([`CHANNEL_DEPTH`]): a
//! reader that falls behind (a feeder blocked in `StreamFeed::deliver`) stops the transport delivering. How far behind
//! it may fall is the transport's: russh's holds up to [`russh::OUTPUT_BUDGET`] more and then closes the channel, as
//! its connection's other channels would otherwise wait on it.

#[cfg(any(test, feature = "fakes"))]
pub mod fake;
pub mod russh;

#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use ngterm_term::GridSize;
use tokio::sync::mpsc;

use crate::api::Machine;

/// Output chunks a channel holds before its transport waits for the reader.
pub const CHANNEL_DEPTH: usize = 64;

/// Why a channel ended with no end of its own: its peer went away without saying how the command ended.
pub const PEER_GONE: &str = "the channel closed without an exit status";

/// What reaching a machine or running a command on it failed with.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransportError {
    /// The machine could not be reached (resolution, connect, timeout, a host key question left unanswered).
    #[error("{0}")]
    Unreachable(String),
    /// The machine was refused, or refused us, for a reason a retry won't fix by itself: a changed host key, an
    /// unknown one the user did not trust, a key it does not accept.
    #[error("{0}")]
    Refused(String),
    /// The connection is up but refused to open the channel.
    #[error("{0}")]
    Rejected(String),
    /// The connection was closed before or while the channel opened.
    #[error("the connection is closed")]
    Closed,
}

/// How a machine is reached: one per core, moved onto its actor.
#[expect(
    async_fn_in_trait,
    reason = "the futures run on the actor's LocalSet and need not be Send"
)]
pub trait Transport: 'static {
    type Connection: Connection;

    /// Connects to `machine`.
    async fn connect(&self, machine: &Machine) -> Result<Self::Connection, TransportError>;

    /// The app's answer about the unknown host key `fingerprint` a connect of `machine_id` is waiting on: `trust`
    /// pins it and the connect goes on, anything else ends that connect. A transport that never asks ignores it.
    fn answer_host_key(&self, machine_id: &str, fingerprint: &str, trust: bool) {
        let _ = (machine_id, fingerprint, trust);
    }
}

/// One machine's connection: every channel to the machine is opened on it.
#[expect(
    async_fn_in_trait,
    reason = "the futures run on the actor's LocalSet and need not be Send"
)]
pub trait Connection: 'static {
    /// Runs `command` (the remote shell line, as `ngterm_core::remote::ssh`'s tails build it) on a new exec channel,
    /// with a pty of `pty`'s size when given.
    async fn exec(&self, command: &str, pty: Option<GridSize>) -> Result<Channel, TransportError>;

    /// Closes the connection: every open channel ends as [`ChannelEnd::Lost`], and later [`Connection::exec`]s fail
    /// with [`TransportError::Closed`].
    fn close(&self);

    /// Resolves once the connection is gone, whether it was lost (the machine or the network went away, its
    /// keepalives went unanswered) or closed by [`Connection::close`]; the actor watches it to reconnect a lost one.
    async fn lost(&self);
}

/// What a channel carries from the machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelOutput {
    /// Bytes of the command's standard output (a pty's whole output).
    Data(Vec<u8>),
    /// Bytes of its standard error (ssh's extended data).
    Stderr(Vec<u8>),
    /// The channel ended; nothing follows.
    End(ChannelEnd),
}

/// How a channel ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelEnd {
    /// The command exited: its status, or the signal that killed it (`StreamFeed::exited`'s arguments).
    Exit { code: Option<u32>, signal: Option<String> },
    /// The link went away with no exit status (a transport error, a closed connection).
    Lost(String),
}

/// What the core sends down a channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelInput {
    /// Bytes for the command's standard input.
    Data(Vec<u8>),
    /// The pty's new size (ssh's window change).
    Resize(GridSize),
    /// The end of the command's input.
    Eof,
    /// Close the channel.
    Close,
}

/// An exec channel, as the core holds it: read and written from different places, so split from the start.
#[derive(Debug)]
pub struct Channel {
    pub reader: ChannelReader,
    pub writer: ChannelWriter,
}

impl Channel {
    /// A channel and the far end its transport drives.
    #[must_use]
    pub fn pair() -> (Channel, ChannelPeer) {
        let (output_tx, output_rx) = mpsc::channel(CHANNEL_DEPTH);
        let (input_tx, input_rx) = mpsc::unbounded_channel();
        let backlog = Backlog::default();
        let refused = Arc::new(AtomicBool::new(false));
        let channel = Channel {
            reader: ChannelReader {
                output: output_rx,
                ended: None,
            },
            writer: ChannelWriter {
                input: input_tx,
                backlog: backlog.clone(),
                refused: Arc::clone(&refused),
            },
        };
        (
            channel,
            ChannelPeer {
                output: output_tx,
                input: input_rx,
                backlog,
                refused,
            },
        )
    }
}

/// The reading half of a [`Channel`].
#[derive(Debug)]
pub struct ChannelReader {
    output: mpsc::Receiver<ChannelOutput>,
    ended: Option<ChannelEnd>,
}

impl ChannelReader {
    /// The next output. After the end, every read returns the same end again; a peer gone without one reads as
    /// [`ChannelEnd::Lost`] ([`PEER_GONE`]).
    pub async fn read(&mut self) -> ChannelOutput {
        if let Some(end) = &self.ended {
            return ChannelOutput::End(end.clone());
        }
        let output = self.output.recv().await;
        self.record(output)
    }

    /// [`ChannelReader::read`] for a thread outside the runtime (a pane's feeder).
    ///
    /// # Panics
    ///
    /// When called inside an async context (tokio refuses to block one).
    pub fn blocking_read(&mut self) -> ChannelOutput {
        if let Some(end) = &self.ended {
            return ChannelOutput::End(end.clone());
        }
        let output = self.output.blocking_recv();
        self.record(output)
    }

    fn record(&mut self, output: Option<ChannelOutput>) -> ChannelOutput {
        let output = output.unwrap_or_else(|| ChannelOutput::End(ChannelEnd::Lost(PEER_GONE.to_owned())));
        if let ChannelOutput::End(end) = &output {
            self.ended = Some(end.clone());
            self.output.close();
        }
        output
    }
}

/// The bytes of standard input a channel's writers queued that its transport has not sent yet, shared by both ends:
/// a writer that may out-produce the machine waits on it ([`ChannelWriter::backlog`]) rather than queue without bound
/// while ssh's window is shut.
#[derive(Debug, Clone, Default)]
pub struct Backlog(Arc<AtomicUsize>);

impl Backlog {
    fn add(&self, bytes: usize) {
        self.0.fetch_add(bytes, Ordering::AcqRel);
    }

    /// `bytes` queued earlier have been sent (or will never be).
    pub fn sent(&self, bytes: usize) {
        let _ = self.0.fetch_update(Ordering::AcqRel, Ordering::Acquire, |queued| {
            Some(queued.saturating_sub(bytes))
        });
    }

    /// The bytes queued and not yet sent.
    #[must_use]
    pub fn get(&self) -> usize {
        self.0.load(Ordering::Acquire)
    }
}

/// The writing half of a [`Channel`]; clones write to the same channel, in the order their sends happen.
#[derive(Debug, Clone)]
pub struct ChannelWriter {
    input: mpsc::UnboundedSender<ChannelInput>,
    backlog: Backlog,
    /// Set once the transport takes no more input (its connection is gone).
    refused: Arc<AtomicBool>,
}

impl ChannelWriter {
    /// Queues `input` for the transport; false once the channel is gone.
    #[must_use]
    pub fn send(&self, input: ChannelInput) -> bool {
        if self.refused.load(Ordering::Acquire) {
            return false;
        }
        let bytes = match &input {
            ChannelInput::Data(data) => data.len(),
            _ => 0,
        };
        self.backlog.add(bytes);
        let sent = self.input.send(input).is_ok();
        if !sent {
            self.backlog.sent(bytes);
        }
        sent
    }

    /// The bytes of standard input queued and not yet sent to the machine.
    #[must_use]
    pub fn backlog(&self) -> usize {
        self.backlog.get()
    }

    /// Queues bytes for the command's standard input.
    #[must_use]
    pub fn write(&self, bytes: &[u8]) -> bool {
        self.send(ChannelInput::Data(bytes.to_vec()))
    }

    /// Queues the pty's new size.
    #[must_use]
    pub fn resize(&self, size: GridSize) -> bool {
        self.send(ChannelInput::Resize(size))
    }

    /// Queues the end of the command's input.
    #[must_use]
    pub fn eof(&self) -> bool {
        self.send(ChannelInput::Eof)
    }

    /// Asks the transport to close the channel.
    #[must_use]
    pub fn close(&self) -> bool {
        self.send(ChannelInput::Close)
    }
}

/// The transport's end of a [`Channel`]: it delivers the machine's output and takes the core's input.
#[derive(Debug)]
pub struct ChannelPeer {
    output: mpsc::Sender<ChannelOutput>,
    input: mpsc::UnboundedReceiver<ChannelInput>,
    /// What the writers queued and the transport has not sent: the input this peer hands out counts as sent.
    backlog: Backlog,
    /// Shared with the writers: set by a transport whose connection is gone, they queue nothing more and report the
    /// channel gone, while what they queued before still reads.
    refused: Arc<AtomicBool>,
}

impl ChannelPeer {
    /// Delivers standard output, waiting while the reader is [`CHANNEL_DEPTH`] chunks behind; false once the reader
    /// is gone.
    pub async fn data(&self, bytes: Vec<u8>) -> bool {
        self.output.send(ChannelOutput::Data(bytes)).await.is_ok()
    }

    /// Delivers standard error, as [`ChannelPeer::data`].
    pub async fn stderr(&self, bytes: Vec<u8>) -> bool {
        self.output.send(ChannelOutput::Stderr(bytes)).await.is_ok()
    }

    /// Ends the channel. A peer dropped without ending reads as [`ChannelEnd::Lost`].
    pub async fn end(self, end: ChannelEnd) {
        // A reader that is gone has nothing left to tell.
        let _ = self.output.send(ChannelOutput::End(end)).await;
    }

    /// The core's next input, or `None` once every writer is gone; it counts as sent.
    pub async fn input(&mut self) -> Option<ChannelInput> {
        let input = self.input.recv().await;
        self.taken(input)
    }

    /// The core's next input if one is queued; it counts as sent.
    pub fn try_input(&mut self) -> Option<ChannelInput> {
        let input = self.input.try_recv().ok();
        self.taken(input)
    }

    fn taken(&self, input: Option<ChannelInput>) -> Option<ChannelInput> {
        if let Some(ChannelInput::Data(bytes)) = &input {
            self.backlog.sent(bytes.len());
        }
        input
    }

    /// Whether the reader is gone (nothing delivered would be read).
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.output.is_closed()
    }
}
