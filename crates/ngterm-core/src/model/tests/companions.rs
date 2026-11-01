//! Companion sessions, ported from upstream's `AppStoreCompanionsTests` (placement, fold, move offers, attention and
//! regrouping), the companion cases of `AppStoreOrganizationTests`, `AppStoreTreeProjectionTests` and
//! `SnapshotRoundTripTests`, and `AppStoreEventTests.aCompanionsCreatedEventNamesItsParent`. The close, undo,
//! respawn, page and duplicate cases come with their tasks.

use ngterm_proto::events::EventKind;
use uuid::Uuid;

use super::support::{add, ids};
use crate::model::{
    AgentIndicator, AgentStatus, AppStore, CompanionOptions, CompanionRefusal, NewSession, ReorderDirection, Session,
    SidebarTint, TintColor,
};
use crate::pane_env::uuid_string;
use crate::persist::{SessionSnapshot, Snapshot, WorkspaceSnapshot};
use crate::sidebar_drop::{DropIndex, SessionDropTarget, SessionSource, flagged_tree_child_index, resolve_sessions};

const HOME: &str = "/home/u";

/// Upstream's `CompanionGroup`: `work` holds `parent` with its companions `first` and `second`, then `after`;
/// `personal` is empty.
struct Group {
    store: AppStore,
    work: Uuid,
    personal: Uuid,
    parent: Uuid,
    first: Uuid,
    second: Uuid,
    after: Uuid,
}

impl Group {
    fn new() -> Self {
        let mut store = AppStore::new();
        let work = store.add_workspace("work");
        let personal = store.add_workspace("personal");
        let parent = add(&mut store, work, "/parent");
        let after = add(&mut store, work, "/after");
        let first = companion(&mut store, parent, cwd("/first"));
        let second = companion(&mut store, parent, cwd("/second"));
        Self {
            store,
            work,
            personal,
            parent,
            first,
            second,
            after,
        }
    }

    /// The order of `work`.
    fn order(&self) -> Vec<Uuid> {
        ids(&self.store, self.work)
    }

    fn session(&self, id: Uuid) -> &Session {
        self.store.session(id).expect("the session exists")
    }
}

fn cwd(path: &str) -> CompanionOptions {
    CompanionOptions {
        cwd: Some(path.to_owned()),
        ..CompanionOptions::default()
    }
}

fn companion(store: &mut AppStore, target: Uuid, options: CompanionOptions) -> Uuid {
    store
        .add_companion(target, options, HOME)
        .expect("the companion is created")
}

// MARK: - Placement

#[test]
fn a_companion_lands_after_the_parents_last_companion() {
    let g = Group::new();
    assert_eq!(g.order(), [g.parent, g.first, g.second, g.after]);
    assert_eq!(g.session(g.first).parent_id(), Some(g.parent));
    assert_eq!(g.session(g.second).parent_id(), Some(g.parent));
    let companions: Vec<Uuid> = g.store.companions(g.parent).into_iter().map(Session::id).collect();
    assert_eq!(companions, [g.first, g.second]);
    assert_eq!(g.store.parent_of(g.session(g.first)).map(Session::id), Some(g.parent));
    assert!(g.store.parent_of(g.session(g.parent)).is_none());
}

#[test]
fn a_companion_takes_its_creation_tint() {
    let mut g = Group::new();
    let tint = SidebarTint::on_icon(TintColor::Blue);
    let tinted = companion(
        &mut g.store,
        g.parent,
        CompanionOptions {
            tint: Some(tint),
            ..CompanionOptions::default()
        },
    );
    assert_eq!(g.session(tinted).tint, Some(tint));
}

#[test]
fn a_companion_of_a_companion_becomes_a_sibling() {
    let mut g = Group::new();
    let third = companion(&mut g.store, g.first, CompanionOptions::default());
    assert_eq!(g.session(third).parent_id(), Some(g.parent));
    assert_eq!(g.order(), [g.parent, g.first, g.second, third, g.after]);
}

