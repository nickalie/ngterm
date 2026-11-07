//! Log pages over control (upstream's `ControlDispatcherOverlayTests` log and follow cases, `HtmlOverlayTests`
//! `followMode*`, `aScrollReportedFollowChange*`, `aReport*`, `aLogPageNodeReportsItsLimitAndLiveState`,
//! `ControlProtocolTests.logPageArgumentsRoundTripAndAreOmittedWhenUnset` and `ControlServer+TextPage`'s log refusals):
//! `--tail` and `--follow-file` make a text page a log page, read only for its tail before it opens, and
//! `session.overlay.follow` follows it or pauses it.

use std::cell::RefCell;
use std::rc::Rc;

use ngterm_proto::command::{Command, SessionCmd};
use ngterm_proto::{Args, Request};
use serde_json::{Value, json};
use uuid::Uuid;

use super::remote_page::Origin;
use super::{Fixture, args, failure, wire};
use crate::html::text_page::{TextLog, TextLogReport, TextLogState};
use crate::html::{HtmlHost, HtmlNavigation, HtmlOverlay, HtmlPageFlags, HtmlSource, errors};
use crate::remote::hub::SUPPORTED_KINDS;

const TAIL_WITHOUT_TEXT: &str = "session.overlay.open: --tail and --follow-file require --text";
const TAIL_WITH_LINE: &str = "session.overlay.open: --tail and --follow-file cannot be combined with --line";
const TAIL_RANGE: &str = "session.overlay.open: --tail must be 1...100000";
const LOG_REMOTE: &str = "session.overlay.open: a log page is not shown on another Mac";

/// What the host was asked to read: a whole text file or a log's tail.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Check {
    Text(String),
    Log(String, usize),
}

#[derive(Default)]
struct Log {
    checked: Vec<Check>,
    failure: Option<String>,
}

struct Host(Rc<RefCell<Log>>);

impl Host {
    fn answer(&self, check: Check) -> Result<(), String> {
        let mut log = self.0.borrow_mut();
        log.checked.push(check);
        log.failure.clone().map_or(Ok(()), Err)
    }
}

