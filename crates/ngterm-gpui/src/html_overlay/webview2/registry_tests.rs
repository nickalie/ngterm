//! The registry over the web view's seam (`fake_native`): the pump, the stores, the requests waiting on an answer, the
//! pages' keyboard and the saved store's clear.

use std::path::Path;
use std::sync::Mutex;

use gpui_kit::{Context, IntoElement, ParentElement as _, Render, Styled as _, TestAppContext, VisualTestContext, div};
use ngterm_core::html::image_page::ImageData;
use ngterm_core::html::{HtmlLoadState, HtmlSource, ReplicaContent, SearchStep};
use tempfile::TempDir;

use super::super::fake_native::{self, Call, FakeEngine, FakeView, Model, Probe, overlay, settle_until, url_overlay};
use super::*;

/// A window's content holding one page's element, as the deck does.
struct Holder {
    element: Option<Entity<PageView>>,
}

impl Render for Holder {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().children(self.element.clone())
    }
}

struct Blank;

impl Render for Blank {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

fn colors() -> PanelColors {
    PanelColors::resolve(&TerminalColors::default(), None)
}

/// The registry on a fresh engine and a running pump.
fn pumped(cx: &mut TestAppContext) -> Rc<FakeEngine> {
    cx.update(gpui_kit::init);
    let engine = fake_native::install();
    cx.update(init);
    engine
}

/// Takes the pump's channel for the test's own: a worker's send from a real thread must not wake GPUI's test scheduler.
fn jobs_for_the_test() -> async_channel::Receiver<Job> {
    let (jobs, queue) = async_channel::unbounded();
    with(|registry| registry.jobs = Some(jobs));
    queue
}

/// The document job a worker sends next, with `waiting` run meanwhile.
#[track_caller]
fn worker_job(queue: &async_channel::Receiver<Job>, mut waiting: impl FnMut()) -> Job {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        waiting();
        if let Ok(job @ Job::DocumentLoaded(..)) = queue.try_recv() {
            return job;
        }
        assert!(std::time::Instant::now() < deadline, "the worker did not finish");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

/// `WebView2` leaves a profile folder for the pump to remove; `WebKit`'s stores are kept by id and leave none.
fn folder_is_gone(folder: &Path) -> bool {
    cfg!(not(windows)) || !folder.exists()
}

fn state_dir_at(dir: &Path) {
    with(|registry| registry.state_dir = Some(dir.to_owned()));
}

/// `overlay`'s page, registered, posting into `model`.
fn page_of(overlay: &HtmlOverlay, storage: Storage, model: &Model) -> Rc<Page> {
    let page = Page::new(overlay, None, colors(), storage, &model.poster());
    add_page(&page);
    page
}

/// A page that has its web view.
fn realized(overlay: &HtmlOverlay, model: &Model) -> (Rc<Page>, Probe) {
    let page = page_of(overlay, Storage::Ephemeral, model);
    let (view, probe) = FakeView::made();
    page.realized(view);
    probe.take();
    (page, probe)
}

fn window_of(cx: &mut TestAppContext) -> &mut VisualTestContext {
    let (_, cx) = cx.add_window_view(|_, _| Blank);
    cx
}

/// Where a test's responder keeps the response it was given.
type Served = Arc<Mutex<Option<wry::http::Response<Vec<u8>>>>>;

fn responder() -> (ReadReply, Served) {
    let slot = Arc::new(Mutex::new(None));
    let filled = Arc::clone(&slot);
    (Box::new(move |response| *filled.lock().unwrap() = Some(response)), slot)
}

// MARK: - what the core asks

#[test]
fn a_window_scale_is_recorded_only_when_it_is_a_real_scale() {
    fake_native::install();
    let window = Uuid::new_v4();
    assert_eq!(PageHost.display_scale(window).to_bits(), 1.0_f64.to_bits());
    set_window_scale(window, 2.5);
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        set_window_scale(window, bad);
        assert_eq!(PageHost.display_scale(window).to_bits(), 2.5_f64.to_bits());
    }
    assert_eq!(PageHost.page_zoom().to_bits(), 1.0_f64.to_bits());
    with(|registry| registry.zoom = Some(1.25));
    assert_eq!(PageHost.page_zoom().to_bits(), 1.25_f64.to_bits());
}

#[test]
fn the_document_checks_read_their_files_and_say_why_not() {
    fake_native::install();
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("gone");
    let gone = path.to_str().unwrap();
    assert_eq!(
        PageHost.check_markdown(gone),
        ngterm_app::documents::check_markdown(&path)
    );
    assert!(PageHost.check_markdown(gone).is_err());
    assert_eq!(PageHost.check_text(gone), ngterm_app::documents::check_text(&path));
    assert_eq!(PageHost.check_log(gone, 5), ngterm_app::documents::check_log(&path, 5));
    let image: Result<ImageData, String> = PageHost.probe_image(gone);
    assert_eq!(image.err(), ngterm_app::documents::probe_image(&path).err());
}

#[test]
fn searches_and_navigations_on_an_unknown_page_are_refused() {
    fake_native::install();
    let id = Uuid::new_v4();
    let step = SearchStep {
        forward: true,
        query: "needle".to_owned(),
        revision: 1,
    };
    assert!(PageHost.step_text_search(id, step).is_none());
    assert!(PageHost.step_text_changes(id, true).is_none());
    assert_eq!(
        PageHost.navigate(id, HtmlNavigation::Back),
        Err(errors::NOT_REALIZED.to_owned())
    );
    assert_eq!(PageHost.web_views_unavailable(), None);
}

#[test]
fn navigation_needs_a_web_view_and_is_then_the_pages_own() {
    fake_native::install();
    let source = url_overlay();
    let model = Model::new(&source);
    let page = page_of(&source, Storage::Ephemeral, &model);
    assert_eq!(
        navigate(page.id(), HtmlNavigation::Back),
        Err(errors::NOT_REALIZED.to_owned())
    );
    let (view, _probe) = FakeView::made();
    page.realized(view);
    assert_eq!(
        navigate(page.id(), HtmlNavigation::Back),
        Err(errors::no_history(HtmlNavigation::Back))
    );
}

#[test]
fn the_saved_store_is_never_replaced_by_a_temporary_one() {
    fake_native::install();
    let mut source = url_overlay();
    assert_eq!(storage(&source), Storage::Ephemeral);
    source.flags.persistent = true;
    assert_eq!(
        storage(&source),
        Storage::Unavailable(errors::PERSISTENT_UNAVAILABLE.to_owned())
    );
    let directory = TempDir::new().unwrap();
    state_dir_at(directory.path());
    assert_eq!(storage(&source), Storage::Saved);
    assert_eq!(PageHost.persistent_store_failure(), None);
    with(|registry| registry.clearing = true);
    assert_eq!(storage(&source), Storage::Unavailable(errors::CLEARING.to_owned()));
    source.source = HtmlSource::File {
        path: "/a.html".to_owned(),
        grant_root: None,
    };
    assert_eq!(storage(&source), Storage::Ephemeral);
}

#[test]
fn a_corrupt_saved_profile_reports_its_path_without_overwriting_it() {
    fake_native::install();
    let directory = TempDir::new().unwrap();
    let path = directory.path().join(profile::FILENAME);
    std::fs::write(&path, "corrupt").unwrap();
    state_dir_at(directory.path());
    assert_eq!(
        persistent_store_failure(),
        Some(format!("browser profile is not a UUID: {}", path.display()))
    );
    assert_eq!(std::fs::read_to_string(path).unwrap(), "corrupt");
}

#[test]
fn install_makes_the_registry_the_cores_page_host_and_records_the_runtime() {
    fake_native::install();
    let directory = TempDir::new().unwrap();
    let source = url_overlay();
    let mut model = Model::new(&source);
    let poster = model.poster();
    install(&mut model.state.core, directory.path().to_owned(), poster);
    assert_eq!(
        with(|registry| registry.state_dir.clone()).as_deref(),
        Some(directory.path())
    );
    assert!(with(|registry| registry.poster.is_some()));
    assert_eq!(PageHost.web_views_unavailable(), runtime::installed_version().err());
}

// MARK: - the pump

#[gpui_kit::test]
fn a_mounted_page_is_made_by_the_pump_placed_and_loaded(cx: &mut TestAppContext) {
    let engine = pumped(cx);
    let directory = TempDir::new().unwrap();
    state_dir_at(directory.path());
    engine.windows_have(7);
    let source = url_overlay();
    let cx = window_of(cx);
    let element = cx
        .update(|window, cx| mount(&source, None, window, cx))
        .expect("the window has a native handle");
    settle_until(cx, || engine.made() == 1);

    let spec = engine.spec(0);
    assert_eq!(spec.parent, 7);
    assert!(spec.store.private);
    assert_eq!(spec.store.profile, store::page_profile(source.id));
    assert_eq!(spec.store.user_data, store::user_data_folder(directory.path()));
    assert_eq!(spec.bounds, None, "never drawn yet");
    assert!(!spec.visible);
    assert!(spec.grant.is_none() && spec.text.is_none() && spec.replica.is_none());
    assert_eq!(
        engine.probe(0).take(),
        [Call::Load("https://example.invalid/start".to_owned())]
    );

    let again = cx.update(|window, cx| mount(&source, None, window, cx)).unwrap();
    assert_eq!(again, element, "one element per window");
    settle_until(cx, || true);
    assert_eq!(engine.made(), 1, "a page makes one web view");
}

#[gpui_kit::test]
fn a_session_moved_to_another_window_gets_a_new_element_and_keeps_its_web_view(cx: &mut TestAppContext) {
    let engine = pumped(cx);
    let directory = TempDir::new().unwrap();
    state_dir_at(directory.path());
    engine.windows_have(7);
    let source = url_overlay();
    let first = window_of(cx)
        .update(|window, cx| mount(&source, None, window, cx))
        .unwrap();
    settle_until(cx, || engine.made() == 1);
    let second = {
        let cx = window_of(cx);
        cx.update(|window, cx| mount(&source, None, window, cx)).unwrap()
    };
    assert_ne!(first, second);
    settle_until(cx, || true);
    assert_eq!(engine.made(), 1);
}

#[gpui_kit::test]
fn a_window_with_no_native_handle_makes_the_page_but_no_element(cx: &mut TestAppContext) {
    pumped(cx);
    let source = url_overlay();
    let cx = window_of(cx);
    assert!(cx.update(|window, cx| mount(&source, None, window, cx)).is_none());
    assert!(
        page(source.id).is_some(),
        "the page exists for the window's next render"
    );
}

#[gpui_kit::test]
fn a_web_view_that_cannot_be_made_fails_the_page(cx: &mut TestAppContext) {
    let engine = pumped(cx);
    let directory = TempDir::new().unwrap();
    state_dir_at(directory.path());
    engine.windows_have(7);
    engine.fail_next("WebView2 said no");
    let source = url_overlay();
    let mut model = Model::new(&source);
    let poster = model.poster();
    with(|registry| registry.poster = Some(poster));
    let cx = window_of(cx);
    cx.update(|window, cx| mount(&source, None, window, cx)).unwrap();
    settle_until(cx, || model.pending() > 0);
    model.drain();
    assert_eq!(model.overlay().load_state, HtmlLoadState::Failed);
    assert_eq!(model.overlay().load_error.as_deref(), Some("WebView2 said no"));
    assert!(!page(source.id).unwrap().wants_native());
}

/// A page's store, state directory, window, whether a pump runs, and why its web view cannot be built.
type Unbuildable<'a> = (Storage, Option<&'a Path>, Option<isize>, bool, &'a str);

#[gpui_kit::test]
fn a_spec_that_cannot_be_built_fails_the_page_with_the_reason(cx: &mut TestAppContext) {
    let engine = pumped(cx);
    let directory = TempDir::new().unwrap();
    let state = Some(directory.path());
    let source = url_overlay();
    let mut model = Model::new(&source);

    // no state directory
    let page = page_of(&source, Storage::Ephemeral, &model);
    build(page.id());
    model.drain();
    assert_eq!(model.overlay().load_error.as_deref(), Some(NO_STATE));

    let unavailable = page_of(&source, Storage::Unavailable("no store".to_owned()), &model);
    assert_eq!(spec(&unavailable).err().as_deref(), Some("no store"));

    let cases: [Unbuildable<'_>; 3] = [
        (Storage::Saved, None, None, true, errors::PERSISTENT_UNAVAILABLE),
        (Storage::Ephemeral, state, None, true, "the page has no window"),
        (Storage::Ephemeral, state, Some(9), false, "the pages have no pump"),
    ];
    for (storage, state, window, pump, reason) in cases {
        let source = url_overlay();
        let mut model = Model::new(&source);
        let page = page_of(&source, storage, &model);
        with(|registry| registry.state_dir = state.map(Path::to_owned));
        if let Some(window) = window {
            page.place_in(window);
        }
        let jobs = with(|registry| {
            if pump {
                registry.jobs.clone()
            } else {
                registry.jobs.take()
            }
        });
        build(page.id());
        with(|registry| registry.jobs = jobs);
        model.drain();
        assert_eq!(model.overlay().load_error.as_deref(), Some(reason));
    }
    build(Uuid::new_v4());
    assert_eq!(engine.made(), 0);
}

/// A page and what its web view must have been built with.
type SourceCheck = (HtmlOverlay, fn(&Spec));

#[gpui_kit::test]
fn each_source_gets_the_web_view_its_page_needs(cx: &mut TestAppContext) {
    let engine = pumped(cx);
    let _workers = jobs_for_the_test();
    let directory = TempDir::new().unwrap();
    state_dir_at(directory.path());
    let grant = directory.path().to_string_lossy().into_owned();
    let mut scripted = url_overlay();
    scripted.flags.javascript = true;
    let cases: [SourceCheck; 8] = [
        (scripted, |spec: &Spec| assert!(spec.javascript && spec.theme.is_some())),
        (
            overlay(HtmlSource::Image {
                path: "a.png".to_owned(),
            }),
            |spec: &Spec| assert!(spec.theme.is_none(), "an image wears no theme"),
        ),
        (
            overlay(HtmlSource::File {
                path: directory.path().join("a.html").to_string_lossy().into_owned(),
                grant_root: Some(grant),
            }),
            |spec: &Spec| assert!(spec.grant.is_some() && spec.text.is_none()),
        ),
        (
            overlay(HtmlSource::File {
                path: "b.html".to_owned(),
                grant_root: None,
            }),
            |spec: &Spec| assert!(spec.grant.is_none() && spec.text.is_some()),
        ),
        (
            overlay(HtmlSource::Text {
                path: "c.txt".to_owned(),
                line: None,
            }),
            |spec: &Spec| assert!(spec.text_channel && spec.text.is_some() && spec.trusted_document_script.is_some()),
        ),
        (
            overlay(HtmlSource::Document {
                path: "d.html".to_owned(),
            }),
            |spec: &Spec| assert!(spec.block_network),
        ),
        (
            overlay(HtmlSource::Replica {
                path: "/e.html".to_owned(),
                content: ReplicaContent::Html,
            }),
            |spec: &Spec| assert!(spec.replica.is_some(), "an HTML replica reads through its protocol"),
        ),
        (
            overlay(HtmlSource::Replica {
                path: "/f.md".to_owned(),
                content: ReplicaContent::Markdown,
            }),
            |spec: &Spec| {
                assert!(
                    spec.replica.is_none(),
                    "a replica built from fetched bytes loads nothing through it"
                );
            },
        ),
    ];
    for (index, (source, check)) in cases.iter().enumerate() {
        let model = Model::new(source);
        let page = page_of(source, Storage::Ephemeral, &model);
        page.place_in(3);
        build(page.id());
        assert_eq!(engine.made(), index + 1);
        check(&engine.spec(index));
    }
}

#[gpui_kit::test]
fn a_saved_page_uses_the_saved_profile_kept_in_the_state_directory(cx: &mut TestAppContext) {
    let engine = pumped(cx);
    let directory = TempDir::new().unwrap();
    state_dir_at(directory.path());
    assert_eq!(persistent_store_failure(), None);
    let source = url_overlay();
    let model = Model::new(&source);
    let page = page_of(&source, Storage::Saved, &model);
    page.place_in(3);
    build(page.id());
    let spec = engine.spec(0);
    assert!(!spec.store.private);
    let id = BrowserProfile::new(directory.path())
        .existing_identifier()
        .unwrap()
        .unwrap();
    assert_eq!(spec.store.profile, store::saved_profile(id));
}

#[gpui_kit::test]
fn a_web_views_reports_and_policy_reach_its_page_through_the_pump(cx: &mut TestAppContext) {
    let engine = pumped(cx);
    let directory = TempDir::new().unwrap();
    state_dir_at(directory.path());
    let source = url_overlay();
    let mut model = Model::new(&source);
    let page = page_of(&source, Storage::Ephemeral, &model);
    page.place_in(3);
    build(page.id());
    let spec = engine.spec(0);
    let allowed = native::Navigation {
        uri: "https://example.invalid/other".to_owned(),
        main_frame: true,
        user: false,
        redirect: false,
        id: 1,
    };
    assert!((spec.decide)(&allowed));
    (spec.post)(Report::Title("From the web view".to_owned()));
    (spec.post)(Report::Source("https://example.invalid/shown".to_owned()));
    settle_until(cx, || model.pending() > 0);
    model.drain();
    let shown = model.overlay().current.clone().unwrap();
    assert_eq!(shown.title.as_deref(), Some("From the web view"));
    drop(page);
    with(|registry| registry.pages.clear());
    assert!(!(spec.decide)(&allowed), "a page gone decides nothing");
}

#[gpui_kit::test]
fn the_pump_moves_the_native_window_and_buries_released_web_views(cx: &mut TestAppContext) {
    pumped(cx);
    let directory = TempDir::new().unwrap();
    let folder = directory.path().join("leftover");
    std::fs::create_dir_all(&folder).unwrap();
    let source = url_overlay();
    let model = Model::new(&source);
    let (page, probe) = realized(&source, &model);
    cx.update(|cx| page.apply(&source, true, cx));
    page.place(native::Bounds {
        x: 1.,
        y: 2.,
        width: 3.,
        height: 4.,
    });
    sync_later(page.id());
    settle_until(cx, || !probe.take().is_empty());

    bury(page.native().unwrap(), Some(folder.clone()));
    assert!(!probe.dropped(), "the page still holds it");
    cx.update(|cx| page.close(cx));
    assert!(!page.active() && page.native().is_none());
    settle_until(cx, || probe.dropped() && folder_is_gone(&folder));
}

#[gpui_kit::test]
fn a_web_view_made_for_a_closed_page_is_buried_with_its_folder(cx: &mut TestAppContext) {
    pumped(cx);
    let directory = TempDir::new().unwrap();
    state_dir_at(directory.path());
    let source = url_overlay();
    let model = Model::new(&source);
    let page = page_of(&source, Storage::Ephemeral, &model);
    let folder = page_profile_folder(page.id()).unwrap();
    assert_eq!(page.private_folder().as_deref(), Some(folder.as_path()));
    std::fs::create_dir_all(&folder).unwrap();
    cx.update(|cx| page.close(cx));
    let (view, probe) = FakeView::made();
    page.realized(view);
    assert!(page.native().is_none());
    settle_until(cx, || probe.dropped() && folder_is_gone(&folder));
}

#[cfg(windows)]
#[test]
fn a_new_environment_sweeps_the_folders_closed_pages_left_only_once() {
    fake_native::install();
    let directory = TempDir::new().unwrap();
    state_dir_at(directory.path());
    let user_data = store::user_data_folder(directory.path());
    let left = store::profile_folder(&user_data, &store::page_profile(Uuid::new_v4()));
    std::fs::create_dir_all(&left).unwrap();
    let store = store_for("profile".to_owned(), true).unwrap();
    assert_eq!(
        (store.user_data, store.profile, store.private),
        (user_data.clone(), "profile".to_owned(), true)
    );
    assert!(!left.exists());
    std::fs::create_dir_all(&left).unwrap();
    store_for("profile".to_owned(), false).unwrap();
    assert!(left.exists(), "only the first store sweeps");
}

// MARK: - the model's value

#[gpui_kit::test]
fn a_commit_closes_the_pages_that_left_and_every_page_follows_the_zoom(cx: &mut TestAppContext) {
    fake_native::install();
    let source = url_overlay();
    let mut model = Model::new(&source);
    let directory = TempDir::new().unwrap();
    let poster = model.poster();
    install(&mut model.state.core, directory.path().to_owned(), poster);
    let (page, probe) = realized(&source, &model);
    let _ = model.state.core.commit();
    cx.update(|cx| settle(&mut model.state.core, cx));
    assert_eq!(probe.take(), [Call::Zoom(1.0)]);
    cx.update(|cx| settle(&mut model.state.core, cx));
    assert!(probe.take().is_empty(), "the zoom did not change");

    assert!(model.state.core.close_html_overlay(page.id()));
    let _ = model.state.core.commit();
    cx.update(|cx| settle(&mut model.state.core, cx));
    assert!(!page.active());
    assert!(self::page(page.id()).is_none());
    assert_eq!(with(|registry| registry.graveyard.len()), 1);
}

#[gpui_kit::test]
fn the_models_value_and_the_terminals_colors_reach_every_page(cx: &mut TestAppContext) {
    fake_native::install();
    let source = url_overlay();
    let model = Model::new(&source);
    let (page, probe) = realized(&source, &model);
    cx.update(|cx| apply(&source, true, cx));
    assert!(page.on_screen());
    cx.update(|cx| apply(&overlay(HtmlSource::Image { path: "x".to_owned() }), false, cx));
    assert!(page.on_screen(), "an unknown page changes nothing");
    let colors = TerminalColors {
        background: Some("#102030".to_owned()),
        foreground: Some("#f0f0f0".to_owned()),
        palette: Vec::new(),
    };
    cx.update(|cx| refresh_themes(&colors, cx));
    assert!(matches!(probe.take().as_slice(), [Call::Theme(_)]));
    assert_eq!(page.theme(), PanelColors::resolve(&colors, None).theme());
    assert_eq!(with(|registry| registry.colors.background.clone()), colors.background);
}

#[gpui_kit::test]
fn copy_link_puts_the_pages_address_on_the_clipboard(cx: &mut TestAppContext) {
    fake_native::install();
    let source = url_overlay();
    let model = Model::new(&source);
    let (page, _probe) = realized(&source, &model);
    cx.update(|cx| copy_link(page.id(), cx));
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()).as_deref(),
        Some("https://example.invalid/start")
    );
    cx.update(|cx| copy_link(Uuid::new_v4(), cx));
}

