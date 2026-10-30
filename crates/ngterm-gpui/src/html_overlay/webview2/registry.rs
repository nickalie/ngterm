//! The pages by id, the `WebView2` environment and the stores pages are built on, the page requests waiting on an
//! answer, and the pump (upstream's `HtmlOverlayRegistry`; the WPE host's `registry.rs` on `WebView2`). It lives on
//! GPUI's main thread; every borrow is short and never spans a call into a page or a web view, which may come back
//! here.
//!
//! The pump ([`init`]) is a task on the main thread that takes the [`Job`]s pages, their web views and the core post:
//! it makes each page's web view, moves, shows and focuses them, closes released ones and clears the saved store, all
//! outside GPUI's callbacks (making a web view pumps the window's messages, a focus change sends the window messages
//! at once, either of which would reach GPUI while its state is borrowed), and hands each web view's reports to its
//! page inside one. The runtime is checked at the start ([`install`], `runtime`); without it every page is refused.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::{AnyWindowHandle, App, AppContext as _, AsyncApp, ClipboardItem, Entity, Window};
use ngterm_core::Core;
use ngterm_core::html::bridge;
use ngterm_core::html::profile::{self, BrowserProfile};
use ngterm_core::html::{ClearProgress, HtmlHost, HtmlNavigation, HtmlOverlay, HtmlSource, StoreClear, errors};
use ngterm_core::remote::replica_page::PageResource;
use ngterm_proto::Response;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use uuid::Uuid;
use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Environment;
use wry::RequestAsyncResponder;

use super::super::panel::{PanelColors, TerminalColors};
use super::super::policy;
use super::native::{self, Native, Report, Spec, Store};
use super::page::{Page, Storage};
use super::view::PageView;
use super::{runtime, store};
use crate::state::{AppState, Poster, post};

/// Why every page fails once `WebView2`'s browser process ended while the app ran.
const ENDED: &str = "the web engine ended unexpectedly; restart ngterm to show pages again";
/// Why a page has no web view: the app keeps no state directory (it always does; a test's registry may not).
const NO_STATE: &str = "pages need a state directory";

/// What the pump does next (module docs). Every job can be sent from any thread: a protocol's read comes from one.
pub enum Job {
    /// Make page `id`'s web view.
    Build(Uuid),
    /// Bring page `id`'s native window to its wanted place.
    Sync(Uuid),
    /// A web view's report about page `id`.
    Report(Uuid, Report),
    /// A read of page `page`'s replica protocol: the app's address and the responder.
    ReplicaRead {
        page: Uuid,
        uri: String,
        reply: RequestAsyncResponder,
    },
    /// Close the released pages' web views.
    Bury,
    /// Empty the saved store.
    Clear,
    /// The saved store's clear ended.
    Cleared(Result<(), String>),
}

#[derive(Default)]
struct Registry {
    pages: HashMap<Uuid, Rc<Page>>,
    /// Where the browser profile and the stores live.
    state_dir: Option<PathBuf>,
    /// Why no page can run: no runtime, or `WebView2`'s browser process ended.
    failure: Option<String>,
    /// The environment every page shares once the first web view made one.
    environment: Option<ICoreWebView2Environment>,
    /// The profile folders closed pages left behind in an earlier run were removed.
    swept: bool,
    /// The page zoom every page shows at; `None` before the first settle (actual size).
    zoom: Option<f64>,
    /// The saved store's profile once the browser profile is read.
    saved: Option<String>,
    clearing: bool,
    clear_failure: Option<String>,
    /// The page requests waiting on the core's answer, by the token the core is handed: the page and its request.
    replies: HashMap<u64, (Uuid, u64)>,
    next_reply: u64,
    /// The replica reads waiting on the machine that opened their page.
    reads: HashMap<u64, RequestAsyncResponder>,
    next_read: u64,
    /// The terminal's colors the pages are drawn in.
    colors: TerminalColors,
    poster: Option<Poster>,
    jobs: Option<async_channel::Sender<Job>>,
    /// The windows a layer the app draws covers, and every window's native handle (the clear's hidden web view sits
    /// in one).
    covered: HashMap<AnyWindowHandle, bool>,
    windows: HashMap<AnyWindowHandle, isize>,
    /// Released pages' web views, closed by the pump, and the in-memory profile folders they leave.
    graveyard: Vec<(Rc<Native>, Option<PathBuf>)>,
}

