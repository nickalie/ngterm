//! `session.overlay.open --text` (upstream's `ControlDispatcherOverlayTests` text cases, `AppStoreTreeProjectionTests.
//! aTextPageProjectsItsFileMappedLanguageAndRequestedLine`, `AppStoreRemotePageTests.aPresentedSessionRefusesATextPage`
//! and `ControlServer+TextPage`'s probe refusal).

use std::cell::RefCell;
use std::rc::Rc;

use ngterm_proto::command::{Command, SessionCmd};
use ngterm_proto::{Args, Request};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{Fixture, args, failure, wire};
use crate::html::{HtmlHost, HtmlNavigation, HtmlSource, errors};

#[derive(Default)]
struct Log {
    checked: Vec<String>,
    failure: Option<String>,
}

struct Host(Rc<RefCell<Log>>);

impl HtmlHost for Host {
    fn persistent_store_failure(&self) -> Option<String> {
        None
    }
    fn navigate(&self, _: Uuid, _: HtmlNavigation) -> Result<(), String> {
        Ok(())
    }
    fn check_text(&self, path: &str) -> Result<(), String> {
        let mut log = self.0.borrow_mut();
        log.checked.push(path.to_owned());
        log.failure.clone().map_or(Ok(()), Err)
    }
}

fn fixture() -> (Fixture, Rc<RefCell<Log>>) {
    let mut f = Fixture::new(1);
    let log = Rc::new(RefCell::new(Log::default()));
    f.core.set_html_host(Box::new(Host(Rc::clone(&log))));
    (f, log)
}

fn path(name: &str) -> String {
    crate::html::os_path(&format!("/tmp/{name}"))
}

