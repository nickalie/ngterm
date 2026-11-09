//! What only `WKWebView` reports: a load's start and its failures as reports of their own, and the keys a page hands
//! to the app.

use gpui_kit::{Keystroke, Modifiers, TestAppContext, VisualTestContext, px, size};
use ngterm_core::keymap::{linux_defaults, parse_keymap};

use super::super::fake_native::{self, FakeView, Model, Probe, url_overlay};
use super::*;
use crate::terminal::input::{KeyStroke, NativeKey, Source};

const OUTSIDE: &str = "https://outside.invalid/path";

struct Shown {
    page: Rc<Page>,
    model: Model,
    _web_view: Probe,
}

/// A URL page whose web view exists, its load in flight.
fn shown() -> Shown {
    fake_native::install();
    let overlay = url_overlay();
    let model = Model::new(&overlay);
    let colors = PanelColors::resolve(&TerminalColors::default(), None);
    let page = Page::new(&overlay, None, colors, Storage::Ephemeral, &model.poster());
    let (web_view, probe) = FakeView::made();
    page.realized(web_view);
    Shown {
        page,
        model,
        _web_view: probe,
    }
}

impl Shown {
    fn report(&self, report: Report, cx: &mut TestAppContext) {
        cx.update(|cx| self.page.report(report, cx));
    }

    /// The model's load state and error, once the page's posts landed.
    fn load(&mut self) -> (HtmlLoadState, Option<String>) {
        self.model.drain();
        let overlay = self.model.overlay();
        (overlay.load_state, overlay.load_error.clone())
    }
}

fn failed(id: u64, reason: &str) -> Report {
    Report::Failed {
        id,
        reason: reason.to_owned(),
    }
}

fn refused(id: u64) -> Report {
    Report::Navigation {
        navigation: Navigation {
            uri: OUTSIDE.to_owned(),
            main_frame: true,
            user: false,
            redirect: false,
            id,
        },
        allowed: false,
    }
}

#[gpui_kit::test]
fn a_platform_failure_fails_the_load_with_its_reason_and_the_next_start_loads_again(cx: &mut TestAppContext) {
    let mut shown = shown();
    shown.report(failed(3, "the server could not be found"), cx);
    assert_eq!(
        shown.load(),
        (HtmlLoadState::Failed, Some("the server could not be found".to_owned()))
    );
    // the page learns of the failure as the model's value comes back to it
    let failed = shown.model.overlay().clone();
    cx.update(|cx| shown.page.apply(&failed, true, cx));
    shown.report(Report::Started, cx);
    assert_eq!(shown.load(), (HtmlLoadState::Loading, None));
}

#[gpui_kit::test]
fn an_interruption_fails_a_load_that_shows_nothing_yet(cx: &mut TestAppContext) {
    let mut shown = shown();
    shown.report(
        Report::Interrupted {
            id: 5,
            reason: "download refused".to_owned(),
        },
        cx,
    );
    assert_eq!(
        shown.load(),
        (HtmlLoadState::Failed, Some("download refused".to_owned()))
    );
}

#[gpui_kit::test]
fn an_interruption_over_a_shown_document_leaves_it_loaded(cx: &mut TestAppContext) {
    let mut shown = shown();
    shown.report(Report::Committed, cx);
    shown.report(
        Report::Interrupted {
            id: 5,
            reason: "download refused".to_owned(),
        },
        cx,
    );
    assert_eq!(shown.load(), (HtmlLoadState::Loaded, None));
}

#[gpui_kit::test]
fn the_failure_ending_a_refused_navigation_is_the_refusals_once(cx: &mut TestAppContext) {
    let mut shown = shown();
    let refusal = (HtmlLoadState::Failed, Some(format!("navigation blocked: {OUTSIDE}")));
    shown.report(refused(4), cx);
    assert_eq!(shown.load(), refusal);
    shown.report(failed(4, "cancelled"), cx);
    assert_eq!(shown.load(), refusal, "the refusal already ended this navigation");
    shown.report(failed(4, "a later failure"), cx);
    assert_eq!(
        shown.load(),
        (HtmlLoadState::Failed, Some("a later failure".to_owned())),
        "only the refused navigation's own end is swallowed"
    );
}

#[gpui_kit::test]
fn an_interruption_ending_a_refused_navigation_is_the_refusals_too(cx: &mut TestAppContext) {
    let mut shown = shown();
    shown.report(refused(9), cx);
    let before = shown.load();
    shown.report(
        Report::Interrupted {
            id: 9,
            reason: "policy change".to_owned(),
        },
        cx,
    );
    assert_eq!(shown.load(), before);
}

fn mounted<'a>(shown: &Shown, cx: &'a mut TestAppContext) -> &'a mut VisualTestContext {
    cx.update(gpui_kit::init);
    let page = Rc::clone(&shown.page);
    let (element, cx) = cx.add_window_view(move |window, cx| PageView::new(page, 11, window, cx));
    drop(element);
    cx.simulate_resize(size(px(800.), px(500.)));
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    cx
}

/// Whether the keymap would take Esc, which it does only to cancel a half-typed leader.
fn a_leader_waits() -> bool {
    let escape = Keystroke {
        key: "escape".into(),
        key_char: None,
        modifiers: Modifiers::default(),
    };
    let native = NativeKey {
        keycode: 9,
        unshifted: Some('\u{1b}'),
        ascii_layout: true,
        ..NativeKey::default()
    };
    crate::keymap_dispatch::takes(&KeyStroke::new(&escape, &Source::Native(native), false))
}

#[gpui_kit::test]
fn a_key_the_app_takes_from_a_page_goes_through_the_keymap(cx: &mut TestAppContext) {
    let shown = shown();
    let keystroke = Keystroke::parse("ctrl-a").unwrap();
    // before a window: nowhere to dispatch it
    shown.report(Report::Key(keystroke.clone()), cx);
    let (keymap, diagnostics) = parse_keymap("command \"g\" ctrl+a>ctrl+g true\n", &linux_defaults());
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    crate::keymap_dispatch::load_keymap(&keymap);
    assert!(!a_leader_waits());
    let cx = mounted(&shown, cx);
    cx.update(|window, cx| {
        crate::keymap_dispatch::init(cx);
        crate::keymap_dispatch::register_window(
            window.window_handle(),
            crate::keymap_dispatch::KeyOrigin::Window(Uuid::new_v4()),
        );
    });
    cx.cx.update(|cx| shown.page.report(Report::Key(keystroke), cx));
    cx.run_until_parked();
    assert!(a_leader_waits(), "the leader is half typed");
}
