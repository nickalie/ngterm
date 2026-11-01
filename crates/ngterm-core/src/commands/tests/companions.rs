//! `session.companion.new`, `session.collapse`/`session.expand`, the companion refusals of `session.move`, a group's
//! `session.close` receipts and the orphan refusal of `recent.open` (upstream's `ControlDispatcherCompanionsTests`
//! new/collapse/expand cases and `ControlServerCompanionsTests` create, move-refusal, fold, close, receipt and reopen
//! cases).

use ngterm_proto::command::{RecentCmd, SessionCmd};
use ngterm_proto::{Args, Command, Response};
use serde_json::json;
use uuid::Uuid;

use super::{Fixture, UNKNOWN, answered_affected, answered_id, args, failure, wire};
use crate::model::{NewSession, SaveRequest, Session, SidebarTint, TintColor};
use crate::persist::{AppSettings, RecentClosedItem, RecentClosedKind, RecentClosedSession, SessionSnapshot};

const PLACEMENT: &str = "a companion takes no workspace or placement: it lands under its parent";
const MOVE_REFUSED: &str = "a companion moves only among its siblings";

fn new_companion(fixture: &mut Fixture, target: Uuid, args: Args) -> Response {
    fixture.session(SessionCmd::CompanionNew, Some(&wire(target)), args)
}

/// A fixture whose first workspace holds `parent` with two companions, then `after`.
fn group(fixture: &mut Fixture) -> (Uuid, [Uuid; 2], Uuid) {
    let (parent, after) = (fixture.sessions[0], fixture.sessions[1]);
    let children = [(); 2].map(|()| {
        let response = new_companion(fixture, parent, args(|a| a.no_select = Some(true)));
        answered_id(&response).expect("a companion")
    });
    (parent, children, after)
}

#[test]
fn a_companion_is_created_under_its_parent_with_its_options_and_answers_with_the_parent() {
    let mut fixture = Fixture::new(1);
    let (parent, after) = (fixture.sessions[0], fixture.sessions[1]);
    let response = new_companion(
        &mut fixture,
        parent,
        args(|a| {
            a.cwd = Some("/tmp".into());
            a.name = Some("logs".into());
            a.command = Some("tail -f log".into());
            a.wait = Some(true);
            a.no_select = Some(true);
            a.tint = Some("red".into());
        }),
    );
    let id = answered_id(&response).expect("an id");
    assert_eq!(response.result.as_ref().unwrap()["parent"], json!(wire(parent)));
    let companion = fixture.store().session(id).unwrap();
    assert_eq!(companion.parent_id(), Some(parent));
    assert_eq!(companion.custom_name.as_deref(), Some("logs"));
    assert_eq!(companion.initial_cwd(), "/tmp");
    assert_eq!(companion.launch.command.as_deref(), Some("tail -f log"));
    assert!(companion.launch.wait);
    assert_eq!(companion.tint, Some(SidebarTint::on_icon(TintColor::Red)));
    assert_eq!(fixture.selected(), Some(after));
    assert_eq!(fixture.order(fixture.workspace), [parent, id, after]);
}

#[test]
fn a_companion_target_means_its_parent_and_the_parents_cwd_is_the_default() {
    let mut fixture = Fixture::new(0);
    let parent = fixture.sessions[0];
    let first = answered_id(&new_companion(&mut fixture, parent, Args::default())).unwrap();
    let sibling = answered_id(&new_companion(&mut fixture, first, Args::default())).unwrap();
    let store = fixture.store();
    assert_eq!(store.session(sibling).unwrap().parent_id(), Some(parent));
    assert_eq!(
        store.session(first).unwrap().initial_cwd(),
        store.session(parent).unwrap().effective_cwd()
    );
    // a foreground create takes the selection
    assert_eq!(fixture.selected(), Some(sibling));
}

#[test]
fn companion_new_refuses_a_workspace_or_a_placement_before_anything_else() {
    let mut fixture = Fixture::new(0);
    let parent = fixture.sessions[0];
    let forms: [fn(&mut Args); 5] = [
        |a| a.workspace = Some("w".into()),
        |a| a.workspace_name = Some("w".into()),
        |a| a.create_workspace = Some(true),
        |a| a.after = Some("a".into()),
        |a| a.before = Some("b".into()),
    ];
    for form in forms {
        // the placement refusal wins over a bad tint and an unknown target
        let refused = fixture.session(
            SessionCmd::CompanionNew,
            Some(UNKNOWN),
            args(|a| {
                form(a);
                a.wait = Some(true);
                a.tint = Some("mauve".into());
            }),
        );
        assert_eq!(refused, failure(PLACEMENT));
    }
    assert_eq!(fixture.order(fixture.workspace), [parent]);
}

