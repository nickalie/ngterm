//! Convert to Companion, ported from upstream's `AppStoreCompanionsTests` (the convert cases) and
//! `WindowLibraryRecentClosedTests` (`ConvertedAfterClose`): a running top-level session and its companions nest
//! under a fresh session in its slot, keep their objects and state, and close, undo and reopen as the fresh parent's
//! group.

use ngterm_host::fakes::FakeSurface;
use ngterm_proto::events::EventKind;
use uuid::Uuid;

use super::support::{add, add_background, ids};
use crate::model::{
    AppStore, CompanionOptions, ConvertRefusal, NewSession, PrimaryPaneExit, Session, StatusPane, WindowLibrary,
};
use crate::persist::RecentClosedItem;

const HOME: &str = "/home/u";

fn companion(store: &mut AppStore, parent: Uuid) -> Uuid {
    store
        .add_companion(parent, CompanionOptions::default(), HOME)
        .expect("the companion is created")
}

fn convert(store: &mut AppStore, id: Uuid) -> Uuid {
    store.convert_to_companion(id).expect("the session converts")
}

fn get(store: &AppStore, id: Uuid) -> &Session {
    store.session(id).expect("the session is in the tree")
}

fn parent_of(store: &AppStore, id: Uuid) -> Option<Uuid> {
    store.session(id).and_then(Session::parent_id)
}

/// Upstream's `CompanionGroup`: `work` holds `parent` with its companions `first` and `second`, then `after`.
struct Group {
    store: AppStore,
    work: Uuid,
    parent: Uuid,
    first: Uuid,
    second: Uuid,
    after: Uuid,
}

impl Group {
    fn new() -> Self {
        let mut store = AppStore::new();
        let work = store.add_workspace("work");
        let parent = add(&mut store, work, "/parent");
        let after = add(&mut store, work, "/after");
        let first = companion(&mut store, parent);
        let second = companion(&mut store, parent);
        Self {
            store,
            work,
            parent,
            first,
            second,
            after,
        }
    }

    fn order(&self) -> Vec<Uuid> {
        ids(&self.store, self.work)
    }
}

#[test]
fn convert_nests_the_same_session_under_a_fresh_session_in_its_slot() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let before = add(&mut store, work, "/before");
    let spec = NewSession {
        name: Some("task".to_owned()),
        ..NewSession::at_cwd("/start")
    };
    let task = store.add_session(work, spec).unwrap();
    let after = add(&mut store, work, "/after");
    store.session_mut(task).unwrap().set_current_cwd("/work/task");
    store.set_flag(true, task);
    store
        .session_mut(task)
        .unwrap()
        .pane_slot_mut(StatusPane::Left)
        .replace(Box::new(FakeSurface::new("task")));
    let pane = get(&store, task).pane_identity;
    store.select_session(Some(before));

    let fresh = convert(&mut store, task);

    assert_eq!(ids(&store, work), [before, fresh, task, after]);
    let parent = get(&store, fresh);
    assert_eq!(parent.parent_id(), None);
    assert_eq!(parent.initial_cwd(), "/work/task");
    assert_eq!(parent.custom_name, None);
    assert!(!parent.flagged);
    assert!(parent.companion.expanded);
    let converted = get(&store, task);
    assert_eq!(converted.parent_id(), Some(fresh));
    assert!(converted.surface.is_present());
    assert_eq!(converted.pane_identity, pane);
    assert_eq!(converted.display_name(), "task");
    assert!(converted.flagged);
    assert_eq!(store.selected_session_id(), Some(task));
}

#[test]
fn convert_carries_the_sessions_companions_under_the_fresh_parent() {
    let mut g = Group::new();

    let fresh = convert(&mut g.store, g.parent);

    assert_eq!(g.order(), [fresh, g.parent, g.first, g.second, g.after]);
    for id in [g.parent, g.first, g.second] {
        assert_eq!(parent_of(&g.store, id), Some(fresh));
    }
    let companions: Vec<Uuid> = g.store.companions(fresh).into_iter().map(Session::id).collect();
    assert_eq!(companions, [g.parent, g.first, g.second]);
}

#[test]
fn convert_refuses_a_companion_a_remote_row_and_an_unknown_session() {
    let mut g = Group::new();
    let spec = NewSession {
        remote_host: Some("buildbox".to_owned()),
        ..NewSession::at_cwd("/r")
    };
    let remote = g.store.add_session(g.work, spec).unwrap();
    let before = g.order();

    assert_eq!(
        g.store.convert_to_companion(g.first),
        Err(ConvertRefusal::AlreadyCompanion)
    );
    assert_eq!(g.store.convert_to_companion(remote), Err(ConvertRefusal::RemoteSession));
    assert_eq!(
        g.store.convert_to_companion(Uuid::new_v4()),
        Err(ConvertRefusal::NoSuchSession)
    );
    assert_eq!(g.order(), before);
    assert_eq!(parent_of(&g.store, g.first), Some(g.parent));
}

