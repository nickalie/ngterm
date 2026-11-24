//! Text page actions on a page the presenting machine shows, on the origin (upstream's `AppStoreRemotePageTests` text
//! cases and `ControlServerRemotePageTests`' origin control, #978): `session.overlay.follow|search|wrap|changes` travel
//! to a presenter listing `page.text` and answer with its result, whose state the tree shows at once; an older
//! presenter keeps the local refusals; the presenter's reports mirror the text state; a page handed on carries its
//! seed.

use ngterm_proto::command::{Command, SessionCmd};
use ngterm_proto::{Args, Request, Response};
use serde_json::{Value, json};

use super::content_pages::{Files, content_origin, open_content};
use super::remote_page::Origin;
use super::{args, failure, wire};
use crate::Core;
use crate::dispatch::Dispatched;
use crate::remote::hub::{PAGE_TEXT_KIND, SUPPORTED_KINDS};

const VIEWER_GONE: &str = "the viewer showing this overlay is gone";
const SILENT: &str = "presenter did not answer";
const SEARCH_REMOTE: &str = "remote page: search it on the Mac that shows it";

/// A presented session showing a following log page of `name` on a presenter listing `kinds`; the page's id.
fn log_origin(files: &Files, name: &str, kinds: &[&str]) -> (Origin, String) {
    std::fs::write(files.root.join(name), "one\ntwo\n").unwrap();
    let mut origin = content_origin(kinds);
    let response = open_content(&mut origin, "text", &files.path(name), |a| a.follow_file = Some(true));
    assert!(response.ok, "{response:?}");
    let page = response.result.unwrap()["pageID"].as_str().unwrap().to_owned();
    (origin, page)
}

/// A plain text page of `name`; the page's id.
fn text_origin(files: &Files, name: &str, kinds: &[&str]) -> (Origin, String) {
    std::fs::write(files.root.join(name), "-a\n+b\n").unwrap();
    let mut origin = content_origin(kinds);
    let response = open_content(&mut origin, "text", &files.path(name), |_| {});
    assert!(response.ok, "{response:?}");
    let page = response.result.unwrap()["pageID"].as_str().unwrap().to_owned();
    (origin, page)
}

fn request(origin: &Origin, cmd: SessionCmd, edit: impl FnOnce(&mut Args)) -> Request {
    Request {
        cmd: Command::Session(cmd),
        target: Some(wire(origin.session())),
        args: Some(args(edit)),
    }
}

/// Starts `cmd` on the origin, which must wait for the presenter.
fn begin(origin: &mut Origin, cmd: SessionCmd, edit: impl FnOnce(&mut Args)) -> Dispatched {
    let request = request(origin, cmd, edit);
    let dispatched = origin.fixture().core.begin(&request);
    assert!(matches!(dispatched, Dispatched::Pending { .. }), "{dispatched:?}");
    dispatched
}

/// Runs one pending step.
fn step(core: &mut Core, dispatched: Dispatched) -> Dispatched {
    match dispatched {
        Dispatched::Pending { resume, .. } => resume(core),
        done @ Dispatched::Done(_) => done,
    }
}

fn answered(dispatched: Dispatched) -> Response {
    match dispatched {
        Dispatched::Done(response) => response,
        Dispatched::Pending { .. } => panic!("still waiting"),
    }
}

/// The last `page.command` the presenter got.
fn last_command(origin: &Origin) -> Value {
    origin.presenter.last("page.command").unwrap()["page"].clone()
}

/// The presenter answers `command` with `result` fields; the `tree.changed` the commit after it fires.
fn answer(origin: &mut Origin, command: &Value, fields: &Value) -> usize {
    let mut result = json!({"page": command["page"], "seq": command["seq"]});
    result
        .as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    origin.report(&json!({"kind": "page.result", "page": result}))
}

fn text_node(origin: &mut Origin) -> Value {
    origin.node()["htmlOverlays"][0]["text"].clone()
}

