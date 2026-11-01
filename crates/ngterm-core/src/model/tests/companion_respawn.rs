//! A parent's exiting shell restarts while it has companions, ported from upstream's `AppStoreCompanionsTests` (the six
//! respawn cases) on an injected clock: the last pane's exit respawns a local parent's primary shell in its last cwd
//! as a plain login shell, unless the previous respawn is under two seconds old, and a session with no companions still
//! closes.

use std::time::{Duration, Instant};

use ngterm_host::fakes::{FakeSurface, FakeSurfaceProbe};
use uuid::Uuid;

use super::support::add;
use crate::model::{
    AgentIndicator, AgentStatus, AppStore, CompanionOptions, NewSession, PrimaryPaneExit, Session, StatusPane,
};

const HOME: &str = "/home/u";

fn companion(store: &mut AppStore, parent: Uuid) -> Uuid {
    store
        .add_companion(parent, CompanionOptions::default(), HOME)
        .expect("the companion is created")
}

/// `work` holding a parent at `/start` with one companion: the store, the parent and the companion.
fn group() -> (AppStore, Uuid, Uuid) {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let parent = add(&mut store, work, "/start");
    let child = companion(&mut store, parent);
    (store, parent, child)
}

fn get(store: &AppStore, id: Uuid) -> &Session {
    store.session(id).expect("the session is in the tree")
}

/// Puts a fresh fake terminal in session `id`'s pane `pane`.
fn mount(store: &mut AppStore, id: Uuid, pane: StatusPane) -> FakeSurfaceProbe {
    let surface = FakeSurface::new(if pane == StatusPane::Left { "L" } else { "R" });
    let probe = surface.probe();
    let session = store.session_mut(id).expect("the session is in the tree");
    session.pane_slot_mut(pane).replace(Box::new(surface));
    probe
}

fn start() -> Instant {
    Instant::now()
}

#[test]
fn a_parents_last_pane_exit_starts_a_fresh_shell_in_its_last_cwd() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let spec = NewSession {
        command: Some("lazygit".to_owned()),
        wait: true,
        ..NewSession::at_cwd("/start")
    };
    let parent = store.add_session(work, spec).expect("the workspace exists");
    companion(&mut store, parent);
    let surface = mount(&mut store, parent, StatusPane::Left);
    let session = store.session_mut(parent).unwrap();
    session.current_cwd = Some("/moved".to_owned());
    session.restore.pending_foreground = Some(vec!["htop".to_owned()]);
    session.osc_title = Some("lazygit".to_owned());
    let identity = session.pane_identity;

    assert_eq!(store.close_primary_pane(parent), PrimaryPaneExit::Respawned);

    let session = get(&store, parent);
    assert_eq!(surface.teardowns(), 1);
    assert!(!session.surface.is_present());
    assert_eq!(session.pane_identity, identity);
    assert_eq!(session.initial_cwd(), "/moved");
    assert_eq!(session.launch.command, None);
    assert!(!session.launch.wait);
    assert_eq!(session.restore.pending_foreground, None);
    assert_eq!(session.osc_title, None);
    assert_eq!(session.primary_respawns(), 1);
}

// a shell exiting as soon as it started respawned forever, saving the store on every turn
#[test]
fn a_parent_whose_respawned_shell_exits_at_once_closes_with_its_group() {
    let (mut store, parent, child) = group();
    let start = start();

    mount(&mut store, parent, StatusPane::Left);
    assert_eq!(store.close_primary_pane_at(parent, start), PrimaryPaneExit::Respawned);
    mount(&mut store, parent, StatusPane::Left);
    assert_eq!(
        store.close_primary_pane_at(parent, start + Duration::from_secs(1)),
        PrimaryPaneExit::Closed
    );

    assert!(store.session(parent).is_none());
    assert!(store.session(child).is_none());
}

#[test]
fn a_respawned_shell_that_ran_a_while_respawns_again() {
    let (mut store, parent, _) = group();
    let start = start();

    mount(&mut store, parent, StatusPane::Left);
    assert_eq!(store.close_primary_pane_at(parent, start), PrimaryPaneExit::Respawned);
    mount(&mut store, parent, StatusPane::Left);
    assert_eq!(
        store.close_primary_pane_at(parent, start + Duration::from_secs(60)),
        PrimaryPaneExit::Respawned
    );
    assert_eq!(get(&store, parent).primary_respawns(), 2);
}

#[test]
fn a_respawned_shell_exiting_at_once_through_its_split_slot_closes_the_group() {
    let (mut store, parent, _) = group();
    let start = start();

    mount(&mut store, parent, StatusPane::Left);
    assert_eq!(store.close_primary_pane_at(parent, start), PrimaryPaneExit::Respawned);
    store.close_split_pane_at(parent, start + Duration::from_secs(1));

    assert!(store.session(parent).is_none());
}

#[test]
fn a_primary_exit_promotes_a_live_split_before_any_respawn() {
    let (mut store, parent, _) = group();
    store.toggle_split(parent, None);
    mount(&mut store, parent, StatusPane::Left);
    mount(&mut store, parent, StatusPane::Right);

    assert_eq!(store.close_primary_pane(parent), PrimaryPaneExit::Promoted);
    assert_eq!(get(&store, parent).primary_respawns(), 0);

    assert_eq!(store.close_primary_pane(parent), PrimaryPaneExit::Respawned);
    assert!(store.session(parent).is_some());
    assert_eq!(get(&store, parent).primary_respawns(), 1);
}

#[test]
fn a_session_without_companions_still_closes_on_its_last_pane_exit() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let session = add(&mut store, work, "/start");
    mount(&mut store, session, StatusPane::Left);

    assert_eq!(store.close_primary_pane(session), PrimaryPaneExit::Closed);
    assert!(store.session(session).is_none());
}

#[test]
fn a_respawn_through_the_split_slot_drops_the_split_and_finalizes_its_daemon() {
    let (mut store, parent, _) = group();
    store.toggle_split(parent, None);
    let split = mount(&mut store, parent, StatusPane::Right);
    let split_identity = get(&store, parent).split_pane_identity.expect("a split pane");
    store.set_agent_indicator(
        AgentIndicator {
            status: AgentStatus::Blocked,
            status_pane: Some(StatusPane::Right),
            ..AgentIndicator::default()
        },
        parent,
    );
    store.take_outbox();

    // the primary slot is empty (no terminal mounted yet), so the split's exit was the last pane's
    store.close_split_pane_at(parent, start());

    let session = get(&store, parent);
    assert_eq!(session.primary_respawns(), 1);
    assert_eq!(split.teardowns(), 1);
    assert!(!session.split.exists && !session.split.shown);
    assert_eq!(session.split_pane_identity, None);
    assert_eq!(session.agent_indicator.status, AgentStatus::Idle);
    assert_eq!(store.outbox().finalized_panes, [split_identity]);
}

#[test]
fn a_remote_parent_never_respawns_here() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let spec = NewSession {
        remote_host: Some("box".to_owned()),
        ..NewSession::at_cwd("/start")
    };
    let parent = store.add_session(work, spec).expect("the workspace exists");
    let child = NewSession {
        parent: Some(parent),
        ..NewSession::at_cwd("/start")
    };
    store.add_session(work, child).expect("the workspace exists");
    assert!(store.has_companions(parent));

    assert!(!store.respawns_on_last_pane_exit(parent, start()));
    assert_eq!(store.close_primary_pane(parent), PrimaryPaneExit::Closed);
}
