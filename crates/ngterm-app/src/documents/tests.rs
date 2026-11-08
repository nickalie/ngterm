use std::fs;
use std::path::PathBuf;

use image::ImageEncoder as _;

use super::*;

fn png() -> Vec<u8> {
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(&[1, 2, 3, 255], 1, 1, image::ExtendedColorType::Rgba8)
        .unwrap();
    bytes
}

fn header_sized_png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = png();
    bytes[16..20].copy_from_slice(&width.to_be_bytes());
    bytes[20..24].copy_from_slice(&height.to_be_bytes());
    let mut crc = u32::MAX;
    for byte in &bytes[12..29] {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0_u32.wrapping_sub(crc & 1));
        }
    }
    bytes[29..33].copy_from_slice(&(!crc).to_be_bytes());
    bytes
}

fn webp_with_exif(orientation: u8) -> Vec<u8> {
    webp(Some(orientation), false)
}

fn webp(orientation: Option<u8>, animated: bool) -> Vec<u8> {
    let mut image = Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(2, 3, image::Rgba([1, 2, 3, 255]))
        .write_to(&mut image, ImageFormat::WebP)
        .unwrap();
    let mut bytes = b"RIFF\0\0\0\0WEBP".to_vec();
    let flags = (u8::from(orientation.is_some()) * 8) | (u8::from(animated) * 2);
    append_riff_chunk(&mut bytes, *b"VP8X", &[flags, 0, 0, 0, 1, 0, 0, 2, 0, 0]);
    let image = image.into_inner();
    if animated {
        append_riff_chunk(&mut bytes, *b"ANIM", &[0; 6]);
        // One complete 2x3 frame, at the canvas origin, displayed for ten milliseconds.
        let mut frame = vec![0, 0, 0, 0, 0, 0, 1, 0, 0, 2, 0, 0, 10, 0, 0, 0];
        frame.extend_from_slice(&image[12..]);
        append_riff_chunk(&mut bytes, *b"ANMF", &frame);
    } else {
        bytes.extend_from_slice(&image[12..]);
    }
    if let Some(orientation) = orientation {
        let mut exif = b"II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0".to_vec();
        exif.extend([orientation, 0, 0, 0, 0, 0, 0, 0]);
        append_riff_chunk(&mut bytes, *b"EXIF", &exif);
    }
    update_riff_length(&mut bytes);
    bytes
}

fn append_riff_chunk(bytes: &mut Vec<u8>, kind: [u8; 4], payload: &[u8]) {
    bytes.extend(kind);
    bytes.extend(u32::try_from(payload.len()).unwrap().to_le_bytes());
    bytes.extend(payload);
    if !payload.len().is_multiple_of(2) {
        bytes.push(0);
    }
}

fn update_riff_length(bytes: &mut [u8]) {
    let size = u32::try_from(bytes.len() - 8).unwrap();
    bytes[4..8].copy_from_slice(&size.to_le_bytes());
}

fn write(dir: &Path, name: &str, bytes: impl AsRef<[u8]>) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn image_sniffs_bytes_not_extension_and_reads_only_header_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let bytes = png();
    let path = write(dir.path(), "shot.jpg", &bytes);
    let probe = probe_image(&path).unwrap();
    assert_eq!((probe.width, probe.height, probe.orientation), (1, 1, 1));
    assert_eq!(probe.mime_type, "image/png");
    assert_eq!(probe.bytes, bytes);
    for name in ["logo.png", "logo.gif", "logo.webp"] {
        let path = write(dir.path(), name, b"<svg><script>1</script></svg>");
        assert_eq!(
            probe_image(&path).unwrap_err(),
            format!("not a png, jpg, gif or webp image: {}", path.display())
        );
    }
    let missing = dir.path().join("missing.png");
    assert_eq!(
        probe_image(&missing).unwrap_err(),
        format!("unreadable image: {}", missing.display())
    );
}

#[test]
fn all_four_raster_decoders_are_enabled() {
    let dir = tempfile::tempdir().unwrap();
    let picture = image::RgbaImage::from_pixel(2, 3, image::Rgba([1, 2, 3, 255]));
    for (name, format, mime) in [
        ("a.png", ImageFormat::Png, "image/png"),
        ("a.jpg", ImageFormat::Jpeg, "image/jpeg"),
        ("a.gif", ImageFormat::Gif, "image/gif"),
        ("a.webp", ImageFormat::WebP, "image/webp"),
    ] {
        let mut bytes = Cursor::new(Vec::new());
        if format == ImageFormat::Jpeg {
            image::DynamicImage::ImageRgba8(picture.clone())
                .to_rgb8()
                .write_to(&mut bytes, format)
                .unwrap();
        } else {
            picture.write_to(&mut bytes, format).unwrap();
        }
        let probe = probe_image(&write(dir.path(), name, bytes.into_inner())).unwrap();
        assert_eq!((probe.width, probe.height, probe.mime_type.as_str()), (2, 3, mime));
        assert_eq!(probe.orientation, 1);
    }
}

