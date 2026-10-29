//! The presentation bridge to an attached row's origin as a child process (upstream's `RemotePresentationProcess.swift`):
//! the ssh running `live present` through the far side's `ngtermctl` (`remote::tree::REMOTE_CLI`). Stdout is read a line at a
//! time on a thread of its own and handed to the main loop, stdin takes the client's frames, and stderr goes to the
//! log, where an ssh failure is worth reading. The core decides everything else ([`ngterm_core::remote::client`]).
//!
//! The frontend's main-loop state takes the lines and the close ([`PresentationViewer`]); the network and resume
//! observer that retries the links stays with each frontend, since it rides on its platform's event sources.
//!
//! The OS forms ([`platform`]): on Unix the bridge runs through `/usr/bin/env`, its stdin is non-blocking (a write that
//! would block fails the link) and a stop sends it `SIGTERM` unless it was reaped. On Windows `ssh` is looked up on
//! `PATH` directly (Windows' own OpenSSH client in `System32\OpenSSH`, or the user's earlier on `PATH`), without a
//! console window; stdin is written from a thread of its own through a queue of [`MAX_PENDING_FRAMES`] lines, a full
//! one failing the link as a full Unix pipe does; and a stop ends the bridge with `TerminateProcess` through a handle
//! the link keeps open, so the pid it names is never another process's.

use std::cell::Cell;
use std::io::{BufRead, BufReader, Read};
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError, Weak};

use ngterm_core::remote::frames::{MAX_FRAME_BYTES, MAX_PENDING_FRAMES};
use ngterm_core::remote::{LinkKey, PresentationLink, PresentationTransport};
use ngterm_host::MainLoop;

/// The main-loop state a presentation bridge reports to: a line it read, and its end with the reason.
pub trait PresentationViewer: 'static {
    /// A line a presentation bridge read.
    fn viewer_line(&mut self, key: LinkKey, line: &[u8]);

    /// A presentation bridge ended.
    fn viewer_closed(&mut self, key: LinkKey, reason: &str);
}

/// Where a bridge's reader threads post: the main loop, held weakly so a reader outliving it posts nothing.
type Poster<S> = Weak<dyn MainLoop<S>>;

/// Bounds the lines a stream's reader may have waiting for the main loop: the reader blocks in
/// [`InboundGate::acquire`] past the limit until a task releases its place (upstream's inbound semaphore).
#[derive(Clone)]
pub struct InboundGate(Arc<(Mutex<usize>, Condvar)>);

impl InboundGate {
    #[must_use]
    pub fn new(limit: usize) -> Self {
        Self(Arc::new((Mutex::new(limit), Condvar::new())))
    }

    pub fn acquire(&self) {
        let (free, wake) = &*self.0;
        let mut free = free.lock().unwrap_or_else(PoisonError::into_inner);
        while *free == 0 {
            free = wake.wait(free).unwrap_or_else(PoisonError::into_inner);
        }
        *free -= 1;
    }

    pub fn release(&self) {
        let (free, wake) = &*self.0;
        *free.lock().unwrap_or_else(PoisonError::into_inner) += 1;
        wake.notify_one();
    }
}

/// Runs each bridge so a user's own `ssh` earlier on `PATH` wins: through `/usr/bin/env` on Unix, by name on Windows.
pub struct ProcessTransport<S> {
    poster: Poster<S>,
}

impl<S> ProcessTransport<S> {
    #[must_use]
    pub fn new(poster: Poster<S>) -> Self {
        Self { poster }
    }
}

impl<S: PresentationViewer> PresentationTransport for ProcessTransport<S> {
    fn open(&self, argv: &[String], key: LinkKey) -> Box<dyn PresentationLink> {
        Box::new(ProcessLink::spawn(argv, key, &self.poster))
    }
}

/// One child and its pipes.
struct ProcessLink {
    /// The child, for the stop; `None` when it never started.
    process: Option<platform::Process>,
    stdin: platform::Input,
    stopped: Cell<bool>,
    /// The child was reaped, so its pid may name another process now.
    reaped: Arc<AtomicBool>,
}

