//! Closing companion groups, ported from upstream's `AppStoreCompanionsTests` (the close and undo cases),
//! `AppStoreEventTests` (`closingAParentEmitsOneClosedEdgePerGroupMember`,
//! `anUndoneCompanionCloseReportsTheParentsCurrentWorkspace`) and `WindowLibraryRecentClosedTests` (the companion
//! cases): a parent closes with its companions into one Open Recent entry, a soft close of it is one undo, and a lone
//! companion reopens only under its parent.

use ngterm_proto::events::EventKind;
use uuid::Uuid;

use super::support::{add, ids};
use crate::model::{AppStore, CompanionOptions, WindowLibrary};
use crate::persist::{RecentClosedItem, RecentClosedKind};

const HOME: &str = "/home/u";

fn companion(store: &mut AppStore, parent: Uuid) -> Uuid {
    store
        .add_companion(parent, CompanionOptions::default(), HOME)
        .expect("the companion is created")
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

/// The snapshot ids a session entry reopens: the session, then its companions.
fn entry_ids(item: &RecentClosedItem) -> Vec<Uuid> {
    let session = item.session.as_ref().expect("a session entry");
    std::iter::once(session.snapshot.id)
        .chain(session.companions.iter().flatten().map(|companion| companion.id))
        .collect()
}

fn session_entry(items: &[RecentClosedItem], session: Uuid) -> RecentClosedItem {
    items
        .iter()
        .find(|item| {
            item.session
                .as_ref()
                .is_some_and(|recent| recent.snapshot.id == session)
        })
        .cloned()
        .expect("the session has an entry")
}

// MARK: - Hard close

#[test]
fn closing_a_parent_closes_its_companions_and_records_one_group_entry() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let parent = add(&mut store, work, "/parent");
    let other = add(&mut store, work, "/other");
    let first = companion(&mut store, parent);
    let second = companion(&mut store, parent);
    store.select_session(Some(second));

    assert_eq!(store.close_session(parent).map(|closed| closed.id()), Some(parent));

    assert_eq!(ids(&store, work), [other]);
    assert_eq!(store.selected_session_id(), Some(other));
    let items = store.recent_closed_items();
    assert_eq!(items.len(), 1);
    assert_eq!(entry_ids(&items[0]), [parent, first, second]);
    let companions = items[0].session.as_ref().unwrap().companions.as_ref().unwrap();
    assert!(companions.iter().all(|companion| companion.parent_id == Some(parent)));
}

#[test]
fn closing_a_companion_closes_only_it() {
    let mut g = Group::new();

    g.store.close_session(g.first);

    assert_eq!(g.order(), [g.parent, g.second, g.after]);
    let items = g.store.recent_closed_items();
    assert_eq!(items.len(), 1);
    assert_eq!(entry_ids(&items[0]), [g.first]);
}

#[test]
fn closing_a_parent_emits_one_closed_edge_per_group_member() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let parent = add(&mut store, work, "/tmp");
    let child = companion(&mut store, parent);
    store.take_outbox();

    store.close_session(parent);

    let closed: Vec<Option<Uuid>> = store
        .take_outbox()
        .events
        .into_iter()
        .filter(|event| event.kind == EventKind::SessionClosed)
        .map(|event| event.session)
        .collect();
    assert_eq!(closed, [Some(child), Some(parent)]);
}

// MARK: - Soft close and undo

#[test]
fn soft_closing_a_parent_is_one_undo_for_the_whole_group() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let parent = add(&mut store, work, "/parent");
    let after = add(&mut store, work, "/after");
    let first = companion(&mut store, parent);
    let second = companion(&mut store, parent);

    assert!(store.soft_close_session(parent));
    assert_eq!(ids(&store, work), [after]);
    let items = store.recent_closed_items();
    assert_eq!(items.len(), 1);
    assert_eq!(entry_ids(&items[0]), [parent, first, second]);

    assert!(store.undo_pending_close(None));
    assert_eq!(ids(&store, work), [parent, first, second, after]);
    for id in [first, second] {
        assert_eq!(store.session(id).unwrap().parent_id(), Some(parent));
    }
    assert!(store.recent_closed_items().is_empty());
}