#[test]
fn compressed_png_metadata_obeys_the_probe_allocation_limit_before_dimensions_are_read() {
    let profile_bytes = 2 << 20;
    let mut bytes = Vec::new();
    let mut encoder = image::codecs::png::PngEncoder::new(&mut bytes);
    encoder.set_icc_profile(vec![0; profile_bytes]).unwrap();
    encoder
        .write_image(&[1, 2, 3, 255], 1, 1, image::ExtendedColorType::Rgba8)
        .unwrap();
    assert!(bytes.len() < 16 << 10, "the metadata is much larger than the file");

    // A small instance of the production budget demonstrates the actual decoder's decompression limit without
    // allocating a huge profile. PNG discards an over-budget optional ICC chunk and still exposes the dimensions.
    let mut bounded = header_decoder(&bytes, ImageFormat::Png, 1 << 20).unwrap();
    assert_eq!(bounded.dimensions(), (1, 1));
    assert_eq!(bounded.icc_profile().unwrap(), None);
    let mut enough = header_decoder(&bytes, ImageFormat::Png, 3 << 20).unwrap();
    assert_eq!(enough.icc_profile().unwrap().unwrap().len(), profile_bytes);

    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "profile.png", &bytes);
    let probe = probe_image(&path).unwrap();
    assert_eq!((probe.width, probe.height), (1, 1));
}

#[test]
fn webp_exif_obeys_a_modest_metadata_budget() {
    for animated in [false, true] {
        let bytes = webp(Some(6), animated);
        let mut bounded = webp_header_decoder(&bytes, 16).unwrap();
        assert_eq!(bounded.dimensions(), (2, 3));
        assert_eq!(bounded.is_animated(), animated);
        assert!(matches!(
            bounded.exif_metadata(),
            Err(image_webp::DecodingError::MemoryLimitExceeded)
        ));
        let mut enough = webp_header_decoder(&bytes, 32).unwrap();
        let exif = enough.exif_metadata().unwrap().unwrap();
        assert_eq!(exif.len(), 26);
        assert_eq!(
            image::metadata::Orientation::from_exif_chunk(&exif).unwrap().to_exif(),
            6
        );
    }
}

#[test]
fn static_and_animated_webp_preserve_orientation_for_direct_and_markdown_callers() {
    let dir = tempfile::tempdir().unwrap();
    for animated in [false, true] {
        for orientation in std::iter::once(None).chain((1..=8).map(Some)) {
            let bytes = webp(orientation, animated);
            let path = write(dir.path(), "picture.webp", &bytes);
            let probe = probe_image(&path).unwrap();
            assert_eq!(
                (probe.width, probe.height, probe.orientation),
                (2, 3, orientation.unwrap_or(1))
            );
            assert_eq!(probe.bytes, bytes);
            if orientation.is_some_and(|value| value >= 5) {
                assert_eq!(probe.natural(1.0), (3.0, 2.0));
            }
            let document = probe_markdown(&write(dir.path(), "doc.md", "![picture](picture.webp)")).unwrap();
            assert_eq!(document.images.uris, [probe.data_uri()]);
            assert_eq!(document.images.references.get("picture.webp"), Some(&0));
        }
    }
}

