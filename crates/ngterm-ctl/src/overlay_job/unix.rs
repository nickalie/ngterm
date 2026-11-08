//! The overlay job's program on Linux and macOS: a process group of its own under the viewer's pty, its foreground
//! while it runs, signalled as a whole.

use std::ffi::OsString;
use std::io::{self, ErrorKind};
use std::os::fd::RawFd;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::Child;
use std::thread;

use ngterm_core::overlay_capture::OverlayLaunchContext;
use ngterm_host::ShellCommand;

use super::Canceler;

/// The pty the program runs under: a descriptor of the helper's.
pub(super) type Terminal = RawFd;
/// The program's process group, which its pid names.
pub(super) type Group = libc::pid_t;

/// The script the shell (`ngterm_host::command_line`'s `/bin/sh -c`) runs. `eval` keeps the command's own exit status
/// as the shell's; the command rides `NGTERM_OVL_CMD` in the context's environment, so nothing interpolates it.
const SHELL_SCRIPT: &str = r#"eval "$NGTERM_OVL_CMD""#;
/// The signals the helper changes, put back to their defaults in the program.
const RESET_SIGNALS: [libc::c_int; 7] = [
    libc::SIGINT,
    libc::SIGQUIT,
    libc::SIGTSTP,
    libc::SIGTTOU,
    libc::SIGTERM,
    libc::SIGHUP,
    libc::SIGPIPE,
];
/// The signals that cancel the program as a `cancel` frame does: losing the terminal, or being told to stop.
const STOP_SIGNALS: [libc::c_int; 2] = [libc::SIGHUP, libc::SIGTERM];
/// The terminal's keys are the program's, and taking the terminal back from its group happens from the background.
const IGNORED_SIGNALS: [libc::c_int; 4] = [libc::SIGINT, libc::SIGQUIT, libc::SIGTSTP, libc::SIGTTOU];

/// The helper's stdin, the viewer's pty in production.
pub(super) fn stdin_terminal() -> Terminal {
    libc::STDIN_FILENO
}

pub(super) fn is_terminal(fd: Terminal) -> bool {
    // SAFETY: `isatty` only inspects the descriptor; an invalid one is an error, not undefined behavior.
    (unsafe { libc::isatty(fd) }) == 1
}

/// The grace stop: SIGTERM to the whole group.
pub(super) fn stop(group: Group) {
    signal_group(group, libc::SIGTERM);
}

/// Hands the terminal back to the helper's own group, the program having ended or failed to start.
pub(super) fn take_terminal_back(tty: Terminal) {
    // SAFETY: `getpgrp` only reads the calling process's group.
    let helper = unsafe { libc::getpgrp() };
    give_terminal(tty, helper);
}

/// Sends `signal` to the whole process group `group` leads.
fn signal_group(group: libc::pid_t, signal: libc::c_int) {
    // SAFETY: `kill` takes any pid and signal; a group that is gone is an error, ignored here.
    unsafe { libc::kill(-group, signal) };
}

/// Whether any process of the group `group` leads still exists.
fn group_alive(group: libc::pid_t) -> bool {
    // SAFETY: signal 0 only checks that the group exists.
    (unsafe { libc::kill(-group, 0) }) == 0
}

/// Makes `group` the terminal's foreground with SIGTTOU held, which a background caller would otherwise get. Only
/// async-signal-safe calls, so the child may make it between fork and exec. Returns `tcsetpgrp`'s result.
fn give_terminal(tty: RawFd, group: libc::pid_t) -> libc::c_int {
    // SAFETY: an all-zero `sigset_t` is a valid (empty) set on Linux, which `sigemptyset` then initializes.
    let mut held: libc::sigset_t = unsafe { std::mem::zeroed() };
    // SAFETY: `held` is a valid, writable set.
    unsafe { libc::sigemptyset(&raw mut held) };
    // SAFETY: as above, and SIGTTOU is a valid signal.
    unsafe { libc::sigaddset(&raw mut held, libc::SIGTTOU) };
    // SAFETY: `previous` is written by `pthread_sigmask` before it is read.
    let mut previous: libc::sigset_t = unsafe { std::mem::zeroed() };
    // SAFETY: changes only this thread's mask; both sets are valid.
    unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, &raw const held, &raw mut previous) };
    // SAFETY: `tcsetpgrp` takes any descriptor and group; a refusal is its result.
    let result = unsafe { libc::tcsetpgrp(tty, group) };
    // SAFETY: restores the mask read above.
    unsafe { libc::pthread_sigmask(libc::SIG_SETMASK, &raw const previous, std::ptr::null_mut()) };
    result
}

