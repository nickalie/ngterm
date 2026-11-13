// The child-process tests run on Unix, and under coverage everywhere.
#[cfg(any(unix, coverage))]
use std::path::Path;
#[cfg(any(unix, coverage))]
use std::process::Command;

/// What tells a child test below it runs as the child: its scratch directory.
#[cfg(any(unix, coverage))]
const CHILD: &str = "COVERAGE_TEST_CHILD";
/// What the child test does: `write` or `skip` the profile before ending past its exit handlers, or `wait` for a
/// termination signal with the handler installed (`wait-profile`: the real one, writing the profile).
#[cfg(any(unix, coverage))]
const MODE: &str = "COVERAGE_TEST_MODE";

/// This test binary run as the child test `name`, with the scratch directory `dir` and `mode`.
#[cfg(any(unix, coverage))]
fn child_test(name: &str, dir: &Path, mode: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().expect("this test binary"));
    command
        .args([name, "--exact", "--test-threads=1", "--nocapture"])
        .env(CHILD, dir)
        .env(MODE, mode);
    command
}

#[test]
fn writing_the_profile_leaves_the_process_running() {
    super::write_profile();
    super::write_profile();
}

#[test]
fn installing_the_termination_profile_write_leaves_the_process_running() {
    super::write_profile_on_termination();
}

/// compiler-rt's raw profile magic (`__llvm_profile_raw_magic`: 0xff, "lprofr", 0x81), little-endian on every target
/// CI builds.
#[cfg(coverage)]
const RAW_MAGIC: u64 = 0xff6c_7072_6f66_7281;

#[cfg(coverage)]
fn is_raw_profile(path: &Path) -> bool {
    std::fs::read(path)
        .is_ok_and(|bytes| bytes.first_chunk::<8>().map(|head| u64::from_le_bytes(*head)) == Some(RAW_MAGIC))
}

/// Under coverage (`cargo llvm-cov test -p ngterm-session`): a process that writes its profile and then ends past its
/// exit handlers, as the app's `exit_now` does, leaves a raw profile; one that skips the write leaves none, which shows
/// the exit handlers did not run. Each runs as this test binary's [`child_ends_past_its_exit_handlers`].
#[cfg(coverage)]
#[test]
fn a_process_ending_past_its_exit_handlers_keeps_its_profile_only_when_written() {
    let dir = tempfile::tempdir().expect("temporary directory");
    for (mode, written) in [("write", true), ("skip", false)] {
        let profile = dir.path().join(format!("{mode}.profraw"));
        let status = child_test("coverage::tests::child_ends_past_its_exit_handlers", dir.path(), mode)
            .env("LLVM_PROFILE_FILE", &profile)
            .status()
            .expect("the child test");
        assert!(status.success(), "{mode}: {status}");
        if written {
            assert!(is_raw_profile(&profile), "{mode}: no raw profile");
        } else {
            assert!(!profile.exists(), "{mode}: the exit handlers ran");
        }
    }
}

/// The child of the test above; nothing when run on its own.
#[cfg(coverage)]
#[test]
fn child_ends_past_its_exit_handlers() {
    if std::env::var_os(CHILD).is_none() {
        return;
    }
    if std::env::var(MODE).as_deref() == Ok("write") {
        super::write_profile();
    }
    #[cfg(unix)]
    // SAFETY: `_exit` only ends the process.
    unsafe {
        libc::_exit(0)
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, TerminateProcess};
        // SAFETY: the current process's pseudo-handle; nothing of this process runs after the call.
        unsafe { TerminateProcess(GetCurrentProcess(), 0) };
    }
}

#[cfg(unix)]
mod termination {
    use std::os::unix::process::{CommandExt as _, ExitStatusExt as _};
    use std::path::Path;
    use std::process::Child;
    use std::time::{Duration, Instant};

    use super::{CHILD, child_test};

    const WAITER: &str = "coverage::tests::termination::child_waits_for_a_termination_signal";

    /// The child test waiting for a signal with the handler on (`mode`), its termination signals at their default
    /// action but `ignored`, as a process started with that one ignored has it, and its profile, if any, into
    /// `profile`; once it says it is ready.
    fn waiter(dir: &Path, mode: &str, ignored: Option<i32>, profile: Option<&Path>) -> Child {
        let mut command = child_test(WAITER, dir, mode);
        if let Some(profile) = profile {
            command.env("LLVM_PROFILE_FILE", profile);
        }
        let reset = move || {
            for signal in super::super::TERMINATION {
                let action = if Some(signal) == ignored {
                    libc::SIG_IGN
                } else {
                    libc::SIG_DFL
                };
                // SAFETY: `signal` with `SIG_IGN` or `SIG_DFL` installs no handler code.
                unsafe { libc::signal(signal, action) };
            }
            Ok(())
        };
        // SAFETY: the hook only calls `signal`, which is async-signal-safe and allocates nothing: fit for the forked
        // child before exec.
        unsafe { command.pre_exec(reset) };
        let child = command.spawn().expect("the child test");
        let deadline = Instant::now() + Duration::from_secs(30);
        while !dir.join("ready").exists() {
            assert!(Instant::now() < deadline, "the child test never got ready");
            std::thread::sleep(Duration::from_millis(10));
        }
        child
    }