#[test]
fn a_batch_soft_close_widens_to_the_companions_and_records_one_entry_per_group() {
    let mut g = Group::new();

    assert!(g.store.soft_close_sessions(&[g.parent, g.after]));

    assert!(g.order().is_empty());
    let entries: Vec<Vec<Uuid>> = g.store.recent_closed_items().iter().map(entry_ids).collect();
    assert_eq!(entries.len(), 2);
    assert!(entries.contains(&vec![g.parent, g.first, g.second]), "{entries:?}");
    assert!(entries.contains(&vec![g.after]), "{entries:?}");
    assert!(g.store.undo_pending_close(None));
    assert_eq!(g.order(), [g.parent, g.first, g.second, g.after]);
}

#[test]
fn an_undone_close_of_a_top_level_session_lands_outside_a_group_that_grew_into_its_slot() {
    let mut g = Group::new();
    assert!(g.store.soft_close_session(g.after));
    let third = companion(&mut g.store, g.parent);

    assert!(g.store.undo_pending_close(None));

    assert_eq!(g.order(), [g.parent, g.first, g.second, third, g.after]);
}

#[test]
fn an_undone_companion_close_follows_a_parent_that_moved_away() {
    let mut store = AppStore::new();
    let home = store.add_workspace("home");
    let away = store.add_workspace("away");
    let parent = add(&mut store, home, "/parent");
    let child = companion(&mut store, parent);
    assert!(store.soft_close_session(child));
    assert!(store.move_session(parent, away, None));
    store.take_outbox();

    assert!(store.undo_pending_close(None));

    assert_eq!(store.session(child).unwrap().parent_id(), Some(parent));
    assert_eq!(ids(&store, away), [parent, child]);
    assert!(ids(&store, home).is_empty());
    // the created edge names the workspace the companion came back to
    let created: Vec<Option<Uuid>> = store
        .take_outbox()
        .events
        .into_iter()
        .filter(|event| event.kind == EventKind::SessionCreated)
        .map(|event| event.workspace)
        .collect();
    assert_eq!(created, [Some(away)]);
}

// a companion parked by a soft close came back top-level after its parent closed for good
#[test]
fn a_parents_hard_close_ends_the_undo_of_its_parked_companions() {
    let mut store = AppStore::new();
    let home = store.add_workspace("home");
    let parent = add(&mut store, home, "/parent");
    let other = add(&mut store, home, "/other");
    let parked = companion(&mut store, parent);
    let mate = companion(&mut store, other);
    assert!(store.soft_close_sessions(&[parked, mate]));

    store.close_session(parent);

    assert!(store.pending_close_session(parked).is_none());
    assert!(store.undo_pending_close(None));
    assert!(store.session(parked).is_none());
    assert_eq!(store.session(mate).unwrap().parent_id(), Some(other));
}

// a hard workspace delete left the parked companion of a parent it removed undoable, to come back top-level
#[test]
fn a_hard_workspace_delete_ends_the_undo_of_its_parents_parked_companions() {
    let mut store = AppStore::new();
    let doomed = store.add_workspace("doomed");
    let kept = store.add_workspace("kept");
    let parent = add(&mut store, doomed, "/parent");
    let other = add(&mut store, kept, "/other");
    let parked = companion(&mut store, parent);
    let mate = companion(&mut store, other);
    assert!(store.soft_close_sessions(&[parked, mate]));

    store.remove_workspace(doomed);

    assert!(store.pending_close_session(parked).is_none());
    assert!(store.undo_pending_close(None));
    assert!(store.session(parked).is_none());
    assert_eq!(store.session(mate).unwrap().parent_id(), Some(other));
}

