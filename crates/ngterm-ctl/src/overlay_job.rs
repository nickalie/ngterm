//! `ngtermctl session overlay run-job <job>` (`agtermctlKit/OverlayRunJob.swift`): the helper that supervises one
//! remote overlay job on the origin. It claims the job over the app's socket (`session.overlay.job.run`), runs the
//! program under the pty it inherited from the viewer's ssh, reports how it ended, and stops it when cancelled.
//! Everything that decides the outcome is [`JobRunner`]; the command only wires signals.
//!
//! The frames after the claim's ordinary reply are [`OverlayJobFrame`] lines: the app sends `context`, then maybe
//! `cancel`; the helper reports `started`, then one of `exited`, `canceled` or `launch-failed`.
//!
//! What runs the program is per OS ([`sys`]). On Linux and macOS the program leads a process group of its own, the
//! terminal's foreground, which a cancel signals as a whole (SIGTERM, SIGKILL past the grace). On Windows its whole
//! process tree sits in a Job object, killed when the helper's handle to it closes; the grace stop is a console
//! Ctrl+Break to the program's own process group and the kill ends the job.

use std::ffi::OsString;
use std::net::Shutdown;
use std::process::ExitCode;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use ngterm_core::overlay_capture::OverlayLaunchContext;
use ngterm_core::overlay_jobs::OverlayJobFrame;
use ngterm_core::paths::Env;
use ngterm_proto::command::SessionCmd;
use ngterm_proto::{Command, MAX_REQUEST_LINE_BYTES, Request, Response};

use crate::client::{self, Connection, os_message};
use crate::{BasicOptions, Failure, RequestCommand};

#[cfg(unix)]
#[path = "overlay_job/unix.rs"]
mod sys;
#[cfg(windows)]
#[path = "overlay_job/windows.rs"]
mod sys;

/// How long a cancelled program gets to exit before it is killed.
pub const DEFAULT_GRACE: Duration = Duration::from_secs(3);
/// Cap on a frame line (upstream's `PresentationCodec.maxFrameBytes`).
const MAX_FRAME_BYTES: usize = 256 * 1024;
/// The helper's status when the program could not be launched.
const LAUNCH_FAILED_STATUS: i32 = 127;
/// How often the supervisor looks at the program, the terminal and the cancel deadline.
const SUPERVISE_TICK: Duration = Duration::from_millis(20);
/// The program's process group and the deadline a cancel set, shared with the threads that cancel.
#[derive(Debug, Default)]
struct Stop {
    /// The program's process group; `None` before it started.
    group: Option<sys::Group>,
    /// When a cancelled program is killed; `None` until cancelled.
    kill_at: Option<Instant>,
}

#[derive(Debug)]
struct Shared {
    grace: Duration,
    stop: Mutex<Stop>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Stop> {
        self.stop.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The grace stop to the program's group now (SIGTERM, a Ctrl+Break on Windows), the kill once the grace has
    /// passed; before the program started, it is stopped as soon as it does.
    fn cancel(&self) {
        let group = {
            let mut stop = self.lock();
            if stop.kill_at.is_none() {
                stop.kill_at = Some(Instant::now() + self.grace);
            }
            stop.group
        };
        if let Some(group) = group {
            sys::stop(group);
        }
    }

    fn deadline(&self) -> Option<Instant> {
        self.lock().kill_at
    }
}

/// Stops the job's program from any thread.
#[derive(Debug, Clone)]
pub struct Canceler(Arc<Shared>);

impl Canceler {
    pub fn cancel(&self) {
        self.0.cancel();
    }
}

/// Supervises one claimed job over its connection to the app (upstream's `OverlayJobRunner`).
#[derive(Debug)]
pub struct JobRunner {
    socket: Arc<Connection>,
    /// The pty the program runs under, the helper's own stdin in production; `None` runs it without one.
    terminal: Option<sys::Terminal>,
    shared: Arc<Shared>,
}

impl JobRunner {
    /// A runner over `socket`, the app's connection. It is close-on-exec (std opens every descriptor so, and every
    /// Windows handle not inheritable), so the program never holds it: only the helper ending closes it, which is how
    /// the app learns an unreported job is over.
    #[must_use]
    fn new(socket: Connection, terminal: Option<sys::Terminal>, grace: Duration) -> Self {
        Self {
            socket: Arc::new(socket),
            terminal,
            shared: Arc::new(Shared {
                grace,
                stop: Mutex::new(Stop::default()),
            }),
        }
    }

