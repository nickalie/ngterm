//! One page's `WebView2`: a child window of its GPUI window, made through wry and wired to `WebView2`'s own events
//! where wry has no API for them (webview2-com): the navigation policy (`NavigationStarting` and
//! `FrameNavigationStarting`, answered at once), the load and page reports, focus, the context menu, script dialogs,
//! new windows and the store's clear. Everything else the page reports is posted to the registry's pump as a
//! [`Report`], never handled in place: `WebView2` raises its events from the window's message loop, where the app's
//! state may be busy.
//!
//! What a page may not do is refused here, as the WPE host refuses it: new windows (offered to the browser by the
//! policy, never opened), script dialogs (answered silently: `WebView2` cancels a dialog no handler accepts),
//! permissions, downloads, dropped files, devtools and the browser's own accelerator keys (upstream's pages are not
//! inspectable either), and the context menu's items that would save, print, share, inspect or open a window. Making
//! a web view pumps the window's messages until `WebView2` answers, so it never runs inside one of GPUI's callbacks
//! (`registry`'s pump makes them).

use std::cell::RefCell;
use std::num::NonZeroIsize;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError};

use raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND, COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND_SEPARATOR, COREWEBVIEW2_KEY_EVENT_KIND,
    COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN, COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN,
    COREWEBVIEW2_PROCESS_FAILED_KIND, COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED,
    COREWEBVIEW2_WEB_ERROR_STATUS, ICoreWebView2, ICoreWebView2_11, ICoreWebView2_13, ICoreWebView2ContextMenuItem,
    ICoreWebView2ContextMenuItemCollection, ICoreWebView2Controller, ICoreWebView2Environment,
    ICoreWebView2NavigationCompletedEventArgs, ICoreWebView2NavigationCompletedEventArgs2,
    ICoreWebView2NavigationStartingEventArgs, ICoreWebView2Profile2,
};
use webview2_com::{
    AcceleratorKeyPressedEventHandler, AddScriptToExecuteOnDocumentCreatedCompletedHandler,
    ClearBrowsingDataCompletedHandler, ContentLoadingEventHandler, ContextMenuRequestedEventHandler,
    DocumentTitleChangedEventHandler, FocusChangedEventHandler, HistoryChangedEventHandler,
    NavigationCompletedEventHandler, NavigationStartingEventHandler, NewWindowRequestedEventHandler,
    ProcessFailedEventHandler, ScriptDialogOpeningEventHandler, SourceChangedEventHandler,
    WebMessageReceivedEventHandler, WindowCloseRequestedEventHandler, take_pwstr,
};
use windows::core::{BOOL, HSTRING, Interface as _, PWSTR, Result as ComResult};
use wry::dpi::{LogicalPosition, LogicalSize};
use wry::{
    PermissionResponse, Rect, RequestAsyncResponder, WebContext, WebView, WebViewBuilder, WebViewBuilderExtWindows,
    WebViewExtWindows,
};

use super::scheme;

/// A page's content area in its window, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Bounds {
    fn rect(self) -> Rect {
        Rect {
            position: LogicalPosition::new(f64::from(self.x), f64::from(self.y)).into(),
            size: LogicalSize::new(f64::from(self.width.max(1.0)), f64::from(self.height.max(1.0))).into(),
        }
    }
}

/// A navigation the policy decides at once (`NavigationStarting`, `FrameNavigationStarting`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Navigation {
    /// The address as the web view spells it.
    pub uri: String,
    pub main_frame: bool,
    /// The user started it (a click on a link), not a script.
    pub user: bool,
    /// A redirect of the navigation in flight.
    pub redirect: bool,
    pub id: u64,
}

/// What a page reports, posted to the registry's pump.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Report {
    /// A main-frame navigation, as the policy decided it.
    Navigation {
        navigation: Navigation,
        allowed: bool,
    },
    /// The main frame's document started loading: the navigation committed.
    Committed,
    /// The web view's address changed (a load, a fragment, `pushState`).
    Source(String),
    History {
        back: bool,
        forward: bool,
    },
    Title(String),
    /// A main-frame navigation ended: `status` is `WebView2`'s error status, `http` the response's status code.
    Completed {
        id: u64,
        success: bool,
        status: i32,
        http: i32,
    },
    /// The page's web content process ended; `browser`: the browser process with it, and every page.
    ProcessFailed {
        browser: bool,
    },
    /// `window.close()`.
    CloseRequested,
    /// A request for a new window (never opened).
    NewWindow {
        uri: String,
        user: bool,
    },
    /// A web message the page posted.
    Message(String),
    /// The web view took or lost the keyboard.
    Focus(bool),
    /// A download the page started, refused.
    DownloadRefused(String),
    /// A key pressed in the page that the app takes (`keys`), kept from the page.
    Key(gpui_kit::Keystroke),
    /// A replaced theme script is in place: later loads wear it.
    ThemeInstalled,
}

