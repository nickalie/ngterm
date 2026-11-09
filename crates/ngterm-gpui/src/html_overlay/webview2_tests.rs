//! Hiding a mounted page through the frontend's public entry point.

use std::rc::Rc;

use gpui_kit::{TestAppContext, px, size};

use super::fake_native::{self, Call, FakeView, url_overlay};
use super::page::{Page, Storage};
use super::*;
use crate::html_overlay::panel::{PanelColors, TerminalColors};

#[gpui_kit::test]
fn hiding_a_mounted_page_hands_back_the_keyboard_and_hides_its_web_view(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    fake_native::install();
    let overlay = url_overlay();
    let page = Page::new(
        &overlay,
        None,
        PanelColors::resolve(&TerminalColors::default(), None),
        Storage::Ephemeral,
        &crate::state::detached_poster(),
    );
    let (native, probe) = FakeView::made();
    page.realized(native);
    let (element, cx) = cx.add_window_view(|window, cx| PageView::new(Rc::clone(&page), 123, window, cx));
    cx.simulate_resize(size(px(640.), px(400.)));
    cx.update(|window, cx| {
        window.activate_window();
        page.apply(&overlay, true, cx);
        element.update(cx, |view, cx| window.focus(view.focus(), cx));
    });
    cx.run_until_parked();
    page.sync_native();
    let shown = probe.take();
    assert!(shown.contains(&Call::Visible(true)), "{shown:?}");
    assert!(shown.contains(&Call::Focus), "{shown:?}");
    cx.update(|_, cx| page.report(native::Report::Focus(true), cx));
    let before_hide = page.overlay();

    cx.update(|_, cx| hide(&element, cx));
    cx.run_until_parked();
    page.sync_native();
    assert_eq!(probe.take(), [Call::FocusParent, Call::Visible(false)]);
    assert_eq!(page.overlay(), before_hide, "hiding preserves the page's state");
    cx.update(|_, cx| hide(&element, cx));
    page.sync_native();
    assert!(probe.take().is_empty(), "hiding twice leaves the web view hidden");
}