#[test]
fn focusing_replica_search_goes_to_a_text_replicas_renderer_only() {
    fake_native::install();
    let text = overlay(HtmlSource::Replica {
        path: "/notes.txt".to_owned(),
        content: ReplicaContent::Text { line: None },
    });
    let model = Model::new(&text);
    let (page, probe) = realized(&text, &model);
    focus_replica_search(page.id());
    assert!(matches!(probe.take().as_slice(), [Call::Renderer(_)]));
    let url = url_overlay();
    let model = Model::new(&url);
    let (page, probe) = realized(&url, &model);
    focus_replica_search(page.id());
    focus_replica_search(Uuid::new_v4());
    assert!(probe.take().is_empty());
}

// MARK: - documents

#[gpui_kit::test]
fn a_document_read_on_a_worker_lands_on_the_page(cx: &mut TestAppContext) {
    pumped(cx);
    let queue = jobs_for_the_test();
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("notes.md");
    std::fs::write(&path, "# Notes").unwrap();
    let source = overlay(HtmlSource::Markdown {
        path: path.to_string_lossy().into_owned(),
    });
    let model = Model::new(&source);
    let (page, probe) = realized(&source, &model);
    let job = worker_job(&queue, || {});
    run(job, &mut cx.to_async());
    let shown = page.text().lock().unwrap().clone().unwrap();
    assert!(shown.contains("Notes"), "{shown}");
    assert_eq!(
        probe.take(),
        [Call::Load(super::super::scheme::text_address(page.id()))]
    );
    document_loaded(Uuid::new_v4(), 0, Ok(String::new()));
}