#[test]
fn webp_chunk_validation_handles_lengths_padding_and_preserves_valid_exif() {
    let dir = tempfile::tempdir().unwrap();
    for orientation in 1..=8 {
        let mut bytes = webp_with_exif(orientation);
        append_riff_chunk(&mut bytes, *b"XMP ", b"x");
        update_riff_length(&mut bytes);
        assert!(webp_riff_end(&bytes).is_some(), "odd chunks include their padding");
        let probe = probe_image(&write(dir.path(), "rotate.webp", &bytes)).unwrap();
        assert_eq!((probe.width, probe.height, probe.orientation), (2, 3, orientation));
        bytes.pop();
        update_riff_length(&mut bytes);
        assert!(webp_riff_end(&bytes).is_none(), "missing odd padding");
    }
    let valid = webp_with_exif(1);
    for prefix in 0..valid.len() {
        assert!(webp_riff_end(&valid[..prefix]).is_none(), "truncated at {prefix}");
    }
    for length in [0, 1, 3, u32::MAX - 1, u32::MAX] {
        let mut bytes = valid.clone();
        bytes[4..8].copy_from_slice(&length.to_le_bytes());
        assert!(webp_riff_end(&bytes).is_none(), "invalid RIFF length {length}");
        bytes = valid.clone();
        bytes[16..20].copy_from_slice(&length.to_le_bytes());
        assert!(webp_riff_end(&bytes).is_none(), "invalid first chunk length {length}");
    }
    for size in [u32::MAX - 1, u32::MAX] {
        let mut bytes = valid.clone();
        let exif = bytes.windows(4).position(|kind| kind == b"EXIF").unwrap();
        bytes[exif + 4..exif + 8].copy_from_slice(&size.to_le_bytes());
        assert!(webp_riff_end(&bytes).is_none(), "oversized metadata length {size}");
    }
    for tail in [b"X".as_slice(), b"XMP \0\0\0".as_slice()] {
        let mut bytes = valid.clone();
        bytes.extend(tail);
        update_riff_length(&mut bytes);
        assert!(webp_riff_end(&bytes).is_none(), "truncated chunk header");
    }
}

/// Bytes after the RIFF container are outside it and ignored, as libwebp and upstream's `ImageIO` ignore them; the page
/// still gets the file as read.
#[test]
fn webp_bytes_after_the_riff_container_are_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let valid = webp_with_exif(1);
    for tail in [b"X".as_slice(), b"trailing data".as_slice()] {
        let mut bytes = valid.clone();
        bytes.extend(tail);
        assert_eq!(webp_riff_end(&bytes), Some(valid.len()), "trailing {tail:?}");
        let probe = probe_image(&write(dir.path(), "trailing.webp", &bytes)).unwrap();
        assert_eq!((probe.width, probe.height, probe.bytes.len()), (2, 3, bytes.len()));
    }
}

#[test]
fn pixel_limit_is_checked_from_header_and_jpeg_exif_orientation_is_retained() {
    let dir = tempfile::tempdir().unwrap();
    let big = write(dir.path(), "big.png", header_sized_png(10_000, 10_001));
    assert_eq!(
        probe_image(&big).unwrap_err(),
        format!("image too large (over 100 megapixels): {}", big.display())
    );
    let picture = image::RgbImage::from_pixel(2, 3, image::Rgb([1, 2, 3]));
    let mut jpeg = Cursor::new(Vec::new());
    picture.write_to(&mut jpeg, ImageFormat::Jpeg).unwrap();
    let jpeg = jpeg.into_inner();
    for orientation in 5_u8..=8 {
        let mut exif = b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0".to_vec();
        exif.extend([orientation, 0, 0, 0, 0, 0, 0, 0]);
        let mut bytes = jpeg[..2].to_vec();
        bytes.extend([255, 225]);
        bytes.extend(u16::try_from(exif.len() + 2).unwrap().to_be_bytes());
        bytes.extend(exif);
        bytes.extend(&jpeg[2..]);
        let probe = probe_image(&write(dir.path(), "rotate.jpg", bytes)).unwrap();
        assert_eq!(probe.orientation, orientation);
        assert_eq!(probe.natural(1.0), (3.0, 2.0));
    }
}

#[test]
fn markdown_plain_utf8_and_checks_have_exact_failures() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "doc.md", b"# Title\n\ntext\n");
    assert_eq!(
        probe_markdown(&path).unwrap(),
        MarkdownDocument {
            markdown: "# Title\n\ntext\n".to_owned(),
            images: Embedded::default()
        }
    );
    check_markdown(&path).unwrap();
    let bad = write(dir.path(), "bin.md", [255, 254, 0, 195]);
    assert_eq!(
        probe_markdown(&bad).unwrap_err(),
        format!("not a UTF-8 text file: {}", bad.display())
    );
    let big = write(dir.path(), "big.md", []);
    File::options()
        .write(true)
        .open(&big)
        .unwrap()
        .set_len((markdown_page::MAX_BYTES + 1) as u64)
        .unwrap();
    let message = format!("markdown file too large (over 4 MB): {}", big.display());
    assert_eq!(check_markdown(&big).unwrap_err(), message);
    assert_eq!(probe_markdown(&big).unwrap_err(), message);
}

