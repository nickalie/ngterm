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
/// existing file or directory, even one created concurrently, is left alone. A dangling symlink counts as no file and is
/// replaced by the starter, its target left uncreated (upstream's `fileExists` check and atomic write). The link is
/// moved aside first and the starter placed only where nothing has appeared since, so a file written at the path at
/// any moment survives, and anything else found there is moved back. One case is left to timing: a link whose target
/// is created in the instant between that last look and the placement is still replaced, its target untouched.
/// Failures are logged, not returned, since a missing starter only costs the user the commented reference.
pub fn ensure_starter_keymap(path: &Path, defaults: &ShippedDefaults) {
    if let Err(error) = write_starter(path, || starter_keymap_conf(defaults), |_| {}) {
        tracing::warn!(path = %path.display(), %error, "could not write the starter keymap.conf");
    }
}

/// Where [`write_starter`] stands when it calls its `step` seam, which tests use to change the path at that moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StarterStep {
    /// The entry at the path was moved aside and is about to be inspected.
    MovedAside,
    /// The moved entry is the captured dangling link and the starter is about to be placed.
    Placing,
}

/// [`ensure_starter_keymap`]'s body, which returns its failure and calls `step` at each [`StarterStep`].
pub(super) fn write_starter(
    path: &Path,
    starter: impl FnOnce() -> String,
    step: impl FnMut(StarterStep),
) -> io::Result<()> {
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
    match dangling_link {
        Some(target) => replace_dangling_link(path, directory, &target, file, step),
        None => place(file, path),
    }
}

/// Moves the staged starter to `path` unless an entry is there, which is then kept.
fn place(file: tempfile::NamedTempFile, path: &Path) -> io::Result<()> {
    match file.persist_noclobber(path) {
        Ok(_) => Ok(()),
        Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error.error),
    }
}

/// Replaces the dangling link to `target` at `path` with the staged starter, never renaming over the path: the entry
/// there is moved into a fresh directory beside it, and only when it is that same link, still dangling, does the
/// starter take its place without clobbering; the link goes with the directory. Any other entry is moved back. No
/// standard call exchanges two names on every platform, so moving aside is the portable form.
fn replace_dangling_link(
    path: &Path,
    directory: &Path,
    target: &Path,
    file: tempfile::NamedTempFile,
    mut step: impl FnMut(StarterStep),
) -> io::Result<()> {
    let aside = tempfile::tempdir_in(directory)?;
    let moved = aside.path().join("keymap.conf");
    match fs::rename(path, &moved) {
        Ok(()) => {}
        // the link is gone, so the path is missing like any other.
        Err(error) if error.kind() == io::ErrorKind::NotFound => return place(file, path),
        Err(error) => return Err(error),
    }
    step(StarterStep::MovedAside);
    // a relative target resolves against the link's own directory, not the one it was moved into.
    let still_dangling = !directory.join(target).try_exists().unwrap_or(true);
    if !(still_dangling && matches!(fs::read_link(&moved), Ok(current) if current == target)) {
        return put_back(aside, &moved, path);
    }
    step(StarterStep::Placing);
    place(file, path).or_else(|error| put_back(aside, &moved, path).and(Err(error)))
}

/// Moves `moved` back to `path` without clobbering, then removes the emptied `aside` directory. When `path` was taken
/// meanwhile, the entry stays in `aside`, which the error names.
fn put_back(aside: tempfile::TempDir, moved: &Path, path: &Path) -> io::Result<()> {
    // the entry must outlive every failure below, so the directory is no longer removed on drop.
    let aside = aside.keep();
    let restored = tempfile::TempPath::try_from_path(moved).and_then(|mut entry| {
        entry.disable_cleanup(true);
        entry.persist_noclobber(path).map_err(|error| error.error)
    });
    match restored {
        Ok(()) => fs::remove_dir(&aside),
        Err(error) => Err(io::Error::new(
            error.kind(),
            format!(
                "could not move {} back from {}: {error}",
                path.display(),
                aside.display()
            ),
        )),
    }
}