impl ProcessLink {
    fn spawn<S: PresentationViewer>(argv: &[String], key: LinkKey, poster: &Poster<S>) -> Self {
        let spawned = platform::command(argv).and_then(|mut command| {
            command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
        });
        let reaped = Arc::new(AtomicBool::new(false));
        let mut child = match spawned {
            Ok(child) => child,
            Err(error) => {
                let reason = format!(
                    "could not run {}: {error}",
                    argv.first().map_or("the bridge", String::as_str)
                );
                closed(poster, key, reason);
                return Self {
                    process: None,
                    stdin: platform::Input::none(),
                    stopped: Cell::new(true),
                    reaped,
                };
            }
        };
        let stdin = platform::Input::new(child.stdin.take());
        if let Some(stderr) = child.stderr.take() {
            let spawned = std::thread::Builder::new()
                .name("ngterm.presentation-stderr".to_owned())
                .spawn(move || log_lines(stderr));
            if spawned.is_err() {
                tracing::warn!("presentation bridge stderr is not read");
            }
        }
        let process = platform::Process::of(&child);
        let (posting, done) = (poster.clone(), Arc::clone(&reaped));
        let stdout = child.stdout.take();
        let spawned = std::thread::Builder::new()
            .name("ngterm.presentation-read".to_owned())
            .spawn(move || {
                if let Some(stdout) = stdout {
                    read_lines(stdout, key, &posting);
                }
                let reason = match child.wait() {
                    Ok(status) => exit_reason(status),
                    Err(error) => format!("wait failed: {error}"),
                };
                done.store(true, Ordering::SeqCst);
                closed(&posting, key, reason);
            });
        if spawned.is_err() {
            tracing::error!("presentation bridge cannot be read; ending it");
        }
        Self {
            process,
            stdin,
            stopped: Cell::new(false),
            reaped,
        }
    }
}

/// How the bridge ended: its exit code, or on Unix the signal that ended it.
fn exit_reason(status: ExitStatus) -> String {
    #[cfg(unix)]
    let signal = std::os::unix::process::ExitStatusExt::signal(&status);
    #[cfg(not(unix))]
    let signal: Option<i32> = None;
    match (status.code(), signal) {
        (Some(code), _) => format!("exit {code}"),
        (None, Some(signal)) => format!("signal {signal}"),
        (None, None) => "ended".to_owned(),
    }
}

impl PresentationLink for ProcessLink {
    fn send(&self, line: &[u8]) {
        if self.stopped.get() {
            return;
        }
        if !self.stdin.send(line) {
            tracing::info!("presentation bridge stopped taking input; ending it");
            self.stop();
        }
    }

    fn stop(&self) {
        if self.stopped.replace(true) {
            return;
        }
        self.stdin.close();
        if let Some(process) = &self.process
            && !self.reaped.load(Ordering::SeqCst)
        {
            process.stop();
        }
    }
}

impl Drop for ProcessLink {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(unix)]
mod platform {
    use std::cell::RefCell;
    use std::io::{self, Write};
    use std::os::fd::AsRawFd;
    use std::process::{Child, ChildStdin, Command};