/// Upstream's `TextProbeTests`: the text read back, and each refusal with its path.
#[test]
fn text_reads_back_and_refuses_missing_directories_oversize_and_binaries() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "main.go", b"\npackage main\n");
    assert_eq!(load_text(&path).unwrap(), "\npackage main\n");
    check_text(&path).unwrap();
    let missing = dir.path().join("missing.go");
    assert_eq!(
        check_text(&missing).unwrap_err(),
        format!("unreadable text file: {}", missing.display())
    );
    let folder = dir.path().join("sub.txt");
    fs::create_dir(&folder).unwrap();
    assert_eq!(
        check_text(&folder).unwrap_err(),
        format!("unreadable text file: {}", folder.display())
    );
    let big = write(dir.path(), "big.log", []);
    File::options()
        .write(true)
        .open(&big)
        .unwrap()
        .set_len((text_page::MAX_BYTES + 1) as u64)
        .unwrap();
    assert_eq!(
        check_text(&big).unwrap_err(),
        format!("text file too large (over 4 MB): {}", big.display())
    );
    let at_cap = write(dir.path(), "cap.log", "a".repeat(text_page::MAX_BYTES));
    assert_eq!(load_text(&at_cap).unwrap().len(), text_page::MAX_BYTES);
    for (name, bytes) in [("nul.txt", &b"a\0b"[..]), ("bad.txt", &[0xFF, 0xFE, 0x61][..])] {
        let binary = write(dir.path(), name, bytes);
        assert_eq!(
            check_text(&binary).unwrap_err(),
            format!("not a UTF-8 text file: {}", binary.display())
        );
    }
}

/// Upstream's `DocumentProbeTests`: an HTML file loads as its page, the policy placed; one over the cap is too large, a
/// binary one is no text, and a missing file or a directory is unreadable.
#[test]
fn an_html_file_loads_as_its_page_and_refuses_missing_directories_oversize_and_binaries() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "pr.html", "<h1>pr</h1>");
    let page = document_page::page(b"<h1>pr</h1>".to_vec()).unwrap();
    assert_eq!(load_document(&path, document_page::page).unwrap(), page);
    assert!(
        load_document(&path, document_page::chromium_page)
            .unwrap()
            .contains(document_page::NO_DNS_PREFETCH)
    );
    check_document(&path).unwrap();
    let big = write(dir.path(), "big.html", []);
    File::options()
        .write(true)
        .open(&big)
        .unwrap()
        .set_len((document_page::MAX_BYTES + 1) as u64)
        .unwrap();
    assert_eq!(
        check_document(&big).unwrap_err(),
        format!("html file too large (over 4 MB): {}", big.display())
    );
    let binary = write(dir.path(), "blob.html", [0x3C, 0x00, 0x3E]);
    assert_eq!(
        check_document(&binary).unwrap_err(),
        format!("not a UTF-8 html file: {}", binary.display())
    );
    let folder = dir.path().join("sub.html");
    fs::create_dir(&folder).unwrap();
    let missing = dir.path().join("missing.html");
    for unreadable in [folder, missing] {
        assert_eq!(
            check_document(&unreadable).unwrap_err(),
            format!("unreadable html file: {}", unreadable.display())
        );
    }
    #[cfg(unix)]
    {
        let fifo = dir.path().join("pipe.html");
        let raw = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: a NUL-terminated path in a private temporary directory.
        assert_eq!(unsafe { libc::mkfifo(raw.as_ptr(), 0o600) }, 0);
        assert_eq!(
            check_document(&fifo).unwrap_err(),
            format!("unreadable html file: {}", fifo.display())
        );
    }
}

#[test]
fn canonical_aliases_embed_one_payload_and_failures_consume_probe_budget() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.png", png());
    write(dir.path(), "late.png", png());
    let aliases = (0..200)
        .map(|n| format!("![a]({}a.png)", "./".repeat(n)))
        .collect::<Vec<_>>()
        .join(" ");
    let found = probe_markdown(&write(dir.path(), "doc.md", aliases)).unwrap();
    assert_eq!(found.images.uris.len(), 1);
    assert_eq!(found.images.references.len(), 200);
    let mut refs = vec!["![a](a.png)".to_owned()];
    for n in 1..markdown_page::MAX_IMAGES {
        if n % 2 == 0 {
            write(dir.path(), &format!("bad{n}.png"), b"x");
        }
        refs.push(format!("![x](bad{n}.png)"));
    }
    refs.extend(["![l](late.png)".to_owned(), "![a](./a.png)".to_owned()]);
    let found = probe_markdown(&write(dir.path(), "doc.md", refs.join(" "))).unwrap();
    assert_eq!(
        found.images.references.keys().map(String::as_str).collect::<Vec<_>>(),
        ["./a.png", "a.png"]
    );
    assert_eq!(found.images.uris.len(), 1);
}