#[gpui_kit::test]
fn a_replica_built_here_asks_its_origin_through_the_core(cx: &mut TestAppContext) {
    pumped(cx);
    let queue = jobs_for_the_test();
    let source = overlay(HtmlSource::Replica {
        path: "/notes.md".to_owned(),
        content: ReplicaContent::Markdown,
    });
    let mut model = Model::new(&source);
    let poster = model.poster();
    with(|registry| registry.poster = Some(poster));
    let (page, _probe) = realized(&source, &model);
    let job = worker_job(&queue, || model.drain());
    run(job, &mut cx.to_async());
    model.drain();
    // no machine is on the other end: the read fails as unreadable, and the page says so
    assert_eq!(model.overlay().load_state, HtmlLoadState::Failed);
    assert!(model.overlay().load_error.is_some());
    assert!(page.active());
}

#[gpui_kit::test]
fn a_text_page_rebuilds_for_a_new_theme_and_stops_with_its_page(cx: &mut TestAppContext) {
    pumped(cx);
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("notes.txt");
    std::fs::write(&path, "hello").unwrap();
    let source = overlay(HtmlSource::Text {
        path: path.to_string_lossy().into_owned(),
        line: None,
    });
    let model = Model::new(&source);
    let (page, probe) = realized(&source, &model);
    settle_until(cx, || probe.take().iter().any(|call| matches!(call, Call::Load(_))));
    let text = page.text().lock().unwrap().clone().unwrap();
    assert!(text.contains("hello"), "{text}");
    cx.update(|cx| page.report(Report::TextPage("{}".to_owned()), cx));
    assert!(
        probe.take().is_empty(),
        "a report that is none of the renderer's does nothing"
    );

    let colors = TerminalColors {
        background: Some("#102030".to_owned()),
        foreground: Some("#f0f0f0".to_owned()),
        palette: Vec::new(),
    };
    cx.update(|cx| page.apply_theme(&colors, cx));
    assert!(probe.take().iter().any(|call| matches!(call, Call::Theme(_))));
    cx.update(|cx| page.report(Report::ThemeInstalled, cx));
    settle_until(cx, || probe.take().iter().any(|call| matches!(call, Call::Load(_))));

    cx.update(|cx| {
        page.apply(&source, false, cx);
        page.close(cx);
    });
    assert!(!page.active());
    settle_until(cx, || true);
}

