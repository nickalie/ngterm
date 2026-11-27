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
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant};

use ngterm_core::remote::frames::{MAX_FRAME_BYTES, MAX_PENDING_FRAMES};
use ngterm_core::remote::{LinkKey, PresentationLink, PresentationTransport};
use ngterm_host::MainLoop;
use uuid::Uuid;

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
    stderr_runs: BridgeRuns,
}

impl<S> ProcessTransport<S> {
    #[must_use]
    pub fn new(poster: Poster<S>) -> Self {
        Self {
            poster,
            stderr_runs: BridgeRuns::default(),
        }
    }
}

impl<S: PresentationViewer> PresentationTransport for ProcessTransport<S> {
    fn open(&self, argv: &[String], key: LinkKey) -> Box<dyn PresentationLink> {
        Box::new(ProcessLink::spawn(argv, key, &self.poster, &self.stderr_runs))
    }

    fn shutdown(&self) {
        flush_bridge_runs(&self.stderr_runs);
    }
}

impl<S> Drop for ProcessTransport<S> {
    fn drop(&mut self) {
        flush_bridge_runs(&self.stderr_runs);
    }
}

fn flush_bridge_runs(runs: &BridgeRuns) {
    let mut runs = runs.lock().unwrap_or_else(PoisonError::into_inner);
    for (_, mut run) in runs.drain() {
        run.flush(BRIDGE_SOURCE);
    }
}

/// The stderr run of each row's bridge, kept across its launches: a bridge that cannot connect writes one line and
/// exits, and each retry is a new process.
type BridgeRuns = Arc<Mutex<HashMap<Uuid, Repeats>>>;

