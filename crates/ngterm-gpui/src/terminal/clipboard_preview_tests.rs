use super::*;

/// Ghostty's `test/clipboard-multipart.py` parts, shared with the core's tests.
const RED_PNG: &[u8] = include_bytes!("../../../ngterm-ghostty-vt/tests/data/clipboard/red.png");
const ARBITRARY: &[u8] = include_bytes!("../../../ngterm-ghostty-vt/tests/data/clipboard/arbitrary.bin");

fn part(mime: &str, data: &[u8]) -> ClipboardContent {
    ClipboardContent {
        mime: mime.into(),
        data: data.to_vec(),
    }
}

/// A PNG `width`×`height` of one color.
fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    image::RgbImage::from_pixel(width, height, image::Rgb([0, 128, 255]))
        .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
        .unwrap();
    bytes
}

#[test]
fn each_part_is_previewed_as_an_image_text_or_a_sized_note() {
    assert_eq!(RED_PNG.len(), 73);
    assert_eq!(ARBITRARY.len(), 2048);
    let pages = pages(&[
        part("text/plain", "héllo\n".as_bytes()),
        part("image/png", RED_PNG),
        part("application/octet-stream", ARBITRARY),
        part("text/x-empty", b""),
    ]);
    // an image part is decoded only when shown
    assert_eq!(format!("{:?}", pages[1].body), "Decoding(73)");
    let shown: Vec<_> = pages
        .into_iter()
        .map(Page::decoded)
        .map(|page| (page.title, format!("{:?}", page.body), page.size))
        .collect();
    assert_eq!(
        shown,
        [
            ("text/plain".to_owned(), "Text(\"héllo\\n\")".to_owned(), 7),
            ("image/png".to_owned(), "Image(2x2)".to_owned(), 73),
            (
                "application/octet-stream".to_owned(),
                "NoPreview(2048)".to_owned(),
                2048
            ),
            ("text/x-empty".to_owned(), "Text(\"\")".to_owned(), 0),
        ]
    );
    // an image type that does not decode falls back to its bytes; a non-image type is never decoded
    assert!(matches!(body("image/png", b"plain words"), Body::Text(_)));
    assert!(matches!(body("image/png", ARBITRARY), Body::NoPreview(2048)));
    assert!(matches!(body("text/plain", RED_PNG), Body::NoPreview(73)));
}

#[test]
fn a_red_png_decodes_to_red_pixels_in_gpui_order() {
    let Some(Body::Image { image, width, height }) = decode_image(RED_PNG) else {
        panic!("the red PNG decodes")
    };
    assert_eq!((width, height), (2, 2));
    // BGRA: blue and red swapped for GPUI
    assert_eq!(&image.as_bytes(0).unwrap()[..4], &[0, 0, 255, 255]);
}

#[test]
fn tall_images_are_clamped_to_the_preview_height_and_small_ones_kept() {
    let tall = png(80, 6000);
    let Body::Image { width, height, .. } = body("image/png", &tall) else {
        panic!("the tall PNG decodes")
    };
    let (w, h) = fitted(width, height, 412.);
    assert!((h - PREVIEW_HEIGHT).abs() < 0.01, "{h}");
    // kept scaled down to the preview at twice its size, never at the image's own
    let Body::Image { image, .. } = body("image/png", &png(4000, 3000)) else {
        panic!("the large PNG decodes")
    };
    let kept = image.size(0);
    assert_eq!((kept.width.0, kept.height.0), (533, 400), "{kept:?}");
    assert_eq!(kept_size(2, 2), (2, 2));
    assert!((w - 80. * PREVIEW_HEIGHT / 6000.).abs() < 0.01, "{w}");
    assert_eq!(fitted(2, 2, 412.), (2., 2.));
    let (w, h) = fitted(4000, 100, 412.);
    assert!((w - 412.).abs() < 0.01 && (h - 10.3).abs() < 0.01, "{w}x{h}");
}

#[test]
fn oversized_images_are_refused_rather_than_decoded() {
    let huge = png(MAX_TEXTURE_SIDE + 1, 1);
    assert!(decode_image(&huge).is_none());
    assert!(matches!(body("image/png", &huge), Body::NoPreview(_)));
}

