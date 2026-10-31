//! One page on `WebView2` and the handlers that keep the model in step with it (upstream's `HtmlOverlayPage`, the
//! WPE host's `page.rs` on `WebView2`'s events): load state, the page and title shown, history, the navigation
//! policy, the hand-off prompt, the bridge and the native window's place.
//!
//! A page is made at its mount and loads at once, shown or not; its web view exists once the registry's pump made it
//! (`registry`), which waits for nothing but `WebView2`. Every report to the core is a posted main-loop task, never a
//! call in place. The native window is placed from the element's layout and the model's value, but moved, shown,
//! hidden and focused only from the pump ([`Page::sync_native`]), outside GPUI's callbacks: a focus change sends the
//! GPUI window messages at once.
//!
//! `WebView2` names the frame of every navigation it asks about and asks again for each redirect, so the policy runs
//! once per navigation, before it starts, as on macOS (the WPE host checks the main frame's response as well, as
//! `WebKit`'s `GLib` ports name no frame). A refused main-frame navigation of a load in flight fails that load
//! (upstream's `loadPending`), one the page started over a shown document leaves it.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::sync::PoisonError;

use gpui_kit::{AnyWindowHandle, App, Entity, WeakEntity};
use ngterm_core::html::{
    HtmlLoadState, HtmlNavigation, HtmlNavigationAction, HtmlNavigationDecision, HtmlNavigationTarget, HtmlOverlay,
    HtmlOverlayTheme, HtmlPageInfo, HtmlReloadTarget, HtmlSource, decide, errors,
};
use url::Url;
use uuid::Uuid;
use webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED;

use super::super::panel::{PanelColors, TerminalColors};
use super::super::policy::{self, LoadEvent, LoadOutcome, LoadTracker, ThemeChange};
use super::native::{Bounds, Grant, Native, Navigation, Report, Text};
use super::view::PageView;
use super::{bridge, registry, scheme};
use crate::modal::{Alert, AlertButton, AlertSpec};
use crate::state::{AppState, Poster, post};

/// `WebView2`'s error status of a navigation another one superseded.
const OPERATION_CANCELED: i32 = COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED.0;

/// The browser store a page is built on, or why it has none and loads nothing (upstream's `Storage`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Storage {
    /// An in-memory profile of its own.
    Ephemeral,
    /// The saved store's profile.
    Saved,
    Unavailable(String),
}

/// What the web view last reported it shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Shown {
    /// The address, as the web view spells it.
    uri: Option<String>,
    title: Option<String>,
    can_go_back: bool,
    can_go_forward: bool,
}

/// What the page's handlers share.
struct PageState {
    /// The model's value as last applied.
    overlay: HtmlOverlay,
    applied_revision: u64,
    load: LoadTracker,
    /// The address of the document a browsing page shows; `None` until its first commit.
    shown_url: Option<Url>,
    storage_failure: Option<String>,
    prompt: Option<Entity<Alert>>,
    /// A declined prompt silences the page until real input reaches it: script can click links in a loop, but it
    /// cannot make trusted pointer or key events.
    prompts_silenced: bool,
    on_screen: bool,
    life: Life,
    engine: Shown,
    /// What this page last posted to the model as its page info (`policy::needs_report`).
    reported: Option<HtmlPageInfo>,
    /// The main-frame navigation the policy refused last, whose end is that refusal's.
    refused: Option<u64>,
    /// A text load the app started is the next main-frame navigation, at the text's address.
    text_pending: bool,
}

impl PageState {
    fn active(&self) -> bool {
        self.life != Life::Released
    }
}

/// Where a page is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Life {
    Active,
    /// The page's `window.close()` asked for its close; it stays asked while the session sits in an undoable close.
    CloseRequested,
    Released,
}

/// Where the native window should be, and where it is.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Placement {
    /// The window it is a child of.
    pub parent: Option<isize>,
    pub bounds: Option<Bounds>,
    pub visible: bool,
    /// It holds the keyboard.
    pub focused: bool,
}

/// One page (upstream's `HtmlOverlayPage`).
pub struct Page {
    id: Uuid,
    /// The overlay's own background (`--background-color`), which replaces the theme's.
    background: Option<String>,
    colors: RefCell<PanelColors>,
    /// The theme the page's scripts carry.
    theme: RefCell<HtmlOverlayTheme>,
    poster: Poster,
    state: RefCell<PageState>,
    storage: Storage,
    /// What ngterm's scripts in the page sign their messages with.
    nonce: String,
    grant: Grant,
    /// The text a file page without a grant shows.
    text: Text,
    native: RefCell<Option<Rc<Native>>>,
    /// The pump is making the web view.
    building: Cell<bool>,
    element: RefCell<Option<WeakEntity<PageView>>>,
    /// The window the element was made in: a session moving to another window gets a new element there.
    element_window: Cell<Option<AnyWindowHandle>>,
    /// The window the page was last drawn in, which its prompts open over.
    window: Cell<Option<AnyWindowHandle>>,
    /// The element's place and keyboard as last laid out, the native window's wanted place, and its place now.
    laid_out: Cell<Placement>,
    applied: Cell<Placement>,
    /// The web view holds the keyboard, as it last reported.
    native_focused: Cell<bool>,
    /// A file page reloads once its replaced theme script is in place.
    reload_after_theme: Cell<bool>,
    me: Weak<Page>,
}