/// Upstream's `testFollowOnARemoteLogPageRunsOnThePresenter`: the action goes as a numbered page command, the reply is
/// the presenter's, and the tree read right after it shows the mirrored state.
#[test]
fn a_follow_runs_on_the_presenter_and_the_tree_shows_its_state() {
    let files = Files::new();
    let (mut origin, page) = log_origin(&files, "app.log", &SUPPORTED_KINDS);
    let waiting = begin(&mut origin, SessionCmd::OverlayFollow, |a| {
        a.mode = Some("off".to_owned());
    });
    let command = last_command(&origin);
    assert_eq!(command["page"], json!(page));
    assert_eq!(
        (command["action"].clone(), command["toggle"].clone()),
        (json!("follow"), json!("off"))
    );
    assert!(command.get("query").is_none() && command.get("step").is_none());
    let state = json!({"following": false, "log": {"lines": 2, "clipped": false, "waiting": false, "unreadable": false},
        "revision": 0, "wrap": false});
    assert_eq!(
        answer(
            &mut origin,
            &command,
            &json!({"ok": true, "text": "paused", "state": state})
        ),
        1,
        "a follow change emits"
    );
    let response = answered(step(&mut origin.fixture().core, waiting));
    assert_eq!(
        response.result.unwrap(),
        json!({"id": wire(origin.session()), "text": "paused"})
    );
    let text = text_node(&mut origin);
    assert_eq!(
        (text["following"].clone(), text["lines"].clone()),
        (json!(false), json!(2))
    );
}

/// A command, its arguments, the fields it travels with and the presenter's answer.
type Case = (SessionCmd, fn(&mut Args), Value, Value);

/// Upstream's `testSearchWrapAndChangesOnARemotePageAnswerAsLocally`: each command's fields, and the presenter's reply
/// or refusal as the caller's.
#[test]
fn search_wrap_and_changes_carry_their_fields_and_answer_with_the_presenter() {
    let files = Files::new();
    let (mut origin, _) = text_origin(&files, "change.diff", &SUPPORTED_KINDS);
    let cases: [Case; 4] = [
        (
            SessionCmd::OverlaySearch,
            |a| a.text = Some("main".to_owned()),
            json!({"action": "search", "query": "main"}),
            json!({"ok": true}),
        ),
        (
            SessionCmd::OverlaySearch,
            |a| a.to = Some("prev".to_owned()),
            json!({"action": "search", "step": "prev"}),
            json!({"ok": true, "text": "2 of 3"}),
        ),
        (
            SessionCmd::OverlayWrap,
            |_| {},
            json!({"action": "wrap", "toggle": "toggle"}),
            json!({"ok": true, "text": "wrapped"}),
        ),
        (
            SessionCmd::OverlayChanges,
            |a| a.to = Some("next".to_owned()),
            json!({"action": "changes", "step": "next"}),
            json!({"ok": false, "error": "page not loaded"}),
        ),
    ];
    let mut seq = 0;
    for (cmd, edit, sent, reply) in cases {
        let waiting = begin(&mut origin, cmd, edit);
        let command = last_command(&origin);
        for (key, value) in sent.as_object().unwrap() {
            assert_eq!(&command[key], value, "{cmd:?} {key}");
        }
        assert!(
            command["seq"].as_i64().unwrap() > seq,
            "each action is numbered after the last"
        );
        seq = command["seq"].as_i64().unwrap();
        answer(&mut origin, &command, &reply);
        let response = answered(step(&mut origin.fixture().core, waiting));
        if reply["ok"] == true {
            let mut expected = json!({"id": wire(origin.session())});
            if let Some(text) = reply.get("text") {
                expected["text"] = text.clone();
            }
            assert_eq!(response.result.unwrap(), expected, "{cmd:?}");
        } else {
            assert_eq!(response, failure("page not loaded"), "{cmd:?}");
        }
    }
}

