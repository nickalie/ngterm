//! The web view's stand-in for the unit tests of `page` and `registry` (the `NativeView` seam): [`FakeView`] records what
//! a page asks of its web view, [`FakeEngine`] makes them, keeps what each was built from and answers the store's clear,
//! and [`Model`] is the core a page's posted reports land in, run by the test.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_channel::Receiver;
use gpui_kit::TestAppContext;
use ngterm_core::Core;
use ngterm_core::html::{HtmlOverlay, HtmlPageFlags, HtmlSource};
use uuid::Uuid;

use super::super::text_page::Answer;
use super::native::{Bounds, Environment, Spec, Store};
use super::native_view::NativeView;
use crate::state::{AppState, Poster};

/// What a page asked of its web view.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Call {
    Theme(String),
    Load(String),
    Reload,
    Back,
    Forward,
    Zoom(f64),
    Background((u8, u8, u8)),
    Bounds(Bounds),
    Visible(bool),
    Focus,
    FocusParent,
    Reparent(isize),
    Post(String),
    Renderer(String),
    #[cfg(windows)]
    Redelivered(Vec<serde_json::Value>),
}

/// One web view as the test watches it: what it was asked, and whether its last holder let it go.
#[derive(Clone, Default)]
pub(super) struct Probe {
    calls: Rc<RefCell<Vec<Call>>>,
    answer: Rc<RefCell<Option<Answer>>>,
    dropped: Rc<Cell<bool>>,
}

impl Probe {
    /// The calls since the last take.
    pub(super) fn take(&self) -> Vec<Call> {
        std::mem::take(&mut *self.calls.borrow_mut())
    }

    /// `call_renderer` answers with `answer` from here on; without one it never answers.
    pub(super) fn answer_with(&self, answer: Answer) {
        self.answer.replace(Some(answer));
    }

    /// The web view was dropped, as the pump's burial does.
    pub(super) fn dropped(&self) -> bool {
        self.dropped.get()
    }
}

/// A web view that records (module docs).
pub(super) struct FakeView(Probe);

impl FakeView {
    /// A web view and the probe watching it.
    pub(super) fn made() -> (Rc<dyn NativeView>, Probe) {
        let probe = Probe::default();
        (Rc::new(Self(probe.clone())), probe)
    }

    fn record(&self, call: Call) {
        self.0.calls.borrow_mut().push(call);
    }
}

impl Drop for FakeView {
    fn drop(&mut self) {
        self.0.dropped.set(true);
    }
}

impl NativeView for FakeView {
    fn set_theme(&self, script: &str) {
        self.record(Call::Theme(script.to_owned()));
    }
    fn load_url(&self, address: &str) {
        self.record(Call::Load(address.to_owned()));
    }
    fn reload(&self) {
        self.record(Call::Reload);
    }
    fn go_back(&self) {
        self.record(Call::Back);
    }
    fn go_forward(&self) {
        self.record(Call::Forward);
    }
    fn set_zoom(&self, zoom: f64) {
        self.record(Call::Zoom(zoom));
    }
    fn set_background(&self, color: (u8, u8, u8)) {
        self.record(Call::Background(color));
    }
    fn set_bounds(&self, bounds: Bounds) {
        self.record(Call::Bounds(bounds));
    }
    fn set_visible(&self, visible: bool) {
        self.record(Call::Visible(visible));
    }
    fn focus(&self) {
        self.record(Call::Focus);
    }
    fn focus_parent(&self) {
        self.record(Call::FocusParent);
    }
    fn reparent(&self, parent: isize) {
        self.record(Call::Reparent(parent));
    }
    fn post_json(&self, json: &str) {
        self.record(Call::Post(json.to_owned()));
    }
    #[cfg(windows)]
    fn redeliver_key(&self, events: &[serde_json::Value]) {
        self.record(Call::Redelivered(events.to_vec()));
    }
    fn call_renderer(&self, expression: &str) -> Receiver<Answer> {
        self.record(Call::Renderer(expression.to_owned()));
        let (sender, answer) = async_channel::bounded(1);
        if let Some(reply) = self.0.answer.borrow().clone() {
            let _ = sender.try_send(reply);
        }
        answer
    }
}

