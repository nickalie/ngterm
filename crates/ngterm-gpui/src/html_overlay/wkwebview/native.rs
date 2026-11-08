//! `WKWebView` through wry, with native navigation and UI delegates enforcing the page policy.
use super::super::policy::GrantRoot;
use super::scheme;
use raw_window_handle::{AppKitWindowHandle, HandleError, HasWindowHandle, RawWindowHandle, WindowHandle};
use std::ffi::{CStr, CString, c_char, c_void};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError};
use wry::dpi::{LogicalPosition, LogicalSize};
use wry::{Rect, RequestAsyncResponder, WebView, WebViewBuilder, WebViewBuilderExtDarwin, WebViewExtMacOS};
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
    /// A real main-frame provisional load began (not a fragment or frame navigation).
    Started,
    /// A platform error, already translated by `WebKit`.
    Failed {
        id: u64,
        reason: String,
    },
    Interrupted {
        id: u64,
        reason: String,
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
    /// A main-frame navigation ended: `status` is the cancellation code (-999) or zero, `http` the response's status code.
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
    /// The network filter of a page that blocks the network ([`Spec::block_network`]) is in place: its document may
    /// load.
    NetworkFilterInstalled,
    /// That filter could not be installed: the page loads nothing.
    NetworkFilterFailed(String),
    /// A text page's renderer reported (its JSON object), through its handler in the renderer's world.
    TextPage(String),
}

/// Decides a navigation at once.
pub type Decide = Rc<dyn Fn(&Navigation) -> bool>;
/// Posts a report to the pump.
pub type Post = Rc<dyn Fn(Report)>;
/// The keystroke the app takes from a key pressed in the page, by its virtual key.
pub type TakeKey = Rc<dyn Fn(u32) -> Option<gpui_kit::Keystroke>>;

/// How the grant's protocol answers: the page's grant, and the last read it refused (an address and
/// why), which a failed main-frame load reports.
#[derive(Clone, Default)]
pub struct Grant {
    pub root: Arc<Mutex<Option<GrantRoot>>>,
    pub refused: Arc<Mutex<Option<(String, String)>>>,
}

/// The text a file page without a grant shows, which its text protocol serves (`scheme`); `None` before its load.
pub type Text = Arc<Mutex<Option<String>>>;

/// How a replica's protocol is answered: its read, the app's address and the responder.
pub type ReplicaRead = Arc<dyn Fn(String, RequestAsyncResponder) + Send + Sync>;

/// Where a page's store is: the environment the pages share once one exists, else the folder a new one keeps its
/// data in, and the profile.
#[derive(Debug, Clone)]
pub struct Environment;

#[derive(Clone)]
pub struct Store {
    pub environment: Option<Environment>,
    pub user_data: PathBuf,
    pub profile: String,
    /// An `InPrivate` profile: kept in memory.
    pub private: bool,
}

/// What a page's web view is built from.
#[expect(
    clippy::struct_excessive_bools,
    reason = "a native page's independent settings, each read once where the page is built"
)]
pub struct Spec {
    pub parent: isize,
    pub store: Store,
    pub javascript: bool,
    /// The page's canvas where it draws nothing; `None` keeps the browser's white.
    pub background: Option<(u8, u8, u8)>,
    pub zoom: f64,
    pub bounds: Option<Bounds>,
    pub visible: bool,
    /// The scripts in load order: the theme first (replaced by [`Native::set_theme`]), then the rest. `None` for a
    /// page that wears no theme (an image).
    pub theme: Option<String>,
    pub scripts: Vec<String>,
    /// App-owned document renderer, installed in its own content world with authored scripts still disabled.
    pub trusted_document_script: Option<String>,
    /// The renderer is a text page's, whose reports and calls go through the renderer's channel (`text_page`).
    pub text_channel: bool,
    pub grant: Option<Grant>,
    pub replica: Option<ReplicaRead>,
    pub text: Option<Text>,
    /// The page loads nothing from the network ([`ngterm_core::html::HtmlOverlay::blocks_network`]): a content rule
    /// list blocking every network scheme goes on it, reported once in place.
    pub block_network: bool,
    pub decide: Decide,
    pub post: Post,
    pub take_key: TakeKey,
}

