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
