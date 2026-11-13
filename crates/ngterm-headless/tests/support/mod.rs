//! The `ngterm-headless` binary the tests run.

use std::path::{Path, PathBuf};

/// Names the directory of another build's programs, whose `ngterm-headless` the tests run in place of cargo's own
/// build: scripts/test-mobile-e2e.sh --coverage points it at the instrumented build its e2e coverage is read against.
pub const BIN_DIR_VARIABLE: &str = "E2E_BIN_DIR";

/// The `ngterm-headless` binary: cargo's own build, or the one in [`BIN_DIR_VARIABLE`]'s directory when it is set.
pub fn binary() -> PathBuf {
    let own = Path::new(env!("CARGO_BIN_EXE_ngterm-headless"));
    let Some(dir) = std::env::var_os(BIN_DIR_VARIABLE).filter(|dir| !dir.is_empty()) else {
        return own.to_path_buf();
    };
    let other = Path::new(&dir).join(own.file_name().unwrap_or_default());
    assert!(other.is_file(), "{BIN_DIR_VARIABLE}: no {} in it", other.display());
    other
}