fn page(f: &mut Fixture, id: Uuid) -> Option<Value> {
    let tree = serde_json::to_value(f.core.tree(None).unwrap()).unwrap();
    tree["workspaces"][0]["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["id"] == wire(id))
        .and_then(|node| node.get("htmlOverlays"))
        .map(|pages| pages[0].clone())
}

/// Upstream's `textOpenRejectsInvalidInputsBeforeCallingActions`: every refusal comes before the target resolves.
#[test]
fn text_open_rejects_invalid_inputs_before_resolving_a_target() {
    const ONE_SOURCE: &str =
        "session.overlay.open takes one of a command, --html, --url, --image, --markdown or --text";
    const OPTIONS: &str = "session.overlay.open: --text takes no --wait, --cwd, --navigation, --js, --chromeless, --persistent or --browse";
    const TYPE: &str = "session.overlay.open: --text must be a source, config, text or log file";
    const LINE: &str = "session.overlay.open: --line must be 1 or more";
    const NO_TEXT: &str = "session.overlay.open: --line requires --text";
    let text = path("a.go");
    let cases = [
        (json!({"command": "cat", "textFile": text}), ONE_SOURCE),
        (json!({"html": path("r.html"), "textFile": text}), ONE_SOURCE),
        (json!({"url": "http://localhost:5173/", "textFile": text}), ONE_SOURCE),
        (json!({"image": path("a.png"), "textFile": text}), ONE_SOURCE),
        (json!({"markdownFile": path("r.md"), "textFile": text}), ONE_SOURCE),
        (json!({"wait": true, "textFile": text}), OPTIONS),
        (json!({"cwd": path(""), "textFile": text}), OPTIONS),
        (json!({"navigation": true, "textFile": text}), OPTIONS),
        (json!({"javascript": true, "textFile": text}), OPTIONS),
        (json!({"chromeless": true, "textFile": text}), OPTIONS),
        (json!({"persistent": true, "textFile": text}), OPTIONS),
        (json!({"browse": true, "textFile": text}), OPTIONS),
        (
            json!({"textFile": "a.go"}),
            "session.overlay.open: --text must be an absolute path",
        ),
        (json!({"textFile": path("a.html")}), TYPE),
        (json!({"textFile": path("a.svg")}), TYPE),
        (json!({"textFile": path("r.md")}), TYPE),
        (json!({"textFile": path("a.go:12")}), TYPE),
        (json!({"textFile": text, "line": 0}), LINE),
        (json!({"textFile": text, "line": -3}), LINE),
        (json!({"line": 4}), NO_TEXT),
        (json!({"command": "cat", "line": 4}), NO_TEXT),
        (json!({"markdownFile": path("r.md"), "line": 4}), NO_TEXT),
        (json!({"image": "relative", "wait": true, "line": 4}), NO_TEXT),
        (json!({"textFile": path("x.svg"), "line": 0}), LINE),
        (json!({"textFile": "x.go", "line": 0, "wait": true}), OPTIONS),
    ];
    let (mut f, log) = fixture();
    for (value, error) in cases {
        let input: Args = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(
            f.session(SessionCmd::OverlayOpen, Some("missing"), input.clone()),
            failure(error),
            "{value}"
        );
        assert_eq!(
            f.session(SessionCmd::OverlayOpen, Some(&wire(f.sessions[0])), input),
            failure(error),
            "{value}"
        );
    }
    assert!(log.borrow().checked.is_empty(), "nothing was read");
}

/// Upstream's `textOpenRoutesTheStandardizedPathAndItsLine`: the path is dot-normalized, taken literally otherwise,
/// and read before the page takes its slot; the tree names the file, its mapped language and the requested line.
#[test]
fn text_open_checks_the_standardized_path_and_projects_its_source() {
    for (raw, standardized, line, language) in [
        ("/tmp/../tmp/Main.GO", "/tmp/Main.GO", Some(12), Some("go")),
        ("/tmp/notes.txt", "/tmp/notes.txt", None, None),
        ("/tmp/src/Makefile", "/tmp/src/Makefile", Some(1), Some("makefile")),
        ("/tmp/a:12.go", "/tmp/a:12.go", Some(7), Some("go")),
    ] {
        if cfg!(windows) && raw.contains(':') {
            // a Windows name holds no colon: `a:` reads as a drive there
            continue;
        }
        let (mut f, log) = fixture();
        let id = f.sessions[0];
        let (raw, standardized) = (crate::html::os_path(raw), crate::html::os_path(standardized));
        let response = f.session(
            SessionCmd::OverlayOpen,
            Some(&wire(id)),
            args(|a| {
                a.text_file = Some(raw.clone());
                a.line = line;
                a.size_percent = Some(95);
            }),
        );
        assert!(response.ok, "{raw}: {response:?}");
        assert_eq!(log.borrow().checked, std::slice::from_ref(&standardized));
        let page = page(&mut f, id).unwrap();
        let mut text = json!({"file": standardized});
        if let Some(language) = language {
            text["language"] = json!(language);
        }
        if let Some(line) = line {
            text["line"] = json!(line);
        }
        assert_eq!(page["text"], text, "{raw}");
        for key in ["file", "url", "image", "markdown", "cwd"] {
            assert!(page.get(key).is_none(), "{key}: {page}");
        }
        assert_eq!(page["javascript"], false);
    }
}

/// Upstream's `aTextOpenRequestRoundTripsItsFileAndLine`.
#[test]
fn a_text_open_request_round_trips_its_file_and_line() {
    let request = Request {
        cmd: Command::Session(SessionCmd::OverlayOpen),
        target: Some("s".to_owned()),
        args: Some(args(|a| {
            a.text_file = Some("/tmp/a:12.go".to_owned());
            a.line = Some(7);
        })),
    };
    let wire_text = serde_json::to_string(&request).unwrap();
    assert!(wire_text.contains(r#""textFile":"/tmp/a:12.go""#), "{wire_text}");
    assert!(wire_text.contains(r#""line":7"#), "{wire_text}");
    assert_eq!(serde_json::from_str::<Request>(&wire_text).unwrap(), request);
    let plain = serde_json::to_string(&args(|a| a.text_file = Some("/tmp/a.go".to_owned()))).unwrap();
    assert!(!plain.contains("line"), "{plain}");
}

/// Upstream's `pageCompanionRefusesATextFile`.
#[test]
fn a_page_companion_refuses_a_text_file() {
    let (mut f, log) = fixture();
    let input = args(|a| {
        a.text_file = Some(path("a.go"));
        a.line = Some(3);
    });
    assert_eq!(
        f.session(SessionCmd::CompanionPage, Some("missing"), input),
        failure("a page companion needs --html or --url")
    );
    assert!(log.borrow().checked.is_empty());
}

/// Upstream's `textFileRefusal`: the file is read before the slot is taken, so an unreadable, oversized or binary file
/// opens nothing.
#[test]
fn a_failed_text_probe_opens_nothing() {
    let (mut f, log) = fixture();
    let id = f.sessions[0];
    let file = path("blob.log");
    log.borrow_mut().failure = Some(format!("not a UTF-8 text file: {file}"));
    let response = f.session(
        SessionCmd::OverlayOpen,
        Some(&wire(id)),
        args(|a| a.text_file = Some(file.clone())),
    );
    assert_eq!(
        response,
        failure(&format!("session.overlay.open: not a UTF-8 text file: {file}"))
    );
    assert!(page(&mut f, id).is_none());
}

/// A text page is built by the app: the browser would get the raw file, and the file manager reveals file pages only.
#[test]
fn a_text_page_refuses_the_browser_and_the_file_manager() {
    let (mut f, _) = fixture();
    let id = f.sessions[0];
    let response = f.session(
        SessionCmd::OverlayOpen,
        Some(&wire(id)),
        args(|a| a.text_file = Some(path("main.go"))),
    );
    assert!(response.ok, "{response:?}");
    let browser = f.session(
        SessionCmd::OverlayNavigate,
        Some(&wire(id)),
        args(|a| a.to = Some("browser".to_owned())),
    );
    assert_eq!(browser, failure(errors::TEXT_IN_OVERLAY));
}

/// A clicked text link opens without a second probe (the frontend read it off its main thread) and never selects its
/// session.
#[test]
fn a_text_click_opens_without_selecting_its_session() {
    let (mut f, log) = fixture();
    let id = f.sessions[0];
    let selected = f.store().selected_session_id();
    let source = HtmlSource::Text {
        path: path("main.go"),
        line: Some(4),
    };
    assert!(f.core.open_document_link(source.clone(), id, Some(95), None));
    assert!(log.borrow().checked.is_empty());
    assert!(!f.core.open_document_link(source, id, Some(95), None), "a busy slot");
    assert_eq!(f.store().selected_session_id(), selected);
    assert_eq!(page(&mut f, id).unwrap()["text"]["line"], 4);
}

/// Upstream's `presentationSource(.text) == nil`: no presenter shows a text page.
#[test]
fn no_presenter_shows_a_text_page() {
    let source = HtmlSource::Text {
        path: path("a.go"),
        line: Some(3),
    };
    assert!(crate::remote::presentation_source(&source).is_none());
}
