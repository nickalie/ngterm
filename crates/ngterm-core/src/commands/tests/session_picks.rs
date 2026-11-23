//! Session-bound picks and `--timeout` (upstream's `SessionPickTests`, `ControlDispatcherPickTests`,
//! `ControlDispatcherAskTests`, `PickTests` and `AskTests` cases of #949): a pick opened with a session target waits on
//! that session, drawn by its presenter when it presents remotely and in its own window otherwise; an ask or a pick
//! opened with a timeout ends cancelled when it passes.

use std::time::Duration;

use ngterm_proto::command::{AskCmd, PickCmd, SessionCmd};
use ngterm_proto::{Args, AskButton, Command, ControlPickOutcome, ControlPickResult, PickItem, Response};
use serde_json::{Value, json};
use uuid::Uuid;

use super::live::Live;
use super::remote::{Viewer, frame, node, presented_listing};
use super::{Fixture, UNKNOWN, args, wire};
use crate::modal::{InteractionExpiry, TIMEOUT_ERROR};
use crate::remote::hub::PICK_KINDS;

fn items() -> Vec<PickItem> {
    ["Alpha", "Beta"]
        .into_iter()
        .map(|label| PickItem {
            id: label[..1].to_lowercase(),
            label: label.to_owned(),
            ..PickItem::default()
        })
        .collect()
}

fn open(fixture: &mut Fixture, session: Option<Uuid>, edit: impl FnOnce(&mut Args)) -> Response {
    let target = session.map(wire);
    let response = fixture.send(
        Command::Pick(PickCmd::Open),
        target.as_deref(),
        args(|a| {
            a.items = Some(items());
            edit(a);
        }),
    );
    fixture.core.commit();
    response
}

fn opened(response: &Response) -> String {
    assert!(response.ok, "{response:?}");
    response.result.as_ref().unwrap()["id"].as_str().unwrap().to_owned()
}

fn result(fixture: &mut Fixture, id: &str) -> Value {
    let response = fixture.send(Command::Pick(PickCmd::Result), Some(id), Args::default());
    assert!(response.ok, "{response:?}");
    response.result.unwrap()["pick"].clone()
}

fn cancel(fixture: &mut Fixture, id: &str) -> Response {
    let response = fixture.send(Command::Pick(PickCmd::Cancel), Some(id), Args::default());
    fixture.core.commit();
    response
}

/// The error a refused response carries.
fn refusal(response: &Response) -> String {
    assert!(!response.ok, "{response:?}");
    response.error.clone().unwrap_or_default()
}

fn picked_beta() -> ControlPickResult {
    ControlPickResult {
        id: Some("b".to_owned()),
        label: Some("Beta".to_owned()),
        index: Some(1),
        ..ControlPickResult::outcome(ControlPickOutcome::Picked)
    }
}

fn slot_pick(fixture: &Fixture, window: Uuid) -> Option<String> {
    fixture
        .core
        .modal_slot(window)
        .and_then(|slot| slot.pending())
        .map(|pick| pick.id.clone())
}

// MARK: - a pick bound to a session, drawn here

/// Upstream's `aLocalPickAnsweredWithNoPollFreesTheSessionForTheNextOne` through `pick.open`.
#[test]
fn a_local_session_pick_waits_on_its_session_and_frees_it_when_answered() {
    let mut fixture = Fixture::new(1);
    let session = fixture.sessions[0];
    assert_eq!(node(&mut fixture, session).get("pick"), None);
    let id = opened(&open(&mut fixture, Some(session), |a| {
        a.prompt = Some("Branch".to_owned());
    }));
    assert_eq!(node(&mut fixture, session)["pick"], json!({"id": id}));
    assert_eq!(slot_pick(&fixture, fixture.window), Some(id.clone()));
    assert_eq!(result(&mut fixture, &id), json!({"result": "pending"}));

    fixture.core.resolve_pick(fixture.window, &id, picked_beta());
    fixture.core.commit();
    assert_eq!(node(&mut fixture, session).get("pick"), None);
    assert_eq!(
        result(&mut fixture, &id),
        json!({"result": "picked", "id": "b", "label": "Beta", "index": 1})
    );
    opened(&open(&mut fixture, Some(session), |_| {}));
}

