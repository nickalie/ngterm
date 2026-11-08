//! Registry behavior at the engine and filesystem boundaries.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui_kit::TestAppContext;
use ngterm_core::html::HtmlPageFlags;
use ngterm_wpe::{EditingCommand, Geometry, Input, Load, MenuToken, PolicyToken, ReplyToken, UserScript, ViewSpec};

use super::*;
use crate::html_overlay::engine::Presenter;
use crate::html_overlay::testing::{Call, Recorder};

/// Keeps the replies and store commands whose arguments the element's recorder does not need.
#[derive(Default)]
pub(crate) struct Recorded {
    pub(crate) recorder: Recorder,
    pub(crate) policies: RefCell<Vec<(PolicyToken, bool)>>,
    pub(crate) replies: RefCell<Vec<(ReplyToken, Result<String, String>)>>,
    pub(crate) resources: RefCell<Vec<(SchemeToken, SchemeResponse)>>,
    pub(crate) cleared: RefCell<Vec<SessionId>>,
    pub(crate) dropped: RefCell<Vec<SessionId>>,
    pub(crate) zooms: RefCell<Vec<f64>>,
    pub(crate) scripts: RefCell<Vec<Vec<UserScript>>>,
    pub(crate) stopped: Cell<bool>,
}

macro_rules! forward {
    ($($name:ident($($arg:ident: $ty:ty),*) $(-> $result:ty)?; )*) => {
        $(fn $name(&self, $($arg: $ty),*) $(-> $result)? {
            self.recorder.$name($($arg),*)
        })*
    };
}

impl Web for Recorded {
    forward! {
        create_session(spec: SessionSpec) -> SessionId;
        create_view(spec: ViewSpec) -> ViewId;
        close_view(view: ViewId);
        load(view: ViewId, load: Load);
        evaluate(view: ViewId, script: String, world: Option<String>) -> async_channel::Receiver<ngterm_wpe::Evaluation>;
        reload(view: ViewId);
        go_back(view: ViewId);
        go_forward(view: ViewId);
        set_geometry(view: ViewId, geometry: Geometry);
        set_visible(view: ViewId, visible: bool);
        set_focused(view: ViewId, focused: bool);
        input(view: ViewId, input: Input);
        fall_back_to_shm(view: ViewId);
        set_clipboard(text: String);
        choose_option(token: MenuToken, index: Option<u32>);
        editing_command(view: ViewId, command: EditingCommand);
        ime_preedit(view: ViewId, text: String, cursor: u32);
        ime_commit(view: ViewId, text: String);
        presenter() -> Presenter;
    }

    fn drop_session(&self, session: SessionId) {
        self.dropped.borrow_mut().push(session);
    }
    fn clear_session(&self, session: SessionId) {
        self.cleared.borrow_mut().push(session);
    }
    fn set_zoom(&self, _: ViewId, zoom: f64) {
        self.zooms.borrow_mut().push(zoom);
    }
    fn set_scripts(&self, _: ViewId, scripts: Vec<UserScript>) {
        self.scripts.borrow_mut().push(scripts);
    }
    fn decide_policy(&self, token: PolicyToken, allow: bool) {
        self.policies.borrow_mut().push((token, allow));
    }
    fn reply_script_message(&self, token: ReplyToken, reply: Result<String, String>) {
        self.replies.borrow_mut().push((token, reply));
    }
    fn finish_scheme_request(&self, token: SchemeToken, response: SchemeResponse) {
        self.resources.borrow_mut().push((token, response));
    }
    fn shut_down(&self) {
        self.stopped.set(true);
    }
}

pub(crate) fn overlay(source: HtmlSource) -> HtmlOverlay {
    HtmlOverlay::new(source, HtmlPageFlags::default())
}

pub(crate) fn url_overlay() -> HtmlOverlay {
    overlay(HtmlSource::Url(
        url::Url::parse("https://example.invalid/start").unwrap(),
    ))
}

pub(crate) fn new_page(overlay: &HtmlOverlay, storage: Storage) -> Rc<Page> {
    Page::new(
        overlay,
        None,
        PanelColors::resolve(&TerminalColors::default(), None),
        storage,
        &crate::state::detached_poster(),
    )
}