    #[must_use]
    pub fn canceler(&self) -> Canceler {
        Canceler(Arc::clone(&self.shared))
    }

    /// Sends the claim and returns the launch context, or the app's refusal, having launched nothing.
    pub fn claim(&self, job: &str) -> Result<OverlayLaunchContext, String> {
        let mut line = serde_json::to_vec(&claim_request(job))
            .map_err(|error| format!("could not encode the request: {error}"))?;
        line.push(b'\n');
        client::send_all(&self.socket, &line).map_err(|_| "could not send the request".to_owned())?;
        let reply = client::read_socket_line(&self.socket, MAX_REQUEST_LINE_BYTES)
            .ok_or_else(|| "the app closed the connection without answering".to_owned())?;
        let response: Response =
            serde_json::from_slice(&reply).map_err(|error| format!("could not decode the reply: {error}"))?;
        if !response.ok {
            return Err(response
                .error
                .unwrap_or_else(|| "the app refused the stream".to_owned()));
        }
        match client::read_socket_line(&self.socket, MAX_FRAME_BYTES).map(|line| OverlayJobFrame::parse(&line)) {
            Some(Ok(OverlayJobFrame::Context { context })) => Ok(context),
            _ => Err("the app sent no launch context".to_owned()),
        }
    }

    /// Runs `context` to the end and returns the helper's own exit status: the program's, 128 plus the signal that
    /// ended it (on Windows: 130 for a program a console Ctrl event ended, 137 for one the kill ended), or 127 when
    /// it could not be launched. The program starts from `base`, the helper's own environment, with the context's
    /// over it, so it keeps HOME, PATH and TERM. A report the app never receives changes nothing.
    pub fn run(&self, context: &OverlayLaunchContext, base: &[(OsString, OsString)]) -> i32 {
        let tty = self.terminal.filter(|tty| sys::is_terminal(*tty));
        let mut program = match sys::spawn(context, base, tty) {
            Ok(program) => program,
            Err(error) => {
                // the child takes the terminal before its exec, which can still fail
                if let Some(tty) = tty {
                    sys::take_terminal_back(tty);
                }
                self.report(&OverlayJobFrame::LaunchFailed {
                    reason: format!("could not start the program: {}", os_message(&error)),
                });
                self.hang_up();
                return LAUNCH_FAILED_STATUS;
            }
        };
        let group = program.group();
        let canceled_early = {
            let mut stop = self.shared.lock();
            stop.group = Some(group);
            stop.kill_at.is_some()
        };
        self.report(&OverlayJobFrame::Started);
        if canceled_early {
            sys::stop(group);
        }
        let listener = self.listen();
        let status = self.supervise(&mut program, tty);
        if let Some(tty) = tty {
            sys::take_terminal_back(tty);
        }
        if self.shared.deadline().is_some() {
            self.report(&OverlayJobFrame::Canceled);
        } else {
            self.report(&OverlayJobFrame::Exited { code: status });
        }
        self.hang_up();
        if let Some(listener) = listener {
            // the listener only reads, and the hang-up just ended its read
            let _ = listener.join();
        }
        status
    }

    /// Ends the connection: the run reported all it will, and the listener's read returns.
    fn hang_up(&self) {
        let _ = self.socket.shutdown(Shutdown::Both);
    }

    /// Waits for the program, watching the terminal for a hangup, which cancels as a cancel frame does. After a cancel
    /// it also waits for the rest of the program's group, killing what outlives the grace, so no descendant is left
    /// running behind a `canceled` report.
    fn supervise(&self, program: &mut sys::Program, tty: Option<sys::Terminal>) -> i32 {
        let mut status = None;
        let mut killed = false;
        loop {
            if status.is_none() {
                status = program.reap();
            }
            // after the reap: a program whose read got the hangup's end-of-file has already exited, and that exit must
            // not pass for an ordinary one while its group lives on
            if let Some(tty) = tty
                && self.shared.deadline().is_none()
                && sys::hung_up(tty)
            {
                self.shared.cancel();
            }
            let deadline = self.shared.deadline();
            if status.is_some() && (deadline.is_none() || !program.alive()) {
                return status.unwrap_or_default();
            }
            if let Some(deadline) = deadline
                && !killed
                && Instant::now() >= deadline
            {
                program.kill();
                killed = true;
            }
            thread::sleep(SUPERVISE_TICK);
        }
    }