/// Decides a navigation at once.
pub type Decide = Rc<dyn Fn(&Navigation) -> bool>;
/// Posts a report to the pump.
pub type Post = Rc<dyn Fn(Report)>;
/// The keystroke the app takes from a key pressed in the page, by its virtual key.
pub type TakeKey = Rc<dyn Fn(u32) -> Option<gpui_kit::Keystroke>>;

/// How the grant's protocol answers: the page's grant as a real path, and the last read it refused (an address and
/// why), which a failed main-frame load reports.
#[derive(Clone, Default)]
pub struct Grant {
    pub root: Arc<Mutex<Option<PathBuf>>>,
    pub refused: Arc<Mutex<Option<(String, String)>>>,
}

/// The text a file page without a grant shows, which its text protocol serves (`scheme`); `None` before its load.
pub type Text = Arc<Mutex<Option<String>>>;

/// How a replica's protocol is answered: its read, the app's address and the responder.
pub type ReplicaRead = Arc<dyn Fn(String, RequestAsyncResponder) + Send + Sync>;

/// Where a page's store is: the environment the pages share once one exists, else the folder a new one keeps its
/// data in, and the profile.
#[derive(Clone)]
pub struct Store {
    pub environment: Option<ICoreWebView2Environment>,
    pub user_data: PathBuf,
    pub profile: String,
    /// An `InPrivate` profile: kept in memory.
    pub private: bool,
}

/// What a page's web view is built from.
pub struct Spec {
    pub parent: isize,
    pub store: Store,
    pub javascript: bool,
    /// The page's canvas where it draws nothing; `None` keeps the browser's white.
    pub background: Option<(u8, u8, u8)>,
    pub zoom: f64,
    pub bounds: Option<Bounds>,
    pub visible: bool,
    /// The scripts in load order: the theme first (replaced by [`Native::set_theme`]), then the rest.
    pub theme: String,
    pub scripts: Vec<String>,
    pub grant: Option<Grant>,
    pub replica: Option<ReplicaRead>,
    pub text: Option<Text>,
    pub decide: Decide,
    pub post: Post,
    pub take_key: TakeKey,
}

/// A parent window handle as wry takes it.
struct Parent(isize);

impl HasWindowHandle for Parent {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let hwnd = NonZeroIsize::new(self.0).ok_or(HandleError::Unavailable)?;
        let raw = RawWindowHandle::Win32(Win32WindowHandle::new(hwnd));
        // SAFETY: the window outlives the call that borrows it: the GPUI window the page is mounted in.
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

/// A page's web view.
pub struct Native {
    view: WebView,
    /// The theme script's id once `WebView2` added it.
    theme: Rc<RefCell<Option<String>>>,
    post: Post,
}

impl Native {
    /// Makes the web view `spec` describes, wired and with its scripts, loading nothing yet. Pumps the window's
    /// messages until it exists (module docs).
    pub fn create(spec: &Spec) -> Result<Self, String> {
        let view = build(spec)?;
        let native = Self {
            view,
            theme: Rc::default(),
            post: Rc::clone(&spec.post),
        };
        let webview = native.webview();
        wire_navigation(&webview, &spec.decide, &spec.post).map_err(|err| err.to_string())?;
        wire_page(&webview, &spec.post).map_err(|err| err.to_string())?;
        wire_refusals(&webview).map_err(|err| err.to_string())?;
        wire_focus(&native.view.controller(), &spec.post).map_err(|err| err.to_string())?;
        wire_keys(&native.view.controller(), &spec.take_key, &spec.post).map_err(|err| err.to_string())?;
        // each script is in place before the first load starts: `WebView2` adds them asynchronously
        let theme = native.add_script_now(&spec.theme)?;
        native.theme.replace(Some(theme));
        for script in &spec.scripts {
            native.add_script_now(script)?;
        }
        Ok(native)
    }

