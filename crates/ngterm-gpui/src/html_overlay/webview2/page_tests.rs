//! One page driven through the seam its web view sits behind (`fake_native`): the calls it makes on the web view, what
//! it posts to the core, the prompts it opens and the keyboard, on GPUI's test windows.

use std::io::Write as _;

use gpui_kit::{TestAppContext, VisualTestContext, px, size};
use ngterm_core::html::ReplicaContent;
use tempfile::{NamedTempFile, TempDir};

use super::super::fake_native::{self, Call, FakeEngine, FakeView, Model, Probe, overlay, url_overlay};
use super::*;

const NEXT: &str = "https://example.invalid/next";
const OUTSIDE: &str = "https://outside.invalid/path";

fn colors() -> PanelColors {
    PanelColors::resolve(&TerminalColors::default(), None)
}

struct Driven {
    page: Rc<Page>,
    view: Probe,
    model: Model,
    overlay: HtmlOverlay,
    engine: Rc<FakeEngine>,
}

/// `overlay`'s page with a web view, what it did at that cleared.
fn driven(overlay: &HtmlOverlay) -> Driven {
    let driven = realized(overlay);
    driven.view.take();
    driven
}

/// `overlay`'s page with a web view.
fn realized(overlay: &HtmlOverlay) -> Driven {
    let engine = fake_native::install();
    let model = Model::new(overlay);
    let page = Page::new(overlay, None, colors(), Storage::Ephemeral, &model.poster());
    let (web_view, view) = FakeView::made();
    page.realized(web_view);
    Driven {
        page,
        view,
        model,
        overlay: overlay.clone(),
        engine,
    }
}

impl Driven {
    fn report(&self, report: Report, cx: &mut TestAppContext) {
        cx.update(|cx| self.page.report(report, cx));
    }

    /// The model's load state and error, once the page's posts landed.
    fn load(&mut self) -> (HtmlLoadState, Option<String>) {
        self.model.drain();
        let overlay = self.model.overlay();
        (overlay.load_state, overlay.load_error.clone())
    }

    /// Takes the page on screen, the model's value as it stands.
    fn show(&self, cx: &mut App) {
        self.page.apply(&self.overlay, true, cx);
    }

    /// The model's next value, reloading `target`.
    fn reloading(&mut self, target: HtmlReloadTarget) -> HtmlOverlay {
        self.overlay.reload_revision += 1;
        self.overlay.reload_target = target;
        self.overlay.clone()
    }
}

fn navigation(uri: &str, main_frame: bool, user: bool, redirect: bool, id: u64) -> Navigation {
    Navigation {
        uri: uri.to_owned(),
        main_frame,
        user,
        redirect,
        id,
    }
}

fn nonce_of(page: &Page) -> String {
    page.nonce().split_once(':').unwrap().0.to_owned()
}

fn reply_of(call: &Call) -> serde_json::Value {
    let Call::Post(json) = call else {
        panic!("not a reply: {call:?}");
    };
    serde_json::from_str(json).unwrap()
}

/// `overlay`'s model with a page element in a test window of 800x500.
fn mounted<'a>(driven: &Driven, cx: &'a mut TestAppContext) -> (Entity<PageView>, &'a mut VisualTestContext) {
    cx.update(gpui_kit::init);
    let page = Rc::clone(&driven.page);
    let (element, cx) = cx.add_window_view(move |window, cx| PageView::new(page, 11, window, cx));
    cx.simulate_resize(size(px(800.), px(500.)));
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    (element, cx)
}

fn focus_element(element: &Entity<PageView>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| element.update(cx, |view, cx| window.focus(view.focus(), cx)));
    cx.run_until_parked();
}

fn file_overlay(html: &str) -> (HtmlOverlay, NamedTempFile) {
    let mut file = NamedTempFile::new().unwrap();
    file.write_all(html.as_bytes()).unwrap();
    let source = overlay(HtmlSource::File {
        path: file.path().to_string_lossy().into_owned(),
        grant_root: None,
    });
    (source, file)
}

fn document_overlay() -> HtmlOverlay {
    overlay(HtmlSource::Document {
        path: std::env::temp_dir().join("clicked.html").to_string_lossy().into_owned(),
    })
}

/// Another document's bytes, delivered as the current read's.
fn read_finishes(page: &Page, result: Result<String, String>) {
    let generation = page.document_generation.next();
    page.document_loaded(generation, result);
}

// MARK: - making the page

#[test]
fn a_url_page_loads_its_source_once_it_has_a_web_view() {
    let source = url_overlay();
    let engine = fake_native::install();
    let model = Model::new(&source);
    let page = Page::new(&source, None, colors(), Storage::Ephemeral, &model.poster());
    assert_eq!(page.id(), source.id);
    assert_eq!(page.overlay(), source);
    assert_eq!(page.storage(), &Storage::Ephemeral);
    assert!(!page.uses_saved_store());
    assert!(page.active() && !page.on_screen() && page.native().is_none());
    assert_eq!(page.theme(), colors().theme());
    assert!(page.element().is_none() && page.element_window().is_none());
    assert!(page.wants_native());
    page.set_building(true);
    assert!(!page.wants_native(), "a web view on its way is not asked for twice");
    let (web_view, view) = FakeView::made();
    page.realized(web_view);
    assert!(page.native().is_some() && !page.wants_native());
    assert_eq!(view.take(), [Call::Load("https://example.invalid/start".to_owned())]);
    assert!(engine.opened().is_empty());
}

#[test]
fn a_page_on_the_saved_store_says_so_and_one_without_a_store_wants_no_web_view() {
    fake_native::install();
    let saved = Page::new(
        &url_overlay(),
        None,
        colors(),
        Storage::Saved,
        &crate::state::detached_poster(),
    );
    assert!(saved.uses_saved_store());
    let unavailable = Storage::Unavailable("no profile".to_owned());
    let without = Page::new(
        &url_overlay(),
        None,
        colors(),
        unavailable,
        &crate::state::detached_poster(),
    );
    assert!(!without.wants_native());
}

#[test]
fn a_page_without_its_store_fails_with_the_reason_and_loads_nothing() {
    let source = url_overlay();
    fake_native::install();
    let mut model = Model::new(&source);
    let unavailable = Storage::Unavailable("no profile".to_owned());
    let page = Page::new(&source, None, colors(), unavailable, &model.poster());
    let (web_view, view) = FakeView::made();
    page.realized(web_view);
    assert!(view.take().is_empty());
    model.drain();
    assert_eq!(model.overlay().load_state, HtmlLoadState::Failed);
    assert_eq!(model.overlay().load_error.as_deref(), Some("no profile"));
}

#[test]
fn only_the_first_engine_failure_is_reported() {
    let mut driven = driven(&url_overlay());
    driven.page.engine_failed("the engine went");
    assert_eq!(
        driven.load(),
        (HtmlLoadState::Failed, Some("the engine went".to_owned()))
    );
    driven.page.engine_failed("and again");
    assert_eq!(driven.model.pending(), 0, "the first failure stands");
    assert!(!driven.page.wants_native());
}

#[test]
fn a_file_page_with_a_grant_loads_through_the_grant_and_shares_its_root() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("page.html");
    std::fs::write(&path, "<p>page</p>").unwrap();
    let path = path.to_string_lossy().into_owned();
    let source = overlay(HtmlSource::File {
        path: path.clone(),
        grant_root: Some(directory.path().to_string_lossy().into_owned()),
    });
    let driven = realized(&source);
    assert_eq!(
        driven.view.take(),
        [Call::Load(scheme::granted_address(&path).unwrap())]
    );
    let root = driven.page.grant().root.lock().unwrap();
    assert_eq!(*root, policy::GrantRoot::new(directory.path()));
}

#[test]
fn a_file_page_named_by_a_relative_path_fails() {
    let source = overlay(HtmlSource::File {
        path: "relative.html".to_owned(),
        grant_root: Some("relative".to_owned()),
    });
    let mut driven = realized(&source);
    assert!(driven.view.take().is_empty());
    assert_eq!(
        driven.load(),
        (
            HtmlLoadState::Failed,
            Some("not an absolute path: relative.html".to_owned())
        )
    );
}