    /// Reads the app's frames until the connection ends, cancelling on a `cancel`. The app going away does not stop
    /// the program: it runs on under the viewer's pty, only unreported. `None` when no thread could be started.
    fn listen(&self) -> Option<JoinHandle<()>> {
        let socket = Arc::clone(&self.socket);
        let shared = Arc::clone(&self.shared);
        thread::Builder::new()
            .name("ngtermctl.run-job.listen".to_owned())
            .spawn(move || {
                while let Some(line) = client::read_socket_line(&socket, MAX_FRAME_BYTES) {
                    if matches!(OverlayJobFrame::parse(&line), Ok(OverlayJobFrame::Cancel)) {
                        shared.cancel();
                    }
                }
            })
            .ok()
    }

    /// Best effort: a report the app never receives changes nothing here.
    fn report(&self, frame: &OverlayJobFrame) {
        if let Ok(line) = frame.line() {
            let _ = client::send_all(&self.socket, &line);
        }
    }
}

/// The claim: the job id rides as the target.
fn claim_request(job: &str) -> Request {
    Request {
        cmd: Command::Session(SessionCmd::OverlayJobRun),
        target: Some(job.to_owned()),
        args: None,
    }
}

/// `ngtermctl session overlay run-job <job>`.
#[derive(Debug, clap::Args)]
#[command(
    name = "run-job",
    about = "Run a remote overlay job this app handed to another Mac.",
    long_about = "Run on this Mac by the ngterm on another Mac, over ssh, when it shows one of this app's overlays: it \
                  claims the job, runs its program under that ssh terminal, and reports how it ended. It is not meant \
                  to be run by hand. A job that cannot be claimed exits 1 having launched nothing, one whose program \
                  cannot be launched exits 127, and otherwise it exits with the program's status."
)]
pub struct RunJob {
    #[arg(help = "The job id the other Mac was handed.")]
    job: String,

    #[command(flatten)]
    options: BasicOptions,
}

impl RequestCommand for RunJob {
    fn options(&self) -> &BasicOptions {
        &self.options
    }

    /// The claim the run sends first.
    fn make_request(&self) -> Request {
        claim_request(&self.job)
    }

