//! A parent's respawn carried to its viewers (upstream's `ControlServerRespawnTests.swift`): the origin publishes
//! `pane.respawn` once a different live leader runs under the daemon's name, and a viewer's held pane attaches again
//! only for a respawn it has not attached to yet.

use std::time::{Duration, Instant};

use serde_json::json;
use uuid::Uuid;

use super::live::{live, spawn_live};
use super::remote::{OLDER_KINDS, subscribe_listing};
use super::viewer::{Viewer, viewer_built};
use super::wire;
use crate::live::daemon_name;
use crate::model::{CompanionOptions, StatusPane};
use ngterm_host::fakes::FakeSurface;

fn viewer() -> Viewer {
    viewer_built(false, |_| {})
}

fn respawns(generations: &serde_json::Value) -> serde_json::Value {
    json!({"epoch": "e1", "generations": generations})
}

impl Viewer {
    fn primary(&self) -> Uuid {
        self.session().pane_identity
    }

    /// The primary replica's attach exited cleanly; its terminal holds.
    fn held(&mut self) {
        let token = wire(self.primary());
        self.fixture.core.pane_held(self.session, &token);
        self.fixture.core.commit();
    }

    fn respawn(&mut self, generation: i64) {
        let daemon = daemon_name(self.origin.0);
        self.frame(json!({"kind": "pane.respawn", "respawn": respawns(&json!({daemon: generation}))}));
    }

    fn reattaches(&mut self) -> usize {
        self.fixture
            .core
            .take_pane_replacements()
            .iter()
            .filter(|replacement| replacement.remote && replacement.session == self.session)
            .count()
    }
}

#[test]
fn the_origin_publishes_a_respawn_once_a_different_live_leader_runs_under_the_name() {
    let mut live = live();
    let parent = live.session;
    let options = CompanionOptions {
        select: false,
        ..CompanionOptions::default()
    };
    live.fixture
        .store_mut()
        .unwrap()
        .add_companion(parent, options, "/home/u")
        .unwrap();
    spawn_live(&mut live);
    let daemon = daemon_name(live.left);
    live.host.set_record(&daemon, Some(1), Some(20), None);
    live.fixture
        .core
        .live
        .leaders
        .borrow_mut()
        .accept(Some([(daemon.clone(), 20)].into()));
    let (_, viewer) = subscribe_listing(&mut live.fixture.core, parent, "mirror", &OLDER_KINDS);
    let snapshot = viewer.last("snapshot").unwrap();
    let epoch = snapshot["snapshot"]["respawns"]["epoch"].clone();
    assert_eq!(snapshot["snapshot"]["respawns"]["generations"], json!({}));

    assert!(live.fixture.core.pane_exited(parent, &wire(live.left)));
    live.fixture.core.commit();
    assert_eq!(live.fixture.store().session(parent).unwrap().primary_respawns(), 1);
    // the old leader still listed is not the new shell
    live.fixture.core.remote_tick(Instant::now());
    live.host.set_record(&daemon, Some(1), Some(21), None);
    let deadline = Instant::now() + Duration::from_secs(5);
    while viewer.frames("pane.respawn").is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
        live.fixture.core.remote_tick(Instant::now());
    }

    let frames = viewer.frames("pane.respawn");
    assert_eq!(frames.len(), 1, "{:?}", viewer.kinds());
    assert_eq!(
        frames[0]["respawn"],
        json!({"epoch": epoch, "generations": {&daemon: 1}})
    );
    // a later subscriber's snapshot carries every count, once the frontend mounted the fresh terminal
    let surface = FakeSurface::new(wire(live.left)).with_live_session(true);
    live.fixture
        .store_mut()
        .and_then(|store| store.session_mut(parent))
        .unwrap()
        .pane_slot_mut(StatusPane::Left)
        .replace(Box::new(surface));
    let (_, later) = subscribe_listing(&mut live.fixture.core, parent, "mirror", &OLDER_KINDS);
    assert_eq!(
        later.last("snapshot").unwrap()["snapshot"]["respawns"],
        json!({"epoch": epoch, "generations": {daemon: 1}})
    );
}

