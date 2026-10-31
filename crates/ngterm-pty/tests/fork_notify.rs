//! macOS: a PTY child forked while another thread makes the process's first libnotify use still runs its program.
//! Without libnotify's setup finished before the fork, such a child died of `SIGILL` in libnotify's fork handler
//! (`os_once_t is corrupt`) before its exec, and the spawn still reported success.
//!
//! The race is open only until a process's first libnotify use, and a fork lands in it now and then, so the test runs
//! it in fresh processes: this binary again, for one round each.
#![cfg(target_os = "macos")]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ngterm_pty::{Command, Size, Spawned};

const TEST: &str = "a_child_forked_during_the_first_libnotify_use_runs";
/// Set for a process that runs one round.
const ROUND: &str = "FORK_NOTIFY_ROUND";
/// Rounds: without the fix a fork caught the first libnotify use in about one round in twenty.
const ROUNDS: usize = 60;

/// Spawn `true` on PTYs until `stop`; the statuses of the children that a signal ended.
fn spawn_until(stop: &AtomicBool) -> Vec<String> {
    let mut children = Vec::new();
    while !stop.load(Ordering::Acquire) && children.len() < 500 {
        let Spawned { master, child } = ngterm_pty::spawn(&Command::new("/usr/bin/true"), Size::cells(24, 80))
            .unwrap_or_else(|err| panic!("spawning failed: {err}"));
        children.push((master, child));
    }
    children
        .into_iter()
        .filter_map(|(_master, mut child)| {
            let status = child.wait().unwrap_or_else(|err| panic!("waiting failed: {err}"));
            status.signal().map(|signal| format!("{status:?} ({signal})"))
        })
        .collect()
}

/// One round: PTY children spawned on three threads while this one makes the process's first libnotify use.
fn round() {
    let stop = Arc::new(AtomicBool::new(false));
    let spawners: Vec<_> = (0..3)
        .map(|_| {
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || spawn_until(&stop))
        })
        .collect();
    std::thread::sleep(Duration::from_millis(20));
    // the first libnotify use, as portable-pty makes it for a command without `HOME`
    // SAFETY: `getpwuid` returns a pointer into libc's static entry, which is not read here.
    unsafe { libc::getpwuid(libc::getuid()) };
    std::thread::sleep(Duration::from_millis(20));
    stop.store(true, Ordering::Release);
    let killed: Vec<String> = spawners
        .into_iter()
        .flat_map(|spawner| spawner.join().unwrap_or_else(|_| panic!("a spawner panicked")))
        .collect();
    assert!(killed.is_empty(), "children killed before their exec: {killed:?}");
}

#[test]
fn a_child_forked_during_the_first_libnotify_use_runs() {
    if std::env::var_os(ROUND).is_some() {
        round();
        return;
    }
    let this = std::env::current_exe().unwrap_or_else(|err| panic!("no test binary: {err}"));
    for _ in 0..ROUNDS {
        let out = std::process::Command::new(&this)
            .args([TEST, "--exact", "--test-threads=1"])
            .env(ROUND, "1")
            .output()
            .unwrap_or_else(|err| panic!("running a round failed: {err}"));
        assert!(
            out.status.success(),
            "a round failed: {}",
            String::from_utf8_lossy(&out.stdout)
        );
    }
}
