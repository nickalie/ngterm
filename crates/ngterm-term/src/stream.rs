//! A stream pane's IO: a pane whose program runs elsewhere and whose bytes travel over a channel its embedder owns (a
//! remote-only frontend's ssh channel running `ngterm-session attach`), in place of a PTY and a child. The pane holds
//! no thread of the channel's: the embedder reads the channel into [`StreamFeed`] and writes what [`StreamOut`] gives
//! it to the channel.
//!
//! - **Output in** ([`StreamFeed::deliver`]): the program's output joins the pane's mailbox ([`crate::mailbox`]) and
//!   the same IO worker feeds it to the VT in slices. Past the mailbox's high mark the call waits until the worker is
//!   down to its low mark, so a pane that cannot keep up holds the channel back (ssh's window then holds the remote
//!   end back), and a worker waiting on a clipboard answer holds the program's output, not its input.
//! - **Out** ([`StreamOut::next`]): the program's input ([`crate::Pane::write`] and every input path), the
//!   terminal's replies and the pane's own bytes, in the order they were made, and the grid after a resize, for the
//!   channel's window change. Replies are never refused, only dropped past [`REPLY_CAP`] queued bytes (a program that
//!   floods queries and reads no input loses replies, as on a PTY); input is refused past the pane's write limit
//!   ([`crate::PaneError::Backpressure`]) while anything is queued, and an empty queue takes a piece of any size.
//! - **Replies**: the pane's terminal answers the program's queries, as a PTY pane's does: an attach client filters
//!   out the queries its server answers, so whatever reaches the pane is its to answer. Clipboard reads stop the
//!   output for the frontend's answer ([`crate::PaneEvent::ClipboardRead`]), as on a PTY.
//! - **End**: [`StreamFeed::end_of_output`] once the channel's output ended; [`StreamFeed::exited`] reports the
//!   program's end (the channel's exit status) under the pane's [`crate::ExitPolicy`]. Call `end_of_output` first,
//!   even when the exit status arrives before the channel's EOF (as OpenSSH sends it): `exited` waits for the output
//!   to drain, and only the end of output ends that wait early, so without it `exited` blocks for a full second and
//!   output fed after it lands below the exit message. Dropping the pane closes
//!   [`StreamOut`] and makes [`StreamFeed::deliver`] refuse, so the embedder's threads end.
//! - **Kitty graphics**: images load from the escape sequence alone: a remote program's files, temporary files and
//!   shared memory are not this machine's.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use crate::mailbox::ReadPause;
use crate::pane::Shared;
use crate::{GridSize, PaneError};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The most bytes queued before the terminal's replies are dropped (when the write limit is lower): the PTY actor's
/// hard cap (`ngterm_pty::REPLY_CAP`), so a program that floods queries without reading its input loses replies on a
/// stream as on a PTY, and the queue never grows without bound.
pub(crate) const REPLY_CAP: usize = 16 * 1024 * 1024;

/// What a stream pane sends its channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outgoing {
    /// Bytes for the program: its input and the terminal's replies, in order.
    Bytes(Vec<u8>),
    /// The pane's grid changed: the channel's window change (cells and pixels).
    Resize(GridSize),
}

#[derive(Debug, Default)]
struct Queue {
    items: VecDeque<Outgoing>,
    /// Bytes queued.
    bytes: usize,
    closed: bool,
}

/// A stream pane's outbound queue: input, replies and resizes, in order.
#[derive(Debug)]
pub(crate) struct Outbound {
    queue: Mutex<Queue>,
    changed: Condvar,
    /// The most bytes queued before input is refused.
    write_limit: usize,
}

impl Outbound {
    pub(crate) fn new(write_limit: usize) -> Self {
        Self {
            queue: Mutex::new(Queue::default()),
            changed: Condvar::new(),
            write_limit,
        }
    }