#[test]
fn a_held_viewer_pane_reattaches_only_after_the_origin_respawned_it() {
    let mut viewer = viewer();
    viewer.connect(&json!({"respawns": respawns(&json!({}))}));
    viewer.held();
    assert_eq!(
        viewer.reattaches(),
        0,
        "a clean exit with no respawn keeps the exit prompt"
    );

    viewer.respawn(1);

    assert_eq!(viewer.reattaches(), 1);
}

#[test]
fn a_respawn_known_before_the_held_exit_reattaches_at_the_exit() {
    let mut viewer = viewer();
    let daemon = daemon_name(viewer.origin.0);
    viewer.connect(&json!({"respawns": respawns(&json!({&daemon: 2}))}));
    viewer.respawn(3);
    assert_eq!(viewer.reattaches(), 0, "a live pane is not reattached");

    viewer.held();

    assert_eq!(viewer.reattaches(), 1);
}

#[test]
fn a_local_wait_exit_is_not_entered_and_a_closed_rows_entry_is_pruned() {
    let mut viewer = viewer();
    let local = viewer.fixture.sessions[0];
    let local_pane = viewer.fixture.store().session(local).unwrap().pane_identity;
    viewer.fixture.core.pane_held(local, &wire(local_pane));
    assert!(
        viewer.fixture.core.remote.viewer.respawns.entry(local_pane).is_none(),
        "a local pane never enters the book"
    );

    viewer.connect(&json!({"respawns": respawns(&json!({}))}));
    let primary = viewer.primary();
    assert!(viewer.fixture.core.remote.viewer.respawns.entry(primary).is_some());
    let session = viewer.session;
    viewer.fixture.store_mut().unwrap().close_session(session);
    viewer.fixture.core.commit();
    viewer.fixture.core.remote_tick(Instant::now());
    assert!(viewer.fixture.core.remote.viewer.respawns.entry(primary).is_none());
}

#[test]
fn a_soft_closed_row_keeps_its_held_exit_through_undo_and_is_pruned_once_final() {
    let mut viewer = viewer();
    viewer.connect(&json!({"respawns": respawns(&json!({}))}));
    viewer.held();
    let (session, primary) = (viewer.session, viewer.primary());

    assert!(viewer.fixture.store_mut().unwrap().soft_close_session(session));
    viewer.fixture.core.commit();
    viewer.fixture.core.remote_tick(Instant::now());
    assert!(
        viewer
            .fixture
            .core
            .remote
            .viewer
            .respawns
            .entry(primary)
            .is_some_and(|entry| entry.held),
        "a soft-closed row keeps its state"
    );
    assert!(viewer.fixture.store_mut().unwrap().undo_pending_close(None));
    viewer.fixture.core.commit();
    // the row's stream comes back with the row, and its snapshot reports the respawn made meanwhile
    let daemon = daemon_name(viewer.origin.0);
    viewer.connect(&json!({"respawns": respawns(&json!({daemon: 1}))}));
    assert_eq!(
        viewer.reattaches(),
        1,
        "the respawn delivered after undo still reattaches the held pane"
    );

    assert!(viewer.fixture.store_mut().unwrap().soft_close_session(session));
    assert!(
        viewer
            .fixture
            .store_mut()
            .unwrap()
            .finalize_pending_close_of_session(session)
    );
    viewer.fixture.core.commit();
    assert!(
        viewer.fixture.core.remote_wake().is_some(),
        "the entry keeps the tick alive"
    );
    viewer.fixture.core.remote_tick(Instant::now());
    assert!(
        viewer.fixture.core.remote.viewer.respawns.entry(primary).is_none(),
        "the tick prunes the last row's entry once it is final"
    );
}