#[test]
fn spent_byte_budget_still_resolves_embedded_aliases() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "a.png", png());
    File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_len(markdown_page::MAX_IMAGE_BYTES as u64)
        .unwrap();
    write(dir.path(), "b.png", png());
    let document = probe_markdown(&write(dir.path(), "doc.md", "![a](a.png) ![b](b.png) ![c](./a.png)")).unwrap();
    assert_eq!(
        document
            .images
            .references
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["./a.png", "a.png"]
    );
    assert_eq!(document.images.uris.len(), 1);
}

#[test]
fn byte_and_pixel_budgets_skip_overlarge_candidates_but_keep_later_small_pictures() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.png", header_sized_png(8000, 8000));
    write(dir.path(), "b.png", header_sized_png(8000, 8000));
    write(dir.path(), "c.png", png());
    let source = "![a](a.png) ![b](b.png) ![c](c.png)";
    let found = probe_markdown(&write(dir.path(), "doc.md", source)).unwrap();
    assert_eq!(
        found.images.references.keys().map(String::as_str).collect::<Vec<_>>(),
        ["a.png", "c.png"]
    );
    for name in ["a.png", "b.png"] {
        let path = write(dir.path(), name, png());
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_len((markdown_page::MAX_IMAGE_BYTES / 2 + 1) as u64)
            .unwrap();
    }
    let found = probe_markdown(&write(dir.path(), "doc.md", source)).unwrap();
    assert_eq!(
        found.images.references.keys().map(String::as_str).collect::<Vec<_>>(),
        ["a.png", "c.png"]
    );
}

#[cfg(unix)]
#[test]
fn fifo_devices_symlinks_and_canonical_escape_are_checked_without_reads() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let fifo = dir.path().join("pipe.png");
    let path = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
    // SAFETY: a nul-terminated path owned by this test, ordinary owner-only permissions.
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    assert!(probe_image(&fifo).unwrap_err().starts_with("unreadable image:"));
    assert!(
        check_markdown(&fifo)
            .unwrap_err()
            .starts_with("unreadable markdown file:")
    );
    let text_fifo = dir.path().join("pipe.log");
    let text_path = std::ffi::CString::new(text_fifo.as_os_str().as_encoded_bytes()).unwrap();
    // SAFETY: a nul-terminated path owned by this test, ordinary owner-only permissions.
    assert_eq!(unsafe { libc::mkfifo(text_path.as_ptr(), 0o600) }, 0);
    assert!(check_text(&text_fifo).unwrap_err().starts_with("unreadable text file:"));
    let device = dir.path().join("zero.png");
    symlink("/dev/zero", &device).unwrap();
    assert!(probe_image(&device).unwrap_err().starts_with("unreadable image:"));
    let huge = write(dir.path(), "huge.png", []);
    File::options()
        .write(true)
        .open(&huge)
        .unwrap()
        .set_len((image_page::MAX_BYTES + 1) as u64)
        .unwrap();
    let linked = dir.path().join("linked.png");
    symlink(&huge, &linked).unwrap();
    assert_eq!(
        probe_image(&linked).unwrap_err(),
        format!("image too large (over 64 MB): {}", linked.display())
    );
    let outside = tempfile::tempdir().unwrap();
    let external = write(outside.path(), "external.png", png());
    symlink(external, dir.path().join("out.png")).unwrap();
    let local = write(dir.path(), "in.png", png());
    symlink(local, dir.path().join("alias.png")).unwrap();
    let doc = probe_markdown(&write(
        dir.path(),
        "doc.md",
        "![o](out.png) ![i](in.png) ![a](alias.png)",
    ))
    .unwrap();
    assert_eq!(doc.images.uris.len(), 1);
    assert!(!doc.images.references.contains_key("out.png"));
    assert_eq!(doc.images.references.len(), 2);
    let folder_link = outside.path().join("folder-link");
    symlink(dir.path(), &folder_link).unwrap();
    let linked_doc = probe_markdown(&folder_link.join("doc.md")).unwrap();
    assert_eq!(linked_doc.images, doc.images);
    let mut references = vec!["![i](in.png)".to_owned()];
    references.extend((1..markdown_page::MAX_IMAGES).map(|n| format!("![m](missing{n}.png)")));
    write(dir.path(), "late.png", png());
    references.extend(["![l](late.png)".to_owned(), "![a](alias.png)".to_owned()]);
    write(dir.path(), "doc.md", references.join(" "));
    let linked_doc = probe_markdown(&folder_link.join("doc.md")).unwrap();
    assert_eq!(
        linked_doc.images.references.len(),
        2,
        "missing probes under a symlinked folder count toward the cap"
    );
    assert!(!linked_doc.images.references.contains_key("late.png"));
}

