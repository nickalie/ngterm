//! Bounded local image/Markdown/text/HTML reads (upstream `ImageProbe.swift`, `MarkdownProbe.swift`, `TextProbe.swift`,
//! `DocumentProbe.swift`, `BoundedFile`), and a log's end ([`load_log`]).
//! Click handlers and renderers call these services on a worker thread. The control path performs its initial
//! probe synchronously, matching upstream; even opening a regular file can wait on network storage.
//!
//! Every read opens its file once and judges what that descriptor names ([`ngterm_core::bounded_file`]): a symlink is
//! measured at its target, and a FIFO or device is refused before a byte is read. Files open with the platform's default
//! sharing, so a log's writer can still append to it, rename it away or replace it while a page reads.

use std::collections::HashMap;
use std::fs::{File, Metadata};
use std::io::Cursor;
use std::path::{Path, PathBuf};

use image::{ImageDecoder as _, ImageFormat, ImageReader};
use ngterm_core::bounded_file::{self, Failure};
use ngterm_core::html::image_page::{self, ImageData};
use ngterm_core::html::markdown_page::{self, Embedded, MarkdownDocument};
use ngterm_core::html::{ReplicaContent, document_page};
use ngterm_core::html::{log_tail, text_page};
use ngterm_core::remote::replica_page::ContentFailures;

/// Read one raster image, judging exactly the descriptor whose bytes are sniffed and probed.
///
/// # Errors
/// The upstream image-probe refusal, including the supplied path.
pub fn probe_image(path: &Path) -> Result<ImageData, String> {
    load_image(path, image_page::MAX_BYTES)
}

fn load_image(path: &Path, max_bytes: usize) -> Result<ImageData, String> {
    let bytes = bounded_file::read(path, max_bytes).map_err(|failure| image_failure(failure, &path.display()))?;
    image_from_bytes(bytes, &path.display().to_string())
}

/// Judges bytes already in hand as [`probe_image`] judges a file's (upstream's `ImageProbe.probe`), a picture fetched
/// from another machine included; `label` only names it in a refusal, its path there.
///
/// # Errors
/// The upstream image-probe refusal with `label`: not a picture, or over the pixel cap.
pub fn image_from_bytes(bytes: Vec<u8>, label: &str) -> Result<ImageData, String> {
    let not_image = || format!("not a png, jpg, gif or webp image: {label}");
    let mime_type = image_page::mime_type(&bytes).ok_or_else(not_image)?;
    let format = match mime_type {
        "image/png" => ImageFormat::Png,
        "image/jpeg" => ImageFormat::Jpeg,
        "image/gif" => ImageFormat::Gif,
        "image/webp" => ImageFormat::WebP,
        _ => return Err(not_image()),
    };
    let (width, height, orientation) = if format == ImageFormat::WebP {
        // the decoder reads the RIFF container alone; bytes after it are ignored, as libwebp ignores them
        let riff = webp_riff_end(&bytes).map(|end| &bytes[..end]).ok_or_else(not_image)?;
        let mut decoder = webp_header_decoder(riff, riff.len()).map_err(|_| not_image())?;
        let (width, height) = checked_dimensions(decoder.dimensions(), label)?;
        let orientation = decoder
            .exif_metadata()
            .ok()
            .flatten()
            .as_deref()
            .and_then(image::metadata::Orientation::from_exif_chunk)
            .map_or(1, image::metadata::Orientation::to_exif);
        (width, height, orientation)
    } else {
        let mut decoder = header_decoder(&bytes, format, image_page::MAX_BYTES as u64).map_err(|_| not_image())?;
        let (width, height) = checked_dimensions(decoder.dimensions(), label)?;
        let orientation = decoder.orientation().map_or(1, image::metadata::Orientation::to_exif);
        (width, height, orientation)
    };
    Ok(ImageData {
        bytes,
        mime_type: mime_type.to_owned(),
        width,
        height,
        orientation,
    })
}

fn checked_dimensions((width, height): (u32, u32), label: &str) -> Result<(u32, u32), String> {
    if width == 0 || height == 0 {
        return Err(format!("not a png, jpg, gif or webp image: {label}"));
    }
    if !image_page::allows_pixels(u64::from(width), u64::from(height)) {
        return Err(format!("image too large (over 100 megapixels): {label}"));
    }
    Ok((width, height))
}

