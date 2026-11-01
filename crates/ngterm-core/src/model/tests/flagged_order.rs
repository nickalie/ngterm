//! The flagged view's saved order, ported from upstream's `AppStoreFlaggedOrderTests.swift`, the flagged cases of
//! `AppStoreNavigationTests.swift` and `AppStoreCloseReselectionTests.swift`, and `WindowLibraryFlaggedLayoutTests.swift`.

use ngterm_proto::events::EventKind;
use uuid::Uuid;

use super::support::add;
use crate::model::{AppStore, FlaggedOrderPlacement, Session, SidebarMode, WindowLibrary, Workspace};
use crate::persist::settings::FlaggedViewLayout;
use crate::persist::{
    LibraryPersistence, PersistenceStore, RecentClosedItem, SessionSnapshot, Snapshot, WorkspaceSnapshot,
};

use FlaggedOrderPlacement::{After, Before};

fn sessions(store: &mut AppStore, cwds: &[&str], workspace: &str) -> Vec<Uuid> {
    let workspace = store.add_workspace(workspace);
    cwds.iter().map(|cwd| add(store, workspace, cwd)).collect()
}

fn order(store: &AppStore) -> Vec<Uuid> {
    store.ordered_flagged_session_ids()
}

/// A store of one workspace with a session per name, flagged in `flag_order` (indices into the sessions).
fn flagged(names: &[&str], flag_order: &[usize]) -> (AppStore, Vec<Uuid>) {
    let mut store = AppStore::new();
    let all = sessions(&mut store, names, "work");
    for index in flag_order {
        store.set_flag(true, all[*index]);
    }
    (store, all)
}

fn pick(ids: &[Uuid], indices: &[usize]) -> Vec<Uuid> {
    indices.iter().map(|index| ids[*index]).collect()
}

fn session_entry(store: &AppStore, id: Uuid) -> RecentClosedItem {
    store
        .recent_closed_items()
        .into_iter()
        .find(|item| item.session.as_ref().is_some_and(|recent| recent.snapshot.id == id))
        .expect("an entry for the session")
}

fn flagged_snapshot(ids: &[Uuid], flagged: impl Fn(usize) -> bool, selected: Uuid, order: &[Uuid]) -> Snapshot {
    let sessions = ids
        .iter()
        .enumerate()
        .map(|(index, id)| SessionSnapshot {
            flagged: Some(flagged(index)),
            ..SessionSnapshot::new(*id, format!("/{index}"))
        })
        .collect();
    Snapshot {
        selected_session_id: Some(selected),
        workspaces: vec![WorkspaceSnapshot {
            id: Uuid::new_v4(),
            name: "work".to_owned(),
            sessions,
            collapsed: None,
            tint: None,
        }],
        sidebar_mode: Some(SidebarMode::Flagged),
        flagged_order: Some(order.to_vec()),
        ..Snapshot::default()
    }
}

// MARK: - maintenance

#[test]
fn flagging_appends_and_unflagging_removes() {
    let (mut store, s) = flagged(&["/a", "/b", "/c"], &[0, 2, 1]);
    assert_eq!(store.flagged_order(), pick(&s, &[0, 2, 1]));
    assert_eq!(order(&store), pick(&s, &[0, 2, 1]));
    let tree: Vec<Uuid> = store.flagged_sessions().into_iter().map(Session::id).collect();
    assert_eq!(tree, s, "the flagged tree keeps tree order");

    store.set_flag(false, s[2]);

    assert_eq!(store.flagged_order(), pick(&s, &[0, 1]));
}

#[test]
fn flag_all_keeps_arranged_positions_and_appends_the_rest_in_tree_order() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let s: Vec<Uuid> = ["/a", "/b", "/c", "/d"]
        .iter()
        .map(|cwd| add(&mut store, work, cwd))
        .collect();
    store.set_flag(true, s[2]);
    store.set_flag(true, s[0]);

    store.apply_workspace_flag(crate::commands::ToggleMode::On, work);

    assert_eq!(store.flagged_order(), pick(&s, &[2, 0, 1, 3]));
}

#[test]
fn flagging_an_already_flagged_session_changes_nothing() {
    let (mut store, s) = flagged(&["/a", "/b"], &[1, 0]);

    store.set_flag(true, s[1]);
    store.set_flags(true, &[s[0], s[1]]);

    assert_eq!(store.flagged_order(), pick(&s, &[1, 0]));
}

