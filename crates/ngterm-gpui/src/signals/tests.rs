//! A handler that could not be installed leaves SIGTERM its default action: the install fails in a child of the test
//! that may start no thread, and the child's SIGTERM ends it.

use std::os::unix::process::ExitStatusExt as _;
use std::process::Command;

/// Names the child that installs the handler with no thread to spare.
const CHILD: &str = "NGTERM_GPUI_TEST_SIGTERM_INSTALL_CHILD";

#[test]
fn a_failed_install_leaves_sigterm_its_default_action() {
    // SAFETY: a plain call.
    if unsafe { libc::geteuid() } == 0 {
        eprintln!("skipped: RLIMIT_NPROC does not bind root");
        return;
    }
    let mut command = Command::new(std::env::current_exe().unwrap());
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("NGTERM_") {
            command.env_remove(key);
        }
    }
    let status = command
        .args([
            "--exact",
            "signals::tests::sigterm_install_child",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD, "1")
        .status()
        .unwrap();
    // a child SIGTERM ends writes no coverage profile: the parent's assertion is what counts
    assert_eq!(status.signal(), Some(libc::SIGTERM), "{status:?}");
}

#[test]
#[ignore = "run by a_failed_install_leaves_sigterm_its_default_action"]
fn sigterm_install_child() {
    if std::env::var_os(CHILD).is_none() {
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
