//! The web view's half that needs no `WebKit`: what its callbacks make of the adapter's answers, and the file and text
//! protocols' answers. The real `WKWebView` is driven in `native_webkit_tests`.

use std::cell::RefCell;
use std::ffi::CString;
use std::os::unix::fs::PermissionsExt as _;

use gpui_kit::Keystroke;
use tempfile::TempDir;
use wry::dpi::{Position, Size};

use super::super::super::policy::{NOT_A_FILE, OUTSIDE_GRANT};
use super::*;

#[path = "native_webkit_tests.rs"]
mod webkit;

pub(super) type Log = Rc<RefCell<Vec<Report>>>;

fn string(value: &str) -> CString {
    CString::new(value).unwrap()
}

/// `spec`'s callbacks over a log of what they posted: a navigation is allowed unless `refuse` names part of it, and the
/// app takes key 7 as `ctrl-a`.
fn callbacks(refuse: &'static str) -> (Box<Callbacks>, Log) {
    let log = Log::default();
    let sink = Rc::clone(&log);
    let callbacks = Box::new(Callbacks {
        decide: Rc::new(move |navigation: &Navigation| refuse.is_empty() || !navigation.uri.contains(refuse)),
        post: Rc::new(move |report| sink.borrow_mut().push(report)),
        take_key: Rc::new(|code| (code == 7).then(|| Keystroke::parse("ctrl-a").unwrap())),
    });
    (callbacks, log)
}

/// A spec of a page that keeps its data in memory and asks for nothing else.
pub(super) fn memory_spec(parent: isize, log: &Log) -> Spec {
    let sink = Rc::clone(log);
    Spec {
        parent,
        store: Store {
            environment: None,
            user_data: PathBuf::new(),
            profile: "page-test".to_owned(),
            private: true,
        },
        javascript: true,
        background: None,
        zoom: 1.0,
        bounds: None,
        visible: true,
        theme: None,
        scripts: Vec::new(),
        trusted_document_script: None,
        text_channel: false,
        grant: None,
        replica: None,
        text: None,
        block_network: false,
        decide: Rc::new(|_| true),
        post: Rc::new(move |report| sink.borrow_mut().push(report)),
        take_key: Rc::new(|_| None),
    }
}

// MARK: - the content area

#[test]
fn a_page_is_placed_in_logical_pixels_and_never_collapses() {
    let bounds = Bounds {
        x: 10.5,
        y: -4.0,
        width: 300.0,
        height: 200.0,
    };
    let rect = bounds.rect();
    assert_eq!(rect.position, Position::Logical(LogicalPosition::new(10.5, -4.0)));
    assert_eq!(rect.size, Size::Logical(LogicalSize::new(300.0, 200.0)));
    let empty = Bounds {
        width: 0.0,
        height: -8.0,
        ..bounds
    };
    assert_eq!(empty.rect().size, Size::Logical(LogicalSize::new(1.0, 1.0)));
}

#[test]
fn only_a_real_view_is_a_parent() {
    assert!(matches!(Parent(0).window_handle(), Err(HandleError::Unavailable)));
    let parent = Parent(0x1000);
    let handle = parent.window_handle().unwrap();
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        panic!("an AppKit view is the parent");
    };
    assert_eq!(appkit.ns_view.as_ptr() as usize, 0x1000);
}

// MARK: - the adapter's callbacks