#[test]
fn a_file_page_without_a_grant_serves_its_text_and_the_app_loads_it_past_the_policy() {
    let (source, _file) = file_overlay("<p>text page</p>");
    let driven = realized(&source);
    let id = driven.page.id();
    assert_eq!(driven.view.take(), [Call::Load(scheme::text_address(id))]);
    assert_eq!(driven.page.text().lock().unwrap().as_deref(), Some("<p>text page</p>"));
    // the app's own load is the next main-frame navigation, wherever the web view says it goes
    let own = navigation(OUTSIDE, true, false, false, 1);
    assert!(driven.page.allows(&own));
    assert!(!driven.page.allows(&own), "only that one navigation");
}

#[test]
fn a_file_page_whose_file_cannot_be_read_fails_with_the_system_error() {
    let directory = TempDir::new().unwrap();
    let missing = directory.path().join("gone.html");
    let source = overlay(HtmlSource::File {
        path: missing.to_string_lossy().into_owned(),
        grant_root: None,
    });
    let expected = std::fs::read_to_string(&missing).unwrap_err().to_string();
    let mut driven = realized(&source);
    assert!(driven.view.take().is_empty());
    assert_eq!(driven.load(), (HtmlLoadState::Failed, Some(expected)));
}

#[test]
fn an_html_replica_loads_through_its_own_protocol() {
    let source = overlay(HtmlSource::Replica {
        path: "/docs/a b.html".to_owned(),
        content: ReplicaContent::Html,
    });
    let driven = realized(&source);
    let id = driven.page.id();
    assert_eq!(
        driven.view.take(),
        [Call::Load(scheme::replica_address(id, "/docs/a b.html").unwrap())]
    );
}

#[test]
fn a_built_document_shows_through_the_text_protocol_only_when_it_is_the_latest_read() {
    let source = overlay(HtmlSource::Image {
        path: "image.png".to_owned(),
    });
    let mut driven = realized(&source);
    assert!(driven.view.take().is_empty(), "the document is read on a worker first");
    let page = Rc::clone(&driven.page);
    let stale = page.document_generation.next();
    let current = page.document_generation.next();
    page.document_loaded(stale, Ok("<p>stale</p>".to_owned()));
    assert!(driven.view.take().is_empty(), "a superseded read is dropped");
    page.document_loaded(current, Ok("<p>page</p>".to_owned()));
    assert_eq!(driven.view.take(), [Call::Load(scheme::text_address(page.id()))]);
    assert_eq!(page.text().lock().unwrap().as_deref(), Some("<p>page</p>"));
    read_finishes(&page, Err("the file went away".to_owned()));
    assert_eq!(
        driven.load(),
        (HtmlLoadState::Failed, Some("the file went away".to_owned()))
    );
}

#[gpui_kit::test]
fn a_document_that_blocks_the_network_waits_for_its_filter(cx: &mut TestAppContext) {
    let source = document_overlay();
    let driven = driven(&source);
    let page = &driven.page;
    read_finishes(page, Ok("<p>document</p>".to_owned()));
    assert!(driven.view.take().is_empty(), "the filter is not in place");
    assert_eq!(page.text().lock().unwrap().as_deref(), Some("<p>document</p>"));
    driven.report(Report::NetworkFilterInstalled, cx);
    assert_eq!(driven.view.take(), [Call::Load(scheme::text_address(page.id()))]);
    driven.report(Report::NetworkFilterInstalled, cx);
    assert!(driven.view.take().is_empty(), "the filter's report loads once");
}

#[gpui_kit::test]
fn a_filter_that_could_not_be_installed_fails_the_page_for_every_read(cx: &mut TestAppContext) {
    let source = document_overlay();
    let mut driven = driven(&source);
    driven.report(Report::NetworkFilterFailed("no filter".to_owned()), cx);
    assert_eq!(
        driven.load(),
        (
            HtmlLoadState::Failed,
            Some(errors::NETWORK_FILTER_UNAVAILABLE.to_owned())
        )
    );
    read_finishes(&driven.page, Ok("<p>document</p>".to_owned()));
    assert!(driven.view.take().is_empty());
    assert_eq!(driven.model.pending(), 1, "the read fails as the filter did");
}

// MARK: - the scripts and the canvas

#[test]
fn a_page_with_scripts_gets_the_helper_only_when_bridged_and_scripted() {
    let (mut file, _held) = file_overlay("");
    let guard = |page: &Page| page.scripts()[0].clone();
    let driven_page = realized(&file);
    assert_eq!(guard(&driven_page.page), bridge::GUARD_SCRIPT);
    assert_eq!(driven_page.page.scripts().len(), 2, "no helper without scripts");
    file.flags.javascript = true;
    let scripted = realized(&file);
    assert_eq!(scripted.page.scripts()[2], bridge::helper_script());
    let mut url = url_overlay();
    url.flags.javascript = true;
    assert_eq!(realized(&url).page.scripts().len(), 2, "a URL page has no bridge");
}

#[test]
fn a_themed_page_shows_the_panel_canvas_and_a_url_page_keeps_the_browser_white() {
    let (file, _held) = file_overlay("");
    assert_eq!(
        realized(&file).page.canvas(),
        rgb8(&colors().background),
        "a file page's document shows the panel background"
    );
    assert_eq!(realized(&url_overlay()).page.canvas(), None);
}

// MARK: - the native window

#[gpui_kit::test]
fn the_native_window_is_placed_by_the_pump_alone_and_follows_the_keyboard(cx: &mut TestAppContext) {
    let driven = driven(&url_overlay());
    let (element, cx) = mounted(&driven, cx);
    assert_eq!(driven.page.element().as_ref(), Some(&element));
    cx.update(|_, cx| driven.show(cx));
    cx.run_until_parked();
    assert!(
        driven.view.take().is_empty(),
        "placed from the pump, outside GPUI's callbacks"
    );
    driven.page.sync_native();
    let calls = driven.view.take();
    assert!(
        matches!(calls.as_slice(), [Call::Bounds(_), Call::Visible(true)]),
        "{calls:?}"
    );
    driven.page.sync_native();
    assert!(driven.view.take().is_empty(), "nothing moved");

    focus_element(&element, cx);
    driven.page.sync_native();
    assert_eq!(driven.view.take(), [Call::Focus]);
    cx.update(|_, cx| driven.page.report(Report::Focus(true), cx));
    assert!(driven.page.holds_native_keyboard());
    driven.page.sync_native();
    assert!(driven.view.take().is_empty(), "the web view holds it already");

    cx.update(gpui_kit::Window::blur);
    cx.run_until_parked();
    driven.page.sync_native();
    assert_eq!(
        driven.view.take(),
        [Call::FocusParent],
        "the window takes the keyboard back"
    );

    focus_element(&element, cx);
    driven.page.sync_native();
    driven.view.take();
    cx.update(|_, cx| driven.page.apply(&driven.overlay, false, cx));
    driven.page.sync_native();
    assert_eq!(
        driven.view.take(),
        [Call::FocusParent, Call::Visible(false)],
        "a hidden page hands the keyboard back before it goes"
    );
}

#[gpui_kit::test]
fn a_click_into_the_web_view_gives_its_element_the_windows_keyboard(cx: &mut TestAppContext) {
    let driven = driven(&url_overlay());
    let (element, cx) = mounted(&driven, cx);
    // the pump's reports run outside any window's update
    cx.cx.update(|cx| driven.page.report(Report::Focus(true), cx));
    cx.run_until_parked();
    let focused = cx.update(|window, cx| element.read(cx).focus().is_focused(window));
    assert!(focused);
    cx.cx.update(|cx| driven.page.report(Report::Focus(false), cx));
    assert!(!driven.page.holds_native_keyboard());
}