impl Page {
    /// The page for `overlay` (its own `background` replacing the theme's), built on `storage`.
    pub fn new(
        overlay: &HtmlOverlay,
        background: Option<&str>,
        colors: PanelColors,
        storage: Storage,
        poster: &Poster,
    ) -> Rc<Self> {
        let storage_failure = match &storage {
            Storage::Unavailable(failure) => Some(failure.clone()),
            _ => None,
        };
        Rc::new_cyclic(|me| Self {
            id: overlay.id,
            background: background.map(str::to_owned),
            theme: RefCell::new(colors.theme()),
            colors: RefCell::new(colors),
            poster: poster.clone(),
            storage,
            nonce: bridge::nonce(),
            grant: Grant::default(),
            text: Text::default(),
            state: RefCell::new(PageState {
                overlay: overlay.clone(),
                applied_revision: overlay.reload_revision,
                load: LoadTracker::default(),
                shown_url: None,
                storage_failure,
                prompt: None,
                prompts_silenced: false,
                on_screen: false,
                life: Life::Active,
                engine: Shown::default(),
                reported: None,
                refused: None,
                text_pending: false,
            }),
            native: RefCell::new(None),
            building: Cell::new(false),
            element: RefCell::new(None),
            element_window: Cell::new(None),
            window: Cell::new(None),
            laid_out: Cell::new(Placement::default()),
            applied: Cell::new(Placement::default()),
            native_focused: Cell::new(false),
            reload_after_theme: Cell::new(false),
            me: me.clone(),
        })
    }

    pub fn id(&self) -> Uuid {
        self.id
    }

    pub fn poster(&self) -> &Poster {
        &self.poster
    }

    pub fn overlay(&self) -> HtmlOverlay {
        self.state.borrow().overlay.clone()
    }

    pub fn colors(&self) -> PanelColors {
        self.colors.borrow().clone()
    }

    pub fn storage(&self) -> &Storage {
        &self.storage
    }

    pub fn uses_saved_store(&self) -> bool {
        self.storage == Storage::Saved
    }

    pub fn on_screen(&self) -> bool {
        self.state.borrow().on_screen
    }

    pub fn active(&self) -> bool {
        self.state.borrow().active()
    }

    pub fn native(&self) -> Option<Rc<Native>> {
        self.native.borrow().clone()
    }

    /// Whether the page waits for its web view: active, with a store, and none made or being made.
    pub fn wants_native(&self) -> bool {
        let state = self.state.borrow();
        state.active() && state.storage_failure.is_none() && self.native.borrow().is_none() && !self.building.get()
    }

    pub fn set_building(&self, building: bool) {
        self.building.set(building);
    }

    /// The page's element is now `element`, made in `window`, whose native window is `parent`.
    pub fn set_element(&self, element: WeakEntity<PageView>, window: AnyWindowHandle, parent: isize) {
        self.element.replace(Some(element));
        self.element_window.set(Some(window));
        let mut laid_out = self.laid_out.get();
        laid_out.parent = Some(parent);
        self.laid_out.set(laid_out);
        self.placement_changed();
    }

    pub fn element_window(&self) -> Option<AnyWindowHandle> {
        self.element_window.get()
    }

    pub fn element(&self) -> Option<Entity<PageView>> {
        self.element.borrow().as_ref().and_then(WeakEntity::upgrade)
    }

    /// The page was drawn in `window`.
    pub fn drawn_in(&self, window: AnyWindowHandle) {
        self.window.set(Some(window));
    }

    /// What the scripts sign their messages with.
    pub fn nonce(&self) -> &str {
        &self.nonce
    }

    /// The grant the page's protocol serves from.
    pub fn grant(&self) -> &Grant {
        &self.grant
    }

    /// The text the page's text protocol serves.
    pub fn text(&self) -> &Text {
        &self.text
    }

    /// The theme the page's scripts carry now.
    pub fn theme(&self) -> HtmlOverlayTheme {
        self.theme.borrow().clone()
    }

    /// The page's canvas: a file page's the panel's background, which its unstyled document shows; a URL page keeps
    /// the browser's white, which a web app styled against it relies on.
    pub fn canvas(&self) -> Option<(u8, u8, u8)> {
        self.overlay()
            .themed()
            .then(|| rgb8(&self.colors.borrow().background))
            .flatten()
    }