#[test]
fn a_document_read_without_a_pump_or_a_poster_does_nothing() {
    fake_native::install();
    read_document(Uuid::new_v4(), 1, HtmlSource::Image { path: "x".to_owned() });
    read_replica_document(Uuid::new_v4(), 1, "/x".to_owned(), ReplicaContent::Markdown);
}

// MARK: - page requests

#[test]
fn a_parked_request_is_answered_once_with_its_id() {
    fake_native::install();
    let source = url_overlay();
    let model = Model::new(&source);
    let (page, probe) = realized(&source, &model);
    let first = park(page.id(), 11);
    let second = park(page.id(), 12);
    assert_ne!(first, second);
    answer(second, &Response::with_result(serde_json::json!({"value": 3})));
    reject(first, "refused");
    reject(first, "again");
    let calls = probe.take();
    assert_eq!(calls.len(), 2, "{calls:?}");
    let replies: Vec<serde_json::Value> = calls
        .iter()
        .map(|call| {
            let Call::Post(json) = call else { panic!("{call:?}") };
            serde_json::from_str(json).unwrap()
        })
        .collect();
    assert_eq!(replies[0]["id"], 12);
    assert_eq!(replies[0]["result"]["value"], 3);
    assert_eq!(replies[1]["id"], 11);
    assert_eq!(replies[1]["error"], "refused");
    let left = park(Uuid::new_v4(), 13);
    reject(left, "its page is gone");
    assert!(probe.take().is_empty());
}