    /// Adds `script` for later documents and waits until `WebView2` did (pumping the window's messages); its id.
    fn add_script_now(&self, script: &str) -> Result<String, String> {
        let webview = self.webview();
        let source = HSTRING::from(script);
        let id = Rc::new(RefCell::new(String::new()));
        let added = Rc::clone(&id);
        AddScriptToExecuteOnDocumentCreatedCompletedHandler::wait_for_async_operation(
            Box::new(move |handler| {
                // SAFETY: a COM call on the web view's own thread.
                unsafe { webview.AddScriptToExecuteOnDocumentCreated(&source, &handler) }.map_err(Into::into)
            }),
            Box::new(move |result, script_id| {
                added.replace(script_id);
                result
            }),
        )
        .map_err(|err| format!("a page's script was not added: {err}"))?;
        Ok(id.take())
    }

    /// The environment the web view runs in, which later pages share.
    pub fn environment(&self) -> ICoreWebView2Environment {
        self.view.environment()
    }

    fn webview(&self) -> ICoreWebView2 {
        self.view.webview()
    }

    /// Adds `script` for later documents; once `WebView2` did, its id goes to `id` and the page is told
    /// ([`Report::ThemeInstalled`]).
    fn add_script(&self, script: &str, id: Rc<RefCell<Option<String>>>) {
        let post = Rc::clone(&self.post);
        let handler = AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(Box::new(move |result, added| {
            if result.is_ok() {
                id.replace(Some(added));
            }
            post(Report::ThemeInstalled);
            Ok(())
        }));
        let webview = self.webview();
        // SAFETY: a COM call on the web view's own thread.
        let added = unsafe { webview.AddScriptToExecuteOnDocumentCreated(&HSTRING::from(script), &handler) };
        if let Err(err) = added {
            tracing::warn!(%err, "a page's script was not added");
        }
    }

    /// Replaces the theme script, which later documents run (the WPE host's `set_scripts`); the page hears once it is
    /// in place ([`Report::ThemeInstalled`]).
    pub fn set_theme(&self, script: &str) {
        if let Some(old) = self.theme.take() {
            let webview = self.webview();
            // SAFETY: a COM call on the web view's own thread.
            let _ = unsafe { webview.RemoveScriptToExecuteOnDocumentCreated(&HSTRING::from(old)) };
        }
        self.add_script(script, Rc::clone(&self.theme));
    }

    /// Loads `address`, as the web view spells it.
    pub fn load_url(&self, address: &str) {
        if let Err(err) = self.view.load_url(address) {
            tracing::warn!(%err, "a page's load did not start");
        }
    }

    pub fn reload(&self) {
        let _ = self.view.reload();
    }

    pub fn go_back(&self) {
        let _ = self.view.go_back();
    }

    pub fn go_forward(&self) {
        let _ = self.view.go_forward();
    }

    pub fn set_zoom(&self, zoom: f64) {
        let _ = self.view.zoom(zoom);
    }

    pub fn set_background(&self, (red, green, blue): (u8, u8, u8)) {
        let _ = self.view.set_background_color((red, green, blue, 255));
    }

    pub fn set_bounds(&self, bounds: Bounds) {
        let _ = self.view.set_bounds(bounds.rect());
    }

    pub fn set_visible(&self, visible: bool) {
        let _ = self.view.set_visible(visible);
    }

    /// Gives the web view the keyboard.
    pub fn focus(&self) {
        let _ = self.view.focus();
    }

    /// Hands the keyboard back to the window.
    pub fn focus_parent(&self) {
        let _ = self.view.focus_parent();
    }

    /// Moves the web view into window `parent` (its session went to another window).
    pub fn reparent(&self, parent: isize) {
        let _ = self.view.reparent(parent);
    }