    /// Redraws the element: the model or the web view changed what it shows.
    fn refresh(&self, cx: &mut App) {
        if let Some(element) = self.element() {
            element.update(cx, |_, cx| cx.notify());
        }
    }

    // MARK: - the native window's place

    /// The content was laid out at `bounds`.
    pub fn place(&self, bounds: Bounds) {
        let mut laid_out = self.laid_out.get();
        if laid_out.bounds == Some(bounds) {
            return;
        }
        laid_out.bounds = Some(bounds);
        self.laid_out.set(laid_out);
        self.placement_changed();
    }

    /// The element took or lost the keyboard.
    pub fn set_focused(&self, focused: bool) {
        let mut laid_out = self.laid_out.get();
        laid_out.focused = focused;
        self.laid_out.set(laid_out);
        self.placement_changed();
    }

    /// Where the native window should be now: shown only while the page is on screen, laid out, not failed (the
    /// failure is drawn where the page would be) and nothing the window draws covers it (`registry::set_covered`).
    pub fn wanted(&self) -> Placement {
        let mut wanted = self.laid_out.get();
        let (shown, failed) = {
            let state = self.state.borrow();
            (
                state.active() && state.on_screen,
                state.overlay.load_state == HtmlLoadState::Failed,
            )
        };
        let covered = self.element_window.get().is_some_and(registry::is_covered);
        wanted.visible = shown && !failed && !covered && wanted.bounds.is_some();
        wanted.focused &= wanted.visible;
        wanted
    }

    /// The wanted place may have moved: the pump moves the native window.
    pub fn placement_changed(&self) {
        if self.native.borrow().is_some() && self.wanted() != self.applied.get() {
            registry::sync_later(self.id);
        }
    }

    /// Moves, shows, hides and focuses the native window as [`Self::wanted`] says; run by the pump, outside GPUI's
    /// callbacks (module docs).
    pub fn sync_native(&self) {
        let Some(native) = self.native() else { return };
        let wanted = self.wanted();
        let applied = self.applied.replace(wanted);
        if let Some(parent) = wanted
            .parent
            .filter(|parent| applied.parent.is_some_and(|old| old != *parent))
        {
            native.reparent(parent);
        }
        if let Some(bounds) = wanted.bounds.filter(|bounds| applied.bounds != Some(*bounds)) {
            native.set_bounds(bounds);
        }
        if wanted.visible != applied.visible {
            if !wanted.visible && self.native_focused.get() {
                native.focus_parent();
            }
            native.set_visible(wanted.visible);
        }
        if wanted.focused && !self.native_focused.get() {
            native.focus();
        } else if !wanted.focused && self.native_focused.get() && wanted.visible {
            native.focus_parent();
        }
    }

    /// Whether the web view holds the keyboard.
    pub fn holds_native_keyboard(&self) -> bool {
        self.native_focused.get()
    }

    // MARK: - the web view

    /// The page could not get a web view, or the engine ended: the page fails with the reason, for good.
    pub fn engine_failed(&self, failure: &str) {
        let first = {
            let mut state = self.state.borrow_mut();
            let first = state.storage_failure.is_none();
            state.storage_failure = Some(failure.to_owned());
            first
        };
        if first {
            self.fail(failure.to_owned());
        }
    }

    /// The pump made the web view: it takes its place and loads the source.
    pub fn realized(&self, native: Native) {
        self.building.set(false);
        if !self.active() {
            registry::bury(Rc::new(native), self.private_folder());
            return;
        }
        let parent = self.laid_out.get().parent;
        self.applied.set(Placement {
            parent,
            ..Placement::default()
        });
        self.native.replace(Some(Rc::new(native)));
        self.placement_changed();
        self.load_original();
    }

    /// The in-memory profile's folder this page leaves behind, removed after it.
    pub fn private_folder(&self) -> Option<PathBuf> {
        (self.storage == Storage::Ephemeral)
            .then(|| registry::page_profile_folder(self.id))
            .flatten()
    }

    /// The scripts after the theme, in load order: the guard, the page script, and a `--js` file page's helper.
    pub fn scripts(&self) -> Vec<String> {
        let overlay = self.overlay();
        let themed = overlay.themed();
        let mut scripts = vec![
            bridge::GUARD_SCRIPT.to_owned(),
            bridge::page_script(&self.nonce, themed, overlay.flags.javascript),
        ];
        if themed && overlay.flags.javascript {
            scripts.push(bridge::helper_script());
        }
        scripts
    }

