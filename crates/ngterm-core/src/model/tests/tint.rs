//! Sidebar row tints on the store, ported from upstream's `AppStoreTintTests.swift` and the tint cases of
//! `SnapshotRoundTripTests.swift`.

use ngterm_proto::events::EventKind;
use uuid::Uuid;

use super::support::{add, add_background};
use crate::model::{
    AppStore, NewSession, SaveRequest, SidebarTint, TintChoice, TintColor, TintParts, WorkspaceOptions,
};
use crate::persist::Snapshot;

fn emitted_tree_changed(store: &AppStore) -> bool {
    store
        .outbox()
        .events
        .iter()
        .any(|event| event.kind == EventKind::TreeChanged)
}

fn session_tint(store: &AppStore, id: Uuid) -> Option<SidebarTint> {
    store.session(id).and_then(|session| session.tint)
}

fn workspace_tint(store: &AppStore, id: Uuid) -> Option<SidebarTint> {
    store.workspace(id).and_then(|workspace| workspace.tint)
}

/// `setTintOnASessionReportsOnlyARealChange`, and the no-op writing and emitting nothing.
#[test]
fn a_session_tint_reports_only_a_real_change_and_a_repeat_writes_nothing() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let session = add(&mut store, work, "/a");
    let red = Some(SidebarTint::new(TintColor::Red, TintParts::BOTH));
    store.take_outbox();

    assert!(store.set_session_tint(red, session));
    assert_eq!(session_tint(&store, session), red);
    assert_eq!(store.outbox().save, SaveRequest::Now);
    assert!(emitted_tree_changed(&store));
    store.take_outbox();

    assert!(!store.set_session_tint(red, session));
    assert_eq!(
        store.outbox().save,
        SaveRequest::None,
        "an unchanged tint must not write"
    );
    assert!(!emitted_tree_changed(&store), "nor emit");

    assert!(store.set_session_tint(None, session));
    assert_eq!(session_tint(&store, session), None);
    assert!(!store.set_session_tint(red, Uuid::new_v4()));
}

/// `setTintOnAWorkspaceLeavesItsSessionsUntinted`.
#[test]
fn a_workspace_tint_leaves_its_sessions_untinted() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let session = add(&mut store, work, "/a");
    let red = Some(SidebarTint::on_icon(TintColor::Red));

    assert!(store.set_workspace_tint(red, work));
    assert_eq!(workspace_tint(&store, work), red);
    assert_eq!(session_tint(&store, session), None);
    assert!(!store.set_workspace_tint(red, work));
    assert!(!store.set_workspace_tint(red, Uuid::new_v4()));
}

/// `batchColorKeepsEachRowsPlacementAndStartsUntintedRowsOnTheIcon`.
#[test]
fn a_batch_color_keeps_each_rows_placement_and_starts_untinted_rows_on_the_icon() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let named = add(&mut store, work, "/a");
    let plain = add(&mut store, work, "/b");
    store.set_session_tint(Some(SidebarTint::new(TintColor::Green, TintParts::NAME)), named);

    assert!(store.apply_session_tints(TintChoice::Color(Some(TintColor::Red)), &[named, plain]));

    assert_eq!(
        session_tint(&store, named),
        Some(SidebarTint::new(TintColor::Red, TintParts::NAME))
    );
    assert_eq!(
        session_tint(&store, plain),
        Some(SidebarTint::new(TintColor::Red, TintParts::ICON))
    );
    store.apply_session_tints(TintChoice::Color(None), &[named, plain]);
    assert_eq!((session_tint(&store, named), session_tint(&store, plain)), (None, None));
}

/// `batchPlacementKeepsEachRowsColorAndSkipsUntintedRows`.
#[test]
fn a_batch_placement_keeps_each_rows_color_and_skips_untinted_rows() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let red = add(&mut store, work, "/a");
    let blue = add(&mut store, work, "/b");
    let plain = add(&mut store, work, "/c");
    store.set_session_tint(Some(SidebarTint::on_icon(TintColor::Red)), red);
    store.set_session_tint(Some(SidebarTint::on_icon(TintColor::Blue)), blue);
    store.take_outbox();

    store.apply_session_tints(TintChoice::Parts(TintParts::BOTH), &[red, blue, plain]);

    assert_eq!(
        session_tint(&store, red),
        Some(SidebarTint::new(TintColor::Red, TintParts::BOTH))
    );
    assert_eq!(
        session_tint(&store, blue),
        Some(SidebarTint::new(TintColor::Blue, TintParts::BOTH))
    );
    assert_eq!(session_tint(&store, plain), None);
    store.take_outbox();
    assert!(
        !store.apply_session_tints(TintChoice::Parts(TintParts::BOTH), &[red, blue, plain]),
        "nothing changes the second time"
    );
    assert_eq!(store.outbox().save, SaveRequest::None);
}