pub(crate) fn replies() -> Rc<Recorded> {
    reset();
    let web = Rc::new(Recorded::default());
    use_web(web.clone());
    web
}

pub(crate) fn register(overlay: &HtmlOverlay, storage: Storage) -> Rc<Page> {
    let page = new_page(overlay, storage);
    with(|registry| registry.pages.insert(page.id(), page.clone()));
    page
}

#[test]
fn display_scale_rejects_nonpositive_and_nonfinite_updates() {
    reset();
    let window = Uuid::new_v4();
    assert_eq!(PageHost.display_scale(window).to_bits(), 1.0_f64.to_bits());
    set_window_scale(window, 2.5);
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        set_window_scale(window, bad);
        assert_eq!(PageHost.display_scale(window).to_bits(), 2.5_f64.to_bits());
    }
    assert_eq!(PageHost.page_zoom().to_bits(), 1.0_f64.to_bits());
    with(|registry| registry.zoom = Some(1.25));
    assert_eq!(PageHost.page_zoom().to_bits(), 1.25_f64.to_bits());
}

#[test]
fn saved_storage_is_never_replaced_by_a_temporary_store() {
    reset();
    let mut overlay = url_overlay();
    assert_eq!(storage(&overlay), Storage::Ephemeral);
    overlay.flags.persistent = true;
    assert_eq!(
        storage(&overlay),
        Storage::Unavailable(errors::PERSISTENT_UNAVAILABLE.to_owned())
    );
    let dir = tempfile::tempdir().unwrap();
    with(|registry| registry.state_dir = Some(dir.path().to_owned()));
    assert_eq!(storage(&overlay), Storage::Saved);
    assert_eq!(PageHost.persistent_store_failure(), None);
    with(|registry| registry.clearing = true);
    assert_eq!(storage(&overlay), Storage::Unavailable(errors::CLEARING.to_owned()));
    overlay.source = HtmlSource::File {
        path: "/a.html".to_owned(),
        grant_root: None,
    };
    assert_eq!(storage(&overlay), Storage::Ephemeral);
}

#[test]
fn a_corrupt_saved_profile_reports_its_path_without_overwriting_it() {
    reset();
    let dir = tempfile::tempdir().unwrap();
    let profile = dir.path().join(profile::FILENAME);
    std::fs::write(&profile, "corrupt").unwrap();
    with(|registry| registry.state_dir = Some(dir.path().to_owned()));
    assert_eq!(
        persistent_store_failure(),
        Some(format!("browser profile is not a UUID: {}", profile.display()))
    );
    assert_eq!(std::fs::read_to_string(profile).unwrap(), "corrupt");
}

#[test]
fn saved_pages_share_one_store_and_temporary_pages_get_separate_stores() {
    let web = replies();
    let engine: Rc<dyn Web> = web.clone();
    assert_eq!(saved_session(&engine), None);
    let dir = tempfile::tempdir().unwrap();
    with(|registry| registry.state_dir = Some(dir.path().to_owned()));
    assert_eq!(persistent_store_failure(), None);
    let first = register(&url_overlay(), Storage::Saved);
    let second = register(&url_overlay(), Storage::Saved);
    let temporary = register(&url_overlay(), Storage::Ephemeral);
    realize_waiting();
    let specs = web.recorder.specs.borrow();
    let saved = with(|registry| registry.saved).unwrap();
    assert_eq!(specs.iter().filter(|spec| spec.session == saved).count(), 2);
    assert_eq!(specs.iter().filter(|spec| spec.session != saved).count(), 1);
    assert!(first.view_id().is_some() && second.view_id().is_some() && temporary.view_id().is_some());
    let calls = web.recorder.take();
    assert_eq!(
        calls
            .iter()
            .filter(|call| matches!(call, Call::CreateSession(_)))
            .count(),
        2
    );
    let id = BrowserProfile::new(dir.path()).existing_identifier().unwrap().unwrap();
    assert!(calls.contains(&Call::CreateSession(SessionSpec::Persistent {
        data_dir: dir.path().join("browser").join(uuid_string(id)).join("data"),
        cache_dir: dir.path().join("browser").join(uuid_string(id)).join("cache"),
    })));
}

