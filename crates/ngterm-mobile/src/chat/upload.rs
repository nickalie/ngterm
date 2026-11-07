//! A file the chat composer attaches, sent to the origin: `ngtermctl agent upload --name=NAME --size BYTES` on an
//! exec channel of the machine's connection, the file's bytes on its standard input, the stored path its answer
//! ([`ngterm_core::agent_chat::upload`] stores it). No sftp: the exec channel every other chat command uses carries it.
//!
//! - **Pacing:** the bytes go in [`CHUNK`]s while the channel's unsent backlog is under [`WINDOW`], so a slow link
//!   holds the sender rather than a queue of the whole file; the progress told is what the transport has sent.
//! - **The end:** after the last byte, the input's end; the origin stores the file and prints its path, or says why
//!   not (its words, as the error).
//! - **Stopping:** cancelling (the app's call dropped) closes the channel, dropping what is still queued, so while
//!   bytes are still to go the origin reads its input short and stores nothing. Once the progress has told every byte
//!   sent, the origin gets the whole file and then its input's end (the end queued after it, or the close itself) and
//!   stores it whatever follows: a cancel then is too late (the upload stays until the week's prune), which is why the
//!   app stops offering Cancel there. A link that moves nothing for [`STALL`] fails the upload as a cancel does.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use ngterm_core::command_restore::shell_quoted_line;
use ngterm_core::remote::tree::{REMOTE_CLI, remote_cli_chain};
use tokio::sync::watch;
use tokio::time::Instant;

use crate::api::MobileError;
use crate::transport::{Channel, ChannelEnd, ChannelOutput, Connection};

/// The most bytes one write queues.
pub(crate) const CHUNK: usize = 32 * 1024;

/// The most unsent bytes the upload keeps queued.
pub(crate) const WINDOW: usize = 256 * 1024;

/// How often a held upload looks at the backlog again.
const POLL: Duration = Duration::from_millis(20);

/// How long an upload may move nothing (no byte sent, no answer) before it fails.
pub(crate) const STALL: Duration = Duration::from_secs(30);

/// The error of an upload whose link moved nothing for [`STALL`].
pub(crate) const STALLED: &str = "the upload stalled: the machine took nothing for 30 seconds";

/// A way to stop an upload ([`crate::MobileCore::upload_file_cancellable`]) besides dropping its call: Swift's async
/// calls run a Rust future to its end whatever becomes of the task that made them, so the iOS app's Cancel upload
/// goes through this. Cancelling it is as dropping the call: the upload stops while bytes are still to go, and once
/// every byte has gone it is too late. Cancelling twice, or after the upload ended, changes nothing.
#[derive(Debug, uniffi::Object)]
pub struct UploadCancel {
    cancelled: watch::Sender<bool>,
}

impl Default for UploadCancel {
    fn default() -> Self {
        Self {
            cancelled: watch::channel(false).0,
        }
    }
}

#[uniffi::export]
impl UploadCancel {
    /// A token no upload has been cancelled through yet.
    #[uniffi::constructor]
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Stops the upload this token was given to, as dropping its call would.
    pub fn cancel(&self) {
        self.cancelled.send_replace(true);
    }
}

impl UploadCancel {
    /// Resolves once [`UploadCancel::cancel`] has been called, at once when it was before.
    pub(crate) async fn cancelled(&self) {
        let mut seen = self.cancelled.subscribe();
        // the sender lives in self, so the wait ends only with a cancel
        let _ = seen.wait_for(|cancelled| *cancelled).await;
    }
}

/// The upload's remote command. The name goes as `--name=NAME`, one word, so a name that starts with `-` stays a value.
#[must_use]
pub(crate) fn upload_tail(name: &str, size: usize) -> String {
    let words = [
        "agent".to_owned(),
        "upload".to_owned(),
        format!("--name={name}"),
        "--size".to_owned(),
        size.to_string(),
    ];
    remote_cli_chain(&format!("{REMOTE_CLI} {}", shell_quoted_line(&words)))
}