/// Upstream's `testATailOfASmallFileStartsAtItsFirstByte`.
#[test]
fn a_tail_of_a_small_file_starts_at_its_first_byte() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "small.log", b"one\ntwo\n");
    let read = read_tail(&path, 64, None).unwrap().unwrap();
    assert_eq!(
        (read.data.as_slice(), read.offset, read.stamp.size),
        (&b"one\ntwo\n"[..], 0, 8)
    );
}

/// Upstream's `testATailOfALargeFileReadsTheWindowAndTheByteBeforeIt`.
#[test]
fn a_tail_of_a_large_file_reads_the_window_and_the_byte_before_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "big.log", b"0123456789abcdef");
    let read = read_tail(&path, 4, None).unwrap().unwrap();
    assert_eq!((read.data.as_slice(), read.offset), (&b"bcdef"[..], 11));
}

/// Upstream's `testATailIgnoresTheWholeFileCapAndMeasuresASymlinkAtItsTarget`.
#[cfg(unix)]
#[test]
fn a_tail_ignores_the_whole_file_cap_and_measures_a_symlink_at_its_target() {
    let dir = tempfile::tempdir().unwrap();
    let big = write(dir.path(), "huge.log", []);
    File::options()
        .write(true)
        .open(&big)
        .unwrap()
        .set_len((text_page::MAX_BYTES * 2) as u64)
        .unwrap();
    let link = dir.path().join("link.log");
    std::os::unix::fs::symlink(&big, &link).unwrap();
    let read = read_tail(&link, 16, None).unwrap().unwrap();
    assert_eq!(read.data.len(), 17);
    assert_eq!(read.stamp.size, (text_page::MAX_BYTES * 2) as u64);
}

/// Upstream's `testATailOfAFifoOrADeviceIsRefused`.
#[cfg(unix)]
#[test]
fn a_tail_of_a_fifo_or_a_device_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let fifo = dir.path().join("pipe.log");
    let raw = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
    // SAFETY: a nul-terminated path owned by this test, ordinary owner-only permissions.
    assert_eq!(unsafe { libc::mkfifo(raw.as_ptr(), 0o600) }, 0);
    let device = dir.path().join("zero.log");
    std::os::unix::fs::symlink("/dev/zero", &device).unwrap();
    for path in [
        &fifo,
        &device,
        &dir.path().join("missing.log"),
        &dir.path().to_path_buf(),
    ] {
        assert_eq!(
            read_tail(path, 16, None),
            Err(Failure::Unreadable),
            "{}",
            path.display()
        );
        assert_eq!(
            check_log(path, 10).unwrap_err(),
            format!("unreadable text file: {}", path.display())
        );
    }
}

/// Upstream's `testATailStampChangesWhenTheFileGrowsOrIsReplaced`.
#[test]
fn a_tail_stamp_changes_when_the_file_grows_or_is_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "app.log", b"a\n");
    let stamp = |path: &Path| read_tail(path, 64, None).unwrap().unwrap().stamp;
    let first = stamp(&path);
    assert_eq!(stamp(&path), first);
    let mut file = File::options().append(true).open(&path).unwrap();
    std::io::Write::write_all(&mut file, b"b\n").unwrap();
    drop(file);
    let grown = stamp(&path);
    // the reader holds no handle between reads, so a writer renames the file away and starts a new one
    fs::rename(&path, dir.path().join("app.log.1")).unwrap();
    write(dir.path(), "app.log", b"a\nb\n");
    let replaced = stamp(&path);
    assert_ne!(grown, first);
    assert_ne!(replaced.identity, grown.identity);
}

/// Upstream's `testATailOfAnUnchangedFileReadsNothing`.
#[test]
fn a_tail_of_an_unchanged_file_reads_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "app.log", b"a\n");
    let stamp = read_tail(&path, 64, None).unwrap().unwrap().stamp;
    assert_eq!(read_tail(&path, 64, Some(&stamp)), Ok(None));
    assert_eq!(load_log(&path, 10, Some(&stamp)), Ok(None));
    let mut file = File::options().append(true).open(&path).unwrap();
    std::io::Write::write_all(&mut file, b"b\n").unwrap();
    drop(file);
    let read = read_tail(&path, 64, Some(&stamp)).unwrap().unwrap();
    assert_eq!(read.data, b"a\nb\n");
}