#[test]
fn a_companion_starts_in_the_parents_cwd_unless_given_one() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let parent = add(&mut store, work, "/parent/dir");
    let options = CompanionOptions {
        command: Some("lazygit".to_owned()),
        name: Some("git".to_owned()),
        wait: true,
        ..CompanionOptions::default()
    };
    let id = companion(&mut store, parent, options);
    let created = store.session(id).unwrap();
    assert_eq!(created.initial_cwd(), "/parent/dir");
    assert_eq!(created.launch.command.as_deref(), Some("lazygit"));
    assert_eq!(created.custom_name.as_deref(), Some("git"));
    assert!(created.launch.wait);
    assert_eq!(store.selected_session_id(), Some(id));
}

#[test]
fn a_background_companion_keeps_the_selection() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let parent = add(&mut store, work, "/parent");
    let options = CompanionOptions {
        select: false,
        ..CompanionOptions::default()
    };
    companion(&mut store, parent, options);
    assert_eq!(store.selected_session_id(), Some(parent));
}

#[test]
fn a_remote_parent_is_refused() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let remote = store
        .add_session(
            work,
            NewSession {
                remote_host: Some("buildbox".to_owned()),
                ..NewSession::at_cwd("/r")
            },
        )
        .unwrap();
    assert_eq!(
        store.add_companion(remote, CompanionOptions::default(), HOME),
        Err(CompanionRefusal::RemoteParent)
    );
    assert_eq!(
        store.add_companion(Uuid::new_v4(), CompanionOptions::default(), HOME),
        Err(CompanionRefusal::NoSuchSession)
    );
    assert_eq!(store.workspaces()[0].sessions.len(), 1);
}

#[test]
fn a_top_level_session_inserted_inside_a_group_lands_after_it() {
    for index in [1, 2, 3] {
        let mut g = Group::new();
        let spec = NewSession {
            at: Some(index),
            ..NewSession::at_cwd("/new")
        };
        let inserted = g.store.add_session(g.work, spec).unwrap();
        assert_eq!(
            g.order(),
            [g.parent, g.first, g.second, inserted, g.after],
            "at {index}"
        );
    }
}

#[test]
fn insertion_before_the_parent_stays_before_the_group() {
    let mut g = Group::new();
    let spec = NewSession {
        at: Some(0),
        ..NewSession::at_cwd("/new")
    };
    let inserted = g.store.add_session(g.work, spec).unwrap();
    assert_eq!(g.order(), [inserted, g.parent, g.first, g.second, g.after]);
}

#[test]
fn plain_duplicate_of_a_parent_lands_after_its_group() {
    let mut g = Group::new();
    let duplicate = g.store.duplicate_session(g.parent, HOME).unwrap();
    assert_eq!(g.session(duplicate).parent_id(), None);
    assert_eq!(g.order(), [g.parent, g.first, g.second, duplicate, g.after]);
}

#[test]
fn a_companions_created_event_names_its_parent() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let parent = add(&mut store, work, "/tmp");
    // a top-level session's event names no parent
    assert_eq!(created_parents(&mut store), [None]);
    let id = companion(&mut store, parent, CompanionOptions::default());
    let events = store.take_outbox().events;
    let created: Vec<_> = events
        .iter()
        .filter(|event| event.kind == EventKind::SessionCreated)
        .collect();
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].session, Some(id));
    assert_eq!(created[0].payload.parent, Some(uuid_string(parent)));
}

fn created_parents(store: &mut AppStore) -> Vec<Option<String>> {
    store
        .take_outbox()
        .events
        .into_iter()
        .filter(|event| event.kind == EventKind::SessionCreated)
        .map(|event| event.payload.parent)
        .collect()
}

// MARK: - Fold and attention