    /// Posts `json` to the page as a web message.
    pub fn post_json(&self, json: &str) {
        let webview = self.webview();
        // SAFETY: a COM call on the web view's own thread.
        let posted = unsafe { webview.PostWebMessageAsJson(&HSTRING::from(json)) };
        if let Err(err) = posted {
            tracing::debug!(%err, "a page's reply was not delivered");
        }
    }
}

/// The web view of `spec` (wry's part of it).
fn build(spec: &Spec) -> Result<WebView, String> {
    let mut context = WebContext::new(Some(spec.store.user_data.clone()));
    let post = Rc::clone(&spec.post);
    let mut builder = WebViewBuilder::new_with_web_context(&mut context)
        .with_visible(spec.visible)
        .with_focused(false)
        .with_devtools(false)
        .with_hotkeys_zoom(false)
        .with_clipboard(false)
        .with_autoplay(false)
        .with_back_forward_navigation_gestures(false)
        .with_incognito(spec.store.private)
        .with_profile_name(spec.store.profile.clone())
        .with_browser_accelerator_keys(false)
        .with_https_scheme(true)
        // no permission is granted: camera, microphone, location, notifications, the clipboard's read
        .with_permission_handler(|_| PermissionResponse::Deny)
        // nothing a page shows is saved to disk
        .with_download_started_handler(move |uri, _| {
            post(Report::DownloadRefused(uri));
            false
        })
        // nothing is dropped onto a page: a handler of wry's turns `WebView2`'s own drops off
        .with_drag_drop_handler(|_| true);
    if let Some(environment) = spec.store.environment.clone() {
        builder = builder.with_environment(environment);
    }
    if !spec.javascript {
        builder = builder.with_javascript_disabled();
    }
    if let Some((red, green, blue)) = spec.background {
        builder = builder.with_background_color((red, green, blue, 255));
    }
    if let Some(bounds) = spec.bounds {
        builder = builder.with_bounds(bounds.rect());
    }
    if let Some(grant) = spec.grant.clone() {
        builder =
            builder.with_asynchronous_custom_protocol(scheme::GRANT_PROTOCOL.to_owned(), move |_, request, reply| {
                serve_granted(&grant, &request.uri().to_string(), reply);
            });
    }
    if let Some(text) = spec.text.clone() {
        builder = builder.with_custom_protocol(scheme::TEXT_PROTOCOL.to_owned(), move |_, request| {
            serve_text(&text, &request.uri().to_string())
        });
    }
    if let Some(replica) = spec.replica.clone() {
        builder =
            builder.with_asynchronous_custom_protocol(scheme::REPLICA_PROTOCOL.to_owned(), move |_, request, reply| {
                replica(scheme::request_app_uri(&request.uri().to_string()), reply);
            });
    }
    let view = builder
        .build_as_child(&Parent(spec.parent))
        .map_err(|err| err.to_string())?;
    if spec.zoom.to_bits() != 1.0f64.to_bits() {
        let _ = view.zoom(spec.zoom);
    }
    Ok(view)
}

// MARK: - reading `WebView2`'s values

/// The string a COM getter writes.
fn string(read: impl FnOnce(*mut PWSTR) -> ComResult<()>) -> ComResult<String> {
    let mut value = PWSTR::null();
    read(&raw mut value)?;
    Ok(take_pwstr(value))
}

/// The flag a COM getter writes.
fn flag(read: impl FnOnce(*mut BOOL) -> ComResult<()>) -> ComResult<bool> {
    let mut value = BOOL::default();
    read(&raw mut value)?;
    Ok(value.as_bool())
}

/// The decision's view of a navigation's arguments.
fn navigation_of(args: &ICoreWebView2NavigationStartingEventArgs, main_frame: bool) -> ComResult<Navigation> {
    let mut id = 0u64;
    // SAFETY: the event's own arguments, read on its thread.
    unsafe { args.NavigationId(&raw mut id) }?;
    Ok(Navigation {
        uri: string(|out| {
            // SAFETY: as above.
            unsafe { args.Uri(out) }
        })?,
        main_frame,
        user: flag(|out| {
            // SAFETY: as above.
            unsafe { args.IsUserInitiated(out) }
        })?,
        redirect: flag(|out| {
            // SAFETY: as above.
            unsafe { args.IsRedirected(out) }
        })?,
        id,
    })
}

/// A finished navigation's report.
fn completion_of(args: &ICoreWebView2NavigationCompletedEventArgs) -> ComResult<Report> {
    let (mut status, mut id, mut http) = (COREWEBVIEW2_WEB_ERROR_STATUS::default(), 0u64, 0);
    // SAFETY: the event's own arguments, read on its thread.
    unsafe { args.WebErrorStatus(&raw mut status) }?;
    // SAFETY: as above.
    unsafe { args.NavigationId(&raw mut id) }?;
    if let Ok(args) = args.cast::<ICoreWebView2NavigationCompletedEventArgs2>() {
        // SAFETY: as above.
        let _ = unsafe { args.HttpStatusCode(&raw mut http) };
    }
    Ok(Report::Completed {
        id,
        success: flag(|out| {
            // SAFETY: as above.
            unsafe { args.IsSuccess(out) }
        })?,
        status: status.0,
        http,
    })
}

// MARK: - events

/// The navigation policy on the main frame and its frames, answered at once and reported.
fn wire_navigation(webview: &ICoreWebView2, decide: &Decide, post: &Post) -> ComResult<()> {
    let mut token = 0i64;
    let (main_decide, main_post) = (Rc::clone(decide), Rc::clone(post));
    let main = NavigationStartingEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args else { return Ok(()) };
        let navigation = navigation_of(&args, true)?;
        let allowed = main_decide(&navigation);
        // SAFETY: the event's own arguments, answered on its thread.
        unsafe { args.SetCancel(!allowed) }?;
        main_post(Report::Navigation { navigation, allowed });
        Ok(())
    }));
    // SAFETY: a COM registration on the web view's own thread; the handler lives as long as the web view.
    unsafe { webview.add_NavigationStarting(&main, &raw mut token) }?;
    let frame_decide = Rc::clone(decide);
    let frames = NavigationStartingEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args else { return Ok(()) };
        let navigation = navigation_of(&args, false)?;
        let allowed = frame_decide(&navigation);
        tracing::debug!(uri = navigation.uri, allowed, "a page's frame navigation");
        // SAFETY: the event's own arguments, answered on its thread.
        unsafe { args.SetCancel(!allowed) }
    }));
    // SAFETY: as above.
    unsafe { webview.add_FrameNavigationStarting(&frames, &raw mut token) }?;
    let sent = Rc::clone(post);
    let completed = NavigationCompletedEventHandler::create(Box::new(move |_, args| {
        if let Some(args) = args {
            sent(completion_of(&args)?);
        }
        Ok(())
    }));
    // SAFETY: as above.
    unsafe { webview.add_NavigationCompleted(&completed, &raw mut token) }?;
    let sent = Rc::clone(post);
    let committed = ContentLoadingEventHandler::create(Box::new(move |_, _| {
        sent(Report::Committed);
        Ok(())
    }));
    // SAFETY: as above.
    unsafe { webview.add_ContentLoading(&committed, &raw mut token) }?;
    let sent = Rc::clone(post);
    let process = ProcessFailedEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args else { return Ok(()) };
        let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
        // SAFETY: the event's own arguments, read on its thread.
        unsafe { args.ProcessFailedKind(&raw mut kind) }?;
        sent(Report::ProcessFailed {
            browser: kind == COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED,
        });
        Ok(())
    }));
    // SAFETY: as above.
    unsafe { webview.add_ProcessFailed(&process, &raw mut token) }
}