/// Upstream's `aSecondPickOnTheSameSessionIsRefused` and `aWindowHoldingAnAskRefusesTheLocalPick`.
#[test]
fn a_second_pick_on_the_session_or_a_held_slot_refuses_it() {
    let mut fixture = Fixture::new(1);
    let (first, second) = (fixture.sessions[0], fixture.sessions[1]);
    opened(&open(&mut fixture, Some(first), |_| {}));
    assert_eq!(
        refusal(&open(&mut fixture, Some(first), |_| {})),
        "pick already pending"
    );
    assert_eq!(
        refusal(&open(&mut fixture, Some(second), |_| {})),
        "pick already pending",
        "the window's slot holds the first"
    );

    let mut fixture = Fixture::new(0);
    let session = fixture.sessions[0];
    let ask = fixture.send(
        Command::Ask(AskCmd::Open),
        None,
        args(|a| {
            a.title = Some("t".to_owned());
            a.style = Some("gui".to_owned());
            a.buttons = Some(vec![AskButton {
                id: "ok".to_owned(),
                label: "OK".to_owned(),
                hotkey: None,
            }]);
        }),
    );
    assert!(ask.ok, "{ask:?}");
    assert_eq!(
        refusal(&open(&mut fixture, Some(session), |_| {})),
        "ask already pending"
    );
    assert_eq!(node(&mut fixture, session).get("pick"), None);
}

/// The session's own window draws it, whichever window is frontmost.
#[test]
fn a_local_session_pick_opens_in_the_sessions_window() {
    let mut fixture = Fixture::new(0);
    let session = fixture.sessions[0];
    let other = fixture.core.library_mut().new_window(None);
    fixture.core.take_frontmost(other);
    let id = opened(&open(&mut fixture, Some(session), |a| a.follow = Some(true)));
    assert_eq!(slot_pick(&fixture, fixture.window), Some(id));
    assert_eq!(slot_pick(&fixture, other), None);
    assert_eq!(fixture.core.library().frontmost_window_id, Some(fixture.window));
}

/// Upstream's `cancellingALocalPickEndsItInItsWindow` and `aPickKeepsItsOwningWindowAfterItEnds`.
#[test]
fn cancelling_a_local_session_pick_ends_it_in_its_window() {
    let mut fixture = Fixture::new(0);
    let session = fixture.sessions[0];
    let id = opened(&open(&mut fixture, Some(session), |_| {}));
    assert!(cancel(&mut fixture, &id).ok);
    assert_eq!(slot_pick(&fixture, fixture.window), None);
    assert_eq!(result(&mut fixture, &id), json!({"result": "cancelled"}));
    assert!(cancel(&mut fixture, &id).ok, "cancelling a finished pick is a no-op");
    assert_eq!(refusal(&cancel(&mut fixture, UNKNOWN)), "unknown pick: zzz");
}

/// With `--window`, a session pick answers only in its own window.
#[test]
fn a_session_pick_answers_window_scoped_commands_only_in_its_window() {
    let mut fixture = Fixture::new(0);
    let session = fixture.sessions[0];
    let other = fixture.core.library_mut().new_window(None);
    let id = opened(&open(&mut fixture, Some(session), |_| {}));
    let scoped = |fixture: &mut Fixture, cmd: PickCmd, window: Uuid| {
        let response = fixture.send(Command::Pick(cmd), Some(&id), args(|a| a.window = Some(wire(window))));
        fixture.core.commit();
        response
    };
    let unknown = format!("unknown pick: {id}");
    assert_eq!(refusal(&scoped(&mut fixture, PickCmd::Result, other)), unknown);
    assert_eq!(refusal(&scoped(&mut fixture, PickCmd::Cancel, other)), unknown);
    assert_eq!(result(&mut fixture, &id), json!({"result": "pending"}));

    let own = fixture.window;
    let pending = scoped(&mut fixture, PickCmd::Result, own);
    assert_eq!(pending.result.unwrap()["pick"], json!({"result": "pending"}));
    assert!(scoped(&mut fixture, PickCmd::Cancel, own).ok);
    assert_eq!(result(&mut fixture, &id), json!({"result": "cancelled"}));
}