#[gpui_kit::test]
fn a_session_moved_to_another_window_reparents_the_web_view(cx: &mut TestAppContext) {
    let driven = driven(&url_overlay());
    let (element, cx) = mounted(&driven, cx);
    let handle = cx.update(|window, _| window.window_handle());
    driven.page.place(Bounds {
        x: 1.,
        y: 2.,
        width: 3.,
        height: 4.,
    });
    driven.page.sync_native();
    driven.view.take();
    driven.page.set_element(element.downgrade(), handle, 22);
    assert_eq!(driven.page.element_window(), Some(handle));
    driven.page.sync_native();
    assert_eq!(driven.view.take(), [Call::Reparent(22)]);
}

#[gpui_kit::test]
fn a_layer_the_app_draws_and_a_failure_hide_the_native_window(cx: &mut TestAppContext) {
    let mut driven = driven(&url_overlay());
    let (_element, cx) = mounted(&driven, cx);
    cx.update(|_, cx| driven.show(cx));
    cx.run_until_parked();
    assert!(driven.page.wanted().visible);
    cx.update(|window, cx| registry::set_covered(window, true, cx));
    assert!(
        !driven.page.wanted().visible,
        "the app's drawing sits under a native window"
    );
    cx.update(|window, cx| registry::set_covered(window, false, cx));
    assert!(driven.page.wanted().visible);
    driven.overlay.load_state = HtmlLoadState::Failed;
    cx.update(|_, cx| driven.show(cx));
    assert!(
        !driven.page.wanted().visible,
        "the failure is drawn where the page would be"
    );
}

#[test]
fn a_page_with_no_web_view_is_not_placed() {
    fake_native::install();
    let page = Page::new(
        &url_overlay(),
        None,
        colors(),
        Storage::Ephemeral,
        &crate::state::detached_poster(),
    );
    page.sync_native();
    assert_eq!(page.wanted(), Placement::default());
}

// MARK: - the theme

fn terminal(background: &str) -> TerminalColors {
    TerminalColors {
        background: Some(background.to_owned()),
        foreground: Some("#f0f0f0".to_owned()),
        palette: Vec::new(),
    }
}

#[gpui_kit::test]
fn a_file_page_reloads_once_its_new_theme_script_is_in_place(cx: &mut TestAppContext) {
    let (source, _held) = file_overlay("<p>page</p>");
    let driven = driven(&source);
    driven.report(Report::Committed, cx);
    let colors = terminal("#102030");
    cx.update(|cx| driven.page.apply_theme(&colors, cx));
    let next = PanelColors::resolve(&colors, None).theme();
    assert_eq!(driven.page.theme(), next);
    assert_eq!(
        driven.view.take(),
        [
            Call::Theme(bridge::theme_script(&next.script(true))),
            Call::Background((0x10, 0x20, 0x30))
        ]
    );
    driven.report(Report::ThemeInstalled, cx);
    assert_eq!(driven.view.take(), [Call::Load(scheme::text_address(driven.page.id()))]);
    driven.report(Report::ThemeInstalled, cx);
    assert!(driven.view.take().is_empty(), "one reload for one change");
    cx.update(|cx| driven.page.apply_theme(&colors, cx));
    assert!(driven.view.take().is_empty(), "the same theme changes nothing");
}

#[gpui_kit::test]
fn a_url_page_wears_a_new_theme_at_its_next_load(cx: &mut TestAppContext) {
    let driven = driven(&url_overlay());
    let colors = terminal("#102030");
    cx.update(|cx| driven.page.apply_theme(&colors, cx));
    let next = PanelColors::resolve(&colors, None).theme();
    assert_eq!(
        driven.view.take(),
        [Call::Theme(bridge::theme_script(&next.script(false)))],
        "no reload and no canvas for a URL page"
    );
}

#[gpui_kit::test]
fn an_image_wears_no_theme_but_a_closed_page_follows_nothing(cx: &mut TestAppContext) {
    let image = driven(&overlay(HtmlSource::Image {
        path: "image.png".to_owned(),
    }));
    let colors = terminal("#102030");
    cx.update(|cx| image.page.apply_theme(&colors, cx));
    assert!(image.view.take().is_empty());
    assert_eq!(
        image.page.colors().background,
        PanelColors::resolve(&colors, None).background
    );

    let closed = driven(&url_overlay());
    cx.update(|cx| {
        closed.page.close(cx);
        closed.page.apply_theme(&colors, cx);
    });
    assert_eq!(closed.page.theme(), self::colors().theme());
}

#[gpui_kit::test]
fn a_new_theme_before_the_web_view_exists_fails_a_page_without_a_store(cx: &mut TestAppContext) {
    let (source, _held) = file_overlay("<p>page</p>");
    fake_native::install();
    let mut model = Model::new(&source);
    let unavailable = Storage::Unavailable("no profile".to_owned());
    let page = Page::new(&source, None, colors(), unavailable, &model.poster());
    cx.update(|cx| page.apply_theme(&terminal("#102030"), cx));
    model.drain();
    assert_eq!(model.overlay().load_error.as_deref(), Some("no profile"));
}

#[gpui_kit::test]
fn a_text_page_waits_for_its_web_view_before_building_for_a_new_theme(cx: &mut TestAppContext) {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("notes.txt");
    std::fs::write(&path, "hello").unwrap();
    let source = overlay(HtmlSource::Text {
        path: path.to_string_lossy().into_owned(),
        line: None,
    });
    fake_native::install();
    let model = Model::new(&source);
    let page = Page::new(&source, None, colors(), Storage::Ephemeral, &model.poster());
    assert!(page.text_bridge.is_some() && page.native().is_none());
    cx.update(|cx| page.apply_theme(&terminal("#102030"), cx));
    assert_eq!(model.pending(), 0, "no document build posts to the model yet");
    assert!(page.text().lock().unwrap().is_none());
    let (view, _probe) = FakeView::made();
    page.realized(view);
    assert!(model.pending() > 0, "the first build starts once the web view exists");
}

// MARK: - the navigation policy

#[test]
fn the_policy_answers_for_the_pages_origin_and_never_for_its_own_addresses() {
    let driven = driven(&url_overlay());
    let page = &driven.page;
    assert!(page.allows(&navigation("https://example.invalid/other", true, false, false, 1)));
    assert!(
        !page.allows(&navigation(OUTSIDE, true, true, false, 2)),
        "another origin is offered, not followed"
    );
    assert!(
        page.allows(&navigation(OUTSIDE, false, false, false, 3)),
        "a frame may embed another origin"
    );
    assert!(!page.allows(&navigation("file:///C:/secret.html", true, true, false, 4)));
    let foreign = format!("{}secret", scheme::text_address(page.id()));
    assert!(
        !page.allows(&navigation(&foreign, true, false, false, 5)),
        "the text protocol serves one document"
    );
    assert!(!page.allows(&navigation("not a url", true, true, false, 6)));
}

#[gpui_kit::test]
fn a_closed_page_allows_nothing_and_reports_nothing(cx: &mut TestAppContext) {
    let driven = driven(&url_overlay());
    cx.update(|cx| driven.page.close(cx));
    assert!(!driven.page.active());
    assert!(
        !driven
            .page
            .allows(&navigation("https://example.invalid/other", true, false, false, 1))
    );
    driven.report(
        Report::Navigation {
            navigation: navigation(OUTSIDE, true, false, false, 2),
            allowed: false,
        },
        cx,
    );
    driven.report(
        Report::Navigation {
            navigation: navigation(NEXT, true, false, false, 3),
            allowed: true,
        },
        cx,
    );
    driven.report(Report::Committed, cx);
    driven.report(Report::Source(NEXT.to_owned()), cx);
    driven.report(Report::TextPage("{}".to_owned()), cx);
    cx.update(|cx| driven.page.apply(&driven.overlay, true, cx));
    driven.page.focused();
    driven.page.user_input(false);
    assert_eq!(driven.model.pending(), 0);
    assert!(driven.view.take().is_empty());
}

// MARK: - the load and the page shown