#[test]
fn a_relay_with_no_way_to_the_core_is_refused() {
    fake_native::install();
    let source = url_overlay();
    let model = Model::new(&source);
    let (page, probe) = realized(&source, &model);
    relay(page.id(), "{}".to_owned(), park(page.id(), 7));
    let calls = probe.take();
    let Call::Post(json) = &calls[0] else {
        panic!("{calls:?}")
    };
    let reply: serde_json::Value = serde_json::from_str(json).unwrap();
    assert_eq!(reply["error"], "page closed");
}

#[test]
fn a_relay_reaches_the_core_and_its_answer_reaches_the_page() {
    fake_native::install();
    let source = overlay(HtmlSource::Replica {
        path: "/page.html".to_owned(),
        content: ReplicaContent::Html,
    });
    let mut model = Model::new(&source);
    let poster = model.poster();
    with(|registry| registry.poster = Some(poster));
    let (page, probe) = realized(&source, &model);
    relay(page.id(), "{}".to_owned(), park(page.id(), 8));
    assert!(probe.take().is_empty(), "it goes on the main loop");
    model.drain();
    let calls = probe.take();
    let Call::Post(json) = &calls[0] else {
        panic!("{calls:?}")
    };
    let reply: serde_json::Value = serde_json::from_str(json).unwrap();
    assert_eq!(reply["id"], 8);
    assert_eq!(reply["error"], "page closed", "no origin is attached");
}