    /// The bridge's command: `argv` through `/usr/bin/env`.
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the Windows form refuses an empty argv; both forms share the spawn's call"
    )]
    pub(super) fn command(argv: &[String]) -> io::Result<Command> {
        let mut command = Command::new("/usr/bin/env");
        command.args(argv);
        Ok(command)
    }

    /// The child's pid, for the stop's signal.
    pub(super) struct Process(i32);

    impl Process {
        pub(super) fn of(child: &Child) -> Option<Self> {
            i32::try_from(child.id()).ok().map(Self)
        }

        /// `SIGTERM`; the caller has checked the child is not reaped yet.
        pub(super) fn stop(&self) {
            // SAFETY: plain kill(2) of the bridge's own child, not yet reaped so its pid is still its own.
            unsafe { libc::kill(self.0, libc::SIGTERM) };
        }
    }

    /// The child's stdin, non-blocking: a frame is a few hundred bytes and the pipe holds 64 KiB, so a write that
    /// would block means the child stopped reading, which fails the link rather than stalling the main loop.
    pub(super) struct Input(RefCell<Option<ChildStdin>>);

    impl Input {
        pub(super) fn none() -> Self {
            Self(RefCell::new(None))
        }

        pub(super) fn new(stdin: Option<ChildStdin>) -> Self {
            if let Some(stdin) = stdin.as_ref() {
                let fd = stdin.as_raw_fd();
                // SAFETY: `fd` is the child's stdin, owned by `stdin` for this call; F_GETFL/F_SETFL touch only its
                // flags.
                let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
                // SAFETY: as above.
                unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
            }
            Self(RefCell::new(stdin))
        }

        /// Whether the whole line was written.
        pub(super) fn send(&self, line: &[u8]) -> bool {
            self.0
                .borrow_mut()
                .as_mut()
                .is_some_and(|stdin| stdin.write_all(line).is_ok())
        }

        pub(super) fn close(&self) {
            self.0.borrow_mut().take();
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::cell::RefCell;
    use std::io::{self, Write};
    use std::os::windows::io::{AsHandle, AsRawHandle, OwnedHandle};
    use std::os::windows::process::CommandExt;
    use std::process::{Child, ChildStdin, Command};
    use std::sync::mpsc::{self, SyncSender};

    use ngterm_core::remote::frames::MAX_PENDING_FRAMES;
    use windows_sys::Win32::System::Threading::{CREATE_NO_WINDOW, TerminateProcess};

    /// The bridge's command: `argv`'s program found on `PATH`, without a console window.
    pub(super) fn command(argv: &[String]) -> io::Result<Command> {
        let (program, arguments) = argv
            .split_first()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "an empty command"))?;
        let mut command = Command::new(program);
        command.args(arguments).creation_flags(CREATE_NO_WINDOW);
        Ok(command)
    }

    /// A handle on the child, kept open so the stop never reaches a process that took its pid.
    pub(super) struct Process(OwnedHandle);

    impl Process {
        pub(super) fn of(child: &Child) -> Option<Self> {
            match child.as_handle().try_clone_to_owned() {
                Ok(handle) => Some(Self(handle)),
                Err(error) => {
                    tracing::warn!(%error, "the presentation bridge cannot be stopped");
                    None
                }
            }
        }

        /// `TerminateProcess` with exit code 1; one already gone fails harmlessly.
        pub(super) fn stop(&self) {
            // SAFETY: the child's process handle, which `self` keeps open for the call.
            unsafe { TerminateProcess(self.0.as_raw_handle(), 1) };
        }
    }

    /// Writes each queued line to `stdin` until the queue closes (the stop) or the child stops reading.
    fn write_queued(mut stdin: ChildStdin, queued: &mpsc::Receiver<Vec<u8>>) {
        for line in queued {
            if stdin.write_all(&line).is_err() {
                return;
            }
        }
    }

    /// The child's stdin, written from a thread of its own through a queue of [`MAX_PENDING_FRAMES`] lines: a full
    /// queue means the child stopped reading, which fails the link rather than stalling the main loop.
    pub(super) struct Input(RefCell<Option<SyncSender<Vec<u8>>>>);

    impl Input {
        pub(super) fn none() -> Self {
            Self(RefCell::new(None))
        }

        pub(super) fn new(stdin: Option<ChildStdin>) -> Self {
            let Some(stdin) = stdin else {
                return Self::none();
            };
            let (lines, queued) = mpsc::sync_channel::<Vec<u8>>(MAX_PENDING_FRAMES);
            let spawned = std::thread::Builder::new()
                .name("ngterm.presentation-write".to_owned())
                .spawn(move || write_queued(stdin, &queued));
            match spawned {
                Ok(_) => Self(RefCell::new(Some(lines))),
                Err(error) => {
                    tracing::warn!(%error, "presentation bridge stdin is not written");
                    Self::none()
                }
            }
        }

        /// Whether the line was queued.
        pub(super) fn send(&self, line: &[u8]) -> bool {
            self.0
                .borrow()
                .as_ref()
                .is_some_and(|lines| lines.try_send(line.to_vec()).is_ok())
        }

        /// The queue closes: the writer drains it, then closes the child's stdin.
        pub(super) fn close(&self) {
            self.0.borrow_mut().take();
        }
    }
}