/// Upstream's `theSessionLeavingCancelsItsPick`.
#[test]
fn the_session_leaving_cancels_its_pick() {
    let mut fixture = Fixture::new(1);
    let session = fixture.sessions[0];
    let id = opened(&open(&mut fixture, Some(session), |_| {}));
    let closed = fixture.session(SessionCmd::Close, Some(&wire(session)), Args::default());
    assert!(closed.ok, "{closed:?}");
    fixture.core.commit();
    assert_eq!(slot_pick(&fixture, fixture.window), None);
    assert_eq!(result(&mut fixture, &id), json!({"result": "cancelled"}));
}

#[test]
fn an_unknown_session_is_refused() {
    let mut fixture = Fixture::new(0);
    let response = fixture.send(
        Command::Pick(PickCmd::Open),
        Some(UNKNOWN),
        args(|a| a.items = Some(items())),
    );
    assert_eq!(refusal(&response), "no such session: zzz");
    assert_eq!(slot_pick(&fixture, fixture.window), None);
}

// MARK: - timeouts

/// Upstream's `anInvalidTimeoutNeverReachesTheHost` for both commands.
#[test]
fn a_timeout_out_of_range_is_refused_before_anything_opens() {
    for timeout in [0.0, -1.0, 86_400.5, f64::INFINITY, f64::NAN] {
        let mut fixture = Fixture::new(0);
        assert_eq!(
            refusal(&open(&mut fixture, None, |a| a.timeout = Some(timeout))),
            TIMEOUT_ERROR
        );
        let ask = fixture.send(
            Command::Ask(AskCmd::Open),
            None,
            args(|a| {
                a.title = Some("t".to_owned());
                a.buttons = Some(vec![AskButton {
                    id: "ok".to_owned(),
                    label: "OK".to_owned(),
                    hotkey: None,
                }]);
                a.timeout = Some(timeout);
            }),
        );
        assert_eq!(refusal(&ask), TIMEOUT_ERROR);
        assert!(!fixture.core.modal_pending(fixture.window), "{timeout}");
        assert!(fixture.core.take_interaction_expiries().is_empty());
    }
}

fn expiry(fixture: &mut Fixture) -> InteractionExpiry {
    let mut expiries = fixture.core.take_interaction_expiries();
    assert_eq!(expiries.len(), 1, "{expiries:?}");
    expiries.remove(0)
}

/// Upstream's `timed`: the deadline ends a pick still pending; an expiry that fires early is armed again.
#[test]
fn a_timed_pick_ends_cancelled_at_its_deadline() {
    let mut fixture = Fixture::new(0);
    let id = opened(&open(&mut fixture, None, |a| a.timeout = Some(60.0)));
    let early = expiry(&mut fixture);
    assert_eq!(early.delay, Duration::from_secs(60));
    fixture.core.expire_interaction(early);
    assert_eq!(result(&mut fixture, &id), json!({"result": "pending"}));
    assert!(
        expiry(&mut fixture).delay <= Duration::from_secs(60),
        "the rest is armed again"
    );

    let mut fixture = Fixture::new(0);
    let session = fixture.sessions[0];
    let id = opened(&open(&mut fixture, Some(session), |a| a.timeout = Some(0.01)));
    let due = expiry(&mut fixture);
    std::thread::sleep(Duration::from_millis(20));
    fixture.core.expire_interaction(due);
    fixture.core.commit();
    assert_eq!(result(&mut fixture, &id), json!({"result": "cancelled"}));
    assert_eq!(node(&mut fixture, session).get("pick"), None);
    assert_eq!(slot_pick(&fixture, fixture.window), None);
}