#[gpui_kit::test]
fn a_load_is_reported_from_start_to_finish_with_the_page_it_shows(cx: &mut TestAppContext) {
    let mut driven = realized(&url_overlay());
    driven.view.take();
    assert_eq!(driven.load().0, HtmlLoadState::Loading);
    driven.report(
        Report::Navigation {
            navigation: navigation(NEXT, true, false, false, 7),
            allowed: true,
        },
        cx,
    );
    assert_eq!(driven.model.pending(), 0, "the same state is not reported again");
    driven.report(Report::Source(NEXT.to_owned()), cx);
    driven.report(Report::Title("Next".to_owned()), cx);
    driven.report(
        Report::History {
            back: true,
            forward: false,
        },
        cx,
    );
    driven.report(Report::Committed, cx);
    driven.report(
        Report::Completed {
            id: 7,
            success: true,
            status: 0,
            http: 200,
        },
        cx,
    );
    assert_eq!(driven.load(), (HtmlLoadState::Loaded, None));
    let shown = driven.model.overlay().current.clone().unwrap();
    assert_eq!(shown.page, NEXT);
    assert_eq!(shown.title.as_deref(), Some("Next"));
    assert!(shown.can_go_back && !shown.can_go_forward);
    assert_eq!(driven.page.browser_url().unwrap().as_str(), NEXT);
}

#[gpui_kit::test]
fn an_http_error_is_a_page_unless_the_apps_own_protocol_refused_the_read(cx: &mut TestAppContext) {
    let mut driven = realized(&url_overlay());
    driven.report(
        Report::Completed {
            id: 1,
            success: true,
            status: 0,
            http: 404,
        },
        cx,
    );
    assert_eq!(driven.load().0, HtmlLoadState::Loaded);

    driven.page.load_original();
    driven.report(Report::Source(NEXT.to_owned()), cx);
    driven
        .page
        .grant()
        .refused
        .lock()
        .unwrap()
        .replace((NEXT.to_owned(), "outside the grant".to_owned()));
    driven.report(
        Report::Completed {
            id: 2,
            success: true,
            status: 0,
            http: 403,
        },
        cx,
    );
    assert_eq!(
        driven.load(),
        (HtmlLoadState::Failed, Some("outside the grant".to_owned()))
    );

    driven
        .page
        .grant()
        .refused
        .lock()
        .unwrap()
        .replace(("https://other.invalid/".to_owned(), "another address".to_owned()));
    driven.page.load_original();
    driven.report(
        Report::Completed {
            id: 3,
            success: true,
            status: 0,
            http: 403,
        },
        cx,
    );
    assert_eq!(
        driven.load().0,
        HtmlLoadState::Loaded,
        "a refusal of another address is not this load's"
    );
}

#[gpui_kit::test]
fn a_failed_navigation_names_why_and_a_superseded_one_is_not_a_failure(cx: &mut TestAppContext) {
    let mut driven = realized(&url_overlay());
    driven.report(
        Report::Completed {
            id: 1,
            success: false,
            status: OPERATION_CANCELED,
            http: 0,
        },
        cx,
    );
    assert_eq!(driven.load().0, HtmlLoadState::Loading);
    driven.report(
        Report::Completed {
            id: 2,
            success: false,
            status: 13,
            http: 0,
        },
        cx,
    );
    assert_eq!(driven.load(), (HtmlLoadState::Failed, Some(error_text(13))));
}

#[gpui_kit::test]
fn a_refused_navigation_of_the_load_in_flight_fails_it_and_its_end_is_the_refusals(cx: &mut TestAppContext) {
    let mut driven = realized(&url_overlay());
    driven.report(
        Report::Navigation {
            navigation: navigation(OUTSIDE, true, false, false, 4),
            allowed: false,
        },
        cx,
    );
    assert_eq!(
        driven.load(),
        (HtmlLoadState::Failed, Some(format!("navigation blocked: {OUTSIDE}")))
    );
    driven.report(
        Report::Completed {
            id: 4,
            success: false,
            status: 13,
            http: 0,
        },
        cx,
    );
    assert_eq!(driven.model.pending(), 0, "its end is the refusal's");
}

#[gpui_kit::test]
fn a_content_process_that_ended_fails_the_page_and_drops_its_address(cx: &mut TestAppContext) {
    let mut driven = realized(&url_overlay());
    driven.report(Report::Committed, cx);
    driven.report(Report::ProcessFailed { browser: false }, cx);
    assert_eq!(
        driven.load(),
        (HtmlLoadState::Failed, Some("web content process terminated".to_owned()))
    );
    assert!(driven.page.state.borrow().shown_url.is_none());
}

#[gpui_kit::test]
fn a_browsing_page_keeps_the_address_it_committed_while_its_origin_holds(cx: &mut TestAppContext) {
    let mut source = url_overlay();
    source.flags.browse = true;
    let driven = driven(&source);
    driven.report(Report::Source(NEXT.to_owned()), cx);
    driven.report(Report::Committed, cx);
    assert_eq!(driven.page.browser_url().unwrap().as_str(), NEXT);
    let page = format!("{NEXT}#fragment");
    driven.report(Report::Source(page.clone()), cx);
    assert_eq!(driven.page.browser_url().unwrap().as_str(), page);
    driven.report(Report::Source(OUTSIDE.to_owned()), cx);
    assert_eq!(
        driven.page.browser_url().unwrap().as_str(),
        page,
        "the old document's address stands until the new one commits"
    );
}

#[gpui_kit::test]
fn window_close_asks_the_core_and_stays_asked_while_the_session_is_held(cx: &mut TestAppContext) {
    let mut driven = driven(&url_overlay());
    driven.report(Report::CloseRequested, cx);
    assert_eq!(driven.model.pending(), 1);
    cx.update(|cx| driven.show(cx));
    assert_eq!(driven.model.pending(), 2, "still asked at the model's next value");
    driven.model.drain();
    assert!(driven.model.state.core.html_overlay_slot(driven.page.id()).is_none());
}

#[gpui_kit::test]
fn a_page_with_no_text_bridge_ignores_a_renderers_reports_and_a_refused_download(cx: &mut TestAppContext) {
    let driven = driven(&url_overlay());
    driven.report(Report::TextPage("{}".to_owned()), cx);
    driven.report(Report::DownloadRefused(NEXT.to_owned()), cx);
    assert!(driven.view.take().is_empty());
    assert_eq!(driven.model.pending(), 0);
}

// MARK: - reloads

#[gpui_kit::test]
fn a_reload_of_a_committed_page_reloads_and_before_a_commit_loads_the_source_again(cx: &mut TestAppContext) {
    let mut driven = realized(&url_overlay());
    driven.view.take();
    let next = driven.reloading(HtmlReloadTarget::Current);
    cx.update(|cx| driven.page.apply(&next, true, cx));
    assert_eq!(
        driven.view.take(),
        [Call::Load("https://example.invalid/start".to_owned())]
    );
    driven.report(Report::Committed, cx);
    let next = driven.reloading(HtmlReloadTarget::Current);
    cx.update(|cx| driven.page.apply(&next, true, cx));
    assert_eq!(driven.view.take(), [Call::Reload]);
    let next = driven.reloading(HtmlReloadTarget::Original);
    cx.update(|cx| driven.page.apply(&next, true, cx));
    assert_eq!(
        driven.view.take(),
        [Call::Load("https://example.invalid/start".to_owned())]
    );
    cx.update(|cx| driven.page.apply(&next, true, cx));
    assert!(driven.view.take().is_empty(), "the same revision reloads nothing");
}

// MARK: - the bridge and the user's input

