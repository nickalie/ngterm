#![cfg(feature = "local")]
//! A test of this crate alone finds no cargo-built `ngterm-session` beside it and builds it (`session_bin`): the
//! executable is moved aside here, in a test binary of its own, so no live test runs while it is gone. Ignored by
//! default: in a workspace run the rebuild resolves `ngterm-session`'s features alone, which can recompile its
//! dependencies and replace the executable the later test binaries run. Run it with
//! `cargo test -p ngterm-term --test session_build -- --ignored`.

mod session_bin;

use std::path::Path;
use std::process::Command;

use session_bin::session_executable;

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