    /// Follows the terminal's `colors` (upstream's `applyTheme`): the panel is redrawn in them, and a changed theme is
    /// given to later loads; a file page wears it at once by reloading what it shows, a URL page gets the variables
    /// alone at its next load. Nothing runs in the live document.
    pub fn apply_theme(&self, terminal: &TerminalColors, cx: &mut App) {
        if !self.active() {
            return;
        }
        let themed = self.overlay().themed();
        let colors = PanelColors::resolve(terminal, self.background.as_deref());
        self.colors.replace(colors.clone());
        self.refresh(cx);
        let next = colors.theme();
        let change = policy::theme_change(&self.theme.borrow(), &next, themed);
        if change == ThemeChange::Unchanged {
            return;
        }
        let script = bridge::theme_script(&next.script(themed));
        self.theme.replace(next);
        let reload = change == ThemeChange::ReinstallAndReload;
        if let Some(native) = self.native() {
            // the reload waits for the new script to be in place
            self.reload_after_theme.set(reload);
            native.set_theme(&script);
            if let Some(canvas) = self.canvas() {
                native.set_background(canvas);
            }
        } else if reload {
            self.reload_shown();
        }
    }

    // MARK: - the navigation policy

    /// The shared policy's decision on `navigation` and its address as the app spells it; `None` for an address that
    /// is never asked about (a real `file:` one would bypass the grant, the text protocol serves nothing else) or does
    /// not parse.
    fn decision(&self, navigation: &Navigation, target: HtmlNavigationTarget) -> Option<(HtmlNavigationDecision, Url)> {
        if navigation.uri.starts_with("file:") || scheme::is_text_resource(&navigation.uri) {
            return None;
        }
        let url = Url::parse(&scheme::app_uri(&navigation.uri)).ok()?;
        let state = self.state.try_borrow().ok()?;
        if !state.active() {
            return None;
        }
        let action = HtmlNavigationAction {
            url: url.clone(),
            target,
            user_activated: navigation.user && target != HtmlNavigationTarget::Subframe,
        };
        Some((decide(&action, &state.overlay), url))
    }

    /// Whether `navigation` goes ahead, answered at once (module docs); the app's own text load is its own.
    pub fn allows(&self, navigation: &Navigation) -> bool {
        if navigation.main_frame {
            let text = self
                .state
                .try_borrow_mut()
                .map(|mut state| std::mem::take(&mut state.text_pending));
            if text.unwrap_or(false) {
                return true;
            }
        }
        let target = if navigation.main_frame {
            HtmlNavigationTarget::MainFrame
        } else {
            HtmlNavigationTarget::Subframe
        };
        self.decision(navigation, target)
            .is_some_and(|(decision, _)| decision == HtmlNavigationDecision::Allow)
    }

    // MARK: - the web view's reports

    /// One of the web view's reports.
    pub fn report(&self, report: Report, cx: &mut App) {
        match report {
            Report::Navigation { navigation, allowed } => self.navigation(&navigation, allowed, cx),
            Report::Committed => self.committed(),
            Report::Source(uri) => self.engine_page(|shown| shown.uri = Some(uri)),
            Report::History { back, forward } => self.engine_page(|shown| {
                shown.can_go_back = back;
                shown.can_go_forward = forward;
            }),
            Report::Title(title) => self.engine_page(|shown| shown.title = Some(title)),
            Report::Completed {
                id,
                success,
                status,
                http,
            } => self.completed(id, success, status, http),
            Report::ProcessFailed { browser: true } => registry::engine_ended(),
            Report::ProcessFailed { browser: false } => {
                self.state.borrow_mut().shown_url = None;
                self.load_event(&LoadEvent::Terminated);
            }
            Report::CloseRequested => {
                {
                    let mut state = self.state.borrow_mut();
                    if state.life == Life::Active {
                        state.life = Life::CloseRequested;
                    }
                }
                self.close_if_requested();
            }
            Report::NewWindow { uri, user } => {
                let navigation = Navigation {
                    uri,
                    main_frame: false,
                    user,
                    redirect: false,
                    id: 0,
                };
                self.offer(&navigation, HtmlNavigationTarget::NewWindow, cx);
            }
            Report::Message(raw) => self.message(&raw),
            Report::Focus(focused) => self.native_focus(focused, cx),
            Report::DownloadRefused(uri) => tracing::info!(uri, "a page's download was refused"),
            Report::Key(keystroke) => self.app_key(&keystroke, cx),
            Report::ThemeInstalled => {
                if self.reload_after_theme.take() {
                    self.reload_shown();
                }
            }
        }
    }

    /// A main-frame navigation as the policy decided it: one going ahead starts the load (a redirect continues it); a
    /// refused one the user did not click is the pending load's own, which it fails at once, and a hand-off the
    /// policy offers is asked about.
    fn navigation(&self, navigation: &Navigation, allowed: bool, cx: &mut App) {
        if allowed {
            if !navigation.redirect {
                self.load_event(&LoadEvent::Started);
            }
            return;
        }
        let outcome = {
            let mut state = self.state.borrow_mut();
            if !state.active() {
                return;
            }
            // its end, a cancellation, is this refusal's
            state.refused = Some(navigation.id);
            if !navigation.user || navigation.redirect {
                state.load.refused(&scheme::app_uri(&navigation.uri));
            }
            state
                .load
                .handle(&LoadEvent::Interrupted("navigation blocked".to_owned()))
        };
        self.report_outcome(outcome);
        self.offer(navigation, HtmlNavigationTarget::MainFrame, cx);
    }