#[test]
fn companion_new_refuses_wait_without_a_command_and_a_bad_tint_before_resolving() {
    let mut fixture = Fixture::new(0);
    let wait = fixture.session(SessionCmd::CompanionNew, Some(UNKNOWN), args(|a| a.wait = Some(true)));
    assert_eq!(wait, failure("--wait requires --command"));
    let tint = fixture.session(
        SessionCmd::CompanionNew,
        Some(UNKNOWN),
        args(|a| a.tint = Some("mauve".into())),
    );
    assert!(!tint.ok);
    assert!(
        tint.error.as_deref().unwrap().starts_with("invalid tint: mauve"),
        "{tint:?}"
    );
    let unknown = fixture.session(SessionCmd::CompanionNew, Some(UNKNOWN), Args::default());
    assert!(!unknown.ok);
    assert_eq!(fixture.order(fixture.workspace).len(), 1);
}

/// A remote row's companion is its origin's to create, which a row with no binding to it cannot reach (upstream's
/// `createRemoteCompanion` guard); the cases with an origin are `remote_companions.rs`'s.
#[test]
fn an_unattached_remote_row_has_no_origin_to_create_a_companion() {
    let mut fixture = Fixture::new(0);
    let workspace = fixture.workspace;
    let remote = fixture
        .store_mut()
        .and_then(|store| {
            store.add_session(
                workspace,
                NewSession {
                    remote_host: Some("far".to_owned()),
                    ..NewSession::at_cwd("/r")
                },
            )
        })
        .unwrap();
    assert_eq!(
        new_companion(&mut fixture, remote, Args::default()),
        failure("the remote session is not attached")
    );
    assert!(fixture.store().companions(remote).is_empty());
}

#[test]
fn folding_a_parent_saves_and_a_lone_session_is_refused() {
    let mut fixture = Fixture::new(1);
    let (parent, _, after) = group(&mut fixture);
    fixture.store_mut().unwrap().take_outbox();
    let folded = fixture.session(SessionCmd::Collapse, Some(&wire(parent)), Args::default());
    assert_eq!(answered_id(&folded), Some(parent));
    assert!(!fixture.store().session(parent).unwrap().companion.expanded);
    assert_eq!(fixture.store().outbox().save, SaveRequest::Now);
    // an unchanged value still answers ok, without a save, and still asks the outline to fold the row (a reveal may
    // have opened it)
    fixture.store_mut().unwrap().take_outbox();
    fixture.store_mut().unwrap().take_sidebar_expansion_requests();
    let again = fixture.session(SessionCmd::Collapse, Some(&wire(parent)), Args::default());
    assert_eq!(answered_id(&again), Some(parent));
    assert_eq!(fixture.store().outbox().save, SaveRequest::None);
    let requests = fixture.store_mut().unwrap().take_sidebar_expansion_requests();
    assert_eq!(requests.into_iter().collect::<Vec<_>>(), [(parent, false)]);
    let unfolded = fixture.session(SessionCmd::Expand, Some(&wire(parent)), Args::default());
    assert_eq!(answered_id(&unfolded), Some(parent));
    assert!(fixture.store().session(parent).unwrap().companion.expanded);
    fixture.store_mut().unwrap().take_sidebar_expansion_requests();
    for cmd in [SessionCmd::Collapse, SessionCmd::Expand] {
        assert_eq!(
            fixture.session(cmd, Some(&wire(after)), Args::default()),
            failure("session has no companions")
        );
    }
    assert!(
        fixture
            .store_mut()
            .unwrap()
            .take_sidebar_expansion_requests()
            .is_empty(),
        "a refusal asks nothing of the outline"
    );
}

#[test]
fn the_tree_reads_back_the_parent_and_the_fold() {
    let mut fixture = Fixture::new(1);
    let (parent, [first, _], _) = group(&mut fixture);
    fixture.session(SessionCmd::Collapse, Some(&wire(parent)), Args::default());
    let tree = fixture.send(ngterm_proto::Command::Tree, None, Args::default());
    let sessions = tree.result.unwrap()["tree"]["workspaces"][0]["sessions"].clone();
    let node = |id: Uuid| {
        sessions
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["id"] == json!(wire(id)))
            .cloned()
            .unwrap()
    };
    assert_eq!(node(first)["parent"], json!(wire(parent)));
    assert_eq!(node(parent)["collapsed"], json!(true));
    assert!(node(parent).get("parent").is_none());
    assert!(node(first).get("collapsed").is_none());
}