/// What the page shows and asks: its address, history, title, messages, its close and new windows.
fn wire_page(webview: &ICoreWebView2, post: &Post) -> ComResult<()> {
    let mut token = 0i64;
    let sent = Rc::clone(post);
    let source = SourceChangedEventHandler::create(Box::new(move |webview, _| {
        if let Some(webview) = webview {
            sent(Report::Source(string(|out| {
                // SAFETY: the event's web view, read on its thread.
                unsafe { webview.Source(out) }
            })?));
        }
        Ok(())
    }));
    // SAFETY: a COM registration on the web view's own thread; the handler lives as long as the web view.
    unsafe { webview.add_SourceChanged(&source, &raw mut token) }?;
    let sent = Rc::clone(post);
    let history = HistoryChangedEventHandler::create(Box::new(move |webview, _| {
        let Some(webview) = webview else { return Ok(()) };
        let back = flag(|out| {
            // SAFETY: the event's web view, read on its thread.
            unsafe { webview.CanGoBack(out) }
        })?;
        let forward = flag(|out| {
            // SAFETY: as above.
            unsafe { webview.CanGoForward(out) }
        })?;
        sent(Report::History { back, forward });
        Ok(())
    }));
    // SAFETY: as above.
    unsafe { webview.add_HistoryChanged(&history, &raw mut token) }?;
    let sent = Rc::clone(post);
    let title = DocumentTitleChangedEventHandler::create(Box::new(move |webview, _| {
        if let Some(webview) = webview {
            sent(Report::Title(string(|out| {
                // SAFETY: the event's web view, read on its thread.
                unsafe { webview.DocumentTitle(out) }
            })?));
        }
        Ok(())
    }));
    // SAFETY: as above.
    unsafe { webview.add_DocumentTitleChanged(&title, &raw mut token) }?;
    let sent = Rc::clone(post);
    let message = WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
        // a message that is no string is none of ngterm's script's
        let text = args.map(|args| {
            string(|out| {
                // SAFETY: the event's own arguments, read on its thread.
                unsafe { args.TryGetWebMessageAsString(out) }
            })
        });
        if let Some(Ok(text)) = text {
            sent(Report::Message(text));
        }
        Ok(())
    }));
    // SAFETY: as above.
    unsafe { webview.add_WebMessageReceived(&message, &raw mut token) }?;
    // wry's own handler destroys the web view's window first; the page closes either way
    let sent = Rc::clone(post);
    let close = WindowCloseRequestedEventHandler::create(Box::new(move |_, _| {
        sent(Report::CloseRequested);
        Ok(())
    }));
    // SAFETY: as above.
    unsafe { webview.add_WindowCloseRequested(&close, &raw mut token) }?;
    wire_new_windows(webview, post)
}

