//! The cargo-built `ngterm-session` of a test's profile (`target/<profile>/`, above the test binary's `deps/`), which
//! the live tests attach to: `cargo test` builds it beside them with the workspace or with `-p ngterm-session`, and a
//! test of this crate alone builds it on first use with `cargo build -p ngterm-session` (cargo lets go of its build
//! lock before it runs the tests), in the same target directory and profile and under the same environment, so an
//! instrumented run (scripts/unit-coverage.sh) gets an instrumented server.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// The cargo-built `ngterm-session` beside this test's profile, built once per test process when it is not there.
pub fn session_executable() -> Result<PathBuf, Box<dyn std::error::Error>> {
    static EXECUTABLE: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    EXECUTABLE.get_or_init(find_or_build).clone().map_err(Into::into)
}

fn find_or_build() -> Result<PathBuf, String> {
    let test = std::env::current_exe().map_err(|error| format!("no test executable path: {error}"))?;
    let profile = test
        .parent()
        .and_then(Path::parent)
        .ok_or("no target profile directory above the test binary")?;
    let executable = profile.join(ngterm_session::launch::EXECUTABLE_FILE);
    if executable.is_file() {
        return Ok(executable);
    }
    let mut build = Command::new(env!("CARGO"));
    build
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(["build", "-p", "ngterm-session", "--bin", "ngterm-session"]);
    match profile.file_name().and_then(|name| name.to_str()) {
        Some("debug") => {}
        Some("release") => {
            build.arg("--release");
        }
        Some(name) => {
            build.args(["--profile", name]);
        }
        None => return Err(format!("no profile name in {}", profile.display())),
    }
    let status = build
        .status()
        .map_err(|error| format!("`cargo build -p ngterm-session` did not start: {error}"))?;
    if !status.success() {
        return Err(format!("`cargo build -p ngterm-session` failed ({status})"));
    }
    if !executable.is_file() {
        return Err(format!(
            "`cargo build -p ngterm-session` left no {} (a `--target` or `--target-dir` given to `cargo test` reaches \
             no nested build: build it there with the same flags, or test with the workspace)",
            executable.display()
        ));
    }
    Ok(executable)
}
