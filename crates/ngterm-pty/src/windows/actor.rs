//! The PTY actor on Windows: a reader thread blocks on the pseudoconsole's output pipe and hands each read to the
//! read callback; a writer thread writes the outbox into its input pipe; resizes go straight to the console.
//!
//! herdr: `src/pty/actor.rs` (the `windows` module: `PtyIoActor`'s reader, writer and control threads, the response
//! ordering lock) at `HERDR_REV` (`build/pins.env`), licensed under the Apache License 2.0, modified: std threads in
//! place of tokio's input task; the outbox of `crate::actor` in place of herdr's write channel; resizes on the
//! caller's thread under the console's lock in place of a control thread; and the handoff herdr's Windows actor does
//! not have: a quiesce parks the reader (its blocked read cancelled with `CancelSynchronousIo`) and waits for the
//! writer to drain, a read that completed meanwhile is held back and delivered on rollback or handed on by the
//! release, and a release leaves the pseudoconsole open for its new owner; a shutdown drains the output pipe only of
//! a console the actor owns, so an actor over a taken-over console that never rolled back reads nothing of it.

use std::fs::File;
use std::io::{self, Read, Write};
use std::os::windows::io::{AsRawHandle, BorrowedHandle, OwnedHandle, RawHandle};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use portable_pty::MasterPty;
use portable_pty::win::conpty::{ConPtyHandoff, ConPtyMasterPty};
use windows_sys::Win32::Foundation::{ERROR_BROKEN_PIPE, ERROR_NO_DATA, ERROR_OPERATION_ABORTED};
use windows_sys::Win32::System::IO::CancelSynchronousIo;

use super::Master;
use crate::Size;
use crate::actor::{ActorConfig, ActorHandle, ExitCallback, ExitReason, Gate, ReadPause, Released, Shared, lock};

/// The most output one read hands to the callback.
const READ_BUFFER: usize = 256 * 1024;
/// How often a waiting thread looks again, besides being notified.
const WAIT_SLICE: Duration = Duration::from_millis(10);
/// How long the writer sleeps on an empty outbox before it looks again (a fallback: every queued chunk notifies it).
const IDLE_WAIT: Duration = Duration::from_secs(1);

/// What the reader does next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Read and deliver.
    Running,
    /// Read nothing: park until rolled back or released.
    Quiesced,
    /// Stop: another process owns the pseudoconsole now.
    Released,
    /// Shut down as the console's owner: read and drop until the console's pipe ends, so its close never waits on a
    /// full pipe.
    Discarding,
    /// Shut down before owning the console (an actor started quiesced that was never rolled back): read nothing, as
    /// the console's pipe still carries the output of the process that owns it.
    Stopped,
}

/// Whether the actor owns its pseudoconsole. A shutdown closes only an owner's console, so only an owner drains its
/// pipe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Console {
    /// Started running, or started quiesced and rolled back.
    Owned,
    /// Started quiesced over a console another process still owns, and not rolled back yet.
    Borrowed,
}

#[derive(Debug)]
struct ReaderState {
    mode: Mode,
    console: Console,
    /// The reader is inside its `ReadFile`: only then may it be cancelled, so no other I/O of the thread is.
    reading: bool,
    /// The reader waits in its park.
    parked: bool,
    /// The reader thread finished.
    exited: bool,
    /// A read that completed while quiescing, not delivered.
    held: Vec<u8>,
}

/// What the handles and the two threads share beside the outbox.
struct Inner {
    reader: Mutex<ReaderState>,
    reader_changed: Condvar,
    /// The reader thread, whose handle a quiesce cancels its blocked read through.
    reader_thread: Mutex<Option<JoinHandle<()>>>,
    /// The pseudoconsole: resized here, closed on shutdown, left open (leaked) on release.
    conpty: Mutex<Option<Box<ConPtyMasterPty>>>,
    input: File,
    output: File,
}

impl Inner {
    fn reader(&self) -> MutexGuard<'_, ReaderState> {
        lock(&self.reader)
    }

    /// Cancel the reader's blocked `ReadFile`, if it is in one; called with the reader's state locked, so the thread
    /// cannot be anywhere else in between.
    fn cancel_read(&self, state: &ReaderState) {
        if !state.reading {
            return;
        }
        if let Some(thread) = lock(&self.reader_thread).as_ref() {
            // SAFETY: cancels the pending synchronous I/O of a thread whose handle the stored JoinHandle keeps open;
            // "nothing to cancel" is a harmless failure.
            unsafe { CancelSynchronousIo(thread.as_raw_handle()) };
        }
    }

    /// Close the pseudoconsole on a thread of its own (`ClosePseudoConsole` may wait for the console host), so the
    /// reader then reads its last output and the end of its pipe.
    fn close_console(&self) {
        if let Some(conpty) = lock(&self.conpty).take() {
            let closing = std::thread::Builder::new()
                .name("pty-close".to_owned())
                .spawn(move || drop(conpty));
            if let Err(err) = closing {
                tracing::debug!(%err, "no thread to close the pseudoconsole on");
            }
        }
    }
}