/// In the child between fork and exec: take the terminal before the program runs a single instruction, or its first
/// read would stop it, then put back the signal dispositions the helper changed and unblock every signal: the child
/// inherits the forking thread's mask, which holds the stop signals for the helper's waiter.
fn prepare_child(tty: Option<RawFd>) -> io::Result<()> {
    // SAFETY: `getpid` only reads the calling process's id.
    let program = unsafe { libc::getpid() };
    if let Some(tty) = tty
        && give_terminal(tty, program) != 0
    {
        return Err(io::Error::last_os_error());
    }
    for signal in RESET_SIGNALS {
        // SAFETY: installs the default action, no handler code, for a valid signal.
        unsafe { libc::signal(signal, libc::SIG_DFL) };
    }
    // SAFETY: an all-zero `sigset_t` is a valid (empty) set on Linux, which `sigemptyset` then initializes.
    let mut unblocked: libc::sigset_t = unsafe { std::mem::zeroed() };
    // SAFETY: `unblocked` is a valid, writable set.
    unsafe { libc::sigemptyset(&raw mut unblocked) };
    // SAFETY: replaces only this thread's mask with a valid set.
    let result = unsafe { libc::pthread_sigmask(libc::SIG_SETMASK, &raw const unblocked, std::ptr::null_mut()) };
    if result != 0 {
        return Err(io::Error::from_raw_os_error(result));
    }
    Ok(())
}

/// Starts the program as the leader of a process group of its own, which a cancel ends as a whole and which becomes
/// the terminal's foreground, so it gets the keys and SIGWINCH directly.
pub(super) fn spawn(
    context: &OverlayLaunchContext,
    base: &[(OsString, OsString)],
    tty: Option<Terminal>,
) -> io::Result<Program> {
    let shell = ShellCommand::posix(SHELL_SCRIPT);
    let mut command = std::process::Command::new(&shell.program);
    command
        .args(&shell.args)
        .env_clear()
        .envs(base.iter().map(|(name, value)| (name, value)))
        .envs(&context.environment)
        .current_dir(&context.cwd)
        .process_group(0);
    // SAFETY: the hook runs in the forked child after std put it in its own group (std leaves it the forking thread's
    // signal mask, which the hook clears), and calls only async-signal-safe functions, allocating nothing.
    unsafe { command.pre_exec(move || prepare_child(tty)) };
    #[cfg(target_os = "macos")]
    crate::local::settle_libnotify();
    let child = command.spawn()?;
    match libc::pid_t::try_from(child.id()) {
        Ok(group) => Ok(Program { child, group }),
        Err(_) => Err(io::Error::from(ErrorKind::InvalidData)),
    }
}

/// The running program: its child, which leads the process group a cancel ends.
#[derive(Debug)]
pub(super) struct Program {
    child: Child,
    group: Group,
}

impl Program {
    pub(super) fn group(&self) -> Group {
        self.group
    }

    /// The program's status once it ended: its exit code, or 128 plus the signal that ended it. `None` while it
    /// runs.
    pub(super) fn reap(&mut self) -> Option<i32> {
        let status = self.child.try_wait().ok()??;
        status.code().or_else(|| status.signal().map(|signal| 128 + signal))
    }

    /// Whether any process of the program's group is still there.
    pub(super) fn alive(&self) -> bool {
        group_alive(self.group)
    }

    /// SIGKILL to the whole group, past the grace.
    pub(super) fn kill(&self) {
        signal_group(self.group, libc::SIGKILL);
    }
}