#[test]
fn only_a_parent_with_companions_folds() {
    let mut g = Group::new();
    g.store.take_outbox();
    assert!(g.store.set_session_expanded(g.parent, false));
    assert!(!g.session(g.parent).companion.expanded);
    assert!(
        g.store
            .take_outbox()
            .events
            .iter()
            .any(|event| event.kind == EventKind::TreeChanged)
    );
    // an unchanged value answers true and writes nothing
    assert!(g.store.set_session_expanded(g.parent, false));
    assert!(g.store.take_outbox().is_empty());
    assert!(!g.store.set_session_expanded(g.after, false));
    assert!(g.session(g.after).companion.expanded);
    assert!(!g.store.set_session_expanded(g.first, false));
    assert!(g.session(g.first).companion.expanded);
}

#[test]
fn blocked_completed_and_unread_companions_need_attention() {
    let mut g = Group::new();
    let third = companion(&mut g.store, g.parent, CompanionOptions::default());
    let fourth = companion(&mut g.store, g.parent, CompanionOptions::default());
    let status = |status| AgentIndicator {
        status,
        ..AgentIndicator::default()
    };
    g.store.set_agent_indicator(status(AgentStatus::Blocked), g.first);
    g.store.set_agent_indicator(status(AgentStatus::Active), g.second);
    g.store.set_agent_indicator(status(AgentStatus::Completed), third);
    g.store.session_mut(fourth).unwrap().unseen_count = 2;
    g.store.set_agent_indicator(status(AgentStatus::Blocked), g.parent);
    let needing: Vec<Uuid> = g
        .store
        .companions_needing_attention(g.parent)
        .into_iter()
        .map(Session::id)
        .collect();
    assert_eq!(needing, [g.first, third, fourth]);
    assert!(g.store.companions_needing_attention(g.after).is_empty());
}

// MARK: - Moves

#[test]
fn only_a_move_inside_its_group_is_offered_for_a_companion() {
    let g = Group::new();
    assert!(g.store.can_move_sessions(&[g.second], g.work, Some(1)));
    assert!(g.store.can_move_sessions(&[g.second], g.work, None));
    assert!(!g.store.can_move_sessions(&[g.second], g.work, Some(0)));
    assert!(!g.store.can_move_sessions(&[g.second], g.work, Some(3)));
    assert!(!g.store.can_move_sessions(&[g.second], g.personal, Some(0)));
    assert!(g.store.can_move_sessions(&[g.parent], g.personal, None));
}

#[test]
fn a_companion_batched_with_a_session_already_there_is_not_offered_that_workspace() {
    let mut g = Group::new();
    let resident = add(&mut g.store, g.personal, "/q");
    assert!(!g.store.can_move_sessions(&[g.first, resident], g.personal, None));
    assert_eq!(g.store.move_sessions(&[g.first, resident], g.personal, None), 0);
}

#[test]
fn a_flagged_companion_of_an_unflagged_parent_cannot_leave_its_group_in_the_flagged_tree() {
    let mut g = Group::new();
    g.store.set_flags(true, &[g.first, g.after]);
    let resolution = resolve_sessions(
        &[SessionSource {
            workspace: g.work,
            index: 1,
        }],
        SessionDropTarget::WorkspaceRow {
            id: g.work,
            session_count: 4,
        },
        flagged_tree_child_index(&[1, 3], DropIndex::Before(2)),
    )
    .expect("the drop moves");
    assert!(
        !g.store
            .can_move_sessions(&[g.first], g.work, Some(resolution.destination))
    );
    assert_eq!(
        g.store.move_sessions(&[g.first], g.work, Some(resolution.destination)),
        0
    );
    assert_eq!(g.order(), [g.parent, g.first, g.second, g.after]);
}

#[test]
fn a_parent_moves_to_another_workspace_with_its_companions() {
    let mut g = Group::new();
    assert!(g.store.move_session(g.parent, g.personal, None));
    assert_eq!(g.order(), [g.after]);
    assert_eq!(ids(&g.store, g.personal), [g.parent, g.first, g.second]);
}