/// The handles' side of the actor.
pub(crate) struct Control {
    inner: Arc<Inner>,
}

fn closed() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "the PTY actor stopped")
}

fn timed_out(what: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        format!("timed out waiting for the PTY actor to {what}"),
    )
}

impl Control {
    pub(crate) fn shutdown(&self, shared: &Shared) {
        shared.outbox().close();
        shared.outbox_changed.notify_all();
        let mut state = self.inner.reader();
        let next = match state.mode {
            Mode::Running => Some(Mode::Discarding),
            Mode::Quiesced if state.console == Console::Owned => Some(Mode::Discarding),
            // the console host and its pipe stay with the process that owns them: a read from here would take its
            // output
            Mode::Quiesced => Some(Mode::Stopped),
            Mode::Released | Mode::Discarding | Mode::Stopped => None,
        };
        if let Some(mode) = next {
            state.mode = mode;
            self.inner.reader_changed.notify_all();
        }
        drop(state);
        self.inner.close_console();
    }
}

impl ActorHandle {
    fn inner(&self) -> &Inner {
        &self.control().inner
    }

    /// Wake the writer to look at what was just queued.
    pub(crate) fn wake(&self) {
        self.shared().outbox_changed.notify_all();
    }

    /// Resize the pseudoconsole (pixel sizes are not passed on: `ConPTY` has none).
    pub fn resize(&self, size: Size) {
        if let Err(err) = self.try_resize(size) {
            tracing::debug!(%err, "resizing the pseudoconsole failed");
        }
    }

    /// [`ActorHandle::resize`], reporting a failure: whether `ResizePseudoConsole` reached the console host. Every
    /// call, to the size it has too, makes the console host repaint its viewport from its own buffer.
    ///
    /// # Errors
    ///
    /// The pseudoconsole is closed or released, or `ResizePseudoConsole` failed.
    pub fn try_resize(&self, size: Size) -> io::Result<()> {
        let conpty = lock(&self.inner().conpty);
        let conpty = conpty.as_ref().ok_or_else(closed)?;
        conpty.resize(size.pty()).map_err(|err| crate::pty_error(&err))
    }

    /// Make the reader look at its [`crate::ReadPause`] again, after it was turned off.
    pub fn wake_reader(&self) {
        let inner = self.inner();
        let state = inner.reader();
        inner.reader_changed.notify_all();
        drop(state);
    }

    /// The child exited: close the pseudoconsole, which otherwise keeps its output pipe open for good, so the reader
    /// delivers the last output and the actor stops with [`ExitReason::Eof`].
    pub fn child_exited(&self) {
        self.inner().close_console();
    }

    /// Quiesce for a handoff: input is refused from now on, the reader parks (a read it completes meanwhile is held
    /// back), what is queued is written, then nothing is read or written until [`ActorHandle::rollback_handoff`] or
    /// [`ActorHandle::release_after_commit`]. A failure rolls back.
    ///
    /// # Errors
    ///
    /// A handoff is already in progress (`WouldBlock`), the actor stopped, or the reader did not park or the outbox
    /// did not drain within `timeout`.
    pub fn begin_handoff(&self, timeout: Duration) -> io::Result<()> {
        {
            let mut outbox = self.shared().outbox();
            match outbox.gate {
                Gate::Open => outbox.gate = Gate::Draining,
                Gate::Closed => return Err(closed()),
                Gate::Draining | Gate::Paused => {
                    return Err(io::Error::new(
                        io::ErrorKind::WouldBlock,
                        "a PTY handoff is already in progress",
                    ));
                }
            }
        }
        let deadline = Instant::now() + timeout;
        let result = self.park_reader(deadline).and_then(|()| self.drain(deadline));
        if result.is_err() {
            let _ = self.rollback_handoff();
        }
        result
    }

    fn park_reader(&self, deadline: Instant) -> io::Result<()> {
        let inner = self.inner();
        let mut state = inner.reader();
        if state.mode != Mode::Running {
            return Err(closed());
        }
        state.mode = Mode::Quiesced;
        // a reader waiting out a read pause parks now
        inner.reader_changed.notify_all();
        loop {
            if state.exited {
                return Err(closed());
            }
            if state.parked {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(timed_out("park its reader"));
            }
            inner.cancel_read(&state);
            state = inner
                .reader_changed
                .wait_timeout(state, WAIT_SLICE)
                .map_or_else(|poisoned| poisoned.into_inner().0, |(guard, _)| guard);
        }
    }