    /// Asks whether to hand `navigation` to the browser when the policy offers that.
    fn offer(&self, navigation: &Navigation, target: HtmlNavigationTarget, cx: &mut App) {
        if let Some((HtmlNavigationDecision::OpenExternal, url)) = self.decision(navigation, target) {
            let page = self.me.clone();
            cx.defer(move |cx| {
                if let Some(page) = page.upgrade() {
                    page.ask_to_open(url, cx);
                }
            });
        }
    }

    /// A main-frame navigation ended: loaded (an HTTP error page is a page, as `WebKit` reads it, unless the app's own
    /// protocol refused the read), superseded, or failed; one the policy refused ended at its refusal.
    fn completed(&self, id: u64, success: bool, status: i32, http: i32) {
        if self
            .state
            .borrow_mut()
            .refused
            .take_if(|refused| *refused == id)
            .is_some()
        {
            return;
        }
        let event = if success && http >= 400 {
            match self.protocol_refusal() {
                Some(message) => LoadEvent::Failed(message),
                None => LoadEvent::Finished,
            }
        } else if success {
            LoadEvent::Finished
        } else if status == OPERATION_CANCELED {
            LoadEvent::Cancelled
        } else {
            LoadEvent::Failed(error_text(status))
        };
        self.load_event(&event);
    }

    /// What the app's protocol refused for the document the web view shows, when it did.
    fn protocol_refusal(&self) -> Option<String> {
        let shown = self
            .state
            .borrow()
            .engine
            .uri
            .clone()
            .map(|uri| scheme::app_uri(&uri))?;
        let refused = self
            .grant
            .refused
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        refused
            .filter(|(address, _)| *address == shown)
            .map(|(_, message)| message)
    }

    fn committed(&self) {
        let web = self.web_url();
        {
            let mut state = self.state.borrow_mut();
            if !state.active() {
                return;
            }
            state.load.handle(&LoadEvent::Committed);
            state.shown_url = web;
        }
        self.report_page();
    }

    /// The web view reported what it shows (`change`): a new address is a URL change (upstream's `urlChanged`),
    /// anything else a page report.
    fn engine_page(&self, change: impl FnOnce(&mut Shown)) {
        let moved = {
            let mut state = self.state.borrow_mut();
            let before = state.engine.uri.clone();
            change(&mut state.engine);
            state.engine.uri != before
        };
        if moved {
            let web = self.web_url();
            let mut state = self.state.borrow_mut();
            state.shown_url = policy::shown_after_change(state.shown_url.as_ref(), web.as_ref());
        }
        self.report_page();
    }

    /// The web view took or lost the keyboard: a click into the page gives its element the window's keyboard too.
    fn native_focus(&self, focused: bool, cx: &mut App) {
        self.native_focused.set(focused);
        if !focused {
            return;
        }
        let (Some(element), Some(window)) = (self.element(), self.element_window.get()) else {
            return;
        };
        let handle = element.read(cx).focus().clone();
        let _ = window.update(cx, |_, window, cx| {
            if !handle.is_focused(window) {
                window.focus(&handle, cx);
            }
        });
    }

    /// A key pressed in the page that the app takes: dispatched to the page's window as the keymap reads it, its
    /// release, which the window never sees, let go at once.
    fn app_key(&self, keystroke: &gpui_kit::Keystroke, cx: &mut App) {
        let Some(window) = self.element_window.get() else {
            return;
        };
        let _ = window.update(cx, |_, window, cx| {
            window.dispatch_keystroke(keystroke.clone(), cx);
            crate::keymap_dispatch::release_dispatched(keystroke);
        });
    }

    // MARK: - the bridge and the user's input

    fn message(&self, raw: &str) {
        match bridge::parse(raw, &self.nonce) {
            Some(bridge::Message::Input { click }) => self.user_input(click),
            Some(bridge::Message::Bridge { id, body }) => self.bridge_message(id, body),
            None => tracing::debug!(page = %self.id, "a web message that is none of ngterm's script's"),
        }
    }

