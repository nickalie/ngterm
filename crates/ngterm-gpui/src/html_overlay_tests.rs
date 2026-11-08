#![cfg(all(feature = "html", target_os = "linux"))]

use super::*;

#[gpui_kit::test]
fn hiding_a_mounted_page_hides_its_engine_view(cx: &mut gpui_kit::TestAppContext) {
    use ngterm_core::html::{HtmlOverlay, HtmlPageFlags, HtmlSource};
    cx.update(gpui_kit::init);
    let web = testing::install();
    let overlay = HtmlOverlay::new(
        HtmlSource::Url(url::Url::parse("https://example.invalid/").unwrap()),
        HtmlPageFlags::default(),
    );
    let (_, cx) = cx.add_window_view(|_, _| gpui_kit::EmptyView);
    let page = cx.update(|window, cx| mount(&overlay, None, window, cx)).unwrap();
    cx.update(|window, cx| {
        apply(&overlay, true, cx);
        set_covered(window, true, cx);
    });
    web.take();
    cx.update(|_, cx| hide(&page, cx));
    let calls = web.take();
    assert!(calls.contains(&testing::Call::Visible(false)), "{calls:?}");
}