fn webp_header_decoder(
    bytes: &[u8],
    metadata_limit: usize,
) -> Result<image_webp::WebPDecoder<Cursor<&[u8]>>, image_webp::DecodingError> {
    // Audited image-webp 0.2.4: construction indexes known chunk kinds plus at most two first-frame subchunks;
    // its animation metadata read is capped at six bytes. Dimensions returns stored fields, and EXIF checks
    // this limit before allocating.
    // This covers our header/EXIF path only: the library's limit does not cover all pixel-decoding allocations.
    let mut decoder = image_webp::WebPDecoder::new(Cursor::new(bytes))?;
    decoder.set_memory_limit(metadata_limit.min(bytes.len()));
    Ok(decoder)
}

/// Where the RIFF container ends when every chunk in it, padding included, lies within it and it lies within `bytes`;
/// `None` otherwise. Bytes after the container are allowed, as libwebp (and upstream's `ImageIO`) allow them.
fn webp_riff_end(bytes: &[u8]) -> Option<usize> {
    // Keep the complete outer RIFF container within the accepted input, independently of the decoder's
    // metadata allocation budget (which also covers ranges it indexes from animation frame subchunks).
    let header = bytes.get(..12)?;
    if &header[..4] != b"RIFF" || &header[8..] != b"WEBP" {
        return None;
    }
    let size = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
    let end = usize::try_from(size).ok()?.checked_add(8)?;
    if end > bytes.len() {
        return None;
    }
    let mut offset = 12;
    while offset < end {
        let header = bytes.get(offset..end)?.get(..8)?;
        let size = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
        let next = usize::try_from(size)
            .ok()
            .and_then(|size| offset.checked_add(8)?.checked_add(size)?.checked_add(size & 1))?;
        if next > end {
            return None;
        }
        offset = next;
    }
    (offset == end).then_some(end)
}

fn header_decoder(
    bytes: &[u8],
    format: ImageFormat,
    allocation_limit: u64,
) -> image::ImageResult<impl image::ImageDecoder + '_> {
    // PNG can inflate compressed metadata while constructing its decoder, before dimensions are available.
    // Set its allocation budget first, independently of the caller's pixel cap; neither step decodes pixels.
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(allocation_limit);
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits);
    reader.into_decoder()
}

/// Check a Markdown candidate without reading or decoding it.
///
/// # Errors
/// The upstream unreadable/oversize Markdown refusal.
pub fn check_markdown(path: &Path) -> Result<(), String> {
    bounded_file::checked(path, markdown_page::MAX_BYTES)
        .map(|_| ())
        .map_err(|failure| markdown_failure(failure, &path.display()))
}

/// Read UTF-8 Markdown and embed bounded, locally-contained raster references.
///
/// # Errors
/// The upstream Markdown-probe refusal; invalid/unreadable pictures are omitted rather than failing the page.
pub fn probe_markdown(path: &Path) -> Result<MarkdownDocument, String> {
    let bytes = bounded_file::read(path, markdown_page::MAX_BYTES)
        .map_err(|failure| markdown_failure(failure, &path.display()))?;
    let markdown = String::from_utf8(bytes).map_err(|_| format!("not a UTF-8 text file: {}", path.display()))?;
    let images = path
        .parent()
        .map_or_else(Embedded::default, |folder| embed(&markdown, folder));
    Ok(MarkdownDocument { markdown, images })
}

/// Read a text page's file and validate it (upstream's `TextProbe.load`): the read itself checks the content, so a
/// binary file fails a click and is revealed instead of opening an empty page.
///
/// # Errors
/// Upstream's text-probe refusal: `unreadable text file: P` (missing, not a regular file, unreadable), `text file too
/// large (over 4 MB): P`, `not a UTF-8 text file: P` (invalid UTF-8 or a NUL byte).
pub fn load_text(path: &Path) -> Result<String, String> {
    let bytes =
        bounded_file::read(path, text_page::MAX_BYTES).map_err(|failure| text_failure(failure, &path.display()))?;
    text_from_bytes(bytes, &path.display().to_string())
}

/// [`load_text`] with the text dropped: what `session.overlay.open --text` and a click check before a page takes its
/// slot (upstream's `TextProbe.check`).
///
/// # Errors
/// [`load_text`]'s refusals.
pub fn check_text(path: &Path) -> Result<(), String> {
    load_text(path).map(|_| ())
}

