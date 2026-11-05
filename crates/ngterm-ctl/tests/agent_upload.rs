//! `ngtermctl agent upload` as the Android app runs it over ssh: the file's bytes on standard input, its stored path
//! printed, nothing stored when the input stops short. Every run's environment is the test's own
//! (`support::isolated`), its state directory a fresh one.

#[expect(dead_code, reason = "the helpers the other agent tests use and this one does not")]
mod support;

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};

use support::{NGTERMCTL, TestResult, isolated};

fn upload(root: &Path, name: &str, size: usize, input: &[u8]) -> TestResult<Output> {
    let mut child = isolated(NGTERMCTL, root)
        .env("NGTERM_STATE_DIR", root.join("state"))
        .args(["agent", "upload", "--name", name, "--size", &size.to_string()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child.stdin.take().ok_or("no stdin")?.write_all(input)?;
    Ok(child.wait_with_output()?)
}

#[test]
fn the_bytes_land_under_the_state_dirs_uploads_and_the_path_is_printed() -> TestResult {
    let root = tempfile::tempdir()?;
    let bytes = (0..=255_u8).cycle().take(200_000).collect::<Vec<u8>>();
    let output = upload(root.path(), "../screen shot.png", bytes.len(), &bytes)?;
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let path = PathBuf::from(String::from_utf8(output.stdout)?.trim_end());
    assert!(path.starts_with(root.path().join("state").join("uploads")), "{}", path.display());
    assert_eq!(path.file_name().and_then(|name| name.to_str()), Some("screen shot.png"));
    assert_eq!(fs::read(&path)?, bytes);
    Ok(())
}

#[test]
fn input_that_stops_short_stores_nothing_and_says_why() -> TestResult {
    let root = tempfile::tempdir()?;
    let output = upload(root.path(), "notes.txt", 10, b"abc")?;
    assert!(!output.status.success());
    let said = String::from_utf8_lossy(&output.stderr);
    assert!(said.contains("the upload stopped after 3 of 10 bytes"), "{said}");
    let uploads = root.path().join("state").join("uploads");
    assert_eq!(fs::read_dir(uploads)?.count(), 0);
    Ok(())
}

#[test]
fn a_size_over_the_cap_is_a_usage_error() -> TestResult {
    let root = tempfile::tempdir()?;
    let output = upload(root.path(), "big.bin", 64 * 1024 * 1024, b"")?;
    // a usage error, upstream's exit 64, before a byte is read
    assert_eq!(output.status.code(), Some(64), "{}", String::from_utf8_lossy(&output.stderr));
    Ok(())
}