/// A new window is never opened (wry answers every one handled when it has no handler of its own); it is reported,
/// for the policy to offer the browser.
fn wire_new_windows(webview: &ICoreWebView2, post: &Post) -> ComResult<()> {
    let mut token = 0i64;
    let sent = Rc::clone(post);
    let handler = NewWindowRequestedEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args else { return Ok(()) };
        // SAFETY: the event's own arguments, answered on its thread.
        unsafe { args.SetHandled(true) }?;
        let uri = string(|out| {
            // SAFETY: as above.
            unsafe { args.Uri(out) }
        })?;
        let user = flag(|out| {
            // SAFETY: as above.
            unsafe { args.IsUserInitiated(out) }
        })?;
        sent(Report::NewWindow { uri, user });
        Ok(())
    }));
    // SAFETY: a COM registration on the web view's own thread; the handler lives as long as the web view.
    unsafe { webview.add_NewWindowRequested(&handler, &raw mut token) }
}

/// Script dialogs answered silently, no host objects, and the context menu trimmed (module docs).
fn wire_refusals(webview: &ICoreWebView2) -> ComResult<()> {
    let mut token = 0i64;
    // SAFETY: a COM call on the web view's own thread.
    let settings = unsafe { webview.Settings() }?;
    // SAFETY: as above.
    unsafe { settings.SetAreDefaultScriptDialogsEnabled(false) }?;
    // SAFETY: as above.
    unsafe { settings.SetAreHostObjectsAllowed(false) }?;
    // a dialog no handler accepts is cancelled: `alert` returns, `confirm` is false, `prompt` null
    let dialogs = ScriptDialogOpeningEventHandler::create(Box::new(|_, _| Ok(())));
    // SAFETY: a COM registration on the web view's own thread; the handler lives as long as the web view.
    unsafe { webview.add_ScriptDialogOpening(&dialogs, &raw mut token) }?;
    let Ok(webview) = webview.cast::<ICoreWebView2_11>() else {
        return Ok(());
    };
    let menu = ContextMenuRequestedEventHandler::create(Box::new(|_, args| {
        let Some(args) = args else { return Ok(()) };
        // SAFETY: the event's own arguments, read on its thread.
        let items = unsafe { args.MenuItems() }?;
        trim_context_menu(&items)
    }));
    // SAFETY: as above.
    unsafe { webview.add_ContextMenuRequested(&menu, &raw mut token) }
}

/// The web view's keyboard focus, taken (a click into the page) or lost.
fn wire_focus(controller: &ICoreWebView2Controller, post: &Post) -> ComResult<()> {
    let mut token = 0i64;
    let sent = Rc::clone(post);
    let got = FocusChangedEventHandler::create(Box::new(move |_, _| {
        sent(Report::Focus(true));
        Ok(())
    }));
    // SAFETY: a COM registration on the controller's own thread; the handler lives as long as the web view.
    unsafe { controller.add_GotFocus(&got, &raw mut token) }?;
    let sent = Rc::clone(post);
    let lost = FocusChangedEventHandler::create(Box::new(move |_, _| {
        sent(Report::Focus(false));
        Ok(())
    }));
    // SAFETY: as above.
    unsafe { controller.add_LostFocus(&lost, &raw mut token) }
}

