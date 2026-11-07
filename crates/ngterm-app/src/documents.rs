//! Bounded local image/Markdown/text/HTML reads (upstream `ImageProbe.swift`, `MarkdownProbe.swift`, `TextProbe.swift`,
//! `DocumentProbe.swift`, `BoundedFile`).
//! Click handlers and renderers call these services on a worker thread. The control path performs its initial
//! probe synchronously, matching upstream; even opening a regular file can wait on network storage.

use std::collections::HashMap;
use std::fs::{File, Metadata, OpenOptions};
use std::io::{Cursor, Read as _};
use std::path::{Path, PathBuf};

use image::{ImageDecoder as _, ImageFormat, ImageReader};
use ngterm_core::html::document_page;
use ngterm_core::html::image_page::{self, ImageData};
use ngterm_core::html::markdown_page::{self, Embedded, MarkdownDocument};
use ngterm_core::html::text_page;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Failure {
    Unreadable,
    TooLarge,
}

/// Read one raster image, judging exactly the descriptor whose bytes are sniffed and probed.
///
/// # Errors
/// The upstream image-probe refusal, including the supplied path.
pub fn probe_image(path: &Path) -> Result<ImageData, String> {
    load_image(path, image_page::MAX_BYTES)
}

fn load_image(path: &Path, max_bytes: usize) -> Result<ImageData, String> {
    let bytes = bounded_read(path, max_bytes).map_err(|failure| image_failure(failure, path))?;
    let not_image = || format!("not a png, jpg, gif or webp image: {}", path.display());
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
        let (width, height) = checked_dimensions(decoder.dimensions(), path)?;
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
        let (width, height) = checked_dimensions(decoder.dimensions(), path)?;
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

fn checked_dimensions((width, height): (u32, u32), path: &Path) -> Result<(u32, u32), String> {
    if width == 0 || height == 0 {
        return Err(format!("not a png, jpg, gif or webp image: {}", path.display()));
    }
    if !image_page::allows_pixels(u64::from(width), u64::from(height)) {
        return Err(format!("image too large (over 100 megapixels): {}", path.display()));
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
    checked_file(path, markdown_page::MAX_BYTES)
        .map(|_| ())
        .map_err(|failure| markdown_failure(failure, path))
}

/// Read UTF-8 Markdown and embed bounded, locally-contained raster references.
///
/// # Errors
/// The upstream Markdown-probe refusal; invalid/unreadable pictures are omitted rather than failing the page.
pub fn probe_markdown(path: &Path) -> Result<MarkdownDocument, String> {
    let bytes = bounded_read(path, markdown_page::MAX_BYTES).map_err(|failure| markdown_failure(failure, path))?;
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
    let bytes = bounded_read(path, text_page::MAX_BYTES).map_err(|failure| match failure {
        Failure::Unreadable => format!("unreadable text file: {}", path.display()),
        Failure::TooLarge => format!(
            "text file too large (over {} MB): {}",
            text_page::MAX_BYTES >> 20,
            path.display()
        ),
    })?;
    text_page::validate(bytes).ok_or_else(|| format!("not a UTF-8 text file: {}", path.display()))
}

/// [`load_text`] with the text dropped: what `session.overlay.open --text` and a click check before a page takes its
/// slot (upstream's `TextProbe.check`).
///
/// # Errors
/// [`load_text`]'s refusals.
pub fn check_text(path: &Path) -> Result<(), String> {
    load_text(path).map(|_| ())
}

/// Read a clicked HTML file and build its page with `page` ([`document_page::page`], or [`document_page::chromium_page`]
/// for a Chromium engine), as upstream's `DocumentProbe.load` does: the read checks the content, so a binary file fails
/// a click and is revealed instead of opening a failed page.
///
/// # Errors
/// Upstream's document-probe refusal: `unreadable html file: P` (missing, not a regular file, unreadable), `html file
/// too large (over 4 MB): P`, `not a UTF-8 html file: P` (invalid UTF-8 or a NUL byte).
pub fn load_document(path: &Path, page: fn(Vec<u8>) -> Option<String>) -> Result<String, String> {
    let bytes = bounded_read(path, document_page::MAX_BYTES).map_err(|failure| match failure {
        Failure::Unreadable => format!("unreadable html file: {}", path.display()),
        Failure::TooLarge => format!(
            "html file too large (over {} MB): {}",
            document_page::MAX_BYTES >> 20,
            path.display()
        ),
    })?;
    page(bytes).ok_or_else(|| format!("not a UTF-8 html file: {}", path.display()))
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
    let mut embedded = Embedded::default();
    let mut cache = HashMap::<_, Option<usize>>::new();
    let (mut bytes, mut pixels) = (0, 0);
    for image in markdown_page::image_references(markdown, folder) {
        // Lexical containment was established before this first filesystem access to the candidate.
        let canonical = canonical_candidate(&image.path);
        if canonical == root || !ngterm_core::html::contains(&root.to_string_lossy(), &canonical.to_string_lossy()) {
            continue;
        }
        if let Some(known) = cache.get(&canonical) {
            if let Some(index) = known {
                embedded.references.insert(image.reference, *index);
            }
            continue;
        }
        if cache.len() >= markdown_page::MAX_IMAGES
            || bytes >= markdown_page::MAX_IMAGE_BYTES
            || pixels >= markdown_page::MAX_IMAGE_PIXELS
        {
            continue;
        }
        let loaded = load_image(&canonical, markdown_page::MAX_IMAGE_BYTES - bytes)
            .ok()
            .filter(|data| pixels + u64::from(data.width) * u64::from(data.height) <= markdown_page::MAX_IMAGE_PIXELS);
        let index = loaded.map(|data| {
            bytes += data.bytes.len();
            pixels += u64::from(data.width) * u64::from(data.height);
            let index = embedded.uris.len();
            embedded.uris.push(data.data_uri());
            embedded.references.insert(image.reference, index);
            index
        });
        cache.insert(canonical, index);
    }
    embedded
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

fn image_failure(failure: Failure, path: &Path) -> String {
    match failure {
        Failure::Unreadable => format!("unreadable image: {}", path.display()),
        Failure::TooLarge => format!("image too large (over 64 MB): {}", path.display()),
    }
}

fn markdown_failure(failure: Failure, path: &Path) -> String {
    match failure {
        Failure::Unreadable => format!("unreadable markdown file: {}", path.display()),
        Failure::TooLarge => format!("markdown file too large (over 4 MB): {}", path.display()),
    }
}

fn bounded_read(path: &Path, maximum: usize) -> Result<Vec<u8>, Failure> {
    let (mut file, size) = checked_file(path, maximum)?;
    let mut bytes = vec![0; size];
    file.read_exact(&mut bytes).map_err(|_| Failure::Unreadable)?;
    Ok(bytes)
}

fn checked_file(path: &Path, maximum: usize) -> Result<(File, usize), Failure> {
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
    let size = usize::try_from(metadata.len()).map_err(|_| Failure::TooLarge)?;
    if size > maximum {
        return Err(Failure::TooLarge);
    }
    Ok((file, size))
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
#[path = "documents/tests.rs"]
mod tests;