#[gpui_kit::test]
fn saved_store_clear_reports_completion_or_one_failure(cx: &mut TestAppContext) {
    let web = replies();
    let dir = tempfile::tempdir().unwrap();
    with(|registry| registry.state_dir = Some(dir.path().to_owned()));
    assert_eq!(PageHost.clear_saved_store(), Ok(StoreClear::Nothing));
    assert!(!dir.path().join(profile::FILENAME).exists());
    assert_eq!(persistent_store_failure(), None);
    for result in [Err("disk full".to_owned()), Ok(())] {
        assert_eq!(PageHost.clear_saved_store(), Ok(StoreClear::Started));
        assert_eq!(PageHost.saved_store_clear(), ClearProgress::Running);
        assert_eq!(PageHost.clear_saved_store(), Err(errors::CLEARING.to_owned()));
        let session = *web.cleared.borrow().last().unwrap();
        cx.update(|cx| {
            dispatch(
                &Event::SessionCleared {
                    session,
                    result: result.clone(),
                },
                cx,
            );
        });
        assert_eq!(
            PageHost.saved_store_clear(),
            result.map_or_else(ClearProgress::Failed, |()| ClearProgress::Done)
        );
        assert_eq!(PageHost.saved_store_clear(), ClearProgress::Done);
    }
    assert_eq!(web.cleared.borrow().len(), 2);
}

#[test]
fn an_engine_exit_finishes_a_pending_clear_and_shutdown_refuses_late_starts() {
    let web = replies();
    with(|registry| registry.clearing = true);
    engine_ended();
    assert_eq!(PageHost.saved_store_clear(), ClearProgress::Failed(ENDED.to_owned()));
    assert_eq!(PageHost.saved_store_clear(), ClearProgress::Done);
    assert_eq!(engine().err().as_deref(), Some(ENDED));
    engine_ended();
    use_web(web.clone());
    shutdown();
    assert!(web.stopped.get());
    assert_eq!(engine().err().as_deref(), Some("the app is quitting"));
    assert!(started_engine(Err(ngterm_wpe::StartError::AlreadyStarted)).is_none());
    engine_ended();
    assert_eq!(engine().err().as_deref(), Some("the app is quitting"));
    shutdown();
}

#[test]
fn parked_replies_are_answered_once_with_their_original_tokens() {
    let web = replies();
    let first = park(ReplyToken::from_raw(91));
    let second = park(ReplyToken::from_raw(92));
    assert_ne!(first, second);
    answer_value(second, Ok(serde_json::json!({"unknown": [1, true]})));
    reject(first, "permission denied");
    reject(first, "duplicate");
    answer_value(second, Err("late".to_owned()));
    assert_eq!(
        *web.replies.borrow(),
        [
            (ReplyToken::from_raw(92), Ok("{\"unknown\":[1,true]}".to_owned())),
            (ReplyToken::from_raw(91), Err("permission denied".to_owned()))
        ]
    );
    let third = park(ReplyToken::from_raw(93));
    answer(third, &Response::with_result(serde_json::json!({"value": 3})));
    assert_eq!(web.replies.borrow().last().unwrap().1, Ok("{\"value\":3}".to_owned()));
    shutdown();
    reject(park(ReplyToken::from_raw(94)), "gone");
    assert_eq!(web.replies.borrow().len(), 3);
}