const BRIDGE_SOURCE: &str = "presentation bridge";

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
    fn spawn<S: PresentationViewer>(
        argv: &[String],
        key: LinkKey,
        poster: &Poster<S>,
        stderr_runs: &BridgeRuns,
    ) -> Self {
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
            let runs = Arc::clone(stderr_runs);
            let spawned = std::thread::Builder::new()
                .name("ngterm.presentation-stderr".to_owned())
                .spawn(move || log_bridge_lines_at(stderr, &runs, key.session, Instant::now));
            if spawned.is_err() {
                tracing::warn!("presentation bridge stderr is not read");
            }
        }
        let process = platform::Process::of(&child);
        let (posting, done, runs) = (poster.clone(), Arc::clone(&reaped), Arc::clone(stderr_runs));
        let stdout = child.stdout.take();
        let spawned = std::thread::Builder::new()
            .name("ngterm.presentation-read".to_owned())
            .spawn(move || {
                if let Some(stdout) = stdout {
                    read_lines(stdout, key, &posting, || end_bridge_run(&runs, key.session));
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

    /// The bridge's command: `argv` through `/usr/bin/env` ([`ngterm_core::remote::tree::local_command`]).
    pub(super) fn command(argv: &[String]) -> io::Result<Command> {
        ngterm_core::remote::tree::local_command(argv)
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
    use std::process::{Child, ChildStdin, Command};
    use std::sync::mpsc::{self, SyncSender};

    use ngterm_core::remote::frames::MAX_PENDING_FRAMES;
    use windows_sys::Win32::System::Threading::TerminateProcess;

    /// The bridge's command: `argv`'s program found on `PATH`, without a console window
    /// ([`ngterm_core::remote::tree::local_command`]).
    pub(super) fn command(argv: &[String]) -> io::Result<Command> {
        ngterm_core::remote::tree::local_command(argv)
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
fn read_lines<S: PresentationViewer>(stdout: impl Read, key: LinkKey, poster: &Poster<S>, first_line: impl FnOnce()) {
    let mut first_line = Some(first_line);
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
        if let Some(first_line) = first_line.take() {
            first_line();
        }
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

/// Drains a child's stderr to its end, logging each line lossily as upstream does, so non-UTF-8 bytes don't stop it.
/// A line holds at most the frame limit, as upstream's reader caps it; a longer one is logged in pieces of the read
/// limit and the drain keeps going, so the child never blocks on or loses its stderr. A run of identical lines logs
/// its first line and then its count ([`Repeats`]), as ssh repeats one error for each refused try for hours.
pub(crate) fn log_lines(stderr: impl Read, source: &str) {
    log_lines_at(stderr, source, Instant::now);
}

fn log_lines_at(stderr: impl Read, source: &str, mut now: impl FnMut() -> Instant) {
    let mut repeats = Repeats::default();
    read_stderr(stderr, |line| repeats.line(line, now(), source));
    repeats.flush(source);
}

/// [`log_lines`] for a row's bridge, on the row's run in `runs`. The end of the stream does not log the count, as the
/// next launch can continue the run.
fn log_bridge_lines_at(stderr: impl Read, runs: &BridgeRuns, row: Uuid, mut now: impl FnMut() -> Instant) {
    read_stderr(stderr, |line| {
        let mut runs = runs.lock().unwrap_or_else(PoisonError::into_inner);
        runs.entry(row).or_default().line(line, now(), BRIDGE_SOURCE);
    });
}

/// Logs and forgets the row's run: its bridge answered, so the run cannot go on.
fn end_bridge_run(runs: &BridgeRuns, row: Uuid) {
    let run = runs.lock().unwrap_or_else(PoisonError::into_inner).remove(&row);
    if let Some(mut run) = run {
        run.flush(BRIDGE_SOURCE);
    }
}

fn read_stderr(stderr: impl Read, mut each: impl FnMut(&str)) {
    let mut reader = BufReader::new(stderr);
    let mut line = Vec::new();
    let limit = u64::try_from(MAX_FRAME_BYTES + 1).unwrap_or(u64::MAX);
    loop {
        line.clear();
        match (&mut reader).take(limit).read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        if line.last() == Some(&b'\n') {
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
        }
        each(&String::from_utf8_lossy(&line));
    }
}

/// The first wait before a run of identical stderr lines logs its count while it goes on; the wait doubles after
/// each count, up to [`MAX_REPEAT_REPORT_INTERVAL`], so a bridge retried every five minutes logs about once an hour.
const REPEAT_REPORT_INTERVAL: Duration = Duration::from_secs(60);
const MAX_REPEAT_REPORT_INTERVAL: Duration = Duration::from_hours(1);

/// The run of identical stderr lines that [`log_lines`] reads: the line, how many copies followed it since the last
/// log event, and when that event was. The count is logged with a different line, or with the first copy that comes
/// `interval` or more after the last log event.
struct Repeats {
    last: Option<String>,
    count: u64,
    since: Option<Instant>,
    interval: Duration,
}

impl Default for Repeats {
    fn default() -> Self {
        Self {
            last: None,
            count: 0,
            since: None,
            interval: REPEAT_REPORT_INTERVAL,
        }
    }
}

impl Repeats {
    fn line(&mut self, text: &str, now: Instant, source: &str) {
        if self.last.as_deref() != Some(text) {
            self.flush(source);
            tracing::info!(line = %text, "{source}");
            self.last = Some(text.to_owned());
            self.since = Some(now);
            self.interval = REPEAT_REPORT_INTERVAL;
            return;
        }
        self.count += 1;
        if self
            .since
            .is_some_and(|since| now.duration_since(since) >= self.interval)
        {
            self.flush(source);
            self.since = Some(now);
            self.interval = (self.interval * 2).min(MAX_REPEAT_REPORT_INTERVAL);
        }
    }

    fn flush(&mut self, source: &str) {
        if self.count > 0 {
            let last = self.last.as_deref().unwrap_or_default();
            tracing::info!(line = %last, repeats = self.count, "{source}: the last line repeated");
            self.count = 0;
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
            let report = receiver.recv_timeout(crate::test_shell::CHILD_DEADLINE).unwrap();
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

    #[cfg(unix)]
    #[test]
    fn a_non_utf8_stderr_line_does_not_stop_the_stderr_drain() {
        assert_eq!(
            reports_of(
                "printf '\\377\\n' >&2; trap '' PIPE; i=0; while [ $i -lt 100000 ]; do echo x >&2 || { echo broken; exit 1; }; i=$((i+1)); done; echo done",
                "",
            ),
            [Report::Line("done".to_owned()), Report::Closed("exit 0".to_owned())]
        );
    }

    /// What [`log_lines`] logs for `stderr`.
    fn stderr_log(stderr: &[u8]) -> String {
        stderr_log_at(stderr, Instant::now)
    }

    /// What [`log_lines`] logs for `stderr` when `now` is its clock.
    fn stderr_log_at(stderr: &[u8], now: impl FnMut() -> Instant) -> String {
        logged(|| log_lines_at(stderr, "test child", now))
    }

    /// What `log_events` logs on this thread.
    fn logged(log_events: impl FnOnce()) -> String {
        let log = tempfile::NamedTempFile::new().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_target(false)
            .with_writer(Mutex::new(log.reopen().unwrap()))
            .finish();
        tracing::subscriber::with_default(subscriber, log_events);
        std::fs::read_to_string(log.path()).unwrap()
    }

    #[test]
    fn a_run_of_identical_bridge_stderr_lines_goes_on_across_launches_of_the_row() {
        let runs = BridgeRuns::default();
        let (row, other_row) = (Uuid::nil(), Uuid::max());
        let text = logged(|| {
            for _ in 0..3 {
                log_bridge_lines_at(&b"refused\n"[..], &runs, row, Instant::now);
            }
            log_bridge_lines_at(&b"refused\n"[..], &runs, other_row, Instant::now);
            log_bridge_lines_at(&b"connected\n"[..], &runs, row, Instant::now);
        });
        assert_eq!(
            text.lines().collect::<Vec<_>>(),
            [
                " INFO presentation bridge line=refused",
                " INFO presentation bridge line=refused",
                " INFO presentation bridge: the last line repeated line=refused repeats=2",
                " INFO presentation bridge line=connected",
            ]
        );
    }

    #[test]
    fn stderr_is_logged_lossily_through_its_unterminated_last_line() {
        let text = stderr_log(b"plain\nbad \xff\r\nlast");
        assert!(text.contains("test child line=plain\n"), "{text:?}");
        assert!(text.contains("test child line=bad \u{fffd}\n"), "{text:?}");
        assert!(text.contains("test child line=last\n"), "{text:?}");
    }

    /// A clock that moves `step` forward on each read.
    fn ticking(step: Duration) -> impl FnMut() -> Instant {
        let mut now = Instant::now();
        move || {
            now += step;
            now
        }
    }

    #[test]
    fn a_run_of_identical_stderr_lines_logs_its_first_line_and_its_count() {
        let mut stderr = b"channel 2: open failed\n".repeat(1000);
        stderr.extend_from_slice(b"other\n");
        let text = stderr_log(&stderr);
        assert_eq!(
            text.lines().collect::<Vec<_>>(),
            [
                " INFO test child line=channel 2: open failed",
                " INFO test child: the last line repeated line=channel 2: open failed repeats=999",
                " INFO test child line=other",
            ]
        );
    }

    #[test]
    fn a_run_of_identical_stderr_lines_logs_its_count_when_the_stream_ends() {
        let text = stderr_log(b"refused\nrefused\r\nrefused");
        assert_eq!(
            text.lines().collect::<Vec<_>>(),
            [
                " INFO test child line=refused",
                " INFO test child: the last line repeated line=refused repeats=2"
            ]
        );
    }

    #[test]
    fn a_long_run_of_identical_stderr_lines_logs_its_count_after_a_minute_then_at_doubling_waits() {
        let text = stderr_log_at(&b"refused\n".repeat(400), ticking(Duration::from_secs(1)));
        assert_eq!(
            text.lines().collect::<Vec<_>>(),
            [
                " INFO test child line=refused",
                " INFO test child: the last line repeated line=refused repeats=60",
                " INFO test child: the last line repeated line=refused repeats=120",
                " INFO test child: the last line repeated line=refused repeats=219",
            ]
        );
    }

    #[test]
    fn a_bridge_retried_every_five_minutes_logs_its_count_less_and_less_often_then_each_hour() {
        let runs = BridgeRuns::default();
        let mut clock = ticking(Duration::from_secs(300));
        let text = logged(|| {
            for _ in 0..53 {
                log_bridge_lines_at(&b"refused\n"[..], &runs, Uuid::nil(), &mut clock);
            }
        });
        let counts: Vec<&str> = text
            .lines()
            .filter_map(|line| line.split_once("repeats=").map(|(_, count)| count))
            .collect();
        assert_eq!(counts, ["1", "1", "1", "2", "4", "7", "12", "12", "12"]);
        assert_eq!(text.lines().count(), 10, "{text}");
    }

    #[test]
    fn a_new_line_starts_its_run_with_the_first_wait_again() {
        let mut stderr = b"refused\n".repeat(200);
        stderr.extend_from_slice(&b"timeout\n".repeat(62));
        let text = stderr_log_at(&stderr, ticking(Duration::from_secs(1)));
        assert_eq!(
            text.lines().collect::<Vec<_>>(),
            [
                " INFO test child line=refused",
                " INFO test child: the last line repeated line=refused repeats=60",
                " INFO test child: the last line repeated line=refused repeats=120",
                " INFO test child: the last line repeated line=refused repeats=19",
                " INFO test child line=timeout",
                " INFO test child: the last line repeated line=timeout repeats=60",
                " INFO test child: the last line repeated line=timeout repeats=1",
            ]
        );
    }

    #[test]
    fn a_row_whose_bridge_answers_logs_its_count_and_forgets_its_run() {
        let runs = BridgeRuns::default();
        let text = logged(|| {
            for _ in 0..3 {
                log_bridge_lines_at(&b"refused\n"[..], &runs, Uuid::nil(), Instant::now);
            }
            end_bridge_run(&runs, Uuid::nil());
            log_bridge_lines_at(&b"refused\n"[..], &runs, Uuid::nil(), Instant::now);
        });
        assert_eq!(
            text.lines().collect::<Vec<_>>(),
            [
                " INFO presentation bridge line=refused",
                " INFO presentation bridge: the last line repeated line=refused repeats=2",
                " INFO presentation bridge line=refused",
            ]
        );
    }

    #[test]
    fn a_shut_down_transport_logs_the_count_of_each_row_and_forgets_its_runs() {
        let transport = ProcessTransport::new(Weak::<FakeMainLoop<Viewer>>::new() as Poster<Viewer>);
        let text = logged(|| {
            for row in [Uuid::nil(), Uuid::max()] {
                for _ in 0..2 {
                    log_bridge_lines_at(&b"refused\n"[..], &transport.stderr_runs, row, Instant::now);
                }
            }
            transport.shutdown();
            drop(transport);
        });
        assert_eq!(
            text.lines().filter(|line| line.ends_with("repeats=1")).count(),
            2,
            "{text}"
        );
    }

    #[test]
    fn a_dropped_transport_logs_the_count_of_each_row() {
        let transport = ProcessTransport::new(Weak::<FakeMainLoop<Viewer>>::new() as Poster<Viewer>);
        let text = logged(|| {
            for row in [Uuid::nil(), Uuid::max()] {
                for _ in 0..2 {
                    log_bridge_lines_at(&b"refused\n"[..], &transport.stderr_runs, row, Instant::now);
                }
            }
            drop(transport);
        });
        assert_eq!(
            text.lines().filter(|line| line.ends_with("repeats=1")).count(),
            2,
            "{text}"
        );
    }

    /// Waits until the run of `KEY`'s row in `transport` holds `count` copies, or `None` for no run.
    fn wait_for_run<S>(transport: &ProcessTransport<S>, count: Option<u64>) {
        let deadline = std::time::Instant::now() + crate::test_shell::CHILD_DEADLINE;
        loop {
            let now = transport
                .stderr_runs
                .lock()
                .unwrap()
                .get(&KEY.session)
                .map(|run| run.count);
            if now == count {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the run holds {now:?}, not {count:?}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn the_launches_of_a_row_share_its_run_until_its_bridge_answers() {
        let (sender, receiver) = mpsc::channel();
        let main_loop = Arc::new(FakeMainLoop::spawn(Viewer(sender)).unwrap());
        let poster: Poster<Viewer> = Arc::downgrade(&main_loop) as Poster<Viewer>;
        let transport = ProcessTransport::new(poster);
        let refused = crate::test_shell::script_argv("echo refused >&2", "[Console]::Error.WriteLine('refused')");
        let answers = crate::test_shell::script_argv("echo up", "[Console]::Out.WriteLine('up')");
        for (launch, argv, run) in [(1, &refused, Some(0)), (2, &refused, Some(1)), (3, &answers, None)] {
            let link = transport.open(argv, LinkKey { launch, ..KEY });
            while !matches!(
                receiver.recv_timeout(crate::test_shell::CHILD_DEADLINE).unwrap(),
                Report::Closed(_)
            ) {}
            drop(link);
            wait_for_run(&transport, run);
        }
        main_loop.close();
    }

    #[test]
    fn a_stderr_line_over_the_frame_limit_is_logged_in_pieces_and_the_drain_goes_on() {
        let mut stderr = vec![b'x'; MAX_FRAME_BYTES + 2];
        stderr.extend_from_slice(b"\nnext\n");
        let text = stderr_log(&stderr);
        let logged: Vec<&str> = text
            .lines()
            .map(|line| line.split_once("test child line=").unwrap().1)
            .collect();
        assert_eq!(
            logged.iter().map(|line| line.len()).collect::<Vec<_>>(),
            [MAX_FRAME_BYTES + 1, 1, 4]
        );
        assert_eq!(logged[2], "next");
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
        let echoed = receiver.recv_timeout(crate::test_shell::CHILD_DEADLINE).unwrap();
        assert_eq!(echoed, Report::Line("got hello".to_owned()));
        link.stop();
        let closed = receiver.recv_timeout(crate::test_shell::CHILD_DEADLINE).unwrap();
        let expected = if cfg!(windows) { "exit 1" } else { "signal 15" };
        assert_eq!(closed, Report::Closed(expected.to_owned()));
        drop(link);
        main_loop.close();
    }

    /// An argv the OS refuses outright starts no bridge: the close says why, and the link takes no lines.
    #[test]
    fn a_bridge_the_os_refuses_reports_why_and_takes_no_lines() {
        let (sender, receiver) = mpsc::channel();
        let main_loop = Arc::new(FakeMainLoop::spawn(Viewer(sender)).unwrap());
        let poster: Poster<Viewer> = Arc::downgrade(&main_loop) as Poster<Viewer>;
        let argv = ["ss\0h".to_owned()];
        let link = ProcessTransport::new(poster).open(&argv, KEY);
        let report = receiver.recv_timeout(crate::test_shell::CHILD_DEADLINE).unwrap();
        assert!(
            matches!(&report, Report::Closed(reason) if reason.starts_with("could not run ss\0h: ")),
            "{report:?}"
        );
        link.send(b"{}\n");
        link.stop();
        drop(link);
        main_loop.close();
        assert!(receiver.recv().is_err(), "nothing more is reported");
    }

    #[test]
    fn a_bridge_that_cannot_start_reports_why() {
        let (sender, receiver) = mpsc::channel();
        let main_loop = Arc::new(FakeMainLoop::spawn(Viewer(sender)).unwrap());
        let poster: Poster<Viewer> = Arc::downgrade(&main_loop) as Poster<Viewer>;
        let argv = ["ngterm-test-no-such-ssh".to_owned()];
        let link = ProcessTransport::new(poster).open(&argv, KEY);
        let report = receiver.recv_timeout(crate::test_shell::CHILD_DEADLINE).unwrap();
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

    /// A bridge that stopped reading its stdin fails the next send, which ends it; a send after that is dropped.
    #[cfg(unix)]
    #[test]
    fn a_bridge_that_stopped_reading_is_ended_by_the_next_send() {
        let (sender, receiver) = mpsc::channel();
        let main_loop = Arc::new(FakeMainLoop::spawn(Viewer(sender)).unwrap());
        let poster: Poster<Viewer> = Arc::downgrade(&main_loop) as Poster<Viewer>;
        let argv = crate::test_shell::script_argv("exec 0<&-; echo ready; exec sleep 30", "");
        let link = ProcessTransport::new(poster).open(&argv, KEY);
        let ready = receiver.recv_timeout(crate::test_shell::CHILD_DEADLINE).unwrap();
        assert_eq!(ready, Report::Line("ready".to_owned()));
        link.send(b"{}\n");
        let closed = receiver.recv_timeout(crate::test_shell::CHILD_DEADLINE).unwrap();
        assert_eq!(closed, Report::Closed("signal 15".to_owned()));
        link.send(b"{}\n");
        link.stop();
        drop(link);
        main_loop.close();
        assert!(receiver.recv().is_err(), "nothing more is reported");
    }

    /// A line longer than a frame ends the read without handing any of it on, nor a line after it; the bridge sees its
    /// stdout closed.
    #[cfg(unix)]
    #[test]
    fn an_oversize_line_ends_the_read() {
        let reports = reports_of(
            &format!(
                "head -c {} /dev/zero | tr '\\0' x; echo; echo later",
                MAX_FRAME_BYTES + 2
            ),
            "",
        );
        assert!(matches!(&reports[..], [Report::Closed(_)]), "{reports:?}");
    }

    /// A bridge whose main loop is closed or gone stops reading at its first line and posts nothing; it then ends on
    /// its broken stdout and is reaped, and a stop after that signals nothing.
    #[cfg(unix)]
    #[test]
    fn a_bridge_whose_loop_is_closed_or_gone_stops_reading_and_is_reaped() {
        let (sender, receiver) = mpsc::channel();
        let closed_loop = Arc::new(FakeMainLoop::spawn(Viewer(sender)).unwrap());
        closed_loop.close();
        let closed: Poster<Viewer> = Arc::downgrade(&closed_loop) as Poster<Viewer>;
        let gone: Poster<Viewer> = Weak::<FakeMainLoop<Viewer>>::new();
        let argv = crate::test_shell::script_argv("while :; do echo x || exit 4; done", "");
        for poster in [closed, gone] {
            let link = ProcessLink::spawn(&argv, KEY, &poster, &BridgeRuns::default());
            let deadline = std::time::Instant::now() + crate::test_shell::CHILD_DEADLINE;
            while !link.reaped.load(Ordering::SeqCst) {
                assert!(std::time::Instant::now() < deadline, "the bridge was never reaped");
                std::thread::sleep(Duration::from_millis(20));
            }
            link.stop();
        }
        drop(receiver);
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