    fn drain(&self, deadline: Instant) -> io::Result<()> {
        let shared = self.shared();
        let mut outbox = shared.outbox();
        loop {
            if outbox.gate == Gate::Closed {
                return Err(closed());
            }
            if outbox.drained() {
                outbox.gate = Gate::Paused;
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(timed_out("write its queue"));
            }
            outbox = shared.wait_outbox(outbox, WAIT_SLICE);
        }
    }

    /// Hand the output a quiesced actor read but held back (a read that completed while it quiesced) to the read
    /// callback, staying quiesced, so it is in the state the handoff passes on rather than left behind in the actor:
    /// a rollback then has nothing held, and a release nothing pending. Nothing unless quiesced. After
    /// [`ActorHandle::begin_handoff`] returned the reader is parked, so no read is in progress and none starts: what
    /// this delivers is all the actor read.
    pub fn deliver_held(&self) {
        let inner = self.inner();
        let mut state = inner.reader();
        if state.mode != Mode::Quiesced || state.held.is_empty() {
            return;
        }
        let held = std::mem::take(&mut state.held);
        drop(state);
        self.shared().deliver(&held);
    }

    /// Duplicates of the handles the pseudoconsole is made of (its pipes' ends and its three console handles), for
    /// the process taking it over; only while quiesced.
    ///
    /// # Safety
    ///
    /// The console handles are read from behind the HPCON, whose layout is undocumented: the caller has checked that
    /// the `ConPTY` implementation this process runs on keeps them there (the session server's support gate).
    ///
    /// # Errors
    ///
    /// The actor is not quiesced or stopped, or a handle could not be duplicated.
    pub unsafe fn duplicate_for_handoff(&self) -> io::Result<ConPtyHandoff> {
        let inner = self.inner();
        if inner.reader().mode != Mode::Quiesced {
            return Err(io::Error::other(
                "the PTY actor must be quiesced before its handoff duplicate",
            ));
        }
        let input = OwnedHandle::from(inner.input.try_clone()?);
        let output = OwnedHandle::from(inner.output.try_clone()?);
        let conpty = lock(&inner.conpty);
        let conpty = conpty.as_ref().ok_or_else(closed)?;
        // SAFETY: the caller's contract: the console handles are where this implementation keeps them.
        let [signal, reference, process] = unsafe { conpty.console_handles() };
        Ok(ConPtyHandoff {
            input,
            output,
            console: [duplicate(signal)?, duplicate(reference)?, duplicate(process)?],
        })
    }

    /// End a quiesce: what the reader held back is delivered first, then it reads again, the outbox is written and
    /// input is taken (also how an actor started quiesced becomes the console's owner).
    ///
    /// # Errors
    ///
    /// The actor stopped or was released.
    pub fn rollback_handoff(&self) -> io::Result<()> {
        let inner = self.inner();
        loop {
            let mut state = inner.reader();
            if state.exited || matches!(state.mode, Mode::Released | Mode::Discarding | Mode::Stopped) {
                return Err(closed());
            }
            if state.held.is_empty() {
                // running again in the step that found nothing held, so no read still in progress (the reader may not
                // have parked) can complete into `held` after it was taken and stay there
                if state.mode == Mode::Quiesced {
                    state.mode = Mode::Running;
                    state.console = Console::Owned;
                }
                inner.reader_changed.notify_all();
                break;
            }
            let held = std::mem::take(&mut state.held);
            drop(state);
            // still quiesced while the held read goes first: a read completing meanwhile is held, taken next round
            self.shared().deliver(&held);
        }
        let mut outbox = self.shared().outbox();
        if outbox.gate != Gate::Closed {
            outbox.gate = Gate::Open;
        }
        drop(outbox);
        self.shared().outbox_changed.notify_all();
        Ok(())
    }

    /// Let go of the pseudoconsole once another process owns it: the actor stops ([`ExitReason::Released`]) without
    /// reading or writing again, what it queued is dropped, and the console is left open (never closed from here).
    /// Only after [`ActorHandle::begin_handoff`]. The output read meanwhile is in [`Released::pending_output`].
    ///
    /// # Errors
    ///
    /// The actor is not quiesced, or stopped.
    pub fn release_after_commit(&self) -> io::Result<Released> {
        let inner = self.inner();
        let mut state = inner.reader();
        if state.mode != Mode::Quiesced {
            return Err(io::Error::other("the PTY actor must be quiesced before its release"));
        }
        state.mode = Mode::Released;
        let pending_output = std::mem::take(&mut state.held);
        inner.reader_changed.notify_all();
        drop(state);
        self.shared().outbox().close();
        self.shared().outbox_changed.notify_all();
        if let Some(conpty) = lock(&inner.conpty).take() {
            // ClosePseudoConsole would end the console host the new owner now holds
            std::mem::forget(conpty);
        }
        Ok(Released { pending_output })
    }
}