/// `creationTintIsSetBeforeTheFirstSave`: the create's own write already carries both tints.
#[test]
fn a_creation_tint_is_set_before_the_first_save() {
    let mut store = AppStore::new();
    let options = WorkspaceOptions {
        tint: Some(SidebarTint::new(TintColor::Orange, TintParts::NAME)),
        ..WorkspaceOptions::default()
    };
    let work = store.add_workspace_with("work", options);
    let spec = NewSession {
        tint: Some(SidebarTint::on_icon(TintColor::Red)),
        ..NewSession::at_cwd("/a")
    };
    let session = store.add_session(work, spec).unwrap();

    let saved = store.snapshot();
    let workspace = saved.workspaces.iter().find(|workspace| workspace.id == work).unwrap();
    assert_eq!(
        workspace.tint,
        Some(SidebarTint::new(TintColor::Orange, TintParts::NAME))
    );
    let stored = workspace.sessions.iter().find(|stored| stored.id == session).unwrap();
    assert_eq!(stored.tint, Some(SidebarTint::on_icon(TintColor::Red)));
}

/// `tintsSurviveASaveAndReload`, through the snapshot's JSON.
#[test]
fn tints_survive_a_save_and_reload() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let session = add(&mut store, work, "/a");
    store.set_session_tint(Some(SidebarTint::new(TintColor::Purple, TintParts::BOTH)), session);
    store.set_workspace_tint(Some(SidebarTint::new(TintColor::Gray, TintParts::NAME)), work);
    let text = serde_json::to_string(&store.snapshot()).unwrap();

    let mut reloaded = AppStore::new();
    reloaded.restore(&serde_json::from_str::<Snapshot>(&text).unwrap(), false);

    assert_eq!(
        session_tint(&reloaded, session),
        Some(SidebarTint::new(TintColor::Purple, TintParts::BOTH))
    );
    assert_eq!(
        workspace_tint(&reloaded, work),
        Some(SidebarTint::new(TintColor::Gray, TintParts::NAME))
    );
}

/// `duplicateDoesNotCopyTheTint`.
#[test]
fn a_duplicate_does_not_copy_the_tint() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let session = add(&mut store, work, "/a");
    store.set_session_tint(Some(SidebarTint::on_icon(TintColor::Red)), session);

    let copy = store.duplicate_session(session, "/home/u").unwrap();

    assert_eq!(session_tint(&store, copy), None);
}

/// `undoingASessionCloseIntoARemovedWorkspaceRebuildsItWithItsTint`.
#[test]
fn undoing_a_session_close_into_a_removed_workspace_rebuilds_it_with_its_tint() {
    let mut store = AppStore::new();
    let keep = store.add_workspace("keep");
    let options = WorkspaceOptions {
        tint: Some(SidebarTint::new(TintColor::Red, TintParts::NAME)),
        ..WorkspaceOptions::default()
    };
    let work = store.add_workspace_with("work", options);
    add(&mut store, keep, "/a");
    let stray = add_background(&mut store, work, "/b");

    assert!(store.soft_close_session(stray));
    let session_close = store.pending_close_summary().unwrap().id;
    assert!(store.soft_remove_workspace(work));

    assert!(store.undo_pending_close(Some(session_close)));
    assert_eq!(
        workspace_tint(&store, work),
        Some(SidebarTint::new(TintColor::Red, TintParts::NAME))
    );
}

/// `reopeningASessionWhoseWorkspaceWasClosedRebuildsItWithItsTint`.
#[test]
fn reopening_a_session_whose_workspace_was_closed_rebuilds_it_with_its_tint() {
    let mut store = AppStore::new();
    let keep = store.add_workspace("keep");
    let options = WorkspaceOptions {
        tint: Some(SidebarTint::on_icon(TintColor::Blue)),
        ..WorkspaceOptions::default()
    };
    let work = store.add_workspace_with("work", options);
    add(&mut store, keep, "/a");
    let stray = add_background(&mut store, work, "/b");
    add_background(&mut store, work, "/c");

    assert!(store.soft_close_session(stray));
    store.finalize_pending_close(store.pending_close_summary().unwrap().id);
    assert!(store.soft_remove_workspace(work));
    store.finalize_pending_close(store.pending_close_summary().unwrap().id);

    let item = store
        .recent_closed_items()
        .into_iter()
        .find(|item| {
            item.session
                .as_ref()
                .is_some_and(|session| session.snapshot.id == stray)
        })
        .unwrap();
    assert!(store.restore_recent_closed(&item));
    assert_eq!(
        workspace_tint(&store, work),
        Some(SidebarTint::on_icon(TintColor::Blue))
    );
}

/// `anUnknownTintColorDropsOnlyTheTint`: the session keeps its flag, the workspace its fold.
#[test]
fn an_unknown_tint_color_drops_only_the_tint() {
    let text = r#"{"version": 1, "workspaces": [{"id": "00000000-0000-0000-0000-000000000004", "name": "w",
        "collapsed": true, "tint": "red", "sessions": [{"id": "00000000-0000-0000-0000-000000000003",
        "cwd": "/tmp", "flagged": true, "tint": {"color": "teal", "on": ["name"]}}]}]}"#;
    let snapshot: Snapshot = serde_json::from_str(text).unwrap();
    let workspace = &snapshot.workspaces[0];
    assert_eq!((workspace.tint, workspace.collapsed), (None, Some(true)));
    let session = &workspace.sessions[0];
    assert_eq!((session.tint, session.flagged), (None, Some(true)));
}

/// `anUntintedTreeWritesNoTintKey`.
#[test]
fn an_untinted_tree_writes_no_tint_key() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    add(&mut store, work, "/a");

    let text = serde_json::to_string(&store.snapshot()).unwrap();

    assert!(!text.contains("\"tint\""), "{text}");
}