#[test]
fn clear_flags_empties_the_order() {
    let (mut store, _) = flagged(&["/a", "/b"], &[0, 1]);

    store.clear_flags();

    assert!(store.flagged_order().is_empty());
    assert!(order(&store).is_empty());
}

// MARK: - move

#[test]
fn move_rearranges_the_block_in_its_current_order() {
    let cases: [(&[usize], FlaggedOrderPlacement, usize, [usize; 4]); 4] = [
        (&[3], Before, 0, [3, 0, 1, 2]),
        (&[0], After, 2, [1, 2, 0, 3]),
        (&[2, 0], After, 3, [1, 3, 0, 2]),
        (&[1, 2], Before, 0, [1, 2, 0, 3]),
    ];
    for (moving, placement, anchor, want) in cases {
        let (mut store, s) = flagged(&["/a", "/b", "/c", "/d"], &[0, 1, 2, 3]);

        assert!(
            store.move_flagged(&pick(&s, moving), placement, s[anchor]),
            "{moving:?}"
        );

        assert_eq!(order(&store), pick(&s, &want), "{moving:?} {placement:?} {anchor}");
    }
}

#[test]
fn a_move_that_leaves_the_order_unchanged_is_a_no_op() {
    let (mut store, s) = flagged(&["/a", "/b", "/c"], &[0, 1, 2]);

    assert!(!store.move_flagged(&[s[1]], After, s[0]));
    assert!(!store.move_flagged(&[s[0]], Before, s[1]));
    assert_eq!(order(&store), s);
}

#[test]
fn a_move_refuses_the_anchor_in_the_block_and_ids_outside_the_flagged_set() {
    let mut store = AppStore::new();
    let s = sessions(&mut store, &["/a", "/b", "/c"], "work");
    store.set_flag(true, s[0]);
    store.set_flag(true, s[1]);

    assert!(!store.move_flagged(&[s[0]], Before, s[0]));
    assert!(!store.move_flagged(&[s[2]], Before, s[0]));
    assert!(!store.move_flagged(&[s[0]], After, s[2]));
    assert!(!store.move_flagged(&[Uuid::new_v4()], After, s[1]));
    assert!(!store.move_flagged(&[], After, s[1]));
    assert_eq!(order(&store), pick(&s, &[0, 1]));
}

#[test]
fn a_move_signals_a_tree_change_and_saves_only_when_the_order_changes() {
    let mut store = AppStore::new();
    let s = sessions(&mut store, &["/a", "/b"], "work");
    store.set_flag(true, s[0]);
    store.set_flag(true, s[1]);
    store.take_outbox();

    store.move_flagged(&[s[0]], Before, s[1]);
    assert!(store.take_outbox().is_empty());

    store.move_flagged(&[s[1]], Before, s[0]);
    let outbox = store.take_outbox();
    let kinds: Vec<EventKind> = outbox.events.iter().map(|event| event.kind).collect();
    assert_eq!(kinds, [EventKind::TreeChanged]);
    assert_eq!(outbox.save, crate::model::SaveRequest::Now);
}

// MARK: - pending closes and Open Recent

#[test]
fn a_move_keeps_a_parked_pending_close_in_its_slot_so_undo_returns_it_in_place() {
    let (mut store, s) = flagged(&["/a", "/b", "/c"], &[0, 1, 2]);
    assert!(store.soft_close_session(s[1]));
    assert_eq!(order(&store), pick(&s, &[0, 2]));

    assert!(store.move_flagged(&[s[2]], Before, s[0]));
    assert_eq!(store.flagged_order(), pick(&s, &[2, 1, 0]));

    assert!(store.undo_pending_close(None));
    assert_eq!(order(&store), pick(&s, &[2, 1, 0]));
}

#[test]
fn finalizing_a_pending_close_drops_it_from_the_order() {
    let (mut store, s) = flagged(&["/a", "/b"], &[0, 1]);
    assert!(store.soft_close_session(s[0]));

    store.finalize_all_pending_closes();

    assert_eq!(store.flagged_order(), [s[1]]);
}

