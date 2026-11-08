//! Hand a validated address to the user's default browser.
use std::process::{Command, ExitStatus};

pub fn open(uri: &str) -> bool {
    open_with(uri, Command::status)
}

/// The browser hand-off, with the process launch supplied by the caller.
fn open_with(uri: &str, launch: impl FnOnce(&mut Command) -> std::io::Result<ExitStatus>) -> bool {
    launch(Command::new("/usr/bin/open").arg("--").arg(uri)).is_ok_and(|status| status.success())
}

#[cfg(test)]
#[path = "browser_tests.rs"]
mod tests;
