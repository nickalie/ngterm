//! Page layout and focus over GPUI's test window, without starting a browser process.

use gpui_kit::{TestAppContext, size};
use ngterm_core::html::{HtmlLoadState, HtmlOverlay, HtmlPageFlags, HtmlSource};

use super::super::page::Storage;
use super::*;
use crate::html_overlay::panel::{PanelColors, TerminalColors};

fn page() -> Rc<Page> {
    let overlay = HtmlOverlay::new(
        HtmlSource::Url(url::Url::parse("https://example.test/").unwrap()),
        HtmlPageFlags::default(),
    );
    Page::new(
        &overlay,
        None,
        PanelColors::resolve(&TerminalColors::default(), None),
        Storage::Unavailable("test has no browser".into()),
        &crate::state::detached_poster(),
    )
}

#[gpui_kit::test]
fn a_page_lays_out_its_native_area_and_tracks_focus_until_released(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let page = page();
    let (view, cx) = cx.add_window_view(|window, cx| PageView::new(Rc::clone(&page), 123, window, cx));
    cx.update(|window, cx| {
        window.activate_window();
        page.apply(&page.overlay(), true, cx);
    });
    cx.simulate_resize(size(px(640.), px(400.)));
    cx.run_until_parked();
    let area = cx
        .debug_bounds("htmlOverlay.page")
        .expect("the page's content area is drawn");
    let wanted = page.wanted();
    let bounds = wanted.bounds.unwrap();
    assert_eq!(wanted.parent, Some(123));
    assert!(wanted.visible);
    assert!((bounds.width - f32::from(area.size.width)).abs() < f32::EPSILON);
    assert!((bounds.height - f32::from(area.size.height)).abs() < f32::EPSILON);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            assert!(Rc::ptr_eq(view.page(), &page));
            assert_eq!(view.focus(), &view.focus_handle(cx));
            window.focus(view.focus(), cx);
        });
    });
    cx.run_until_parked();
    assert!(
        page.wanted().focused,
        "the web view must receive the element's keyboard"
    );
    cx.update(Window::blur);
    cx.run_until_parked();
    assert!(!page.wanted().focused, "leaving the element hands the keyboard back");
    cx.update(|_, cx| view.update(cx, PageView::release));
    cx.run_until_parked();
    cx.update(|window, cx| view.update(cx, |view, cx| window.focus(view.focus(), cx)));
    cx.run_until_parked();
    assert!(
        !page.wanted().focused,
        "a released element cannot give the native view focus"
    );
}

#[gpui_kit::test]
fn only_a_failed_load_shows_its_current_selectable_error(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let page = page();
    let (view, cx) = cx.add_window_view(|window, cx| PageView::new(Rc::clone(&page), 0, window, cx));
    let mut overlay = page.overlay();
    overlay.load_error = Some("stale error".into());
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            assert!(
                view.error_text(&overlay, window, cx).is_none(),
                "loading hides a stale error"
            );
            overlay.load_state = HtmlLoadState::Failed;
            assert!(view.error_text(&overlay, window, cx).is_some());
            assert_eq!(view.error.as_ref().unwrap().text(cx).as_ref(), "stale error");
            overlay.load_error = Some("updated failure".into());
            assert!(view.error_text(&overlay, window, cx).is_some());
            assert_eq!(view.error.as_ref().unwrap().text(cx).as_ref(), "updated failure");
            overlay.load_error = None;
            assert!(view.error_text(&overlay, window, cx).is_none());
        });
    });
    overlay.load_state = HtmlLoadState::Failed;
    overlay.load_error = Some("could not load this page".into());
    cx.update(|_, cx| page.apply(&overlay, true, cx));
    cx.run_until_parked();
    assert!(cx.debug_bounds("htmlOverlay.page").is_some());
    view.read_with(cx, |view, cx| {
        assert_eq!(
            view.error.as_ref().unwrap().text(cx).as_ref(),
            "could not load this page"
        );
    });
    cx.update(|_, cx| page.apply(&overlay, false, cx));
    cx.run_until_parked();
    assert!(!page.wanted().visible);
}