/// One version of a file (upstream's `BoundedFile.FileStamp`): a change of identity means it was replaced, of size or
/// modification time that it was written. Unix: the device and inode, the size and the modification time to the
/// nanosecond; Windows: the volume serial number and file index, the size and the last-write time
/// (`GetFileInformationByHandle`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStamp {
    /// The device or volume, then the inode or file index.
    pub identity: (u64, u64),
    pub size: u64,
    /// The modification time in the platform's own units: seconds and nanoseconds on Unix, 100 ns ticks on Windows.
    pub modified: (i64, i64),
}

/// The end of a file (upstream's `BoundedFile.TailRead`): its bytes, the file offset they start at, and its stamp.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TailRead {
    data: Vec<u8>,
    offset: u64,
    stamp: FileStamp,
}

/// The last `max_bytes` of the regular file at `path` plus the byte before them, which tells
/// [`log_tail::snapshot`] whether the window opens on a line boundary (upstream's `BoundedFile.readTail`). Any size is
/// accepted. `None` when the file still has `unchanged_from`'s stamp: nothing is read then.
///
/// Fails as unreadable when the file is missing, not a regular file, unreadable, or shrank under the read.
fn read_tail(path: &Path, max_bytes: u64, unchanged_from: Option<&FileStamp>) -> Result<Option<TailRead>, Failure> {
    let (file, metadata) = bounded_file::opened(path)?;
    let stamp = file_stamp(&file, &metadata).ok_or(Failure::Unreadable)?;
    if unchanged_from == Some(&stamp) {
        return Ok(None);
    }
    let offset = stamp.size.saturating_sub(max_bytes.saturating_add(1));
    let count = usize::try_from(stamp.size - offset).map_err(|_| Failure::Unreadable)?;
    let data = read_fully(&file, offset, count).ok_or(Failure::Unreadable)?;
    Ok(Some(TailRead { data, offset, stamp }))
}

/// What a log page shows and the version of the file it came from (upstream's `TextProbe.LogRead`); `snapshot` is
/// `None` when the file's end is binary, so an unchanged binary file is not read again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogRead {
    pub snapshot: Option<log_tail::Snapshot>,
    pub stamp: FileStamp,
}

/// The last `tail` lines of `path` within [`text_page::MAX_TAIL_BYTES`] (upstream's `TextProbe.loadLog`); `None` when
/// the file still has `unchanged_from`'s stamp, which costs one metadata query and no read.
///
/// # Errors
/// `unreadable text file: P`.
pub fn load_log(path: &Path, tail: usize, unchanged_from: Option<&FileStamp>) -> Result<Option<LogRead>, String> {
    let read = read_tail(path, text_page::MAX_TAIL_BYTES as u64, unchanged_from)
        .map_err(|_| format!("unreadable text file: {}", path.display()))?;
    Ok(read.map(|read| LogRead {
        snapshot: log_tail::snapshot(&read.data, read.offset, tail),
        stamp: read.stamp,
    }))
}

/// Read the last `tail` lines of a log the way its page will (upstream's `TextProbe.checkLog`), so a binary or
/// unreadable file is refused at any size, and only for the lines the page shows.
///
/// # Errors
/// `unreadable text file: P`, `not a UTF-8 text file: P` (a NUL byte or invalid UTF-8 in the kept lines).
pub fn check_log(path: &Path, tail: usize) -> Result<(), String> {
    match load_log(path, tail, None)? {
        Some(LogRead { snapshot: Some(_), .. }) => Ok(()),
        _ => Err(format!("not a UTF-8 text file: {}", path.display())),
    }
}

/// Read a clicked HTML file and build its page with `page` ([`document_page::page`], or [`document_page::chromium_page`]
/// for a Chromium engine), as upstream's `DocumentProbe.load` does: the read checks the content, so a binary file fails
/// a click and is revealed instead of opening a failed page.
///
/// # Errors
/// Upstream's document-probe refusal: `unreadable html file: P` (missing, not a regular file, unreadable), `html file
/// too large (over 4 MB): P`, `not a UTF-8 html file: P` (invalid UTF-8 or a NUL byte).
pub fn load_document(path: &Path, page: fn(Vec<u8>) -> Option<String>) -> Result<String, String> {
    let bytes = bounded_file::read(path, document_page::MAX_BYTES)
        .map_err(|failure| document_failure(failure, &path.display()))?;
    document_from_bytes(bytes, &path.display().to_string(), page)
}