#[gpui_kit::test]
fn a_bridge_request_is_answered_once_with_its_id(cx: &mut TestAppContext) {
    let (source, _held) = file_overlay("<p>page</p>");
    let mut driven = driven(&source);
    registry::add_page(&driven.page);
    let nonce = nonce_of(&driven.page);
    let raw = format!(r#"{{"n":"{nonce}","k":"bridge","id":9,"body":{{"cmd":"notify","args":{{"text":"hi"}}}}}}"#);
    driven.report(Report::Message(raw), cx);
    assert!(driven.view.take().is_empty(), "the core answers on the main loop");
    driven.model.drain();
    let calls = driven.view.take();
    assert_eq!(calls.len(), 1, "{calls:?}");
    let reply = reply_of(&calls[0]);
    assert_eq!(reply["ngterm"], "reply");
    assert_eq!(reply["id"], 9);
}

#[gpui_kit::test]
fn a_bridge_request_the_core_cannot_read_is_rejected(cx: &mut TestAppContext) {
    let (source, _held) = file_overlay("<p>page</p>");
    let mut driven = driven(&source);
    registry::add_page(&driven.page);
    let nonce = nonce_of(&driven.page);
    let raw = format!(r#"{{"n":"{nonce}","k":"bridge","id":3,"body":"not an object"}}"#);
    driven.report(Report::Message(raw), cx);
    driven.model.drain();
    let calls = driven.view.take();
    assert!(reply_of(&calls[0])["error"].is_string(), "{calls:?}");
}

#[test]
fn a_closed_page_refuses_a_bridge_request_with_page_closed() {
    let driven = driven(&url_overlay());
    let page = Rc::clone(&driven.page);
    page.state.borrow_mut().life = Life::Released;
    page.bridge_message(5, "{}".to_owned());
    let calls = driven.view.take();
    assert_eq!(reply_of(&calls[0])["error"], "page closed");
    assert_eq!(reply_of(&calls[0])["id"], 5);
    assert_eq!(driven.model.pending(), 0);
}

#[gpui_kit::test]
fn a_replicas_request_goes_to_the_machine_that_opened_it(cx: &mut TestAppContext) {
    let source = overlay(HtmlSource::Replica {
        path: "/page.html".to_owned(),
        content: ReplicaContent::Html,
    });
    let driven = driven(&source);
    registry::add_page(&driven.page);
    let nonce = nonce_of(&driven.page);
    let raw = format!(r#"{{"n":"{nonce}","k":"bridge","id":6,"body":{{"cmd":"notify"}}}}"#);
    driven.report(Report::Message(raw), cx);
    // the registry has no way to the core here: a relay that cannot go says so
    let calls = driven.view.take();
    assert_eq!(reply_of(&calls[0])["error"], "page closed");
}

#[gpui_kit::test]
fn a_message_that_is_not_ngterms_own_does_nothing(cx: &mut TestAppContext) {
    let driven = driven(&url_overlay());
    driven.report(Report::Message("{\"page\":\"own\"}".to_owned()), cx);
    assert_eq!(driven.model.pending(), 0);
    assert!(driven.view.take().is_empty());
}

#[gpui_kit::test]
fn a_click_in_a_page_reaches_the_core_as_user_activity(cx: &mut TestAppContext) {
    let mut driven = driven(&url_overlay());
    let nonce = nonce_of(&driven.page);
    let raw = format!(r#"{{"n":"{nonce}","k":"input","click":true}}"#);
    driven.report(Report::Message(raw), cx);
    assert_eq!(driven.model.pending(), 1);
    driven.model.drain();
    assert_eq!(driven.model.pending(), 0);
    driven.page.focused();
    assert_eq!(driven.model.pending(), 1, "taking the keyboard is reported too");
    driven.model.drain();
}

#[gpui_kit::test]
fn input_for_a_page_that_left_its_slot_is_dropped(cx: &mut TestAppContext) {
    let mut driven = driven(&url_overlay());
    let nonce = nonce_of(&driven.page);
    let raw = format!(r#"{{"n":"{nonce}","k":"input","click":true}}"#);
    driven.report(Report::CloseRequested, cx);
    driven.model.drain();
    driven.report(Report::Message(raw), cx);
    driven.model.drain();
    assert!(driven.model.state.core.html_overlay_slot(driven.page.id()).is_none());
}

#[gpui_kit::test]
fn a_text_page_channel_reaches_the_web_view_only_while_the_page_is_active(cx: &mut TestAppContext) {
    let driven = driven(&url_overlay());
    let channel: &dyn PageChannel = &*driven.page;
    assert!(channel.holds_slot());
    driven.view.answer_with(Ok(Some("42".to_owned())));
    let answer = channel.call("1 + 41".to_owned()).unwrap();
    assert_eq!(driven.view.take(), [Call::Renderer("1 + 41".to_owned())]);
    assert_eq!(answer.try_recv().unwrap(), Ok(Some("42".to_owned())));
    channel.show_document("<p>built</p>".to_owned());
    assert_eq!(driven.view.take(), [Call::Load(scheme::text_address(driven.page.id()))]);
    channel.fail("the build failed".to_owned());
    assert_eq!(driven.model.pending(), 1, "a failure is posted");
    cx.update(|cx| driven.page.close(cx));
    assert!(!channel.holds_slot());
    assert!(
        channel.call("1".to_owned()).is_none(),
        "the web view went with the page"
    );
    channel.show_document("<p>late</p>".to_owned());
    channel.fail("late".to_owned());
    assert_eq!(driven.model.pending(), 1, "a closed page reports nothing more");
    assert!(driven.view.take().is_empty());
}

// MARK: - navigation and hand-offs

#[gpui_kit::test]
fn a_history_step_needs_history_and_a_web_view(cx: &mut TestAppContext) {
    let driven = driven(&url_overlay());
    let page = &driven.page;
    for step in [HtmlNavigation::Back, HtmlNavigation::Forward] {
        assert_eq!(page.navigate(step), Err(errors::no_history(step)));
    }
    driven.report(
        Report::History {
            back: true,
            forward: true,
        },
        cx,
    );
    page.navigate(HtmlNavigation::Back).unwrap();
    page.navigate(HtmlNavigation::Forward).unwrap();
    assert_eq!(driven.view.take(), [Call::Back, Call::Forward]);
}

#[test]
fn the_browser_hand_off_opens_what_the_page_shows_or_says_why_not() {
    let driven = driven(&url_overlay());
    driven.page.navigate(HtmlNavigation::Browser).unwrap();
    assert_eq!(driven.engine.opened(), ["https://example.invalid/start"]);
    driven.engine.no_browser();
    assert_eq!(
        driven.page.navigate(HtmlNavigation::Browser),
        Err(errors::NO_BROWSER.to_owned())
    );

    let markdown = self::driven(&overlay(HtmlSource::Markdown {
        path: "notes.md".to_owned(),
    }));
    assert_eq!(
        markdown.page.navigate(HtmlNavigation::Browser),
        Err(errors::MARKDOWN_IN_OVERLAY.to_owned())
    );
    assert!(markdown.engine.opened().is_empty());
}

#[test]
fn a_replica_is_never_handed_to_this_machine() {
    let driven = driven(&overlay(HtmlSource::Replica {
        path: "/page.html".to_owned(),
        content: ReplicaContent::Html,
    }));
    for hand_off in [HtmlNavigation::Browser, HtmlNavigation::Finder] {
        assert_eq!(driven.page.navigate(hand_off), Err(errors::REMOTE_FILE.to_owned()));
    }
    assert!(driven.engine.opened().is_empty());
}

#[test]
fn show_in_files_is_a_file_pages_and_reveals_its_file() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("page.html");
    std::fs::write(&path, "<p>page</p>").unwrap();
    let source = overlay(HtmlSource::File {
        path: path.to_string_lossy().into_owned(),
        grant_root: None,
    });
    let mut driven = driven(&source);
    driven.page.navigate(HtmlNavigation::Finder).unwrap();
    driven.model.drain();
    assert_eq!(driven.model.state.take_frontend_work().reveals, [path]);

    let url = self::driven(&url_overlay());
    assert_eq!(
        url.page.navigate(HtmlNavigation::Finder),
        Err(errors::FINDER_REQUIRES_FILE.to_owned())
    );
}

#[test]
fn the_page_zoom_goes_to_the_web_view_when_there_is_one() {
    let driven = driven(&url_overlay());
    driven.page.set_zoom(1.5);
    assert_eq!(driven.view.take(), [Call::Zoom(1.5)]);
    fake_native::install();
    let none = Page::new(
        &url_overlay(),
        None,
        colors(),
        Storage::Ephemeral,
        &crate::state::detached_poster(),
    );
    none.set_zoom(1.5);
    none.reply(1, Ok(serde_json::Value::Null));
}

// MARK: - prompts

/// A page shown in a test window, its prompts opening over it.
fn prompting<'a>(source: &HtmlOverlay, cx: &'a mut TestAppContext) -> (Driven, &'a mut VisualTestContext) {
    let cx = crate::modal::tests::host(cx);
    let driven = driven(source);
    driven.page.drawn_in(cx.update(|window, _| window.window_handle()));
    cx.update(|_, cx| driven.show(cx));
    (driven, cx)
}

fn asks(driven: &Driven, cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        driven.page.report(
            Report::NewWindow {
                uri: OUTSIDE.to_owned(),
                user: true,
            },
            cx,
        );
    });
    cx.run_until_parked();
}

fn prompt_up(cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| crate::modal::alert_open(window.window_handle(), cx))
}