/// The deadline runs on the modal clock: an expiry that fires early is armed again for the rest, and an answer
/// before the deadline stays answered while one past it resolves cancelled.
#[test]
fn a_timed_pick_keeps_its_deadline_on_the_modal_clock() {
    let mut fixture = Fixture::new(0);
    fixture.core.modals.advance_clock(Duration::from_secs(1000));
    let id = opened(&open(&mut fixture, None, |a| a.timeout = Some(60.0)));
    let early = expiry(&mut fixture);
    fixture.core.modals.advance_clock(Duration::from_secs(45));
    fixture.core.expire_interaction(early);
    let rest = expiry(&mut fixture).delay;
    assert!(
        rest > Duration::from_secs(14) && rest <= Duration::from_secs(15),
        "{rest:?}"
    );
    fixture.core.resolve_pick(fixture.window, &id, picked_beta());
    assert_eq!(result(&mut fixture, &id), json!(picked_beta()));

    let mut fixture = Fixture::new(0);
    let id = opened(&open(&mut fixture, None, |a| a.timeout = Some(60.0)));
    let due = expiry(&mut fixture);
    fixture.core.modals.advance_clock(Duration::from_secs(60));
    fixture.core.expire_interaction(due);
    fixture.core.commit();
    assert!(fixture.core.take_interaction_expiries().is_empty());
    assert_eq!(result(&mut fixture, &id), json!({"result": "cancelled"}));

    let mut fixture = Fixture::new(0);
    let id = opened(&open(&mut fixture, None, |a| a.timeout = Some(60.0)));
    fixture.core.modals.advance_clock(Duration::from_secs(60));
    fixture.core.resolve_pick(fixture.window, &id, picked_beta());
    assert_eq!(result(&mut fixture, &id), json!({"result": "cancelled"}));
}

/// Upstream's `aPickAnsweredPastItsDeadlineResolvesCancelled`.
#[test]
fn a_pick_answered_past_its_deadline_resolves_cancelled() {
    let mut fixture = Fixture::new(0);
    let id = opened(&open(&mut fixture, None, |a| a.timeout = Some(0.01)));
    std::thread::sleep(Duration::from_millis(20));
    fixture.core.resolve_pick(fixture.window, &id, picked_beta());
    assert_eq!(result(&mut fixture, &id), json!({"result": "cancelled"}));
}

fn open_ask(fixture: &mut Fixture, style: &str, timeout: f64) -> String {
    let target = wire(fixture.sessions[0]);
    let response = fixture.send(
        Command::Ask(AskCmd::Open),
        Some(&target),
        args(|a| {
            a.title = Some("t".to_owned());
            a.style = Some(style.to_owned());
            a.buttons = Some(vec![AskButton {
                id: "ok".to_owned(),
                label: "OK".to_owned(),
                hotkey: None,
            }]);
            a.timeout = Some(timeout);
        }),
    );
    fixture.core.commit();
    opened(&response)
}

fn ask_outcome(fixture: &mut Fixture, id: &str) -> Value {
    let response = fixture.send(Command::Ask(AskCmd::Result), Some(id), Args::default());
    assert!(response.ok, "{response:?}");
    response.result.unwrap()["ask"]["result"].clone()
}

/// Upstream's `aTimeoutBecomesTheAsksDeadline` with `timed`: a terminal ask and a GUI one end cancelled at their
/// deadline.
#[test]
fn a_timed_ask_ends_cancelled_at_its_deadline() {
    for style in ["terminal", "gui"] {
        let mut fixture = Fixture::new(0);
        let id = open_ask(&mut fixture, style, 0.01);
        let due = expiry(&mut fixture);
        assert_eq!(due.delay, Duration::from_millis(10));
        std::thread::sleep(Duration::from_millis(20));
        fixture.core.expire_interaction(due);
        fixture.core.commit();
        assert_eq!(ask_outcome(&mut fixture, &id), json!("cancelled"), "{style}");
        assert!(!fixture.core.modal_pending(fixture.window));
    }
}

/// Upstream's `anAnswerPastTheDeadlineResolvesCancelledBeforeAnyExpiryRuns`, for a terminal ask and a GUI one.
#[test]
fn an_ask_answered_past_its_deadline_resolves_cancelled() {
    let mut fixture = Fixture::new(0);
    let session = fixture.sessions[0];
    let id = open_ask(&mut fixture, "terminal", 0.01);
    std::thread::sleep(Duration::from_millis(20));
    fixture.core.answer_session_ask_button(session, &id, 0);
    fixture.core.commit();
    assert_eq!(ask_outcome(&mut fixture, &id), json!("cancelled"));

    let mut fixture = Fixture::new(0);
    let id = open_ask(&mut fixture, "gui", 0.01);
    std::thread::sleep(Duration::from_millis(20));
    fixture.core.answer_window_ask_button(fixture.window, &id, 0);
    fixture.core.commit();
    assert_eq!(ask_outcome(&mut fixture, &id), json!("cancelled"));
}

