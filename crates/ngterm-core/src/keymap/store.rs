//! Reading `keymap.conf` from disk (upstream's `KeymapStore.load`) and seeding the starter file when there is none.

use std::fs;
use std::io::{self, Write as _};
use std::path::Path;

use super::KeymapDiagnostic;
use super::defaults::ShippedDefaults;
use super::parse::parse_keymap;
use super::parsed::Keymap;
use super::starter::starter_keymap_conf;
use crate::hooks::{read_config_text, read_error_reason};

/// Loads and parses the keymap at `path`. A missing file is the empty keymap with no diagnostics; one that exists
/// but cannot be read as UTF-8 text is the empty keymap with a single line-0 diagnostic, worded as `hooks.conf`'s.
#[must_use]
pub fn load_keymap(path: &Path, defaults: &ShippedDefaults) -> (Keymap, Vec<KeymapDiagnostic>) {
    let unreadable = |reason: String| {
        let message = format!("could not read keymap.conf: {reason}");
        (Keymap::empty(defaults), vec![KeymapDiagnostic::new(0, message)])
    };
    match read_config_text(path) {
        Ok(text) => parse_keymap(&text, defaults),
        Err(error) if error.kind() == io::ErrorKind::NotFound => (Keymap::empty(defaults), Vec::new()),
        Err(error) => unreadable(read_error_reason(&error)),
    }
}

/// Writes the starter `keymap.conf` at `path` when no file is there, creating its directory. Never overwrites: an
/// existing file, even one created concurrently, is left alone. A dangling symlink counts as no file and is replaced
/// by the starter, its target left uncreated (upstream's `fileExists` check and atomic write). Failures are logged,
/// not returned, since a missing starter only costs the user the commented reference.
pub fn ensure_starter_keymap(path: &Path, defaults: &ShippedDefaults) {
    if let Err(error) = write_starter(path, || starter_keymap_conf(defaults)) {
        tracing::warn!(path = %path.display(), %error, "could not write the starter keymap.conf");
    }
}

pub(super) fn write_starter(path: &Path, starter: impl FnOnce() -> String) -> io::Result<()> {
    if path.try_exists()? {
        return Ok(());
    }
    let dangling_link = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Some(fs::read_link(path)?),
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    if let Some(directory) = path.parent() {
        fs::create_dir_all(directory)?;
    }
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(starter().as_bytes())?;
    file.as_file().sync_all()?;
    if let Some(target) = dangling_link {
        // a changed or unreadable entry is left alone, including a link whose target now exists.
        if !matches!(fs::read_link(path), Ok(current) if current == target) || path.try_exists()? {
            return Ok(());
        }
        return file.persist(path).map(|_| ()).map_err(|error| error.error);
    }
    match file.persist_noclobber(path) {
        Ok(_) => Ok(()),
        Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error.error),
    }
}