fn duplicate(raw: RawHandle) -> io::Result<OwnedHandle> {
    // SAFETY: `raw` is one of the console handles, open as long as the pseudoconsole the caller holds locked.
    unsafe { BorrowedHandle::borrow_raw(raw) }.try_clone_to_owned()
}

pub(crate) fn spawn_actor(config: ActorConfig) -> io::Result<ActorHandle> {
    let ActorConfig {
        name,
        master,
        initially_quiesced,
        input_limit,
        read_pause,
        on_read,
        on_exit,
    } = config;
    let Master { conpty, input, output } = master;
    let shared = Arc::new(Shared::new(on_read, input_limit, initially_quiesced, read_pause));
    let inner = Arc::new(Inner {
        reader: Mutex::new(ReaderState {
            mode: if initially_quiesced {
                Mode::Quiesced
            } else {
                Mode::Running
            },
            console: if initially_quiesced {
                Console::Borrowed
            } else {
                Console::Owned
            },
            reading: false,
            parked: false,
            exited: false,
            held: Vec::new(),
        }),
        reader_changed: Condvar::new(),
        reader_thread: Mutex::new(None),
        conpty: Mutex::new(Some(conpty)),
        input,
        output,
    });
    let control = Control {
        inner: Arc::clone(&inner),
    };
    let writer = {
        let (writer_shared, writer_inner) = (Arc::clone(&shared), Arc::clone(&inner));
        std::thread::Builder::new()
            .name(format!("pty-write-{name}"))
            .spawn(move || write_loop(&writer_shared, &writer_inner.input))
    };
    if let Err(err) = writer {
        control.shutdown(&shared);
        return Err(err);
    }
    let reader = {
        let (reader_shared, reader_inner) = (Arc::clone(&shared), Arc::clone(&inner));
        std::thread::Builder::new()
            .name(format!("pty-read-{name}"))
            .spawn(move || run_reader(&reader_shared, &reader_inner, on_exit))
    };
    match reader {
        Ok(thread) => *lock(&inner.reader_thread) = Some(thread),
        Err(err) => {
            control.shutdown(&shared);
            return Err(err);
        }
    }
    Ok(ActorHandle::new(shared, control))
}

fn run_reader(shared: &Shared, inner: &Inner, on_exit: Option<ExitCallback>) {
    let reason = read_loop(shared, inner);
    let mut state = inner.reader();
    state.exited = true;
    state.reading = false;
    inner.reader_changed.notify_all();
    drop(state);
    shared.outbox().close();
    shared.outbox_changed.notify_all();
    tracing::debug!(?reason, "the PTY actor stops");
    if let Some(on_exit) = on_exit {
        on_exit(reason);
    }
}

/// Read until the pipe ends or the actor is released; returns why it stopped.
fn read_loop(shared: &Shared, inner: &Inner) -> ExitReason {
    let mut buffer = vec![0_u8; READ_BUFFER];
    loop {
        if let Some(reason) = wait_to_read(inner, &shared.read_pause) {
            return reason;
        }
        let result = (&inner.output).read(&mut buffer);
        let mut state = inner.reader();
        state.reading = false;
        match result {
            Ok(0) => return ended(state.mode),
            Ok(read) => match state.mode {
                Mode::Running => {
                    drop(state);
                    shared.deliver(&buffer[..read]);
                }
                Mode::Quiesced | Mode::Released | Mode::Stopped => state.held.extend_from_slice(&buffer[..read]),
                Mode::Discarding => {}
            },
            Err(err) if os_error(&err) == Some(ERROR_OPERATION_ABORTED) => {}
            Err(err) if matches!(os_error(&err), Some(ERROR_BROKEN_PIPE | ERROR_NO_DATA)) => {
                return ended(state.mode);
            }
            Err(err) => {
                tracing::debug!(%err, "reading the pseudoconsole failed; the PTY actor stops");
                return if matches!(state.mode, Mode::Discarding | Mode::Stopped) {
                    ExitReason::Shutdown
                } else {
                    ExitReason::Failed
                };
            }
        }
    }
}

/// Why the reader stopped when the pipe ended in `mode`.
fn ended(mode: Mode) -> ExitReason {
    match mode {
        Mode::Discarding | Mode::Stopped => ExitReason::Shutdown,
        Mode::Released => ExitReason::Released,
        Mode::Running | Mode::Quiesced => ExitReason::Eof,
    }
}