#[test]
fn the_macos_form_shows_the_text_or_a_summary_and_the_first_image() {
    let with_text = summary(&[part("image/png", RED_PNG), part("text/plain", b"copied")]);
    assert_eq!(format!("{:?}", with_text.image), "Some(Decoding(73))");
    let with_text = with_text.decoded();
    assert_eq!(format!("{:?}", with_text.body), "Text(\"copied\")");
    assert_eq!(format!("{:?}", with_text.image), "Some(Image(2x2))");
    let without = summary(&[part("application/octet-stream", ARBITRARY), part("text/x-empty", b"")]);
    assert_eq!(
        format!("{:?}", without.body),
        "Text(\"application/octet-stream (2048 bytes)\\ntext/x-empty (0 bytes)\")"
    );
    assert!(without.image.is_none());
    // no image part decodes: nothing under the text
    let undecodable = summary(&[part("image/png", b"not a png"), part("text/plain", b"x")]).decoded();
    assert!(undecodable.image.is_none());
    assert!(pages_for_os(&[]).is_empty(), "no representation, no page");
    assert_eq!(
        pages_for_os(&[part("text/plain", b"x"), part("text/html", b"y")]).len(),
        if cfg!(target_os = "macos") { 1 } else { 2 }
    );
}

#[test]
fn sizes_read_as_glib_writes_them() {
    assert_eq!(format_size(0), "0 bytes");
    assert_eq!(format_size(1), "1 byte");
    assert_eq!(format_size(999), "999 bytes");
    assert_eq!(format_size(2048), "2.0 kB");
    assert_eq!(format_size(1_500_000), "1.5 MB");
}

#[gpui_kit::test]
fn the_pager_moves_between_parts_and_stops_at_the_ends(cx: &mut gpui_kit::TestAppContext) {
    let preview = cx.new(|_| Preview::new(pages(&[part("text/plain", b"a"), part("image/png", RED_PNG)]), "none"));
    preview.update(cx, |preview, cx| {
        assert_eq!(preview.selected(), 0);
        preview.select(1, cx);
        assert_eq!(preview.selected(), 1);
        preview.select(2, cx);
        assert_eq!(preview.selected(), 1, "no page past the last");
    });
}

#[gpui_kit::test]
fn only_the_shown_page_decodes_off_the_main_thread_and_leaving_it_drops_the_image(cx: &mut gpui_kit::TestAppContext) {
    let preview = cx.new(|_| {
        Preview::new(
            pages(&[
                part("image/png", RED_PNG),
                part("text/plain", b"a"),
                part("image/png", RED_PNG),
            ]),
            "none",
        )
    });
    let bodies = |cx: &mut gpui_kit::TestAppContext| {
        preview.read_with(cx, |preview, _| {
            preview
                .pages()
                .iter()
                .map(|page| format!("{:?}", page.body))
                .collect::<Vec<_>>()
        })
    };
    // nothing decodes before the view asks (a queued prompt is never drawn)
    cx.run_until_parked();
    assert_eq!(bodies(cx), ["Decoding(73)", "Text(\"a\")", "Decoding(73)"]);
    preview.update(cx, Preview::load_selected);
    cx.run_until_parked();
    assert_eq!(bodies(cx), ["Image(2x2)", "Text(\"a\")", "Decoding(73)"]);
    // a page change drops the decoded image and decodes the new page's
    preview.update(cx, |preview, cx| preview.select(2, cx));
    cx.run_until_parked();
    assert_eq!(bodies(cx), ["Decoding(73)", "Text(\"a\")", "Image(2x2)"]);
    // a decode the page left before it landed is discarded
    preview.update(cx, |preview, cx| {
        preview.select(0, cx);
        preview.select(1, cx);
    });
    cx.run_until_parked();
    assert_eq!(bodies(cx), ["Decoding(73)", "Text(\"a\")", "Decoding(73)"]);
}