/// Upstream's `testARemoteTextActionTimesOut`: no result within the deadline.
#[test]
fn a_presenter_that_never_answers_is_reported_silent() {
    let files = Files::new();
    let (mut origin, _) = log_origin(&files, "app.log", &SUPPORTED_KINDS);
    let response = origin.overlay(SessionCmd::OverlayFollow, |a| a.mode = Some("on".to_owned()));
    assert_eq!(response, failure(SILENT));
    assert!(origin.fixture().core.remote.text_waits_empty(), "the wait is dropped");
}

/// Upstream's `testARemoteTextActionSettlesWhenThePageEnds` and `aTextActionOnAPageHeldForNoMacAnswersViewerGone`.
#[test]
fn a_page_that_ends_or_parks_while_an_action_waits_answers_the_viewer_is_gone() {
    let files = Files::new();
    let (mut origin, page) = log_origin(&files, "app.log", &SUPPORTED_KINDS);
    let waiting = begin(&mut origin, SessionCmd::OverlayWrap, |a| a.mode = Some("on".to_owned()));
    origin.report(&json!({"kind": "page.closed", "page": {"page": page}}));
    assert_eq!(
        answered(step(&mut origin.fixture().core, waiting)),
        failure(VIEWER_GONE)
    );

    let (mut origin, page) = log_origin(&files, "app.log", &SUPPORTED_KINDS);
    let waiting = begin(&mut origin, SessionCmd::OverlayWrap, |a| a.mode = Some("on".to_owned()));
    let command = last_command(&origin);
    let before = text_node(&mut origin);
    origin.report(&json!({"kind": "page.detached", "page": {"page": page}}));
    // a result for a page its presenter detached settles nothing and mirrors nothing
    answer(
        &mut origin,
        &command,
        &json!({"ok": true, "text": "wrapped", "state": {"following": false, "revision": 0, "wrap": true}}),
    );
    assert_eq!(
        answered(step(&mut origin.fixture().core, waiting)),
        failure(VIEWER_GONE)
    );
    assert_eq!(text_node(&mut origin), before);
    assert_eq!(
        origin.overlay(SessionCmd::OverlayFollow, |_| {}),
        failure(VIEWER_GONE),
        "a parked page has no presenter to run on: {page}"
    );
}

/// Upstream's `withdrawRemotePage` settling the job's waits: a page handed to a new presenter while an action waits
/// answers at once, as the new presenter never got the command.
#[test]
fn a_page_handed_to_another_presenter_while_an_action_waits_answers_the_viewer_is_gone() {
    let files = Files::new();
    let (mut origin, _) = log_origin(&files, "app.log", &SUPPORTED_KINDS);
    let waiting = begin(&mut origin, SessionCmd::OverlayWrap, |a| a.mode = Some("on".to_owned()));
    origin.lose_presenter();
    origin.reattach(&SUPPORTED_KINDS);
    assert!(
        origin.presenter.last("page.request").is_some(),
        "the page went to the new presenter"
    );
    assert_eq!(
        answered(step(&mut origin.fixture().core, waiting)),
        failure(VIEWER_GONE)
    );
    assert!(origin.fixture().core.remote.text_waits_empty(), "the wait is dropped");
}

/// Upstream's capability split: a presenter that does not list `page.text` is sent nothing and the local refusal
/// answers.
#[test]
fn an_older_presenter_is_sent_no_text_action() {
    let files = Files::new();
    let older: Vec<&str> = SUPPORTED_KINDS
        .into_iter()
        .filter(|kind| *kind != PAGE_TEXT_KIND)
        .collect();
    let (mut origin, _) = text_origin(&files, "notes.txt", &older);
    assert_eq!(
        origin.overlay(SessionCmd::OverlaySearch, |a| a.text = Some("x".to_owned())),
        failure(SEARCH_REMOTE)
    );
    assert!(origin.presenter.last("page.command").is_none());
}