#[test]
fn each_report_of_the_adapter_reaches_the_page_as_its_kind() {
    for (json, expected) in [
        (r#"{"kind":"started"}"#, Report::Started),
        (
            r#"{"kind":"failure","id":3,"reason":"no route"}"#,
            Report::Failed {
                id: 3,
                reason: "no route".to_owned(),
            },
        ),
        (
            r#"{"kind":"interrupted","id":4,"reason":"download refused"}"#,
            Report::Interrupted {
                id: 4,
                reason: "download refused".to_owned(),
            },
        ),
        (r#"{"kind":"message","body":"m"}"#, Report::Message("m".to_owned())),
        (r#"{"kind":"text","body":"{}"}"#, Report::TextPage("{}".to_owned())),
        (
            r#"{"kind":"download","url":"https://e.invalid/a.zip"}"#,
            Report::DownloadRefused("https://e.invalid/a.zip".to_owned()),
        ),
        (r#"{"kind":"focus","focused":true}"#, Report::Focus(true)),
        (r#"{"kind":"commit"}"#, Report::Committed),
        (
            r#"{"kind":"source","url":"about:blank"}"#,
            Report::Source("about:blank".to_owned()),
        ),
        (r#"{"kind":"title","title":"T"}"#, Report::Title("T".to_owned())),
        (
            r#"{"kind":"history","back":true,"forward":false}"#,
            Report::History {
                back: true,
                forward: false,
            },
        ),
        (
            r#"{"kind":"complete","id":9,"success":true,"status":-999,"http":404}"#,
            Report::Completed {
                id: 9,
                success: true,
                status: -999,
                http: 404,
            },
        ),
        (r#"{"kind":"close"}"#, Report::CloseRequested),
        (r#"{"kind":"process"}"#, Report::ProcessFailed { browser: false }),
        (
            r#"{"kind":"new","url":"https://e.invalid/","user":true}"#,
            Report::NewWindow {
                uri: "https://e.invalid/".to_owned(),
                user: true,
            },
        ),
    ] {
        let (mut callbacks, log) = callbacks("");
        report((&raw mut *callbacks).cast(), string(json).as_ptr());
        assert_eq!(*log.borrow(), vec![expected], "{json}");
    }
}

#[test]
fn a_report_missing_its_fields_reads_as_their_defaults() {
    for (json, expected) in [
        (
            r#"{"kind":"failure"}"#,
            Report::Failed {
                id: 0,
                reason: String::new(),
            },
        ),
        (r#"{"kind":"focus"}"#, Report::Focus(false)),
        (
            r#"{"kind":"history"}"#,
            Report::History {
                back: false,
                forward: false,
            },
        ),
        (
            // a status beyond an i32 is no status
            r#"{"kind":"complete","status":4294967296,"http":-4294967296}"#,
            Report::Completed {
                id: 0,
                success: false,
                status: 0,
                http: 0,
            },
        ),
        (
            r#"{"kind":"new"}"#,
            Report::NewWindow {
                uri: String::new(),
                user: false,
            },
        ),
    ] {
        let (mut callbacks, log) = callbacks("");
        report((&raw mut *callbacks).cast(), string(json).as_ptr());
        assert_eq!(*log.borrow(), vec![expected], "{json}");
    }
}

#[test]
fn what_the_adapter_cannot_mean_is_dropped() {
    for json in ["not json", "[]", r#"{"kind":"unheard of"}"#, "{}"] {
        let (mut callbacks, log) = callbacks("");
        report((&raw mut *callbacks).cast(), string(json).as_ptr());
        assert_eq!(*log.borrow(), Vec::new(), "{json}");
    }
}

fn navigation_uri() -> CString {
    string("https://shown.invalid/next")
}

#[test]
fn an_allowed_navigation_is_answered_and_not_reported() {
    let (mut callbacks, log) = callbacks("");
    let allowed = decide(
        (&raw mut *callbacks).cast(),
        navigation_uri().as_ptr(),
        true,
        true,
        false,
        12,
    );
    assert!(allowed);
    assert_eq!(*log.borrow(), Vec::new());
}

#[test]
fn a_refused_main_frame_navigation_is_reported_with_what_the_web_view_said() {
    let (mut callbacks, log) = callbacks("shown.invalid");
    let allowed = decide(
        (&raw mut *callbacks).cast(),
        navigation_uri().as_ptr(),
        true,
        true,
        true,
        12,
    );
    assert!(!allowed);
    assert_eq!(
        *log.borrow(),
        vec![Report::Navigation {
            navigation: Navigation {
                uri: "https://shown.invalid/next".to_owned(),
                main_frame: true,
                user: true,
                redirect: true,
                id: 12,
            },
            allowed: false,
        }]
    );
}

#[test]
fn a_refused_frame_is_answered_without_touching_the_load() {
    let (mut callbacks, log) = callbacks("shown.invalid");
    let allowed = decide(
        (&raw mut *callbacks).cast(),
        navigation_uri().as_ptr(),
        false,
        false,
        false,
        1,
    );
    assert!(!allowed);
    assert_eq!(*log.borrow(), Vec::new());
}

#[test]
fn a_key_the_app_takes_is_kept_from_the_page_and_reported() {
    let (mut callbacks, log) = callbacks("");
    assert!(take_key((&raw mut *callbacks).cast(), 7));
    assert_eq!(*log.borrow(), vec![Report::Key(Keystroke::parse("ctrl-a").unwrap())]);
    log.borrow_mut().clear();
    assert!(!take_key((&raw mut *callbacks).cast(), 8));
    assert_eq!(*log.borrow(), Vec::new());
}

/// A renderer call's channel and the context the adapter would hand back.
fn pending_answer() -> (
    *mut c_void,
    async_channel::Receiver<super::super::super::text_page::Answer>,
) {
    let (sender, answer) = async_channel::bounded(1);
    (Box::into_raw(Box::new(sender)).cast(), answer)
}

#[test]
fn a_renderer_call_answers_its_value_nothing_or_its_error() {
    let (context, answer) = pending_answer();
    called(context, string("[1,2]").as_ptr(), std::ptr::null());
    assert_eq!(answer.try_recv().unwrap(), Ok(Some("[1,2]".to_owned())));

    let (context, answer) = pending_answer();
    called(context, std::ptr::null(), std::ptr::null());
    assert_eq!(answer.try_recv().unwrap(), Ok(None));

    let (context, answer) = pending_answer();
    called(context, string("ignored").as_ptr(), string("script failed").as_ptr());
    assert_eq!(answer.try_recv().unwrap(), Err("script failed".to_owned()));
}

/// The context the rule list's answer takes back.
fn pending_install(log: &Log) -> *mut c_void {
    let sink = Rc::clone(log);
    let post: Post = Rc::new(move |report| sink.borrow_mut().push(report));
    Box::into_raw(Box::new(post)).cast()
}

#[test]
fn the_network_filter_reports_in_place_or_why_it_is_not() {
    let log = Log::default();
    network_blocked(pending_install(&log), true, std::ptr::null());
    network_blocked(pending_install(&log), false, string("compile error").as_ptr());
    network_blocked(pending_install(&log), false, std::ptr::null());
    assert_eq!(
        *log.borrow(),
        vec![
            Report::NetworkFilterInstalled,
            Report::NetworkFilterFailed("compile error".to_owned()),
            Report::NetworkFilterFailed("the content rule list did not compile".to_owned()),
        ]
    );
}

// MARK: - the protocols

fn granted_dir() -> (TempDir, GrantRoot) {
    let directory = TempDir::new().unwrap();
    let root = GrantRoot::new(directory.path()).unwrap();
    (directory, root)
}

fn file_uri(path: &std::path::Path) -> String {
    url::Url::from_file_path(path).unwrap().to_string()
}

#[test]
fn a_granted_file_is_read_with_its_mime_type() {
    let (directory, root) = granted_dir();
    let file = directory.path().join("page.html");
    std::fs::write(&file, "<p>hello</p>").unwrap();
    assert_eq!(
        read_granted(Some(&root), &file_uri(&file)),
        Ok(("text/html".to_owned(), b"<p>hello</p>".to_vec()))
    );
}

#[test]
fn a_file_outside_the_grant_is_refused() {
    let (directory, root) = granted_dir();
    let outside = TempDir::new().unwrap();
    let file = outside.path().join("secret.html");
    std::fs::write(&file, "secret").unwrap();
    assert_eq!(
        read_granted(Some(&root), &file_uri(&file)),
        Err(OUTSIDE_GRANT.to_owned())
    );
    let inside = directory.path().join("page.html");
    std::fs::write(&inside, "inside").unwrap();
    assert_eq!(read_granted(None, &file_uri(&inside)), Err(OUTSIDE_GRANT.to_owned()));
    assert_eq!(
        read_granted(Some(&root), "not an address"),
        Err(OUTSIDE_GRANT.to_owned())
    );
}

#[test]
fn only_a_file_can_be_read() {
    let (directory, root) = granted_dir();
    let folder = directory.path().join("folder");
    std::fs::create_dir(&folder).unwrap();
    assert_eq!(
        read_granted(Some(&root), &file_uri(&folder)),
        Err(NOT_A_FILE.to_owned())
    );
}

#[test]
fn a_file_the_system_will_not_read_says_why() {
    let (directory, root) = granted_dir();
    let file = directory.path().join("locked.html");
    std::fs::write(&file, "locked").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
    let answer = read_granted(Some(&root), &file_uri(&file));
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(answer, Err("Permission denied (os error 13)".to_owned()));
}

fn text_of(body: Option<&str>) -> Text {
    Arc::new(Mutex::new(body.map(str::to_owned)))
}

#[test]
fn the_text_protocol_serves_the_document_uncached() {
    let text = text_of(Some("<p>body</p>"));
    let response = serve_text(&text, &scheme::text_address(uuid::Uuid::nil()));
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["Content-Type"], "text/html; charset=utf-8");
    assert_eq!(response.headers()["Cache-Control"], "no-store");
    assert_eq!(response.body().as_ref(), b"<p>body</p>");
}

#[test]
fn the_text_protocol_serves_nothing_but_a_loaded_document() {
    let address = scheme::text_address(uuid::Uuid::nil());
    for (text, uri) in [
        (text_of(Some("<p>body</p>")), format!("{address}logo.png")),
        (text_of(None), address.clone()),
    ] {
        let response = serve_text(&text, &uri);
        assert_eq!(response.status(), 404, "{uri}");
        assert_eq!(response.headers()["Content-Type"], "text/plain; charset=utf-8");
        assert!(response.body().is_empty());
    }
}

// MARK: - what is refused before a web view exists

#[test]
fn clearing_a_profile_that_is_not_a_store_is_refused() {
    let store = Store {
        environment: None,
        user_data: PathBuf::new(),
        profile: "page-1".to_owned(),
        private: false,
    };
    let answer = Arc::new(Mutex::new(None));
    let slot = Arc::clone(&answer);
    let environment = clear_profile(0, store, Box::new(move |result| *slot.lock().unwrap() = Some(result)));
    assert!(environment.is_none());
    assert_eq!(
        answer.lock().unwrap().take(),
        Some(Err("invalid browser profile".to_owned()))
    );
}

#[test]
fn a_saved_page_needs_a_store_named_by_a_uuid() {
    let log = Log::default();
    let mut spec = memory_spec(0, &log);
    spec.store.private = false;
    spec.store.profile = "ngterm-not-a-uuid".to_owned();
    let refused = Native::create(&spec).map(|_| ()).unwrap_err();
    assert_eq!(refused, "invalid browser profile");
}