/// An ask answered once the modal clock passed its deadline resolves cancelled, a terminal ask and a GUI one.
#[test]
fn an_ask_answered_past_its_deadline_on_the_modal_clock_resolves_cancelled() {
    let mut fixture = Fixture::new(0);
    let session = fixture.sessions[0];
    let id = open_ask(&mut fixture, "terminal", 60.0);
    fixture.core.modals.advance_clock(Duration::from_secs(60));
    fixture.core.answer_session_ask_button(session, &id, 0);
    fixture.core.commit();
    assert_eq!(ask_outcome(&mut fixture, &id), json!("cancelled"));

    let mut fixture = Fixture::new(0);
    let id = open_ask(&mut fixture, "gui", 60.0);
    fixture.core.modals.advance_clock(Duration::from_secs(60));
    fixture.core.answer_window_ask_button(fixture.window, &id, 0);
    fixture.core.commit();
    assert_eq!(ask_outcome(&mut fixture, &id), json!("cancelled"));
}

// MARK: - a pick bound to a presented session

fn open_live(live: &mut Live, edit: impl FnOnce(&mut Args)) -> Response {
    let session = Some(live.session);
    open(&mut live.fixture, session, edit)
}

fn request(presenter: &Viewer) -> Value {
    presenter.last("pick.request").expect("a pick request")["pick"].clone()
}

fn answer(live: &mut Live, serial: u64, presenter: &Viewer, body: &Value) {
    let encoded = frame(presenter, body);
    live.fixture.core.presentation_line(serial, &encoded);
    live.fixture.core.commit();
}

/// Upstream's `aPresentedSessionHandsThePickToItsPresenter` and `theRequestCarriesFollow`.
#[test]
fn a_presented_session_hands_the_pick_to_its_presenter() {
    let (mut live, _, presenter) = presented_listing(&PICK_KINDS);
    let session = live.session;
    let id = opened(&open_live(&mut live, |a| {
        a.prompt = Some("Branch".to_owned());
        a.allow_custom = Some(true);
        a.follow = Some(true);
    }));
    let sent = request(&presenter);
    assert_eq!(
        sent,
        json!({"id": id, "owner": sent["owner"], "items": [{"id": "a", "label": "Alpha"}, {"id": "b", "label": "Beta"}],
            "prompt": "Branch", "allowCustom": true, "follow": true})
    );
    assert_eq!(
        node(&mut live.fixture, session)["pick"],
        json!({"id": id, "remote": true})
    );
    assert_eq!(slot_pick(&live.fixture, live.fixture.window), None);
    assert_eq!(result(&mut live.fixture, &id), json!({"result": "pending"}));
    assert_eq!(refusal(&open_live(&mut live, |_| {})), "pick already pending");

    let (mut live, _, presenter) = presented_listing(&PICK_KINDS);
    opened(&open_live(&mut live, |_| {}));
    assert_eq!(request(&presenter).get("follow"), None);
}

/// Upstream's `aPresenterWithoutThePickKindsKeepsThePickLocal`.
#[test]
fn a_presenter_without_the_pick_kinds_keeps_the_pick_here() {
    let (mut live, _, presenter) = presented_listing(&["status"]);
    let id = opened(&open_live(&mut live, |_| {}));
    assert!(presenter.last("pick.request").is_none());
    assert_eq!(slot_pick(&live.fixture, live.fixture.window), Some(id));
}

/// Upstream's `thePresentersAnswerResolvesUnderItsOwnerOnly`.
#[test]
fn the_presenters_answer_resolves_under_its_owner_only() {
    let (mut live, serial, presenter) = presented_listing(&PICK_KINDS);
    let session = live.session;
    let id = opened(&open_live(&mut live, |_| {}));
    let owner = request(&presenter)["owner"].as_i64().unwrap();
    for refused in [
        json!({"id": id, "owner": owner + 1, "item": "a"}),
        json!({"id": id, "owner": owner, "item": "zzz"}),
        json!({"id": id, "owner": owner, "query": "typed"}),
    ] {
        answer(
            &mut live,
            serial,
            &presenter,
            &json!({"kind": "pick.resolve", "pick": refused}),
        );
        assert_eq!(result(&mut live.fixture, &id), json!({"result": "pending"}));
    }
    answer(
        &mut live,
        serial,
        &presenter,
        &json!({"kind": "pick.resolve", "pick": {"id": id, "owner": owner, "item": "b"}}),
    );
    assert_eq!(
        result(&mut live.fixture, &id),
        json!({"result": "picked", "id": "b", "label": "Beta", "index": 1})
    );
    assert_eq!(node(&mut live.fixture, session).get("pick"), None);
}

