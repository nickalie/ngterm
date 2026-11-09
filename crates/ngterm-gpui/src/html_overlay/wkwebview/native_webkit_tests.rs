//! A real `WKWebView`, through every call a page makes of it. `AppKit` and `WebKit` want the process's main thread,
//! which libtest keeps for itself, so the checks run in a child of this test binary, in a function the loader runs
//! before `main`; libtest then finds no test to run and exits, writing the child's coverage as any test's. `WebKit`'s
//! stores go under the child's own home, which the test removes.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{Message as _, class, msg_send};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use tempfile::TempDir;
use uuid::Uuid;

use super::super::super::super::policy::OUTSIDE_GRANT;
use super::super::super::native_view;
use super::super::*;
use super::{Log, memory_spec};

/// Set in the child: run the checks.
const CHILD: &str = "WKWEBVIEW_CHECKS_CHILD";
/// Printed once the checks finish, so that a child that ran nothing fails.
const PASSED: &str = "every WKWebView check passed";

#[test]
fn a_real_web_view_does_what_a_page_asks() {
    let home = TempDir::new().unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap());
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("NGTERM_") {
            child.env_remove(key);
        }
    }
    let output = child
        .args(["--exact", "the checks already ran"])
        .env(CHILD, "1")
        .env("CFFIXED_USER_HOME", home.path())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() && stdout.contains(PASSED),
        "{}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        output.status
    );
}

#[used]
#[unsafe(link_section = "__DATA,__mod_init_func")]
static BEFORE_MAIN: extern "C" fn() = before_main;

extern "C" fn before_main() {
    if std::env::var_os(CHILD).is_some() {
        // A panic cannot leave an `extern "C"` function: it would abort. The panic hook has printed the failed check by
        // now, for the test to report.
        if std::panic::catch_unwind(|| objc2::rc::autoreleasepool(|_| run())).is_err() {
            std::process::exit(1);
        }
        println!("{PASSED}");
    }
}

// MARK: - the stage

fn pump_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        // SAFETY: the main thread; these selectors return live autoreleased Foundation objects.
        let run_loop: *mut AnyObject = unsafe { msg_send![class!(NSRunLoop), currentRunLoop] };
        // SAFETY: as above.
        let until: *mut AnyObject = unsafe { msg_send![class!(NSDate), dateWithTimeIntervalSinceNow: 0.02f64] };
        // SAFETY: the main run loop and the live date returned above.
        let _: () = unsafe { msg_send![run_loop, runUntilDate: until] };
    }
}

struct Stage {
    window: Retained<AnyObject>,
    parent: Retained<AnyObject>,
    other: Retained<AnyObject>,
}

impl Stage {
    fn new() -> Self {
        // SAFETY: the main thread; the shared application is retained by AppKit.
        let _app: *mut AnyObject = unsafe { msg_send![class!(NSApplication), sharedApplication] };
        let frame = NSRect::new(NSPoint::new(0., 0.), NSSize::new(640., 480.));
        // SAFETY: the main thread; an allocation initialized at once below.
        let allocated: *mut AnyObject = unsafe { msg_send![class!(NSWindow), alloc] };
        // SAFETY: a valid frame, titled style and buffered backing; the window is never ordered on screen.
        let window: *mut AnyObject = unsafe {
            msg_send![allocated, initWithContentRect: frame, styleMask: 1usize, backing: 2usize, defer: false]
        };
        // SAFETY: init returned an owned NSWindow, whose ownership this takes.
        let window = unsafe { Retained::from_raw(window) }.unwrap();
        // SAFETY: the main thread; a new view, owned here and retained by the window once added.
        let parent: Retained<AnyObject> = unsafe { msg_send![class!(NSView), new] };
        // SAFETY: as above.
        let other: Retained<AnyObject> = unsafe { msg_send![class!(NSView), new] };
        // SAFETY: the window retains its content view.
        let _: () = unsafe { msg_send![&*window, setContentView: &*parent] };
        // SAFETY: the content view retains its subview.
        let _: () = unsafe { msg_send![&*parent, addSubview: &*other] };
        Self { window, parent, other }
    }

