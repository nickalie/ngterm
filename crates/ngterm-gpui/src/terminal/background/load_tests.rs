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

use super::super::super::{TerminalView, ViewRole};
use super::super::BackgroundPaint;
use gpui_kit::VisualTestContext;

fn prepared(view: &Entity<TerminalView>, cx: &mut VisualTestContext) -> Option<BackgroundPaint> {
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            let metrics = view.shared.metrics();
            view.prepare_images(None, &metrics, window, cx).background
        })
    })
}

fn background_config(path: &std::path::Path, repeat: bool) -> TerminalConfig {
    let mut config = TerminalConfig::default();
    config.background_image.path = Some(ngterm_core::ghostty_config::ConfigPath {
        path: path.to_path_buf(),
        optional: false,
    });
    config.background_image.repeat = repeat;
    config.background_image.fit = ngterm_core::ghostty_config::BackgroundImageFit::None;
    Arc::make_mut(&mut config.palette).background_alpha = 128;
    config
}

fn loaded(view: &Entity<TerminalView>, cx: &mut VisualTestContext) -> BackgroundPaint {
    for _ in 0..4 {
        prepared(view, cx);
        cx.run_until_parked();
    }
    prepared(view, cx).expect("the background loaded")
}

fn assert_retained(fallback: &BackgroundPaint, loaded: &BackgroundPaint) {
    assert!(Arc::ptr_eq(&fallback.decoded, &loaded.decoded));
    assert_eq!(
        fallback.spec.repeat, loaded.spec.repeat,
        "the texture keeps its loaded repeat mode"
    );
    let screen = (800., 600.);
    assert_eq!(
        super::super::layout(screen, fallback.decoded.size, fallback.decoded.tiles, &fallback.spec),
        super::super::layout(screen, loaded.decoded.size, loaded.decoded.tiles, &loaded.spec),
        "each source tile keeps its size"
    );
    assert_eq!(fallback.color_areas(screen), loaded.color_areas(screen));
}

#[gpui_kit::test]
fn a_missing_background_keeps_the_loaded_repeat_geometry(cx: &mut TestAppContext) {
    for repeat in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pattern.png");
        image::RgbaImage::from_fn(4, 2, |x, y| {
            image::Rgba([u8::try_from(x * 60).unwrap(), u8::try_from(y * 120).unwrap(), 30, 255])
        })
        .save(&path)
        .unwrap();
        let (terminal, shared) = idle(cx);
        let mut config = background_config(&path, repeat);
        terminal.update(cx, |terminal, cx| terminal.set_config(config.clone(), cx));
        let cx = cx.add_empty_window();
        let view = cx.new(|cx| TerminalView::new(&terminal, ViewRole::Owner, cx));
        let loaded = loaded(&view, cx);
        assert_eq!(loaded.spec.repeat, repeat);
        assert_eq!(loaded.decoded.tiles, if repeat { (64, 128) } else { (1, 1) });
        let screen = (800., 600.);
        let geometry = super::super::layout(screen, loaded.decoded.size, loaded.decoded.tiles, &loaded.spec);
        assert_eq!(
            (geometry[0].width, geometry[0].height),
            if repeat { (256., 256.) } else { (4., 2.) }
        );
        std::fs::remove_file(&path).unwrap();
        config.background_image.repeat = !repeat;
        terminal.update(cx, |terminal, cx| terminal.set_config(config, cx));
        for _ in 0..2 {
            let fallback = prepared(&view, cx).unwrap();
            assert_retained(&fallback, &loaded);
            cx.run_until_parked();
        }
        assert!(!shared.background_loader().is_loading());
    }
}

#[gpui_kit::test]
fn a_replacement_background_changes_repeat_only_when_its_texture_is_ready(cx: &mut TestAppContext) {
    for valid in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.png");
        image::RgbaImage::from_pixel(4, 2, image::Rgba([10, 20, 30, 255]))
            .save(&path)
            .unwrap();
        let (terminal, shared) = idle(cx);
        terminal.update(cx, |terminal, cx| {
            terminal.set_config(background_config(&path, true), cx);
        });
        // only explicit preparations drive the load, so a redraw cannot finish both stages between assertions
        let cx = cx.add_empty_window();
        let view = cx.new(|cx| TerminalView::new(&terminal, ViewRole::Owner, cx));
        let initial = loaded(&view, cx);
        let replacement = dir.path().join("new.png");
        if valid {
            image::RgbaImage::from_pixel(4, 2, image::Rgba([40, 50, 60, 255]))
                .save(&replacement)
                .unwrap();
        } else {
            std::fs::write(&replacement, b"broken image").unwrap();
        }
        terminal.update(cx, |terminal, cx| {
            terminal.set_config(background_config(&replacement, false), cx);
        });
        assert_retained(&prepared(&view, cx).unwrap(), &initial);
        assert!(shared.background_loader().is_loading(), "the decode is in flight");
        cx.run_until_parked();
        assert_retained(&prepared(&view, cx).unwrap(), &initial);
        if valid {
            assert!(
                shared.background_loader().pending_bake().is_some(),
                "the bake is in flight"
            );
            let replacement = loaded(&view, cx);
            assert!(!Arc::ptr_eq(&replacement.decoded, &initial.decoded));
            assert!(!replacement.spec.repeat);
            assert_eq!(replacement.decoded.tiles, (1, 1));
            let rects = super::super::layout((800., 600.), replacement.decoded.size, (1, 1), &replacement.spec);
            assert_eq!(rects.len(), 1);
            assert_eq!((rects[0].width, rects[0].height), (4., 2.));
        } else {
            cx.run_until_parked();
            assert_retained(&prepared(&view, cx).unwrap(), &initial);
        }
        assert!(!shared.background_loader().is_loading());
    }
}