fn answer(cx: &mut VisualTestContext, label: &str) {
    cx.update(|window, cx| assert!(crate::modal::answer_front_alert(window, label, cx)));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_hand_off_asks_first_and_opens_only_what_was_approved(cx: &mut TestAppContext) {
    let (driven, cx) = prompting(&url_overlay(), cx);
    asks(&driven, cx);
    assert!(prompt_up(cx));
    let spec = cx.update(|window, cx| {
        crate::modal::front_alert(window.window_handle(), cx)
            .unwrap()
            .read(cx)
            .spec()
            .clone()
    });
    assert_eq!(spec.heading, "Open https://outside.invalid in your browser?");
    assert_eq!(spec.body, OUTSIDE);
    answer(cx, "Open");
    assert!(!prompt_up(cx));
    assert_eq!(driven.engine.opened(), [OUTSIDE]);
}

#[gpui_kit::test]
fn a_declined_hand_off_silences_the_page_until_real_input_reaches_it(cx: &mut TestAppContext) {
    let (driven, cx) = prompting(&url_overlay(), cx);
    asks(&driven, cx);
    answer(cx, "Cancel");
    assert!(driven.engine.opened().is_empty());
    asks(&driven, cx);
    assert!(!prompt_up(cx), "a declined prompt silences the page");
    driven.page.user_input(false);
    asks(&driven, cx);
    assert!(prompt_up(cx), "real input lifts it");
    answer(cx, "Cancel");
}

#[gpui_kit::test]
fn a_hand_off_with_no_browser_changes_nothing_else(cx: &mut TestAppContext) {
    let (driven, cx) = prompting(&url_overlay(), cx);
    driven.engine.no_browser();
    asks(&driven, cx);
    answer(cx, "Open");
    assert!(!prompt_up(cx));
    assert_eq!(driven.engine.opened(), [OUTSIDE]);
}

#[gpui_kit::test]
fn a_second_request_while_a_prompt_is_up_is_dropped_and_a_page_off_screen_does_not_ask(cx: &mut TestAppContext) {
    let (driven, cx) = prompting(&url_overlay(), cx);
    asks(&driven, cx);
    let first = cx.update(|window, cx| crate::modal::front_alert(window.window_handle(), cx));
    asks(&driven, cx);
    let second = cx.update(|window, cx| crate::modal::front_alert(window.window_handle(), cx));
    assert_eq!(first, second);
    cx.update(|_, cx| driven.page.apply(&driven.overlay, false, cx));
    cx.run_until_parked();
    assert!(!prompt_up(cx), "a page going off screen withdraws its prompt");
    asks(&driven, cx);
    assert!(!prompt_up(cx), "a page out of sight asks nothing");
}

#[gpui_kit::test]
fn a_new_window_nobody_clicked_is_not_offered_and_a_page_never_drawn_cannot_ask(cx: &mut TestAppContext) {
    let (driven, cx) = prompting(&url_overlay(), cx);
    cx.update(|_, cx| {
        driven.page.report(
            Report::NewWindow {
                uri: OUTSIDE.to_owned(),
                user: false,
            },
            cx,
        );
    });
    cx.run_until_parked();
    assert!(!prompt_up(cx), "a script's window.open is not offered");

    let never_drawn = self::driven(&url_overlay());
    cx.update(|_, cx| never_drawn.show(cx));
    asks(&never_drawn, cx);
    assert!(!prompt_up(cx));
}

#[gpui_kit::test]
fn a_refused_click_on_a_shown_page_is_offered_and_the_shown_document_stays(cx: &mut TestAppContext) {
    let (mut driven, cx) = prompting(&url_overlay(), cx);
    cx.update(|_, cx| {
        driven.page.report(Report::Committed, cx);
        driven.page.report(
            Report::Completed {
                id: 1,
                success: true,
                status: 0,
                http: 200,
            },
            cx,
        );
    });
    assert_eq!(driven.load().0, HtmlLoadState::Loaded);
    cx.update(|_, cx| {
        driven.page.report(
            Report::Navigation {
                navigation: navigation(OUTSIDE, true, true, false, 8),
                allowed: false,
            },
            cx,
        );
    });
    cx.run_until_parked();
    assert_eq!(driven.load().0, HtmlLoadState::Loaded);
    assert!(prompt_up(cx));
    answer(cx, "Cancel");
}

#[gpui_kit::test]
fn closing_a_page_withdraws_its_prompt_and_releases_its_element(cx: &mut TestAppContext) {
    let driven = driven(&url_overlay());
    let (element, cx) = mounted(&driven, cx);
    driven.page.drawn_in(cx.update(|window, _| window.window_handle()));
    cx.update(|_, cx| driven.show(cx));
    asks(&driven, cx);
    assert!(prompt_up(cx));
    cx.update(gpui_kit::Window::blur);
    cx.run_until_parked();
    assert!(!driven.page.laid_out.get().focused);
    cx.update(|_, cx| driven.page.close(cx));
    cx.run_until_parked();
    assert!(!prompt_up(cx), "the prompt goes with the page");
    focus_element(&element, cx);
    assert!(
        !driven.page.laid_out.get().focused,
        "the released element ignores focus even while the page is held"
    );
    assert!(!driven.page.active() && driven.page.native().is_none());
    assert!(!driven.view.dropped(), "the pump buries the web view");
}

#[cfg(windows)]
#[gpui_kit::test]
fn a_key_the_app_takes_from_a_page_goes_through_the_keymap(cx: &mut TestAppContext) {
    use gpui_kit::Keystroke;

    let driven = driven(&url_overlay());
    let keystroke = Keystroke::parse("ctrl-a").unwrap();
    // before a window: nowhere to dispatch it
    cx.update(|cx| driven.page.report(Report::Key(keystroke.clone(), 0, false), cx));
    assert!(!crate::keymap_dispatch::sequence_is_live());
    let (keymap, _) = ngterm_core::keymap::parse_keymap(
        "command \"g\" ctrl+a>ctrl+g true\n",
        &ngterm_core::keymap::linux_defaults(),
    );
    crate::keymap_dispatch::load_keymap(&keymap);
    let (_element, cx) = mounted(&driven, cx);
    cx.update(|window, cx| {
        crate::keymap_dispatch::init(cx);
        crate::keymap_dispatch::register_window(
            window.window_handle(),
            crate::keymap_dispatch::KeyOrigin::Window(Uuid::new_v4()),
        );
    });
    cx.cx
        .update(|cx| driven.page.report(Report::Key(keystroke, 0, false), cx));
    cx.run_until_parked();
    assert!(crate::keymap_dispatch::sequence_is_live(), "the leader is half typed");
}

/// A half-typed `ctrl+a>ctrl+g`, its tail's report asked about and queued.
#[cfg(windows)]
fn leader_tail_queued() -> gpui_kit::Keystroke {
    let (keymap, _) = ngterm_core::keymap::parse_keymap(
        "command \"g\" ctrl+a>ctrl+g true\n",
        &ngterm_core::keymap::linux_defaults(),
    );
    crate::keymap_dispatch::load_keymap(&keymap);
    assert_eq!(crate::keymap_dispatch::press_dispatched("ctrl-a"), Some(false));
    let tail = gpui_kit::Keystroke::parse("ctrl-g").unwrap();
    assert!(crate::keymap_dispatch::takes(&crate::keymap_dispatch::page_stroke(
        &tail, 0, false
    )));
    tail
}

/// Whether the page would keep `ctrl-x`, which ends a live sequence.
#[cfg(windows)]
fn page_keeps_ctrl_x() -> bool {
    let kept = gpui_kit::Keystroke::parse("ctrl-x").unwrap();
    !crate::keymap_dispatch::takes(&crate::keymap_dispatch::page_stroke(&kept, 0, false))
}

/// A report with no window to go to leaves the queue, so a press the page keeps next ends the leader at once.
#[cfg(windows)]
#[gpui_kit::test]
fn a_key_report_with_no_window_leaves_the_queue(cx: &mut TestAppContext) {
    let driven = driven(&url_overlay());
    let tail = leader_tail_queued();
    cx.update(|cx| driven.page.report(Report::Key(tail, 0, false), cx));
    assert!(page_keeps_ctrl_x());
    assert!(!crate::keymap_dispatch::sequence_is_live());
}

/// A mounted page can outlive its window: a report the closed window refuses must leave the queue too.
#[cfg(windows)]
#[gpui_kit::test]
fn a_key_report_with_a_closed_window_leaves_the_queue(cx: &mut TestAppContext) {
    let driven = driven(&url_overlay());
    let (_element, visual) = mounted(&driven, cx);
    let window = visual.update(|window, _| window.window_handle());
    visual.update(|window, _| window.remove_window());
    let tail = leader_tail_queued();
    cx.update(|cx| {
        assert!(window.update(cx, |_, _, _| ()).is_err(), "the handle is stale");
        driven.page.report(Report::Key(tail, 0, false), cx);
    });
    assert!(page_keeps_ctrl_x());
    assert!(!crate::keymap_dispatch::sequence_is_live());
}

/// The page's script's report of a key it stopped: its virtual key, Shift, autorepeat and the key's names.
#[cfg(windows)]
fn stopped_key(page: &Page, virtual_key: u32, shift: bool, repeat: bool, names: (&str, &str)) -> Report {
    let nonce = page.nonce.split_once(':').unwrap().0;
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let (key, code) = names;
    Report::Message(format!(
        r#"{{"n":"{nonce}","k":"key","vk":{virtual_key},"shift":{shift},"repeat":{repeat},"key":"{key}","code":"{code}","at":{at}}}"#
    ))
}

/// `driven`'s page in a window under the key monitor, `keymap` loaded and `pressed` dispatched from the page.
#[cfg(windows)]
fn under_monitor<'a>(
    driven: &Driven,
    cx: &'a mut TestAppContext,
    keymap: &str,
    pressed: &[&str],
) -> &'a mut VisualTestContext {
    let (keymap, _) = ngterm_core::keymap::parse_keymap(keymap, &ngterm_core::keymap::linux_defaults());
    crate::keymap_dispatch::load_keymap(&keymap);
    let (_element, cx) = mounted(driven, cx);
    cx.update(|window, cx| {
        crate::keymap_dispatch::init(cx);
        crate::keymap_dispatch::register_window(
            window.window_handle(),
            crate::keymap_dispatch::KeyOrigin::Window(Uuid::new_v4()),
        );
    });
    for key in pressed {
        assert!(crate::keymap_dispatch::press_dispatched(key).is_some(), "{key}");
    }
    cx
}