#[test]
fn a_parent_moved_down_lands_with_its_group_after_the_next_session() {
    let mut g = Group::new();
    g.store.move_session(g.parent, g.work, Some(3));
    assert_eq!(g.order(), [g.after, g.parent, g.first, g.second]);
}

#[test]
fn a_parent_moved_into_its_own_group_stays_put() {
    let mut g = Group::new();
    g.store.move_session(g.parent, g.work, Some(1));
    assert_eq!(g.order(), [g.parent, g.first, g.second, g.after]);
}

#[test]
fn a_session_moved_into_a_group_lands_after_it() {
    let mut g = Group::new();
    g.store.move_session(g.after, g.work, Some(1));
    assert_eq!(g.order(), [g.parent, g.first, g.second, g.after]);
    g.store.move_session(g.after, g.work, Some(0));
    assert_eq!(g.order(), [g.after, g.parent, g.first, g.second]);
}

#[test]
fn a_companion_reorders_only_among_its_siblings() {
    let mut g = Group::new();
    assert!(g.store.move_session(g.second, g.work, Some(1)));
    assert_eq!(g.order(), [g.parent, g.second, g.first, g.after]);
    assert!(g.store.move_session(g.second, g.work, None));
    assert_eq!(g.order(), [g.parent, g.first, g.second, g.after]);
    assert!(!g.store.move_session(g.second, g.work, Some(0)));
    assert!(!g.store.move_session(g.second, g.work, Some(3)));
    assert!(!g.store.move_session(g.first, g.personal, None));
    assert_eq!(g.order(), [g.parent, g.first, g.second, g.after]);
    assert!(ids(&g.store, g.personal).is_empty());
    assert_eq!(g.session(g.first).parent_id(), Some(g.parent));
}

#[test]
fn selected_siblings_reorder_among_themselves_without_their_parent() {
    let mut g = Group::new();
    let third = companion(&mut g.store, g.parent, CompanionOptions::default());
    assert_eq!(g.store.move_sessions(&[g.first, g.second], g.work, Some(2)), 2);
    assert_eq!(g.order(), [g.parent, third, g.first, g.second, g.after]);
    assert_eq!(g.store.move_sessions(&[g.first, g.second], g.work, Some(0)), 0);
    assert_eq!(g.order(), [g.parent, third, g.first, g.second, g.after]);
}

#[test]
fn a_single_companion_move_counts_only_itself() {
    let mut g = Group::new();
    assert!(g.store.companions(g.first).is_empty());
    assert_eq!(g.store.move_sessions(&[g.second], g.work, Some(1)), 1);
    assert_eq!(g.order(), [g.parent, g.second, g.first, g.after]);
}

#[test]
fn a_batch_carries_companions_once_and_leaves_a_lone_companion() {
    let mut g = Group::new();
    assert_eq!(
        g.store.move_sessions(&[g.first, g.parent, g.second], g.personal, None),
        3
    );
    assert_eq!(ids(&g.store, g.personal), [g.parent, g.first, g.second]);
    assert_eq!(g.store.move_sessions(&[g.first, g.after], g.personal, None), 1);
    assert!(g.order().is_empty());
    assert_eq!(ids(&g.store, g.personal), [g.parent, g.first, g.second, g.after]);
}

#[test]
fn a_batch_drop_index_counts_with_the_selection_removed() {
    let mut g = Group::new();
    let last = add(&mut g.store, g.work, "/last");
    g.store.move_sessions(&[g.parent, g.after], g.work, Some(3));
    assert_eq!(g.order(), [last, g.parent, g.first, g.second, g.after]);
}