/// [`load_document`] with the page dropped: what a click checks before the page takes its slot.
///
/// # Errors
/// [`load_document`]'s refusals.
pub fn check_document(path: &Path) -> Result<(), String> {
    load_document(path, document_page::page).map(|_| ())
}

fn embed(markdown: &str, folder: &Path) -> Embedded {
    let Ok(root) = folder.canonicalize() else {
        return Embedded::default();
    };
    let mut embedding = Embedding::default();
    for image in markdown_page::image_references(markdown, folder) {
        // Lexical containment was established before this first filesystem access to the candidate.
        let canonical = canonical_candidate(&image.path);
        if canonical == root || !ngterm_core::html::contains(&root.to_string_lossy(), &canonical.to_string_lossy()) {
            continue;
        }
        let key = canonical.to_string_lossy().into_owned();
        let Some(budget) = embedding.room(&key, &image.reference) else {
            continue;
        };
        embedding.add(key, image.reference, load_image(&canonical, budget).ok());
    }
    embedding.embedded()
}

// Missing files still consume a probe and alias cache entry. Resolve their existing parent, too: on macOS
// `/var` itself is a symlink, so retaining the lexical spelling would incorrectly classify a failed read as outside.
fn canonical_candidate(path: &Path) -> PathBuf {
    let mut existing = path;
    let mut missing = Vec::new();
    loop {
        if let Ok(mut canonical) = existing.canonicalize() {
            for name in missing.into_iter().rev() {
                canonical.push(name);
            }
            return canonical;
        }
        let (Some(name), Some(parent)) = (existing.file_name(), existing.parent()) else {
            return path.to_path_buf();
        };
        missing.push(name);
        existing = parent;
    }
}

fn image_failure(failure: Failure, label: &dyn std::fmt::Display) -> String {
    match failure {
        Failure::Unreadable => format!("unreadable image: {label}"),
        Failure::TooLarge => format!("image too large (over 64 MB): {label}"),
    }
}

fn markdown_failure(failure: Failure, label: &dyn std::fmt::Display) -> String {
    match failure {
        Failure::Unreadable => format!("unreadable markdown file: {label}"),
        Failure::TooLarge => format!("markdown file too large (over 4 MB): {label}"),
    }
}

fn text_failure(failure: Failure, label: &dyn std::fmt::Display) -> String {
    match failure {
        Failure::Unreadable => format!("unreadable text file: {label}"),
        Failure::TooLarge => format!("text file too large (over {} MB): {label}", text_page::MAX_BYTES >> 20),
    }
}

fn document_failure(failure: Failure, label: &dyn std::fmt::Display) -> String {
    match failure {
        Failure::Unreadable => format!("unreadable html file: {label}"),
        Failure::TooLarge => format!(
            "html file too large (over {} MB): {label}",
            document_page::MAX_BYTES >> 20
        ),
    }
}

/// How a probe words a failed read of the file it names.
type Wording = fn(Failure, &dyn std::fmt::Display) -> String;

/// How a content page shown here for another machine reads `path` there: its kind's cap, and its failures worded as
/// the local probe of its kind words a file it could not read (upstream's `fetchReplica` `limit` and `local` pairs);
/// `None` for an HTML replica, which loads through its scheme instead.
#[must_use]
pub fn replica_read(content: ReplicaContent, path: &str) -> Option<(usize, ContentFailures)> {
    let (limit, words): (usize, Wording) = match content {
        ReplicaContent::Html => return None,
        ReplicaContent::Image { .. } => (image_page::MAX_BYTES, image_failure),
        ReplicaContent::Markdown => (markdown_page::MAX_BYTES, markdown_failure),
        ReplicaContent::Text { .. } => (text_page::MAX_BYTES, text_failure),
        ReplicaContent::Document => (document_page::MAX_BYTES, document_failure),
    };
    let failures = ContentFailures {
        too_large: words(Failure::TooLarge, &path),
        unreadable: words(Failure::Unreadable, &path),
    };
    Some((limit, failures))
}