thread_local! {
    static REGISTRY: RefCell<Registry> = RefCell::default();
}

fn with<R>(change: impl FnOnce(&mut Registry) -> R) -> R {
    REGISTRY.with(|registry| change(&mut registry.borrow_mut()))
}

fn page(id: Uuid) -> Option<Rc<Page>> {
    with(|registry| registry.pages.get(&id).cloned())
}

fn send(job: Job) {
    if let Some(jobs) = with(|registry| registry.jobs.clone()) {
        let _ = jobs.try_send(job);
    }
}

/// The core's way to the pages (upstream's registry calls in `ControlServer+SessionActions.swift`).
struct PageHost;

impl HtmlHost for PageHost {
    fn web_views_unavailable(&self) -> Option<String> {
        with(|registry| registry.failure.clone())
    }

    fn persistent_store_failure(&self) -> Option<String> {
        persistent_store_failure()
    }

    fn navigate(&self, id: Uuid, navigation: HtmlNavigation) -> Result<(), String> {
        navigate(id, navigation)
    }

    fn clear_saved_store(&self) -> Result<StoreClear, String> {
        clear_saved_store()
    }

    fn saved_store_clear(&self) -> ClearProgress {
        with(|registry| {
            if registry.clearing {
                ClearProgress::Running
            } else {
                registry
                    .clear_failure
                    .take()
                    .map_or(ClearProgress::Done, ClearProgress::Failed)
            }
        })
    }
}

/// Makes the registry the core's page host, its stores kept under `state_dir`, its reports posted through `poster`
/// (upstream's `install` and `profile`), and checks for the `WebView2` runtime: without it every page is refused,
/// with the reason logged now.
pub fn install(core: &mut Core, state_dir: PathBuf, poster: Poster) {
    let failure = match runtime::installed_version() {
        Ok(version) => {
            tracing::info!(version, "the WebView2 runtime");
            None
        }
        Err(failure) => {
            tracing::warn!(failure, "HTML pages are not available");
            Some(failure)
        }
    };
    with(|registry| {
        registry.state_dir = Some(state_dir);
        registry.poster = Some(poster);
        registry.failure = failure;
    });
    core.set_html_host(Box::new(PageHost));
}

/// Spawns the pump (module docs).
pub fn init(cx: &mut App) {
    let (jobs, queue) = async_channel::unbounded::<Job>();
    with(|registry| registry.jobs = Some(jobs));
    cx.spawn(async move |cx| {
        while let Ok(job) = queue.recv().await {
            run(job, cx);
        }
    })
    .detach();
}

/// One of the pump's jobs: the web views' work outside GPUI's callbacks, the pages' inside one.
fn run(job: Job, cx: &mut AsyncApp) {
    match job {
        Job::Build(id) => build(id),
        Job::Sync(id) => {
            if let Some(page) = page(id) {
                page.sync_native();
            }
        }
        Job::Report(id, report) => cx.update(|cx| {
            if let Some(page) = page(id) {
                page.report(report, cx);
            }
        }),
        Job::ReplicaRead { page, uri, reply } => cx.update(|_| replica_read(page, &uri, reply)),
        Job::Bury => {
            let buried = with(|registry| std::mem::take(&mut registry.graveyard));
            for (native, folder) in buried {
                drop(native);
                if let Some(folder) = folder {
                    store::remove_later(folder);
                }
            }
        }
        Job::Clear => clear(),
        Job::Cleared(result) => with(|registry| {
            registry.clearing = false;
            if let Err(err) = &result {
                tracing::warn!(err, "WebView2 did not empty the saved browser store");
            }
            registry.clear_failure = result.err();
        }),
    }
}

/// Ends the pages at the quit. Their web views are let go, not closed: a close sends the window messages while the
/// app is quitting inside GPUI's callbacks, and `WebView2`'s processes end with the app's.
pub fn shutdown() {
    let (pages, buried) = with(|registry| {
        registry.failure = Some("the app is quitting".to_owned());
        (
            std::mem::take(&mut registry.pages),
            std::mem::take(&mut registry.graveyard),
        )
    });
    std::mem::forget(pages);
    std::mem::forget(buried);
}