/// Upstream's `TextProbe.checkLog`: a log is read only for its tail, so its size is never refused, and its kept lines
/// alone judge it binary.
#[test]
fn a_log_check_reads_only_its_tail() {
    let dir = tempfile::tempdir().unwrap();
    let big = write(dir.path(), "big.log", []);
    let mut file = File::options().write(true).open(&big).unwrap();
    file.set_len((text_page::MAX_BYTES * 2) as u64).unwrap();
    std::io::Seek::seek(&mut file, std::io::SeekFrom::End(0)).unwrap();
    std::io::Write::write_all(&mut file, b"\nlast line\n").unwrap();
    drop(file);
    check_log(&big, 1).unwrap();
    let read = load_log(&big, 1, None).unwrap().unwrap();
    let snapshot = read.snapshot.unwrap();
    assert_eq!((snapshot.text.as_str(), snapshot.lines), ("last line\n", 1));
    // the window opens inside the NUL run, a partial line it drops
    check_log(&big, 3).unwrap();
    let binary = write(dir.path(), "bin.log", b"a\0b\n\xFF\xFE\nok\n");
    for tail in [3, 2] {
        assert_eq!(
            check_log(&binary, tail).unwrap_err(),
            format!("not a UTF-8 text file: {}", binary.display())
        );
    }
    check_log(&binary, 1).unwrap();
}

/// Upstream's `ImageProbeTests.testBytesInHandAreJudgedLikeAFile` and `TextProbeTests.testBytesInHandAreJudgedLikeAFile`:
/// a picture or text fetched from another machine is judged as a file of this one's, named by its path there.
#[test]
fn bytes_in_hand_are_judged_like_a_file() {
    let picture = image_from_bytes(header_sized_png(12, 7), "/o/a.png").unwrap();
    assert_eq!(
        (picture.width, picture.height, picture.mime_type.as_str()),
        (12, 7, "image/png")
    );
    assert_eq!(
        image_from_bytes(b"<svg/>".to_vec(), "/o/logo.png").unwrap_err(),
        "not a png, jpg, gif or webp image: /o/logo.png"
    );
    assert_eq!(
        image_from_bytes(header_sized_png(10_001, 10_000), "/o/bomb.png").unwrap_err(),
        "image too large (over 100 megapixels): /o/bomb.png"
    );
    assert_eq!(
        text_from_bytes(b"let x = 1\n".to_vec(), "/o/a.swift").unwrap(),
        "let x = 1\n"
    );
    for bytes in [vec![0x61, 0x00, 0x62], vec![0xFF, 0xFE, 0x61]] {
        assert_eq!(
            text_from_bytes(bytes, "/o/nul.txt").unwrap_err(),
            "not a UTF-8 text file: /o/nul.txt"
        );
    }
    let page = document_from_bytes(b"<p>x</p>".to_vec(), "/o/pr.html", document_page::page).unwrap();
    assert!(page.contains("<p>x</p>"), "{page}");
    assert_eq!(
        document_from_bytes(vec![0x61, 0x00], "/o/pr.html", document_page::page).unwrap_err(),
        "not a UTF-8 html file: /o/pr.html"
    );
}

/// Upstream's `MarkdownProbeTests.testAnEmbeddingFedFromAnyReaderKeepsOneCopyAndSpendsTheByteBudget`.
#[test]
fn an_embedding_fed_from_any_reader_keeps_one_copy_and_spends_the_byte_budget() {
    let picture = image_from_bytes(png(), "/o/a.png").unwrap();
    let size = picture.bytes.len();
    let mut embedding = Embedding::default();
    assert_eq!(
        embedding.room("/o/a.png", "a.png"),
        Some(markdown_page::MAX_IMAGE_BYTES)
    );
    embedding.add("/o/a.png".to_owned(), "a.png".to_owned(), Some(picture));
    assert_eq!(
        embedding.room("/o/a.png", "./a.png"),
        None,
        "an alias of a picture read"
    );
    assert_eq!(
        embedding.room("/o/b.png", "b.png"),
        Some(markdown_page::MAX_IMAGE_BYTES - size)
    );
    embedding.add("/o/b.png".to_owned(), "b.png".to_owned(), None);
    assert_eq!(
        embedding.room("/o/b.png", "b.png"),
        None,
        "a failed read is not read again"
    );
    let embedded = embedding.embedded();
    assert_eq!(embedded.uris.len(), 1);
    assert_eq!(
        embedded.references,
        std::collections::BTreeMap::from([("a.png".to_owned(), 0), ("./a.png".to_owned(), 0)])
    );
}