    fn parent_handle(&self) -> isize {
        Retained::as_ptr(&self.parent) as isize
    }
}

fn completed(log: &Log) -> usize {
    log.borrow()
        .iter()
        .filter(|report| matches!(report, Report::Completed { success: true, .. }))
        .count()
}

fn wait_for(log: &Log, what: &str, wanted: impl Fn(&Report) -> bool) {
    pump_until(what, || log.borrow().iter().any(&wanted));
}

fn load(native: &Native, log: &Log, address: &str) {
    let before = completed(log);
    native.load_url(address);
    // The count alone could be met by an earlier load still ending, as a refused page's does.
    pump_until(address, || completed(log) > before && !is_loading(native));
}

fn is_loading(native: &Native) -> bool {
    let web = web_object(native);
    // SAFETY: the main thread and a live view.
    unsafe { msg_send![&*web, isLoading] }
}

/// Waits for the load in flight to end. The history's cursor moves, and is reported, while a step's load is still
/// running, and a `WKWebView` loses a step asked for by then: the first step's load goes on, and the cursor stays.
fn settle(native: &Native, what: &str) {
    pump_until(what, || !is_loading(native));
}

/// Runs `script` in the page's own world: the value as JSON.
fn page_value(native: &Native, script: &str) -> String {
    let value = Arc::new(Mutex::new(None));
    let slot = Arc::clone(&value);
    native
        .view
        .evaluate_script_with_callback(script, move |answer| *slot.lock().unwrap() = Some(answer))
        .unwrap();
    pump_until(script, || value.lock().unwrap().is_some());
    value.lock().unwrap().take().unwrap()
}

fn renderer_answer(native: &dyn NativeView, expression: &str) -> super::super::super::super::text_page::Answer {
    let answer = native.call_renderer(expression);
    let mut got = None;
    pump_until(expression, || {
        got = answer.try_recv().ok();
        got.is_some()
    });
    got.unwrap()
}

// MARK: - the checks

struct Protocols {
    grant: Grant,
    served: Arc<Mutex<Vec<String>>>,
    directory: TempDir,
    page: Uuid,
}

fn full_spec(stage: &Stage, log: &Log) -> (Spec, Protocols) {
    let directory = TempDir::new().unwrap();
    std::fs::write(directory.path().join("a.html"), "<title>granted</title><p>a</p>").unwrap();
    let grant = Grant::default();
    *grant.root.lock().unwrap() = GrantRoot::new(directory.path());
    let served = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&served);
    let replica: ReplicaRead = Arc::new(move |app: String, reply: RequestAsyncResponder| {
        seen.lock().unwrap().push(app);
        let response = wry::http::Response::builder()
            .status(200)
            .header("Content-Type", "text/html")
            .body(b"<title>replica</title>".to_vec())
            .unwrap();
        reply.respond(response);
    });
    let spec = Spec {
        background: Some((1, 2, 3)),
        bounds: Some(Bounds {
            x: 0.0,
            y: 0.0,
            width: 640.0,
            height: 480.0,
        }),
        theme: Some("window.themed = 'old';".to_owned()),
        scripts: vec!["window.scripted = 1;".to_owned()],
        trusted_document_script: Some("document.documentElement.dataset.renderer = 'done';".to_owned()),
        text_channel: true,
        grant: Some(grant.clone()),
        replica: Some(replica),
        text: Some(Arc::new(Mutex::new(Some("<title>text doc</title><p>t</p>".to_owned())))),
        decide: Rc::new(|navigation| !navigation.uri.contains("refused.invalid")),
        ..memory_spec(stage.parent_handle(), log)
    };
    let protocols = Protocols {
        grant,
        served,
        directory,
        page: Uuid::new_v4(),
    };
    (spec, protocols)
}