/// `WebView2`'s browser process ended while the app ran: every page fails, for good, and a clear in flight with them.
pub fn engine_ended() {
    let pages = with(|registry| {
        if registry.failure.is_some() {
            return None;
        }
        registry.failure = Some(ENDED.to_owned());
        if registry.clearing {
            registry.clearing = false;
            registry.clear_failure = Some(ENDED.to_owned());
        }
        Some(registry.pages.values().cloned().collect::<Vec<_>>())
    });
    let Some(pages) = pages else { return };
    tracing::error!("{ENDED}");
    for page in pages {
        page.engine_failed(ENDED);
    }
}

// MARK: - web views

/// The native handle of GPUI window `window`.
fn native_window(window: &Window) -> Option<isize> {
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
        _ => None,
    }
}

/// Asks the pump for `page`'s web view, unless it has one, is getting one, or cannot have one.
fn request_build(page: &Page) {
    if let Some(failure) = with(|registry| registry.failure.clone()) {
        page.engine_failed(&failure);
        return;
    }
    if page.wants_native() {
        page.set_building(true);
        send(Job::Build(page.id()));
    }
}

/// Makes page `id`'s web view (the pump, outside GPUI's callbacks).
fn build(id: Uuid) {
    let Some(page) = page(id) else { return };
    let spec = match spec(&page) {
        Ok(spec) => spec,
        Err(failure) => {
            page.set_building(false);
            page.engine_failed(&failure);
            return;
        }
    };
    match Native::create(spec) {
        Ok(native) => {
            let environment = native.environment();
            with(|registry| {
                registry.environment.get_or_insert(environment);
            });
            page.realized(native);
        }
        Err(failure) => {
            tracing::warn!(page = %id, failure, "WebView2 could not make the page's web view");
            page.set_building(false);
            page.engine_failed(&failure);
        }
    }
}

/// The store `profile` lives in (`private`: in memory); the first one sweeps the profile folders closed pages left.
fn store_for(profile: String, private: bool) -> Result<Store, String> {
    let (state_dir, environment, swept) = with(|registry| {
        let swept = std::mem::replace(&mut registry.swept, true);
        (registry.state_dir.clone(), registry.environment.clone(), swept)
    });
    let user_data = store::user_data_folder(&state_dir.ok_or_else(|| NO_STATE.to_owned())?);
    if !swept && environment.is_none() {
        for folder in store::page_profile_folders(&user_data) {
            let _ = std::fs::remove_dir_all(folder);
        }
    }
    Ok(Store {
        environment,
        user_data,
        profile,
        private,
    })
}

/// What page `page`'s web view is made from.
fn spec(page: &Rc<Page>) -> Result<Spec, String> {
    let overlay = page.overlay();
    let profile = match page.storage() {
        Storage::Unavailable(failure) => return Err(failure.clone()),
        Storage::Saved => {
            with(|registry| registry.saved.clone()).ok_or_else(|| errors::PERSISTENT_UNAVAILABLE.to_owned())?
        }
        Storage::Ephemeral => store::page_profile(page.id()),
    };
    let store = store_for(profile, *page.storage() == Storage::Ephemeral)?;
    let placement = page.wanted();
    let parent = placement.parent.ok_or_else(|| "the page has no window".to_owned())?;
    let jobs = with(|registry| registry.jobs.clone()).ok_or_else(|| "the pages have no pump".to_owned())?;
    let id = page.id();
    let weak = Rc::downgrade(page);
    let reports = jobs.clone();
    let replica = matches!(overlay.source, HtmlSource::Replica { .. }).then(|| {
        let jobs = jobs.clone();
        Arc::new(move |uri: String, reply: RequestAsyncResponder| {
            let _ = jobs.try_send(Job::ReplicaRead { page: id, uri, reply });
        }) as native::ReplicaRead
    });
    let granted = matches!(
        overlay.source,
        HtmlSource::File {
            grant_root: Some(_),
            ..
        }
    );
    Ok(Spec {
        parent,
        store,
        javascript: overlay.flags.javascript,
        background: page.canvas(),
        zoom: with(|registry| registry.zoom.unwrap_or(1.0)),
        bounds: placement.bounds,
        visible: false,
        theme: page.theme().script(overlay.themed()),
        scripts: page.scripts(),
        grant: granted.then(|| page.grant().clone()),
        replica,
        decide: Rc::new(move |navigation| weak.upgrade().is_some_and(|page| page.allows(navigation))),
        post: Rc::new(move |report| {
            let _ = reports.try_send(Job::Report(id, report));
        }),
        take_key: Rc::new(super::keys::app_key),
    })
}