#[gpui_kit::test]
fn a_replica_read_is_served_only_for_its_own_page(cx: &mut TestAppContext) {
    pumped(cx);
    let source = overlay(HtmlSource::Replica {
        path: "/page.html".to_owned(),
        content: ReplicaContent::Html,
    });
    let model = Model::new(&source);
    let (page, _probe) = realized(&source, &model);
    let other = format!("ngterm-page://{}/secret", Uuid::new_v4());

    // no poster: nothing can read it
    let (reply, slot) = responder();
    send(Job::ReplicaRead {
        page: page.id(),
        uri: format!("ngterm-page://{}/page.html", page.id()),
        reply,
    });
    settle_until(cx, || slot.lock().unwrap().is_some());
    let refused = slot.lock().unwrap().take().unwrap();
    assert_eq!(refused.status(), 403);
    assert_eq!(refused.body().as_slice(), policy::NOT_THIS_PAGE.as_bytes());

    // a read naming another page never reaches the origin
    let mut model = model;
    let poster = model.poster();
    with(|registry| registry.poster = Some(poster));
    let (reply, slot) = responder();
    send(Job::ReplicaRead {
        page: page.id(),
        uri: other,
        reply,
    });
    settle_until(cx, || slot.lock().unwrap().is_some());
    assert_eq!(model.pending(), 0);

    // its own file goes to the core, and what comes back is answered
    let (reply, slot) = responder();
    send(Job::ReplicaRead {
        page: page.id(),
        uri: format!("ngterm-page://{}/page.html", page.id()),
        reply,
    });
    settle_until(cx, || model.pending() > 0);
    model.drain();
    let answered = slot.lock().unwrap().take().unwrap();
    assert_eq!(answered.status(), 403, "no origin is attached");
}