/// Upstream's `aReportChangingOnlyCountsEmitsNothing` and `aFollowQueryOrWrapChangeEmitsOnce`.
#[test]
fn a_report_emits_only_on_a_follow_query_or_wrap_change() {
    let files = Files::new();
    let (mut origin, page) = log_origin(&files, "app.log", &SUPPORTED_KINDS);
    origin.tree_changes();
    let report = |origin: &mut Origin, text: Value| {
        origin.report(
            &json!({"kind": "page.state", "page": {"page": page, "load": "loaded", "canGoBack": false,
            "canGoForward": false, "hidden": false, "appliedSeq": 0, "text": text}}),
        )
    };
    let base = json!({"following": true, "log": {"lines": 5}, "revision": 0, "wrap": false});
    assert_eq!(report(&mut origin, base.clone()), 0, "the counts alone");
    let mut counted = base.clone();
    counted["log"]["lines"] = json!(9);
    assert_eq!(report(&mut origin, counted), 0, "more lines");
    for (key, value) in [
        ("following", json!(false)),
        ("query", json!("needle")),
        ("wrap", json!(true)),
    ] {
        let mut changed = base.clone();
        changed[key] = value;
        if key == "query" {
            changed["revision"] = json!(1);
            changed["matches"] = json!(4);
            changed["current"] = json!(2);
        }
        assert_eq!(report(&mut origin, changed), 1, "{key}");
        assert_eq!(report(&mut origin, base.clone()), 1, "{key} back");
    }
    let mut searched = base;
    searched["query"] = json!("needle");
    searched["revision"] = json!(3);
    searched["matches"] = json!(4);
    searched["current"] = json!(2);
    report(&mut origin, searched);
    let text = text_node(&mut origin);
    assert_eq!(text["search"], json!({"query": "needle", "matches": 4, "current": 2}));
    assert_eq!(text["lines"], 5);
}

/// Upstream's `offerPendingPage` seed: a page handed on resumes with the following, query and wrap mirrored last.
#[test]
fn a_page_handed_on_carries_its_text_seed() {
    let files = Files::new();
    let (mut origin, page) = log_origin(&files, "app.log", &SUPPORTED_KINDS);
    assert!(
        origin.presenter.last("page.request").unwrap()["page"]
            .get("textSeed")
            .is_none(),
        "a fresh open has no seed"
    );
    origin.report(
        &json!({"kind": "page.state", "page": {"page": page, "load": "loaded", "canGoBack": false,
        "canGoForward": false, "hidden": false, "appliedSeq": 0,
        "text": {"following": false, "query": "err", "revision": 2, "wrap": true}}}),
    );
    origin.lose_presenter();
    origin.reattach(&SUPPORTED_KINDS);
    let request = origin.presenter.last("page.request").unwrap()["page"].clone();
    assert_eq!(request["page"], json!(page));
    assert_eq!(request["textLog"], json!({"tail": 5_000, "follow": true}));
    let seed = &request["textSeed"];
    assert_eq!(
        (seed["following"].clone(), seed["query"].clone(), seed["wrap"].clone()),
        (json!(false), json!("err"), json!(true))
    );
}

/// Upstream's held log page: a parked log page waits for a presenter listing `page.text`.
#[test]
fn a_parked_log_page_waits_for_a_presenter_with_live_text_pages() {
    let files = Files::new();
    let (mut origin, page) = log_origin(&files, "app.log", &SUPPORTED_KINDS);
    origin.lose_presenter();
    let older: Vec<&str> = SUPPORTED_KINDS
        .into_iter()
        .filter(|kind| *kind != PAGE_TEXT_KIND)
        .collect();
    origin.reattach(&older);
    assert!(
        origin.presenter.last("page.request").is_none(),
        "held for a later presenter"
    );
    origin.lose_presenter();
    origin.reattach(&SUPPORTED_KINDS);
    assert_eq!(
        origin.presenter.last("page.request").unwrap()["page"]["page"],
        json!(page)
    );
}
