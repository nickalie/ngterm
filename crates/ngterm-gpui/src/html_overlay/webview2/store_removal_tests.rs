//! Profile removal against a real Windows file handle that temporarily refuses deletion.
//!
//! `remove_later` tries every 2 s, 10 times; with its schedule fixed, the child paces itself by the clock from the
//! moment it handed the folders over, a second clear of each try it times.

use std::os::windows::fs::OpenOptionsExt as _;
use std::time::{Duration, Instant};

use super::*;

#[test]
fn closed_profiles_are_retried_and_missing_profiles_are_harmless() {
    crate::system_window::tests::run_child("html_overlay::webview2::store::removal_tests::removal_child");
}

fn wait_removed(folder: &Path) {
    let deadline = Instant::now() + Duration::from_secs(6);
    while folder.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!folder.exists(), "{} was not removed", folder.display());
}

/// A profile folder `name` under `dir` with a file a browser still holds: no try can remove it until it is dropped.
fn held_profile(dir: &Path, name: &str) -> (PathBuf, std::fs::File) {
    let folder = dir.join(name);
    std::fs::create_dir(&folder).unwrap();
    let held = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .share_mode(0)
        .open(folder.join("held"))
        .unwrap();
    (folder, held)
}

fn sleep_until(began: Instant, elapsed: Duration) {
    std::thread::sleep(elapsed.saturating_sub(began.elapsed()));
}

#[test]
#[ignore = "run by closed_profiles_are_retried_and_missing_profiles_are_harmless"]
fn removal_child() {
    if !crate::system_window::tests::is_child("html_overlay::webview2::store::removal_tests::removal_child") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (last_try, released_late) = held_profile(dir.path(), "last-try");
    let (exhausted, retained) = held_profile(dir.path(), "still-open");
    let began = Instant::now();
    remove_later(last_try.clone());
    remove_later(exhausted.clone());
    let immediate = dir.path().join("closed");
    std::fs::create_dir(&immediate).unwrap();
    std::fs::write(immediate.join("data"), "profile").unwrap();
    remove_later(immediate.clone());
    wait_removed(&immediate);
    let (locked, file) = held_profile(dir.path(), "locked");
    remove_later(locked.clone());
    std::thread::sleep(Duration::from_millis(2300));
    drop(file);
    wait_removed(&locked);
    remove_later(dir.path().join("missing"));
    // between the ninth try (18 s) and the tenth (20 s)
    sleep_until(began, Duration::from_secs(19));
    drop(released_late);
    wait_removed(&last_try);
    // past the tenth try: a remover still trying would take the folder at 22 s
    sleep_until(began, Duration::from_secs(21));
    drop(retained);
    sleep_until(began, Duration::from_millis(23_500));
    assert!(
        exhausted.join("held").exists(),
        "a profile still busy after ten tries is left for the next start"
    );
    std::fs::remove_dir_all(&exhausted).unwrap();
}