#[gpui_kit::test]
fn events_for_closed_pages_are_refused_and_resources_preserve_bytes(cx: &mut TestAppContext) {
    let web = replies();
    let view = ViewId::from_raw(999);
    let policy = PolicyToken::from_raw(1);
    let reply = ReplyToken::from_raw(2);
    cx.update(|cx| {
        dispatch(
            &Event::Policy(ngterm_wpe::PolicyRequest {
                token: policy,
                view,
                kind: ngterm_wpe::PolicyKind::Response {
                    uri: "https://example.invalid".to_owned(),
                    main_frame: true,
                    mime_supported: true,
                },
            }),
            cx,
        );
        dispatch(
            &Event::ScriptMessage(ngterm_wpe::ScriptMessage {
                token: reply,
                view,
                handler: "ngterm".to_owned(),
                body: Some("{}".to_owned()),
            }),
            cx,
        );
        for view in [None, Some(view)] {
            dispatch(
                &Event::SchemeRequest(SchemeRequest {
                    token: SchemeToken::from_raw(3),
                    view,
                    uri: "ngterm-file:///secret".to_owned(),
                }),
                cx,
            );
        }
    });
    assert_eq!(*web.policies.borrow(), [(policy, false)]);
    assert_eq!(*web.replies.borrow(), [(reply, Err("page closed".to_owned()))]);
    assert!(web.resources.borrow().iter().all(|(_, response)| matches!(response,
        SchemeResponse::Error { kind: SchemeError::PermissionDenied, message } if message == policy::OUTSIDE_GRANT)));
    answer_resource(
        SchemeToken::from_raw(4),
        Ok(PageResource {
            body: vec![0, 255, 1],
            mime_type: "application/octet-stream".to_owned(),
        }),
    );
    answer_resource(SchemeToken::from_raw(5), Err("origin disconnected".to_owned()));
    let resources = web.resources.borrow();
    assert!(matches!(&resources[2], (token, SchemeResponse::Bytes { data, mime })
        if *token == SchemeToken::from_raw(4) && data == &[0, 255, 1] && mime == "application/octet-stream"));
    assert!(
        matches!(&resources[3], (token, SchemeResponse::Error { kind: SchemeError::Failed, message })
        if *token == SchemeToken::from_raw(5) && message == "origin disconnected")
    );
}

#[derive(Default)]
struct Link {
    reads: RefCell<Vec<(Uuid, String, SchemeToken)>>,
    bridges: RefCell<Vec<(Uuid, String, u64)>>,
    content_reads: RefCell<Vec<(Uuid, String)>>,
}

impl ReplicaLink for Link {
    fn fetch(&self, page: Uuid, path: String, token: SchemeToken) {
        self.reads.borrow_mut().push((page, path, token));
    }
    fn bridge(&self, page: Uuid, body: String, reply: u64) {
        self.bridges.borrow_mut().push((page, body, reply));
    }
    /// An HTML replica fetches its files: a content read never comes here.
    fn read(&self, page: Uuid, path: String, _: usize, _: ContentFailures, _: ReadAnswer) {
        self.content_reads.borrow_mut().push((page, path));
    }
}

#[test]
fn replicas_read_only_their_own_origin_and_relay_the_body_unchanged() {
    let web = replies();
    let source = overlay(HtmlSource::Replica {
        path: "/docs/a b.html".to_owned(),
        content: ngterm_core::html::ReplicaContent::Html,
    });
    let page = register(&source, Storage::Ephemeral);
    realize_waiting();
    let view = page.view_id().unwrap();
    let uri = policy::replica_uri(page.id(), "/docs/a b.html").unwrap();
    let request = SchemeRequest {
        token: SchemeToken::from_raw(1),
        view: Some(view),
        uri,
    };
    assert!(!relay(page.id(), "{}".to_owned(), 7));
    serve_replica(&request);
    let link = Rc::new(Link::default());
    set_replica_link(link.clone());
    serve_replica(&request);
    let mut wrong = request.clone();
    wrong.uri = policy::replica_uri(Uuid::new_v4(), "/secret").unwrap();
    serve_replica(&wrong);
    wrong.view = None;
    serve_replica(&wrong);
    assert_eq!(
        *link.reads.borrow(),
        [(page.id(), "/docs/a b.html".to_owned(), request.token)]
    );
    assert!(relay(page.id(), "{\"extra\":true}".to_owned(), 7));
    assert_eq!(*link.bridges.borrow(), [(page.id(), "{\"extra\":true}".to_owned(), 7)]);
    assert!(link.content_reads.borrow().is_empty());
    assert_eq!(web.resources.borrow().len(), 3);
    assert!(web.resources.borrow().iter().all(|(_, result)| *result
        == SchemeResponse::Error {
            kind: SchemeError::PermissionDenied,
            message: policy::NOT_THIS_PAGE.to_owned(),
        }));
}

#[test]
fn navigation_requires_a_realized_page_and_preserves_history_refusals() {
    let _web = replies();
    let page = register(&url_overlay(), Storage::Ephemeral);
    for id in [Uuid::new_v4(), page.id()] {
        assert_eq!(
            PageHost.navigate(id, HtmlNavigation::Back),
            Err("html overlay not realized".to_owned())
        );
    }
    realize_waiting();
    assert_eq!(
        PageHost.navigate(page.id(), HtmlNavigation::Back),
        Err("no page to go back to".to_owned())
    );
    assert_eq!(
        PageHost.navigate(page.id(), HtmlNavigation::Forward),
        Err("no page to go forward to".to_owned())
    );
}