/// Page `id`'s native window moves at the pump's next turn.
pub fn sync_later(id: Uuid) {
    send(Job::Sync(id));
}

/// A released page's web view, closed by the pump, and the in-memory profile folder it leaves.
pub fn bury(native: Rc<Native>, folder: Option<PathBuf>) {
    with(|registry| registry.graveyard.push((native, folder)));
    send(Job::Bury);
}

/// Where page `id`'s in-memory profile keeps its folder.
pub fn page_profile_folder(id: Uuid) -> Option<PathBuf> {
    let state_dir = with(|registry| registry.state_dir.clone())?;
    Some(store::profile_folder(
        &store::user_data_folder(&state_dir),
        &store::page_profile(id),
    ))
}

/// Whether a layer the app draws covers `window` (`set_covered`).
pub fn is_covered(window: AnyWindowHandle) -> bool {
    with(|registry| registry.covered.get(&window).copied().unwrap_or(false))
}

/// Whether a layer the app draws (a dialog, the palette, a popover, the switcher, an alert) covers `window`'s content:
/// its pages' native windows, which sit above everything the window draws, step aside meanwhile. Called by the
/// window's view at every render.
pub fn set_covered(window: &Window, covered: bool, _: &mut App) {
    let handle = window.window_handle();
    let native = native_window(window);
    let changed = with(|registry| {
        if let Some(native) = native {
            registry.windows.insert(handle, native);
        }
        registry.covered.insert(handle, covered) != Some(covered)
    });
    if !changed {
        return;
    }
    let pages: Vec<Rc<Page>> = with(|registry| registry.pages.values().cloned().collect());
    for page in pages.into_iter().filter(|page| page.element_window() == Some(handle)) {
        page.placement_changed();
    }
}

// MARK: - mounting and the model's value

/// The element of page `overlay`, made with the page on first use and loading (upstream's
/// `page(for:store:backgroundColor:)`); `background` is the overlay's own background, which replaces the theme's.
pub fn mount(
    overlay: &HtmlOverlay,
    background: Option<&str>,
    window: &mut Window,
    cx: &mut App,
) -> Option<Entity<PageView>> {
    let page = page(overlay.id).unwrap_or_else(|| {
        let (colors, poster) = with(|registry| (registry.colors.clone(), registry.poster.clone()));
        let poster = poster.unwrap_or_else(crate::state::detached_poster);
        let page = Page::new(
            overlay,
            background,
            PanelColors::resolve(&colors, background),
            storage(overlay),
            &poster,
        );
        with(|registry| registry.pages.insert(overlay.id, Rc::clone(&page)));
        page
    });
    // a session moved to another window gets an element there: an element's focus is its window's
    if let Some(element) = page
        .element()
        .filter(|_| page.element_window() == Some(window.window_handle()))
    {
        return Some(element);
    }
    let native = native_window(window)?;
    with(|registry| registry.windows.insert(window.window_handle(), native));
    let element = cx.new(|cx| PageView::new(Rc::clone(&page), native, window, cx));
    request_build(&page);
    Some(element)
}

/// Gives page `overlay.id` the model's latest value and whether it is on screen (upstream's `updateNSView`).
pub fn apply(overlay: &HtmlOverlay, on_screen: bool, cx: &mut App) {
    if let Some(page) = page(overlay.id) {
        page.apply(overlay, on_screen, cx);
    }
}

/// After a commit: the pages that left every slot release their web views (upstream's `release`), and every page
/// follows the page zoom setting (upstream's `setZoom`).
pub fn settle(core: &mut Core, cx: &mut App) {
    for id in core.take_released_pages() {
        if let Some(page) = with(|registry| registry.pages.remove(&id)) {
            page.close(cx);
        }
    }
    let zoom = core.html_zoom();
    let changed = with(|registry| {
        let changed = registry.zoom.map(f64::to_bits) != Some(zoom.to_bits());
        registry.zoom = Some(zoom);
        changed.then(|| registry.pages.values().cloned().collect::<Vec<_>>())
    });
    for page in changed.unwrap_or_default() {
        page.set_zoom(zoom);
    }
}