/// Whether the terminal hung up, asked without reading, so the program's input is never taken.
pub(super) fn hung_up(tty: Terminal) -> bool {
    let mut probe = libc::pollfd {
        fd: tty,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: `probe` is one valid `pollfd`, and a zero timeout never blocks.
    if (unsafe { libc::poll(&raw mut probe, 1, 0) }) <= 0 {
        return false;
    }
    probe.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0
}

/// Ignores the terminal's keys and SIGTTOU, and turns SIGHUP and SIGTERM into a cancel: like a shell running a
/// foreground command. The stop signals are blocked in the calling thread before any other starts, so every later
/// thread inherits the block and only the waiting thread takes them.
pub(super) fn route_signals(canceler: Canceler) {
    for signal in IGNORED_SIGNALS {
        // SAFETY: ignoring a valid signal installs no handler code.
        unsafe { libc::signal(signal, libc::SIG_IGN) };
    }
    // SAFETY: an all-zero `sigset_t` is a valid (empty) set on Linux, which `sigemptyset` then initializes.
    let mut stops: libc::sigset_t = unsafe { std::mem::zeroed() };
    // SAFETY: `stops` is a valid, writable set.
    unsafe { libc::sigemptyset(&raw mut stops) };
    for signal in STOP_SIGNALS {
        // SAFETY: as above, for a valid signal.
        unsafe { libc::sigaddset(&raw mut stops, signal) };
    }
    // SAFETY: changes only this thread's mask, inherited by the threads it starts next.
    unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, &raw const stops, std::ptr::null_mut()) };
    let waiter = thread::Builder::new()
        .name("ngtermctl.run-job.signals".to_owned())
        .spawn(move || {
            loop {
                let mut received: libc::c_int = 0;
                // SAFETY: `stops` is a valid set of signals this thread blocks, and `received` is writable.
                if unsafe { libc::sigwait(&raw const stops, &raw mut received) } == 0 {
                    canceler.cancel();
                }
            }
        });
    // without the waiter the stop signals stay blocked: the program still ends with the terminal's hangup
    drop(waiter);
}

#[cfg(test)]
mod tests {
    use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
    use std::time::Duration;
    #[cfg(target_os = "linux")]
    use std::time::Instant;

    use super::*;
    use ngterm_core::overlay_jobs::OverlayJobFrame;

