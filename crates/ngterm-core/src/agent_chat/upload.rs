//! Files the Android chat view attaches to a prompt: `ngtermctl agent upload` takes a file's bytes on standard input
//! and stores them where the pane's agent can read them, and the phone puts the stored path into the prompt (Claude
//! Code and Codex read a path in a prompt; an image's path makes Claude Code look at the image).
//!
//! - **Where:** `<state dir>/uploads/<id>/<name>` ([`uploads_dir`]), on the origin, so the agent's project is left
//!   alone (no stray files in a repository's working tree). An agent may ask before reading outside its project; the
//!   chat shows that prompt as any other.
//! - **The name** is the file's own, made safe on every OS ([`safe_name`]): no directory part, no control or
//!   reserved characters, no name Windows keeps for a device. Each upload gets a fresh directory ([`store`]), so two
//!   files of one name never collide and the name stays the one the user picked.
//! - **The size** is announced first and capped at [`UPLOAD_CAP`]; input that ends short of it (the phone cancelled,
//!   or its link went) or runs past it leaves nothing behind.
//! - **Keeping:** each upload first removes the uploads older than [`KEEP_FOR`] ([`prune`]).
//! - **Who reads it:** the user alone. On Unix the uploads directory and each upload's directory are `0700` and the
//!   file `0600`, whatever the umask, and an uploads directory found wider is narrowed; on Windows they keep the ACL
//!   they inherit from the user's profile, where the state directory lives.