/// Every page follows the terminal's `colors` (upstream's `refreshThemes`).
pub fn refresh_themes(colors: &TerminalColors, cx: &mut App) {
    let pages = with(|registry| {
        registry.colors = colors.clone();
        registry.pages.values().cloned().collect::<Vec<_>>()
    });
    for page in pages {
        page.apply_theme(colors, cx);
    }
}

/// A history step or hand-off on page `id` (upstream's `navigate`), shared by the strip and the control API.
pub fn navigate(id: Uuid, navigation: HtmlNavigation) -> Result<(), String> {
    let page = page(id).ok_or_else(|| errors::NOT_REALIZED.to_owned())?;
    if page.native().is_none() {
        return Err(errors::NOT_REALIZED.to_owned());
    }
    page.navigate(navigation)
}

/// Puts page `id`'s address on the clipboard (upstream's `copyLink`).
pub fn copy_link(id: Uuid, cx: &mut App) {
    if let Some(link) = page(id).and_then(|page| page.browser_url()) {
        cx.write_to_clipboard(ClipboardItem::new_string(link.as_str().to_owned()));
    }
}

// MARK: - browser stores

/// Why the saved store cannot be used, `None` when it can, reading the profile on first use (upstream's
/// `persistentStoreFailure`): its profile is named by the browser profile id (`store`).
fn persistent_store_failure() -> Option<String> {
    let (known, clearing, state_dir) =
        with(|registry| (registry.saved.is_some(), registry.clearing, registry.state_dir.clone()));
    if clearing {
        return Some(errors::CLEARING.to_owned());
    }
    if known {
        return None;
    }
    let Some(state_dir) = state_dir else {
        return Some(errors::PERSISTENT_UNAVAILABLE.to_owned());
    };
    let id = match BrowserProfile::new(&state_dir).identifier() {
        Ok(id) => id,
        Err(failure) => return Some(failure.to_string()),
    };
    with(|registry| registry.saved = Some(store::saved_profile(id)));
    None
}

/// Starts removing everything the saved store holds (upstream's `clearPersistentStore`): refused while a clear runs,
/// while a page built on the store is still registered, and when the profile cannot be read; a profile never created
/// has nothing to remove and is not created for it. The pump clears it through a hidden web view on its profile.
fn clear_saved_store() -> Result<StoreClear, String> {
    let (clearing, open, state_dir, failure) = with(|registry| {
        let open = registry.pages.values().filter(|page| page.uses_saved_store()).count();
        (
            registry.clearing,
            open,
            registry.state_dir.clone(),
            registry.failure.clone(),
        )
    });
    let profile = state_dir.as_deref().map(BrowserProfile::new);
    if !profile::clear_check(clearing, open, profile.as_ref())? {
        return Ok(StoreClear::Nothing);
    }
    if let Some(failure) = persistent_store_failure() {
        return Err(failure);
    }
    if failure.is_some() || with(|registry| registry.windows.is_empty() || registry.jobs.is_none()) {
        return Err(errors::PERSISTENT_UNAVAILABLE.to_owned());
    }
    with(|registry| {
        registry.clearing = true;
        registry.clear_failure = None;
    });
    send(Job::Clear);
    Ok(StoreClear::Started)
}

/// The pump's clear of the saved store, in a window's hidden web view.
fn clear() {
    let (parent, profile) = with(|registry| (registry.windows.values().next().copied(), registry.saved.clone()));
    let jobs = with(|registry| registry.jobs.clone());
    let (Some(parent), Some(profile), Some(jobs)) = (parent, profile, jobs) else {
        run_cleared(Err(errors::PERSISTENT_UNAVAILABLE.to_owned()));
        return;
    };
    let store = match store_for(profile, false) {
        Ok(store) => store,
        Err(failure) => return run_cleared(Err(failure)),
    };
    let done = Box::new(move |result| {
        let _ = jobs.try_send(Job::Cleared(result));
    });
    if let Some(environment) = native::clear_profile(parent, store, done) {
        with(|registry| {
            registry.environment.get_or_insert(environment);
        });
    }
}

fn run_cleared(result: Result<(), String>) {
    send(Job::Cleared(result));
}

