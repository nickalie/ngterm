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
