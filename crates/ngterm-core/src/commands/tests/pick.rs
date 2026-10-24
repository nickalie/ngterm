//! `pick.*` (upstream's `ControlDispatcherPickTests`, `ControlServerPickTests` and `PickTests`): full error strings,
//! the window slot, results by global id, retention and the races between an answer, a cancel and the next open.

use ngterm_proto::command::{AskCmd, PickCmd, QuickCmd};
use ngterm_proto::{Args, AskButton, Command, ControlPickOutcome, ControlPickResult, PickItem, Response};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{Fixture, UNKNOWN, args, failure, wire};
use crate::modal::PendingPick;

fn item(id: &str, label: &str, subtitle: Option<&str>) -> PickItem {
    PickItem {
        id: id.to_owned(),
        label: label.to_owned(),
        subtitle: subtitle.map(str::to_owned),
        ..PickItem::default()
    }
}

/// An item carrying the three status fields, each `None` left absent.
fn status_item(id: &str, status: Option<&str>, color: Option<&str>, shape: Option<&str>) -> PickItem {
    PickItem {
        status: status.map(str::to_owned),
        status_color: color.map(str::to_owned),
        status_shape: shape.map(str::to_owned),
        ..item(id, &id.to_uppercase(), None)
    }
}

fn items() -> Vec<PickItem> {
    vec![item("a", "Alpha", Some("first")), item("b", "Beta", None)]
}

fn open(fixture: &mut Fixture, edit: impl FnOnce(&mut Args)) -> Response {
    fixture.send(
        Command::Pick(PickCmd::Open),
        None,
        args(|a| {
            a.items = Some(items());
            edit(a);
        }),
    )
}

fn opened_id(response: &Response) -> String {
    assert!(response.ok, "{response:?}");
    response.result.as_ref().unwrap()["id"].as_str().unwrap().to_owned()
}

fn outcome(fixture: &mut Fixture, id: &str) -> Value {
    let response = fixture.send(Command::Pick(PickCmd::Result), Some(id), Args::default());
    assert!(response.ok, "{response:?}");
    response.result.unwrap()["pick"].clone()
}

fn cancel(fixture: &mut Fixture, id: &str) -> Response {
    fixture.send(Command::Pick(PickCmd::Cancel), Some(id), Args::default())
}

fn pick_pending(fixture: &Fixture) -> Option<String> {
    fixture.core.tree(None).unwrap().pick_pending
}

fn pending(fixture: &Fixture, window: Uuid) -> PendingPick {
    fixture.core.modal_slot(window).unwrap().pending().unwrap().clone()
}

#[test]
fn open_refusals_carry_upstreams_strings() {
    let mut fixture = Fixture::new(0);
    let many: Vec<PickItem> = (0..1001).map(|n| item(&n.to_string(), &n.to_string(), None)).collect();
    let cases: Vec<(Args, &str)> = vec![
        (Args::default(), "pick.open requires items"),
        (
            args(|a| a.items = Some(Vec::new())),
            "pick.open requires at least one item",
        ),
        (args(|a| a.items = Some(many)), "too many items (max 1000)"),
        (
            args(|a| a.items = Some(vec![item("a", "", None)])),
            "pick item label must not be empty",
        ),
        (
            args(|a| a.items = Some(vec![item("a", "A", None), item("a", "B", None)])),
            "pick item ids must be unique",
        ),
        (
            args(|a| a.items = Some(vec![item("a", "A\u{7}", None)])),
            "item text must not contain control characters",
        ),
        (
            args(|a| a.items = Some(vec![item("a", "A", Some("x\ny"))])),
            "item text must not contain control characters",
        ),
        (
            args(|a| {
                a.items = Some(items());
                a.selection = Some("c".into());
            }),
            "pick select must name an item id",
        ),
        (
            args(|a| {
                a.items = Some(Vec::new());
                a.allow_custom = Some(true);
                a.selection = Some("a".into());
            }),
            "pick select must name an item id",
        ),
    ];
    for (request, error) in cases {
        assert_eq!(
            fixture.send(Command::Pick(PickCmd::Open), None, request),
            failure(error)
        );
    }
    assert_eq!(pick_pending(&fixture), None);
    for cmd in [PickCmd::Result, PickCmd::Cancel] {
        let noun = if cmd == PickCmd::Result {
            "pick.result"
        } else {
            "pick.cancel"
        };
        assert_eq!(
            fixture.send(Command::Pick(cmd), None, Args::default()),
            failure(&format!("{noun} requires a pick id"))
        );
        assert_eq!(
            fixture.send(Command::Pick(cmd), Some(UNKNOWN), Args::default()),
            failure(&format!("unknown pick: {UNKNOWN}"))
        );
    }
}

const BAD_STATUS: &str = "invalid pick item status (idle|active|completed|blocked)";
const BAD_COLOR: &str = "invalid pick item color (expected #rrggbb)";
const BAD_SHAPE: &str = "invalid pick item shape (circle|square|triangle|diamond|capsule|star)";