/// The page's scripts ran in their worlds, on the app's text protocol.
fn scripts_and_worlds(native: &Native, log: &Log, protocols: &Protocols) {
    load(native, log, &scheme::text_address(protocols.page));
    wait_for(log, "the title", |report| {
        *report == Report::Title("text doc".to_owned())
    });
    assert!(log.borrow().contains(&Report::Started));
    assert!(log.borrow().contains(&Report::Committed));
    assert_eq!(page_value(native, "window.scripted"), "1");
    assert_eq!(page_value(native, "window.themed"), "\"old\"");
    assert_eq!(
        renderer_answer(native, "document.documentElement.dataset.renderer"),
        Ok(Some("\"done\"".to_owned())),
        "the trusted script runs in the renderer's world, at the document's end"
    );
    assert_eq!(
        renderer_answer(native, "typeof window.scripted"),
        Ok(Some("\"undefined\"".to_owned())),
        "the page's world stays apart from the renderer's"
    );
}

/// A new theme replaces the old one for later documents, and says when it is in place.
fn theme_replacement(native: &Native, log: &Log) {
    native.set_theme("window.themed = 'new';");
    wait_for(log, "the theme", |report| *report == Report::ThemeInstalled);
    let before = completed(log);
    native.reload();
    pump_until("the reload", || completed(log) > before);
    assert_eq!(page_value(native, "window.themed"), "\"new\"");
    assert_eq!(page_value(native, "window.scripted"), "1", "the other scripts stay");
}