/// Reads stdout one line at a time until its end, handing each to the main loop; a line over the frame limit ends the
/// read, which the child sees as a closed pipe. The reader stops reading while [`MAX_PENDING_FRAMES`] lines wait for
/// the loop, so a busy loop backs the child up into its own pipe.
fn read_lines<S: PresentationViewer>(stdout: impl Read, key: LinkKey, poster: &Poster<S>) {
    let gate = InboundGate::new(MAX_PENDING_FRAMES);
    let mut reader = BufReader::new(stdout);
    loop {
        let mut line = Vec::new();
        let limit = u64::try_from(MAX_FRAME_BYTES + 1).unwrap_or(u64::MAX);
        match (&mut reader).take(limit).read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        if line.last() != Some(&b'\n') {
            tracing::warn!("presentation bridge sent an oversize or unterminated line; ending the read");
            return;
        }
        line.pop();
        gate.acquire();
        let released = gate.clone();
        let task = move |state: &mut S| {
            state.viewer_line(key, &line);
            released.release();
        };
        let delivered = poster
            .upgrade()
            .is_some_and(|main_loop| main_loop.post(Box::new(task)).is_ok());
        if !delivered {
            return;
        }
    }
}

fn log_lines(stderr: impl Read) {
    for line in BufReader::new(stderr).lines() {
        match line {
            Ok(line) => tracing::info!(line = %line, "presentation bridge"),
            Err(_) => return,
        }
    }
}