/// Validates text bytes already in hand as [`load_text`] validates a file's (upstream's `TextProbe.text`), a file
/// fetched from another machine included; `label` only names it.
///
/// # Errors
/// `not a UTF-8 text file: label` (invalid UTF-8 or a NUL byte).
pub fn text_from_bytes(bytes: Vec<u8>, label: &str) -> Result<String, String> {
    text_page::validate(bytes).ok_or_else(|| format!("not a UTF-8 text file: {label}"))
}

/// Builds a clicked HTML file's page from bytes already in hand as [`load_document`] does from a file's (upstream's
/// `DocumentProbe.html`); `label` only names it.
///
/// # Errors
/// `not a UTF-8 html file: label`.
pub fn document_from_bytes(bytes: Vec<u8>, label: &str, page: fn(Vec<u8>) -> Option<String>) -> Result<String, String> {
    page(bytes).ok_or_else(|| format!("not a UTF-8 html file: {label}"))
}

/// Builds a Markdown page's document from bytes fetched from another machine (upstream's replica Markdown arm): the
/// references are listed lexically against `path`'s folder there by that machine's path rules, whatever OS this one
/// runs ([`markdown_page::remote_image_references`]), each picture read through `fetch` (its path there and the bytes
/// it may take) under the page's shared budget and judged here.
///
/// # Errors
/// `not a UTF-8 text file: path`.
pub fn markdown_from_bytes(
    bytes: Vec<u8>,
    path: &str,
    mut fetch: impl FnMut(&str, usize) -> Option<Vec<u8>>,
) -> Result<MarkdownDocument, String> {
    let markdown = String::from_utf8(bytes).map_err(|_| format!("not a UTF-8 text file: {path}"))?;
    let mut embedding = Embedding::default();
    for image in markdown_page::remote_image_references(&markdown, path) {
        let key = image.path;
        let Some(room) = embedding.room(&key, &image.reference) else {
            continue;
        };
        let picture = fetch(&key, room).and_then(|bytes| image_from_bytes(bytes, &key).ok());
        embedding.add(key, image.reference, picture);
    }
    Ok(MarkdownDocument {
        markdown,
        images: embedding.embedded(),
    })
}

/// The pictures one Markdown page embeds under its file, byte and pixel budgets, whichever machine read the bytes
/// (upstream's `MarkdownEmbedding`): each file once, keyed by `key`, and every reference naming it pointed at that one
/// copy.
#[derive(Debug, Default)]
pub struct Embedding {
    embedded: Embedded,
    read: HashMap<String, Option<usize>>,
    bytes: usize,
    pixels: u64,
}

impl Embedding {
    /// How many bytes reading the picture `key` may take, `None` when it needs no read: a file already read, whose
    /// `reference` is recorded here, or a spent budget.
    pub fn room(&mut self, key: &str, reference: &str) -> Option<usize> {
        if let Some(known) = self.read.get(key) {
            if let Some(index) = known {
                self.embedded.references.insert(reference.to_owned(), *index);
            }
            return None;
        }
        if self.read.len() >= markdown_page::MAX_IMAGES
            || self.bytes >= markdown_page::MAX_IMAGE_BYTES
            || self.pixels >= markdown_page::MAX_IMAGE_PIXELS
        {
            return None;
        }
        Some(markdown_page::MAX_IMAGE_BYTES - self.bytes)
    }

    /// Records what reading the picture `key` gave: a probed picture still inside the pixel budget is embedded, and
    /// anything else, a failed read included, leaves `reference` to its alt text.
    pub fn add(&mut self, key: String, reference: String, picture: Option<ImageData>) {
        let picture = picture.filter(|data| {
            self.pixels + u64::from(data.width) * u64::from(data.height) <= markdown_page::MAX_IMAGE_PIXELS
        });
        let Some(data) = picture else {
            self.read.insert(key, None);
            return;
        };
        self.bytes += data.bytes.len();
        self.pixels += u64::from(data.width) * u64::from(data.height);
        let index = self.embedded.uris.len();
        self.embedded.uris.push(data.data_uri());
        self.embedded.references.insert(reference, index);
        self.read.insert(key, Some(index));
    }

    /// What the page embeds.
    #[must_use]
    pub fn embedded(self) -> Embedded {
        self.embedded
    }
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
#[path = "documents/tests.rs"]
mod tests;