#[test]
fn convert_refusals_carry_their_message() {
    assert_eq!(ConvertRefusal::AlreadyCompanion.message(), "already a companion");
    assert_eq!(
        ConvertRefusal::RemoteSession.message(),
        "convert it on the Mac it runs on"
    );
    assert_eq!(ConvertRefusal::NoSuchSession.message(), "no such session");
}

#[test]
fn convert_announces_only_the_fresh_session() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let task = add(&mut store, work, "/task");
    store.take_outbox();

    let fresh = convert(&mut store, task);

    let events = store.take_outbox().events;
    let created: Vec<Option<Uuid>> = events
        .iter()
        .filter(|event| event.kind == EventKind::SessionCreated)
        .map(|event| event.session)
        .collect();
    assert_eq!(created, [Some(fresh)]);
    assert!(!events.iter().any(|event| event.kind == EventKind::SessionClosed));
    assert!(!events.iter().any(|event| event.kind == EventKind::SessionSelected));
    assert!(events.iter().any(|event| event.kind == EventKind::TreeChanged));
}

#[test]
fn convert_selects_a_background_session() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let selected = add(&mut store, work, "/selected");
    let task = add_background(&mut store, work, "/task");

    let fresh = convert(&mut store, task);

    assert_eq!(store.selected_session_id(), Some(task));
    assert_eq!(ids(&store, work), [selected, fresh, task]);
}

#[test]
fn a_converted_session_closes_on_its_exit_while_the_fresh_parent_respawns() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let task = add(&mut store, work, "/task");
    companion(&mut store, task);
    let fresh = convert(&mut store, task);
    for id in [task, fresh] {
        let session = store.session_mut(id).unwrap();
        session
            .pane_slot_mut(StatusPane::Left)
            .replace(Box::new(FakeSurface::new("L")));
    }

    assert_eq!(store.close_primary_pane(task), PrimaryPaneExit::Closed);
    assert!(store.session(task).is_none());
    assert_eq!(store.close_primary_pane(fresh), PrimaryPaneExit::Respawned);
    assert!(store.session(fresh).is_some());
}

#[test]
fn a_converted_group_round_trips_through_snapshot() {
    let mut g = Group::new();
    let fresh = convert(&mut g.store, g.parent);

    let mut restored = AppStore::new();
    restored.restore(&g.store.snapshot(), false);

    let sessions = &restored.workspaces()[0].sessions;
    let order: Vec<Uuid> = sessions.iter().map(Session::id).collect();
    assert_eq!(order, [fresh, g.parent, g.first, g.second, g.after]);
    assert_eq!(sessions[0].parent_id(), None);
    assert!(sessions[1..4].iter().all(|session| session.parent_id() == Some(fresh)));
}

#[test]
fn a_converted_group_closes_and_reopens_whole() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let task = add(&mut store, work, "/task");
    let other = add(&mut store, work, "/other");
    let old = companion(&mut store, task);
    let fresh = convert(&mut store, task);

    store.close_session(fresh);

    assert_eq!(ids(&store, work), [other]);
    let item = store.recent_closed_items()[0].clone();
    let recent = item.session.as_ref().unwrap();
    assert_eq!(recent.snapshot.id, fresh);
    let companions: Vec<Uuid> = recent.companions.iter().flatten().map(|snapshot| snapshot.id).collect();
    assert_eq!(companions, [task, old]);
    assert!(store.restore_recent_closed(&item));
    assert_eq!(ids(&store, work), [fresh, task, old, other]);
    assert_eq!(parent_of(&store, task), Some(fresh));
    assert_eq!(parent_of(&store, old), Some(fresh));
}

#[test]
fn an_undone_companion_close_follows_its_parent_into_the_converted_group() {
    let mut g = Group::new();
    assert!(g.store.soft_close_session(g.first));
    let fresh = convert(&mut g.store, g.parent);

    assert!(g.store.undo_pending_close(None));

    assert_eq!(parent_of(&g.store, g.first), Some(fresh));
    let mut companions: Vec<Uuid> = g.store.companions(fresh).into_iter().map(Session::id).collect();
    companions.sort();
    let mut expected = vec![g.parent, g.first, g.second];
    expected.sort();
    assert_eq!(companions, expected);
}

#[test]
fn a_converted_groups_hard_close_ends_the_undo_of_companions_parked_under_the_converted_session() {
    let mut g = Group::new();
    assert!(g.store.soft_close_session(g.first));
    let fresh = convert(&mut g.store, g.parent);

    g.store.close_session(fresh);

    assert!(g.store.pending_close_session(g.first).is_none());
    assert!(!g.store.undo_pending_close(None));
    assert!(g.store.session(g.first).is_none());
}