    fn invoke(&self, env: &dyn Env) -> Result<(), Failure> {
        let socket = client::connect(&self.options.socket_path(env))?;
        let runner = JobRunner::new(socket, Some(sys::stdin_terminal()), DEFAULT_GRACE);
        let context = match runner.claim(&self.job) {
            Ok(context) => context,
            Err(message) => {
                eprintln!("error: {message}");
                return Err(Failure::Exit(ExitCode::FAILURE));
            }
        };
        sys::route_signals(runner.canceler());
        let base: Vec<(OsString, OsString)> = std::env::vars_os().collect();
        match u8::try_from(runner.run(&context, &base)) {
            Ok(0) => Ok(()),
            Ok(status) => Err(Failure::Exit(ExitCode::from(status))),
            Err(_) => Err(Failure::Exit(ExitCode::FAILURE)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use ngterm_core::pane_env::PaneEnv;

    use super::*;

    const OK_REPLY: &str = r#"{"ok":true,"result":{"id":"job"}}"#;

    /// The app's end of a helper connection: it answers the claim with `reply`, sends `context`, and records every
    /// frame the helper reports until the connection ends.
    struct FakeOrigin {
        /// Where a test sends frames from; `None` once the origin hung up.
        writer: Option<Connection>,
        served: JoinHandle<(Vec<u8>, Vec<OverlayJobFrame>)>,
    }

    impl FakeOrigin {
        /// A connected pair: the helper's end, and the origin serving the other. With `hang_up`, the origin goes away
        /// right after the context.
        fn serve(reply: &str, context: Option<OverlayLaunchContext>, hang_up: bool) -> (Connection, Self) {
            let (helper, origin) = ngterm_server::endpoint::pair().unwrap();
            let writer = (!hang_up).then(|| origin.try_clone().unwrap());
            let reply = format!("{reply}\n");
            let served = thread::spawn(move || {
                let claim = client::read_socket_line(&origin, MAX_REQUEST_LINE_BYTES).unwrap_or_default();
                client::send_all(&origin, reply.as_bytes()).unwrap();
                if let Some(context) = context {
                    let line = OverlayJobFrame::Context { context }.line().unwrap();
                    client::send_all(&origin, &line).unwrap();
                }
                if hang_up {
                    return (claim, Vec::new());
                }
                let mut frames = Vec::new();
                while let Some(line) = client::read_socket_line(&origin, MAX_FRAME_BYTES) {
                    frames.push(OverlayJobFrame::parse(&line).unwrap());
                }
                (claim, frames)
            });
            (helper, Self { writer, served })
        }

        fn send(&self, frame: &OverlayJobFrame) {
            if let Some(writer) = &self.writer {
                client::send_all(writer, &frame.line().unwrap()).unwrap();
            }
        }

        /// The claim line the origin read and the frames it got, once the helper's end is closed.
        fn finish(self) -> (Vec<u8>, Vec<OverlayJobFrame>) {
            drop(self.writer);
            self.served.join().unwrap()
        }

        fn frames(self) -> Vec<OverlayJobFrame> {
            self.finish().1
        }
    }

    fn runner(helper: Connection, grace: Duration) -> JobRunner {
        JobRunner::new(helper, None, grace)
    }

    fn temp_cwd() -> String {
        std::env::temp_dir().to_string_lossy().into_owned()
    }

    fn context(command: &str) -> OverlayLaunchContext {
        let environment = PaneEnv::from([("NGTERM_ENABLED".to_owned(), "1".to_owned())]);
        OverlayLaunchContext::new(command, &temp_cwd(), environment)
    }

    #[cfg(unix)]
    fn base(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        pairs
            .iter()
            .map(|(name, value)| (OsString::from(name), OsString::from(value)))
            .collect()
    }

    #[cfg(unix)]
    fn plain_base() -> Vec<(OsString, OsString)> {
        base(&[("PATH", "/usr/bin:/bin"), ("HOME", "/tmp")])
    }

    /// The test's own environment: a Windows program needs `SystemRoot` and its kin, which the helper's would carry.
    #[cfg(windows)]
    fn plain_base() -> Vec<(OsString, OsString)> {
        std::env::vars_os().collect()
    }

    /// Runs `command` to the end against a fresh origin: the helper's status and the frames the origin got.
    fn run_to_end(command: &str, base: &[(OsString, OsString)]) -> (i32, Vec<OverlayJobFrame>) {
        let (helper, origin) = FakeOrigin::serve(OK_REPLY, Some(context(command)), false);
        let runner = runner(helper, DEFAULT_GRACE);
        let claimed = runner.claim("job").unwrap();
        let status = runner.run(&claimed, base);
        drop(runner);
        (status, origin.frames())
    }

    /// The pid written to `path`, waiting for it to appear.
    fn read_pid(path: &Path) -> u32 {
        for _ in 0..250 {
            if let Ok(pid) = std::fs::read_to_string(path).unwrap_or_default().trim().parse() {
                return pid;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!("no pid in {}", path.display())
    }

    /// Whether `pid` is gone, waiting up to two seconds.
    #[cfg(unix)]
    fn gone(pid: u32) -> bool {
        let pid = libc::pid_t::try_from(pid).unwrap();
        for _ in 0..100 {
            // SAFETY: signal 0 only checks that the process exists.
            if unsafe { libc::kill(pid, 0) } != 0 {
                return true;
            }
            thread::sleep(Duration::from_millis(20));
        }
        false
    }

    /// Whether `pid` is gone, waiting up to two seconds.
    #[cfg(windows)]
    fn gone(pid: u32) -> bool {
        use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
        use windows_sys::Win32::Foundation::{FALSE, WAIT_OBJECT_0};
        use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};
        // SAFETY: plain arguments; a process that is gone fails the call.
        let raw = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, FALSE, pid) };
        if raw.is_null() {
            return true;
        }
        // SAFETY: `raw` is the process handle just opened, owned by nothing else.
        let process = unsafe { OwnedHandle::from_raw_handle(raw) };
        // SAFETY: the handle is open with the access a wait needs.
        (unsafe { WaitForSingleObject(process.as_raw_handle(), 2_000) }) == WAIT_OBJECT_0
    }

    #[test]
    fn a_program_exiting_three_reports_three_and_the_helper_exits_three() {
        let (helper, origin) = FakeOrigin::serve(OK_REPLY, Some(context("exit 3")), false);
        let runner = runner(helper, DEFAULT_GRACE);
        let status = runner.run(&runner.claim("job-id").unwrap(), &plain_base());
        drop(runner);
        assert_eq!(status, 3);
        let (claim, frames) = origin.finish();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&claim).unwrap(),
            serde_json::json!({"cmd": "session.overlay.job.run", "target": "job-id"})
        );
        assert_eq!(frames, [OverlayJobFrame::Started, OverlayJobFrame::Exited { code: 3 }]);
    }