#[gpui_kit::test]
fn clipboard_events_offer_text_with_or_without_html(cx: &mut TestAppContext) {
    let _web = replies();
    for html in [None, Some("<b>copied</b>".to_owned())] {
        cx.update(|cx| {
            dispatch(
                &Event::Clipboard {
                    text: "copied".to_owned(),
                    html,
                },
                cx,
            );
        });
        assert_eq!(cx.read_from_clipboard().unwrap().text().as_deref(), Some("copied"));
    }
    cx.update(|cx| copy_link(Uuid::new_v4(), cx));
    assert_eq!(cx.read_from_clipboard().unwrap().text().as_deref(), Some("copied"));
    let page = register(&url_overlay(), Storage::Ephemeral);
    cx.update(|cx| copy_link(page.id(), cx));
    assert_eq!(
        cx.read_from_clipboard().unwrap().text().as_deref(),
        Some("https://example.invalid/start")
    );
}

/// A queued main-loop boundary, so page reports run against the same model the app uses.
#[derive(Default)]
pub(crate) struct Posted(std::sync::Mutex<Vec<ngterm_host::MainTask<AppState>>>);

impl ngterm_host::MainLoop<AppState> for Posted {
    fn post(&self, task: ngterm_host::MainTask<AppState>) -> Result<(), ngterm_host::MainLoopClosed> {
        self.0.lock().unwrap().push(task);
        Ok(())
    }
    fn post_after(
        &self,
        _: std::time::Duration,
        _: ngterm_host::MainTask<AppState>,
    ) -> Result<(), ngterm_host::MainLoopClosed> {
        Ok(())
    }
}

pub(crate) struct Model {
    pub(crate) state: AppState,
    pub(crate) window: Uuid,
    pub(crate) session: Uuid,
    posted: Arc<Posted>,
    main_loop: Arc<dyn ngterm_host::MainLoop<AppState>>,
}

impl Model {
    pub(crate) fn new(overlay: &HtmlOverlay) -> Self {
        let mut library = ngterm_core::model::WindowLibrary::seeded("/tmp");
        let window = library.active_window_id().unwrap();
        let store = library.store_mut(window).unwrap();
        let session = store.sessions().next().unwrap().id();
        store
            .open_html_overlay(session, None, overlay.clone(), None, None)
            .unwrap();
        let core = Core::new(
            ngterm_core::CoreConfig {
                app: ngterm_proto::AppIdentity::from_recorded("1.2.3", None),
                capabilities: Vec::new(),
            },
            ngterm_core::CoreState {
                library,
                ..Default::default()
            },
        );
        let posted = Arc::new(Posted::default());
        let main_loop: Arc<dyn ngterm_host::MainLoop<AppState>> = posted.clone();
        Self {
            state: AppState::new(core),
            window,
            session,
            posted,
            main_loop,
        }
    }

    pub(crate) fn poster(&self) -> Poster {
        Arc::downgrade(&self.main_loop)
    }

    /// How many posted tasks wait for [`Model::drain`].
    pub(crate) fn pending(&self) -> usize {
        self.posted.0.lock().unwrap().len()
    }

    pub(crate) fn drain(&mut self) {
        let tasks = std::mem::take(&mut *self.posted.0.lock().unwrap());
        for task in tasks {
            task(&mut self.state);
        }
    }

    pub(crate) fn overlay(&self) -> &HtmlOverlay {
        self.state
            .core
            .library()
            .store(self.window)
            .unwrap()
            .html_overlay(self.session, None)
            .unwrap()
    }

    /// Whether the session's overlays are hidden.
    pub(crate) fn session_overlay_hidden(&self) -> bool {
        self.state
            .core
            .library()
            .store(self.window)
            .unwrap()
            .session(self.session)
            .unwrap()
            .overlay_hidden(None)
    }
}