#[test]
fn a_replicas_answer_is_served_whole_or_as_its_failure() {
    fake_native::install();
    let (reply, slot) = responder();
    with(|registry| registry.reads.insert(1, reply));
    answer_read(
        1,
        Ok(PageResource {
            body: vec![0, 255, 1],
            mime_type: "application/octet-stream".to_owned(),
        }),
    );
    let served = slot.lock().unwrap().take().unwrap();
    assert_eq!(served.status(), 200);
    assert_eq!(served.headers()["Content-Type"], "application/octet-stream");
    assert_eq!(served.body(), &[0, 255, 1]);

    let (reply, slot) = responder();
    with(|registry| registry.reads.insert(2, reply));
    answer_read(2, Err("origin disconnected".to_owned()));
    let failed = slot.lock().unwrap().take().unwrap();
    assert_eq!(failed.status(), 403);
    assert_eq!(failed.headers()["Content-Type"], "text/plain; charset=utf-8");
    assert_eq!(failed.body().as_slice(), b"origin disconnected");
    answer_read(3, Err("nobody waits".to_owned()));
}

// MARK: - the saved store's clear

fn store_ready(engine: &FakeEngine, cx: &mut TestAppContext, directory: &TempDir) {
    state_dir_at(directory.path());
    assert_eq!(persistent_store_failure(), None);
    engine.windows_have(21);
    let cx = window_of(cx);
    cx.update(|window, cx| set_covered(window, false, cx));
}

#[gpui_kit::test]
fn the_saved_store_clears_in_a_windows_hidden_web_view_and_reports_once(cx: &mut TestAppContext) {
    let engine = pumped(cx);
    let directory = TempDir::new().unwrap();
    store_ready(&engine, cx, &directory);
    for result in [Err("disk full".to_owned()), Ok(())] {
        assert_eq!(PageHost.clear_saved_store(), Ok(StoreClear::Started));
        assert_eq!(PageHost.saved_store_clear(), ClearProgress::Running);
        assert_eq!(PageHost.clear_saved_store(), Err(errors::CLEARING.to_owned()));
        let cleared = engine.clears().len();
        settle_until(cx, || engine.clears().len() == cleared + 1);
        let (parent, store) = engine.clears().last().unwrap().clone();
        assert_eq!(parent, 21);
        assert!(!store.private);
        engine.finish_clear(result.clone());
        settle_until(cx, || !with(|registry| registry.clearing));
        assert_eq!(
            PageHost.saved_store_clear(),
            result.map_or_else(ClearProgress::Failed, |()| ClearProgress::Done)
        );
        assert_eq!(PageHost.saved_store_clear(), ClearProgress::Done);
    }
}

#[gpui_kit::test]
fn a_clear_is_refused_while_it_cannot_run(cx: &mut TestAppContext) {
    let engine = pumped(cx);
    let directory = TempDir::new().unwrap();
    state_dir_at(directory.path());
    assert_eq!(
        PageHost.clear_saved_store(),
        Ok(StoreClear::Nothing),
        "a profile never created"
    );
    assert!(!directory.path().join(profile::FILENAME).exists());
    assert_eq!(persistent_store_failure(), None);
    assert_eq!(
        PageHost.clear_saved_store(),
        Err(errors::PERSISTENT_UNAVAILABLE.to_owned()),
        "no window to run in"
    );

    store_ready(&engine, cx, &directory);
    let source = url_overlay();
    let model = Model::new(&source);
    let open = page_of(&source, Storage::Saved, &model);
    assert_eq!(PageHost.clear_saved_store(), Err(errors::pages_open(1)));
    with(|registry| registry.pages.remove(&open.id()));

    with(|registry| registry.failure = Some("the app is quitting".to_owned()));
    assert_eq!(
        PageHost.clear_saved_store(),
        Err(errors::PERSISTENT_UNAVAILABLE.to_owned())
    );
}

#[test]
fn a_clear_with_nothing_to_run_in_reports_it() {
    let engine = fake_native::install();
    clear();
    assert!(engine.clears().is_empty());
    // no pump either: nobody hears, and nothing panics
    let (jobs, queue) = async_channel::unbounded::<Job>();
    with(|registry| registry.jobs = Some(jobs));
    clear();
    let Ok(Job::Cleared(result)) = queue.try_recv() else {
        panic!("the clear ends");
    };
    assert_eq!(result, Err(errors::PERSISTENT_UNAVAILABLE.to_owned()));
}

#[gpui_kit::test]
fn a_clear_with_no_state_directory_reports_it(cx: &mut TestAppContext) {
    fake_native::install();
    let window = cx.add_empty_window().update(|window, _| window.window_handle());
    let (jobs, queue) = async_channel::unbounded::<Job>();
    with(|registry| {
        registry.jobs = Some(jobs);
        registry.saved = Some("profile".to_owned());
        registry.windows.insert(window, 21);
    });
    clear();
    let Ok(Job::Cleared(result)) = queue.try_recv() else {
        panic!("the clear ends");
    };
    assert_eq!(result, Err(NO_STATE.to_owned()));
}

#[test]
fn an_engine_that_ended_fails_every_page_and_a_clear_in_flight_once() {
    fake_native::install();
    let source = url_overlay();
    let mut model = Model::new(&source);
    let page = page_of(&source, Storage::Ephemeral, &model);
    with(|registry| registry.clearing = true);
    engine_ended();
    model.drain();
    assert_eq!(model.overlay().load_error.as_deref(), Some(ENDED));
    assert_eq!(PageHost.saved_store_clear(), ClearProgress::Failed(ENDED.to_owned()));
    assert_eq!(PageHost.web_views_unavailable().as_deref(), Some(ENDED));
    engine_ended();
    assert_eq!(model.pending(), 0, "only the first ending reports");
    drop(page);
}