#[test]
fn reordering_a_top_level_session_steps_over_a_whole_group() {
    let mut g = Group::new();
    g.store.reorder_session(g.after, ReorderDirection::Up);
    assert_eq!(g.order(), [g.after, g.parent, g.first, g.second]);
    g.store.reorder_session(g.after, ReorderDirection::Down);
    assert_eq!(g.order(), [g.parent, g.first, g.second, g.after]);
    g.store.reorder_session(g.parent, ReorderDirection::Down);
    assert_eq!(g.order(), [g.after, g.parent, g.first, g.second]);
    g.store.reorder_session(g.parent, ReorderDirection::Top);
    assert_eq!(g.order(), [g.parent, g.first, g.second, g.after]);
}

#[test]
fn reordering_a_companion_stays_within_its_siblings() {
    let mut g = Group::new();
    g.store.reorder_session(g.first, ReorderDirection::Down);
    assert_eq!(g.order(), [g.parent, g.second, g.first, g.after]);
    g.store.take_outbox();
    g.store.reorder_session(g.first, ReorderDirection::Down);
    assert_eq!(g.order(), [g.parent, g.second, g.first, g.after]);
    // no write at an end
    assert!(g.store.take_outbox().is_empty());
    g.store.reorder_session(g.first, ReorderDirection::Top);
    assert_eq!(g.order(), [g.parent, g.first, g.second, g.after]);
}

// MARK: - Projection and persistence

#[test]
fn a_companion_reports_its_parent_and_a_folded_parent_reports_collapsed() {
    let mut g = Group::new();
    g.store.set_session_expanded(g.parent, false);
    let tree = crate::projection::control_tree(&g.store, &tree_context());
    let nodes = &tree.workspaces[0].sessions;
    let order: Vec<String> = g.order().into_iter().map(uuid_string).collect();
    assert_eq!(nodes.iter().map(|node| node.id.clone()).collect::<Vec<_>>(), order);
    let node = |id: Uuid| nodes.iter().find(|node| node.id == uuid_string(id)).unwrap();
    assert_eq!(node(g.first).parent, Some(uuid_string(g.parent)));
    assert_eq!(node(g.parent).parent, None);
    assert_eq!(node(g.parent).collapsed, Some(true));
    assert_eq!(node(g.first).collapsed, None);
    assert_eq!(node(g.parent).content, None);
}

#[test]
fn a_folded_parent_whose_last_companion_closed_no_longer_reports_collapsed() {
    let mut g = Group::new();
    g.store.set_session_expanded(g.parent, false);
    g.store.close_session(g.first);
    g.store.close_session(g.second);
    let tree = crate::projection::control_tree(&g.store, &tree_context());
    let parent = tree.workspaces[0]
        .sessions
        .iter()
        .find(|node| node.id == uuid_string(g.parent))
        .unwrap();
    assert_eq!(parent.collapsed, None);
}

#[test]
fn parent_and_collapsed_are_omitted_from_json_for_a_plain_session() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    add(&mut store, work, "/repo/a");
    let tree = crate::projection::control_tree(&store, &tree_context());
    let json = serde_json::to_string(&tree).unwrap();
    assert!(!json.contains("\"parent\""));
    assert!(!json.contains("\"collapsed\""));
    assert!(!json.contains("\"content\""));
}

pub(super) fn tree_context() -> crate::projection::TreeContext<'static> {
    static APP: std::sync::LazyLock<ngterm_proto::AppIdentity> =
        std::sync::LazyLock::new(|| ngterm_proto::AppIdentity::from_recorded("0.1.0", None));
    static SETTINGS: std::sync::LazyLock<crate::persist::AppSettings> =
        std::sync::LazyLock::new(crate::persist::AppSettings::default);
    crate::projection::TreeContext {
        app: &APP,
        settings: &SETTINGS,
        index_unsaved: false,
        quick_visible: false,
    }
}

