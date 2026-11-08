//! Profile removal against a real Windows file handle that temporarily refuses deletion.
//!
//! The tests own `remove_retrying`'s pause: it counts the tries and closes the handle between two of them, so the
//! tries are observed rather than timed. Only the last test runs `remove_later` itself, on its real schedule.

use std::os::windows::fs::OpenOptionsExt as _;
use std::time::{Duration, Instant};

use super::*;

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

/// Removes `folder` with no real pause, closing `held` just before try number `release_before`: the tries it took.
fn tries_releasing_before(folder: &Path, held: std::fs::File, release_before: u32) -> u32 {
    let mut held = Some(held);
    let mut tries = 0;
    remove_retrying(folder, || {
        tries += 1;
        if tries == release_before {
            drop(held.take());
        }
    });
    tries
}

#[test]
fn a_profile_still_held_is_retried_and_goes_on_the_try_after_its_handle_closes() {
    let dir = tempfile::tempdir().unwrap();
    let (folder, held) = held_profile(dir.path(), "locked");
    assert_eq!(tries_releasing_before(&folder, held, 2), 2);
    assert!(!folder.exists(), "the second try removes the released folder");
}

#[test]
fn a_profile_released_just_before_the_tenth_try_is_still_removed() {
    let dir = tempfile::tempdir().unwrap();
    let (folder, held) = held_profile(dir.path(), "last-try");
    assert_eq!(tries_releasing_before(&folder, held, 10), 10);
    assert!(!folder.exists(), "the tenth try removes the released folder");
}

#[test]
fn a_profile_still_held_after_ten_tries_is_left_for_the_next_start() {
    let dir = tempfile::tempdir().unwrap();
    let (folder, held) = held_profile(dir.path(), "still-open");
    // an eleventh try never comes, so the handle outlives all of them
    assert_eq!(tries_releasing_before(&folder, held, 11), 10);
    assert!(folder.join("held").exists(), "the folder stays for the next start");
}

#[test]
fn a_missing_profile_ends_the_tries_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let mut tries = 0;
    remove_retrying(&dir.path().join("missing"), || tries += 1);
    assert_eq!(tries, 1);
}

#[test]
fn remove_later_gives_the_browser_its_pause_and_then_removes_the_folder() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("closed");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("data"), "profile").unwrap();
    let began = Instant::now();
    remove_later(folder.clone());
    let deadline = began + Duration::from_secs(60);
    while folder.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!folder.exists(), "{} was not removed", folder.display());
    // a lower bound only, with room for a Windows timer tick: a slow machine can't fail it
    assert!(
        began.elapsed() >= Duration::from_millis(1900),
        "the first try waits out the 2 s pause"
    );
}
