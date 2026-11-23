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

use serde::{Deserialize, Serialize};

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

/// One version of a file (upstream's `BoundedFile.FileStamp`): a change of identity means it was replaced, of size or
/// modification time that it was written. Unix: the device and inode, the size and the modification time to the
/// nanosecond; Windows: the volume serial number and file index, the size and the last-write time
/// (`GetFileInformationByHandle`). On the wire (a log tail on the page channel) it is upstream's object, `device`,
/// `inode`, `size`, `modifiedSeconds` and `modifiedNanoseconds`, which the reader only hands back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "WireStamp", into = "WireStamp")]
pub struct FileStamp {
    /// The device or volume, then the inode or file index.
    pub identity: (u64, u64),
    pub size: u64,
    /// The modification time in the platform's own units: seconds and nanoseconds on Unix, 100 ns ticks on Windows.
    pub modified: (i64, i64),
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireStamp {
    device: u64,
    inode: u64,
    size: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
}

impl From<WireStamp> for FileStamp {
    fn from(wire: WireStamp) -> Self {
        Self {
            identity: (wire.device, wire.inode),
            size: wire.size,
            modified: (wire.modified_seconds, wire.modified_nanoseconds),
        }
    }
}

impl From<FileStamp> for WireStamp {
    fn from(stamp: FileStamp) -> Self {
        Self {
            device: stamp.identity.0,
            inode: stamp.identity.1,
            size: stamp.size,
            modified_seconds: stamp.modified.0,
            modified_nanoseconds: stamp.modified.1,
        }
    }
}

/// The end of a file (upstream's `BoundedFile.TailRead`): its bytes, the file offset they start at, and its stamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TailRead {
    pub data: Vec<u8>,
    pub offset: u64,
    pub stamp: FileStamp,
}

/// The last `max_bytes` of the regular file at `path` plus the byte before them, which tells
/// [`crate::html::log_tail::snapshot`] whether the window opens on a line boundary (upstream's `BoundedFile.readTail`).
/// Any size is accepted. `None` when the file still has `unchanged_from`'s stamp: nothing is read then.
///
/// # Errors
///
/// [`Failure::Unreadable`] when the file is missing, not a regular file, unreadable, or shrank under the read.
pub fn read_tail(path: &Path, max_bytes: u64, unchanged_from: Option<&FileStamp>) -> Result<Option<TailRead>, Failure> {
    let (file, metadata) = opened(path)?;
    let stamp = file_stamp(&file, &metadata).ok_or(Failure::Unreadable)?;
    if unchanged_from == Some(&stamp) {
        return Ok(None);
    }
    let offset = stamp.size.saturating_sub(max_bytes.saturating_add(1));
    let count = usize::try_from(stamp.size - offset).map_err(|_| Failure::Unreadable)?;
    let data = read_fully(&file, offset, count).ok_or(Failure::Unreadable)?;
    Ok(Some(TailRead { data, offset, stamp }))
}

/// `count` bytes of `file` from `offset`, positioned reads that leave the descriptor's own offset alone; `None` when
/// the file ends first, as one truncated under the read does.
fn read_fully(file: &File, offset: u64, count: usize) -> Option<Vec<u8>> {
    let mut data = vec![0; count];
    let mut filled = 0;
    while filled < count {
        match read_at(file, &mut data[filled..], offset + filled as u64) {
            Ok(0) => return None,
            Ok(read) => filled += read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
    Some(data)
}

#[cfg(unix)]
fn read_at(file: &File, buffer: &mut [u8], offset: u64) -> std::io::Result<usize> {
    std::os::unix::fs::FileExt::read_at(file, buffer, offset)
}

#[cfg(windows)]
fn read_at(file: &File, buffer: &mut [u8], offset: u64) -> std::io::Result<usize> {
    std::os::windows::fs::FileExt::seek_read(file, buffer, offset)
}

#[cfg(unix)]
#[expect(
    clippy::unnecessary_wraps,
    reason = "Windows' stamp comes from a query that can fail; both platforms share the signature"
)]
fn file_stamp(_: &File, metadata: &Metadata) -> Option<FileStamp> {
    use std::os::unix::fs::MetadataExt as _;
    Some(FileStamp {
        identity: (metadata.dev(), metadata.ino()),
        size: metadata.len(),
        modified: (metadata.mtime(), metadata.mtime_nsec()),
    })
}

#[cfg(windows)]
fn file_stamp(file: &File, _: &Metadata) -> Option<FileStamp> {
    use std::os::windows::io::AsRawHandle as _;
    use windows_sys::Win32::Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle};
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: the borrowed File owns a live handle throughout this query, and `info` is a valid out pointer.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &raw mut info) } == 0 {
        return None;
    }
    let wide = |high: u32, low: u32| (u64::from(high) << 32) | u64::from(low);
    let written = wide(info.ftLastWriteTime.dwHighDateTime, info.ftLastWriteTime.dwLowDateTime);
    Some(FileStamp {
        identity: (
            u64::from(info.dwVolumeSerialNumber),
            wide(info.nFileIndexHigh, info.nFileIndexLow),
        ),
        size: wide(info.nFileSizeHigh, info.nFileSizeLow),
        modified: (i64::try_from(written).unwrap_or(i64::MAX), 0),
    })
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