#[test]
fn moving_a_companion_out_of_its_group_is_refused_and_an_ordinary_no_op_is_not() {
    let mut fixture = Fixture::new(1);
    let (parent, children, after) = group(&mut fixture);
    let (other, _) = fixture.add_workspace("other", 0);
    let home = fixture.workspace;
    let to_other = |a: &mut Args| a.workspace = Some(wire(other));

    let refused = fixture.session(SessionCmd::Move, Some(&wire(children[0])), args(to_other));
    assert_eq!(refused, failure(MOVE_REFUSED));
    let batch = fixture.session(
        SessionCmd::Move,
        Some(&wire(children[0])),
        args(|a| {
            to_other(a);
            a.targets = Some(children.iter().copied().map(wire).collect());
        }),
    );
    assert_eq!(batch, failure(MOVE_REFUSED));
    // a batch already in its workspace moves nothing, which is no refusal
    let no_op = fixture.session(
        SessionCmd::Move,
        Some(&wire(parent)),
        args(|a| {
            a.workspace = Some(wire(home));
            a.targets = Some(vec![wire(parent), wire(after)]);
        }),
    );
    assert_eq!(answered_affected(&no_op), Some(0));
    // next to an anchor outside the group
    let placed = fixture.session(
        SessionCmd::Move,
        Some(&wire(children[1])),
        args(|a| a.after = Some(wire(after))),
    );
    assert_eq!(placed, failure(MOVE_REFUSED));
    let placed_batch = fixture.session(
        SessionCmd::Move,
        Some(&wire(children[0])),
        args(|a| {
            a.before = Some(wire(parent));
            a.targets = Some(children.iter().copied().map(wire).collect());
        }),
    );
    assert_eq!(placed_batch, failure(MOVE_REFUSED));
    assert_eq!(fixture.order(home), [parent, children[0], children[1], after]);
}

#[test]
fn a_companion_moves_among_its_siblings_and_a_parent_carries_its_group() {
    let mut fixture = Fixture::new(1);
    let (parent, [first, second], after) = group(&mut fixture);
    let (other, _) = fixture.add_workspace("other", 0);
    let up = fixture.session(
        SessionCmd::Move,
        Some(&wire(second)),
        args(|a| a.to = Some("up".into())),
    );
    assert_eq!(answered_id(&up), Some(second));
    assert_eq!(fixture.order(fixture.workspace), [parent, second, first, after]);
    let before = fixture.session(
        SessionCmd::Move,
        Some(&wire(first)),
        args(|a| a.before = Some(wire(second))),
    );
    assert_eq!(answered_id(&before), Some(first));
    assert_eq!(fixture.order(fixture.workspace), [parent, first, second, after]);
    let moved = fixture.session(
        SessionCmd::Move,
        Some(&wire(parent)),
        args(|a| a.workspace = Some(wire(other))),
    );
    assert_eq!(answered_id(&moved), Some(parent));
    assert_eq!(fixture.order(fixture.workspace), [after]);
    assert_eq!(fixture.order(other), [parent, first, second]);
}

// MARK: - Close and reopen (`ControlServerCompanionsTests` close, receipt and reopen cases)

fn grace(enabled: bool) -> AppSettings {
    AppSettings {
        close_grace_undo_enabled: Some(enabled),
        ..AppSettings::default()
    }
}

fn close_targets(ids: &[Uuid]) -> Args {
    args(|a| a.targets = Some(wires(ids)))
}

fn wires(ids: &[Uuid]) -> Vec<String> {
    ids.iter().map(|id| wire(*id)).collect()
}

fn closed(response: &Response) -> Vec<String> {
    assert!(response.ok, "{response:?}");
    serde_json::from_value(response.result.as_ref().unwrap()["closed"].clone()).unwrap()
}

#[test]
fn closing_a_parent_reports_every_closed_id_and_the_group_count() {
    let mut fixture = Fixture::with_settings(1, grace(false));
    let (parent, [first, second], _) = group(&mut fixture);

    let response = fixture.session(SessionCmd::Close, Some(&wire(parent)), Args::default());

    assert_eq!(answered_id(&response), Some(parent));
    assert_eq!(answered_affected(&response), Some(3));
    assert_eq!(closed(&response), wires(&[parent, first, second]));
    assert!(fixture.store().session(first).is_none());
}