/// Upstream's `cancellingAPresentedPickDismissesItOnThePresenter`, and the same at the pick's deadline.
#[test]
fn cancelling_a_presented_pick_dismisses_it_on_the_presenter() {
    let (mut live, _, presenter) = presented_listing(&PICK_KINDS);
    let id = opened(&open_live(&mut live, |_| {}));
    let owner = request(&presenter)["owner"].clone();
    assert!(cancel(&mut live.fixture, &id).ok);
    assert_eq!(
        presenter.last("pick.dismiss").unwrap()["pick"],
        json!({"id": id, "owner": owner})
    );
    assert_eq!(result(&mut live.fixture, &id), json!({"result": "cancelled"}));

    let id = opened(&open_live(&mut live, |a| a.timeout = Some(0.01)));
    let due = expiry(&mut live.fixture);
    std::thread::sleep(Duration::from_millis(20));
    live.fixture.core.expire_interaction(due);
    live.fixture.core.commit();
    assert_eq!(presenter.last("pick.dismiss").unwrap()["pick"]["id"], json!(id));
    assert_eq!(result(&mut live.fixture, &id), json!({"result": "cancelled"}));
}

/// Upstream's `aRefusedPickMovesIntoTheSessionsWindow`: a refusal under another owner takes nothing back.
#[test]
fn a_refused_pick_moves_into_the_sessions_window() {
    let (mut live, serial, presenter) = presented_listing(&PICK_KINDS);
    let session = live.session;
    let id = opened(&open_live(&mut live, |_| {}));
    let owner = request(&presenter)["owner"].as_i64().unwrap();
    answer(
        &mut live,
        serial,
        &presenter,
        &json!({"kind": "pick.rejected", "pick": {"id": id, "owner": owner + 1}}),
    );
    assert_eq!(slot_pick(&live.fixture, live.fixture.window), None);
    answer(
        &mut live,
        serial,
        &presenter,
        &json!({"kind": "pick.rejected", "pick": {"id": id, "owner": owner}}),
    );
    let window = live.fixture.window;
    assert_eq!(slot_pick(&live.fixture, window), Some(id.clone()));
    assert_eq!(node(&mut live.fixture, session)["pick"], json!({"id": id}));
    live.fixture.core.resolve_pick(window, &id, picked_beta());
    assert_eq!(result(&mut live.fixture, &id)["result"], json!("picked"));
}

/// Upstream's `aHandbackIntoAnOccupiedWindowCancelsAndLeavesTheOtherModal`, the presenter lost.
#[test]
fn a_lost_presenter_hands_the_pick_back_or_cancels_it_when_the_slot_is_held() {
    let (mut live, serial, _) = presented_listing(&PICK_KINDS);
    let id = opened(&open_live(&mut live, |_| {}));
    live.fixture.core.presentation_closed(serial);
    live.fixture.core.commit();
    assert_eq!(slot_pick(&live.fixture, live.fixture.window), Some(id));

    let (mut live, serial, _) = presented_listing(&PICK_KINDS);
    let id = opened(&open_live(&mut live, |_| {}));
    let unrelated = opened(&open(&mut live.fixture, None, |_| {}));
    live.fixture.core.presentation_closed(serial);
    live.fixture.core.commit();
    assert_eq!(slot_pick(&live.fixture, live.fixture.window), Some(unrelated));
    assert_eq!(result(&mut live.fixture, &id), json!({"result": "cancelled"}));
}

/// A presenter's answer that arrives past the pick's deadline, before its expiry ran, resolves cancelled.
#[test]
fn a_presenter_answer_past_the_deadline_resolves_cancelled() {
    let (mut live, serial, presenter) = presented_listing(&PICK_KINDS);
    let id = opened(&open_live(&mut live, |a| a.timeout = Some(0.01)));
    let owner = request(&presenter)["owner"].as_i64().unwrap();
    std::thread::sleep(Duration::from_millis(20));
    answer(
        &mut live,
        serial,
        &presenter,
        &json!({"kind": "pick.resolve", "pick": {"id": id, "owner": owner, "item": "a"}}),
    );
    assert_eq!(result(&mut live.fixture, &id), json!({"result": "cancelled"}));
}