/// The store page `overlay` is built on (upstream's `storage(for:)`): a persistent URL page the saved one, or none
/// when it cannot be used, since an in-memory one would hold a login the user asked to keep; every other page an
/// in-memory one of its own.
fn storage(overlay: &HtmlOverlay) -> Storage {
    if !(overlay.flags.persistent && matches!(overlay.source, HtmlSource::Url(_))) {
        return Storage::Ephemeral;
    }
    match persistent_store_failure() {
        Some(failure) => Storage::Unavailable(failure),
        None => Storage::Saved,
    }
}

// MARK: - page requests

/// Keeps page `page`'s bridge request `request` until its answer; the token names it.
pub fn park(page: Uuid, request: u64) -> u64 {
    with(|registry| {
        registry.next_reply += 1;
        let token = registry.next_reply;
        registry.replies.insert(token, (page, request));
        token
    })
}

/// Answers the page request `token` with `response` (upstream's `HtmlOverlayBridge.reply`).
pub fn answer(token: u64, response: &Response) {
    finish(token, bridge::reply(response));
}

/// Rejects the page request `token` with `message`.
pub fn reject(token: u64, message: &str) {
    finish(token, Err(message.to_owned()));
}

fn finish(token: u64, answer: Result<serde_json::Value, String>) {
    let Some((id, request)) = with(|registry| registry.replies.remove(&token)) else {
        return;
    };
    if let Some(page) = page(id) {
        page.reply(request, answer);
    }
}

// MARK: - pages shown for another machine

/// Relays replica `page`'s request through the core's page channel, answered by the parked `reply`.
pub fn relay(page: Uuid, body: String, reply: u64) {
    let Some(poster) = with(|registry| registry.poster.clone()) else {
        reject(reply, "page closed");
        return;
    };
    post(&poster, move |state: &mut AppState| {
        state
            .core
            .replica_bridge(page, &body, Box::new(move |result| finish(reply, result)));
        state.settle();
    });
}

/// A read of the replica protocol (upstream's `HtmlPageSchemeHandler`): served only for the replica page whose id the
/// address names as its host, from the file of that path on the machine that opened it; a read naming another page,
/// or from a page that is no replica, fails without reaching that machine.
fn replica_read(page_id: Uuid, uri: &str, reply: RequestAsyncResponder) {
    let path = page(page_id).and_then(|page| {
        matches!(page.overlay().source, HtmlSource::Replica { .. })
            .then(|| policy::replica_read(uri, page_id))
            .flatten()
    });
    let poster = with(|registry| registry.poster.clone());
    let (Some(path), Some(poster)) = (path, poster) else {
        respond(reply, Err(policy::NOT_THIS_PAGE.to_owned()));
        return;
    };
    let token = with(|registry| {
        registry.next_read += 1;
        registry.reads.insert(registry.next_read, reply);
        registry.next_read
    });
    post(&poster, move |state: &mut AppState| {
        state
            .core
            .replica_fetch(page_id, &path, Box::new(move |result| answer_read(token, result)));
        state.settle();
    });
}

/// Answers the replica read `token`: its body whole, or the failure its load reports.
fn answer_read(token: u64, result: Result<PageResource, String>) {
    if let Some(reply) = with(|registry| registry.reads.remove(&token)) {
        respond(reply, result.map(|resource| (resource.mime_type, resource.body)));
    }
}

fn respond(reply: RequestAsyncResponder, answer: Result<(String, Vec<u8>), String>) {
    let response = match answer {
        Ok((mime, body)) => wry::http::Response::builder()
            .status(200)
            .header("Content-Type", mime)
            .body(body),
        Err(message) => wry::http::Response::builder()
            .status(403)
            .header("Content-Type", "text/plain; charset=utf-8")
            .body(message.into_bytes()),
    };
    if let Ok(response) = response {
        reply.respond(response);
    }
}

// MARK: - the keyboard

/// A key the app's interceptor hands on while a page's element holds `window`'s keyboard but its web view does not
/// yet: the web view takes the keyboard for the keys after it. Whether one did.
pub fn key_down(
    _keystroke: &gpui_kit::Keystroke,
    _stroke: &crate::terminal::input::KeyStroke,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    let pages: Vec<Rc<Page>> = with(|registry| registry.pages.values().cloned().collect());
    let focused = pages.into_iter().find(|page| {
        page.element()
            .is_some_and(|element| element.read(cx).focus().is_focused(window))
    });
    let Some(page) = focused else { return false };
    if !page.holds_native_keyboard() {
        page.placement_changed();
        sync_later(page.id());
    }
    true
}
