//! `ngterm --build-identity`, which `scripts/stage.sh` writes the staged identity file from: the binary prints the
//! identity it was built with, in the file's format, and ends before the app starts. It runs with an empty
//! environment, so a binary that ignored the flag finds no display, bus or state to reach and fails instead of drawing.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long the query may take; it starts nothing, so a run this long means the app started.
const DEADLINE: Duration = Duration::from_secs(30);

#[test]
fn build_identity_prints_the_compiled_identity_and_exits() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ngterm"))
        .arg(ngterm_gpui::app::BUILD_IDENTITY_FLAG)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let started = Instant::now();
    while child.try_wait().unwrap().is_none() {
        if started.elapsed() > DEADLINE {
            child.kill().unwrap();
            panic!("ngterm --build-identity still running after {DEADLINE:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{:?}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert_eq!(text, ngterm_gpui::app::build_identity_text());
    assert_eq!(
        ngterm_core::identity::staged_identity(&text),
        Some(ngterm_gpui::app::identity())
    );
}