#[gpui_kit::test]
fn settle_releases_pages_and_updates_zoom_only_when_it_changes(cx: &mut TestAppContext) {
    let web = replies();
    let overlay = url_overlay();
    let mut model = Model::new(&overlay);
    model.state.core.set_html_host(Box::new(PageHost));
    let page = register(&overlay, Storage::Ephemeral);
    model.state.core.commit();
    realize_waiting();
    let view = page.view_id().unwrap();
    cx.update(|cx| {
        settle(&mut model.state.core, cx);
        settle(&mut model.state.core, cx);
    });
    assert_eq!(*web.zooms.borrow(), [1.0]);
    assert!(model.state.core.close_html_overlay(overlay.id));
    model.state.core.commit();
    cx.update(|cx| settle(&mut model.state.core, cx));
    assert!(web.recorder.take().contains(&Call::CloseView(view)));
    assert_eq!(web.dropped.borrow().len(), 1);
    assert_eq!(
        PageHost.navigate(overlay.id, HtmlNavigation::Back),
        Err("html overlay not realized".to_owned())
    );
}

#[test]
fn an_open_saved_page_refuses_a_clear_even_when_it_is_not_shown() {
    let _web = replies();
    register(&url_overlay(), Storage::Saved);
    assert_eq!(clear_saved_store(), Err("1 persistent page still open".to_owned()));
}

#[test]
fn the_core_link_answers_requests_after_the_origin_has_gone() {
    let web = replies();
    let source = url_overlay();
    let mut model = Model::new(&source);
    let dir = tempfile::tempdir().unwrap();
    let poster = model.poster();
    install(&mut model.state.core, dir.path().to_owned(), poster);
    let link = replica_link().unwrap();
    let id = Uuid::new_v4();
    link.fetch(id, "/a.html".to_owned(), SchemeToken::from_raw(21));
    link.bridge(id, "{}".to_owned(), park(ReplyToken::from_raw(22)));
    model.drain();
    assert_eq!(
        *web.replies.borrow(),
        [(ReplyToken::from_raw(22), Err("page closed".to_owned()))]
    );
    assert_eq!(
        *web.resources.borrow(),
        [(
            SchemeToken::from_raw(21),
            SchemeResponse::Error {
                kind: SchemeError::Failed,
                message: "the page's file is on another Mac".to_owned(),
            }
        )]
    );
}

#[test]
fn the_document_host_checks_real_files_and_reports_invalid_documents() {
    reset();
    let dir = tempfile::tempdir().unwrap();
    let markdown = dir.path().join("a.md");
    std::fs::write(&markdown, "# heading").unwrap();
    assert_eq!(PageHost.check_markdown(markdown.to_str().unwrap()), Ok(()));
    assert!(
        PageHost
            .check_markdown(dir.path().join("missing.md").to_str().unwrap())
            .is_err()
    );
    assert!(PageHost.probe_image(markdown.to_str().unwrap()).is_err());
    let image = dir.path().join("a.png");
    image::RgbaImage::from_pixel(2, 3, image::Rgba([10, 20, 30, 255]))
        .save(&image)
        .unwrap();
    let probe = PageHost.probe_image(image.to_str().unwrap()).unwrap();
    assert_eq!((probe.width, probe.height), (2, 3));
}

#[gpui_kit::test]
fn unmounted_frames_and_popups_are_answered_without_a_page(cx: &mut TestAppContext) {
    let web = replies();
    let view = ViewId::from_raw(100);
    let frame = Event::Frame(ngterm_wpe::Frame {
        view,
        buffer: ngterm_wpe::BufferId::from_raw(23),
        width: 1,
        height: 1,
        damage: Vec::new(),
        content: ngterm_wpe::FrameContent::Shm(Vec::new()),
    });
    cx.update(|cx| {
        dispatch(&frame, cx);
        dispatch(
            &Event::OptionMenu(ngterm_wpe::OptionMenu {
                token: MenuToken::from_raw(24),
                view,
                area: ngterm_wpe::Area {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                },
                items: Vec::new(),
            }),
            cx,
        );
        dispatch(
            &Event::DownloadRefused {
                view: Some(view),
                session: SessionId::from_raw(1),
                uri: "https://example.invalid/download".to_owned(),
            },
            cx,
        );
    });
    assert_eq!(web.recorder.presented(), [ngterm_wpe::BufferId::from_raw(23)]);
    assert_eq!(web.recorder.take(), [Call::Choose(MenuToken::from_raw(24), None)]);
}