    #[test]
    fn hangup_checks_leave_input_for_the_program() {
        use std::io::{Read as _, Write as _};
        let (mut reader, mut writer) = std::os::unix::net::UnixStream::pair().unwrap();
        assert!(!is_terminal(reader.as_raw_fd()));
        assert!(!hung_up(reader.as_raw_fd()));
        writer.write_all(b"keys").unwrap();
        assert!(!hung_up(reader.as_raw_fd()));
        let mut bytes = [0; 4];
        reader.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"keys");
        drop(writer);
        assert!(hung_up(reader.as_raw_fd()));
        assert!(hung_up(i32::MAX));
        assert!(!is_terminal(-1));
        assert_eq!(stdin_terminal(), libc::STDIN_FILENO);
    }

    /// Signal dispositions and session ownership belong to this subprocess alone.
    #[test]
    fn terminal_probe() {
        let Ok(mode) = std::env::var("CTL_UNIX_PROBE") else {
            return;
        };
        if mode == "signals" {
            signal_probe();
            return;
        }
        assert_eq!(prepare_child(Some(-1)).unwrap_err().raw_os_error(), Some(libc::EBADF));
        let (mut master, mut slave) = (-1, -1);
        // SAFETY: both outputs point at initialized descriptor slots; optional settings are null.
        let opened = unsafe {
            libc::openpty(
                &raw mut master,
                &raw mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(opened, 0);
        // SAFETY: openpty returned two fresh owned descriptors.
        let master = unsafe { OwnedFd::from_raw_fd(master) };
        // SAFETY: as above.
        let slave = unsafe { OwnedFd::from_raw_fd(slave) };
        // SAFETY: this isolated process is not a process-group leader and may start its own session.
        assert!(unsafe { libc::setsid() } > 0);
        // SAFETY: the slave is an open pty in this process's new session.
        assert_eq!(unsafe { libc::ioctl(slave.as_raw_fd(), libc::TIOCSCTTY, 0) }, 0);
        // A master hangup sends SIGHUP to this session; keep it from ending the probe.
        // SAFETY: installs no handler code, for a valid signal, in this subprocess alone.
        unsafe { libc::signal(libc::SIGHUP, libc::SIG_IGN) };
        assert!(is_terminal(slave.as_raw_fd()));
        prepare_child(Some(slave.as_raw_fd())).unwrap();
        // SAFETY: tcgetpgrp only reads this open terminal's foreground group.
        assert_eq!(unsafe { libc::tcgetpgrp(slave.as_raw_fd()) }, unsafe { libc::getpid() });
        take_terminal_back(slave.as_raw_fd());

        run_pty_jobs(slave.as_raw_fd());
        // prepare_child reset SIGHUP; ignore it again before closing the master.
        // SAFETY: valid signal and no handler code, in this subprocess alone.
        unsafe { libc::signal(libc::SIGHUP, libc::SIG_IGN) };
        drop(master);
        assert!(hung_up(slave.as_raw_fd()));
    }

    fn run_pty_jobs(tty: RawFd) {
        for (command, cwd, expected) in [("exit 0", "/tmp", 0), ("exit 0", "/ngterm-missing-directory", 127)] {
            let (origin, helper) = ngterm_server::endpoint::pair().unwrap();
            let frames = thread::spawn(move || {
                let mut frames = Vec::new();
                while let Some(line) = crate::client::read_socket_line(&origin, 1024) {
                    frames.push(ngterm_core::overlay_jobs::OverlayJobFrame::parse(&line).unwrap());
                }
                frames
            });
            let runner = super::super::JobRunner::new(helper, Some(tty), Duration::from_secs(1));
            let context = OverlayLaunchContext::new(command, cwd, ngterm_core::pane_env::PaneEnv::new());
            assert_eq!(runner.run(&context, &[]), expected);
            let frames = frames.join().unwrap();
            if expected == 0 {
                assert_eq!(frames, [OverlayJobFrame::Started, OverlayJobFrame::Exited { code: 0 }]);
            } else {
                assert!(
                    matches!(frames.as_slice(), [OverlayJobFrame::LaunchFailed { reason }] if reason.starts_with("could not start the program:"))
                );
            }
            // SAFETY: these calls read this subprocess's terminal and process group.
            assert_eq!(unsafe { libc::tcgetpgrp(tty) }, unsafe { libc::getpgrp() });
        }
    }

    #[cfg(target_os = "linux")]
    fn signal_probe() {
        let shared = std::sync::Arc::new(super::super::Shared {
            grace: Duration::from_secs(1),
            stop: std::sync::Mutex::new(super::super::Stop::default()),
        });
        route_signals(Canceler(std::sync::Arc::clone(&shared)));
        let deadline = Instant::now() + Duration::from_secs(5);
        let tid = loop {
            let tid = std::fs::read_dir("/proc/self/task")
                .unwrap()
                .filter_map(Result::ok)
                .find_map(|entry| {
                    let name = std::fs::read_to_string(entry.path().join("comm")).ok()?;
                    (name.trim() == "ngtermctl.run-j")
                        .then(|| entry.file_name().to_string_lossy().parse::<libc::pid_t>().unwrap())
                });
            if let Some(tid) = tid {
                break tid;
            }
            assert!(Instant::now() < deadline, "signal waiter never started");
            thread::yield_now();
        };
        // SAFETY: getpid reads this subprocess's id.
        let pid = unsafe { libc::getpid() };
        // SAFETY: direct a stop signal to the blocked waiter in this process, leaving the test harness alone.
        let sent = unsafe { libc::syscall(libc::SYS_tgkill, pid, tid, libc::SIGTERM) };
        assert_eq!(sent, 0);
        while shared.deadline().is_none() {
            assert!(Instant::now() < deadline, "stop did not cancel the job");
            thread::yield_now();
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn signal_probe() {}

    #[test]
    fn a_child_takes_the_terminal_and_a_stop_signal_cancels_the_helper() {
        let mut modes = vec!["terminal"];
        if cfg!(target_os = "linux") {
            modes.push("signals");
        }
        for mode in modes {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "overlay_job::sys::tests::terminal_probe",
                    "--exact",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env_clear()
                .envs(std::env::vars_os().filter(|(key, _)| !key.to_string_lossy().starts_with("NGTERM_")))
                .env("CTL_UNIX_PROBE", mode)
                .output()
                .unwrap();
            assert!(output.status.success(), "{mode}: {output:?}");
            // the filter matched the probe: a mistyped name would pass with no test run
            assert!(
                String::from_utf8_lossy(&output.stdout).contains("1 passed"),
                "{mode}: {output:?}"
            );
        }
    }
}