/// Park while quiesced, and wait while reads are paused; `Some` once released or stopped, else mark the reader as
/// reading and return. A shutdown (discarding) reads despite a pause, so the console's close never waits on its pipe.
fn wait_to_read(inner: &Inner, pause: &ReadPause) -> Option<ExitReason> {
    let mut state = inner.reader();
    loop {
        match state.mode {
            Mode::Running if pause.is_paused() => {
                state = inner
                    .reader_changed
                    .wait_timeout(state, IDLE_WAIT)
                    .map_or_else(|poisoned| poisoned.into_inner().0, |(guard, _)| guard);
            }
            Mode::Running | Mode::Discarding => {
                state.reading = true;
                return None;
            }
            Mode::Released => return Some(ExitReason::Released),
            Mode::Stopped => return Some(ExitReason::Shutdown),
            Mode::Quiesced => {
                state.parked = true;
                inner.reader_changed.notify_all();
                state = inner
                    .reader_changed
                    .wait_timeout(state, IDLE_WAIT)
                    .map_or_else(|poisoned| poisoned.into_inner().0, |(guard, _)| guard);
                state.parked = false;
            }
        }
    }
}

fn os_error(err: &io::Error) -> Option<u32> {
    err.raw_os_error().and_then(|code| u32::try_from(code).ok())
}

/// The next chunk for the writer, which holds it (`busy`) until written; `None` once the outbox closed.
fn next_chunk(shared: &Shared) -> Option<Vec<u8>> {
    let mut outbox = shared.outbox();
    loop {
        if outbox.gate == Gate::Closed {
            return None;
        }
        if outbox.writable()
            && let Some(chunk) = outbox.take_front()
        {
            outbox.busy = true;
            return Some(chunk);
        }
        outbox = shared.wait_outbox(outbox, IDLE_WAIT);
    }
}

/// Write the outbox into the console's input pipe, a chunk at a time, until it closes or a write fails.
fn write_loop(shared: &Shared, input: &File) {
    while let Some(chunk) = next_chunk(shared) {
        let mut writer = input;
        let result = writer.write_all(&chunk).and_then(|()| writer.flush());
        shared.outbox().busy = false;
        shared.outbox_changed.notify_all();
        if let Err(err) = result {
            tracing::debug!(%err, "writing the pseudoconsole failed; the PTY takes no more input");
            shared.outbox().close();
            shared.outbox_changed.notify_all();
            return;
        }
    }
}

/// The actor over real pseudoconsoles with no child: over the console's own pipes, or over pipes of the test's in
/// their place (what the writer wrote is read from one, what the reader reads is written into the other, and a pipe
/// nobody reads holds the writer back), or over an output it cannot read.
#[cfg(test)]
mod tests {
    use std::io::{PipeReader, PipeWriter};
    use std::sync::mpsc::{self, Receiver};

    use portable_pty::PtySystem;
    use portable_pty::win::conpty::ConPtySystem;

    use super::*;
    use crate::{Actor, WriteError};

    /// How long a test waits for the actor's threads (a CI runner can be slow to start a console host).
    const PATIENCE: Duration = Duration::from_secs(60);
    /// More than a pipe's buffer holds: a write of it blocks until the other end reads.
    const BIG: usize = 4 * 1024 * 1024;
    /// What a stopped actor's calls fail with.
    const STOPPED: &str = "the PTY actor stopped";

    #[test]
    fn the_pipe_ending_names_the_reason_of_the_mode() {
        assert_eq!(ended(Mode::Running), ExitReason::Eof);
        assert_eq!(ended(Mode::Quiesced), ExitReason::Eof);
        assert_eq!(ended(Mode::Released), ExitReason::Released);
        assert_eq!(ended(Mode::Discarding), ExitReason::Shutdown);
        assert_eq!(ended(Mode::Stopped), ExitReason::Shutdown);
    }

    /// A pseudoconsole with no child.
    fn console() -> ConPtyMasterPty {
        let pair = ConPtySystem::default()
            .openpty(Size::cells(24, 80).pty())
            .expect("a pseudoconsole");
        drop(pair.slave);
        let master: Box<dyn MasterPty> = pair.master;
        let conpty = master
            .downcast::<ConPtyMasterPty>()
            .map_err(|_| "another kind of master");
        *conpty.expect("a ConPTY master")
    }

    /// A master over a pseudoconsole and its own pipes.
    fn console_master() -> Master {
        Master::new(Box::new(console())).expect("the console's pipes")
    }

    /// The test's ends of the pipes a [`piped_master`] holds in place of the console's.
    struct Pipes {
        /// What the actor's writer wrote.
        input: PipeReader,
        /// What the actor's reader reads.
        output: PipeWriter,
    }

    impl Pipes {
        /// Write `bytes` as the console's output.
        fn feed(&self, bytes: &[u8]) {
            (&self.output).write_all(bytes).expect("the output pipe takes it");
        }

        /// The next `len` bytes the actor wrote as the console's input.
        fn written(&self, len: usize) -> Vec<u8> {
            let mut input = self.input.try_clone().expect("a clone of the input pipe");
            let (sends, read) = mpsc::channel();
            std::thread::spawn(move || {
                let mut bytes = vec![0; len];
                let _ = sends.send(input.read_exact(&mut bytes).map(|()| bytes));
            });
            read.recv_timeout(PATIENCE)
                .expect("the input never came")
                .expect("the input pipe broke")
        }
    }