// MARK: - a replica on the viewer

fn pick_request(id: &str, owner: i64) -> Value {
    json!({"kind": "pick.request", "pick": {"id": id, "owner": owner,
        "items": [{"id": "a", "label": "Alpha"}, {"id": "b", "label": "Beta"}], "allowCustom": true}})
}

fn replica_viewer() -> super::viewer::Viewer {
    let mut viewer = super::viewer::viewer_built(false, |_| {});
    viewer.connect_listing(&PICK_KINDS, &json!({}));
    viewer
}

/// Upstream's `aReplicaAnswerGoesBackToTheOrigin`: a row, a custom answer and a cancel each answer the origin under
/// its owner.
#[test]
fn a_replica_answer_goes_back_to_the_origin() {
    let mut viewer = replica_viewer();
    let window = viewer.fixture.window;
    let hello = viewer.link().0.borrow().sent[0].clone();
    assert!(
        PICK_KINDS
            .iter()
            .all(|kind| hello["hello"]["kinds"].as_array().unwrap().contains(&json!(kind)))
    );

    viewer.frame(pick_request("K1", 4));
    assert_eq!(viewer.node()["pick"], json!({"id": "K1", "replica": true}));
    assert_eq!(slot_pick(&viewer.fixture, window), Some("K1".to_owned()));
    viewer.fixture.core.resolve_pick(window, "K1", picked_beta());
    viewer.fixture.core.commit();
    assert_eq!(
        viewer.sent("pick.resolve").last().unwrap()["pick"],
        json!({"id": "K1", "owner": 4, "item": "b"})
    );
    assert_eq!(viewer.node().get("pick"), None);

    viewer.frame(pick_request("K2", 4));
    let custom = crate::modal::PendingPick::custom("typed");
    viewer.fixture.core.resolve_pick(window, "K2", custom);
    viewer.fixture.core.commit();
    assert_eq!(
        viewer.sent("pick.resolve").last().unwrap()["pick"],
        json!({"id": "K2", "owner": 4, "query": "typed"})
    );

    viewer.frame(pick_request("K3", 4));
    let cancelled = ControlPickResult::outcome(ControlPickOutcome::Cancelled);
    viewer.fixture.core.resolve_pick(window, "K3", cancelled);
    viewer.fixture.core.commit();
    assert_eq!(
        viewer.sent("pick.resolve").last().unwrap()["pick"],
        json!({"id": "K3", "owner": 4})
    );
}

/// A held slot refuses the replica, so the origin takes it back; the origin's dismissal under its owner takes one down
/// without answering, and so does losing the stream.
#[test]
fn a_replica_is_refused_in_a_held_slot_and_dismissed_without_an_answer() {
    let mut viewer = replica_viewer();
    let window = viewer.fixture.window;
    viewer.frame(pick_request("K1", 4));
    viewer.frame(pick_request("K2", 4));
    assert_eq!(
        viewer.sent("pick.rejected").last().unwrap()["pick"],
        json!({"id": "K2", "owner": 4})
    );

    viewer.frame(json!({"kind": "pick.dismiss", "pick": {"id": "K1", "owner": 5}}));
    assert_eq!(
        slot_pick(&viewer.fixture, window),
        Some("K1".to_owned()),
        "another owner's dismissal"
    );
    viewer.frame(json!({"kind": "pick.dismiss", "pick": {"id": "K1", "owner": 4}}));
    assert_eq!(slot_pick(&viewer.fixture, window), None);
    assert_eq!(viewer.node().get("pick"), None);

    viewer.frame(pick_request("K3", 4));
    let key = viewer.link().0.borrow().key.unwrap();
    viewer.fixture.core.viewer_closed(key, "exit 255");
    viewer.fixture.core.commit();
    assert_eq!(slot_pick(&viewer.fixture, window), None);
    assert!(
        viewer.sent("pick.resolve").is_empty(),
        "a dismissed or dropped replica is never answered"
    );
}

