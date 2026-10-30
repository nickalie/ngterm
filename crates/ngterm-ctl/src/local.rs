//! What the local-only subcommands (`clipboard set`, `terminfo install`) share: they run child processes instead of
//! talking to the socket.

use std::process::ExitStatus;

/// SIGPIPE ignored while it lives, the previous action restored on drop. `main` puts SIGPIPE back to its default so a
/// closed stdout ends the CLI quietly; a write into a child's stdin must instead get `EPIPE` when the child exits
/// without reading, so its own status is reported (upstream's `F_SETNOSIGPIPE` on the pipe, which Linux lacks). The
/// disposition is process-wide: the local subcommands run on the main thread, before any output is printed.
///
/// Windows has no SIGPIPE: a write to a pipe whose reader is gone fails with an error there, so the guard holds
/// nothing.
#[cfg(unix)]
pub struct SigpipeIgnored {
    previous: libc::sighandler_t,
}

/// SIGPIPE's guard on Windows, which has no SIGPIPE: nothing to change.
#[cfg(windows)]
pub struct SigpipeIgnored;

#[cfg(windows)]
impl SigpipeIgnored {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

#[cfg(unix)]
impl SigpipeIgnored {
    #[must_use]
    pub fn new() -> Self {
        // SAFETY: `signal` with a valid signal number and `SIG_IGN` installs no handler code.
        let previous = unsafe { libc::signal(libc::SIGPIPE, libc::SIG_IGN) };
        Self { previous }
    }
}

#[cfg(unix)]
impl Drop for SigpipeIgnored {
    fn drop(&mut self) {
        if self.previous != libc::SIG_ERR {
            // SAFETY: `previous` is the action `signal` returned for SIGPIPE, so it is a valid one to reinstate.
            unsafe { libc::signal(libc::SIGPIPE, self.previous) };
        }
    }
}

/// A child's exit status as Foundation's `terminationStatus` reads it: the exit code, else the signal that ended it.
/// A Windows process always has an exit code (an `NTSTATUS` for a crash, which reads negative).
#[must_use]
pub fn exit_status(status: ExitStatus) -> i32 {
    #[cfg(unix)]
    let signal = || std::os::unix::process::ExitStatusExt::signal(&status);
    #[cfg(windows)]
    let signal = || None;
    status.code().or_else(signal).unwrap_or(-1)
}

/// Every signal unblocked and back to its default action, for a child about to exec (upstream's
/// `POSIX_SPAWN_SETSIGMASK` with an empty mask and `POSIX_SPAWN_SETSIGDEF` with every signal): the mask and ignored
/// actions survive `exec` (a handler does not), so a caller blocking or ignoring SIGTERM would otherwise leave the
/// child deaf to it. Only `sigemptyset`, `sigprocmask` and `signal` run, all async-signal-safe, so this may run
/// between `fork` and `exec`.
#[cfg(unix)]
pub fn reset_signals() {
    // SAFETY: a zeroed set is initialized by `sigemptyset` before use.
    let mut none: libc::sigset_t = unsafe { std::mem::zeroed() };
    // SAFETY: `none` is a valid set to clear.
    unsafe { libc::sigemptyset(&raw mut none) };
    // SAFETY: `none` is a valid, empty set; the old mask is not asked for.
    unsafe { libc::sigprocmask(libc::SIG_SETMASK, &raw const none, std::ptr::null_mut()) };
    for signal in 1..=64 {
        // SAFETY: `signal` with `SIG_DFL` installs no handler code; SIGKILL, SIGSTOP and the numbers glibc reserves
        // answer EINVAL, which leaves them as they are.
        unsafe { libc::signal(signal, libc::SIG_DFL) };
    }
}

/// Marks every descriptor above the standard three close-on-exec, for a child about to exec, so it inherits only
/// stdin, stdout and stderr (upstream's `POSIX_SPAWN_CLOEXEC_DEFAULT`): std opens its own descriptors close-on-exec,
/// but one the caller leaked would otherwise reach the child. Marked rather than closed, so std's own exec-error pipe
/// still reports a failed exec. `close_range` is a plain system call, safe between `fork` and `exec`; a kernel older
/// than 5.11 refuses the flag and the descriptors stay as they are.
#[cfg(target_os = "linux")]
pub fn close_inherited_descriptors() {
    // SAFETY: `close_range` takes plain integers and only changes descriptor flags.
    unsafe { libc::syscall(libc::SYS_close_range, 3_u32, u32::MAX, libc::CLOSE_RANGE_CLOEXEC) };
}

/// Marks every descriptor above the standard three close-on-exec, for a child about to exec, as the Linux form
/// does; macOS has no `close_range`, so each descriptor below the process's limit (capped, as a raised limit can be
/// in the millions) gets `FD_CLOEXEC` by `fcntl`, which is async-signal-safe. A number that is not open answers
/// `EBADF` and is skipped.
#[cfg(all(unix, not(target_os = "linux")))]
pub fn close_inherited_descriptors() {
    /// The highest descriptor number marked: far above what the CLI's caller leaks in practice.
    const MAX_DESCRIPTORS: libc::c_int = 1 << 16;
    // SAFETY: `getdtablesize` only reads the process's descriptor limit.
    let limit = unsafe { libc::getdtablesize() }.clamp(3, MAX_DESCRIPTORS);
    for fd in 3..limit {
        // SAFETY: `fcntl` takes any descriptor number; one that is not open is an error, not undefined behavior.
        unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
    }
}

/// Running a test as the real binary runs, with SIGPIPE at its default action. std ignores SIGPIPE before the test
/// harness's `main`, and only `ngtermctl`'s own `main` restores the default, so a test of [`SigpipeIgnored`] run in the
/// harness passes whether or not the guard does anything. Setting the default in the harness itself would let a
/// concurrent test's `EPIPE` kill the whole test binary, so the test runs again, alone, in a fresh copy of the binary.
#[cfg(test)]
#[cfg(unix)]
pub mod default_sigpipe {
    use std::os::unix::process::ExitStatusExt;
    use std::process::Command;

