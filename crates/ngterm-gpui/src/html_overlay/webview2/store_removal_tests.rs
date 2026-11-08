//! Profile removal against a real Windows file handle that temporarily refuses deletion.

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

#[test]
#[ignore = "run by closed_profiles_are_retried_and_missing_profiles_are_harmless"]
fn removal_child() {
    if !crate::system_window::tests::is_child("html_overlay::webview2::store::removal_tests::removal_child") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let exhausted = dir.path().join("still-open");
    std::fs::create_dir(&exhausted).unwrap();
    let retained = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .share_mode(0)
        .open(exhausted.join("held"))
        .unwrap();
    let began = Instant::now();
    remove_later(exhausted.clone());
    let immediate = dir.path().join("closed");
    std::fs::create_dir(&immediate).unwrap();
    std::fs::write(immediate.join("data"), "profile").unwrap();
    remove_later(immediate.clone());
    wait_removed(&immediate);
    let locked = dir.path().join("locked");
    std::fs::create_dir(&locked).unwrap();
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .share_mode(0)
        .open(locked.join("held"))
        .unwrap();
    remove_later(locked.clone());
    std::thread::sleep(Duration::from_millis(2300));
    assert!(locked.exists(), "an open browser handle keeps the profile alive");
    drop(file);
    wait_removed(&locked);
    remove_later(dir.path().join("missing"));
    std::thread::sleep(Duration::from_millis(2300));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    std::thread::sleep(Duration::from_secs(21).saturating_sub(began.elapsed()));
    assert!(
        exhausted.join("held").exists(),
        "a permanently busy profile is left for the next start"
    );
    drop(retained);
    std::fs::remove_dir_all(&exhausted).unwrap();
}