type Done = Box<dyn FnOnce(Result<(), String>) + Send>;

/// The stand-in for `native::Native::create`, `native::clear_profile` and the system browser.
#[derive(Default)]
pub(super) struct FakeEngine {
    specs: RefCell<Vec<Rc<Spec>>>,
    probes: RefCell<Vec<Probe>>,
    /// The next web view's creation fails with this.
    failure: RefCell<Option<String>>,
    clears: RefCell<Vec<(isize, Store)>>,
    done: RefCell<Option<Done>>,
    /// The native handle GPUI's test windows do not have.
    window: Cell<Option<isize>>,
    opened: RefCell<Vec<String>>,
    browser_missing: Cell<bool>,
}

impl FakeEngine {
    pub(super) fn create(&self, spec: &Spec) -> Result<Rc<dyn NativeView>, String> {
        if let Some(failure) = self.failure.take() {
            return Err(failure);
        }
        self.specs.borrow_mut().push(Rc::new(Spec {
            parent: spec.parent,
            store: spec.store.clone(),
            javascript: spec.javascript,
            background: spec.background,
            zoom: spec.zoom,
            bounds: spec.bounds,
            visible: spec.visible,
            theme: spec.theme.clone(),
            scripts: spec.scripts.clone(),
            trusted_document_script: spec.trusted_document_script.clone(),
            text_channel: spec.text_channel,
            grant: spec.grant.clone(),
            replica: spec.replica.clone(),
            text: spec.text.clone(),
            block_network: spec.block_network,
            decide: Rc::clone(&spec.decide),
            post: Rc::clone(&spec.post),
            take_key: Rc::clone(&spec.take_key),
        }));
        let (view, probe) = FakeView::made();
        self.probes.borrow_mut().push(probe);
        Ok(view)
    }

    pub(super) fn clear_profile(&self, parent: isize, store: Store, done: Done) -> Option<Environment> {
        self.clears.borrow_mut().push((parent, store));
        self.done.replace(Some(done));
        None
    }

    pub(super) fn open_browser(&self, uri: &str) -> bool {
        self.opened.borrow_mut().push(uri.to_owned());
        !self.browser_missing.get()
    }

    /// The next web view's creation fails with `failure`.
    pub(super) fn fail_next(&self, failure: &str) {
        self.failure.replace(Some(failure.to_owned()));
    }

    /// Windows have native handle `handle`.
    pub(super) fn windows_have(&self, handle: isize) {
        self.window.set(Some(handle));
    }

    /// There is no default web browser.
    pub(super) fn no_browser(&self) {
        self.browser_missing.set(true);
    }

    /// The web views made so far.
    pub(super) fn made(&self) -> usize {
        self.specs.borrow().len()
    }

    /// What web view `index` was built from.
    pub(super) fn spec(&self, index: usize) -> Rc<Spec> {
        Rc::clone(&self.specs.borrow()[index])
    }

    /// Web view `index`'s probe.
    pub(super) fn probe(&self, index: usize) -> Probe {
        self.probes.borrow()[index].clone()
    }

    /// The store clears started so far: the window and the store of each.
    pub(super) fn clears(&self) -> Vec<(isize, Store)> {
        self.clears.borrow().clone()
    }

    /// Finishes the clear in flight.
    pub(super) fn finish_clear(&self, result: Result<(), String>) {
        let done = self.done.take().expect("a clear is running");
        done(result);
    }

    /// The addresses handed to the browser.
    pub(super) fn opened(&self) -> Vec<String> {
        self.opened.borrow().clone()
    }
}