    #[test]
    fn a_refused_claim_launches_nothing() {
        let (helper, origin) = FakeOrigin::serve(r#"{"ok":false,"error":"job not claimable"}"#, None, false);
        let runner = runner(helper, DEFAULT_GRACE);
        assert_eq!(runner.claim("job"), Err("job not claimable".to_owned()));
        drop(runner);
        assert!(origin.frames().is_empty());
    }

    #[test]
    fn a_claim_answered_without_a_context_launches_nothing() {
        let (helper, origin) = FakeOrigin::serve(OK_REPLY, None, true);
        let runner = runner(helper, DEFAULT_GRACE);
        assert_eq!(runner.claim("job"), Err("the app sent no launch context".to_owned()));
        drop(runner);
        assert!(origin.frames().is_empty());
    }

    /// A program that runs until it is stopped, and the status its stop reads as.
    #[cfg(unix)]
    fn long_running() -> (OverlayLaunchContext, i32) {
        (context("sleep 30"), 128 + libc::SIGTERM)
    }

    #[cfg(windows)]
    fn long_running() -> (OverlayLaunchContext, i32) {
        (probe(Probe::Sleep, &[]), sys::INTERRUPTED_STATUS)
    }

    /// A program that ignores the grace stop.
    #[cfg(unix)]
    fn stubborn() -> OverlayLaunchContext {
        context(r#"trap "" TERM; sleep 30"#)
    }

    #[cfg(windows)]
    fn stubborn() -> OverlayLaunchContext {
        probe(Probe::Ignore, &[])
    }

    /// The status the kill past the grace sets.
    #[cfg(unix)]
    const KILLED: i32 = 128 + libc::SIGKILL;
    #[cfg(windows)]
    const KILLED: i32 = sys::KILLED_STATUS;

    #[test]
    fn a_cancel_from_the_app_ends_the_program_canceled() {
        let (context, stopped) = long_running();
        let (helper, origin) = FakeOrigin::serve(OK_REPLY, Some(context), false);
        let runner = runner(helper, Duration::from_millis(300));
        let claimed = runner.claim("job").unwrap();
        let status = thread::scope(|scope| {
            scope.spawn(|| {
                thread::sleep(Duration::from_millis(300));
                origin.send(&OverlayJobFrame::Cancel);
            });
            runner.run(&claimed, &plain_base())
        });
        drop(runner);
        assert_eq!(status, stopped);
        assert_eq!(origin.frames(), [OverlayJobFrame::Started, OverlayJobFrame::Canceled]);
    }

    #[test]
    fn a_cancel_kills_a_program_that_ignores_the_first_signal() {
        let (helper, origin) = FakeOrigin::serve(OK_REPLY, Some(stubborn()), false);
        let runner = runner(helper, Duration::from_millis(300));
        let claimed = runner.claim("job").unwrap();
        let canceler = runner.canceler();
        let started = Instant::now();
        let status = thread::scope(|scope| {
            scope.spawn(|| {
                thread::sleep(Duration::from_millis(300));
                canceler.cancel();
            });
            runner.run(&claimed, &plain_base())
        });
        drop(runner);
        assert_eq!(status, KILLED);
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(origin.frames(), [OverlayJobFrame::Started, OverlayJobFrame::Canceled]);
    }

    /// Blocks SIGTERM in the calling thread, as `route_signals` does in run-job's, until dropped.
    #[cfg(unix)]
    struct SigtermBlocked(libc::sigset_t);

    #[cfg(unix)]
    impl SigtermBlocked {
        fn new() -> Self {
            // SAFETY: zeroed sets are initialized by `sigemptyset` or written by `pthread_sigmask` before use.
            let (mut term, mut previous): (libc::sigset_t, libc::sigset_t) = unsafe { std::mem::zeroed() };
            // SAFETY: `term` is a valid, writable set.
            unsafe { libc::sigemptyset(&raw mut term) };
            // SAFETY: as above, and SIGTERM is a valid signal.
            unsafe { libc::sigaddset(&raw mut term, libc::SIGTERM) };
            // SAFETY: changes only this thread's mask; both sets are valid.
            let result = unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, &raw const term, &raw mut previous) };
            assert_eq!(result, 0);
            Self(previous)
        }
    }