#[test]
fn undo_after_clear_flags_appends_the_returning_session() {
    let (mut store, s) = flagged(&["/a", "/b", "/c"], &[0, 1]);
    assert!(store.soft_close_session(s[0]));
    store.clear_flags();
    store.set_flag(true, s[2]);

    assert!(store.undo_pending_close(None));

    assert_eq!(order(&store), pick(&s, &[2, 0]));
}

#[test]
fn reopened_sessions_append_in_arrival_order_not_tree_order() {
    let mut store = AppStore::new();
    let s = sessions(&mut store, &["/a", "/b", "/c"], "work");
    for id in &s {
        store.set_flag(true, *id);
    }
    store.close_session(s[0]);
    store.close_session(s[1]);
    assert_eq!(store.flagged_order(), [s[2]]);

    for id in [s[1], s[0]] {
        let item = session_entry(&store, id);
        assert!(store.restore_recent_closed(&item));
    }

    let tree: Vec<Uuid> = store.sessions().map(Session::id).collect();
    assert_eq!(tree, s);
    assert_eq!(order(&store), pick(&s, &[2, 1, 0]));
}

#[test]
fn a_flag_after_a_reopen_lands_after_the_reopened_session() {
    let mut store = AppStore::new();
    let s = sessions(&mut store, &["/b", "/c", "/d"], "work");
    store.set_flag(true, s[0]);
    store.set_flag(true, s[1]);
    store.close_session(s[0]);
    let item = session_entry(&store, s[0]);
    assert!(store.restore_recent_closed(&item));

    store.set_flag(true, s[2]);

    assert_eq!(order(&store), pick(&s, &[1, 0, 2]));
}

#[test]
fn a_hard_closed_then_reopened_session_appends() {
    let mut store = AppStore::new();
    let s = sessions(&mut store, &["/a", "/b", "/c"], "work");
    for id in &s {
        store.set_flag(true, *id);
    }
    store.close_session(s[1]);
    assert_eq!(store.flagged_order(), pick(&s, &[0, 2]));

    let item = session_entry(&store, s[1]);
    assert!(store.restore_recent_closed(&item));

    assert_eq!(order(&store), pick(&s, &[0, 2, 1]));
}

#[test]
fn a_deleted_then_reopened_workspace_appends_its_flagged_sessions() {
    let mut store = AppStore::new();
    let kept = sessions(&mut store, &["/a"], "kept");
    let gone = sessions(&mut store, &["/x", "/y"], "gone");
    store.set_flag(true, gone[0]);
    store.set_flag(true, kept[0]);
    store.set_flag(true, gone[1]);
    let gone_id = store.workspace_for_session(gone[0]).map(Workspace::id).unwrap();

    store.remove_workspace(gone_id);
    assert_eq!(store.flagged_order(), [kept[0]]);

    let item = store
        .recent_closed_items()
        .into_iter()
        .find(|item| item.workspace.is_some())
        .expect("the workspace entry");
    assert!(store.restore_recent_closed(&item));

    assert_eq!(order(&store), [kept[0], gone[0], gone[1]]);
}

#[test]
fn reopening_into_another_window_appends_to_that_windows_order() {
    let mut library = WindowLibrary::new("/home/u");
    let first = library.new_window(None);
    let second = library.new_window(None);
    let moved = {
        let store = library.store_mut(first).unwrap();
        let moved = sessions(store, &["/a"], "work")[0];
        store.set_flag(true, moved);
        store.close_session(moved);
        moved
    };
    let resident = {
        let store = library.store_mut(second).unwrap();
        let resident = sessions(store, &["/z"], "work")[0];
        store.set_flag(true, resident);
        resident
    };
    let item = library
        .recent_closed_items()
        .into_iter()
        .find(|item| item.session.as_ref().is_some_and(|recent| recent.snapshot.id == moved))
        .expect("the session entry");

    let store = library.store_mut(second).unwrap();
    assert!(store.restore_recent_closed(&item));

    assert_eq!(order(store), [resident, moved]);
}

// MARK: - snapshot

