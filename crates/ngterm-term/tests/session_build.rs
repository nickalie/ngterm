#![cfg(feature = "local")]
//! A test of this crate alone finds no cargo-built `ngterm-session` beside it and builds it (`session_bin`): the
//! executable is moved aside here, in a test binary of its own, so no live test runs while it is gone. Ignored by
//! default: in a workspace run the rebuild resolves `ngterm-session`'s features alone, which can recompile its
//! dependencies and replace the executable the later test binaries run. Run it with
//! `cargo test -p ngterm-term --test session_build -- --ignored`. The other tests here check where that build writes.

mod session_bin;

use std::path::Path;
use std::process::Command;

use session_bin::{session_executable, target_args};

#[test]
#[ignore = "rebuilds the shared ngterm-session executable: run it on its own"]
fn a_missing_session_executable_is_built_before_its_first_use() -> Result<(), Box<dyn std::error::Error>> {
    let test = std::env::current_exe()?;
    let profile = test
        .parent()
        .and_then(Path::parent)
        .ok_or("no target profile directory above the test binary")?;
    let expected = profile.join(ngterm_session::launch::EXECUTABLE_FILE);
    let aside = expected.with_extension(format!("aside-{}", std::process::id()));
    if expected.is_file() {
        std::fs::rename(&expected, &aside)?;
    }
    let built = session_executable();
    if built.is_err() && aside.is_file() {
        std::fs::rename(&aside, &expected)?;
    } else {
        // on Windows a server of an earlier test binary, still in its idle grace, keeps its executable
        let _ = std::fs::remove_file(&aside);
    }
    assert_eq!(built?, expected);
    let version = Command::new(&expected).arg("version").output()?;
    assert!(version.status.success(), "{version:?}");
    Ok(())
}

#[test]
fn the_nested_build_writes_to_the_target_directory_of_the_test() -> Result<(), Box<dyn std::error::Error>> {
    let target = tempfile::tempdir()?;
    std::fs::write(target.path().join(".rustc_info.json"), "{}")?;
    let profile = target.path().join("debug");
    assert_eq!(
        target_args(&profile),
        ["--target-dir".into(), target.path().as_os_str().to_owned()]
    );
    Ok(())
}

#[test]
fn the_nested_build_keeps_the_target_triple_of_the_test() -> Result<(), Box<dyn std::error::Error>> {
    let target = tempfile::tempdir()?;
    std::fs::write(target.path().join(".rustc_info.json"), "{}")?;
    let profile = target.path().join("x86_64-unknown-linux-gnu").join("release");
    assert_eq!(
        target_args(&profile),
        [
            "--target-dir".into(),
            target.path().as_os_str().to_owned(),
            "--target".into(),
            "x86_64-unknown-linux-gnu".into(),
        ]
    );
    Ok(())
}