    /// Queue `bytes`, at the end of the bytes queued last when nothing came between.
    fn push(queue: &mut Queue, bytes: &[u8]) {
        queue.bytes += bytes.len();
        if let Some(Outgoing::Bytes(last)) = queue.items.back_mut() {
            last.extend_from_slice(bytes);
        } else {
            queue.items.push_back(Outgoing::Bytes(bytes.to_vec()));
        }
    }

    /// Queue a reply of the terminal: never refused, dropped past [`REPLY_CAP`] (or the write limit, when higher).
    pub(crate) fn push_reply(&self, bytes: &[u8]) {
        let mut queue = lock(&self.queue);
        if queue.closed || bytes.is_empty() {
            return;
        }
        if queue.bytes + bytes.len() > REPLY_CAP.max(self.write_limit) {
            drop(queue);
            tracing::warn!("the program reads no input; a terminal reply is dropped");
            return;
        }
        Self::push(&mut queue, bytes);
        drop(queue);
        self.changed.notify_all();
    }

    /// Queue the program's input, within the write limit while anything is queued.
    pub(crate) fn input(&self, bytes: &[u8]) -> Result<(), PaneError> {
        let mut queue = lock(&self.queue);
        if queue.closed {
            return Err(PaneError::Closed);
        }
        if bytes.is_empty() {
            return Ok(());
        }
        // an empty queue takes a piece of any size (a whole paste), as the PTY actor's and a live pane's do
        if queue.bytes > 0 && queue.bytes + bytes.len() > self.write_limit {
            return Err(PaneError::Backpressure);
        }
        Self::push(&mut queue, bytes);
        drop(queue);
        self.changed.notify_all();
        Ok(())
    }

    /// Queue a resize; one queued and not taken yet is replaced.
    pub(crate) fn resize(&self, size: GridSize) {
        let mut queue = lock(&self.queue);
        if queue.closed {
            return;
        }
        if let Some(Outgoing::Resize(last)) = queue.items.back_mut() {
            *last = size;
        } else {
            queue.items.push_back(Outgoing::Resize(size));
        }
        drop(queue);
        self.changed.notify_all();
    }

    /// Bytes queued and not taken yet.
    pub(crate) fn queued(&self) -> usize {
        lock(&self.queue).bytes
    }

    pub(crate) fn close(&self) {
        let mut queue = lock(&self.queue);
        queue.closed = true;
        queue.items.clear();
        queue.bytes = 0;
        drop(queue);
        self.changed.notify_all();
    }

    /// The next item, waiting up to `timeout` (forever without one): `Ok(None)` when it passed with nothing queued.
    pub(crate) fn next(&self, timeout: Option<Duration>) -> Result<Option<Outgoing>, PaneError> {
        let deadline = timeout.map(|timeout| Instant::now() + timeout);
        let mut queue = lock(&self.queue);
        loop {
            if let Some(item) = queue.items.pop_front() {
                if let Outgoing::Bytes(bytes) = &item {
                    queue.bytes -= bytes.len();
                }
                return Ok(Some(item));
            }
            if queue.closed {
                return Err(PaneError::Closed);
            }
            queue = match deadline {
                None => self.changed.wait(queue).unwrap_or_else(PoisonError::into_inner),
                Some(deadline) => {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        return Ok(None);
                    }
                    self.changed
                        .wait_timeout(queue, left)
                        .unwrap_or_else(PoisonError::into_inner)
                        .0
                }
            };
        }
    }
}

/// The embedder's reader waits here while the mailbox holds its reads back.
#[derive(Debug)]
pub(crate) struct Gate {
    pause: ReadPause,
    closed: Mutex<bool>,
    changed: Condvar,
}

impl Gate {
    pub(crate) fn new(pause: ReadPause) -> Self {
        Self {
            pause,
            closed: Mutex::new(false),
            changed: Condvar::new(),
        }
    }