#[test]
fn finalizing_a_parked_parent_ends_the_undo_of_its_parked_companions() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let parent = add(&mut store, work, "/parent");
    let child = companion(&mut store, parent);
    let other = add(&mut store, work, "/other");
    assert!(store.soft_close_sessions(&[parent, child, other]));

    assert!(store.finalize_pending_close_of_session(parent));

    assert!(store.pending_close_session(child).is_none());
    assert!(store.undo_pending_close(None));
    assert!(store.session(child).is_none());
    assert!(store.session(other).is_some());
}

// MARK: - Reopening (`WindowLibraryRecentClosedTests`)

fn library() -> (WindowLibrary, Uuid) {
    let library = WindowLibrary::seeded(HOME);
    let window = library.active_window_id().unwrap();
    (library, window)
}

fn store_mut(library: &mut WindowLibrary, window: Uuid) -> &mut AppStore {
    library.store_mut(window).expect("the window is open")
}

#[test]
fn reopening_a_closed_parent_brings_its_companions_back() {
    let (mut library, window) = library();
    let store = store_mut(&mut library, window);
    let project = store.add_workspace("project");
    let parent = add(store, project, "/project");
    let child = companion(store, parent);
    store.close_session(parent);
    let recent = library.recent_closed_items()[0].clone();

    assert!(library.reopen_recent_closed_outcome(recent.id, None).unwrap().complete);

    let store = library.store(window).unwrap();
    assert_eq!(ids(store, project), [parent, child]);
    assert_eq!(store.session(child).unwrap().parent_id(), Some(parent));
}

#[test]
fn a_lone_companion_reopens_under_its_live_parent_wherever_it_moved() {
    let (mut library, window) = library();
    let store = store_mut(&mut library, window);
    let project = store.add_workspace("project");
    let other = store.add_workspace("other");
    let parent = add(store, project, "/project");
    let child = companion(store, parent);
    store.close_session(child);
    store.move_session(parent, other, None);
    let recent = library.recent_closed_items()[0].clone();

    assert!(library.reopen_recent_closed_outcome(recent.id, None).unwrap().complete);

    let store = library.store(window).unwrap();
    assert_eq!(ids(store, other), [parent, child]);
    assert!(ids(store, project).is_empty());
}

#[test]
fn a_lone_companion_of_a_closed_parent_reopens_the_parent_first() {
    let (mut library, window) = library();
    let store = store_mut(&mut library, window);
    let project = store.add_workspace("project");
    let parent = add(store, project, "/project");
    let child = companion(store, parent);
    store.close_session(child);
    store.close_session(parent);
    let entry = session_entry(&library.recent_closed_items(), child);

    assert!(library.reopen_recent_closed_outcome(entry.id, None).unwrap().complete);

    assert_eq!(ids(library.store(window).unwrap(), project), [parent, child]);
    assert!(library.recent_closed_items().is_empty());
}

#[test]
fn a_lone_companion_of_a_parent_in_a_removed_workspace_reopens_that_workspace_first() {
    let (mut library, window) = library();
    let store = store_mut(&mut library, window);
    let project = store.add_workspace("project");
    store.add_workspace("keep");
    let parent = add(store, project, "/project");
    let child = companion(store, parent);
    store.close_session(child);
    store.remove_workspace(project);
    let entry = session_entry(&library.recent_closed_items(), child);

    assert!(library.reopen_recent_closed_outcome(entry.id, None).unwrap().complete);

    let store = library.store(window).unwrap();
    assert_eq!(ids(store, project), [parent, child]);
    assert_eq!(store.session(child).unwrap().parent_id(), Some(parent));
}

/// Drops the entries matching `drop` from the library's list, as a removal on disk would.
fn forget_entries(library: &mut WindowLibrary, drop: impl Fn(&RecentClosedItem) -> bool) {
    let kept: Vec<RecentClosedItem> = library
        .recent_closed_items()
        .into_iter()
        .filter(|item| !drop(item))
        .collect();
    library.set_recent_closed_items(kept);
}

