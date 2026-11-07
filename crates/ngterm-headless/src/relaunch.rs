//! The restart a confirmed Live sessions reset quits for (the GPUI frontend's relauncher, `live_runtime.rs`): the
//! reset marker the quit armed is consumed by the next launch, which kills the selected sessions before any pane
//! attaches. On Unix the binary replaces this process in place (`exec`): the same pid, so a service manager sees no
//! exit, and the state lock goes with the old image's descriptors. The cost: a child the old image started (the
//! session server, a hook still running) outlives the threads that reaped it, so one that exits later stays a zombie
//! until this process exits (`docs/backlog/headless-exec-restart-leaves-old-children-unreaped.md`). On Windows a new
//! instance starts on this console and waits for the lock this process holds until it exits ([`LOCK_WAIT`]).

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::Options;

/// The flag a relaunched instance starts with: it waits for the state lock rather than refusing at once.
pub const RELAUNCHED_FLAG: &str = "--relaunched";

/// How long a relaunched instance waits for the state lock its predecessor still holds, and how often it tries.
pub const LOCK_WAIT: Duration = Duration::from_secs(10);
pub const LOCK_POLL: Duration = Duration::from_millis(200);

/// The arguments the next instance runs with: this one's grid and agent setup, and [`RELAUNCHED_FLAG`].
#[must_use]
pub fn arguments(options: &Options) -> Vec<OsString> {
    let (columns, rows) = options.grid;
    let mut arguments: Vec<OsString> = vec!["--grid".into(), format!("{columns}x{rows}").into()];
    if options.skip_agent_setup {
        arguments.push("--skip-agent-setup".into());
    } else if let Some(version) = options.opencode_version {
        arguments.extend(["--opencode-version".into(), version.raw().into()]);
    }
    arguments.push(RELAUNCHED_FLAG.into());
    arguments
}

/// The binary to start again, as `current_exe` names it: itself while it exists, else, for a file Linux reports
/// deleted (`<path> (deleted)` once an upgrade replaced it), the file now at `<path>` (the GPUI frontend's
/// `reopen_target`); `None` when neither exists.
#[must_use]
pub fn target(executable: &Path) -> Option<PathBuf> {
    if executable.exists() {
        return Some(executable.to_path_buf());
    }
    let replacement = without_deleted_suffix(executable)?;
    replacement.exists().then_some(replacement)
}

#[cfg(unix)]
fn without_deleted_suffix(executable: &Path) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt as _;
    /// What Linux appends to `/proc/self/exe` once the running binary's file was unlinked or replaced.
    const DELETED_SUFFIX: &[u8] = b" (deleted)";
    let stripped = executable.as_os_str().as_bytes().strip_suffix(DELETED_SUFFIX)?;
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(stripped)))
}

#[cfg(not(unix))]
fn without_deleted_suffix(_: &Path) -> Option<PathBuf> {
    None
}

fn executable() -> io::Result<PathBuf> {
    let current = std::env::current_exe()?;
    target(&current).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "the binary to restart is gone"))
}

/// Replaces this process with a fresh instance of the binary. It returns only when that failed.
#[cfg(unix)]
#[must_use]
pub fn relaunch(options: &Options) -> io::Error {
    use std::os::unix::process::CommandExt as _;
    match executable() {
        Ok(executable) => {
            // `exec` runs no exit handler: an instrumented build's coverage profile is written first
            ngterm_app::coverage::write_profile();
            std::process::Command::new(executable).args(arguments(options)).exec()
        }
        Err(error) => error,
    }
}

/// Starts a fresh instance of the binary on this console, which takes over once this process exited.
///
/// # Errors
///
/// The binary is gone or did not start.
#[cfg(windows)]
pub fn relaunch(options: &Options) -> io::Result<()> {
    std::process::Command::new(executable()?)
        .args(arguments(options))
        .spawn()
        .map(drop)
}

#[cfg(test)]
#[path = "relaunch_tests.rs"]
mod tests;