    /// Runs one request the page sent and answers it once (upstream's `handleBridgeRequest`): the core resolves the
    /// page where it sits when the task runs, so a page that left its slot is refused before anything runs.
    fn bridge_message(&self, id: u64, body: String) {
        if !self.active() {
            self.reply(id, Err("page closed".to_owned()));
            return;
        }
        let token = registry::park(self.id, id);
        let page = self.id;
        // a page shown for another machine acts there: its request goes as sent, and that machine fills the defaults
        if matches!(self.overlay().source, HtmlSource::Replica { .. }) {
            registry::relay(page, body, token);
            return;
        }
        post(&self.poster, move |state: &mut AppState| {
            match state.core.html_bridge_request(page, body.as_bytes()) {
                Ok(request) => state.handle_page_request(&request, token),
                Err(message) => registry::reject(token, &message),
            }
        });
    }

    /// Settles the page's bridge request `id` with `answer`.
    pub fn reply(&self, id: u64, answer: Result<serde_json::Value, String>) {
        if let Some(native) = self.native() {
            native.post_json(&bridge::reply(id, &self.nonce, answer));
        }
    }

    /// Typing and clicks are user activity, so auto-follow cannot switch sessions under them, and real input lifts a
    /// declined prompt's silence; a click on a pane page moves the split's focus to its pane (upstream's
    /// `onUserInput`, `onFocus`, `onClick`).
    fn user_input(&self, click: bool) {
        {
            let mut state = self.state.borrow_mut();
            if !state.active() {
                return;
            }
            state.prompts_silenced = false;
        }
        let id = self.id;
        post(&self.poster, move |state: &mut AppState| {
            let Some(slot) = state.core.html_overlay_slot(id) else {
                return;
            };
            state.core.note_user_activity(slot.session);
            let moved = click && (state.core.html_page_focused(id) | state.core.html_page_clicked(id));
            if moved {
                state.settle();
            }
            state.page_input();
        });
    }

    /// The page took the keyboard (the user's click, or the deck): a pane page's split focus follows it.
    pub fn focused(&self) {
        if !self.active() {
            return;
        }
        let id = self.id;
        post(&self.poster, move |state: &mut AppState| {
            if state.core.html_page_focused(id) {
                state.settle();
            }
        });
    }

    // MARK: - load state and the page shown

    fn load_event(&self, event: &LoadEvent) {
        let outcome = {
            let mut state = self.state.borrow_mut();
            if !state.active() {
                return;
            }
            state.load.handle(event)
        };
        self.report_outcome(outcome);
    }

    fn report_outcome(&self, outcome: LoadOutcome) {
        match outcome {
            LoadOutcome::None => {}
            LoadOutcome::Loading => self.report_load(HtmlLoadState::Loading, None),
            LoadOutcome::Loaded => {
                self.report_page();
                self.report_load(HtmlLoadState::Loaded, None);
            }
            LoadOutcome::Failed(message) => self.report_load(HtmlLoadState::Failed, Some(message)),
        }
    }

    fn report_load(&self, load: HtmlLoadState, error: Option<String>) {
        {
            let state = self.state.borrow();
            if state.overlay.load_state == load && state.overlay.load_error == error {
                return;
            }
        }
        let id = self.id;
        post(&self.poster, move |state: &mut AppState| {
            if state.core.set_html_load_state(id, load, error) {
                state.settle();
            }
        });
    }

    pub fn fail(&self, message: String) {
        let outcome = self.state.borrow_mut().load.fail(message);
        self.report_outcome(outcome);
    }

    /// The web view's address as the app spells it.
    fn web_url(&self) -> Option<Url> {
        let uri = self.state.borrow().engine.uri.clone()?;
        Url::parse(&scheme::app_uri(&uri)).ok()
    }

    /// Reports what the page shows and where its history can go (upstream's `reportPage`).
    fn report_page(&self) {
        if self.state.borrow().engine.uri.is_none() {
            return;
        }
        let web = self.web_url();
        let info = {
            let mut state = self.state.borrow_mut();
            if !state.active() {
                return;
            }
            let Some(url) = policy::page_url(&state.overlay, state.shown_url.as_ref(), web.as_ref()) else {
                return;
            };
            let info = policy::page_info(
                &url,
                state.engine.title.as_deref(),
                state.engine.can_go_back,
                state.engine.can_go_forward,
            );
            if !policy::needs_report(state.overlay.current.as_ref(), state.reported.as_ref(), &info) {
                return;
            }
            state.reported = Some(info.clone());
            info
        };
        let id = self.id;
        post(&self.poster, move |state: &mut AppState| {
            if state.core.set_html_page(id, Some(info)) {
                state.settle();
            }
        });
    }

    // MARK: - the model's value

    /// Takes the model's latest value and whether the page is on screen; reloads when its revision moved (upstream's
    /// `apply` and `setOnScreen`). A page out of sight asks nothing, and its pending prompt goes with it.
    pub fn apply(&self, overlay: &HtmlOverlay, on_screen: bool, cx: &mut App) {
        let reload = {
            let mut state = self.state.borrow_mut();
            if !state.active() {
                return;
            }
            state.overlay = overlay.clone();
            state.on_screen = on_screen;
            let moved = overlay.reload_revision != state.applied_revision;
            state.applied_revision = overlay.reload_revision;
            moved.then_some(overlay.reload_target)
        };
        self.placement_changed();
        self.refresh(cx);
        if !on_screen {
            self.end_prompt(cx);
        }
        self.close_if_requested();
        match reload {
            Some(HtmlReloadTarget::Current) => self.reload_shown(),
            Some(HtmlReloadTarget::Original) => self.load_original(),
            None => {}
        }
    }