#[test]
fn a_soft_closed_converted_group_undoes_whole() {
    let mut g = Group::new();
    let fresh = convert(&mut g.store, g.parent);

    assert!(g.store.soft_close_session(fresh));
    assert_eq!(g.order(), [g.after]);
    assert!(g.store.undo_pending_close(None));

    assert_eq!(g.order(), [fresh, g.parent, g.first, g.second, g.after]);
    for id in [g.parent, g.first, g.second] {
        assert_eq!(parent_of(&g.store, id), Some(fresh));
    }
}

// MARK: - Reopening (`WindowLibraryRecentClosedTests`)

/// Upstream's `ConvertedAfterClose`: `project` holds `task`, whose companion `closed` was closed before `task` was
/// converted.
struct ConvertedAfterClose {
    library: WindowLibrary,
    window: Uuid,
    project: Uuid,
    task: Uuid,
    closed: Uuid,
    fresh: Uuid,
}

impl ConvertedAfterClose {
    fn new() -> Self {
        let mut library = WindowLibrary::seeded(HOME);
        let window = library.active_window_id().unwrap();
        let store = library.store_mut(window).unwrap();
        let project = store.add_workspace("project");
        let task = add(store, project, "/project");
        let closed = companion(store, task);
        store.close_session(closed);
        let fresh = convert(store, task);
        Self {
            library,
            window,
            project,
            task,
            closed,
            fresh,
        }
    }

    fn store(&self) -> &AppStore {
        self.library.store(self.window).unwrap()
    }

    fn store_mut(&mut self) -> &mut AppStore {
        self.library.store_mut(self.window).unwrap()
    }

    /// The entry of the closed companion.
    fn entry(&self) -> RecentClosedItem {
        self.library
            .recent_closed_items()
            .into_iter()
            .find(|item| {
                item.session
                    .as_ref()
                    .is_some_and(|recent| recent.snapshot.id == self.closed)
            })
            .expect("the closed companion has an entry")
    }

    fn reopen_entry(&mut self) {
        let entry = self.entry();
        let reopened = self.library.reopen_recent_closed_outcome(entry.id, None);
        assert!(reopened.is_some_and(|outcome| outcome.complete));
    }

    fn assert_both_under_the_fresh_parent(&self) {
        assert_eq!(parent_of(self.store(), self.task), Some(self.fresh));
        assert_eq!(parent_of(self.store(), self.closed), Some(self.fresh));
    }
}

#[test]
fn a_companion_closed_before_its_parent_converted_reopens_under_the_fresh_parent() {
    let mut c = ConvertedAfterClose::new();

    c.reopen_entry();

    c.assert_both_under_the_fresh_parent();
    let order = ids(c.store(), c.project);
    assert_eq!(order[0], c.fresh);
    assert_eq!(order.len(), 3);
}

#[test]
fn a_companion_of_a_closed_converted_session_reopens_under_the_fresh_parent() {
    let mut c = ConvertedAfterClose::new();
    let task = c.task;
    c.store_mut().close_session(task);

    c.reopen_entry();

    c.assert_both_under_the_fresh_parent();
}

#[test]
fn a_companion_of_a_converted_session_in_a_closed_group_reopens_that_group_first() {
    let mut c = ConvertedAfterClose::new();
    let fresh = c.fresh;
    c.store_mut().close_session(fresh);
    assert!(!c.library.recent_closed_item_lacks_parent(c.entry().id));

    c.reopen_entry();

    c.assert_both_under_the_fresh_parent();
    assert!(c.library.recent_closed_items().is_empty());
}

#[test]
fn a_companion_of_a_converted_session_in_a_removed_workspace_reopens_that_workspace_first() {
    let mut c = ConvertedAfterClose::new();
    let project = c.project;
    c.store_mut().add_workspace("keep");
    c.store_mut().remove_workspace(project);
    assert!(!c.library.recent_closed_item_lacks_parent(c.entry().id));

    c.reopen_entry();

    c.assert_both_under_the_fresh_parent();
}

#[test]
fn a_companion_reopens_through_a_chain_of_closed_converted_parents() {
    let mut c = ConvertedAfterClose::new();
    let (task, fresh) = (c.task, c.fresh);
    c.store_mut().close_session(task);
    let outer = convert(c.store_mut(), fresh);
    c.store_mut().close_session(fresh);
    assert!(!c.library.recent_closed_item_lacks_parent(c.entry().id));

    c.reopen_entry();

    for id in [fresh, task, c.closed] {
        assert_eq!(parent_of(c.store(), id), Some(outer), "{id}");
    }
    assert!(c.library.recent_closed_items().is_empty());
}