#[test]
fn an_undoable_close_of_a_parent_counts_its_group_and_one_undo_brings_it_back() {
    let mut fixture = Fixture::new(1);
    let (parent, [first, second], after) = group(&mut fixture);

    let response = fixture.session(
        SessionCmd::Close,
        Some(&wire(parent)),
        args(|a| a.undoable = Some(true)),
    );

    let receipt =
        json!({"id": wire(parent), "affected": 3, "undoable": true, "closed": wires(&[parent, first, second])});
    assert_eq!(
        serde_json::to_value(&response).unwrap(),
        json!({"ok": true, "result": receipt})
    );
    assert_eq!(fixture.order(fixture.workspace), [after]);
    assert!(fixture.store_mut().unwrap().undo_pending_close(None));
    assert_eq!(fixture.order(fixture.workspace), [parent, first, second, after]);
}

#[test]
fn a_plain_close_keeps_its_bare_receipt() {
    let mut fixture = Fixture::new(1);
    let (_, [first, _], after) = group(&mut fixture);
    for lone in [after, first] {
        let response = fixture.session(SessionCmd::Close, Some(&wire(lone)), Args::default());
        assert_eq!(answered_affected(&response), None);
        assert_eq!(closed(&response), [wire(lone)]);
    }
}

#[test]
fn a_batch_naming_a_parent_and_its_child_counts_each_once_hard_and_soft() {
    for soft in [false, true] {
        let mut fixture = Fixture::with_settings(1, grace(soft));
        let (parent, [first, second], other) = group(&mut fixture);

        let response = fixture.session(SessionCmd::Close, None, close_targets(&[parent, first, other]));

        assert_eq!(answered_affected(&response), Some(4), "soft: {soft}");
        assert_eq!(
            closed(&response),
            wires(&[parent, first, second, other]),
            "soft: {soft}"
        );
        assert!(fixture.order(fixture.workspace).is_empty(), "soft: {soft}");
    }
}

// a child named before its parent was archived on its own, so reopening the parent lost the child
#[test]
fn a_hard_batch_naming_the_child_first_keeps_the_group_in_one_recent_entry() {
    let mut fixture = Fixture::with_settings(1, grace(false));
    let (parent, children, _) = group(&mut fixture);

    let response = fixture.session(SessionCmd::Close, None, close_targets(&[children[1], parent]));

    assert_eq!(closed(&response), wires(&[parent, children[0], children[1]]));
    let items = fixture.core.library().recent_closed_items();
    let entry_of = |id: Uuid| {
        items
            .iter()
            .find(|item| item.session.as_ref().is_some_and(|recent| recent.snapshot.id == id))
            .map(|item| item.id)
    };
    assert_eq!(entry_of(children[1]), None, "the child rides the parent's entry");
    let entry = entry_of(parent).expect("the parent's entry");
    assert!(fixture.core.library_mut().reopen_recent_closed(entry, None));
    let companions: Vec<Uuid> = fixture
        .store()
        .companions(parent)
        .into_iter()
        .map(Session::id)
        .collect();
    assert_eq!(companions, children);
}

#[test]
fn reopening_a_companion_whose_parent_is_gone_names_that_reason() {
    let mut fixture = Fixture::new(0);
    let entry = Uuid::new_v4();
    let snapshot = SessionSnapshot {
        custom_name: Some("logs".to_owned()),
        parent_id: Some(Uuid::new_v4()),
        ..SessionSnapshot::new(Uuid::new_v4(), "/tmp")
    };
    let item = RecentClosedItem {
        id: entry,
        kind: RecentClosedKind::Session,
        title: "logs".to_owned(),
        subtitle: None,
        closed_at: std::time::SystemTime::now(),
        session: Some(RecentClosedSession {
            workspace_id: Uuid::new_v4(),
            workspace_name: "gone".to_owned(),
            workspace_index: 0,
            session_index: 0,
            snapshot,
            companions: None,
        }),
        workspace: None,
    };
    fixture.core.library_mut().set_recent_closed_items(vec![item]);

    let response = fixture.send(Command::Recent(RecentCmd::Open), Some(&wire(entry)), Args::default());

    assert_eq!(response, failure("parent session is not open or in Recent Closed"));
}