/// Reports the bridge's end to the main loop; a closed or gone loop drops it.
fn closed<S: PresentationViewer>(poster: &Poster<S>, key: LinkKey, reason: String) {
    let Some(main_loop) = poster.upgrade() else {
        return;
    };
    let task = move |state: &mut S| state.viewer_closed(key, &reason);
    if main_loop.post(Box::new(task)).is_err() {
        tracing::debug!("main-loop task dropped: the loop is closed");
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use ngterm_host::fakes::FakeMainLoop;
    use uuid::Uuid;

    use super::*;

    /// What the bridge reported to the loop, in order.
    #[derive(Debug, PartialEq, Eq)]
    enum Report {
        Line(String),
        Closed(String),
    }

    struct Viewer(mpsc::Sender<Report>);

    impl PresentationViewer for Viewer {
        fn viewer_line(&mut self, _key: LinkKey, line: &[u8]) {
            self.0
                .send(Report::Line(String::from_utf8_lossy(line).into_owned()))
                .unwrap();
        }

        fn viewer_closed(&mut self, _key: LinkKey, reason: &str) {
            self.0.send(Report::Closed(reason.to_owned())).unwrap();
        }
    }

    const KEY: LinkKey = LinkKey {
        session: Uuid::nil(),
        client: 1,
        launch: 1,
    };

    /// Opens a bridge running `unix` under `/bin/sh` (`windows` under PowerShell on Windows) and collects its reports
    /// until it closes.
    fn reports_of(unix: &str, windows: &str) -> Vec<Report> {
        let (sender, receiver) = mpsc::channel();
        let main_loop = Arc::new(FakeMainLoop::spawn(Viewer(sender)).unwrap());
        let poster: Poster<Viewer> = Arc::downgrade(&main_loop) as Poster<Viewer>;
        let transport = ProcessTransport::new(poster);
        let argv = crate::test_shell::script_argv(unix, windows);
        let link = transport.open(&argv, KEY);
        let mut reports = Vec::new();
        loop {
            let report = receiver.recv_timeout(Duration::from_secs(10)).unwrap();
            let closed = matches!(report, Report::Closed(_));
            reports.push(report);
            if closed {
                break;
            }
        }
        drop(link);
        main_loop.close();
        reports
    }

    #[test]
    fn the_bridge_hands_each_line_to_the_loop_then_its_exit() {
        assert_eq!(
            reports_of("printf 'one\\ntwo\\n'", "[Console]::Out.Write(\"one`ntwo`n\")"),
            [
                Report::Line("one".to_owned()),
                Report::Line("two".to_owned()),
                Report::Closed("exit 0".to_owned()),
            ]
        );
    }

    #[test]
    fn an_unterminated_line_is_not_handed_on() {
        assert_eq!(
            reports_of(
                "printf 'whole\\npartial'; exit 3",
                "[Console]::Out.Write(\"whole`npartial\"); exit 3"
            ),
            [Report::Line("whole".to_owned()), Report::Closed("exit 3".to_owned())]
        );
    }

    /// A stop ends a bridge still running: `SIGTERM` on Unix, `TerminateProcess` (exit 1) on Windows. A line sent
    /// before it reaches the bridge's stdin, which echoes it.
    #[test]
    fn a_line_reaches_the_bridge_and_a_stop_ends_it() {
        let (sender, receiver) = mpsc::channel();
        let main_loop = Arc::new(FakeMainLoop::spawn(Viewer(sender)).unwrap());
        let poster: Poster<Viewer> = Arc::downgrade(&main_loop) as Poster<Viewer>;
        let argv = crate::test_shell::script_argv(
            "read line; printf '%s\\n' \"got $line\"; exec sleep 30",
            "$line = [Console]::In.ReadLine(); [Console]::Out.Write(\"got $line`n\"); Start-Sleep 30",
        );
        let link = ProcessTransport::new(poster).open(&argv, KEY);
        link.send(b"hello\n");
        let echoed = receiver.recv_timeout(Duration::from_secs(20)).unwrap();
        assert_eq!(echoed, Report::Line("got hello".to_owned()));
        link.stop();
        let closed = receiver.recv_timeout(Duration::from_secs(10)).unwrap();
        let expected = if cfg!(windows) { "exit 1" } else { "signal 15" };
        assert_eq!(closed, Report::Closed(expected.to_owned()));
        drop(link);
        main_loop.close();
    }

    #[test]
    fn a_bridge_that_cannot_start_reports_why() {
        let (sender, receiver) = mpsc::channel();
        let main_loop = Arc::new(FakeMainLoop::spawn(Viewer(sender)).unwrap());
        let poster: Poster<Viewer> = Arc::downgrade(&main_loop) as Poster<Viewer>;
        let argv = ["ngterm-test-no-such-ssh".to_owned()];
        let link = ProcessTransport::new(poster).open(&argv, KEY);
        let report = receiver.recv_timeout(Duration::from_secs(10)).unwrap();
        // `/usr/bin/env` starts on Unix and exits 127 for the missing program; Windows refuses the spawn itself
        let ok = match &report {
            Report::Closed(reason) if cfg!(windows) => reason.starts_with("could not run ngterm-test-no-such-ssh"),
            Report::Closed(reason) => reason == "exit 127",
            Report::Line(_) => false,
        };
        assert!(ok, "{report:?}");
        drop(link);
        main_loop.close();
    }

    #[test]
    fn the_inbound_gate_holds_the_reader_past_its_limit_until_a_line_is_handled() {
        let gate = InboundGate::new(2);
        gate.acquire();
        gate.acquire();
        let (done, finished) = std::sync::mpsc::channel();
        let reader = gate.clone();
        let blocked = std::thread::spawn(move || {
            reader.acquire();
            done.send(()).unwrap();
        });
        assert!(
            finished.recv_timeout(Duration::from_millis(100)).is_err(),
            "a third line waits"
        );
        gate.release();
        finished.recv_timeout(Duration::from_secs(5)).unwrap();
        blocked.join().unwrap();
    }
}