#[test]
fn the_saved_order_survives_a_snapshot_round_trip() {
    let (store, s) = flagged(&["/a", "/b", "/c"], &[2, 0]);

    let json = serde_json::to_string(&store.snapshot()).unwrap();
    let decoded: Snapshot = serde_json::from_str(&json).unwrap();
    let mut restored = AppStore::new();
    restored.restore(&decoded, false);

    assert!(json.contains(&format!(r#""flaggedOrder":["{}""#, s[2].to_string().to_uppercase())));
    assert_eq!(decoded.flagged_order, Some(pick(&s, &[2, 0])));
    assert_eq!(order(&restored), pick(&s, &[2, 0]));
}

#[test]
fn a_snapshot_without_the_order_restores_in_tree_order() {
    let (store, s) = flagged(&["/a", "/b", "/c"], &[2, 0]);
    let mut snapshot = store.snapshot();
    snapshot.flagged_order = None;

    let mut restored = AppStore::new();
    restored.restore(&snapshot, false);

    assert_eq!(restored.flagged_order(), pick(&s, &[0, 2]));
}

#[test]
fn restore_drops_unknown_unflagged_and_repeated_ids_and_appends_missing_flagged_ones() {
    let (store, s) = flagged(&["/a", "/b", "/c", "/d"], &[3, 0, 2]);
    let mut snapshot = store.snapshot();
    snapshot.flagged_order = Some(vec![s[2], Uuid::new_v4(), s[1], s[2], s[3]]);

    let mut restored = AppStore::new();
    restored.restore(&snapshot, false);

    assert_eq!(restored.flagged_order(), pick(&s, &[2, 3, 0]));
}

#[test]
fn the_snapshot_omits_the_order_when_nothing_is_flagged() {
    let mut store = AppStore::new();
    sessions(&mut store, &["/a"], "work");

    assert_eq!(store.snapshot().flagged_order, None);
    assert!(
        !serde_json::to_string(&store.snapshot())
            .unwrap()
            .contains("flaggedOrder")
    );
}

// MARK: - navigation and reselection (AppStoreNavigationTests, AppStoreCloseReselectionTests)

#[test]
fn flagged_navigation_follows_the_order_the_layout_shows() {
    let cases = [
        (FlaggedViewLayout::Flat, [3, 0, 2]),
        (FlaggedViewLayout::Plain, [3, 0, 2]),
        (FlaggedViewLayout::Tree, [0, 2, 3]),
    ];
    for (layout, want) in cases {
        let mut store = AppStore::new();
        let mut ids = sessions(&mut store, &["/a", "/b"], "work");
        ids.extend(sessions(&mut store, &["/c", "/d"], "personal"));
        for index in [3, 0, 2] {
            store.set_flag(true, ids[index]);
        }
        store.set_sidebar_mode(SidebarMode::Flagged);
        store.set_flagged_layout(layout);

        assert_eq!(store.navigable_session_ids(), pick(&ids, &want), "{layout:?}");
    }
}

#[test]
fn restore_into_the_flagged_view_reselects_the_first_row_of_the_layouts_order() {
    for (layout, want) in [(FlaggedViewLayout::Plain, 2), (FlaggedViewLayout::Tree, 0)] {
        let ids = [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
        let mut store = AppStore::new();
        store.set_flagged_layout(layout);

        store.restore(
            &flagged_snapshot(&ids, |index| index != 1, ids[1], &[ids[2], ids[0]]),
            false,
        );

        assert_eq!(store.selected_session_id(), Some(ids[want]), "{layout:?}");
    }
}

#[test]
fn a_close_in_the_flagged_view_with_no_recency_picks_the_neighbour_in_the_order_shown() {
    let cases = [
        (FlaggedViewLayout::Flat, 0),
        (FlaggedViewLayout::Plain, 0),
        (FlaggedViewLayout::Tree, 2),
    ];
    for (layout, want) in cases {
        let ids = [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
        let mut store = AppStore::new();
        store.set_flagged_layout(layout);
        store.restore(
            &flagged_snapshot(&ids, |_| true, ids[1], &[ids[2], ids[1], ids[0], ids[3]]),
            false,
        );
        assert_eq!(store.session_recency().items(), [ids[1]]);

        store.close_session(ids[1]);

        assert_eq!(store.selected_session_id(), Some(ids[want]), "{layout:?}");
    }
}

#[test]
fn a_soft_close_in_the_flat_flagged_view_picks_the_saved_order_neighbour() {
    let ids = [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
    let mut store = AppStore::new();
    store.restore(
        &flagged_snapshot(&ids, |_| true, ids[0], &[ids[1], ids[0], ids[2]]),
        false,
    );

    assert!(store.soft_close_session(ids[0]));

    assert_eq!(store.selected_session_id(), Some(ids[2]));
}

// MARK: - the layout mirror (WindowLibraryFlaggedLayoutTests)

#[test]
fn launch_restored_stores_carry_the_layout_passed_to_the_bootstrap() {
    let dir = tempfile::tempdir().unwrap();
    let library = LibraryPersistence::new(dir.path()).bootstrap_with_flagged_layout("/home/u", FlaggedViewLayout::Tree);
    let first = library.active_window_id().unwrap();

    assert_eq!(library.flagged_layout(), FlaggedViewLayout::Tree);
    assert_eq!(library.store(first).unwrap().flagged_layout(), FlaggedViewLayout::Tree);
}

/// A window saved in the flagged view whose selection is hidden (session 1 is unflagged) and whose saved order leads
/// with session 2, while the tree leads with session 0.
fn hidden_selection_flagged_window(ids: &[Uuid; 3]) -> Snapshot {
    flagged_snapshot(ids, |index| index != 1, ids[1], &[ids[2], ids[0]])
}

#[test]
fn a_launch_restore_repairs_the_selection_in_the_bootstrap_layouts_order() {
    let dir = tempfile::tempdir().unwrap();
    let mut persistence = LibraryPersistence::new(dir.path());
    let library = persistence.bootstrap("/home/u");
    let window = library.active_window_id().unwrap();
    assert!(persistence.save_all_checked(&library));
    let ids = [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
    PersistenceStore::new(persistence.window_path(window))
        .save(&hidden_selection_flagged_window(&ids))
        .unwrap();

    let library = LibraryPersistence::new(dir.path()).bootstrap_with_flagged_layout("/home/u", FlaggedViewLayout::Tree);

    let store = library.store(window).expect("the window reopens");
    assert_eq!(
        store.selected_session_id(),
        Some(ids[0]),
        "the tree's first flagged row"
    );
}

#[test]
fn a_runtime_load_repairs_the_selection_in_the_librarys_layout_order() {
    let dir = tempfile::tempdir().unwrap();
    let mut persistence = LibraryPersistence::new(dir.path());
    let mut library = persistence.bootstrap("/home/u");
    let closed = library.new_window(Some("closed"));
    persistence.close_window(&mut library, closed);
    let ids = [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
    PersistenceStore::new(persistence.window_path(closed))
        .save(&hidden_selection_flagged_window(&ids))
        .unwrap();

    library.set_flagged_layout(FlaggedViewLayout::Tree);
    assert!(persistence.load_window(&mut library, closed));

    let store = library.store(closed).unwrap();
    assert_eq!(
        store.selected_session_id(),
        Some(ids[0]),
        "the tree's first flagged row"
    );
}

#[test]
fn the_plain_bootstrap_defaults_to_flat() {
    let dir = tempfile::tempdir().unwrap();
    let library = LibraryPersistence::new(dir.path()).bootstrap("/home/u");

    assert_eq!(library.flagged_layout(), FlaggedViewLayout::Flat);
}

#[test]
fn setting_the_layout_reaches_every_open_store() {
    let mut library = WindowLibrary::seeded("/home/u");
    let first = library.active_window_id().unwrap();
    let second = library.new_window(Some("second"));

    library.set_flagged_layout(FlaggedViewLayout::Plain);

    for window in [first, second] {
        assert_eq!(
            library.store(window).unwrap().flagged_layout(),
            FlaggedViewLayout::Plain
        );
    }
}

#[test]
fn a_store_opened_after_the_change_takes_the_current_layout() {
    let dir = tempfile::tempdir().unwrap();
    let mut persistence = LibraryPersistence::new(dir.path());
    let mut library = persistence.bootstrap("/home/u");
    let closed = library.new_window(Some("closed"));
    persistence.close_window(&mut library, closed);

    library.set_flagged_layout(FlaggedViewLayout::Tree);
    assert!(persistence.load_window(&mut library, closed));

    assert_eq!(library.store(closed).unwrap().flagged_layout(), FlaggedViewLayout::Tree);
}
