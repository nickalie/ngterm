//! Bounded reads of a file named by untrusted output (upstream's `BoundedFile`): the file is opened once and judged by
//! what that descriptor names, so a symlink is measured at its target, and a FIFO, device, offloaded placeholder or
//! file over the caller's cap is refused before a byte is read. A FIFO is opened without blocking, so a writer that
//! never comes stalls nothing. Files open with the platform's default sharing (on Windows read, write and delete), so a
//! log's writer can still append to it, rename it away or replace it while a page reads.
//!
//! The app's page probes read through it (`ngterm-app`'s `documents`), and so does an origin serving a content page's
//! files to the presenting machine (`crate::remote::page_serve`).

use std::fs::{File, Metadata, OpenOptions};
use std::io::Read as _;
use std::path::Path;

/// Why a bounded read failed (upstream's `BoundedFile.Failure`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// Missing, not a regular file, unreadable, or cut short under the read.
    Unreadable,
    /// Over the caller's cap.
    TooLarge,
}

/// The whole regular file at `path`, through the one descriptor it checked (upstream's `BoundedFile.read`); a file
/// that grew past `maximum` since it was named is refused, as its size is read from the open descriptor.
///
/// # Errors
///
/// [`Failure::TooLarge`] over `maximum`, else [`Failure::Unreadable`] for what [`opened`] refuses or a read that
/// fails.
pub fn read(path: &Path, maximum: usize) -> Result<Vec<u8>, Failure> {
    let (mut file, size) = checked(path, maximum)?;
    let mut bytes = vec![0; size];
    file.read_exact(&mut bytes).map_err(|_| Failure::Unreadable)?;
    Ok(bytes)
}

/// `path` opened and judged without reading (upstream's `BoundedFile.check`): the open file and its size.
///
/// # Errors
///
/// As [`read`].
pub fn checked(path: &Path, maximum: usize) -> Result<(File, usize), Failure> {
    let (file, metadata) = opened(path)?;
    let size = usize::try_from(metadata.len()).map_err(|_| Failure::TooLarge)?;
    if size > maximum {
        return Err(Failure::TooLarge);
    }
    Ok((file, size))
}

/// `path` opened for reading, with what its descriptor names: a regular file, never a FIFO, device or offloaded
/// placeholder. The default sharing is kept (module docs).
///
/// # Errors
///
/// [`Failure::Unreadable`] for anything else, or a file that cannot be opened.
pub fn opened(path: &Path) -> Result<(File, Metadata), Failure> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    let file = options.open(path).map_err(|_| Failure::Unreadable)?;
    let metadata = file.metadata().map_err(|_| Failure::Unreadable)?;
    if !metadata.is_file() || dataless(&metadata) || !regular_handle(&file) {
        return Err(Failure::Unreadable);
    }
    Ok((file, metadata))
}

#[cfg(target_os = "macos")]
fn dataless(metadata: &Metadata) -> bool {
    use std::os::macos::fs::MetadataExt as _;
    // SF_DATALESS, from Apple's xnu/bsd/sys/stat.h (not yet exposed by libc).
    metadata.st_flags() & 0x4000_0000 != 0
}

#[cfg(not(target_os = "macos"))]
fn dataless(_: &Metadata) -> bool {
    false
}

#[cfg(windows)]
fn regular_handle(file: &File) -> bool {
    use std::os::windows::io::AsRawHandle as _;
    use windows_sys::Win32::Storage::FileSystem::{FILE_TYPE_DISK, GetFileType};
    // SAFETY: the borrowed File owns a live handle throughout this query.
    unsafe { GetFileType(file.as_raw_handle()) == FILE_TYPE_DISK }
}

#[cfg(not(windows))]
fn regular_handle(_: &File) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_reads_whole_under_its_cap_and_is_refused_over_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, "abc").unwrap();
        assert_eq!(read(&path, 3), Ok(b"abc".to_vec()));
        assert_eq!(read(&path, 2), Err(Failure::TooLarge));
        assert_eq!(read(&dir.path().join("missing"), 3), Err(Failure::Unreadable));
        assert_eq!(read(dir.path(), 3), Err(Failure::Unreadable), "a folder");
    }

    /// A FIFO is refused without waiting for a writer.
    #[cfg(unix)]
    #[test]
    fn a_fifo_is_refused_without_blocking() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fifo");
        let name = std::ffi::CString::new(path.to_string_lossy().as_bytes()).unwrap();
        // SAFETY: a valid NUL-terminated path and mode.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert_eq!(read(&path, 10), Err(Failure::Unreadable));
    }
}
