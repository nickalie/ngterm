//! The `WebKit` availability check against the running macOS, and hiding a page, which needs no web view.

use std::rc::Rc;

use gpui_kit::{TestAppContext, px, size};
use ngterm_core::html::{HtmlOverlay, HtmlPageFlags, HtmlSource};

use super::page::{Page, Storage};
use super::runtime::{installed_version, version_for};
use super::{PageView, hide};
use crate::html_overlay::panel::{PanelColors, TerminalColors};

#[test]
fn a_supported_mac_reports_the_system_webkit() {
    assert_eq!(installed_version().as_deref(), Ok("system WebKit"));
}

#[test]
fn a_mac_too_old_for_webkit_says_what_it_needs() {
    assert_eq!(version_for(true), Ok("system WebKit".to_owned()));
    assert_eq!(
        version_for(false),
        Err("HTML pages require macOS 14 or newer".to_owned())
    );
}

#[gpui_kit::test]
fn hiding_a_view_takes_its_page_off_the_screen(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let overlay = HtmlOverlay::new(
        HtmlSource::Url(url::Url::parse("https://example.test/").unwrap()),
        HtmlPageFlags::default(),
    );
    let page = Page::new(
        &overlay,
        None,
        PanelColors::resolve(&TerminalColors::default(), None),
        Storage::Unavailable("test has no browser".into()),
        &crate::state::detached_poster(),
    );
    let (view, cx) = cx.add_window_view(|window, cx| PageView::new(Rc::clone(&page), 1, window, cx));
    cx.update(|_, cx| page.apply(&page.overlay(), true, cx));
    cx.simulate_resize(size(px(640.), px(400.)));
    cx.run_until_parked();
    assert!(page.wanted().visible);
    cx.update(|_, cx| hide(&view, cx));
    cx.run_until_parked();
    assert!(!page.wanted().visible);
}