/// A Markdown file fetched from another machine lists its pictures lexically against its folder there, reads each one
/// once through the fetch under the shared budget and keeps a reference whose bytes are no picture as its alt text.
#[test]
fn a_fetched_markdown_file_embeds_the_pictures_its_fetch_brings() {
    let folder = PathBuf::from(if cfg!(windows) { r"C:\o\docs" } else { "/o/docs" });
    let path = folder.join("README.md").to_string_lossy().into_owned();
    let shot = folder.join("shot.png").to_string_lossy().into_owned();
    let mut asked = Vec::new();
    let document = markdown_from_bytes(
        "# Remote\n\n![s](shot.png) ![again](./shot.png) ![x](fake.png) ![out](../out.png)\n"
            .as_bytes()
            .to_vec(),
        &path,
        |requested, room| {
            asked.push((requested.to_owned(), room));
            (requested == shot).then(png).or_else(|| Some(b"<svg/>".to_vec()))
        },
    )
    .unwrap();
    let fake = folder.join("fake.png").to_string_lossy().into_owned();
    assert_eq!(
        asked,
        [
            (shot, markdown_page::MAX_IMAGE_BYTES),
            (fake, markdown_page::MAX_IMAGE_BYTES - png().len())
        ]
    );
    assert_eq!(document.images.uris.len(), 1);
    assert_eq!(
        document.images.references.keys().collect::<Vec<_>>(),
        ["./shot.png", "shot.png"]
    );
    assert_eq!(
        markdown_from_bytes(vec![0xFF], &path, |_, _| None).unwrap_err(),
        format!("not a UTF-8 text file: {path}")
    );
}

/// A viewer lists a fetched Markdown file's pictures by its origin's path rules whatever OS it runs: a POSIX origin's
/// and a Windows origin's pictures are asked for as each spells them there, on any host.
#[test]
fn a_fetched_markdown_file_lists_its_pictures_by_its_origins_path_rules() {
    let markdown = "![a](img/a.png) ![b](img\\b.png) ![up](../up.png)\n";
    for (path, expected) in [
        ("/srv/docs/README.md", ["/srv/docs/img/a.png", "/srv/docs/img\\b.png"]),
        (
            r"C:\Users\me\docs\README.md",
            [r"C:\Users\me\docs\img\a.png", r"C:\Users\me\docs\img\b.png"],
        ),
    ] {
        let mut asked = Vec::new();
        let document = markdown_from_bytes(markdown.as_bytes().to_vec(), path, |requested, _| {
            asked.push(requested.to_owned());
            Some(png())
        })
        .unwrap();
        assert_eq!(asked, expected, "{path}");
        assert_eq!(document.images.uris.len(), 2, "{path}");
    }
}

/// A content replica reads under its kind's cap and words its failures as that kind's local probe; an HTML replica
/// loads through its scheme instead.
#[test]
fn a_replica_reads_under_its_kinds_cap_and_words_failures_as_its_probe() {
    let read = |content| replica_read(content, "/o/a").unwrap();
    let (limit, failures) = read(ReplicaContent::Image { width: 1, height: 1 });
    assert_eq!(limit, image_page::MAX_BYTES);
    assert_eq!(
        (failures.too_large.as_str(), failures.unreadable.as_str()),
        ("image too large (over 64 MB): /o/a", "unreadable image: /o/a")
    );
    let (limit, failures) = read(ReplicaContent::Markdown);
    assert_eq!(limit, markdown_page::MAX_BYTES);
    assert_eq!(failures.unreadable, "unreadable markdown file: /o/a");
    let (limit, failures) = read(ReplicaContent::Text { line: None });
    assert_eq!(limit, text_page::MAX_BYTES);
    assert_eq!(failures.too_large, "text file too large (over 4 MB): /o/a");
    let (limit, failures) = read(ReplicaContent::Document);
    assert_eq!(limit, document_page::MAX_BYTES);
    assert_eq!(failures.unreadable, "unreadable html file: /o/a");
    assert!(replica_read(ReplicaContent::Html, "/o/a").is_none());
}