    /// Reloads what the page shows; before a first commit there is nothing to reload, so the source loads again.
    fn reload_shown(&self) {
        let reload = {
            let mut state = self.state.borrow_mut();
            let reload = !state.overlay.text_loaded() && state.load.committed();
            if reload {
                state.load.begin();
            }
            reload
        };
        match (reload, self.native()) {
            (true, Some(native)) => native.reload(),
            _ => self.load_original(),
        }
    }

    /// Loads the page's source (upstream's `loadOriginal`): a URL as is, a granted file through the grant's protocol, a
    /// file without a grant from its text (read again each time) through the text protocol, a replica's file through
    /// its own protocol; a page without its store loads nothing and fails.
    fn load_original(&self) {
        let source = {
            let mut state = self.state.borrow_mut();
            if let Some(failure) = state.storage_failure.clone() {
                drop(state);
                self.fail(failure);
                return;
            }
            state.load.begin();
            state.overlay.source.clone()
        };
        let Some(native) = self.native() else { return };
        let address = match source {
            HtmlSource::Url(url) => url.as_str().to_owned(),
            HtmlSource::File {
                path,
                grant_root: Some(root),
            } => {
                *self.grant.root.lock().unwrap_or_else(PoisonError::into_inner) =
                    policy::GrantRoot::new(std::path::Path::new(&root));
                match scheme::granted_address(&path) {
                    Some(address) => address,
                    None => return self.fail(format!("not an absolute path: {path}")),
                }
            }
            HtmlSource::File { path, grant_root: None } => return self.load_text(&native, &path),
            HtmlSource::Replica { path } => match scheme::replica_address(self.id, &path) {
                Some(address) => address,
                None => return self.fail(errors::REMOTE_FILE.to_owned()),
            },
        };
        native.load_url(&address);
    }

    /// Loads file `path`'s text through the text protocol, at the one address that replaces the text shown (module
    /// docs of `scheme`).
    fn load_text(&self, native: &Native, path: &str) {
        let html = match std::fs::read_to_string(path) {
            Ok(html) => html,
            Err(error) => return self.fail(error.to_string()),
        };
        self.text.lock().unwrap_or_else(PoisonError::into_inner).replace(html);
        self.state.borrow_mut().text_pending = true;
        native.load_url(&scheme::text_address(self.id));
    }

    pub fn set_zoom(&self, zoom: f64) {
        if let Some(native) = self.native() {
            native.set_zoom(zoom);
        }
    }

    // MARK: - navigation and hand-offs

    /// A history step or hand-off (upstream's `navigate`).
    pub fn navigate(&self, navigation: HtmlNavigation) -> Result<(), String> {
        if let Some(refusal) =
            ngterm_core::remote::replica_page::replica_navigation_refusal(&self.overlay(), navigation)
        {
            return Err(refusal.to_owned());
        }
        let native = self.native();
        let shown = self.state.borrow().engine.clone();
        match navigation {
            HtmlNavigation::Back => {
                let (Some(native), true) = (native, shown.can_go_back) else {
                    return Err(errors::no_history(navigation));
                };
                native.go_back();
            }
            HtmlNavigation::Forward => {
                let (Some(native), true) = (native, shown.can_go_forward) else {
                    return Err(errors::no_history(navigation));
                };
                native.go_forward();
            }
            HtmlNavigation::Browser => {
                let opened = self.browser_url().is_some_and(|url| super::browser::open(url.as_str()));
                if !opened {
                    return Err(errors::NO_BROWSER.to_owned());
                }
            }
            HtmlNavigation::Finder => {
                if !self.overlay().themed() {
                    return Err(errors::FINDER_REQUIRES_FILE.to_owned());
                }
                if let Some(path) = self.page_path() {
                    post(&self.poster, move |state: &mut AppState| state.reveal(path));
                }
            }
        }
        Ok(())
    }

    /// What a hand-off to the browser opens (upstream's `browserURL`).
    pub fn browser_url(&self) -> Option<Url> {
        let web = self.web_url();
        let state = self.state.borrow();
        policy::browser_url(&state.overlay, state.shown_url.as_ref(), web.as_ref())
    }

    /// The file the page shows, which the file manager reveals.
    fn page_path(&self) -> Option<PathBuf> {
        let web = self.web_url();
        let state = self.state.borrow();
        policy::page_url(&state.overlay, state.shown_url.as_ref(), web.as_ref())?
            .to_file_path()
            .ok()
    }