/// A large text is previewed by its first [`MAX_PREVIEW_TEXT`] bytes, cut between characters, in every form; the
/// page keeps the whole size for its caption.
#[test]
fn a_large_text_previews_its_head_and_how_much_is_left() {
    // "é" is two bytes, so the cap falls inside one: the cut moves back to the boundary before it
    let mut large = "x".repeat(MAX_PREVIEW_TEXT - 1);
    large.push_str(&"é".repeat(1000));
    let head = format!("{}\n… 2.0 kB more", "x".repeat(MAX_PREVIEW_TEXT - 1));
    assert_eq!(bounded_text(&large), head);
    assert_eq!(bounded_text("small ✓"), "small ✓");
    let exact = "y".repeat(MAX_PREVIEW_TEXT);
    assert_eq!(bounded_text(&exact), exact);

    let shown = |body: &Body| match body {
        Body::Text(text) => text.clone(),
        other => panic!("not text: {other:?}"),
    };
    let page = pages(&[part("text/plain", large.as_bytes())]).remove(0);
    assert_eq!((shown(&page.body), page.size), (head.clone(), large.len()));
    let page = summary(&[part("text/plain", large.as_bytes())]);
    assert_eq!((shown(&page.body), page.size), (head.clone(), large.len()));
    // an image part that does not decode falls back to its text, bounded too
    let page = pages(&[part("image/png", large.as_bytes())]).remove(0).decoded();
    assert_eq!(shown(&page.body), head);
}

use gpui_kit::AppContext as _;

/// What the preview draws for its page, by the kind of body (and the caption and pager beside it).
const DRAWN: [&str; 9] = [
    "clipboard-preview-text",
    "clipboard-preview-empty-part",
    "clipboard-preview-image",
    "clipboard-preview-loading",
    "clipboard-preview-no-preview",
    "clipboard-preview-note",
    "clipboard-preview-caption",
    "clipboard-preview-previous",
    "clipboard-preview-next",
];

/// `preview` under a Kitty write prompt in a test window, drawn.
fn prompt_over(
    cx: &mut gpui_kit::TestAppContext,
    preview: impl FnOnce(&mut App) -> gpui_kit::Entity<Preview>,
) -> (&mut gpui_kit::VisualTestContext, gpui_kit::Entity<Preview>) {
    let cx = crate::modal::tests::host(cx);
    let preview = cx.update(|window, cx| {
        let preview = preview(cx);
        crate::modal::open_alert_with(
            window,
            super::super::clipboard::kitty_alert(ClipboardAccess::Write, false),
            Some(preview.clone().into()),
            |_, _, _| {},
            cx,
        );
        preview
    });
    cx.run_until_parked();
    (cx, preview)
}

fn drawn(cx: &mut gpui_kit::VisualTestContext) -> Vec<&'static str> {
    DRAWN
        .into_iter()
        .filter(|selector| cx.debug_bounds(selector).is_some())
        .collect()
}

fn click(cx: &mut gpui_kit::VisualTestContext, selector: &'static str) {
    let at = cx.debug_bounds(selector).expect("laid out").center();
    cx.simulate_click(at, gpui_kit::Modifiers::default());
    cx.run_until_parked();
}

#[gpui_kit::test]
fn the_pager_steps_through_every_kind_of_part_and_stops_at_the_ends(cx: &mut gpui_kit::TestAppContext) {
    let tall = png(300, 400);
    let contents = [
        part("text/plain", "héllo".as_bytes()),
        part("image/png", &tall),
        part("application/octet-stream", ARBITRARY),
        part("text/x-empty", b""),
    ];
    let (cx, preview) = prompt_over(cx, |cx| cx.new(|_| Preview::new(pages(&contents), "none")));
    let selected = |cx: &mut gpui_kit::VisualTestContext| preview.read_with(cx, |preview, _| preview.selected());
    let pager = [
        "clipboard-preview-caption",
        "clipboard-preview-previous",
        "clipboard-preview-next",
    ];
    let shows = |body: &'static str| {
        let mut expected = vec![body];
        expected.extend(pager);
        expected.sort_by_key(|selector| DRAWN.iter().position(|known| known == selector));
        expected
    };
    assert_eq!(drawn(cx), shows("clipboard-preview-text"));
    assert!(cx.debug_bounds("clipboard-preview-text").unwrap().size.height > gpui_kit::px(0.));
    let well = cx.debug_bounds("clipboard-preview").unwrap();
    assert_eq!(well.size.height, gpui_kit::px(PREVIEW_HEIGHT));
    // nothing before the first part
    click(cx, "clipboard-preview-previous");
    assert_eq!(selected(cx), 0);
    // the image part decodes once shown and is drawn fitted to the preview's height
    click(cx, "clipboard-preview-next");
    assert_eq!(selected(cx), 1);
    assert_eq!(drawn(cx), shows("clipboard-preview-image"));
    let image = cx.debug_bounds("clipboard-preview-image").unwrap();
    assert_eq!(
        (image.size.width, image.size.height),
        (gpui_kit::px(150.), gpui_kit::px(PREVIEW_HEIGHT))
    );
    click(cx, "clipboard-preview-next");
    assert_eq!(drawn(cx), shows("clipboard-preview-no-preview"));
    click(cx, "clipboard-preview-next");
    assert_eq!(drawn(cx), shows("clipboard-preview-empty-part"));
    // nothing past the last part
    click(cx, "clipboard-preview-next");
    assert_eq!(selected(cx), 3);
    click(cx, "clipboard-preview-previous");
    assert_eq!(selected(cx), 2);
    assert_eq!(drawn(cx), shows("clipboard-preview-no-preview"));
}

