//! The SIGTERM handler, each test in a child process of its own: a handler that could not be installed leaves SIGTERM
//! its default action (the install fails in a child that may start no thread, and the child's SIGTERM ends it), and a
//! SIGTERM marks the quit for the web worker. [`run_child`] and [`is_child`] also serve the other modules' tests of
//! that process-wide mark.

use std::os::unix::process::ExitStatusExt as _;
use std::process::{Command, ExitStatus};

/// Names the child test a process of its own runs.
const CHILD: &str = "NGTERM_GPUI_TEST_SIGNALS_CHILD";

/// Runs the ignored test `name` alone in a process of its own, with no `NGTERM_*` of ours.
pub(crate) fn run_child(name: &str) -> ExitStatus {
    let mut command = Command::new(std::env::current_exe().unwrap());
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("NGTERM_") {
            command.env_remove(key);
        }
    }
    command
        .args(["--exact", name, "--ignored", "--nocapture", "--test-threads=1"])
        .env(CHILD, name)
        .status()
        .unwrap()
}

/// Whether this process is [`run_child`]'s for `name`.
pub(crate) fn is_child(name: &str) -> bool {
    std::env::var(CHILD).as_deref() == Ok(name)
}

#[test]
fn a_failed_install_leaves_sigterm_its_default_action() {
    // SAFETY: a plain call.
    if unsafe { libc::geteuid() } == 0 {
        eprintln!("skipped: RLIMIT_NPROC does not bind root");
        return;
    }
    let status = run_child("signals::tests::sigterm_install_child");
    // a child SIGTERM ends writes no coverage profile: the parent's assertion is what counts
    assert_eq!(status.signal(), Some(libc::SIGTERM), "{status:?}");
}

#[test]
#[ignore = "run by a_failed_install_leaves_sigterm_its_default_action"]
fn sigterm_install_child() {
    if !is_child("signals::tests::sigterm_install_child") {
        return;
    }
    let none = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: a plain call with a live local.
    assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_NPROC, &raw const none) }, 0);
    let installed = super::quit_on_sigterm(crate::state::detached_poster());
    assert!(installed.is_err(), "the thread started after all");
    // SAFETY: raising a signal at this process.
    unsafe { libc::raise(libc::SIGTERM) };
    eprintln!("survived SIGTERM");
}

#[cfg(feature = "html")]
#[test]
fn a_sigterm_marks_the_quit_for_the_web_worker_at_once() {
    let status = run_child("signals::tests::sigterm_quit_mark_child");
    assert!(status.success(), "{status:?}");
}

#[cfg(feature = "html")]
#[test]
#[ignore = "run by a_sigterm_marks_the_quit_for_the_web_worker_at_once"]
fn sigterm_quit_mark_child() {
    if !is_child("signals::tests::sigterm_quit_mark_child") {
        return;
    }
    // a gone main loop: the mark cannot wait for it
    super::quit_on_sigterm(crate::state::detached_poster()).unwrap();
    assert!(!ngterm_wpe_client::worker::app_is_quitting());
    // SAFETY: raising a signal at this process, whose handler is installed.
    unsafe { libc::raise(libc::SIGTERM) };
    // raise returns once the handler ran on this thread: no other thread had to run for the mark
    assert!(ngterm_wpe_client::worker::app_is_quitting());
}