/// Sends `bytes` as `name` on `connection` and returns the stored path, telling `progress` (bytes sent, total) as the
/// transport sends them; `cancelled` resolving stops it.
///
/// # Errors
///
/// [`MobileError::Cancelled`] when `cancelled` resolved first, [`MobileError::Remote`] with the origin's words when it
/// stored nothing, or with the transport's when the link failed or stalled.
pub(crate) async fn upload<C: Connection>(
    connection: &C,
    name: &str,
    bytes: &[u8],
    progress: &dyn Fn(u64, u64),
    cancelled: impl Future<Output = ()>,
) -> Result<String, MobileError> {
    tokio::pin!(cancelled);
    let command = upload_tail(name, bytes.len());
    let channel = tokio::select! {
        opened = connection.exec(&command, None) => {
            opened.map_err(|error| MobileError::Remote { reason: error.to_string() })?
        }
        () = &mut cancelled => return Err(MobileError::Cancelled),
    };
    let closer = channel.writer.clone();
    tokio::select! {
        answer = transfer(channel, bytes, progress) => answer,
        () = &mut cancelled => {
            // a channel already gone has nothing to close
            let _ = closer.close();
            Err(MobileError::Cancelled)
        }
    }
}

/// What the origin said on a channel, collected until it ended.
#[derive(Default)]
struct Said {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// Writes `bytes` down `channel` paced by its backlog, then its end, and reads the origin's answer.
async fn transfer(channel: Channel, bytes: &[u8], progress: &dyn Fn(u64, u64)) -> Result<String, MobileError> {
    let Channel { mut reader, writer } = channel;
    let total = bytes.len();
    let mut said = Said::default();
    let mut queued = 0;
    let mut ended_input = false;
    let mut told = None;
    let mut moved = Instant::now();
    let end = loop {
        while queued < total && writer.backlog() < WINDOW {
            let next = (queued + CHUNK).min(total);
            if !writer.write(&bytes[queued..next]) {
                // the channel is gone: its end says why
                break;
            }
            queued = next;
        }
        if queued == total && !ended_input {
            ended_input = writer.eof();
        }
        let sent = queued - writer.backlog().min(queued);
        if told != Some(sent) {
            told = Some(sent);
            moved = Instant::now();
            progress(sent as u64, total as u64);
        }
        tokio::select! {
            output = reader.read() => {
                moved = Instant::now();
                match output {
                    ChannelOutput::Data(data) => said.stdout.extend_from_slice(&data),
                    ChannelOutput::Stderr(data) => said.stderr.extend_from_slice(&data),
                    ChannelOutput::End(end) => break end,
                }
            }
            () = tokio::time::sleep(POLL) => {
                if moved.elapsed() >= STALL {
                    let _ = writer.close();
                    return Err(MobileError::Remote { reason: STALLED.to_owned() });
                }
            }
        }
    };
    answer(&end, &said)
}

/// The stored path the origin printed, or why it stored nothing.
fn answer(end: &ChannelEnd, said: &Said) -> Result<String, MobileError> {
    let stderr = String::from_utf8_lossy(&said.stderr);
    let stderr = stderr.trim();
    let reason = match end {
        ChannelEnd::Exit { code: Some(0), .. } => {
            let stdout = String::from_utf8_lossy(&said.stdout);
            let path = stdout.trim_end_matches(['\r', '\n']);
            if !path.is_empty() {
                return Ok(path.to_owned());
            }
            "the machine stored the file but named no path".to_owned()
        }
        _ if !stderr.is_empty() => stderr.strip_prefix("Error: ").unwrap_or(stderr).to_owned(),
        ChannelEnd::Exit { code: Some(code), .. } => format!("exit {code}"),
        ChannelEnd::Exit { signal, .. } => format!("ended by {}", signal.as_deref().unwrap_or("a signal")),
        ChannelEnd::Lost(reason) => reason.clone(),
    };
    Err(MobileError::Remote { reason })
}