thread_local! {
    static ENGINE: RefCell<Option<Rc<FakeEngine>>> = const { RefCell::new(None) };
}

/// The engine standing in on this thread, when a test installed one.
pub(super) fn engine() -> Option<Rc<FakeEngine>> {
    ENGINE.with_borrow(Clone::clone)
}

/// Runs the registry on a fresh engine, which it returns, with nothing registered.
pub(super) fn install() -> Rc<FakeEngine> {
    super::registry::reset();
    let engine = Rc::new(FakeEngine::default());
    ENGINE.with_borrow_mut(|slot| *slot = Some(Rc::clone(&engine)));
    engine
}

/// The native handle of a test window, which has none of its own.
pub(super) fn window_handle(_: &gpui_kit::Window) -> Option<isize> {
    engine()?.window.get()
}

/// A loop whose tasks the test runs.
#[derive(Default)]
struct Posted(Mutex<Vec<ngterm_host::MainTask<AppState>>>);

impl ngterm_host::MainLoop<AppState> for Posted {
    fn post(&self, task: ngterm_host::MainTask<AppState>) -> Result<(), ngterm_host::MainLoopClosed> {
        self.0.lock().unwrap().push(task);
        Ok(())
    }

    fn post_after(&self, _: Duration, _: ngterm_host::MainTask<AppState>) -> Result<(), ngterm_host::MainLoopClosed> {
        Ok(())
    }
}

/// A core holding one overlay in one session, a page's posted reports waiting for [`Model::drain`].
pub(super) struct Model {
    pub(super) state: AppState,
    pub(super) window: Uuid,
    pub(super) session: Uuid,
    posted: Arc<Posted>,
    main_loop: Arc<dyn ngterm_host::MainLoop<AppState>>,
}

impl Model {
    /// A core showing `overlay`.
    pub(super) fn new(overlay: &HtmlOverlay) -> Self {
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

    pub(super) fn poster(&self) -> Poster {
        Arc::downgrade(&self.main_loop)
    }

    /// How many posted tasks wait for [`Model::drain`].
    pub(super) fn pending(&self) -> usize {
        self.posted.0.lock().unwrap().len()
    }

    /// Runs the posted tasks.
    pub(super) fn drain(&mut self) {
        let tasks = std::mem::take(&mut *self.posted.0.lock().unwrap());
        for task in tasks {
            task(&mut self.state);
        }
    }

    /// The overlay as the core holds it.
    pub(super) fn overlay(&self) -> &HtmlOverlay {
        self.state
            .core
            .library()
            .store(self.window)
            .unwrap()
            .html_overlay(self.session, None)
            .unwrap()
    }
}

/// `source` as a default page.
pub(super) fn overlay(source: HtmlSource) -> HtmlOverlay {
    HtmlOverlay::new(source, HtmlPageFlags::default())
}

/// A URL page.
pub(super) fn url_overlay() -> HtmlOverlay {
    overlay(HtmlSource::Url(
        url::Url::parse("https://example.invalid/start").unwrap(),
    ))
}

/// Runs the app's tasks, and the thread's own messages (`outside`'s timer), until `ready` holds.
#[track_caller]
pub(super) fn settle_until(cx: &TestAppContext, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        cx.run_until_parked();
        dispatch_messages();
        cx.run_until_parked();
        if ready() {
            return;
        }
        assert!(Instant::now() < deadline, "the page did not settle");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Dispatches the thread's waiting messages.
#[cfg(windows)]
fn dispatch_messages() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{DispatchMessageW, MSG, PM_REMOVE, PeekMessageW};
    let mut message = MSG::default();
    // SAFETY: a message buffer owned by this thread; only its own windows' and timers' messages are dispatched.
    while unsafe { PeekMessageW(&raw mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
        // SAFETY: the message just removed from this thread's queue.
        unsafe { DispatchMessageW(&raw const message) };
    }
}

#[cfg(not(windows))]
fn dispatch_messages() {}