    /// Names the test a re-run copy of the binary is running with SIGPIPE at its default.
    const PROBE: &str = "NGTERMCTL_TEST_DEFAULT_SIGPIPE";

    /// Runs `body` with SIGPIPE at its default action: in the copy of the binary [`PROBE`] names `test` to (its full
    /// path, `module::tests::name`), it sets the default and runs `body`; anywhere else it re-runs `test` alone in such
    /// a copy and asserts the copy passed, rather than failing or dying of SIGPIPE.
    pub fn run(test: &str, body: impl FnOnce()) {
        if std::env::var(PROBE).is_ok_and(|probed| probed == test) {
            // SAFETY: `signal` with a valid signal number and `SIG_DFL` installs no handler code.
            unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
            body();
            return;
        }
        let output = Command::new(std::env::current_exe().unwrap())
            .args([test, "--exact", "--test-threads=1", "--nocapture"])
            .env(PROBE, test)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{test} with SIGPIPE at its default: {:?} (signal {:?})\n{}{}",
            output.status.code(),
            output.status.signal(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        // the filter matched the test: a mistyped name would pass with no test run
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("1 passed"),
            "{test} did not run"
        );
    }
}

#[cfg(test)]
#[cfg(windows)]
mod windows_tests {
    use std::process::Command;

    use super::*;

    #[test]
    fn an_exit_code_reads_as_itself() {
        let exited = Command::new("cmd.exe").args(["/d", "/c", "exit 7"]).status().unwrap();
        assert_eq!(exit_status(exited), 7);
    }
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use std::process::Command;

    use super::*;

    #[test]
    fn an_exit_code_reads_as_itself_and_a_signal_as_its_number() {
        let exited = Command::new("/bin/sh").args(["-c", "exit 7"]).status().unwrap();
        assert_eq!(exit_status(exited), 7);
        let killed = Command::new("/bin/sh").args(["-c", "kill -TERM $$"]).status().unwrap();
        assert_eq!(exit_status(killed), libc::SIGTERM);
    }

    /// SIGPIPE's current action, read back by installing `SIG_IGN` and putting the old one back.
    fn sigpipe_action() -> libc::sighandler_t {
        // SAFETY: installing `SIG_IGN` runs no handler code.
        let current = unsafe { libc::signal(libc::SIGPIPE, libc::SIG_IGN) };
        // SAFETY: `current` is the action `signal` just returned for SIGPIPE.
        unsafe { libc::signal(libc::SIGPIPE, current) };
        current
    }

    #[test]
    fn sigpipe_is_ignored_while_the_guard_lives_and_default_again_after() {
        default_sigpipe::run(
            "local::tests::sigpipe_is_ignored_while_the_guard_lives_and_default_again_after",
            || {
                assert_eq!(sigpipe_action(), libc::SIG_DFL);
                {
                    let _ignored = SigpipeIgnored::new();
                    assert_eq!(sigpipe_action(), libc::SIG_IGN);
                }
                assert_eq!(sigpipe_action(), libc::SIG_DFL);
            },
        );
    }
}