#[test]
fn companion_links_and_parent_fold_round_trip_through_snapshot() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let parent = add(&mut store, work, "/parent");
    let id = companion(&mut store, parent, CompanionOptions::default());
    store.session_mut(parent).unwrap().companion.expanded = false;
    let snapshot = store.snapshot();
    let mut restored = AppStore::new();
    restored.restore(&snapshot, false);
    let sessions = &restored.workspaces()[0].sessions;
    assert_eq!(sessions.iter().map(Session::id).collect::<Vec<_>>(), [parent, id]);
    assert_eq!(sessions[1].parent_id(), Some(parent));
    assert!(!sessions[0].companion.expanded);
    assert!(sessions[1].companion.expanded);
    assert_eq!(snapshot.workspaces[0].sessions[0].collapsed, Some(true));
    assert_eq!(snapshot.workspaces[0].sessions[1].collapsed, None);
    let text = serde_json::to_string(&snapshot.workspaces[0].sessions[1]).unwrap();
    assert!(
        text.contains(&format!("\"parentID\":\"{}\"", uuid_string(parent))),
        "{text}"
    );
}

#[test]
fn a_snapshot_without_companion_fields_decodes_a_top_level_expanded_session() {
    let json = format!(
        r#"{{"id":"{}","cwd":"/tmp","parentID":42,"customName":"keep"}}"#,
        Uuid::new_v4()
    );
    let snapshot: SessionSnapshot = serde_json::from_str(&json).unwrap();
    assert_eq!(snapshot.parent_id, None);
    assert_eq!(snapshot.collapsed, None);
    assert_eq!(snapshot.custom_name.as_deref(), Some("keep"));
}

#[test]
fn restore_drops_an_invalid_companion_link_and_regroups_a_hand_edited_order() {
    let (parent, child, orphan, other) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    let linked = |id: Uuid, cwd: &str, parent: Option<Uuid>| SessionSnapshot {
        parent_id: parent,
        ..SessionSnapshot::new(id, cwd)
    };
    let snapshot = Snapshot {
        workspaces: vec![WorkspaceSnapshot {
            id: Uuid::new_v4(),
            name: "work".to_owned(),
            sessions: vec![
                linked(child, "/c", Some(parent)),
                linked(other, "/o", None),
                linked(parent, "/p", None),
                linked(orphan, "/x", Some(Uuid::new_v4())),
            ],
            collapsed: None,
            tint: None,
        }],
        ..Snapshot::default()
    };
    let mut store = AppStore::new();
    store.restore(&snapshot, false);
    let sessions = &store.workspaces()[0].sessions;
    assert_eq!(
        sessions.iter().map(Session::id).collect::<Vec<_>>(),
        [other, parent, child, orphan]
    );
    assert_eq!(
        sessions.iter().map(Session::parent_id).collect::<Vec<_>>(),
        [None, None, Some(parent), None]
    );
}

#[test]
fn regrouping_drops_links_without_a_top_level_parent_and_rebuilds_order() {
    let parent = Session::new(crate::model::SessionSeed {
        initial_cwd: "/p".to_owned(),
        ..crate::model::SessionSeed::default()
    });
    let with_parent = |cwd: &str, parent: Option<Uuid>| {
        Session::new(crate::model::SessionSeed {
            initial_cwd: cwd.to_owned(),
            parent_id: parent,
            ..crate::model::SessionSeed::default()
        })
    };
    let child = with_parent("/c", Some(parent.id()));
    let orphan = with_parent("/o", Some(Uuid::new_v4()));
    let chained = with_parent("/x", Some(child.id()));
    let other = with_parent("/other", None);
    let expected = [other.id(), orphan.id(), parent.id(), child.id(), chained.id()];
    let parent_id = parent.id();
    let regrouped = crate::model::companions::regrouped_companions(vec![child, other, orphan, parent, chained]);
    assert_eq!(regrouped.iter().map(Session::id).collect::<Vec<_>>(), expected);
    assert_eq!(regrouped[1].parent_id(), None);
    assert_eq!(regrouped[3].parent_id(), Some(parent_id));
    assert_eq!(regrouped[4].parent_id(), None);
}