/// Upstream's `openReplica` and `drain` on a headless origin's row: a pick handed over while the window is busy waits,
/// pending, behind it and shows once the slot frees, a GUI ask's as well as a pick's.
#[test]
fn a_headless_origins_replica_pick_queues_behind_a_busy_window() {
    let mut viewer = super::viewer::headless_viewer_built(false);
    viewer.connect_listing(&PICK_KINDS, &json!({}));
    let window = viewer.fixture.window;
    viewer.frame(pick_request("K1", 4));
    viewer.frame(pick_request("K2", 4));
    viewer.frame(pick_request("K3", 4));
    assert!(viewer.sent("pick.rejected").is_empty(), "a busy window queues them");
    assert_eq!(slot_pick(&viewer.fixture, window), Some("K1".to_owned()));
    assert_eq!(result(&mut viewer.fixture, "K2"), json!({"result": "pending"}));

    viewer.fixture.core.resolve_pick(window, "K1", picked_beta());
    viewer.fixture.core.commit();
    assert_eq!(
        slot_pick(&viewer.fixture, window),
        Some("K2".to_owned()),
        "the oldest goes next"
    );
    assert_eq!(viewer.node()["pick"], json!({"id": "K2", "replica": true}));

    // the queued one is cancelled here: the origin hears it, and its outcome stays readable
    assert!(cancel(&mut viewer.fixture, "K3").ok);
    assert_eq!(
        viewer.sent("pick.resolve").last().unwrap()["pick"],
        json!({"id": "K3", "owner": 4})
    );
    assert_eq!(result(&mut viewer.fixture, "K3"), json!({"result": "cancelled"}));

    // the origin's dismissal and the stream's loss take a queued one away unanswered
    viewer.frame(pick_request("K4", 4));
    viewer.frame(json!({"kind": "pick.dismiss", "pick": {"id": "K4", "owner": 4}}));
    viewer.frame(pick_request("K5", 4));
    let answered = viewer.sent("pick.resolve").len();
    let key = viewer.link().0.borrow().key.unwrap();
    viewer.fixture.core.viewer_closed(key, "exit 255");
    viewer.fixture.core.commit();
    assert_eq!(slot_pick(&viewer.fixture, window), None);
    assert_eq!(
        viewer.sent("pick.resolve").len(),
        answered,
        "nothing dropped is answered"
    );
    assert_eq!(viewer.node().get("pick"), None);
}

/// A window's GUI ask frees the slot for a headless origin's queued replica, and a closing window detaches what it
/// queued without answering.
#[test]
fn a_queued_replica_follows_a_window_ask_and_leaves_with_its_window() {
    let mut viewer = super::viewer::headless_viewer_built(false);
    viewer.connect_listing(&PICK_KINDS, &json!({}));
    let window = viewer.fixture.window;
    let ask = viewer.fixture.send(
        Command::Ask(AskCmd::Open),
        None,
        args(|a| {
            a.title = Some("t".to_owned());
            a.style = Some("gui".to_owned());
            a.buttons = Some(vec![AskButton {
                id: "ok".to_owned(),
                label: "OK".to_owned(),
                hotkey: None,
            }]);
        }),
    );
    assert!(ask.ok, "{ask:?}");
    viewer.frame(pick_request("K1", 4));
    assert!(viewer.sent("pick.rejected").is_empty());
    assert_eq!(slot_pick(&viewer.fixture, window), None);
    let cancelled = ngterm_proto::ControlAskResult::outcome(ngterm_proto::ControlAskOutcome::Cancelled);
    viewer.fixture.core.modals.resolve_window_ask(window, cancelled);
    assert_eq!(slot_pick(&viewer.fixture, window), Some("K1".to_owned()));

    viewer.frame(pick_request("K2", 4));
    viewer.fixture.core.modals.unregister(window);
    viewer.fixture.core.commit();
    assert!(
        viewer.sent("pick.resolve").is_empty(),
        "a closing window answers nothing"
    );
    assert_eq!(viewer.fixture.core.modals.session_picks.result("K2"), None);
}

/// A row from an origin with a screen keeps the refusal: a busy window hands the replica back.
#[test]
fn a_desktop_origins_replica_pick_is_refused_by_a_busy_window() {
    let mut viewer = replica_viewer();
    viewer.frame(pick_request("K1", 4));
    viewer.frame(pick_request("K2", 4));
    assert_eq!(
        viewer.sent("pick.rejected").last().unwrap()["pick"],
        json!({"id": "K2", "owner": 4})
    );
    assert_eq!(viewer.fixture.core.modals.session_picks.result("K2"), None);
}