/// The channels between the page and the app, both ways.
fn channels(native: &Native, log: &Log) {
    assert_eq!(renderer_answer(native, "1 + 1"), Ok(Some("2".to_owned())));
    assert_eq!(renderer_answer(native, "undefined"), Ok(None));
    assert!(renderer_answer(native, "(() => { throw new Error('boom'); })()").is_err());
    assert_eq!(
        renderer_answer(
            native,
            "window.webkit.messageHandlers.ngtermTextPage.postMessage({line: 3})"
        ),
        Ok(None)
    );
    wait_for(log, "the renderer's report", |report| {
        *report == Report::TextPage(r#"{"line":3}"#.to_owned())
    });
    page_value(native, "window.ipc.postMessage('from the page'); 1");
    wait_for(log, "the page's message", |report| {
        *report == Report::Message("from the page".to_owned())
    });
    page_value(
        native,
        "window.webkit.messageHandlers.ngtermBridge.postMessage(JSON.stringify({id: 7}))
           .then(r => { window.bridged = r.id + ':' + r.text; }, e => { window.bridged = 'error:' + e; }); 1",
    );
    wait_for(
        log,
        "the bridge's request",
        |report| matches!(report, Report::Message(body) if body == r#"{"id":7}"#),
    );
    native.post_json(r#"{"id":7,"text":"answer"}"#);
    pump_until("the bridge's answer", || {
        page_value(native, "window.bridged") == "\"7:answer\""
    });
}

/// A `\0` cannot cross into the adapter: nothing happens, and a renderer call ends unanswered.
fn strings_that_cannot_cross(native: &Native, log: &Log) {
    let before = log.borrow().len();
    native.set_theme("window.themed = 'bad\0';");
    native.post_json("{\0}");
    assert_eq!(log.borrow().len(), before, "a refused theme is never installed");
    assert_eq!(
        native.call_renderer("1\0").try_recv(),
        Err(async_channel::TryRecvError::Closed)
    );
    let before = completed(log);
    native.reload();
    pump_until("the theme after a refused replacement", || completed(log) > before);
    assert_eq!(page_value(native, "window.themed"), "\"new\"");
}

/// The app's file protocol: a granted file, a refused one, and the replica's read.
fn protocols(native: &Native, log: &Log, protocols: &Protocols) {
    let file = protocols.directory.path().join("a.html");
    let address = scheme::granted_address(file.to_str().unwrap()).unwrap();
    load(native, log, &address);
    wait_for(log, "the granted title", |report| {
        *report == Report::Title("granted".to_owned())
    });
    let elsewhere = TempDir::new().unwrap();
    let secret = elsewhere.path().join("secret.html");
    std::fs::write(&secret, "secret").unwrap();
    let refused = scheme::granted_address(secret.to_str().unwrap()).unwrap();
    native.load_url(&refused);
    pump_until("the refusal", || protocols.grant.refused.lock().unwrap().is_some());
    let (asked, why) = protocols.grant.refused.lock().unwrap().clone().unwrap();
    assert_eq!((asked, why.as_str()), (scheme::app_uri(&refused), OUTSIDE_GRANT));
    let replica = scheme::replica_address(protocols.page, "/served.html").unwrap();
    load(native, log, &replica);
    wait_for(log, "the replica's title", |report| {
        *report == Report::Title("replica".to_owned())
    });
    assert_eq!(*protocols.served.lock().unwrap(), vec![scheme::app_uri(&replica)]);
}

/// Refusals and the history.
fn navigation(native: &Native, log: &Log) {
    native.load_url("https://refused.invalid/");
    wait_for(log, "the refusal", |report| {
        matches!(report, Report::Navigation { navigation, allowed: false }
            if navigation.uri == "https://refused.invalid/" && navigation.main_frame)
    });
    native.go_back();
    wait_for(log, "the way back", |report| {
        matches!(report, Report::History { forward: true, .. })
    });
    settle(native, "the way back's load");
    log.borrow_mut().clear();
    native.go_forward();
    wait_for(log, "the way forward", |report| {
        matches!(report, Report::History { forward: false, .. })
    });
    settle(native, "the way forward's load");
}

fn web_object(native: &Native) -> Retained<AnyObject> {
    let view = native.view.webview();
    let object: &AnyObject = &view;
    object.retain()
}

/// What the view shows and where it sits.
fn geometry(native: &Native, stage: &Stage) {
    let web = web_object(native);
    native.set_visible(false);
    // SAFETY: the main thread and a live view.
    let hidden: bool = unsafe { msg_send![&*web, isHidden] };
    assert!(hidden);
    native.set_visible(true);
    native.set_bounds(Bounds {
        x: 10.0,
        y: 20.0,
        width: 300.0,
        height: 200.0,
    });
    // SAFETY: as above.
    let frame: NSRect = unsafe { msg_send![&*web, frame] };
    assert_eq!((frame.origin.x, frame.origin.y), (10.0, 260.0));
    assert_eq!((frame.size.width, frame.size.height), (300.0, 200.0));
    native.set_zoom(2.0);
    // SAFETY: as above.
    let zoom: f64 = unsafe { msg_send![&*web, pageZoom] };
    assert!((zoom - 2.0).abs() < f64::EPSILON);
    native.set_zoom(1.0);
    native.set_background((10, 20, 30));
    background(&web);
    // SAFETY: as above; both views are live.
    let parent: *const AnyObject = unsafe { msg_send![&*web, superview] };
    assert_eq!(parent, Retained::as_ptr(&stage.parent));
    native.reparent(Retained::as_ptr(&stage.other) as isize);
    // SAFETY: as above.
    let moved: *const AnyObject = unsafe { msg_send![&*web, superview] };
    assert_eq!(moved, Retained::as_ptr(&stage.other));
    native.reparent(stage.parent_handle());
}

fn background(web: &AnyObject) {
    // SAFETY: the main thread and a live WKWebView; both returned objects live through the autorelease pool.
    let color: *mut AnyObject = unsafe { msg_send![web, underPageBackgroundColor] };
    // SAFETY: a live shared color space, used below to read the color's RGB components.
    let space: *mut AnyObject = unsafe { msg_send![class!(NSColorSpace), sRGBColorSpace] };
    // SAFETY: the color and color space are live objects of the expected classes.
    let color: *mut AnyObject = unsafe { msg_send![color, colorUsingColorSpace: space] };
    assert!(!color.is_null());
    let components: [f64; 4] = [
        // SAFETY: the converted color is in RGB space, so these component selectors are valid.
        unsafe { msg_send![color, redComponent] },
        // SAFETY: as above.
        unsafe { msg_send![color, greenComponent] },
        // SAFETY: as above.
        unsafe { msg_send![color, blueComponent] },
        // SAFETY: as above.
        unsafe { msg_send![color, alphaComponent] },
    ];
    for (actual, expected) in components.into_iter().zip([10.0, 20.0, 30.0, 255.0]) {
        assert!(
            (actual - expected / 255.0).abs() < 1e-6,
            "component {actual}, expected {expected}/255"
        );
    }
}

fn first_responder(stage: &Stage) -> *const AnyObject {
    // SAFETY: the main thread and a live window.
    unsafe { msg_send![&*stage.window, firstResponder] }
}

/// The keyboard moves to the page and back to the window.
fn keyboard(native: &Native, log: &Log, stage: &Stage) {
    let web = web_object(native);
    native.focus();
    assert_eq!(first_responder(stage), Retained::as_ptr(&web));
    wait_for(log, "the page's focus", |report| *report == Report::Focus(true));
    native.focus_parent();
    assert_eq!(first_responder(stage), Retained::as_ptr(&stage.parent));
    wait_for(log, "the window's focus", |report| *report == Report::Focus(false));
}

fn a_full_page(stage: &Stage) {
    let log = Log::default();
    let (spec, protocols_of_page) = full_spec(stage, &log);
    let native = Native::create(&spec).unwrap_or_else(|error| panic!("{error}"));
    let _: Environment = native.environment();
    scripts_and_worlds(&native, &log, &protocols_of_page);
    theme_replacement(&native, &log);
    channels(&native, &log);
    strings_that_cannot_cross(&native, &log);
    protocols(&native, &log, &protocols_of_page);
    navigation(&native, &log);
    geometry(&native, stage);
    keyboard(&native, &log, stage);
}

/// A hidden page made through the seam disables authored scripts and reports its installed network filter.
fn a_locked_down_page(stage: &Stage) {
    let log = Log::default();
    let spec = Spec {
        javascript: false,
        visible: false,
        block_network: true,
        scripts: vec![super::super::super::bridge::GUARD_SCRIPT.to_owned(), "1;".to_owned()],
        text: Some(Arc::new(Mutex::new(Some(
            "<title>locked down</title><script>document.documentElement.dataset.authored = 'ran';</script>".to_owned(),
        )))),
        ..memory_spec(stage.parent_handle(), &log)
    };
    let (view, environment) = native_view::create(&spec).unwrap_or_else(|error| panic!("{error}"));
    assert!(environment.is_some());
    wait_for(&log, "the network filter", |report| {
        *report == Report::NetworkFilterInstalled
    });
    view.load_url(&scheme::text_address(Uuid::new_v4()));
    pump_until("the locked-down document", || completed(&log) > 0);
    assert_eq!(
        renderer_answer(&*view, "document.title"),
        Ok(Some("\"locked down\"".to_owned()))
    );
    assert_eq!(
        renderer_answer(&*view, "document.documentElement.dataset.authored"),
        Ok(None)
    );
    drop(view);
}

/// A saved profile's store is built by its id, and emptied through the seam.
fn a_saved_profile(stage: &Stage) {
    let log = Log::default();
    let id = Uuid::new_v4();
    let store = Store {
        environment: None,
        user_data: PathBuf::new(),
        profile: format!("ngterm-{id}"),
        private: false,
    };
    let spec = Spec {
        store: store.clone(),
        ..memory_spec(stage.parent_handle(), &log)
    };
    drop(Native::create(&spec).unwrap_or_else(|error| panic!("{error}")));
    let answer = Arc::new(Mutex::new(None));
    let slot = Arc::clone(&answer);
    let environment = native_view::clear_profile(
        stage.parent_handle(),
        store,
        Box::new(move |result| *slot.lock().unwrap() = Some(result)),
    );
    assert!(environment.is_some());
    pump_until("the store's clear", || answer.lock().unwrap().is_some());
    assert_eq!(answer.lock().unwrap().take(), Some(Ok(())));
}

/// What a web view cannot be built from.
fn impossible_pages(stage: &Stage) {
    let log = Log::default();
    let refused = |spec: &Spec| Native::create(spec).map(|_| ()).unwrap_err();
    let spec = Spec {
        trusted_document_script: Some("a\0b".to_owned()),
        ..memory_spec(stage.parent_handle(), &log)
    };
    assert_eq!(refused(&spec), "nul byte found in provided data at position: 1");
    assert!(!refused(&memory_spec(0, &log)).is_empty());
    assert!(!native_view::open_browser("https://example.invalid/\0"));
}

fn run() {
    let stage = Stage::new();
    a_full_page(&stage);
    a_locked_down_page(&stage);
    a_saved_profile(&stage);
    impossible_pages(&stage);
    drop(stage);
}