/// Upstream's monitor sees every press in a page: a leader's tail that types (a letter, Enter, Backspace) fires it, and
/// its press never reaches the page. `AcceleratorKeyPressed` reports none of these, so it is the page's script that
/// stops them.
#[cfg(windows)]
#[gpui_kit::test]
fn a_leaders_typing_tail_stopped_by_the_pages_script_fires_it(cx: &mut TestAppContext) {
    for (shortcut, virtual_key, key, code) in [
        ("g", 0x47, "g", "KeyG"),
        ("return", 0x0D, "Enter", "Enter"),
        ("delete", 0x08, "Backspace", "Backspace"),
    ] {
        let driven = driven(&url_overlay());
        let visual = under_monitor(
            &driven,
            cx,
            &format!("command \"t\" ctrl+a>{shortcut} true\n"),
            &["ctrl-a"],
        );
        crate::keymap_dispatch::take_fired();
        driven.view.take();
        let report = stopped_key(&driven.page, virtual_key, false, false, (key, code));
        visual.cx.update(|cx| driven.page.report(report, cx));
        visual.run_until_parked();
        assert_eq!(crate::keymap_dispatch::take_fired().len(), 1, "{shortcut} fires");
        assert!(
            !driven
                .view
                .take()
                .iter()
                .any(|call| matches!(call, Call::Redelivered(_))),
            "{shortcut} stays off the page"
        );
        assert!(
            !crate::keymap_dispatch::sequence_is_live(),
            "{shortcut} ends the leader"
        );
    }
}

/// What the page's script was last told of a live sequence's keys, of the calls since the last take.
#[cfg(windows)]
fn told_keys(view: &Probe) -> Vec<Vec<u32>> {
    view.take()
        .into_iter()
        .filter_map(|call| match call {
            Call::Post(json) => {
                let message: serde_json::Value = serde_json::from_str(&json).unwrap();
                (message["ngterm"] == "keys").then(|| {
                    message["keys"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|key| u32::try_from(key.as_u64().unwrap()).unwrap())
                        .collect()
                })
            }
            _ => None,
        })
        .collect()
}

/// A press of `key` the page's accelerator reports, as `wire_keys` hands it over.
#[cfg(windows)]
fn accelerator_key(driven: &Driven, visual: &mut VisualTestContext, key: &str) {
    let keystroke = gpui_kit::Keystroke::parse(key).unwrap();
    assert!(crate::keymap_dispatch::takes(&crate::keymap_dispatch::page_stroke(
        &keystroke, 0, false
    )));
    visual
        .cx
        .update(|cx| driven.page.report(Report::Key(keystroke, 0, false), cx));
    visual.run_until_parked();
}

/// The host tells the script when a leader arms and when it ends, and which keys its next chord can be.
#[cfg(windows)]
#[gpui_kit::test]
fn the_script_hears_of_a_live_leader_and_of_its_end(cx: &mut TestAppContext) {
    let driven = driven(&url_overlay());
    let visual = under_monitor(
        &driven,
        cx,
        "command \"g\" ctrl+a>g true\ncommand \"h\" ctrl+a>shift+h true\n",
        &[],
    );
    driven.view.take();
    accelerator_key(&driven, visual, "ctrl-a");
    assert_eq!(
        told_keys(&driven.view),
        vec![vec![0x47, 0x48 | 0x100]],
        "the leader's plain tails"
    );
    let report = stopped_key(&driven.page, 0x47, false, false, ("g", "KeyG"));
    visual.cx.update(|cx| driven.page.report(report, cx));
    visual.run_until_parked();
    assert_eq!(told_keys(&driven.view), vec![Vec::<u32>::new()], "the leader ended");
    crate::keymap_dispatch::take_fired();
    // and with no sequence live, a key and a report say nothing to the script
    accelerator_key(&driven, visual, "ctrl-shift-t");
    driven.report(
        Report::Message(format!(
            r#"{{"n":"{}","k":"input","click":false}}"#,
            driven.page.nonce.split_once(':').unwrap().0
        )),
        &mut visual.cx,
    );
    assert_eq!(told_keys(&driven.view), Vec::<Vec<u32>>::new(), "nothing is live");
}