#[gpui_kit::test]
fn a_single_part_shows_no_pager_and_an_image_that_does_not_decode_shows_a_note(cx: &mut gpui_kit::TestAppContext) {
    let contents = [part("image/png", ARBITRARY)];
    let (cx, _) = prompt_over(cx, |cx| cx.new(|_| Preview::new(pages(&contents), "none")));
    assert_eq!(drawn(cx), ["clipboard-preview-no-preview", "clipboard-preview-caption"]);
}

#[gpui_kit::test]
fn a_write_of_one_text_part_shows_it_with_its_caption(cx: &mut gpui_kit::TestAppContext) {
    let contents = [part("text/plain", b"copied")];
    let (cx, _) = prompt_over(cx, |cx| view(&contents, ClipboardAccess::Write, cx));
    assert_eq!(drawn(cx), ["clipboard-preview-text", "clipboard-preview-caption"]);
}

#[gpui_kit::test]
fn nothing_to_show_draws_the_directions_note_alone(cx: &mut gpui_kit::TestAppContext) {
    let (cx, preview) = prompt_over(cx, |cx| view(&[], ClipboardAccess::Write, cx));
    assert_eq!(drawn(cx), ["clipboard-preview-note"]);
    let note = preview.read_with(cx, |preview, _| preview.empty());
    assert_eq!(note, empty_note(ClipboardAccess::Write));
}

#[gpui_kit::test]
fn the_macos_form_draws_the_first_image_under_the_text(cx: &mut gpui_kit::TestAppContext) {
    let contents = [part("text/plain", b"copied"), part("image/png", &png(40, 20))];
    let (cx, _) = prompt_over(cx, |cx| cx.new(|_| Preview::new(vec![summary(&contents)], "none")));
    assert_eq!(
        drawn(cx),
        [
            "clipboard-preview-text",
            "clipboard-preview-image",
            "clipboard-preview-caption"
        ]
    );
    let text = cx.debug_bounds("clipboard-preview-text").unwrap();
    let image = cx.debug_bounds("clipboard-preview-image").unwrap();
    assert!(image.top() >= text.bottom(), "{image:?} under {text:?}");
    assert_eq!(
        (image.size.width, image.size.height),
        (gpui_kit::px(40.), gpui_kit::px(20.))
    );
}

#[gpui_kit::test]
fn leaving_a_page_drops_the_image_under_its_text_too(cx: &mut gpui_kit::TestAppContext) {
    let page = |text: &[u8]| summary(&[part("text/plain", text), part("image/png", RED_PNG)]);
    let preview = cx.new(|_| Preview::new(vec![page(b"one"), page(b"two")], "none"));
    let images = |cx: &mut gpui_kit::TestAppContext| {
        preview.read_with(cx, |preview, _| {
            preview
                .pages()
                .iter()
                .map(|page| format!("{:?}", page.image))
                .collect::<Vec<_>>()
        })
    };
    preview.update(cx, Preview::load_selected);
    cx.run_until_parked();
    assert_eq!(images(cx), ["Some(Image(2x2))", "Some(Decoding(73))"]);
    preview.update(cx, |preview, cx| preview.select(1, cx));
    cx.run_until_parked();
    assert_eq!(images(cx), ["Some(Decoding(73))", "Some(Image(2x2))"]);
}

#[test]
fn a_page_debugs_as_what_it_shows_never_its_bytes() {
    let page = pages(&[part("image/png", RED_PNG)]).remove(0);
    assert_eq!(
        format!("{page:?}"),
        "Page { title: \"image/png\", body: Decoding(73), size: 73, image: None, .. }"
    );
}