impl HtmlHost for Host {
    fn persistent_store_failure(&self) -> Option<String> {
        None
    }
    fn navigate(&self, _: Uuid, _: HtmlNavigation) -> Result<(), String> {
        Ok(())
    }
    fn check_text(&self, path: &str) -> Result<(), String> {
        self.answer(Check::Text(path.to_owned()))
    }
    fn check_log(&self, path: &str, tail: usize) -> Result<(), String> {
        self.answer(Check::Log(path.to_owned(), tail))
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

/// The `text` node of the session-wide page of `id`.
fn text(f: &mut Fixture, id: Uuid) -> Value {
    let tree = serde_json::to_value(f.core.tree(None).unwrap()).unwrap();
    let session = tree["workspaces"][0]["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["id"] == wire(id))
        .unwrap()
        .clone();
    session["htmlOverlays"][0]["text"].clone()
}

/// Opens `app.log` in session `id` with `edit` applied.
fn open_log(f: &mut Fixture, id: Uuid, edit: impl FnOnce(&mut Args)) -> ngterm_proto::Response {
    f.session(
        SessionCmd::OverlayOpen,
        Some(&wire(id)),
        args(|a| {
            a.text_file = Some(path("app.log"));
            edit(a);
        }),
    )
}

fn follow(f: &mut Fixture, target: &str, mode: Option<&str>) -> ngterm_proto::Response {
    f.session(
        SessionCmd::OverlayFollow,
        Some(target),
        args(|a| a.mode = mode.map(str::to_owned)),
    )
}

/// Upstream's `textOpenRejectsInvalidInputsBeforeCallingActions` log rows: a log option with no text file is refused
/// ahead of any source, then beside a line, then out of range, all before the target resolves.
#[test]
fn log_open_rejects_invalid_inputs_before_resolving_a_target() {
    let log = path("a.log");
    let cases = [
        (json!({"tail": 10}), TAIL_WITHOUT_TEXT),
        (json!({"followFile": true}), TAIL_WITHOUT_TEXT),
        (json!({"command": "cat", "tail": 10}), TAIL_WITHOUT_TEXT),
        (json!({"image": path("a.png"), "followFile": true}), TAIL_WITHOUT_TEXT),
        (json!({"markdownFile": path("r.md"), "tail": 10}), TAIL_WITHOUT_TEXT),
        (json!({"html": path("r.html"), "followFile": true}), TAIL_WITHOUT_TEXT),
        // the line's own refusal comes first
        (
            json!({"line": 3, "tail": 10}),
            "session.overlay.open: --line requires --text",
        ),
        (json!({"textFile": log, "line": 3, "tail": 10}), TAIL_WITH_LINE),
        (json!({"textFile": log, "line": 3, "followFile": true}), TAIL_WITH_LINE),
        (json!({"textFile": log, "tail": 0}), TAIL_RANGE),
        (json!({"textFile": log, "tail": -1}), TAIL_RANGE),
        (json!({"textFile": log, "tail": 100_001}), TAIL_RANGE),
        // the text page's own checks still come first
        (
            json!({"textFile": log, "tail": 0, "wait": true}),
            "session.overlay.open: --text takes no --wait, --cwd, --navigation, --js, --chromeless, --persistent or --browse",
        ),
        (
            json!({"textFile": log, "line": 0, "tail": 10}),
            "session.overlay.open: --line must be 1 or more",
        ),
        (
            json!({"textFile": path("a.md"), "tail": 10}),
            "session.overlay.open: --text must be a source, config, text or log file",
        ),
    ];
    let (mut f, host) = fixture();
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
    // a page companion takes no text file, and so no log options either
    assert_eq!(
        f.session(SessionCmd::CompanionPage, Some("missing"), args(|a| a.tail = Some(10))),
        failure(TAIL_WITHOUT_TEXT)
    );
    assert!(host.borrow().checked.is_empty(), "nothing was read");
}

/// Upstream's `textOpenCarriesTheLogRequest`: a tail alone shows it once, `--follow-file` follows with the default tail
/// unless one is given, and a false `followFile` is a plain page. A log is read only for its tail.
#[test]
fn text_open_carries_the_log_request() {
    for (tail, follow_file, log) in [
        (Some(50), None, Some((50, false))),
        (None, Some(true), Some((5_000, true))),
        (Some(100_000), Some(true), Some((100_000, true))),
        (None, Some(false), None),
    ] {
        let (mut f, host) = fixture();
        let id = f.sessions[0];
        let response = open_log(&mut f, id, |a| {
            a.tail = tail;
            a.follow_file = follow_file;
        });
        assert!(response.ok, "{response:?}");
        let page = f.store().session(id).unwrap().overlay.page().unwrap().clone();
        let expected = log.map(|(tail, follow)| TextLog { tail, follow });
        assert_eq!(page.text_log, expected);
        assert_eq!(page.log_state, expected.map(TextLogState::new));
        let check = match expected {
            Some(log) => Check::Log(path("app.log"), log.tail),
            None => Check::Text(path("app.log")),
        };
        assert_eq!(host.borrow().checked, [check]);
    }
}

/// Upstream's `textFileRefusal` for a log: its tail is read before the slot is taken, so a binary end opens nothing.
#[test]
fn a_failed_log_probe_opens_nothing() {
    let (mut f, host) = fixture();
    let id = f.sessions[0];
    let file = path("app.log");
    host.borrow_mut().failure = Some(format!("not a UTF-8 text file: {file}"));
    let response = open_log(&mut f, id, |a| a.follow_file = Some(true));
    assert_eq!(
        response,
        failure(&format!("session.overlay.open: not a UTF-8 text file: {file}"))
    );
    assert!(f.store().session(id).unwrap().overlay.page().is_none());
}

/// Upstream's `aLogPageNodeReportsItsLimitAndLiveState` and `aReportIsWrittenWithNoEventAndKeepsFollowing`: the node
/// carries the limit and the live state, and a report changes it with no `tree.changed`.
#[test]
fn a_log_page_node_reports_its_limit_and_live_state() {
    let (mut f, _) = fixture();
    let id = f.sessions[0];
    assert!(open_log(&mut f, id, |a| a.tail = Some(50)).ok);
    let file = path("app.log");
    assert_eq!(
        text(&mut f, id),
        json!({"file": file, "tail": 50, "following": false, "lines": 0, "clipped": false, "waiting": false,
            "unreadable": false})
    );
    f.core.commit();
    let page = f.store().session(id).unwrap().overlay.page().unwrap().id;
    let report = TextLogReport {
        lines: 7,
        clipped: true,
        waiting: false,
        unreadable: true,
    };
    f.core.library_mut().set_text_log_report(page, report);
    assert!(f.core.commit().tree_changed.is_empty(), "a report wakes no hook");
    assert_eq!(
        text(&mut f, id),
        json!({"file": file, "tail": 50, "following": false, "lines": 7, "clipped": true, "waiting": false,
            "unreadable": true})
    );
}

/// Upstream's `followModeSetsTheLogPage` and `followModeThatChangesNothingEmitsNothing`: an absent mode toggles, on and
/// off are idempotent, the answer names the state left, and only a change emits `tree.changed`.
#[test]
fn follow_answers_the_state_it_left_and_emits_only_on_a_change() {
    let (mut f, _) = fixture();
    let id = f.sessions[0];
    assert!(open_log(&mut f, id, |a| a.follow_file = Some(true)).ok);
    f.core.commit();
    for (mode, state, changed) in [
        (Some("on"), "following", false),
        (Some("off"), "paused", true),
        (Some("off"), "paused", false),
        (Some("toggle"), "following", true),
        (None, "paused", true),
        (Some("on"), "following", true),
    ] {
        let response = follow(&mut f, &wire(id), mode);
        assert_eq!(
            response.result,
            Some(json!({"id": wire(id), "text": state})),
            "{mode:?}"
        );
        assert_eq!(f.core.commit().tree_changed.is_empty(), !changed, "{mode:?}");
        assert_eq!(text(&mut f, id)["following"], state == "following", "{mode:?}");
    }
}

/// Upstream's `aScrollReportedFollowChangeEmitsOnlyWhenItChanges` and `aReportForAPageThatIsNotALogPageIsIgnored`: the
/// page's own report follows the page wherever it sits.
#[test]
fn a_page_reported_follow_change_emits_only_when_it_changes() {
    let (mut f, _) = fixture();
    let id = f.sessions[0];
    assert!(open_log(&mut f, id, |a| a.follow_file = Some(true)).ok);
    f.core.commit();
    let page = f.store().session(id).unwrap().overlay.page().unwrap().id;
    assert!(f.core.library_mut().set_text_log_following(page, false));
    assert!(!f.core.library_mut().set_text_log_following(page, false));
    assert_eq!(f.core.commit().tree_changed.len(), 1);
    assert_eq!(text(&mut f, id)["following"], false);

    assert!(f.session(SessionCmd::OverlayClose, Some(&wire(id)), Args::default()).ok);
    assert!(
        f.session(
            SessionCmd::OverlayOpen,
            Some(&wire(id)),
            args(|a| a.text_file = Some(path("a.txt")))
        )
        .ok
    );
    let plain = f.store().session(id).unwrap().overlay.page().unwrap().id;
    assert!(!f.core.library_mut().set_text_log_following(plain, true));
    f.core.library_mut().set_text_log_report(
        plain,
        TextLogReport {
            lines: 3,
            ..TextLogReport::default()
        },
    );
    assert_eq!(f.store().session(id).unwrap().overlay.page().unwrap().log_state, None);
}

/// Upstream's `overlayFollowRejectsBadInputBeforeCallingActions` and `followModeRefusesEverythingButALogPage`: the mode,
/// then the pane, before the session resolves; then the slot's occupant.
#[test]
fn follow_refuses_everything_but_a_log_page() {
    let (mut f, _) = fixture();
    let id = f.sessions[0];
    assert_eq!(
        follow(&mut f, "missing", Some("maybe")),
        failure("invalid overlay follow mode: maybe")
    );
    assert_eq!(
        f.session(
            SessionCmd::OverlayFollow,
            Some("missing"),
            args(|a| {
                a.mode = Some("on".to_owned());
                a.pane = Some("middle".to_owned());
            })
        ),
        failure("session.overlay: --pane must be left or right")
    );
    assert_eq!(
        follow(&mut f, "missing", Some("on")),
        failure("no such session: missing")
    );
    assert_eq!(follow(&mut f, &wire(id), Some("on")), failure("no overlay"));
    assert!(
        f.session(
            SessionCmd::OverlayOpen,
            Some(&wire(id)),
            args(|a| a.text_file = Some(path("a.txt")))
        )
        .ok
    );
    assert_eq!(follow(&mut f, &wire(id), Some("on")), failure("no log page"));
    assert!(f.session(SessionCmd::OverlayClose, Some(&wire(id)), Args::default()).ok);
    assert!(
        f.session(
            SessionCmd::OverlayOpen,
            Some(&wire(id)),
            args(|a| a.command = Some("less".to_owned()))
        )
        .ok
    );
    assert_eq!(
        follow(&mut f, &wire(id), Some("on")),
        failure("the overlay is not an html page")
    );
    // a page in a pane slot is addressed with --pane
    assert_eq!(
        f.session(
            SessionCmd::OverlayFollow,
            Some(&wire(id)),
            args(|a| a.pane = Some("left".to_owned()))
        ),
        failure("no overlay")
    );
}

/// A replica of another machine's log page keeps upstream's replica refusal: that machine alone follows its file.
#[test]
fn a_replica_log_page_is_followed_where_it_was_opened() {
    let (mut f, _) = fixture();
    let id = f.sessions[0];
    let replica = HtmlOverlay {
        replica_of: Some(Uuid::new_v4()),
        ..HtmlOverlay::new(
            HtmlSource::Replica {
                path: "/tmp/app.log".to_owned(),
            },
            HtmlPageFlags::default(),
        )
    }
    .with_text_log(Some(TextLog { tail: 10, follow: true }));
    f.store_mut()
        .unwrap()
        .open_html_overlay(id, None, replica, None, None)
        .unwrap();
    assert_eq!(follow(&mut f, &wire(id), Some("off")), failure(errors::REPLICA_SUBMIT));
}

/// Upstream's `textLogRefusal`: a log page the placement rule would hand to the presenter is refused, after its probe,
/// where a plain text page meets the presenter refusal.
#[test]
fn a_presented_session_refuses_a_log_page_after_its_probe() {
    let mut origin = Origin::build(false, &SUPPORTED_KINDS, false);
    let host = Rc::new(RefCell::new(Log::default()));
    origin.fixture().core.set_html_host(Box::new(Host(Rc::clone(&host))));
    let file = path("app.log");
    let open = |origin: &mut Origin, follow: bool| {
        origin.overlay(SessionCmd::OverlayOpen, |a| {
            a.text_file = Some(file.clone());
            a.follow_file = Some(follow);
        })
    };
    assert_eq!(open(&mut origin, true), failure(LOG_REMOTE));
    assert_eq!(host.borrow().checked, [Check::Log(file.clone(), 5_000)]);
    host.borrow_mut().failure = Some(format!("unreadable text file: {file}"));
    assert_eq!(
        open(&mut origin, true),
        failure(&format!("session.overlay.open: unreadable text file: {file}"))
    );
    host.borrow_mut().failure = None;
    assert_eq!(
        open(&mut origin, false),
        failure("a viewer presents this session: an html overlay would open where nobody sees it")
    );
}

/// Upstream's `logPageArgumentsRoundTripAndAreOmittedWhenUnset`.
#[test]
fn log_page_arguments_round_trip_and_are_omitted_when_unset() {
    let open = Request {
        cmd: Command::Session(SessionCmd::OverlayOpen),
        target: None,
        args: Some(args(|a| {
            a.text_file = Some("/tmp/a.log".to_owned());
            a.tail = Some(50);
            a.follow_file = Some(true);
        })),
    };
    let text = serde_json::to_string(&open).unwrap();
    assert!(
        text.contains(r#""tail":50"#) && text.contains(r#""followFile":true"#),
        "{text}"
    );
    assert_eq!(serde_json::from_str::<Request>(&text).unwrap(), open);
    let follow = Request {
        cmd: Command::Session(SessionCmd::OverlayFollow),
        target: None,
        args: Some(args(|a| {
            a.mode = Some("off".to_owned());
            a.pane = Some("left".to_owned());
        })),
    };
    let text = serde_json::to_string(&follow).unwrap();
    assert!(text.contains(r#""cmd":"session.overlay.follow""#), "{text}");
    assert_eq!(serde_json::from_str::<Request>(&text).unwrap(), follow);
    let plain = serde_json::to_string(&args(|a| a.text_file = Some("/tmp/a.txt".to_owned()))).unwrap();
    assert!(!plain.contains("tail") && !plain.contains("followFile"), "{plain}");
}
