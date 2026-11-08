use super::*;

#[test]
fn decoding_refuses_unknown_and_truncated_images_with_a_reason() {
    let dir = tempfile::tempdir().unwrap();
    let unknown = dir.path().join("unknown");
    std::fs::write(&unknown, b"not an image").unwrap();
    assert_eq!(decode(&unknown, false).unwrap_err(), "cannot determine the file type");
    let truncated = dir.path().join("truncated.png");
    std::fs::write(&truncated, b"\x89PNG\r\n\x1a\n").unwrap();
    assert!(!decode(&truncated, false).unwrap_err().is_empty());
}

#[test]
fn large_images_fit_the_atlas_without_losing_their_color() {
    let source = image::RgbaImage::from_pixel(MAX_TEXTURE_SIDE + 1, 1, image::Rgba([10, 20, 30, 255]));
    let fitted = fit(source).unwrap();
    assert_eq!(fitted.dimensions(), (MAX_TEXTURE_SIDE, 1));
    assert!(fitted.pixels().all(|pixel| *pixel == image::Rgba([10, 20, 30, 255])));
}

#[test]
fn repeating_keeps_large_and_empty_images_and_tiles_small_ones() {
    for (width, height) in [(0, 0), (MIN_REPEAT_SIDE, MIN_REPEAT_SIDE)] {
        let image = image::RgbaImage::new(width, height);
        let (pixels, tiles) = repeated(&image);
        assert_eq!(tiles, (1, 1));
        assert_eq!(pixels, image);
    }
    let original = image::RgbaImage::from_fn(2, 3, |x, y| {
        image::Rgba([u8::try_from(x).unwrap(), u8::try_from(y).unwrap(), 42, 255])
    });
    let (pixels, (across, down)) = repeated(&original);
    assert!(pixels.width() >= MIN_REPEAT_SIDE && pixels.height() >= MIN_REPEAT_SIDE);
    assert_eq!(pixels.dimensions(), (2 * across, 3 * down));
    for (x, y, pixel) in pixels.enumerate_pixels() {
        assert_eq!(pixel, original.get_pixel(x % 2, y % 3));
    }
}

use super::super::super::{Terminal, TerminalConfig, TerminalSpec};
use gpui_kit::{AppContext as _, Entity, TestAppContext};

fn idle(cx: &mut TestAppContext) -> (Entity<Terminal>, Rc<Shared>) {
    let terminal =
        cx.new(|cx| Terminal::gated(TerminalSpec::default(), TerminalConfig::default(), Box::new(|| {}), cx));
    let shared = terminal.read_with(cx, |terminal, _| Rc::clone(terminal.shared()));
    (terminal, shared)
}

fn spec(path: PathBuf) -> Spec {
    Spec {
        path,
        alpha: 1.,
        composite: None,
        fit: ngterm_core::ghostty_config::BackgroundImageFit::Contain,
        position: ngterm_core::ghostty_config::BackgroundImagePosition::Center,
        repeat: false,
    }
}

#[gpui_kit::test]
fn terminals_join_an_inflight_bake_and_reuse_a_completed_source(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("image.png");
    image::RgbaImage::from_pixel(4, 2, image::Rgba([10, 20, 30, 255]))
        .save(&file)
        .unwrap();
    let spec = spec(file);
    let (_a, a) = idle(cx);
    let (_b, b) = idle(cx);
    cx.update(|cx| {
        assert!(a.background_image(Some(spec.clone()), cx).is_none());
        assert!(b.background_image(Some(spec.clone()), cx).is_none());
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(a.background_image(Some(spec.clone()), cx).is_none());
        assert!(b.background_image(Some(spec.clone()), cx).is_none());
    });
    cx.run_until_parked();
    let first = a.background_loader().current().unwrap();
    let second = b.background_loader().current().unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    let (_c, c) = idle(cx);
    let cached = cx.update(|cx| c.background_image(Some(spec.clone()), cx)).unwrap();
    assert!(Arc::ptr_eq(&first, &cached));
    assert_eq!(first.texture.as_bytes(0).unwrap(), &[30, 20, 10, 255].repeat(8));
    assert!(!c.background_loader().is_loading());
}

#[test]
fn image_diagnostics_describe_geometry_without_dumping_pixels() {
    let source = Source {
        pixels: image::RgbaImage::new(2, 3),
        size: (2, 3),
        tiles: (1, 1),
    };
    assert_eq!(format!("{source:?}"), "Source { size: (2, 3), tiles: (1, 1), .. }");
    let decoded = texture(&source, Bake::of(&spec("image.png".into())));
    assert_eq!(
        format!("{decoded:?}"),
        "Decoded { size: (2, 3), tiles: (1, 1), bake: Bake { alpha: 1065353216, composite: None }, .. }"
    );
}

#[gpui_kit::test]
fn a_file_that_goes_missing_keeps_painting_the_image_on_screen(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.png");
    image::RgbaImage::from_pixel(4, 2, image::Rgba([10, 20, 30, 255]))
        .save(&path)
        .unwrap();
    let (_terminal, shared) = idle(cx);
    let old = spec(path);
    for _ in 0..4 {
        cx.update(|cx| shared.background_image(Some(old.clone()), cx));
        cx.run_until_parked();
    }
    let on_screen = shared.background_loader().current().unwrap();
    let missing = spec(dir.path().join("missing.png"));
    for _ in 0..2 {
        let painted = cx.update(|cx| shared.background_image(Some(missing.clone()), cx));
        assert!(
            painted.is_some_and(|image| Arc::ptr_eq(&image, &on_screen)),
            "the old image is painted"
        );
        cx.run_until_parked();
    }
    assert!(!shared.background_loader().is_loading(), "nothing to load");
}