#[gpui_kit::test]
fn a_browser_process_that_ended_fails_the_page_it_reported_through(cx: &mut TestAppContext) {
    fake_native::install();
    let source = url_overlay();
    let mut model = Model::new(&source);
    let (page, _probe) = realized(&source, &model);
    cx.update(|cx| page.report(Report::ProcessFailed { browser: true }, cx));
    model.drain();
    assert_eq!(model.overlay().load_error.as_deref(), Some(ENDED));
}

#[gpui_kit::test]
fn a_page_mounted_after_the_engine_ended_fails_at_once(cx: &mut TestAppContext) {
    let engine = pumped(cx);
    engine.windows_have(7);
    with(|registry| registry.failure = Some(ENDED.to_owned()));
    let source = url_overlay();
    let mut model = Model::new(&source);
    let poster = model.poster();
    with(|registry| registry.poster = Some(poster));
    let cx = window_of(cx);
    cx.update(|window, cx| mount(&source, None, window, cx)).unwrap();
    model.drain();
    assert_eq!(model.overlay().load_error.as_deref(), Some(ENDED));
    settle_until(cx, || true);
    assert_eq!(engine.made(), 0);
}

#[test]
fn shutdown_lets_the_pages_go_and_refuses_what_comes_after() {
    // its own thread: the shutdown ends this thread's `outside` queue for good
    std::thread::spawn(|| {
        fake_native::install();
        let source = url_overlay();
        let model = Model::new(&source);
        let (page, probe) = realized(&source, &model);
        with(|registry| registry.graveyard.push((page.native().unwrap(), None)));
        shutdown();
        assert!(self::page(page.id()).is_none());
        assert_eq!(PageHost.web_views_unavailable().as_deref(), Some("the app is quitting"));
        assert!(
            !probe.dropped(),
            "let go, not closed: a close would run inside the quit"
        );
        drop(page);
    })
    .join()
    .unwrap();
}

// MARK: - the window and the keyboard

#[gpui_kit::test]
fn a_layer_over_a_window_covers_only_that_windows_pages(cx: &mut TestAppContext) {
    let engine = pumped(cx);
    engine.windows_have(7);
    let source = url_overlay();
    let model = Model::new(&source);
    let (page, probe) = realized(&source, &model);
    let cx = window_of(cx);
    let handle = cx.update(|window, _| window.window_handle());
    let _element = cx.update(|window, cx| cx.new(|cx| PageView::new(Rc::clone(&page), 7, window, cx)));
    cx.update(|_, cx| apply(&source, true, cx));
    page.place(native::Bounds {
        x: 0.,
        y: 0.,
        width: 10.,
        height: 10.,
    });
    assert!(!is_covered(handle));
    assert!(page.wanted().visible);
    cx.update(|window, cx| set_covered(window, true, cx));
    assert!(is_covered(handle));
    assert_eq!(with(|registry| registry.windows.get(&handle).copied()), Some(7));
    assert!(!page.wanted().visible, "the pages of a covered window step aside");
    cx.update(|window, cx| set_covered(window, true, cx));
    settle_until(cx, || true);
    assert!(probe.take().iter().all(|call| matches!(call, Call::Bounds(_))));
}

#[gpui_kit::test]
fn a_page_holds_the_windows_keyboard_through_its_element_or_its_web_view(cx: &mut TestAppContext) {
    let engine = pumped(cx);
    let directory = TempDir::new().unwrap();
    state_dir_at(directory.path());
    engine.windows_have(7);
    let source = url_overlay();
    let (root, cx) = cx.add_window_view(|_, _| Holder { element: None });
    cx.update(|window, _| window.activate_window());
    let element = cx.update(|window, cx| mount(&source, None, window, cx)).unwrap();
    root.update(cx, |root, cx| {
        root.element = Some(element.clone());
        cx.notify();
    });
    cx.update(|_, cx| apply(&source, true, cx));
    settle_until(cx, || engine.made() == 1);
    let page = page(source.id).unwrap();
    page.place(native::Bounds {
        x: 0.,
        y: 0.,
        width: 10.,
        height: 10.,
    });
    let held = |cx: &mut VisualTestContext| cx.update(|window, cx| page_holds_keyboard(window, cx));
    assert!(!held(cx));
    let keystroke = gpui_kit::Keystroke::parse("x").unwrap();
    let stroke = crate::terminal::input::KeyStroke::new(&keystroke, &crate::terminal::input::Source::Keystroke, false);
    assert!(
        !cx.update(|window, cx| key_down(&keystroke, &stroke, window, cx)),
        "no page has the keyboard"
    );

    cx.update(|window, cx| element.update(cx, |view, cx| window.focus(view.focus(), cx)));
    cx.run_until_parked();
    assert!(held(cx));
    assert!(
        cx.update(|window, cx| key_down(&keystroke, &stroke, window, cx)),
        "the key hands the keyboard to the web view"
    );
    settle_until(cx, || engine.probe(0).take().contains(&Call::Focus));

    cx.update(gpui_kit::Window::blur);
    cx.run_until_parked();
    assert!(!held(cx));
    cx.update(|_, cx| page.report(Report::Focus(true), cx));
    cx.run_until_parked();
    assert!(held(cx), "a field inside the page holds the native keyboard");
}
