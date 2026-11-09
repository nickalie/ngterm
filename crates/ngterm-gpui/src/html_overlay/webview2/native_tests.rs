//! The browser adapter on a hidden `Win32` parent, with local documents and real `WebView2` reports.

use std::os::windows::fs::OpenOptionsExt as _;
use std::time::{Duration, Instant};

use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DestroyWindow, DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
};

use super::*;

#[test]
fn malformed_renderer_reports_and_exceptions_have_useful_answers() {
    for event in [
        "garbage",
        "null",
        r#"{"name":"ngtermTextPageBinding"}"#,
        r#"{"name":"ngtermTextPageBinding","executionContextId":3,"payload":false}"#,
    ] {
        assert_eq!(binding_called(event), None);
    }
    assert!(evaluation(Ok("not json".into())).is_err());
    assert_eq!(
        evaluation(Ok(r#"{"exceptionDetails":{"text":"bad expression"}}"#.into())),
        Err("bad expression".into())
    );
    assert_eq!(
        evaluation(Ok(r#"{"exceptionDetails":{}}"#.into())),
        Err("the script failed".into())
    );
    assert_eq!(
        evaluation(Ok(r#"{"result":{"type":"number","value":42}}"#.into())),
        Ok(Some("42".into()))
    );
    assert_eq!(
        evaluation(Ok(r#"{"result":{"type":"object","value":null}}"#.into())),
        Ok(Some("null".into()))
    );
    assert_eq!(evaluation(Ok(r#"{"result":{"type":"object"}}"#.into())), Ok(None));
    assert_eq!(without_fragment("https://example.test/a#b#c"), "https://example.test/a");
    assert_eq!(without_fragment("https://example.test/a"), "https://example.test/a");
    assert!(matches!(Parent(0).window_handle(), Err(HandleError::Unavailable)));
}

#[test]
fn local_protocols_return_only_the_document_or_granted_files() {
    let text = Text::default();
    let address = format!("ngterm-text://{}/", uuid::Uuid::new_v4());
    assert_eq!(serve_text(&text, &address).status(), 404);
    text.lock().unwrap().replace("<title>通知</title>".into());
    let response = serve_text(&text, &address);
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["Content-Type"], "text/html; charset=utf-8");
    assert_eq!(response.headers()["Cache-Control"], "no-store");
    assert_eq!(response.body().as_ref(), "<title>通知</title>".as_bytes());
    assert_eq!(serve_text(&text, &(address + "secret")).status(), 404);
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("page.html");
    std::fs::write(&file, "<p>granted</p>").unwrap();
    let uri = url::Url::from_file_path(&file).unwrap().to_string();
    let root = GrantRoot::new(dir.path()).unwrap();
    assert_eq!(
        read_granted(Some(&root), &uri).unwrap(),
        ("text/html".into(), b"<p>granted</p>".to_vec())
    );
    assert_eq!(
        read_granted(None, &uri),
        Err("outside the page's granted directory".into())
    );
    assert_eq!(
        read_granted(Some(&root), "not a file URL"),
        Err("outside the page's granted directory".into())
    );
    let outside = tempfile::NamedTempFile::new().unwrap();
    let outside = url::Url::from_file_path(outside.path()).unwrap().to_string();
    assert_eq!(
        read_granted(Some(&root), &outside),
        Err("outside the page's granted directory".into())
    );
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&file)
        .unwrap();
    assert_eq!(
        read_granted(Some(&root), &uri),
        Err(std::io::Error::from_raw_os_error(32).to_string())
    );
    drop(held);
}

#[test]
fn a_real_browser_reports_local_documents_in_an_isolated_process() {
    crate::system_window::tests::run_child("html_overlay::webview2::native::native_tests::browser_child");
}

#[test]
fn the_browser_outlives_the_last_page_in_an_isolated_process() {
    crate::system_window::tests::run_child("html_overlay::webview2::native::native_tests::last_page_child");
}

/// How long the browser is watched past its last page: `WebView2` ends an unheld browser within this.
const BROWSER_WATCH: Duration = Duration::from_secs(5);

/// A browser whose last page closed and ended took the next page through a fresh browser start, seconds that a page
/// open pays each time (CI's `test_reload_and_navigate_a_page`): the browser has to stay for the next page.
#[test]
#[ignore = "run by the_browser_outlives_the_last_page_in_an_isolated_process"]
fn last_page_child() {
    if !crate::system_window::tests::is_child("html_overlay::webview2::native::native_tests::last_page_child") {
        return;
    }
    // SAFETY: the browser owns this child test thread's STA and leaves it after every COM object is dropped.
    assert!(unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED.cast_unsigned()) } >= 0);
    let dir = tempfile::tempdir().unwrap();
    let parent = host_window().unwrap();
    let reports = Rc::new(RefCell::new(Vec::new()));
    let first = spec(parent, dir.path(), &reports);
    let native = Native::create(&first).unwrap();
    let environment = native.environment();
    let pid = browser_pid(&native);
    // SAFETY: opening only this test's browser, for the waits below.
    let browser = unsafe { windows_sys::Win32::System::Threading::OpenProcess(0x0010_0001, 0, pid) };
    assert!(!browser.is_null());
    drop(native);
    let exited = || {
        // SAFETY: the process handle opened above, polled without blocking the message loop.
        (unsafe { windows_sys::Win32::System::Threading::WaitForSingleObject(browser, 0) }) == 0
    };
    let watched = Instant::now();
    pump_until(|| {
        assert!(!exited(), "the browser ended with its last page");
        watched.elapsed() >= BROWSER_WATCH
    });
    let mut second = spec(parent, dir.path(), &reports);
    second.store.environment = Some(environment.clone());
    second.store.profile = "coverage-next".into();
    let next = Native::create(&second).unwrap();
    assert_eq!(browser_pid(&next), pid, "the next page runs in the same browser");
    drop(next);
    drop((first, second, environment));
    drop(ANCHOR.take());
    // SAFETY: destroy only the hidden parent made above, after all child web views were dropped.
    let destroyed = unsafe { DestroyWindow(std::ptr::with_exposed_provenance_mut(parent.cast_unsigned())) };
    assert_ne!(destroyed, 0);
    HOST.set(0);
    pump_until(exited);
    // SAFETY: the process handle opened above is closed exactly once.
    unsafe { windows_sys::Win32::Foundation::CloseHandle(browser) };
    // SAFETY: balances this thread's successful apartment initialization.
    unsafe { CoUninitialize() };
    profiles_released(dir);
}

fn browser_pid(native: &Native) -> u32 {
    let mut pid = 0;
    // SAFETY: reading the live web view's browser pid on its owning thread.
    unsafe { native.webview().BrowserProcessId(&raw mut pid) }.unwrap();
    pid
}

/// Pumps COM callbacks and hidden-window messages until an observable answer arrives, with a finite deadline.
#[track_caller]
fn pump_until(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !ready() {
        assert!(Instant::now() < deadline, "WebView2 did not report its result");
        let mut message = MSG::default();
        // SAFETY: a message buffer owned by this thread; dispatch only its own windows' messages.
        while unsafe { PeekMessageW(&raw mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
            // SAFETY: messages just removed from this thread's queue.
            unsafe { TranslateMessage(&raw const message) };
            // SAFETY: dispatching the message just removed from this thread's queue.
            unsafe { DispatchMessageW(&raw const message) };
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[track_caller]
fn take_report(reports: &Rc<RefCell<Vec<Report>>>, matches: impl Fn(&Report) -> bool) -> Report {
    let mut found = None;
    pump_until(|| {
        let mut reports = reports.borrow_mut();
        if let Some(index) = reports.iter().position(&matches) {
            found = Some(reports.remove(index));
        }
        found.is_some()
    });
    found.unwrap()
}

#[track_caller]
fn renderer_answer(native: &dyn NativeView, expression: &str) -> super::super::super::text_page::Answer {
    let answer = native.call_renderer(expression);
    let mut result = None;
    pump_until(|| {
        result = answer.try_recv().ok();
        result.is_some()
    });
    result.unwrap()
}

fn spec(parent: isize, dir: &Path, reports: &Rc<RefCell<Vec<Report>>>) -> Spec {
    let sent = Rc::clone(reports);
    Spec {
        parent,
        store: Store {
            environment: None,
            user_data: dir.to_path_buf(),
            profile: "coverage".into(),
            private: true,
        },
        javascript: true,
        background: Some((10, 20, 30)),
        zoom: 1.25,
        bounds: Some(Bounds {
            x: 0.,
            y: 0.,
            width: 320.,
            height: 200.,
        }),
        visible: false,
        theme: Some("window.coverageTheme = 'first'".into()),
        scripts: vec!["window.coverageScript = true".into()],
        trusted_document_script: Some(
            "globalThis.coverageValue = 41; ngtermTextPageBinding('{\"ready\":true}')".into(),
        ),
        text_channel: true,
        grant: Some(Grant::default()),
        replica: None,
        text: Some(Text::default()),
        block_network: true,
        decide: Rc::new(|navigation| !navigation.uri.contains("refused")),
        post: Rc::new(move |report| {
            eprintln!("WebView2 report: {report:?}");
            sent.borrow_mut().push(report);
        }),
        take_key: Rc::new(|_, _, _| None),
    }
}

#[test]
#[ignore = "run by a_real_browser_reports_local_documents_in_an_isolated_process"]
fn browser_child() {
    if !crate::system_window::tests::is_child("html_overlay::webview2::native::native_tests::browser_child") {
        return;
    }
    // SAFETY: the browser owns this child test thread's STA and leaves it after every COM object is dropped.
    assert!(unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED.cast_unsigned()) } >= 0);
    let dir = tempfile::tempdir().unwrap();
    let parent = host_window().unwrap();
    assert_eq!(host_window().unwrap(), parent, "one hidden parent per thread");
    let reports = Rc::new(RefCell::new(Vec::new()));
    let spec = spec(parent, dir.path(), &reports);
    let native = Native::create(&spec).unwrap();
    assert_eq!(
        renderer_answer(&native, "1"),
        Err("the renderer has not reported".into())
    );
    assert_eq!(
        take_report(&reports, |r| matches!(r, Report::NetworkFilterInstalled)),
        Report::NetworkFilterInstalled
    );
    load_and_report(&native, &spec, &reports);
    browser_properties(&native, &reports, parent);
    browser_messages(&native, &reports);
    assert_eq!(
        renderer_answer(&native, "confirm('coverage')"),
        Ok(Some("false".into()))
    );
    assert_eq!(renderer_answer(&native, "prompt('coverage')"), Ok(Some("null".into())));
    assert_eq!(
        renderer_answer(&native, "Notification.requestPermission()"),
        Ok(Some("\"denied\"".into()))
    );
    context_menu(&native);
    saved_profile_clear(parent, &native.environment(), dir.path());
    local_protocol_pages(parent, &native.environment(), dir.path());
    close_single_page(parent, &native.environment(), dir.path());
    network_pages(parent, &native, dir.path(), &reports);
    script_stops_a_live_sequences_keys(parent, &native.environment(), dir.path());
    consumed_keys_stay_off_a_real_page(parent, &native.environment(), dir.path());
    the_script_reads_what_it_took_before_the_page_ran(parent, &native.environment(), dir.path());
    invalid_specifications(parent, &native.environment(), dir.path());
    let mut browser_pid = 0;
    // SAFETY: reading the live web view's browser pid on its owning thread.
    unsafe { native.webview().BrowserProcessId(&raw mut browser_pid) }.unwrap();
    // SAFETY: opening only this test browser for the termination check and the subsequent wait.
    let browser = unsafe { windows_sys::Win32::System::Threading::OpenProcess(0x0010_0001, 0, browser_pid) };
    assert!(!browser.is_null());
    browser_failures(&native, &reports, browser);
    drop(native);
    drop(spec);
    // SAFETY: destroy only the hidden parent made above, after all child web views were dropped.
    let destroyed = unsafe { DestroyWindow(std::ptr::with_exposed_provenance_mut(parent.cast_unsigned())) };
    assert_ne!(destroyed, 0);
    HOST.set(0);
    pump_until(|| {
        // SAFETY: the process handle opened above, polled without blocking the message loop.
        (unsafe { windows_sys::Win32::System::Threading::WaitForSingleObject(browser, 0) }) == 0
    });
    // SAFETY: the process handle opened above is closed exactly once.
    unsafe { windows_sys::Win32::Foundation::CloseHandle(browser) };
    // SAFETY: balances this thread's successful apartment initialization.
    unsafe { CoUninitialize() };
    profiles_released(dir);
}

/// Removes the pages' user data folder, which fails while any process of the ended browser still holds a file there:
/// a helper that outlives the browser fails the test instead of leaving the folder in `%TEMP%`.
fn profiles_released(dir: tempfile::TempDir) {
    let folder = dir.keep();
    let deadline = Instant::now() + Duration::from_secs(15);
    while let Err(error) = std::fs::remove_dir_all(&folder) {
        assert!(
            Instant::now() < deadline,
            "the ended browser still holds {}: {error}",
            folder.display()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn load_and_report(native: &Native, spec: &Spec, reports: &Rc<RefCell<Vec<Report>>>) {
    spec.text
        .as_ref()
        .unwrap()
        .lock()
        .unwrap()
        .replace("<!doctype html><title>Coverage document</title><p>local</p>".into());
    let address = scheme::text_address(uuid::Uuid::new_v4());
    native.load_url(&address);
    take_report(reports, |r| matches!(r, Report::Completed { success: true, .. }));
    assert_eq!(
        take_report(reports, |r| matches!(r, Report::Title(s) if s == "Coverage document")),
        Report::Title("Coverage document".into())
    );
    take_report(reports, |r| matches!(r, Report::Committed));
    take_report(reports, |r| matches!(r, Report::Source(s) if s == &address));
    take_report(reports, |r| matches!(r, Report::TextPage(_)));
    assert_eq!(renderer_answer(native, "coverageValue + 1"), Ok(Some("42".into())));
    assert_eq!(renderer_answer(native, "void 0"), Ok(None));
    assert!(
        renderer_answer(native, "throw new Error('coverage failure')")
            .unwrap_err()
            .starts_with("Error: coverage failure")
    );
    let navigation = take_report(reports, |r| matches!(r, Report::Navigation { allowed: true, .. }));
    assert!(matches!(
        navigation,
        Report::Navigation {
            navigation: Navigation { main_frame: true, .. },
            allowed: true
        }
    ));
    native.set_theme("window.coverageTheme = 'second'");
    take_report(reports, |r| matches!(r, Report::ThemeInstalled));
    native.reload();
    take_report(reports, |r| matches!(r, Report::Completed { success: true, .. }));
    take_report(reports, |r| matches!(r, Report::TextPage(_)));
    native.load_url("https://refused.invalid/");
    take_report(reports, |r| matches!(r, Report::Navigation { allowed: false, .. }));
}

fn browser_properties(native: &Native, reports: &Rc<RefCell<Vec<Report>>>, parent: isize) {
    assert!(
        native
            .devtools_now("Coverage.invalidMethod", &serde_json::json!({}))
            .is_err()
    );
    assert!(
        native
            .add_script_now("")
            .unwrap_err()
            .starts_with("a page's script was not added: ")
    );
    native.set_zoom(1.5);
    let mut zoom = 0.;
    // SAFETY: the real controller is read on its owning STA.
    unsafe { native.view.controller().ZoomFactor(&raw mut zoom) }.unwrap();
    assert!((zoom - 1.5).abs() < f64::EPSILON);
    native.set_background((31, 32, 33));
    let mut original = windows::Win32::Foundation::RECT::default();
    // SAFETY: the live controller's bounds are read on its owning thread.
    unsafe { native.view.controller().Bounds(&raw mut original) }.unwrap();
    native.set_bounds(Bounds {
        x: 3.,
        y: 4.,
        width: 150.,
        height: 100.,
    });
    let mut bounds = windows::Win32::Foundation::RECT::default();
    // SAFETY: the controller and its initialized output rectangle belong to this thread.
    unsafe { native.view.controller().Bounds(&raw mut bounds) }.unwrap();
    let expected_width = f64::from(original.right - original.left) * 150.0 / 320.0;
    let expected_height = f64::from(original.bottom - original.top) * 100.0 / 200.0;
    assert!((f64::from(bounds.right - bounds.left) - expected_width).abs() <= 1.0);
    assert!((f64::from(bounds.bottom - bounds.top) - expected_height).abs() <= 1.0);
    native.set_visible(true);
    let mut visible = BOOL::default();
    // SAFETY: the controller's own visibility flag, on its thread.
    unsafe { native.view.controller().IsVisible(&raw mut visible) }.unwrap();
    assert!(visible.as_bool());
    native.focus();
    native.focus_parent();
    native.reparent(parent);
    native.set_visible(false);
    native.post_json(r#"{"coverage":true}"#);
    assert!(renderer_answer(native, "globalThis.coverageValue").is_ok());
    assert!(
        !reports
            .borrow()
            .iter()
            .any(|r| matches!(r, Report::NetworkFilterFailed(_)))
    );
}

fn main_script(native: &Native, expression: &str) {
    native
        .devtools_now(
            "Runtime.evaluate",
            &serde_json::json!({ "expression": expression, "awaitPromise": true }),
        )
        .unwrap();
}

fn browser_messages(native: &Native, reports: &Rc<RefCell<Vec<Report>>>) {
    main_script(native, "chrome.webview.postMessage('from-page')");
    assert_eq!(
        take_report(reports, |r| matches!(r, Report::Message(s) if s == "from-page")),
        Report::Message("from-page".into())
    );
    main_script(native, "history.pushState({}, '', '#second')");
    take_report(reports, |r| matches!(r, Report::History { back: true, .. }));
    native.go_back();
    take_report(reports, |r| matches!(r, Report::History { forward: true, .. }));
    native.go_forward();
    take_report(reports, |r| matches!(r, Report::History { back: true, .. }));
    main_script(native, "window.open('https://refused.invalid/new')");
    assert!(
        matches!(take_report(reports, |r| matches!(r, Report::NewWindow { .. })),
        Report::NewWindow { uri, .. } if uri == "https://refused.invalid/new")
    );
    main_script(
        native,
        "chrome.webview.addEventListener('message', event => chrome.webview.postMessage('echo:' + event.data.coverage))",
    );
    native.post_json(r#"{"coverage":"reply"}"#);
    take_report(reports, |r| matches!(r, Report::Message(s) if s == "echo:reply"));
}

/// What the script test's page signs with.
const KEYS_NONCE: &str = "post-nonce:reply-nonce";

/// A focused page with an input field and the real page script, as the script test types into it.
fn keys_page(
    parent: isize,
    environment: &ICoreWebView2Environment,
    dir: &Path,
    reports: &Rc<RefCell<Vec<Report>>>,
) -> Native {
    let mut spec = spec(parent, &dir.join("keys"), reports);
    spec.store.environment = Some(environment.clone());
    spec.store.profile = "keys".into();
    spec.theme = None;
    spec.scripts = vec![super::super::bridge::page_script(KEYS_NONCE, false, true)];
    spec.trusted_document_script = None;
    spec.text_channel = false;
    spec.block_network = false;
    spec.bounds = Some(Bounds {
        x: 0.,
        y: 0.,
        width: 320.,
        height: 200.,
    });
    spec.visible = true;
    let native = Native::create(&spec).unwrap();
    spec.text
        .as_ref()
        .unwrap()
        .lock()
        .unwrap()
        .replace("<!doctype html><input id=field autofocus>".into());
    native.load_url(&scheme::text_address(uuid::Uuid::new_v4()));
    take_report(reports, |r| matches!(r, Report::Completed { success: true, .. }));
    native
        .devtools_now(
            "Emulation.setFocusEmulationEnabled",
            &serde_json::json!({"enabled": true}),
        )
        .unwrap();
    main_script(
        &native,
        "chrome.webview.addEventListener('message', event => { if (event.data.ngterm === 'keys') \
         chrome.webview.postMessage('told'); }); field.focus()",
    );
    native
}

/// The page script on a real page, driven by the trusted input CDP gives: it stops a live sequence's key before the
/// page sees it and reports it, a key that matches nothing reaches the page, a key stopped by stale word goes back into
/// the page by `redelivery`, and with no sequence live nothing is stopped.
fn script_stops_a_live_sequences_keys(parent: isize, environment: &ICoreWebView2Environment, dir: &Path) {
    use super::super::bridge::{self, Message, StoppedKey};

    const NONCE: &str = KEYS_NONCE;
    let reports = Rc::new(RefCell::new(Vec::new()));
    let native = keys_page(parent, environment, dir, &reports);
    let told = |keys: &[u32]| {
        native.post_json(&bridge::live_keys(NONCE, keys, 60_000, None));
        take_report(&reports, |r| matches!(r, Report::Message(s) if s == "told"));
    };
    let field = || {
        main_script(&native, "chrome.webview.postMessage('field:' + field.value)");
        let Report::Message(value) =
            take_report(&reports, |r| matches!(r, Report::Message(s) if s.starts_with("field:")))
        else {
            unreachable!()
        };
        value
    };
    let key = |letter: &str, virtual_key: u32| StoppedKey {
        virtual_key,
        shift: false,
        repeat: false,
        key: letter.to_owned(),
        code: format!("Key{}", letter.to_uppercase()),
        at: 0,
    };
    let stopped_key_report = |repeat: bool| {
        let Report::Message(raw) = take_report(
            &reports,
            |r| matches!(r, Report::Message(raw) if matches!(bridge::parse(raw, NONCE), Some(Message::Key(_)))),
        ) else {
            unreachable!()
        };
        let Some(Message::Key(key)) = bridge::parse(&raw, NONCE) else {
            unreachable!()
        };
        assert_eq!((key.virtual_key, key.repeat), (0x47, repeat));
    };
    let typed_input = |letter: &str| {
        take_report(
            &reports,
            |r| matches!(r, Report::Message(raw) if raw.contains(&format!(r#""key":"{letter}""#)) && raw.contains(r#""k":"input""#)),
        );
    };

    told(&[0x47]);
    let [press, release] = bridge::redelivery(&key("g", 0x47));
    native.redeliver_key(std::slice::from_ref(&press));
    stopped_key_report(false);
    let mut autorepeat = press.clone();
    autorepeat["autoRepeat"] = serde_json::Value::Bool(true);
    native.redeliver_key(std::slice::from_ref(&autorepeat));
    stopped_key_report(true);
    native.redeliver_key(std::slice::from_ref(&release));
    assert_eq!(
        field(),
        "field:",
        "the page never had the stopped key or its autorepeat"
    );

    native.redeliver_key(&bridge::redelivery(&key("x", 0x58)));
    typed_input("x");
    assert_eq!(field(), "field:x", "a key the sequence does not take is the page's");

    told(&[]);
    native.redeliver_key(&bridge::redelivery(&key("g", 0x47)));
    typed_input("g");
    assert_eq!(field(), "field:xg", "with no live sequence nothing is stopped");
}

/// A press and release as trusted CDP input, each finished before it returns: the script's word changes between them.
fn cdp_key(native: &Native, event: &serde_json::Value) {
    native.devtools_now("Input.dispatchKeyEvent", event).unwrap();
}

/// What the page's own listeners have seen of the keyboard, as `keys:` report text.
fn page_keys_seen(native: &Native, reports: &Rc<RefCell<Vec<Report>>>) -> String {
    main_script(native, "chrome.webview.postMessage('keys:' + seen.join())");
    let Report::Message(seen) = take_report(reports, |r| matches!(r, Report::Message(s) if s.starts_with("keys:")))
    else {
        unreachable!()
    };
    seen
}

/// A stopped press, its autorepeat after the sequence ended, and its release stay off a real page's listeners and out
/// of its field, and so do Enter's and Backspace's; a key the sequence does not take reaches them whole, and so does
/// the same key once nothing is live.
fn consumed_keys_stay_off_a_real_page(parent: isize, environment: &ICoreWebView2Environment, dir: &Path) {
    use super::super::bridge::{self, StoppedKey};

    let reports = Rc::new(RefCell::new(Vec::new()));
    let native = keys_page(parent, environment, dir, &reports);
    main_script(
        &native,
        "window.seen = []; for (const kind of ['keydown', 'keyup']) \
         window.addEventListener(kind, event => seen.push(kind + ':' + event.key));",
    );
    let told = |keys: &[u32]| {
        native.post_json(&bridge::live_keys(KEYS_NONCE, keys, 60_000, None));
        take_report(&reports, |r| matches!(r, Report::Message(s) if s == "told"));
    };
    let key = |letter: &str, code: &str, virtual_key: u32| {
        bridge::redelivery(&StoppedKey {
            virtual_key,
            shift: false,
            repeat: false,
            key: letter.to_owned(),
            code: code.to_owned(),
            at: 0,
        })
    };
    let tap = |letter: &str, code: &str, virtual_key: u32| {
        for event in key(letter, code, virtual_key) {
            cdp_key(&native, &event);
        }
    };

    told(&[0x47]);
    let [press, release] = key("g", "KeyG", 0x47);
    cdp_key(&native, &press);
    told(&[]);
    let mut autorepeat = press;
    autorepeat["autoRepeat"] = serde_json::Value::Bool(true);
    cdp_key(&native, &autorepeat);
    cdp_key(&native, &release);
    told(&[0x0D, 0x08]);
    tap("Enter", "Enter", 0x0D);
    tap("Backspace", "Backspace", 0x08);
    assert_eq!(page_keys_seen(&native, &reports), "keys:");

    tap("x", "KeyX", 0x58);
    assert_eq!(page_keys_seen(&native, &reports), "keys:keydown:x,keyup:x");
    told(&[]);
    tap("g", "KeyG", 0x47);
    assert_eq!(
        page_keys_seen(&native, &reports),
        "keys:keydown:x,keyup:x,keydown:g,keyup:g"
    );
    main_script(&native, "chrome.webview.postMessage('field:' + field.value)");
    take_report(&reports, |r| matches!(r, Report::Message(s) if s == "field:xg"));
}

/// A page that replaces the fields of its events on the prototype changes neither report: what the script reads, it
/// took before the page ran.
fn the_script_reads_what_it_took_before_the_page_ran(
    parent: isize,
    environment: &ICoreWebView2Environment,
    dir: &Path,
) {
    use super::super::bridge::{self, Message, StoppedKey};

    let reports = Rc::new(RefCell::new(Vec::new()));
    let native = keys_page(parent, environment, dir, &reports);
    main_script(
        &native,
        "Object.defineProperty(Event.prototype, 'timeStamp', {get: () => undefined}); \
         for (const [name, value] of [['repeat', true], ['ctrlKey', true], ['altKey', true], ['key', 'Shift'], ['code', 'KeyZ']]) \
         Object.defineProperty(KeyboardEvent.prototype, name, {get: () => value});",
    );
    let pressed = u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap();
    let tap = |letter: &str, code: &str, virtual_key: u32| {
        for event in bridge::redelivery(&StoppedKey {
            virtual_key,
            shift: false,
            repeat: false,
            key: letter.to_owned(),
            code: code.to_owned(),
            at: pressed,
        }) {
            cdp_key(&native, &event);
        }
    };
    let told = |keys: &[u32]| {
        native.post_json(&bridge::live_keys(KEYS_NONCE, keys, 60_000, None));
        take_report(&reports, |r| matches!(r, Report::Message(s) if s == "told"));
    };

    told(&[0x47]);
    tap("g", "KeyG", 0x47);
    let Report::Message(raw) = take_report(
        &reports,
        |r| matches!(r, Report::Message(raw) if matches!(bridge::parse(raw, KEYS_NONCE), Some(Message::Key(_)))),
    ) else {
        unreachable!()
    };
    let Some(Message::Key(stopped)) = bridge::parse(&raw, KEYS_NONCE) else {
        unreachable!()
    };
    assert_eq!(stopped.key, "g");
    assert!(
        stopped.at.abs_diff(pressed) <= 2,
        "redelivery keeps its press time {pressed}: {raw}"
    );

    // Keep the press inside this document's lifetime, then let a newer leader overtake its redelivery.
    std::thread::sleep(Duration::from_millis(250));
    let (keymap, _) =
        ngterm_core::keymap::parse_keymap("command \"g\" ctrl+a>g true\n", &ngterm_core::keymap::linux_defaults());
    crate::keymap_dispatch::load_keymap(&keymap);
    assert_eq!(crate::keymap_dispatch::press_dispatched("ctrl-a"), Some(false));
    native.post_json(&bridge::live_keys(
        KEYS_NONCE,
        &[0x58],
        60_000,
        crate::keymap_dispatch::page_press_at(),
    ));
    take_report(&reports, |r| matches!(r, Report::Message(s) if s == "told"));
    tap("x", "KeyX", 0x58);
    let Report::Message(raw) = take_report(
        &reports,
        |r| matches!(r, Report::Message(raw) if matches!(bridge::parse(raw, KEYS_NONCE), Some(Message::Input { .. }))),
    ) else {
        unreachable!()
    };
    redelivered_input_keeps_the_newer_leader(&raw, pressed);
    a_newer_live_key_is_still_stopped(&native, &reports);
}

/// The cutoff exempts the old redelivery, while a fresh press of that same key is still the sequence's.
fn a_newer_live_key_is_still_stopped(native: &Native, reports: &Rc<RefCell<Vec<Report>>>) {
    use super::super::bridge::{self, Message, StoppedKey};

    let key = StoppedKey {
        virtual_key: 0x58,
        shift: false,
        repeat: false,
        key: "x".into(),
        code: "KeyX".into(),
        at: crate::keymap_dispatch::page_press_at().unwrap(),
    };
    for event in bridge::redelivery(&key) {
        cdp_key(native, &event);
    }
    let Report::Message(raw) = take_report(
        reports,
        |r| matches!(r, Report::Message(raw) if matches!(bridge::parse(raw, KEYS_NONCE), Some(Message::Key(_)))),
    ) else {
        unreachable!()
    };
    let Some(Message::Key(stopped)) = bridge::parse(&raw, KEYS_NONCE) else {
        unreachable!()
    };
    assert_eq!(stopped.virtual_key, key.virtual_key);
}

/// The real renderer's typed report keeps its original time and physical identity all the way to the monitor.
fn redelivered_input_keeps_the_newer_leader(raw: &str, pressed: u64) {
    use super::super::bridge::{self, Message};

    let Some(Message::Input {
        click: false,
        typed_at,
        typed_code,
    }) = bridge::parse(raw, KEYS_NONCE)
    else {
        unreachable!()
    };
    assert!(
        typed_at.is_some_and(|at| at.abs_diff(pressed) <= 2),
        "a typed redelivery keeps its old press time {pressed}: {raw}"
    );
    assert_eq!(
        typed_code.as_deref(),
        Some("KeyX"),
        "physical identity uses the captured getter"
    );
    crate::keymap_dispatch::page_typed(
        typed_at.unwrap(),
        typed_code.as_deref().and_then(crate::terminal::input::xkb_keycode),
    );
    assert!(
        crate::keymap_dispatch::sequence_is_live(),
        "redelivery leaves the newer leader waiting"
    );
    assert_eq!(crate::keymap_dispatch::press_dispatched("g"), Some(true));
}

fn saved_profile_clear(parent: isize, environment: &ICoreWebView2Environment, dir: &Path) {
    let reports = Rc::new(RefCell::new(Vec::new()));
    let mut saved = spec(parent, dir, &reports);
    saved.store.environment = Some(environment.clone());
    saved.store.private = false;
    saved.store.profile = "coverage-saved".into();
    saved.javascript = false;
    saved.theme = None;
    saved.scripts.clear();
    saved.block_network = false;
    let (native, made_environment) = super::super::native_view::create(&saved).unwrap();
    assert_eq!(made_environment.as_ref(), Some(environment));
    saved
        .text
        .as_ref()
        .unwrap()
        .lock()
        .unwrap()
        .replace("<title>Saved profile</title>".into());
    native.load_url(&scheme::text_address(uuid::Uuid::new_v4()));
    take_report(&reports, |r| matches!(r, Report::Completed { success: true, .. }));
    take_report(&reports, |r| matches!(r, Report::TextPage(_)));
    let cookie = renderer_answer(native.as_ref(), "document.cookie='coverage=1; path=/'; document.cookie")
        .unwrap()
        .unwrap();
    assert!(
        cookie.contains("coverage=1"),
        "the saved profile stores cookies: {cookie}"
    );
    let result = Arc::new(Mutex::new(None));
    let done = Arc::clone(&result);
    let returned = super::super::native_view::clear_profile(
        parent,
        saved.store.clone(),
        Box::new(move |answer| {
            done.lock().unwrap().replace(answer);
        }),
    );
    assert!(returned.is_some(), "the clear keeps the shared environment");
    pump_until(|| result.lock().unwrap().is_some());
    assert_eq!(result.lock().unwrap().take().unwrap(), Ok(()));
    assert_eq!(
        renderer_answer(native.as_ref(), "document.cookie"),
        Ok(Some("\"\"".into()))
    );
    drop(native);
    drop(ANCHOR.take());
}

fn custom_menu(native: &Native) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND_COMMAND, COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND_SUBMENU,
        ICoreWebView2Environment9,
    };
    let factory = native.environment().cast::<ICoreWebView2Environment9>().unwrap();
    // SAFETY: real menu items made by this browser's environment on its owning STA, with no icon stream.
    let root = unsafe {
        factory.CreateContextMenuItem(
            &HSTRING::from("test submenu"),
            None,
            COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND_SUBMENU,
        )
    }
    .unwrap();
    // SAFETY: the submenu's own collection belongs to this thread.
    let items = unsafe { root.Children() }.unwrap();
    trim_context_menu(&items).unwrap();
    for kind in [
        COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND_SEPARATOR,
        COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND_COMMAND,
    ] {
        // SAFETY: as above; custom commands have no built-in editing action and must be removed.
        let item = unsafe { factory.CreateContextMenuItem(&HSTRING::from("test item"), None, kind) }.unwrap();
        assert_eq!(
            menu_item(&item).unwrap().0,
            kind == COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND_SEPARATOR
        );
        // SAFETY: adding this environment's item to its own collection.
        unsafe { items.InsertValueAtIndex(0, &item) }.unwrap();
    }
    trim_context_menu(&items).unwrap();
    let mut count = 99;
    // SAFETY: reading the real collection's remaining item count.
    unsafe { items.Count(&raw mut count) }.unwrap();
    assert_eq!(count, 0, "no unrecognized custom action or separator remains");
}

fn context_menu(native: &Native) {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Environment9;
    custom_menu(native);
    let factory = native.environment().cast::<ICoreWebView2Environment9>().unwrap();
    // SAFETY: a real separator from this browser's environment, inserted before its editing actions below.
    let leading =
        unsafe { factory.CreateContextMenuItem(&HSTRING::new(), None, COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND_SEPARATOR) }
            .unwrap();
    let seen = Rc::new(RefCell::new(None));
    let capture = Rc::clone(&seen);
    let handler = ContextMenuRequestedEventHandler::create(Box::new(move |_, args| {
        let args = args.unwrap();
        // SAFETY: the real event and its menu collection, read only on this callback's STA.
        let items = unsafe { args.MenuItems() }?;
        // SAFETY: a valid separator from this environment, added to the real menu collection on its owning STA.
        unsafe { items.InsertValueAtIndex(0, &leading) }?;
        trim_context_menu(&items)?;
        let mut count = 0;
        // SAFETY: the event's own menu collection.
        unsafe { items.Count(&raw mut count) }?;
        let mut names = Vec::new();
        for index in 0..count {
            // SAFETY: an index strictly below this collection's count.
            names.push(menu_item(&unsafe { items.GetValueAtIndex(index) }?)?);
        }
        capture.replace(Some(names));
        // SAFETY: suppress the actual OS popup; the native adapter's earlier handler already trimmed the collection.
        unsafe { args.SetHandled(true) }
    }));
    let webview = native.webview().cast::<ICoreWebView2_11>().unwrap();
    let mut token = 0;
    // SAFETY: subscribe on the owning thread, before synthesizing browser input through CDP.
    unsafe { webview.add_ContextMenuRequested(&handler, &raw mut token) }.unwrap();
    native.set_visible(true);
    main_script(native, "getSelection().selectAllChildren(document.body)");
    native
        .devtools_now(
            "Input.dispatchMouseEvent",
            &serde_json::json!({"type":"mousePressed", "x":10, "y":20, "button":"right", "clickCount":1}),
        )
        .unwrap();
    native
        .devtools_now(
            "Input.dispatchMouseEvent",
            &serde_json::json!({"type":"mouseReleased", "x":10, "y":20, "button":"right", "clickCount":1}),
        )
        .unwrap();
    pump_until(|| seen.borrow().is_some());
    let items = seen.take().unwrap();
    assert!(!items.is_empty(), "selection offers editing actions");
    assert!(!items.first().unwrap().0 && !items.last().unwrap().0);
    assert!(
        items
            .iter()
            .all(|(separator, name)| *separator || keeps_menu_item(name))
    );
    assert!(items.windows(2).all(|pair| !pair[0].0 || !pair[1].0));
}

fn local_protocol_pages(parent: isize, environment: &ICoreWebView2Environment, dir: &Path) {
    let reports = Rc::new(RefCell::new(Vec::new()));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&requests);
    let mut local = spec(parent, dir, &reports);
    local.store.environment = Some(environment.clone());
    local.store.profile = "coverage-protocols".into();
    local.trusted_document_script = None;
    local.text_channel = false;
    local.replica = Some(Arc::new(move |uri, reply| {
        seen.lock().unwrap().push(uri);
        reply.respond(
            wry::http::Response::builder()
                .status(200)
                .header("Content-Type", "text/html")
                .body(b"<title>Replica document</title>".to_vec())
                .unwrap(),
        );
    }));
    let grant = local.grant.as_ref().unwrap();
    grant.root.lock().unwrap().replace(GrantRoot::new(dir).unwrap());
    let file = dir.join("granted.html");
    std::fs::write(&file, "<title>Granted document</title><p>read from disk</p>").unwrap();
    let native = Native::create(&local).unwrap();
    native.load_url(&scheme::granted_address(&file.to_string_lossy()).unwrap());
    take_report(&reports, |r| matches!(r, Report::Completed { http: 200, .. }));
    take_report(
        &reports,
        |r| matches!(r, Report::Title(title) if title == "Granted document"),
    );
    let outside = tempfile::NamedTempFile::new().unwrap();
    native.load_url(&scheme::granted_address(&outside.path().to_string_lossy()).unwrap());
    take_report(&reports, |r| matches!(r, Report::Completed { http: 403, .. }));
    assert_eq!(
        grant.refused.lock().unwrap().as_ref().unwrap().1,
        "outside the page's granted directory"
    );
    let page = uuid::Uuid::new_v4();
    native.load_url(&scheme::replica_address(page, "/document.html").unwrap());
    take_report(
        &reports,
        |r| matches!(r, Report::Title(title) if title == "Replica document"),
    );
    take_report(&reports, |r| matches!(r, Report::Completed { http: 200, .. }));
    assert!(
        requests
            .lock()
            .unwrap()
            .iter()
            .any(|uri| uri == &format!("ngterm-page://{page}/document.html"))
    );
    main_script(
        &native,
        "let a = document.createElement('a'); a.href = URL.createObjectURL(new Blob(['download'], {type:'application/octet-stream'})); a.download = 'never-saved.txt'; a.click()",
    );
    take_report(
        &reports,
        |r| matches!(r, Report::DownloadRefused(uri) if uri.starts_with("blob:")),
    );
}

fn close_single_page(parent: isize, environment: &ICoreWebView2Environment, dir: &Path) {
    let reports = Rc::new(RefCell::new(Vec::new()));
    let mut closing = spec(parent, dir, &reports);
    closing.store.environment = Some(environment.clone());
    closing.store.profile = "coverage-close".into();
    closing.trusted_document_script = None;
    closing.text_channel = false;
    closing
        .text
        .as_ref()
        .unwrap()
        .lock()
        .unwrap()
        .replace("<title>Closable</title>".into());
    let native = Native::create(&closing).unwrap();
    native.load_url(&scheme::text_address(uuid::Uuid::new_v4()));
    take_report(&reports, |r| matches!(r, Report::Completed { http: 200, .. }));
    let handler = webview2_com::ExecuteScriptCompletedHandler::create(Box::new(|_, _| Ok(())));
    // SAFETY: a page with just one history entry may close itself; no synchronous wait spans the window's destruction.
    unsafe {
        native
            .webview()
            .ExecuteScript(&HSTRING::from("window.close()"), &handler)
    }
    .unwrap();
    take_report(&reports, |r| matches!(r, Report::CloseRequested));
}

struct Fixture {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Fixture {
    fn start() -> Self {
        use std::io::Write as _;
        use std::sync::atomic::{AtomicBool, Ordering};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (seen, stopped) = (Arc::clone(&requests), Arc::clone(&stop));
        let thread = std::thread::spawn(move || {
            while !stopped.load(Ordering::Acquire) {
                let Ok((mut stream, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                };
                // Winsock hands the listener's non-blocking mode to the sockets it accepts
                stream.set_nonblocking(false).unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
                stream.set_write_timeout(Some(Duration::from_secs(1))).unwrap();
                let Some(path) = requested_path(&mut stream) else {
                    continue;
                };
                seen.lock().unwrap().push(path.clone());
                let body = if path == "/" {
                    "<title>Network fixture</title><script>addEventListener('load',()=>chrome.webview.postMessage('frames-complete'))</script><iframe src='/allowed'></iframe><iframe src='/refused'></iframe>"
                } else {
                    "<title>Frame</title>"
                };
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes());
            }
        });
        Self {
            url,
            requests,
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        self.thread.take().unwrap().join().unwrap();
    }
}

fn network_pages(parent: isize, blocked: &Native, dir: &Path, blocked_reports: &Rc<RefCell<Vec<Report>>>) {
    let fixture = Fixture::start();
    let reports = Rc::new(RefCell::new(Vec::new()));
    let mut network = spec(parent, dir, &reports);
    network.store.environment = Some(blocked.environment());
    network.store.profile = "coverage-network".into();
    network.block_network = false;
    network.grant = None;
    network.text = None;
    network.background = None;
    network.zoom = 1.0;
    network.trusted_document_script = None;
    network.text_channel = false;
    let native = Native::create(&network).unwrap();
    native.load_url(&fixture.url);
    take_report(&reports, |r| matches!(r, Report::Completed { http: 200, .. }));
    take_report(
        &reports,
        |r| matches!(r, Report::Message(message) if message == "frames-complete"),
    );
    let requested = fixture.requests.lock().unwrap().clone();
    assert!(requested.iter().any(|path| path == "/allowed"), "{requested:?}");
    assert!(
        !requested.iter().any(|path| path == "/refused"),
        "a refused frame cannot reach the server"
    );
    blocked.load_url(&format!("{}blocked", fixture.url));
    take_report(blocked_reports, |r| matches!(r, Report::Completed { http: 403, .. }));
    assert!(!fixture.requests.lock().unwrap().iter().any(|path| path == "/blocked"));
}

fn browser_failures(
    native: &Native,
    reports: &Rc<RefCell<Vec<Report>>>,
    browser: windows_sys::Win32::Foundation::HANDLE,
) {
    let webview = native.webview().cast::<ICoreWebView2_13>().unwrap();
    // SAFETY: obtaining this live browser profile on its owning STA, before the browser exits.
    let profile = unsafe { webview.Profile() }
        .unwrap()
        .cast::<ICoreWebView2Profile2>()
        .unwrap();
    let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(|_, _| Ok(())));
    // SAFETY: crash only this test web view's renderer, exercising the engine's actual process-failure event.
    unsafe {
        native
            .webview()
            .CallDevToolsProtocolMethod(&HSTRING::from("Page.crash"), &HSTRING::from("{}"), &handler)
    }
    .unwrap();
    take_report(reports, |r| matches!(r, Report::ProcessFailed { browser: false }));
    // SAFETY: this handle names only the browser started under this test's temporary profile, opened with terminate access.
    let terminated = unsafe { windows_sys::Win32::System::Threading::TerminateProcess(browser, 0) };
    assert_ne!(terminated, 0);
    take_report(reports, |r| matches!(r, Report::ProcessFailed { browser: true }));
    assert!(
        native
            .add_isolated_document_script("1")
            .unwrap_err()
            .starts_with("the isolated document renderer was not added: ")
    );
    assert!(
        native
            .wire_renderer_channel(&native.post)
            .unwrap_err()
            .starts_with("the renderer's channel was not added: ")
    );
    native.load_url("about:blank");
    native.set_theme("window.theme = true");
    native.post_json("{}");
    let answer = native.call_renderer("1");
    pump_until(|| match answer.try_recv() {
        Ok(answer) => {
            assert!(answer.is_err());
            true
        }
        Err(async_channel::TryRecvError::Closed) => true,
        Err(async_channel::TryRecvError::Empty) => false,
    });
    let result = Rc::new(RefCell::new(None));
    let done = Rc::clone(&result);
    start_clear(
        &profile,
        Box::new(move |answer| {
            done.replace(Some(answer));
        }),
    );
    pump_until(|| result.borrow().is_some());
    assert!(
        result.take().unwrap().is_err(),
        "a clear of an ended browser reports failure"
    );
}

fn requested_path(stream: &mut std::net::TcpStream) -> Option<String> {
    use std::io::{BufRead as _, Read as _};
    let mut reader = std::io::BufReader::new(stream.take(16 * 1024));
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let path = line.split_whitespace().nth(1)?.to_owned();
    loop {
        line.clear();
        if reader.read_line(&mut line).ok()? == 0 || line == "\r\n" {
            return Some(path);
        }
    }
}

fn invalid_specifications(parent: isize, environment: &ICoreWebView2Environment, dir: &Path) {
    let reports = Rc::new(RefCell::new(Vec::new()));
    let mut invalid = spec(0, dir, &reports);
    invalid.store.environment = Some(environment.clone());
    assert!(
        Native::create(&invalid).is_err(),
        "a missing parent handle cannot create a page"
    );
    invalid.store.profile = "invalid/profile".into();
    let result = Arc::new(Mutex::new(None));
    let done = Arc::clone(&result);
    let environment = super::super::native_view::clear_profile(
        parent,
        invalid.store,
        Box::new(move |answer| {
            done.lock().unwrap().replace(answer);
        }),
    );
    assert!(environment.is_none(), "a refused profile creates no anchor");
    assert!(
        result.lock().unwrap().take().unwrap().is_err(),
        "the clear reports why it could not start"
    );
}
