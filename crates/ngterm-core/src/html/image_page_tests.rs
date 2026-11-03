use super::*;

#[test]
fn natural_size_applies_display_scale_and_each_exif_rotation() {
    for (orientation, scale, expected) in [
        (1, 2.0, (600.0, 400.0)),
        (1, 1.0, (1200.0, 800.0)),
        (6, 2.0, (400.0, 600.0)),
        (8, 1.0, (800.0, 1200.0)),
        (3, 1.0, (1200.0, 800.0)),
        (1, 0.0, (1200.0, 800.0)),
    ] {
        assert_eq!(natural(1200, 800, orientation, scale), expected);
    }
    for orientation in 5..=8 {
        assert_eq!(natural(1200, 800, orientation, 1.0), (800.0, 1200.0));
    }
}

#[test]
fn points_follow_zoom_minimum_width_and_aspect_preserving_cap() {
    for (size, zoom, expected) in [
        ((600.0, 400.0), 1.0, (600, 400)),
        ((600.0, 400.0), 1.5, (900, 600)),
        ((100.0, 50.0), 1.0, (240, 120)),
        ((3000.5, 300.0), 1.0, (3001, 300)),
    ] {
        assert_eq!(
            points(size, zoom, None),
            OverlayPointSize {
                width: Some(expected.0),
                height: Some(expected.1)
            }
        );
    }
    let bounds = Some(HudPaneFrame {
        width: 1000.0,
        height: 800.0,
        ..HudPaneFrame::default()
    });
    for (size, expected) in [
        ((3000.0, 300.0), (800, 80)),
        ((600.0, 3000.0), (122, 611)),
        ((600.0, 400.0), (600, 400)),
        ((1000.0, 1000.0), (611, 611)),
        ((100.0, 50.0), (240, 120)),
    ] {
        assert_eq!(
            points(size, 1.0, bounds),
            OverlayPointSize {
                width: Some(expected.0),
                height: Some(expected.1)
            }
        );
    }
}

#[test]
fn signatures_allow_only_four_raster_formats_and_pixel_cap_cannot_overflow() {
    for (bytes, mime) in [
        (&b"\x89PNG\r\n\x1a\n\0"[..], Some("image/png")),
        (&b"\xff\xd8\xff\xe0\0"[..], Some("image/jpeg")),
        (&b"GIF89a.."[..], Some("image/gif")),
        (&b"GIF87a.."[..], Some("image/gif")),
        (&b"RIFF\0\0\0\0WEBPVP8 "[..], Some("image/webp")),
        (&b"<svg xmlns="[..], None),
        (&b"<html><body>"[..], None),
        (&b"RIFF\0\0\0\0WAVEfmt "[..], None),
        (&b"\x89PN"[..], None),
        (&b""[..], None),
    ] {
        assert_eq!(mime_type(bytes), mime);
    }
    assert!(allows_pixels(10_000, 10_000));
    assert!(!allows_pixels(10_000, 10_001));
    assert!(!allows_pixels(12_000, 12_000));
    assert!(!allows_pixels(u64::MAX, 2));
}

#[test]
fn image_html_embeds_data_with_aspect_fill_and_no_remote_loading() {
    let document = html(&[1, 2, 3], "image/png");
    assert!(document.contains("src=\"data:image/png;base64,AQID\""));
    assert!(document.contains("width:100%;height:100%;object-fit:contain"));
    assert!(document.contains("default-src 'none'; img-src data:"));
    assert!(!document.contains("<script"));
}