    /// The mailbox resumed the reads (or the pane closed): look again.
    pub(crate) fn wake(&self) {
        let _closed = lock(&self.closed);
        self.changed.notify_all();
    }

    pub(crate) fn close(&self) {
        *lock(&self.closed) = true;
        self.changed.notify_all();
    }

    /// Wait while the reads are paused; whether the pane still takes output.
    fn wait_resumed(&self) -> bool {
        let mut closed = lock(&self.closed);
        while !*closed && self.pause.is_paused() {
            closed = self.changed.wait(closed).unwrap_or_else(PoisonError::into_inner);
        }
        !*closed
    }

    fn is_closed(&self) -> bool {
        *lock(&self.closed)
    }
}

/// A stream pane's IO, held by the pane.
pub(crate) struct Stream {
    pub(crate) outbound: Arc<Outbound>,
    pub(crate) gate: Arc<Gate>,
}

impl Stream {
    pub(crate) fn close(&self) {
        self.outbound.close();
        self.gate.close();
    }
}

/// The input side of a stream pane's channel: the embedder's reader hands the program's output in here. `Send` and
/// `Sync`; it outlives the pane harmlessly (every call then refuses or does nothing).
pub struct StreamFeed {
    pub(crate) shared: Arc<Shared>,
    pub(crate) gate: Arc<Gate>,
    pub(crate) exited: AtomicBool,
}

impl std::fmt::Debug for StreamFeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamFeed").finish_non_exhaustive()
    }
}

impl StreamFeed {
    /// Queue the program's output for the terminal; waits while the pane is past its high mark (see the module docs).
    /// `false` once the pane is gone: the embedder stops reading.
    pub fn deliver(&self, bytes: &[u8]) -> bool {
        if self.gate.is_closed() {
            return false;
        }
        self.shared.mailbox().deliver(bytes);
        self.gate.wait_resumed()
    }

    /// The channel's output ended: the terminal is fed what came, and the pane's output is over.
    pub fn end_of_output(&self) {
        self.shared.mailbox().end_of_output();
    }

    /// The program ended with `code` (`None`: unknown, as a channel closed with no exit status; 1 when `signal` ended
    /// it) and `signal`: a clipboard read waiting is denied, the output fed so far reaches the screen, the exit
    /// policy's message is written, and [`crate::PaneEvent::Exited`] is reported. Runs on the caller's thread; only
    /// the first call counts.
    ///
    /// Call [`Self::end_of_output`] first: the output drains up to its end, and without one this waits the whole
    /// second it gives the output before reporting the exit (see the module docs).
    pub fn exited(&self, code: Option<u32>, signal: Option<String>) {
        if self.exited.swap(true, Ordering::AcqRel) {
            return;
        }
        // a signal's end is code 1, as a local child's is
        let code = code.or(signal.as_ref().map(|_| 1));
        if let Some(code) = code {
            crate::pane::record_status(&self.shared, code);
        }
        self.shared.mailbox().child_exited();
        crate::pane::report_exit(&self.shared, code, signal);
    }
}

/// The output side of a stream pane's channel: the embedder's writer takes what goes to the program here. `Send` and
/// `Sync`.
#[derive(Debug)]
pub struct StreamOut {
    pub(crate) outbound: Arc<Outbound>,
}

impl StreamOut {
    /// The next thing to send, waiting up to `timeout` (forever without one); `Ok(None)` when it passed with nothing
    /// to send. Consecutive bytes come as one [`Outgoing::Bytes`].
    ///
    /// # Errors
    ///
    /// [`PaneError::Closed`] once the pane is gone (what it had queued is dropped with it).
    pub fn next(&self, timeout: Option<Duration>) -> Result<Option<Outgoing>, PaneError> {
        self.outbound.next(timeout)
    }

    /// Bytes queued and not taken yet.
    #[must_use]
    pub fn queued(&self) -> usize {
        self.outbound.queued()
    }
}

#[cfg(test)]
mod tests;