    /// A page's own hand-off asks first, one prompt at a time, only while the page is on screen; a request while one
    /// is up is dropped (upstream's `askToOpen`). Cancel is the default, so typing meant for the page cannot approve
    /// a surprise prompt.
    fn ask_to_open(&self, url: Url, cx: &mut App) {
        {
            let state = self.state.borrow();
            if !state.active() || !state.on_screen || state.prompt.is_some() || state.prompts_silenced {
                return;
            }
        }
        let Some(window) = self.window.get() else {
            return;
        };
        let shown = HtmlSource::origin_of(&url).unwrap_or_else(|| url.as_str().to_owned());
        let spec = AlertSpec {
            heading: format!("Open {shown} in your browser?"),
            body: url.as_str().to_owned(),
            buttons: vec![
                AlertButton::cancel("Cancel").as_default(),
                AlertButton::plain("Open").suggested(),
            ],
            checks: Vec::new(),
            field: None,
        };
        let page = self.me.clone();
        let opened = window.update(cx, |_, window, cx| {
            crate::modal::open_alert(
                window,
                spec,
                move |answer, _, _| {
                    if let Some(page) = page.upgrade() {
                        page.answer_prompt(&url, answer.button == 1);
                    }
                },
                cx,
            )
        });
        if let Ok(alert) = opened {
            self.state.borrow_mut().prompt = Some(alert);
        }
    }

    fn answer_prompt(&self, url: &Url, approved: bool) {
        {
            let mut state = self.state.borrow_mut();
            if state.prompt.take().is_none() {
                return;
            }
            if !approved {
                state.prompts_silenced = true;
            }
        }
        if approved && !super::browser::open(url.as_str()) {
            tracing::warn!("no default web browser to open the link in");
        }
    }

    /// Withdraws the page's prompt, unanswered.
    fn end_prompt(&self, cx: &mut App) {
        let prompt = self.state.borrow_mut().prompt.take();
        if let (Some(alert), Some(window)) = (prompt, self.window.get()) {
            let _ = window.update(cx, |_, window, cx| crate::modal::withdraw_alert(&alert, window, cx));
        }
    }

    fn close_if_requested(&self) {
        if self.state.borrow().life != Life::CloseRequested {
            return;
        }
        let id = self.id;
        post(&self.poster, move |state: &mut AppState| {
            if state.core.close_html_overlay(id) {
                state.settle();
            }
        });
    }

    // MARK: - release

    /// Ends the page (upstream's `close`): its prompt goes, its web view is handed to the pump to close, and the
    /// handlers still reached do nothing.
    pub fn close(&self, cx: &mut App) {
        self.end_prompt(cx);
        self.state.borrow_mut().life = Life::Released;
        if let Some(native) = self.native.take() {
            registry::bury(native, self.private_folder());
        }
        if let Some(element) = self.element() {
            element.update(cx, PageView::release);
        }
    }
}

/// `#rrggbb` as its channels.
fn rgb8(hex: &str) -> Option<(u8, u8, u8)> {
    let digits = hex.strip_prefix('#').filter(|digits| digits.len() == 6)?;
    let channel = |at: usize| u8::from_str_radix(digits.get(at..at + 2)?, 16).ok();
    Some((channel(0)?, channel(2)?, channel(4)?))
}

/// A failed navigation's reason, by `WebView2`'s error status (`COREWEBVIEW2_WEB_ERROR_STATUS`).
#[must_use]
pub fn error_text(status: i32) -> String {
    let reason = match status {
        1 => "the server's certificate names another host",
        2 => "the server's certificate has expired",
        3 => "the client's certificate has errors",
        4 => "the server's certificate has been revoked",
        5 => "the server's certificate is invalid",
        6 => "the server could not be reached",
        7 => "the request timed out",
        8 => "the server returned an invalid response",
        9 => "the connection was aborted",
        10 => "the connection was reset",
        11 => "the internet connection was lost",
        12 => "could not connect to the server",
        13 => "the server's name could not be resolved",
        15 => "the redirect failed",
        16 => "an unexpected error occurred",
        17 => "the server requires authentication",
        18 => "the proxy requires authentication",
        _ => "the page could not be loaded",
    };
    reason.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_read_as_channels() {
        assert_eq!(rgb8("#102030"), Some((0x10, 0x20, 0x30)));
        assert_eq!(rgb8("102030"), None);
        assert_eq!(rgb8("#1020"), None);
        assert_eq!(rgb8("#zz2030"), None);
    }

    #[test]
    fn a_failed_navigation_names_its_reason() {
        assert_eq!(error_text(13), "the server's name could not be resolved");
        assert_eq!(error_text(12), "could not connect to the server");
        assert_eq!(error_text(999), "the page could not be loaded");
    }
}