use std::fs::{self, OpenOptions};
use std::io::{self, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The largest file an upload takes.
pub const UPLOAD_CAP: u64 = 32 * 1024 * 1024;

/// How long an upload is kept: long enough for the agent to read it, then the next upload removes it.
pub const KEEP_FOR: Duration = Duration::from_hours(7 * 24);

/// A name with nothing left after [`safe_name`] stands as this.
const FALLBACK_NAME: &str = "upload";

/// The most bytes of a stored name, well under every OS's limit on one path component.
const NAME_CAP: usize = 120;

/// Names Windows keeps for devices, whatever their extension.
const DEVICE_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2",
    "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Why an upload stored nothing.
#[derive(Debug, thiserror::Error)]
pub enum UploadError {
    /// The announced size is over [`UPLOAD_CAP`].
    #[error("the file is {size} bytes; an upload takes at most {cap} bytes")]
    TooLarge { size: u64, cap: u64 },
    /// The input ended before the announced size: the sender stopped.
    #[error("the upload stopped after {got} of {size} bytes")]
    Short { got: u64, size: u64 },
    /// The input ran past the announced size.
    #[error("the upload sent more than the {size} bytes it announced")]
    Long { size: u64 },
    #[error("could not store the upload: {0}")]
    Io(#[from] io::Error),
}

/// The uploads' directory in the state directory `state_dir`.
#[must_use]
pub fn uploads_dir(state_dir: &Path) -> PathBuf {
    state_dir.join("uploads")
}

/// `name` as a file name every OS takes: its last path component, without control characters or the ones Windows
/// reserves (each becomes `_`), without trailing dots and spaces, not a device name, at most [`NAME_CAP`] bytes with
/// its extension kept; [`FALLBACK_NAME`] when nothing is left.
#[must_use]
pub fn safe_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_end_matches(['.', ' ']);
    if trimmed.is_empty() || trimmed.chars().all(|c| c == '.') {
        return FALLBACK_NAME.to_owned();
    }
    let stem = trimmed.split('.').next().unwrap_or_default();
    let named = if DEVICE_NAMES
        .iter()
        .any(|device| device.eq_ignore_ascii_case(stem.trim()))
    {
        format!("_{trimmed}")
    } else {
        trimmed.to_owned()
    };
    capped(&named)
}

/// `name` cut to [`NAME_CAP`] bytes on a character boundary, its extension (a short one) kept.
fn capped(name: &str) -> String {
    if name.len() <= NAME_CAP {
        return name.to_owned();
    }
    let extension = name
        .rfind('.')
        .filter(|&dot| dot > 0 && name.len() - dot <= 16)
        .map_or("", |dot| &name[dot..]);
    let stem = &name[..name.len() - extension.len()];
    let end = stem.floor_char_boundary(NAME_CAP - extension.len());
    format!("{}{extension}", &stem[..end])
}

/// Stores `size` bytes read from `input` as `name` ([`safe_name`]) in a fresh directory under `dir`, named after
/// `now`, and returns the file's path. Nothing is left behind when it fails.
///
/// # Errors
///
/// [`UploadError::TooLarge`] before reading anything, [`UploadError::Short`] or [`UploadError::Long`] when the input
/// does not hold exactly `size` bytes, [`UploadError::Io`] when the directory or the file can't be written.
pub fn store(
    dir: &Path,
    name: &str,
    size: u64,
    input: &mut impl Read,
    now: SystemTime,
) -> Result<PathBuf, UploadError> {
    if size > UPLOAD_CAP {
        return Err(UploadError::TooLarge { size, cap: UPLOAD_CAP });
    }
    private_root(dir)?;
    let folder = fresh_folder(dir, now)?;
    let path = folder.join(safe_name(name));
    let written = write_exactly(&path, size, input);
    if let Err(error) = written {
        let _ = fs::remove_dir_all(&folder);
        return Err(error);
    }
    Ok(path)
}

/// Makes `dir` (and its missing parents) and leaves it its owner's alone on Unix: made `0700`, or narrowed to that
/// when it was there wider.
fn private_root(dir: &Path) -> io::Result<()> {
    if let Some(parent) = dir.parent() {
        fs::create_dir_all(parent)?;
    }
    if let Err(error) = private_dir(dir)
        && error.kind() != ErrorKind::AlreadyExists
    {
        return Err(error);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = fs::metadata(dir)?.permissions().mode();
        if mode & 0o077 != 0 {
            fs::set_permissions(dir, fs::Permissions::from_mode(mode & 0o700))?;
        }
    }
    Ok(())
}

/// Makes the directory `path`, its owner's alone on Unix (`0700`).
fn private_dir(path: &Path) -> io::Result<()> {
    let builder = fs::DirBuilder::new();
    // the mode is Unix's alone: elsewhere the builder is never changed, so it is not `mut` there
    #[cfg(unix)]
    let builder = {
        let mut builder = builder;
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder
    };
    builder.create(path)
}

/// A new directory under `dir`, named by `now` in milliseconds, with a count after it when that name is taken.
fn fresh_folder(dir: &Path, now: SystemTime) -> io::Result<PathBuf> {
    let millis = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_millis();
    for attempt in 0_u32.. {
        let name = if attempt == 0 {
            millis.to_string()
        } else {
            format!("{millis}-{attempt}")
        };
        let folder = dir.join(name);
        match private_dir(&folder) {
            Ok(()) => return Ok(folder),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    unreachable!("an unbounded count always finds a free name")
}

/// Copies exactly `size` bytes of `input` into a new file at `path`, its owner's alone on Unix (`0600`).
fn write_exactly(path: &Path, size: u64, input: &mut impl Read) -> Result<(), UploadError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(path)?;
    let got = io::copy(&mut input.take(size), &mut file)?;
    if got < size {
        return Err(UploadError::Short { got, size });
    }
    let mut extra = [0_u8; 1];
    if input.read(&mut extra)? > 0 {
        return Err(UploadError::Long { size });
    }
    file.flush()?;
    file.sync_all()?;
    Ok(())
}

/// Removes the uploads under `dir` last changed more than [`KEEP_FOR`] before `now`. What can't be read or removed
/// stays, for the next upload to try again.
pub fn prune(dir: &Path, now: SystemTime) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .is_ok_and(|changed| now.duration_since(changed).is_ok_and(|age| age > KEEP_FOR));
        if old && entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}
