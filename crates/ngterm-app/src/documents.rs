//! Bounded local image/Markdown reads (upstream `ImageProbe.swift`, `MarkdownProbe.swift`, `BoundedFile`).
//! Click handlers and renderers call these services on a worker thread. The control path performs its initial
//! probe synchronously, matching upstream; even opening a regular file can wait on network storage.

use std::collections::HashMap;
use std::fs::{File, Metadata, OpenOptions};
use std::io::{Cursor, Read as _};
use std::path::{Path, PathBuf};

use image::{ImageDecoder as _, ImageFormat, ImageReader};
use ngterm_core::html::image_page::{self, ImageData};
use ngterm_core::html::markdown_page::{self, Embedded, MarkdownDocument};

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
    let mut reader = ImageReader::with_format(Cursor::new(&bytes), format);
    // Only metadata is requested. The separate pixel cap below applies before any pixel allocation or decode.
    reader.no_limits();
    let mut decoder = reader.into_decoder().map_err(|_| not_image())?;
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 {
        return Err(not_image());
    }
    if !image_page::allows_pixels(u64::from(width), u64::from(height)) {
        return Err(format!("image too large (over 100 megapixels): {}", path.display()));
    }
    let orientation = decoder.orientation().map_or(1, image::metadata::Orientation::to_exif);
    drop(decoder);
    Ok(ImageData {
        bytes,
        mime_type: mime_type.to_owned(),
        width,
        height,
        orientation,
    })
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