/// Upstream's `openRejectsAnInvalidItemStatusWithoutCallingHost`: a bad status, color or shape is refused with a
/// message that never echoes the value, and no picker opens.
#[test]
fn an_invalid_item_status_is_refused_without_opening_a_picker() {
    let mut fixture = Fixture::new(0);
    let cases = [
        (status_item("a", Some("busy"), None, None), BAD_STATUS),
        (status_item("a", Some("blocked"), Some("red"), None), BAD_COLOR),
        (status_item("a", Some("active"), None, Some("hexagon")), BAD_SHAPE),
        (status_item("a", None, Some("#12345"), None), BAD_COLOR),
        (status_item("a", None, None, Some("")), BAD_SHAPE),
    ];
    for (bad, error) in cases {
        let response = fixture.send(Command::Pick(PickCmd::Open), None, args(|a| a.items = Some(vec![bad])));
        assert_eq!(response, failure(error));
        assert_eq!(pick_pending(&fixture), None);
    }
}

/// The status check runs after the control-character check and before the selection check; within it the first
/// item with an error wins, and an item's status is checked before its color and its color before its shape.
#[test]
fn the_item_status_check_keeps_upstreams_order() {
    let mut fixture = Fixture::new(0);
    let control = PickItem {
        label: "A\u{7}".to_owned(),
        ..status_item("a", Some("busy"), None, None)
    };
    let cases: Vec<(Args, &str)> = vec![
        (
            args(|a| a.items = Some(vec![control])),
            "item text must not contain control characters",
        ),
        (
            args(|a| {
                a.items = Some(vec![status_item("a", Some("busy"), None, None)]);
                a.selection = Some("zzz".into());
            }),
            BAD_STATUS,
        ),
        (
            args(|a| {
                a.items = Some(vec![
                    status_item("a", None, None, Some("hexagon")),
                    status_item("b", Some("busy"), None, None),
                ]);
            }),
            BAD_SHAPE,
        ),
        (
            args(|a| a.items = Some(vec![status_item("a", Some("busy"), Some("red"), Some("hexagon"))])),
            BAD_STATUS,
        ),
        (
            args(|a| a.items = Some(vec![status_item("a", Some("idle"), Some("red"), Some("hexagon"))])),
            BAD_COLOR,
        ),
    ];
    for (request, error) in cases {
        assert_eq!(
            fixture.send(Command::Pick(PickCmd::Open), None, request),
            failure(error)
        );
    }
}

/// Upstream's `openPassesItemStatusThrough`: valid status fields reach the picker as sent, `idle` included.
#[test]
fn item_status_fields_reach_the_picker() {
    let mut fixture = Fixture::new(0);
    let sent = vec![
        status_item("one", Some("completed"), Some("#00aa00"), Some("star")),
        status_item("two", Some("idle"), None, None),
    ];
    let request = args(|a| a.items = Some(sent.clone()));
    opened_id(&fixture.send(Command::Pick(PickCmd::Open), None, request));
    assert_eq!(pending(&fixture, fixture.window).items, sent);
}

#[test]
fn a_pick_waits_in_the_window_slot_until_cancelled() {
    let mut fixture = Fixture::new(0);
    let id = opened_id(&open(&mut fixture, |a| {
        a.prompt = Some("Branch".into());
        a.query = Some(String::new());
        a.selection = Some("b".into());
    }));
    assert_eq!(pick_pending(&fixture), Some(id.clone()));
    let held = pending(&fixture, fixture.window);
    assert_eq!(
        (held.prompt.as_deref(), held.selection.as_deref()),
        (Some("Branch"), Some("b"))
    );
    assert_eq!(outcome(&mut fixture, &id), json!({"result": "pending"}));
    assert_eq!(open(&mut fixture, |_| {}), failure("pick already pending"));
    let ask = fixture.send(
        Command::Ask(AskCmd::Open),
        None,
        args(|a| {
            a.title = Some("Proceed?".into());
            a.style = Some("gui".into());
            a.buttons = Some(vec![AskButton {
                id: "ok".into(),
                label: "OK".into(),
                hotkey: None,
            }]);
        }),
    );
    assert_eq!(ask, failure("pick already pending"));

    assert!(cancel(&mut fixture, &id).ok);
    assert_eq!(pick_pending(&fixture), None);
    assert_eq!(outcome(&mut fixture, &id), json!({"result": "cancelled"}));
    assert!(cancel(&mut fixture, &id).ok, "a finished pick cancels as a no-op");
    assert_eq!(outcome(&mut fixture, &id), json!({"result": "cancelled"}));
}

#[test]
fn an_empty_list_is_a_text_prompt_answered_with_custom_text() {
    let mut fixture = Fixture::new(0);
    let window = fixture.window;
    let id = opened_id(&fixture.send(
        Command::Pick(PickCmd::Open),
        None,
        args(|a| {
            a.items = Some(Vec::new());
            a.allow_custom = Some(true);
        }),
    ));
    fixture.core.resolve_pick(window, &id, PendingPick::custom("feature/x"));
    assert_eq!(
        outcome(&mut fixture, &id),
        json!({"result": "custom", "query": "feature/x"})
    );
}