    /// A master over a pseudoconsole whose pipes are the test's (the console's own stay inside it, unused).
    fn piped_master() -> (Master, Pipes) {
        let (input, writer) = std::io::pipe().expect("an input pipe");
        let (reader, output) = std::io::pipe().expect("an output pipe");
        let master = Master {
            conpty: Box::new(console()),
            input: File::from(OwnedHandle::from(writer)),
            output: File::from(OwnedHandle::from(reader)),
        };
        (master, Pipes { input, output })
    }

    /// A master whose output is a file opened for writing only, so its every read fails, with the test's end of its
    /// input pipe.
    fn unreadable_master(dir: &tempfile::TempDir) -> (Master, PipeReader) {
        let (input, writer) = std::io::pipe().expect("an input pipe");
        let master = Master {
            conpty: Box::new(console()),
            input: File::from(OwnedHandle::from(writer)),
            output: File::create(dir.path().join("output")).expect("a write-only output"),
        };
        (master, input)
    }

    /// A started actor: what it delivered to its read callback, and why it stopped.
    struct Started {
        handle: ActorHandle,
        delivered: Receiver<Vec<u8>>,
        exit: Receiver<ExitReason>,
    }

    impl Started {
        /// What the actor delivered, in order, until `len` bytes came.
        fn received(&self, len: usize) -> Vec<u8> {
            let deadline = Instant::now() + PATIENCE;
            let mut got = Vec::new();
            while got.len() < len {
                let left = deadline.saturating_duration_since(Instant::now());
                let read = self.delivered.recv_timeout(left);
                got.extend(read.unwrap_or_else(|_| panic!("only {got:?} was delivered")));
            }
            got
        }

        fn stopped(&self) -> ExitReason {
            self.exit.recv_timeout(PATIENCE).expect("the actor never stopped")
        }
    }

    /// Start an actor over `master`, its config changed by `tune`; its read callback answers the cursor query a
    /// pseudoconsole starts with.
    fn start(master: Master, tune: impl FnOnce(&mut ActorConfig)) -> Started {
        let (reads, delivered) = mpsc::channel();
        let (exits, exit) = mpsc::channel();
        let on_read = Box::new(move |bytes: &[u8]| {
            let _ = reads.send(bytes.to_vec());
            if bytes.windows(4).any(|window| window == b"\x1b[6n") {
                vec![b"\x1b[1;1R".to_vec()]
            } else {
                Vec::new()
            }
        });
        let mut config = ActorConfig::new("test", master, on_read);
        config.on_exit = Some(Box::new(move |reason| {
            let _ = exits.send(reason);
        }));
        tune(&mut config);
        Started {
            handle: Actor::spawn(config).expect("an actor"),
            delivered,
            exit,
        }
    }

    fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting until {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn assert_error<T>(result: io::Result<T>, kind: io::ErrorKind, message: &str) {
        match result {
            Ok(_) => panic!("succeeded where {message:?} was expected"),
            Err(err) => assert_eq!((err.kind(), err.to_string().as_str()), (kind, message)),
        }
    }

    /// Quiesce the actor as a handoff does while its reader is inside a read, but without cancelling that read: the
    /// race a handoff's quiesce can lose, its read completing before the cancel reaches it, made certain.
    fn quiesce_mid_read(handle: &ActorHandle) {
        let inner = handle.inner();
        // nothing is in the pipe, so the reader stays in its read until the test writes some
        wait_until("the reader reads", || inner.reader().reading);
        handle.shared().outbox().gate = Gate::Paused;
        inner.reader().mode = Mode::Quiesced;
    }

    /// Wait for the reader to hold `len` bytes it read while quiesced, and to park.
    fn wait_for_held(handle: &ActorHandle, len: usize) {
        wait_until("the read is held", || {
            let state = handle.inner().reader();
            state.parked && state.held.len() == len
        });
    }

    #[test]
    fn a_second_handoff_is_refused_while_one_is_in_progress() {
        let actor = start(console_master(), |_| {});
        actor
            .handle
            .begin_handoff(PATIENCE)
            .expect("the first handoff quiesces");
        assert_error(
            actor.handle.begin_handoff(PATIENCE),
            io::ErrorKind::WouldBlock,
            "a PTY handoff is already in progress",
        );
        actor.handle.rollback_handoff().expect("the first handoff rolls back");
        assert_eq!(actor.handle.write_input(b"x"), Ok(()), "running again");
    }

    #[test]
    fn an_actor_started_quiesced_takes_a_handoff_only_once_rolled_back() {
        let actor = start(console_master(), |config| config.initially_quiesced = true);
        assert_error(
            actor.handle.begin_handoff(PATIENCE),
            io::ErrorKind::WouldBlock,
            "a PTY handoff is already in progress",
        );
        actor
            .handle
            .rollback_handoff()
            .expect("the rollback makes it the owner");
        actor.handle.begin_handoff(PATIENCE).expect("the owner quiesces");
        actor.handle.rollback_handoff().expect("and rolls back");
    }

    #[test]
    fn a_stopped_actor_refuses_every_handoff_step() {
        let actor = start(console_master(), |_| {});
        actor.handle.shutdown();
        assert_eq!(actor.stopped(), ExitReason::Shutdown);
        assert_error(actor.handle.begin_handoff(PATIENCE), io::ErrorKind::BrokenPipe, STOPPED);
        assert_error(actor.handle.rollback_handoff(), io::ErrorKind::BrokenPipe, STOPPED);
        assert_error(
            actor.handle.release_after_commit(),
            io::ErrorKind::Other,
            "the PTY actor must be quiesced before its release",
        );
        assert_error(
            // SAFETY: refused before any console handle is read.
            unsafe { actor.handle.duplicate_for_handoff() },
            io::ErrorKind::Other,
            "the PTY actor must be quiesced before its handoff duplicate",
        );
        assert_eq!(actor.handle.write_input(b"x"), Err(WriteError::Closed));
    }

    #[test]
    fn a_closed_console_takes_no_resize() {
        let actor = start(console_master(), |_| {});
        actor.handle.child_exited();
        assert_eq!(actor.stopped(), ExitReason::Eof);
        assert_error(
            actor.handle.try_resize(Size::cells(30, 100)),
            io::ErrorKind::BrokenPipe,
            STOPPED,
        );
        // the same failure, only logged
        actor.handle.resize(Size::cells(30, 100));
    }

    /// A size beyond `i16` is refused with the requested dimensions, and the console keeps its last size.
    #[test]
    fn a_resize_the_console_refuses_is_reported() {
        let actor = start(console_master(), |_| {});
        for (rows, cols) in [(24, 32_768), (32_768, 80), (40_000, 40_000), (u16::MAX, u16::MAX)] {
            let err = actor
                .handle
                .try_resize(Size::cells(rows, cols))
                .expect_err("the console took a size past i16");
            assert_eq!(
                err.to_string(),
                format!("pseudoconsole size {cols}x{rows} exceeds the maximum 32767x32767")
            );
            let console = lock(&actor.handle.inner().conpty);
            let size = console.as_ref().expect("the console").get_size().expect("the size");
            assert_eq!((size.rows, size.cols), (24, 80));
        }
        actor
            .handle
            .try_resize(Size::cells(30, 100))
            .expect("a size the console takes");
    }

    #[test]
    fn handoff_steps_out_of_order_are_refused_and_leave_the_actor_running() {
        let (master, pipes) = piped_master();
        let actor = start(master, |_| {});
        assert_error(
            actor.handle.release_after_commit(),
            io::ErrorKind::Other,
            "the PTY actor must be quiesced before its release",
        );
        assert_error(
            // SAFETY: refused before any console handle is read.
            unsafe { actor.handle.duplicate_for_handoff() },
            io::ErrorKind::Other,
            "the PTY actor must be quiesced before its handoff duplicate",
        );
        // nothing is held while running
        actor.handle.deliver_held();
        pipes.feed(b"still read");
        assert_eq!(actor.received(10), b"still read");
        actor.handle.write_input(b"still written").expect("input is taken");
        assert_eq!(pipes.written(13), b"still written");
    }

    /// A deadline that passes before the reader parks fails the handoff, which rolls back: input is taken and output
    /// delivered again.
    #[test]
    fn a_reader_not_parked_in_time_fails_the_handoff_and_rolls_it_back() {
        let (master, pipes) = piped_master();
        let actor = start(master, |_| {});
        assert_error(
            actor.handle.begin_handoff(Duration::ZERO),
            io::ErrorKind::TimedOut,
            "timed out waiting for the PTY actor to park its reader",
        );
        actor.handle.write_input(b"in").expect("input is taken again");
        assert_eq!(pipes.written(2), b"in");
        pipes.feed(b"out");
        assert_eq!(actor.received(3), b"out");
    }

    /// Input the console does not read holds the writer back past the deadline: the handoff fails and rolls back, and
    /// what was queued is written in order with what comes after.
    #[test]
    fn a_queue_not_written_in_time_fails_the_handoff_and_rolls_it_back() {
        let (master, pipes) = piped_master();
        let actor = start(master, |config| config.input_limit = 2 * BIG);
        actor.handle.write_input(&vec![b'q'; BIG]).expect("the queue takes it");
        assert_error(
            actor.handle.begin_handoff(Duration::from_secs(1)),
            io::ErrorKind::TimedOut,
            "timed out waiting for the PTY actor to write its queue",
        );
        actor.handle.write_input(b"after").expect("input is taken again");
        let written = pipes.written(BIG + 5);
        assert!(
            written[..BIG].iter().all(|&byte| byte == b'q'),
            "the queue was not written whole"
        );
        assert_eq!(&written[BIG..], b"after");
    }

    /// The console's input pipe breaking under the writer while the queue drains fails the handoff: the actor takes
    /// no input any more, and what was queued is dropped.
    #[test]
    fn a_write_failing_while_the_queue_drains_fails_the_handoff() {
        let (master, Pipes { input, output }) = piped_master();
        let actor = start(master, |config| config.input_limit = 2 * BIG);
        actor.handle.write_input(&vec![b'q'; BIG]).expect("the queue takes it");
        // still queued behind the blocked write when it fails
        actor.handle.write_input(b"next").expect("the queue takes it");
        let handle = actor.handle.clone();
        let handoff = std::thread::spawn(move || handle.begin_handoff(PATIENCE));
        wait_until("the handoff refuses input", || {
            actor.handle.write_input(b"") == Err(WriteError::Closed)
        });
        drop(input);
        let result = handoff.join().expect("the handoff thread");
        assert_error(result, io::ErrorKind::BrokenPipe, STOPPED);
        assert_eq!(actor.handle.queued(), 0, "the queue was kept");
        assert_eq!(actor.handle.write_input(b"x"), Err(WriteError::Closed));
        drop(output);
    }

    /// A read that completes while the actor quiesces is held back, not delivered; `deliver_held` hands it on and the
    /// actor stays quiesced, with nothing left for its release.
    #[test]
    fn a_read_completed_while_quiescing_is_held_until_delivered() {
        let (master, pipes) = piped_master();
        let actor = start(master, |_| {});
        quiesce_mid_read(&actor.handle);
        pipes.feed(b"held back");
        wait_for_held(&actor.handle, 9);
        assert!(actor.delivered.try_recv().is_err(), "delivered while quiesced");
        actor.handle.deliver_held();
        assert_eq!(actor.received(9), b"held back");
        assert_eq!(
            actor.handle.write_input(b"x"),
            Err(WriteError::Closed),
            "still quiesced"
        );
        // Keep the original console owner for cleanup: a release otherwise forgets it for the next owner.
        let _console = lock(&actor.handle.inner().conpty).take().expect("the console");
        let released = actor.handle.release_after_commit().expect("the release");
        assert_eq!(released.pending_output, b"", "the delivered read is pending again");
        assert_eq!(actor.stopped(), ExitReason::Released);
    }

    #[test]
    fn a_rollback_delivers_the_held_read_before_it_reads_again() {
        let (master, pipes) = piped_master();
        let actor = start(master, |_| {});
        quiesce_mid_read(&actor.handle);
        pipes.feed(b"first");
        wait_for_held(&actor.handle, 5);
        actor.handle.rollback_handoff().expect("the rollback");
        pipes.feed(b"second");
        assert_eq!(actor.received(11), b"firstsecond");
        actor.handle.write_input(b"in").expect("input is taken again");
        assert_eq!(pipes.written(2), b"in");
    }

    #[test]
    fn a_release_hands_the_held_read_on() {
        let (master, pipes) = piped_master();
        let actor = start(master, |_| {});
        quiesce_mid_read(&actor.handle);
        pipes.feed(b"pending");
        wait_for_held(&actor.handle, 7);
        // Keep the original console owner for cleanup: a release otherwise forgets it for the next owner.
        let _console = lock(&actor.handle.inner().conpty).take().expect("the console");
        let released = actor.handle.release_after_commit().expect("the release");
        assert_eq!(released.pending_output, b"pending");
        assert_eq!(actor.stopped(), ExitReason::Released);
        assert!(actor.delivered.try_recv().is_err(), "the held read was delivered too");
    }

    #[test]
    fn a_failed_read_stops_the_actor_as_failed() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (master, _input) = unreadable_master(&dir);
        let actor = start(master, |_| {});
        assert_eq!(actor.stopped(), ExitReason::Failed);
        assert_eq!(actor.handle.write_input(b"x"), Err(WriteError::Closed));
        assert_error(actor.handle.begin_handoff(PATIENCE), io::ErrorKind::BrokenPipe, STOPPED);
    }

    /// A shutdown reads despite a read pause, so the console's close never waits on its pipe; a read failing then is
    /// part of the shutdown, not a failure.
    #[test]
    fn a_read_failing_while_it_shuts_down_stops_the_actor_as_shut_down() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (master, _input) = unreadable_master(&dir);
        let pause = ReadPause::new();
        pause.pause();
        let actor = start(master, |config| config.read_pause = pause.clone());
        assert!(
            actor.exit.recv_timeout(Duration::from_millis(300)).is_err(),
            "read while paused"
        );
        actor.handle.shutdown();
        assert_eq!(actor.stopped(), ExitReason::Shutdown);
    }
}