#[test]
fn only_a_companion_with_no_parent_anywhere_lacks_its_parent() {
    let (mut library, window) = library();
    let store = store_mut(&mut library, window);
    let project = store.add_workspace("project");
    store.add_workspace("keep");
    let parent = add(store, project, "/project");
    let child = companion(store, parent);
    store.close_session(child);
    let entry = session_entry(&library.recent_closed_items(), child);
    assert!(!library.recent_closed_item_lacks_parent(entry.id), "the parent is open");

    store_mut(&mut library, window).remove_workspace(project);
    assert!(
        !library.recent_closed_item_lacks_parent(entry.id),
        "the parent sits in the removed workspace's entry"
    );

    forget_entries(&mut library, |item| item.kind == RecentClosedKind::Workspace);
    assert!(library.recent_closed_item_lacks_parent(entry.id));
}

#[test]
fn a_lone_companion_with_no_parent_anywhere_stays_unavailable() {
    let (mut library, window) = library();
    let store = store_mut(&mut library, window);
    let project = store.add_workspace("project");
    let parent = add(store, project, "/project");
    let child = companion(store, parent);
    store.close_session(child);
    store.close_session(parent);
    forget_entries(&mut library, |item| {
        item.session.as_ref().is_some_and(|recent| recent.snapshot.id == parent)
    });
    let entry = library.recent_closed_items()[0].clone();

    assert_eq!(library.reopen_recent_closed_outcome(entry.id, None), None);

    assert!(library.store(window).unwrap().session(child).is_none());
    let left: Vec<Uuid> = library.recent_closed_items().iter().map(|item| item.id).collect();
    assert_eq!(left, [entry.id]);
}

/// Rewrites the parent each listed session entry's snapshot names, as a hand-edited `recent-closed.json` could.
fn rename_parents(library: &mut WindowLibrary, parents: &[(Uuid, Uuid)]) {
    let items: Vec<RecentClosedItem> = library
        .recent_closed_items()
        .into_iter()
        .map(|mut item| {
            if let Some(recent) = item.session.as_mut()
                && let Some((_, parent)) = parents.iter().find(|(session, _)| *session == recent.snapshot.id)
            {
                recent.snapshot.parent_id = Some(*parent);
            }
            item
        })
        .collect();
    library.set_recent_closed_items(items);
}

#[test]
fn a_companion_entry_naming_itself_its_parent_lacks_a_parent_and_does_not_reopen() {
    let (mut library, window) = library();
    let store = store_mut(&mut library, window);
    let project = store.add_workspace("project");
    let parent = add(store, project, "/project");
    let child = companion(store, parent);
    store.close_session(child);
    rename_parents(&mut library, &[(child, child)]);
    let entry = session_entry(&library.recent_closed_items(), child);

    assert!(library.recent_closed_item_lacks_parent(entry.id));
    assert_eq!(library.reopen_recent_closed_outcome(entry.id, None), None);
    assert!(library.store(window).unwrap().session(child).is_none());
}

#[test]
fn companion_entries_naming_each_other_their_parent_lack_a_parent_and_do_not_reopen() {
    let (mut library, window) = library();
    let store = store_mut(&mut library, window);
    let project = store.add_workspace("project");
    let parent = add(store, project, "/project");
    let first = companion(store, parent);
    let second = companion(store, parent);
    store.close_session(first);
    store.close_session(second);
    rename_parents(&mut library, &[(first, second), (second, first)]);

    for session in [first, second] {
        let entry = session_entry(&library.recent_closed_items(), session);
        assert!(library.recent_closed_item_lacks_parent(entry.id));
        assert_eq!(library.reopen_recent_closed_outcome(entry.id, None), None);
    }
    assert_eq!(library.recent_closed_items().len(), 2);
}