#[test]
fn a_picked_row_reports_its_caller_index_and_a_stale_answer_is_ignored() {
    let mut fixture = Fixture::new(0);
    let window = fixture.window;
    let id = opened_id(&open(&mut fixture, |_| {}));
    let held = pending(&fixture, window);
    fixture.core.resolve_pick(window, "stale", held.picked(0).unwrap());
    assert_eq!(outcome(&mut fixture, &id), json!({"result": "pending"}));
    fixture.core.resolve_pick(window, &id, held.picked(1).unwrap());
    assert_eq!(
        outcome(&mut fixture, &id),
        json!({"result": "picked", "id": "b", "label": "Beta", "index": 1})
    );
    // a cancel racing the answer changes nothing, and the next open keeps the answer readable
    assert!(cancel(&mut fixture, &id).ok);
    let next = opened_id(&open(&mut fixture, |_| {}));
    assert_eq!(outcome(&mut fixture, &id)["result"], "picked");
    assert_eq!(outcome(&mut fixture, &next), json!({"result": "pending"}));
}

#[test]
fn results_follow_the_pick_id_across_windows_and_outlive_their_window() {
    let mut fixture = Fixture::new(0);
    let window = fixture.window;
    let id = opened_id(&open(&mut fixture, |_| {}));
    let other = fixture.core.library_mut().new_window(None);
    fixture.core.report_frontmost(other);
    assert_eq!(
        outcome(&mut fixture, &id),
        json!({"result": "pending"}),
        "found by id, not the frontmost"
    );
    let scoped = |fixture: &mut Fixture, cmd: PickCmd, window: Uuid| {
        fixture.send(Command::Pick(cmd), Some(&id), args(|a| a.window = Some(wire(window))))
    };
    assert_eq!(
        scoped(&mut fixture, PickCmd::Result, other),
        failure(&format!("unknown pick: {id}"))
    );
    assert_eq!(
        scoped(&mut fixture, PickCmd::Cancel, other),
        failure(&format!("unknown pick: {id}"))
    );
    assert!(scoped(&mut fixture, PickCmd::Result, window).ok);

    fixture.core.close_window(window);
    fixture.core.commit();
    assert_eq!(
        outcome(&mut fixture, &id),
        json!({"result": "cancelled"}),
        "closing the window cancels"
    );
    assert!(scoped(&mut fixture, PickCmd::Result, window).ok);
    assert_eq!(
        scoped(&mut fixture, PickCmd::Result, other),
        failure(&format!("unknown pick: {id}"))
    );
    assert!(cancel(&mut fixture, &id).ok);
}

#[test]
fn a_pick_hides_the_quick_terminal_pauses_auto_follow_and_refuses_its_show() {
    let mut fixture = Fixture::new(0);
    let window = fixture.window;
    let show = |fixture: &mut Fixture| {
        fixture.send(
            Command::Quick(QuickCmd::Visibility),
            None,
            args(|a| a.mode = Some("show".into())),
        )
    };
    assert!(show(&mut fixture).ok);
    let id = opened_id(&open(&mut fixture, |_| {}));
    assert!(!fixture.core.tree(None).unwrap().quick_visible.unwrap_or(false));
    assert_eq!(show(&mut fixture), failure("pick pending"));
    fixture.core.toggle_quick_terminal();
    assert!(
        !fixture.core.tree(None).unwrap().quick_visible.unwrap_or(false),
        "the hotkey stands down too"
    );
    assert!(fixture.core.modal_pending(window));
    assert_eq!(fixture.core.modals.suppressing, vec![window]);

    fixture
        .core
        .resolve_pick(window, &id, ControlPickResult::outcome(ControlPickOutcome::Cancelled));
    assert!(fixture.core.modals.suppressing.is_empty());
    assert!(show(&mut fixture).ok);
}

#[test]
fn a_window_keeps_eight_results_readable_after_it_closes() {
    let mut fixture = Fixture::new(0);
    let window = fixture.window;
    let ids: Vec<String> = (0..9)
        .map(|_| {
            let id = opened_id(&open(&mut fixture, |_| {}));
            assert!(cancel(&mut fixture, &id).ok);
            id
        })
        .collect();
    assert_eq!(
        fixture.send(Command::Pick(PickCmd::Result), Some(&ids[0]), Args::default()),
        failure(&format!("unknown pick: {}", ids[0]))
    );
    assert_eq!(outcome(&mut fixture, &ids[1])["result"], "cancelled");
    fixture.core.close_window(window);
    fixture.core.commit();
    assert_eq!(outcome(&mut fixture, &ids[8])["result"], "cancelled");
}

#[test]
fn no_open_window_refuses_the_picker() {
    let mut fixture = Fixture::new(0);
    let window = fixture.window;
    fixture.core.close_window(window);
    fixture.core.commit();
    assert_eq!(open(&mut fixture, |_| {}), failure("no open window"));
}