/// A leader armed outside the page tells the script its tails as soon as the page gains focus, even before mount.
#[cfg(windows)]
#[gpui_kit::test]
fn a_leader_begun_outside_the_page_goes_on_when_it_takes_focus(cx: &mut TestAppContext) {
    let driven = driven(&url_overlay());
    let (keymap, _) =
        ngterm_core::keymap::parse_keymap("command \"g\" ctrl+a>g true\n", &ngterm_core::keymap::linux_defaults());
    crate::keymap_dispatch::load_keymap(&keymap);
    assert_eq!(crate::keymap_dispatch::press_dispatched("ctrl-a"), Some(false));
    driven.report(Report::Focus(false), cx);
    assert!(told_keys(&driven.view).is_empty());
    driven.report(Report::Focus(true), cx);
    assert_eq!(told_keys(&driven.view), vec![vec![0x47]]);
}

/// Ownership of a script-stopped keypad Enter follows the physical key when focus moves to a terminal.
#[cfg(windows)]
#[gpui_kit::test]
fn a_stopped_key_keeps_its_physical_identity(cx: &mut TestAppContext) {
    use crate::terminal::input::{self, KeyStroke, NativeKey, Source};

    for (code, expected, other) in [
        ("Enter", "Enter", "NumpadEnter"),
        ("NumpadEnter", "NumpadEnter", "Enter"),
        ("", "Enter", "NumpadEnter"),
    ] {
        crate::keymap_dispatch::keyboard_left();
        let driven = driven(&url_overlay());
        let visual = under_monitor(&driven, cx, "command \"enter\" ctrl+a>return true\n", &["ctrl-a"]);
        let report = stopped_key(&driven.page, 0x0D, false, false, ("Enter", code));
        visual.cx.update(|cx| driven.page.report(report, cx));
        visual.run_until_parked();
        assert_eq!(crate::keymap_dispatch::take_fired().len(), 1);
        let keystroke = gpui_kit::Keystroke::parse("enter").unwrap();
        let held = |code| {
            KeyStroke::new(
                &keystroke,
                &Source::Native(NativeKey {
                    keycode: input::xkb_keycode(code).unwrap(),
                    repeat: true,
                    ..NativeKey::default()
                }),
                false,
            )
        };
        assert!(crate::keymap_dispatch::takes(&held(expected)), "{code} is still owned");
        crate::keymap_dispatch::page_report_dropped();
        assert!(
            !crate::keymap_dispatch::takes(&held(other)),
            "the other Enter is not owned"
        );
    }
}

/// A leader whose tails are all chords has nothing for the script to stop: `AcceleratorKeyPressed` reports those.
#[cfg(windows)]
#[gpui_kit::test]
fn a_leader_of_chords_only_leaves_the_script_doing_nothing(cx: &mut TestAppContext) {
    let driven = driven(&url_overlay());
    let visual = under_monitor(&driven, cx, "command \"g\" ctrl+a>ctrl+g true\n", &[]);
    driven.view.take();
    accelerator_key(&driven, visual, "ctrl-a");
    assert!(crate::keymap_dispatch::sequence_is_live());
    assert_eq!(told_keys(&driven.view), Vec::<Vec<u32>>::new());
}

/// A key the script stopped on stale word (the sequence ended first) goes back to the page, so no key is ever lost; a
/// key the live sequence does not take is the same, and it ends the sequence, as any key the page keeps does.
#[cfg(windows)]
#[gpui_kit::test]
fn a_stopped_key_the_keymap_does_not_take_goes_back_to_the_page(cx: &mut TestAppContext) {
    let driven = driven(&url_overlay());
    let visual = under_monitor(&driven, cx, "command \"g\" ctrl+a>g true\n", &["ctrl-a"]);
    driven.view.take();
    let redelivered = |view: &Probe| -> usize {
        view.take()
            .iter()
            .filter(|call| matches!(call, Call::Redelivered(events) if events.len() == 2))
            .count()
    };
    let report = stopped_key(&driven.page, 0x58, false, false, ("x", "KeyX"));
    visual.cx.update(|cx| driven.page.report(report, cx));
    assert_eq!(redelivered(&driven.view), 1, "x matches nothing");
    assert!(!crate::keymap_dispatch::sequence_is_live(), "and ends the leader");
    let report = stopped_key(&driven.page, 0x47, false, false, ("g", "KeyG"));
    visual.cx.update(|cx| driven.page.report(report, cx));
    assert_eq!(redelivered(&driven.view), 1, "the leader was already gone");
    assert!(crate::keymap_dispatch::take_fired().is_empty(), "nothing fired");
}

/// With no window to dispatch into, a stopped key still goes back to the page.
#[cfg(windows)]
#[gpui_kit::test]
fn a_stopped_key_with_no_window_goes_back_to_the_page(cx: &mut TestAppContext) {
    let driven = driven(&url_overlay());
    let report = stopped_key(&driven.page, 0x47, false, false, ("g", "KeyG"));
    cx.update(|cx| driven.page.report(report, cx));
    assert!(matches!(driven.view.take().as_slice(), [Call::Redelivered(events)] if events[0]["text"] == "g"));
}

/// `driven`'s script stopped key `g`, pressed or autorepeating, and the report has acted.
#[cfg(windows)]
fn stopped_g(driven: &Driven, repeat: bool, visual: &mut VisualTestContext) {
    let report = stopped_key(&driven.page, 0x47, false, repeat, ("g", "KeyG"));
    visual.cx.update(|cx| driven.page.report(report, cx));
    visual.run_until_parked();
}

/// A leader's held tail fires once, and its autorepeats stay the keymap's without firing or going back to the page.
#[cfg(windows)]
#[gpui_kit::test]
fn a_stopped_tails_autorepeat_fires_nothing_and_stays_the_keymaps(cx: &mut TestAppContext) {
    let driven = driven(&url_overlay());
    let visual = under_monitor(&driven, cx, "command \"g\" ctrl+a>g true\n", &["ctrl-a"]);
    crate::keymap_dispatch::take_fired();
    driven.view.take();
    stopped_g(&driven, false, visual);
    assert_eq!(crate::keymap_dispatch::take_fired().len(), 1);
    stopped_g(&driven, true, visual);
    assert!(
        crate::keymap_dispatch::take_fired().is_empty(),
        "the autorepeat fires nothing"
    );
    assert!(
        !driven
            .view
            .take()
            .iter()
            .any(|call| matches!(call, Call::Redelivered(_))),
        "and is not given back"
    );
}

/// A `--repeat` tail's autorepeat fires again, the window staying open, and the script keeps stopping it.
#[cfg(windows)]
#[gpui_kit::test]
fn a_stopped_repeat_tails_autorepeat_fires_again(cx: &mut TestAppContext) {
    let driven = driven(&url_overlay());
    let visual = under_monitor(&driven, cx, "command \"g\" ctrl+a>g --repeat true\n", &["ctrl-a"]);
    crate::keymap_dispatch::take_fired();
    driven.view.take();
    stopped_g(&driven, false, visual);
    assert_eq!(crate::keymap_dispatch::take_fired().len(), 1);
    stopped_g(&driven, true, visual);
    assert_eq!(
        crate::keymap_dispatch::take_fired().len(),
        1,
        "a live tail's autorepeat fires again"
    );
    assert!(crate::keymap_dispatch::sequence_is_live(), "the window stays open");
    assert_eq!(
        told_keys(&driven.view).last().unwrap(),
        &vec![0x47],
        "the script is told the tail"
    );
}

#[test]
fn every_navigation_error_names_its_reason() {
    for (status, reason) in [
        (1, "the server's certificate names another host"),
        (2, "the server's certificate has expired"),
        (3, "the client's certificate has errors"),
        (4, "the server's certificate has been revoked"),
        (5, "the server's certificate is invalid"),
        (6, "the server could not be reached"),
        (7, "the request timed out"),
        (8, "the server returned an invalid response"),
        (9, "the connection was aborted"),
        (10, "the connection was reset"),
        (11, "the internet connection was lost"),
        (15, "the redirect failed"),
        (16, "an unexpected error occurred"),
        (17, "the server requires authentication"),
        (18, "the proxy requires authentication"),
    ] {
        assert_eq!(error_text(status), reason);
    }
}
