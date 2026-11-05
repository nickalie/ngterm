//! Storing an upload: its name made safe on every OS, exactly the announced bytes or nothing, a fresh directory each,
//! and the old ones pruned.

use std::fs::{self, File};
use std::io::{self, Cursor, Read};
use std::time::{Duration, SystemTime};

use crate::agent_chat::upload::{KEEP_FOR, UPLOAD_CAP, UploadError, prune, safe_name, store, uploads_dir};

fn now() -> SystemTime {
    // early 2027
    SystemTime::UNIX_EPOCH + Duration::from_hours(500_000)
}

#[test]
fn a_name_keeps_only_its_last_component_and_what_every_os_takes() {
    assert_eq!(safe_name("photo.png"), "photo.png");
    assert_eq!(safe_name("../../etc/passwd"), "passwd");
    assert_eq!(safe_name(r"C:\Users\me\report.pdf"), "report.pdf");
    assert_eq!(safe_name("a:b*c?.txt"), "a_b_c_.txt");
    assert_eq!(safe_name("tab\there.md"), "tab_here.md");
    assert_eq!(safe_name("notes. . "), "notes");
    assert_eq!(safe_name("Скриншот 2026.png"), "Скриншот 2026.png");
}

#[test]
fn a_device_name_or_nothing_left_gets_a_name_of_its_own() {
    assert_eq!(safe_name("con.txt"), "_con.txt");
    assert_eq!(safe_name("LPT1"), "_LPT1");
    assert_eq!(safe_name("console.txt"), "console.txt");
    assert_eq!(safe_name(""), "upload");
    assert_eq!(safe_name("dir/"), "upload");
    assert_eq!(safe_name(".."), "upload");
}

#[test]
fn a_long_name_is_cut_on_a_character_keeping_its_extension() {
    let long = format!("{}.jpeg", "ж".repeat(100));
    let safe = safe_name(&long);
    assert!(safe.len() <= 120, "{} bytes", safe.len());
    assert!(safe.ends_with("ж.jpeg"), "{safe}");
}

#[test]
fn an_upload_lands_whole_under_a_fresh_directory_of_its_own() {
    let state = tempfile::tempdir().unwrap();
    let dir = uploads_dir(state.path());
    let first = store(&dir, "shot.png", 3, &mut Cursor::new(b"png"), now()).unwrap();
    let second = store(&dir, "shot.png", 2, &mut Cursor::new(b"ok"), now()).unwrap();
    assert_eq!(fs::read(&first).unwrap(), b"png");
    assert_eq!(fs::read(&second).unwrap(), b"ok");
    assert_ne!(first.parent(), second.parent());
    assert_eq!(first.file_name(), second.file_name());
    assert!(first.starts_with(&dir));
}

#[test]
fn input_short_of_or_past_the_announced_size_leaves_nothing() {
    let state = tempfile::tempdir().unwrap();
    let dir = uploads_dir(state.path());
    let short = store(&dir, "a.txt", 10, &mut Cursor::new(b"abc"), now());
    assert!(matches!(short, Err(UploadError::Short { got: 3, size: 10 })), "{short:?}");
    let long = store(&dir, "a.txt", 2, &mut Cursor::new(b"abc"), now());
    assert!(matches!(long, Err(UploadError::Long { size: 2 })), "{long:?}");
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
}

#[test]
fn a_file_over_the_cap_is_refused_before_a_byte_is_read() {
    struct Untouched;
    impl Read for Untouched {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            panic!("read an upload that is over the cap")
        }
    }
    let state = tempfile::tempdir().unwrap();
    let refused = store(&uploads_dir(state.path()), "big.bin", UPLOAD_CAP + 1, &mut Untouched, now());
    assert!(matches!(refused, Err(UploadError::TooLarge { .. })), "{refused:?}");
}

#[test]
fn pruning_removes_only_the_uploads_older_than_a_week() {
    let state = tempfile::tempdir().unwrap();
    let dir = uploads_dir(state.path());
    let path = store(&dir, "kept.txt", 1, &mut Cursor::new(b"k"), now()).unwrap();
    let modified = fs::metadata(path.parent().unwrap()).unwrap().modified().unwrap();
    prune(&dir, modified + KEEP_FOR - Duration::from_secs(60));
    assert!(path.exists());
    prune(&dir, modified + KEEP_FOR + Duration::from_secs(60));
    assert!(!path.exists());
    // a stray file in the uploads directory is not an upload
    let stray = dir.join("stray");
    File::create(&stray).unwrap();
    prune(&dir, modified + KEEP_FOR * 4);
    assert!(stray.exists());
}