struct Parent(isize);
impl HasWindowHandle for Parent {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let ptr = std::ptr::NonNull::new(self.0 as *mut c_void).ok_or(HandleError::Unavailable)?;
        // SAFETY: GPUI's NSView outlives the web view's attachment.
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::AppKit(AppKitWindowHandle::new(ptr))) })
    }
}
struct Callbacks {
    decide: Decide,
    post: Post,
    take_key: TakeKey,
}
unsafe extern "C" {
    fn ngterm_web_attach(
        view: *mut c_void,
        context: *mut c_void,
        decide: extern "C" fn(*mut c_void, *const c_char, bool, bool, bool, u64) -> bool,
        report: extern "C" fn(*mut c_void, *const c_char),
        key: extern "C" fn(*mut c_void, u32) -> bool,
    ) -> *mut c_void;
    fn ngterm_web_detach(delegate: *mut c_void);
    fn ngterm_web_reparent(view: *mut c_void, parent: *mut c_void);
    fn ngterm_web_focus_parent(view: *mut c_void);
    fn ngterm_web_reply(view: *mut c_void, json: *const c_char);
    fn ngterm_web_theme(view: *mut c_void, script: *const c_char);
    fn ngterm_web_document_script(view: *mut c_void, script: *const c_char, world: *const c_char);
    fn ngterm_web_text_channel(delegate: *mut c_void, name: *const c_char, world: *const c_char);
    fn ngterm_web_world_call(
        view: *mut c_void,
        script: *const c_char,
        world: *const c_char,
        context: *mut c_void,
        done: extern "C" fn(*mut c_void, *const c_char, *const c_char),
    );
    fn ngterm_web_block_network(
        view: *mut c_void,
        context: *mut c_void,
        done: extern "C" fn(*mut c_void, bool, *const c_char),
    );
}
/// The rule list's answer: the page's post, given back once.
extern "C" fn network_blocked(context: *mut c_void, installed: bool, message: *const c_char) {
    // SAFETY: the Box leaked for this install, which the adapter answers exactly once on the main thread.
    let post = unsafe { Box::from_raw(context.cast::<Post>()) };
    let report = if installed {
        Report::NetworkFilterInstalled
    } else if message.is_null() {
        Report::NetworkFilterFailed("the content rule list did not compile".to_owned())
    } else {
        // SAFETY: the adapter passes a live NUL-terminated message for this call.
        Report::NetworkFilterFailed(unsafe { CStr::from_ptr(message) }.to_string_lossy().into_owned())
    };
    post(report);
}
extern "C" fn decide(
    context: *mut c_void,
    uri: *const c_char,
    main_frame: bool,
    user: bool,
    redirect: bool,
    id: u64,
) -> bool {
    // SAFETY: the delegate's context and string are alive during this synchronous call.
    let callbacks = unsafe { &*context.cast::<Callbacks>() };
    // SAFETY: the delegate passes a live NUL-terminated URL.
    let uri = unsafe { CStr::from_ptr(uri) }.to_string_lossy().into_owned();
    let navigation = Navigation {
        uri,
        main_frame,
        user,
        redirect,
        id,
    };
    let allowed = (callbacks.decide)(&navigation);
    tracing::debug!(uri = %navigation.uri, main_frame, redirect, allowed, "WKWebView navigation policy");
    // Policy applies to all frames; only main-frame refusals affect the document's load tracker.
    if main_frame && !allowed {
        (callbacks.post)(Report::Navigation { navigation, allowed });
    }
    allowed
}
extern "C" fn report(context: *mut c_void, json: *const c_char) {
    // SAFETY: the delegate owns neither pointer and calls only while Native lives.
    let callbacks = unsafe { &*context.cast::<Callbacks>() };
    // SAFETY: the delegate passes a live NUL-terminated JSON string.
    let json = unsafe { CStr::from_ptr(json) }.to_string_lossy();
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&json) else {
        return;
    };
    let text = |key| value[key].as_str().unwrap_or_default().to_owned();
    let event = match value["kind"].as_str().unwrap_or_default() {
        "started" => Report::Started,
        "failure" => Report::Failed {
            id: value["id"].as_u64().unwrap_or(0),
            reason: text("reason"),
        },
        "interrupted" => Report::Interrupted {
            id: value["id"].as_u64().unwrap_or(0),
            reason: text("reason"),
        },
        "message" => Report::Message(text("body")),
        "text" => Report::TextPage(text("body")),
        "download" => Report::DownloadRefused(text("url")),
        "focus" => Report::Focus(value["focused"].as_bool().unwrap_or(false)),
        "commit" => Report::Committed,
        "source" => Report::Source(text("url")),
        "title" => Report::Title(text("title")),
        "history" => Report::History {
            back: value["back"].as_bool().unwrap_or(false),
            forward: value["forward"].as_bool().unwrap_or(false),
        },
        "complete" => Report::Completed {
            id: value["id"].as_u64().unwrap_or(0),
            success: value["success"].as_bool().unwrap_or(false),
            status: value["status"]
                .as_i64()
                .and_then(|v| i32::try_from(v).ok())
                .unwrap_or(0),
            http: value["http"]
                .as_i64()
                .and_then(|value| i32::try_from(value).ok())
                .unwrap_or(0),
        },
        "close" => Report::CloseRequested,
        "process" => Report::ProcessFailed { browser: false },
        "new" => Report::NewWindow {
            uri: text("url"),
            user: value["user"].as_bool().unwrap_or(false),
        },
        _ => return,
    };
    (callbacks.post)(event);
}
extern "C" fn take_key(context: *mut c_void, code: u32) -> bool {
    // SAFETY: the delegate is detached before its stable callback Box is dropped.
    let callbacks = unsafe { &*context.cast::<Callbacks>() };
    if let Some(key) = (callbacks.take_key)(code) {
        (callbacks.post)(Report::Key(key));
        true
    } else {
        false
    }
}
/// A renderer call's answer: the sender leaked for it, given back once with the value's JSON or the error.
extern "C" fn called(context: *mut c_void, json: *const c_char, error: *const c_char) {
    // SAFETY: the Box leaked for this call, which the adapter answers exactly once on the main thread.
    let sender = unsafe { Box::from_raw(context.cast::<async_channel::Sender<super::super::text_page::Answer>>()) };
    let answer = if error.is_null() {
        // SAFETY: the adapter passes null or a live NUL-terminated JSON string for this call.
        Ok((!json.is_null()).then(|| unsafe { CStr::from_ptr(json) }.to_string_lossy().into_owned()))
    } else {
        // SAFETY: the adapter passes a live NUL-terminated message for this call.
        Err(unsafe { CStr::from_ptr(error) }.to_string_lossy().into_owned())
    };
    let _ = sender.try_send(answer);
}
pub struct Native {
    view: WebView,
    delegate: *mut c_void,
    _callbacks: Box<Callbacks>,
    post: Post,
}
impl Drop for Native {
    fn drop(&mut self) {
        // SAFETY: removes delegates before the callbacks and web view are released.
        unsafe { ngterm_web_detach(self.delegate) };
    }
}
impl Native {
    pub fn create(spec: &Spec) -> Result<Self, String> {
        let view = build(spec)?;
        if let Some(script) = &spec.trusted_document_script {
            let script = CString::new(script.as_str()).map_err(|error| error.to_string())?;
            let world = CString::new(super::super::documents::WORLD).map_err(|error| error.to_string())?;
            // SAFETY: the native adapter copies both live strings into a document-end script in that named world.
            unsafe {
                ngterm_web_document_script(
                    objc2::rc::Retained::as_ptr(&view.webview()).cast_mut().cast::<c_void>(),
                    script.as_ptr(),
                    world.as_ptr(),
                );
            }
        }
        if spec.block_network {
            let context = Box::into_raw(Box::new(spec.post.clone())).cast();
            // SAFETY: a main-thread web view; the completion takes the context back once.
            unsafe {
                ngterm_web_block_network(
                    objc2::rc::Retained::as_ptr(&view.webview()).cast_mut().cast::<c_void>(),
                    context,
                    network_blocked,
                );
            }
        }
        let mut callbacks = Box::new(Callbacks {
            decide: spec.decide.clone(),
            post: spec.post.clone(),
            take_key: spec.take_key.clone(),
        });
        // SAFETY: the stable Box lives until the delegate is detached in Drop.
        let delegate = unsafe {
            ngterm_web_attach(
                objc2::rc::Retained::as_ptr(&view.webview()).cast_mut().cast::<c_void>(),
                (&raw mut *callbacks).cast(),
                decide,
                report,
                take_key,
            )
        };
        if spec.text_channel {
            let name = CString::new(super::super::text_page::HANDLER).map_err(|error| error.to_string())?;
            let world = CString::new(super::super::documents::WORLD).map_err(|error| error.to_string())?;
            // SAFETY: the live delegate registers itself as the handler in that world, removed again at its detach.
            unsafe { ngterm_web_text_channel(delegate, name.as_ptr(), world.as_ptr()) };
        }
        Ok(Self {
            view,
            delegate,
            _callbacks: callbacks,
            post: spec.post.clone(),
        })
    }
    #[expect(
        clippy::unused_self,
        reason = "shared native host API; WKWebView has no environment object"
    )]
    pub fn environment(&self) -> Environment {
        Environment
    }
    pub fn set_theme(&self, script: &str) {
        if let Ok(script) = CString::new(script) {
            // SAFETY: main-thread web view and a live string copied by the adapter.
            unsafe {
                ngterm_web_theme(
                    objc2::rc::Retained::as_ptr(&self.view.webview())
                        .cast_mut()
                        .cast::<c_void>(),
                    script.as_ptr(),
                );
            };
            (self.post)(Report::ThemeInstalled);
        }
    }
    pub fn load_url(&self, address: &str) {
        let _ = self.view.load_url(address);
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
    pub fn set_background(&self, (r, g, b): (u8, u8, u8)) {
        let _ = self.view.set_background_color((r, g, b, 255));
    }
    pub fn set_bounds(&self, bounds: Bounds) {
        let _ = self.view.set_bounds(bounds.rect());
    }
    pub fn set_visible(&self, visible: bool) {
        let _ = self.view.set_visible(visible);
    }
    pub fn focus(&self) {
        let _ = self.view.focus();
    }
    pub fn focus_parent(&self) {
        // SAFETY: the adapter resolves the current parent after any reparenting.
        unsafe { ngterm_web_focus_parent(objc2::rc::Retained::as_ptr(&self.view.webview()).cast_mut().cast()) };
    }
    pub fn reparent(&self, parent: isize) {
        // SAFETY: both pointers are live main-thread NSViews; the adapter preserves the WKWebView.
        unsafe {
            ngterm_web_reparent(
                objc2::rc::Retained::as_ptr(&self.view.webview())
                    .cast_mut()
                    .cast::<c_void>(),
                parent as *mut c_void,
            );
        };
    }
    /// Evaluates `expression` in the renderer's world (`callAsyncJavaScript`, so a promise is awaited); its answer
    /// arrives on the returned channel.
    pub fn call_renderer(&self, expression: &str) -> async_channel::Receiver<super::super::text_page::Answer> {
        let (sender, answer) = async_channel::bounded(1);
        let (Ok(script), Ok(world)) = (
            CString::new(format!("return ({expression});")),
            CString::new(super::super::documents::WORLD),
        ) else {
            return answer;
        };
        let context = Box::into_raw(Box::new(sender)).cast();
        // SAFETY: a main-thread web view; the adapter copies both strings and answers the context once.
        unsafe {
            ngterm_web_world_call(
                objc2::rc::Retained::as_ptr(&self.view.webview()).cast_mut().cast(),
                script.as_ptr(),
                world.as_ptr(),
                context,
                called,
            );
        };
        answer
    }
    pub fn post_json(&self, json: &str) {
        if let Ok(json) = CString::new(json) {
            // SAFETY: the native handler completes the matching main-frame request, copying the JSON.
            unsafe {
                ngterm_web_reply(
                    objc2::rc::Retained::as_ptr(&self.view.webview()).cast_mut().cast(),
                    json.as_ptr(),
                );
            };
        }
    }
}
fn build(spec: &Spec) -> Result<WebView, String> {
    let mut builder = WebViewBuilder::new()
        .with_visible(spec.visible)
        .with_focused(false)
        .with_devtools(false)
        .with_hotkeys_zoom(false)
        .with_clipboard(false)
        .with_autoplay(false)
        .with_back_forward_navigation_gestures(false)
        .with_incognito(spec.store.private)
        .with_drag_drop_handler(|_| true);
    if !spec.store.private {
        let id = spec
            .store
            .profile
            .strip_prefix("ngterm-")
            .and_then(|id| uuid::Uuid::parse_str(id).ok())
            .ok_or("invalid browser profile")?;
        builder = builder.with_data_store_identifier(*id.as_bytes());
    }
    if !spec.javascript {
        builder = builder.with_javascript_disabled();
    }
    if let Some((r, g, b)) = spec.background {
        builder = builder.with_background_color((r, g, b, 255));
    }
    if let Some(bounds) = spec.bounds {
        builder = builder.with_bounds(bounds.rect());
    }
    if let Some(theme) = &spec.theme {
        builder = builder.with_initialization_script(format!("/* ngterm-theme */\n{theme}"));
    }
    for script in &spec.scripts {
        builder =
            builder.with_initialization_script_for_main_only(script.clone(), script != super::bridge::GUARD_SCRIPT);
    }
    let post = spec.post.clone();
    builder = builder.with_ipc_handler(move |request| post(Report::Message(request.body().clone())));
    if let Some(grant) = spec.grant.clone() {
        builder = builder.with_asynchronous_custom_protocol(scheme::GRANT_PROTOCOL.into(), move |_, request, reply| {
            serve_granted(&grant, &request.uri().to_string(), reply);
        });
    }
    if let Some(text) = spec.text.clone() {
        builder = builder.with_custom_protocol(scheme::TEXT_PROTOCOL.into(), move |_, request| {
            serve_text(&text, &request.uri().to_string())
        });
    }
    if let Some(replica) = spec.replica.clone() {
        builder =
            builder.with_asynchronous_custom_protocol(scheme::REPLICA_PROTOCOL.into(), move |_, request, reply| {
                replica(scheme::request_app_uri(&request.uri().to_string()), reply);
            });
    }
    let view = builder
        .build_as_child(&Parent(spec.parent))
        .map_err(|err| err.to_string())?;
    let _ = view.zoom(spec.zoom);
    Ok(view)
}
#[expect(
    clippy::needless_pass_by_value,
    reason = "shared native host API transfers the store to the clear operation"
)]
pub fn clear_profile(
    _parent: isize,
    store: Store,
    done: Box<dyn FnOnce(Result<(), String>) + Send>,
) -> Option<Environment> {
    type Done = Box<dyn FnOnce(Result<(), String>) + Send>;
    extern "C" fn cleared(context: *mut c_void) {
        // SAFETY: the completion owns the Box allocated for this clear and is invoked once.
        let done = unsafe { Box::from_raw(context.cast::<Done>()) };
        done(Ok(()));
    }
    unsafe extern "C" {
        fn ngterm_web_clear(uuid: *const u8, context: *mut c_void, done: extern "C" fn(*mut c_void));
    }
    let Some(id) = store
        .profile
        .strip_prefix("ngterm-")
        .and_then(|id| uuid::Uuid::parse_str(id).ok())
    else {
        done(Err("invalid browser profile".into()));
        return None;
    };
    let context = Box::into_raw(Box::new(done)).cast();
    // SAFETY: the UUID is copied and the completion assumes ownership of context.
    unsafe { ngterm_web_clear(id.as_bytes().as_ptr(), context, cleared) };
    Some(Environment)
}
fn serve_granted(grant: &Grant, uri: &str, reply: RequestAsyncResponder) {
    let app = scheme::request_app_uri(uri);
    let (root, refused) = (Arc::clone(&grant.root), Arc::clone(&grant.refused));
    let spawned = std::thread::Builder::new().name("page-file".to_owned()).spawn(move || {
        let root = root.lock().unwrap_or_else(PoisonError::into_inner).clone();
        let answer = read_granted(root.as_ref(), &app);
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
fn read_granted(root: Option<&GrantRoot>, app: &str) -> Result<(String, Vec<u8>), String> {
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