    fn signal(child: &Child, signal: i32) {
        let pid = libc::pid_t::try_from(child.id()).expect("a pid");
        // SAFETY: `kill` takes integers only; the pid is this test's own running child.
        let sent = unsafe { libc::kill(pid, signal) };
        assert_eq!(sent, 0, "{}", std::io::Error::last_os_error());
    }

    #[test]
    fn a_termination_signal_runs_the_action_then_ends_the_process_by_its_default_action() {
        for number in super::super::TERMINATION {
            let dir = tempfile::tempdir().expect("temporary directory");
            let mut child = waiter(dir.path(), "wait", None, None);
            signal(&child, number);
            let status = child.wait().expect("the child's status");
            assert_eq!(status.signal(), Some(number), "{status}");
            assert!(
                dir.path().join("ran").exists(),
                "signal {number}: the action did not run"
            );
        }
    }

    #[test]
    fn a_signal_ignored_at_start_stays_ignored() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let mut child = waiter(dir.path(), "wait", Some(libc::SIGHUP), None);
        signal(&child, libc::SIGHUP);
        std::thread::sleep(Duration::from_millis(300));
        assert!(
            child.try_wait().expect("the child's state").is_none(),
            "an ignored hangup ended it"
        );
        assert!(!dir.path().join("ran").exists(), "the action ran on an ignored hangup");
        signal(&child, libc::SIGTERM);
        assert_eq!(child.wait().expect("the child's status").signal(), Some(libc::SIGTERM));
        assert!(dir.path().join("ran").exists());
    }

    /// Under coverage: a hangup (an ssh login's pty closing under an attach client) ends the process by the signal,
    /// its profile written first.
    #[cfg(coverage)]
    #[test]
    fn a_hangup_ends_an_instrumented_process_with_its_profile_written() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let profile = dir.path().join("hangup.profraw");
        let mut child = waiter(dir.path(), "wait-profile", None, Some(&profile));
        signal(&child, libc::SIGHUP);
        assert_eq!(child.wait().expect("the child's status").signal(), Some(libc::SIGHUP));
        assert!(super::is_raw_profile(&profile), "no raw profile");
    }

    #[cfg(target_os = "linux")]
    const UNSTARTED: &str = "coverage::tests::termination::child_cannot_start_the_thread";

    /// A thread that cannot start (`RLIMIT_NPROC` binds threads on Linux only, and not root) leaves SIGTERM its default
    /// action.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_thread_that_cannot_start_leaves_sigterm_its_default_action() {
        // SAFETY: a plain call.
        if unsafe { libc::geteuid() } == 0 {
            eprintln!("skipped: RLIMIT_NPROC does not bind root");
            return;
        }
        let dir = tempfile::tempdir().expect("temporary directory");
        let mut command = super::child_test(UNSTARTED, dir.path(), "unstarted");
        let reset = || {
            // SAFETY: `SIG_DFL` installs no handler code.
            unsafe { libc::signal(libc::SIGTERM, libc::SIG_DFL) };
            Ok(())
        };
        // SAFETY: the hook only calls `signal`, async-signal-safe and allocation-free, fit for the forked child.
        unsafe { command.pre_exec(reset) };
        let status = command.status().expect("the child test");
        assert_eq!(status.signal(), Some(libc::SIGTERM), "{status}");
    }

    /// The child of the test above; nothing when run on its own.
    #[cfg(target_os = "linux")]
    #[test]
    fn child_cannot_start_the_thread() {
        if std::env::var_os(CHILD).is_none() {
            return;
        }
        let none = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: a plain call with a live local.
        assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_NPROC, &raw const none) }, 0);
        assert!(
            super::super::before_termination(mark).is_err(),
            "the thread started after all"
        );
        // SAFETY: raising a signal at this process.
        unsafe { libc::raise(libc::SIGTERM) };
        eprintln!("survived SIGTERM");
    }

    /// Marks the action's run in the child's scratch directory.
    fn mark() {
        if let Some(dir) = std::env::var_os(CHILD) {
            let _ = std::fs::write(Path::new(&dir).join("ran"), "");
        }
    }

    /// The child of the tests above; nothing when run on its own. Installs the handler (the real profile write for
    /// `wait-profile`, else [`mark`]), says it is ready and waits to be ended by a signal.
    #[test]
    fn child_waits_for_a_termination_signal() {
        let Some(dir) = std::env::var_os(CHILD) else {
            return;
        };
        if std::env::var(super::MODE).as_deref() == Ok("wait-profile") {
            super::super::write_profile_on_termination();
        } else {
            super::super::before_termination(mark).expect("the handler");
        }
        std::fs::write(Path::new(&dir).join("ready"), "").expect("say ready");
        std::thread::sleep(Duration::from_secs(60));
        panic!("no signal ended the child");
    }
}