/// The keys pressed in the page that the app takes (`keys`): kept from the page and reported.
fn wire_keys(controller: &ICoreWebView2Controller, take_key: &TakeKey, post: &Post) -> ComResult<()> {
    let mut token = 0i64;
    let (take, sent) = (Rc::clone(take_key), Rc::clone(post));
    let handler = AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args else { return Ok(()) };
        let mut kind = COREWEBVIEW2_KEY_EVENT_KIND::default();
        // SAFETY: the event's own arguments, read on its thread.
        unsafe { args.KeyEventKind(&raw mut kind) }?;
        if kind != COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN && kind != COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN {
            return Ok(());
        }
        let mut virtual_key = 0u32;
        // SAFETY: as above.
        unsafe { args.VirtualKey(&raw mut virtual_key) }?;
        if let Some(keystroke) = take(virtual_key) {
            // SAFETY: as above, answered on its thread.
            unsafe { args.SetHandled(true) }?;
            sent(Report::Key(keystroke));
        }
        Ok(())
    }));
    // SAFETY: a COM registration on the controller's own thread; the handler lives as long as the web view.
    unsafe { controller.add_AcceleratorKeyPressed(&handler, &raw mut token) }
}

// MARK: - the context menu

/// The context menu items a page keeps (`WebView2`'s item names): history and reload, editing, and copying a link's,
/// an image's or a media's address or the image itself; the rest (save, print, share, inspect, open in a new window,
/// web capture) go, as the WPE host's menu leaves them out.
#[must_use]
pub fn keeps_menu_item(name: &str) -> bool {
    matches!(
        name,
        "back"
            | "forward"
            | "reload"
            | "undo"
            | "redo"
            | "cut"
            | "copy"
            | "paste"
            | "pasteAndMatchStyle"
            | "selectAll"
            | "copyLinkLocation"
            | "copyImage"
            | "copyImageLocation"
            | "copyMediaLocation"
            | "emoji"
    )
}

/// Whether menu item `item` is a separator, and its name.
fn menu_item(item: &ICoreWebView2ContextMenuItem) -> ComResult<(bool, String)> {
    let mut kind = COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND::default();
    // SAFETY: the event's own item, read on its thread.
    unsafe { item.Kind(&raw mut kind) }?;
    let name = string(|out| {
        // SAFETY: as above.
        unsafe { item.Name(out) }
    })?;
    Ok((kind == COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND_SEPARATOR, name))
}

/// Takes the items a page does not keep out of `items`, and the separators that would lead, trail or repeat.
fn trim_context_menu(items: &ICoreWebView2ContextMenuItemCollection) -> ComResult<()> {
    let mut count = 0u32;
    // SAFETY: the event's own collection, read and changed on its thread.
    unsafe { items.Count(&raw mut count) }?;
    // back to front, so a removal leaves the indexes still to visit in place; `kept` is whether an item after this
    // one stays
    let mut kept = false;
    for index in (0..count).rev() {
        // SAFETY: as above.
        let item = unsafe { items.GetValueAtIndex(index) }?;
        let (separator, name) = menu_item(&item)?;
        let keep = if separator { kept } else { keeps_menu_item(&name) };
        if keep {
            kept = !separator;
        } else {
            // SAFETY: as above.
            unsafe { items.RemoveValueAtIndex(index) }?;
        }
    }
    // SAFETY: as above.
    unsafe { items.Count(&raw mut count) }?;
    if count > 0 {
        // SAFETY: as above.
        let first = unsafe { items.GetValueAtIndex(0) }?;
        if menu_item(&first)?.0 {
            // SAFETY: as above.
            unsafe { items.RemoveValueAtIndex(0) }?;
        }
    }
    Ok(())
}

// MARK: - the grant's protocol

/// Answers a read of the grant's protocol: the file when it sits inside the grant, read on a thread of its own, else
/// the refusal, kept for a failed main-frame load to report.
fn serve_granted(grant: &Grant, uri: &str, reply: RequestAsyncResponder) {
    let app = scheme::request_app_uri(uri);
    let (root, refused) = (Arc::clone(&grant.root), Arc::clone(&grant.refused));
    let spawned = std::thread::Builder::new().name("page-file".to_owned()).spawn(move || {
        let root = root.lock().unwrap_or_else(PoisonError::into_inner).clone();
        let answer = read_granted(root.as_deref(), &app);
        if let Err(message) = &answer {
            let mut refused = refused.lock().unwrap_or_else(PoisonError::into_inner);
            refused.replace((app, message.clone()));
        }
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
    });
    if let Err(err) = spawned {
        tracing::warn!(%err, "a page's file read did not start");
    }
}

/// The file `app` names, inside the grant `root`: its MIME type and bytes, or the refusal's text.
fn read_granted(root: Option<&std::path::Path>, app: &str) -> Result<(String, Vec<u8>), String> {
    let requested = url::Url::parse(app).ok().and_then(|url| url.to_file_path().ok());
    let path = super::super::policy::granted_file(root, requested.as_deref()).map_err(str::to_owned)?;
    let body = std::fs::read(&path).map_err(|err| err.to_string())?;
    Ok((ngterm_core::remote::page_serve::mime_type(&path), body))
}