    #[cfg(unix)]
    impl Drop for SigtermBlocked {
        fn drop(&mut self) {
            // SAFETY: restores the mask read in `new`.
            unsafe { libc::pthread_sigmask(libc::SIG_SETMASK, &raw const self.0, std::ptr::null_mut()) };
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_program_gets_the_stop_signals_the_helpers_thread_blocks() {
        let _blocked = SigtermBlocked::new();
        let dir = tempfile::tempdir().unwrap();

        let mask_file = dir.path().join("mask");
        // the shell reads its own mask by becoming the reader: a forked `grep` reading `/proc/$$/status` races dash,
        // which blocks every signal around its (v)fork until the child execs (seen on ubuntu-26.04 as a full mask)
        let probe = format!("exec grep '^SigBlk:' /proc/self/status > {}", mask_file.display());
        let (status, _) = run_to_end(&probe, &plain_base());
        assert_eq!(status, 0);
        let line = std::fs::read_to_string(&mask_file).unwrap();
        let mask = u64::from_str_radix(line.trim_start_matches("SigBlk:").trim(), 16).unwrap();
        assert_eq!(
            mask & (1 << (libc::SIGTERM - 1)),
            0,
            "the shell started with SIGTERM blocked: {line}"
        );

        let ready = dir.path().join("ready");
        let command = format!("trap 'exit 0' TERM; : > {}; while :; do :; done", ready.display());
        let grace = Duration::from_secs(2);
        let (helper, origin) = FakeOrigin::serve(OK_REPLY, Some(context(&command)), false);
        let runner = runner(helper, grace);
        let claimed = runner.claim("job").unwrap();
        let canceler = runner.canceler();
        let (status, canceled_at) = thread::scope(|scope| {
            let canceling = scope.spawn(|| {
                while !ready.exists() {
                    thread::sleep(Duration::from_millis(20));
                }
                canceler.cancel();
                Instant::now()
            });
            let status = runner.run(&claimed, &plain_base());
            (status, canceling.join().unwrap())
        });
        drop(runner);
        assert_eq!(status, 0, "the trap never ran");
        assert!(canceled_at.elapsed() < grace);
        assert_eq!(origin.frames(), [OverlayJobFrame::Started, OverlayJobFrame::Canceled]);
    }

    #[test]
    fn a_report_the_app_never_gets_leaves_the_helpers_status_alone() {
        #[cfg(unix)]
        let command = "sleep 0.3; exit 5";
        #[cfg(windows)]
        let command = "ping -n 2 127.0.0.1 >nul & exit 5";
        let (helper, origin) = FakeOrigin::serve(OK_REPLY, Some(context(command)), true);
        let runner = runner(helper, DEFAULT_GRACE);
        let claimed = runner.claim("job").unwrap();
        assert!(origin.frames().is_empty());
        assert_eq!(runner.run(&claimed, &plain_base()), 5);
    }

    #[cfg(unix)]
    #[test]
    fn the_program_does_not_inherit_the_app_connection() {
        use std::os::fd::AsRawFd as _;
        let (helper, origin) = FakeOrigin::serve(OK_REPLY, Some(context("exit 9")), false);
        let fd = helper.as_raw_fd();
        let runner = runner(helper, DEFAULT_GRACE);
        let mut claimed = runner.claim("job").unwrap();
        // the context is the app's to choose, so the test may aim it at the descriptor it now knows
        let probe = format!("test ! -e /dev/fd/{fd}");
        claimed.environment.insert("NGTERM_OVL_CMD".to_owned(), probe.clone());
        claimed.command = probe;
        let status = runner.run(&claimed, &plain_base());
        drop(runner);
        assert_eq!(status, 0, "the connection's descriptor reached the program");
        assert_eq!(
            origin.frames(),
            [OverlayJobFrame::Started, OverlayJobFrame::Exited { code: 0 }]
        );
    }

    #[test]
    fn the_terminal_type_comes_from_the_helpers_own_environment() {
        let mut base = plain_base();
        base.retain(|(name, _)| !name.eq_ignore_ascii_case("TERM"));
        base.push(("TERM".into(), "xterm-kitty".into()));
        #[cfg(unix)]
        let command = r#"test "$TERM" = xterm-kitty"#;
        #[cfg(windows)]
        let command = r#"if "%TERM%"=="xterm-kitty" (exit 0) else (exit 1)"#;
        let (status, frames) = run_to_end(command, &base);
        assert_eq!(status, 0);
        assert_eq!(frames, [OverlayJobFrame::Started, OverlayJobFrame::Exited { code: 0 }]);
    }

    #[cfg(unix)]
    #[test]
    fn the_program_gets_the_helpers_environment_under_the_context() {
        let bin = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("/bin/true", bin.path().join("ngterm-fixture-tool")).unwrap();
        let session = "0F2C7A4E-9B1D-4C3A-8E5F-1A2B3C4D5E6F";
        let environment = PaneEnv::from([("NGTERM_SESSION_ID".to_owned(), session.to_owned())]);
        let command = format!(r#"ngterm-fixture-tool && test -n "$HOME" && test "$NGTERM_SESSION_ID" = "{session}""#);
        let (helper, origin) = FakeOrigin::serve(
            OK_REPLY,
            Some(OverlayLaunchContext::new(&command, "/tmp", environment)),
            false,
        );
        let runner = runner(helper, DEFAULT_GRACE);
        let path = format!("{}:/usr/bin:/bin", bin.path().display());
        let base = base(&[("PATH", &path), ("HOME", "/home/x"), ("NGTERM_SESSION_ID", "stale")]);
        let status = runner.run(&runner.claim("job").unwrap(), &base);
        drop(runner);
        assert_eq!(status, 0);
        assert_eq!(
            origin.frames(),
            [OverlayJobFrame::Started, OverlayJobFrame::Exited { code: 0 }]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_cancel_ends_the_programs_descendants_too() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("descendant.pid");
        let command = format!("/bin/sleep 60 & echo $! > {}; wait", pid_file.display());
        let (helper, origin) = FakeOrigin::serve(OK_REPLY, Some(context(&command)), false);
        let runner = runner(helper, Duration::from_millis(300));
        let claimed = runner.claim("job").unwrap();
        thread::scope(|scope| {
            scope.spawn(|| {
                thread::sleep(Duration::from_millis(300));
                origin.send(&OverlayJobFrame::Cancel);
            });
            runner.run(&claimed, &plain_base())
        });
        drop(runner);
        assert!(gone(read_pid(&pid_file)), "the descendant outlived the cancel");
        assert_eq!(origin.frames(), [OverlayJobFrame::Started, OverlayJobFrame::Canceled]);
    }

    #[test]
    fn a_program_that_cannot_start_reports_launch_failed() {
        let context = OverlayLaunchContext::new("true", "/nonexistent-ngterm-run-job", PaneEnv::new());
        let (helper, origin) = FakeOrigin::serve(OK_REPLY, Some(context), false);
        let runner = runner(helper, DEFAULT_GRACE);
        let status = runner.run(&runner.claim("job").unwrap(), &plain_base());
        drop(runner);
        assert_eq!(status, 127);
        let frames = origin.frames();
        assert!(
            matches!(frames.as_slice(), [OverlayJobFrame::LaunchFailed { .. }]),
            "{frames:?}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn the_program_gets_the_helpers_environment_under_the_context() {
        let session = "0F2C7A4E-9B1D-4C3A-8E5F-1A2B3C4D5E6F";
        let environment = PaneEnv::from([("NGTERM_SESSION_ID".to_owned(), session.to_owned())]);
        let command =
            format!(r#"if "%NGTERM_SESSION_ID%"=="{session}" (if defined HOME (exit 0) else (exit 2)) else (exit 1)"#);
        let context = OverlayLaunchContext::new(&command, &temp_cwd(), environment);
        let (helper, origin) = FakeOrigin::serve(OK_REPLY, Some(context), false);
        let runner = runner(helper, DEFAULT_GRACE);
        let mut base = plain_base();
        base.retain(|(name, _)| {
            !["HOME", "NGTERM_SESSION_ID"]
                .iter()
                .any(|taken| name.eq_ignore_ascii_case(taken))
        });
        base.extend([
            ("HOME".into(), "C:\\Users\\x".into()),
            ("NGTERM_SESSION_ID".into(), "stale".into()),
        ]);
        let status = runner.run(&runner.claim("job").unwrap(), &base);
        drop(runner);
        assert_eq!(status, 0);
        assert_eq!(
            origin.frames(),
            [OverlayJobFrame::Started, OverlayJobFrame::Exited { code: 0 }]
        );
    }

    /// What the probe (this test binary, run again as the job's program) does until it is stopped.
    #[cfg(windows)]
    #[derive(Debug, Clone, Copy)]
    enum Probe {
        /// Sleeps, so a Ctrl+Break ends it with `STATUS_CONTROL_C_EXIT`.
        Sleep,
        /// Sleeps, ignoring every Ctrl event, so only the kill ends it.
        Ignore,
        /// Sleeps, exiting 0 on a Ctrl+Break, as a program that handles the stop.
        Trap,
    }

    /// The probe's environment variables: its mode, where it writes its pid, and a file it creates once it is ready.
    #[cfg(windows)]
    const PROBE_MODE: &str = "NGTERMCTL_TEST_OVERLAY_PROBE";
    #[cfg(windows)]
    const PROBE_PID: &str = "NGTERMCTL_TEST_OVERLAY_PROBE_PID";
    #[cfg(windows)]
    const PROBE_READY: &str = "NGTERMCTL_TEST_OVERLAY_PROBE_READY";

    /// A context whose program is the probe in `mode`, with `extra` in its environment.
    #[cfg(windows)]
    fn probe(mode: Probe, extra: &[(&str, &Path)]) -> OverlayLaunchContext {
        let exe = std::env::current_exe().unwrap();
        let command = format!(
            r#""{}" overlay_job::tests::the_probe --exact --nocapture --test-threads=1"#,
            exe.display()
        );
        let mut environment = PaneEnv::from([(PROBE_MODE.to_owned(), format!("{mode:?}"))]);
        for (name, path) in extra {
            environment.insert((*name).to_owned(), path.to_string_lossy().into_owned());
        }
        OverlayLaunchContext::new(&command, &temp_cwd(), environment)
    }

    #[cfg(windows)]
    unsafe extern "system" fn ignore_every_event(_event: u32) -> windows_sys::core::BOOL {
        windows_sys::Win32::Foundation::TRUE
    }

    #[cfg(windows)]
    unsafe extern "system" fn exit_on_break(_event: u32) -> windows_sys::core::BOOL {
        std::process::exit(0)
    }

    /// Not a test of its own: the job's program in the tests above, when the probe's mode is set; without it, nothing.
    #[cfg(windows)]
    #[test]
    fn the_probe() {
        use windows_sys::Win32::Foundation::TRUE;
        use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;
        let Ok(mode) = std::env::var(PROBE_MODE) else {
            return;
        };
        let handler: Option<unsafe extern "system" fn(u32) -> windows_sys::core::BOOL> = match mode.as_str() {
            "Ignore" => Some(ignore_every_event),
            "Trap" => Some(exit_on_break),
            _ => None,
        };
        if handler.is_some() {
            // SAFETY: the handler is a valid function for the process's whole life.
            unsafe { SetConsoleCtrlHandler(handler, TRUE) };
        }
        if let Some(path) = std::env::var_os(PROBE_PID) {
            std::fs::write(path, std::process::id().to_string()).unwrap();
        }
        if let Some(path) = std::env::var_os(PROBE_READY) {
            std::fs::write(path, b"").unwrap();
        }
        thread::sleep(Duration::from_secs(30));
    }

    #[cfg(windows)]
    #[test]
    fn a_program_that_handles_the_stop_ends_with_its_own_status_within_the_grace() {
        let dir = tempfile::tempdir().unwrap();
        let ready = dir.path().join("ready");
        let grace = Duration::from_secs(5);
        let (helper, origin) = FakeOrigin::serve(OK_REPLY, Some(probe(Probe::Trap, &[(PROBE_READY, &ready)])), false);
        let runner = runner(helper, grace);
        let claimed = runner.claim("job").unwrap();
        let canceler = runner.canceler();
        let (status, canceled_at) = thread::scope(|scope| {
            let canceling = scope.spawn(|| {
                while !ready.exists() {
                    thread::sleep(Duration::from_millis(20));
                }
                canceler.cancel();
                Instant::now()
            });
            let status = runner.run(&claimed, &plain_base());
            (status, canceling.join().unwrap())
        });
        drop(runner);
        assert_eq!(status, 0, "the handler never ran");
        assert!(canceled_at.elapsed() < grace);
        assert_eq!(origin.frames(), [OverlayJobFrame::Started, OverlayJobFrame::Canceled]);
    }

    #[cfg(windows)]
    #[test]
    fn a_cancel_ends_the_programs_descendants_too() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("descendant.pid");
        // the probe is the shell's child: a descendant of the program, which is the shell
        let context = probe(Probe::Ignore, &[(PROBE_PID, &pid_file)]);
        let (helper, origin) = FakeOrigin::serve(OK_REPLY, Some(context), false);
        let runner = runner(helper, Duration::from_millis(300));
        let claimed = runner.claim("job").unwrap();
        thread::scope(|scope| {
            scope.spawn(|| {
                let _ = read_pid(&pid_file);
                origin.send(&OverlayJobFrame::Cancel);
            });
            runner.run(&claimed, &plain_base())
        });
        drop(runner);
        assert!(gone(read_pid(&pid_file)), "the descendant outlived the cancel");
        assert_eq!(origin.frames(), [OverlayJobFrame::Started, OverlayJobFrame::Canceled]);
    }
}