// MARK: - the text protocol

/// Answers a read of the text protocol: the page's text for its document, nothing else.
fn serve_text(text: &Text, uri: &str) -> wry::http::Response<std::borrow::Cow<'static, [u8]>> {
    let body = scheme::requests_text_document(uri)
        .then(|| text.lock().unwrap_or_else(PoisonError::into_inner).clone())
        .flatten();
    let (status, mime, body) = match body {
        Some(body) => (200, "text/html; charset=utf-8", body.into_bytes()),
        None => (404, "text/plain; charset=utf-8", Vec::new()),
    };
    wry::http::Response::builder()
        .status(status)
        .header("Content-Type", mime)
        .header("Cache-Control", "no-store")
        .body(std::borrow::Cow::Owned(body))
        .unwrap_or_default()
}

// MARK: - the saved store's clear

/// Empties profile `profile` of `store`: a hidden web view on it in window `parent` asks `WebView2` to clear its
/// browsing data; `done` gets the outcome, after which the web view goes. Pumps the window's messages while the web
/// view is made (module docs). The environment it ran in, for the pages after it.
pub fn clear_profile(
    parent: isize,
    store: Store,
    done: Box<dyn FnOnce(Result<(), String>)>,
) -> Option<ICoreWebView2Environment> {
    let mut context = WebContext::new(Some(store.user_data));
    let mut builder = WebViewBuilder::new_with_web_context(&mut context)
        .with_visible(false)
        .with_devtools(false)
        .with_profile_name(store.profile)
        .with_bounds(
            Bounds {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            }
            .rect(),
        );
    if let Some(environment) = store.environment {
        builder = builder.with_environment(environment);
    }
    let view = match builder.build_as_child(&Parent(parent)) {
        Ok(view) => view,
        Err(err) => {
            done(Err(err.to_string()));
            return None;
        }
    };
    let environment = view.environment();
    let profile = view
        .webview()
        .cast::<ICoreWebView2_13>()
        .and_then(|webview| {
            // SAFETY: a COM call on the web view's own thread.
            unsafe { webview.Profile() }
        })
        .and_then(|profile| profile.cast::<ICoreWebView2Profile2>());
    match profile {
        Ok(profile) => start_clear(&profile, view, done),
        Err(err) => done(Err(err.to_string())),
    }
    Some(environment)
}

/// Starts the clear of `profile`, `view` kept until it is answered.
fn start_clear(profile: &ICoreWebView2Profile2, view: WebView, done: Box<dyn FnOnce(Result<(), String>)>) {
    let held = Rc::new(RefCell::new(Some(view)));
    let answer = Rc::new(RefCell::new(Some(done)));
    let (kept, answered) = (Rc::clone(&held), Rc::clone(&answer));
    let handler = ClearBrowsingDataCompletedHandler::create(Box::new(move |result| {
        // the hidden web view goes with the answer
        kept.take();
        if let Some(done) = answered.take() {
            done(result.map_err(|err| err.to_string()));
        }
        Ok(())
    }));
    // SAFETY: a COM call on the web view's own thread; the handler keeps the web view until it is answered.
    let started = unsafe { profile.ClearBrowsingDataAll(&handler) };
    if let Err(err) = started {
        held.take();
        if let Some(done) = answer.take() {
            done(Err(err.to_string()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_context_menu_keeps_editing_history_and_copies_only() {
        for kept in [
            "back",
            "reload",
            "copy",
            "paste",
            "selectAll",
            "copyLinkLocation",
            "copyImage",
        ] {
            assert!(keeps_menu_item(kept), "{kept}");
        }
        for refused in [
            "saveAs",
            "print",
            "inspectElement",
            "openLinkInNewWindow",
            "saveLinkAs",
            "saveImageAs",
            "share",
            "webCapture",
            "createQrCode",
            "",
        ] {
            assert!(!keeps_menu_item(refused), "{refused}");
        }
    }

    #[test]
    fn bounds_never_collapse_to_nothing() {
        let rect = Bounds {
            x: 10.0,
            y: 29.0,
            width: 0.0,
            height: 300.5,
        }
        .rect();
        let size = rect.size.to_logical::<f64>(1.0);
        assert!((size.width - 1.0).abs() < f64::EPSILON && (size.height - 300.5).abs() < f64::EPSILON);
    }
}
