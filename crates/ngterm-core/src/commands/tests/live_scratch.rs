//! The server's scratch as a live session of its own (upstream's `ServerActionsScratchTests` and `ServerPanesTests`):
//! its seed and its end over a [`FakeSessionHost`], its catalog entry, and the operation pinned to it.

use std::sync::Arc;

use ngterm_host::KillOutcome;
use ngterm_host::fakes::{FakeSessionHost, FakeSurface};
use ngterm_proto::Command;
use ngterm_proto::command::SessionCmd;
use uuid::Uuid;

use super::catalog_receipts::{receipt_of, serve};
use super::catalog_served::{entry, served, subscribed};
use super::live::{install, live, resources};
use super::{Fixture, args, wire};
use crate::catalog::operations::outcome;
use crate::live::{daemon_name, scratch_daemon_name};
use crate::pane_env::uuid_string;
use crate::persist::RestoreMode;

/// Puts a shown scratch terminal carrying `identity` in the first session's scratch slot, as the server's scratch
/// when `live`.
fn scratch(fixture: &mut Fixture, identity: Uuid, live: bool) {
    let session = fixture.sessions[0];
    let found = fixture.store_mut().unwrap().session_mut(session).unwrap();
    found.scratch_surface.replace(Box::new(
        FakeSurface::new(uuid_string(identity)).with_live_session(live),
    ));
    found.scratch_identity = live.then_some(identity);
    found.scratch_active = true;
}

/// A server: a served catalog over a live install.
fn server() -> Fixture {
    let mut fixture = served(0);
    let host = Arc::new(FakeSessionHost::new());
    let resources = resources();
    install(&mut fixture.core, &host, resources.path(), RestoreMode::Live);
    // the wrap checks its executable only when a seed is built
    drop(resources);
    fixture
}

fn send_scratch(fixture: &mut Fixture, edit: impl FnOnce(&mut ngterm_proto::Args)) -> ngterm_proto::Response {
    let target = wire(fixture.sessions[0]);
    let response = fixture.send(Command::Session(SessionCmd::Scratch), Some(&target), args(edit));
    let _ = fixture.core.commit();
    response
}

fn chosen(fixture: &Fixture) -> Option<Uuid> {
    fixture.store().session(fixture.sessions[0]).unwrap().scratch_identity
}

/// After upstream's `onOffOnShowsTheSameDaemon`: the server names the scratch's live session when it first shows it,
/// and a hide and a show keep it.
#[test]
fn a_server_names_the_scratch_once_and_a_hide_keeps_it() {
    let mut fixture = server();

    assert!(send_scratch(&mut fixture, |a| a.mode = Some("on".into())).ok);
    let named = chosen(&fixture).unwrap();
    let session = fixture.sessions[0];
    fixture
        .store_mut()
        .unwrap()
        .session_mut(session)
        .unwrap()
        .scratch_surface
        .replace(Box::new(FakeSurface::new(uuid_string(named)).with_live_session(true)));
    assert!(send_scratch(&mut fixture, |a| a.mode = Some("off".into())).ok);
    assert!(send_scratch(&mut fixture, |a| a.mode = Some("toggle".into())).ok);

    assert_eq!(chosen(&fixture), Some(named));
    assert!(fixture.store().session(session).unwrap().scratch_active);
}

/// After upstream's `onWithACommandReplacesTheScratch`, shown and hidden.
#[test]
fn a_command_replaces_the_servers_scratch_with_a_new_one() {
    for shown in [true, false] {
        let mut fixture = server();
        let old = Uuid::new_v4();
        scratch(&mut fixture, old, true);
        if !shown {
            assert!(send_scratch(&mut fixture, |a| a.mode = Some("off".into())).ok);
        }

        assert!(
            send_scratch(&mut fixture, |a| {
                a.mode = Some("on".into());
                a.command = Some("htop".into());
            })
            .ok
        );

        let found = fixture.store().session(fixture.sessions[0]).unwrap();
        let new = found.scratch_identity.unwrap();
        assert_ne!(new, old);
        assert!(found.scratch_active);
        assert!(!found.scratch_surface.is_present());
        assert_eq!(found.scratch_command.as_deref(), Some("htop"));
    }
}

/// The desktop app's scratch, and a server's that cannot wrap panes, is an ordinary terminal with no name of its own.
#[test]
fn a_scratch_outside_a_wrapping_server_is_not_named() {
    let mut desktop = live().fixture;
    let mut unwrapped = served(0);

    for fixture in [&mut desktop, &mut unwrapped] {
        assert!(send_scratch(fixture, |a| a.mode = Some("on".into())).ok);
        assert_eq!(chosen(fixture), None);
    }
}

#[test]
fn closing_the_scratch_forgets_its_name() {
    let mut fixture = server();
    assert!(send_scratch(&mut fixture, |a| a.mode = Some("on".into())).ok);
    let session = fixture.sessions[0];

    fixture.store_mut().unwrap().close_scratch(session, true);

    assert_eq!(chosen(&fixture), None);
}

#[test]
fn a_scratch_seed_attaches_to_a_live_session_of_the_scratch_name() {
    let live = live();
    let identity = Uuid::new_v4();

    let seed = live.fixture.core.scratch_seed(identity, None).unwrap();

    assert!(seed.live && !seed.wait_after_command);
    let attach = seed.attach.clone().unwrap();
    assert_eq!(attach.session, scratch_daemon_name(identity));
    assert!(seed.command.unwrap().contains(&scratch_daemon_name(identity)));
    let pane_id = seed.env.iter().find(|(name, _)| name == "NGTERM_PANE_ID");
    assert_eq!(pane_id.map(|(_, value)| value.clone()), Some(uuid_string(identity)));
}

#[test]
fn a_scratch_command_runs_once_in_place_of_the_login_shell() {
    let live = live();

    let seed = live.fixture.core.scratch_seed(Uuid::new_v4(), Some("htop")).unwrap();

    let argv = seed.attach.unwrap().argv.join(" ");
    assert!(argv.contains("htop"), "{argv}");
}

#[test]
fn an_instance_that_wraps_no_pane_seeds_no_live_scratch() {
    let fixture = Fixture::new(0);

    assert_eq!(fixture.core.scratch_seed(Uuid::new_v4(), None), None);
}

/// After upstream's `closingTheSessionEndsItsScratch` and `onWithACommandReplacesTheScratch`: what drops a scratch
/// ends its live session by the scratch name, never a pane's.
#[test]
fn ending_a_scratch_kills_its_live_session_by_the_scratch_name() {
    let live = live();
    let identity = Uuid::new_v4();
    live.host.insert(scratch_daemon_name(identity), "");

    assert!(live.fixture.core.end_scratch(identity));

    assert_eq!(live.host.kills(), [(scratch_daemon_name(identity), true)]);
    assert!(live.host.session(&scratch_daemon_name(identity)).is_none());
}

#[test]
fn a_failed_scratch_kill_is_reported() {
    let live = live();
    let identity = Uuid::new_v4();
    live.host
        .set_kill_outcome(&scratch_daemon_name(identity), Some(KillOutcome::Failed("busy".into())));

    assert!(!live.fixture.core.end_scratch(identity));
}

/// Upstream's `unownedScratchDaemonsAreEndedAndTheRestKept`, at a start, where no scratch is this run's.
#[test]
fn a_previous_runs_scratch_daemons_are_ended_and_the_rest_kept() {
    let mut fixture = Fixture::new(0);
    let host = Arc::new(FakeSessionHost::new());
    let resources = resources();
    install(&mut fixture.core, &host, resources.path(), RestoreMode::Live);
    let pane = daemon_name(Uuid::new_v4());
    let leftover = scratch_daemon_name(Uuid::new_v4());
    for name in [pane.as_str(), leftover.as_str(), "work"] {
        host.insert(name, "");
    }

    let ended = fixture.core.end_previous_scratches();

    assert_eq!(ended, std::slice::from_ref(&leftover));
    assert_eq!(host.kills(), [(leftover, true)]);
    for kept in [pane.as_str(), "work"] {
        assert!(host.session(kept).is_some(), "{kept}");
    }
}

/// After upstream's `aScratchWhoseKillFailedIsNoLongerOwned`: a scratch the start could not end is not reported ended,
/// and nothing is listed or killed without a session host.
#[test]
fn a_scratch_whose_kill_failed_is_not_reported_ended() {
    let live = live();
    let leftover = scratch_daemon_name(Uuid::new_v4());
    live.host.insert(leftover.clone(), "");
    live.host
        .set_kill_outcome(&leftover, Some(KillOutcome::Failed("busy".into())));

    assert!(live.fixture.core.end_previous_scratches().is_empty());
    assert_eq!(live.host.kills(), [(leftover, true)]);
    assert!(Fixture::new(0).core.end_previous_scratches().is_empty());
}

#[test]
fn a_live_scratch_is_published_with_its_live_session_and_shown_state() {
    let mut fixture = served(0);
    let session = fixture.sessions[0];
    let identity = Uuid::new_v4();
    scratch(&mut fixture, identity, true);

    let shown = entry(&subscribed(&mut fixture, false).snapshot(), session);
    fixture.store_mut().unwrap().toggle_scratch(session, false);
    let hidden = entry(&subscribed(&mut fixture, false).snapshot(), session);

    let published = shown.fields.scratch.unwrap();
    assert_eq!(published.pane.identity, identity);
    assert_eq!(published.pane.daemon, scratch_daemon_name(identity));
    assert!(published.shown);
    assert!(!hidden.fields.scratch.unwrap().shown);
}

#[test]
fn a_scratch_that_is_no_live_session_is_not_published() {
    let mut fixture = served(0);
    let session = fixture.sessions[0];
    scratch(&mut fixture, Uuid::new_v4(), false);

    let published = entry(&subscribed(&mut fixture, false).snapshot(), session);

    assert_eq!(published.fields.scratch, None);
}

/// Upstream's plan: a session whose last pane is lost stays in the catalog ended, its scratch still running.
#[test]
fn an_ended_session_keeps_its_scratch_in_the_catalog() {
    let mut fixture = served(0);
    let session = fixture.sessions[0];
    let identity = Uuid::new_v4();
    scratch(&mut fixture, identity, true);

    assert!(fixture.store_mut().unwrap().mark_session_ended(session));
    let ended = entry(&subscribed(&mut fixture, false).snapshot(), session);

    assert!(ended.panes.is_empty());
    assert_eq!(
        ended.fields.scratch.map(|scratch| scratch.pane.identity),
        Some(identity)
    );
}

/// Upstream's `aStaleExpectedScratchIsRefusedAndLeavesTheReplacementRunning`.
#[test]
fn a_stale_expected_scratch_is_refused_and_leaves_the_replacement_running() {
    let mut fixture = served(0);
    let session = fixture.sessions[0];
    let current = Uuid::new_v4();
    scratch(&mut fixture, current, true);
    let stale = args(|a| {
        a.mode = Some("on".into());
        a.command = Some("top".into());
        a.scratch = Some(wire(Uuid::new_v4()));
    });

    let reply = serve(
        &mut fixture,
        Command::Session(SessionCmd::Scratch),
        Some(&wire(session)),
        stale,
        "replace",
    );

    assert!(!reply.ok);
    assert_eq!(
        reply.error.as_deref(),
        Some("the session's scratch changed; take the catalog again")
    );
    let found = fixture.store().session(session).unwrap();
    assert_eq!(found.scratch_identity, Some(current));
    assert_eq!(found.scratch_command, None);
}

/// Upstream's `anExpectedScratchThatMatchesIsServedWithAReceipt`, for a running scratch and for none.
#[test]
fn an_expected_scratch_that_matches_is_served_with_a_receipt() {
    let mut fixture = served(0);
    let session = fixture.sessions[0];
    let none = args(|a| {
        a.mode = Some("on".into());
        a.scratch = Some("none".into());
    });
    let mut fixture_server = server();
    let target = wire(fixture_server.sessions[0]);
    let first = serve(
        &mut fixture_server,
        Command::Session(SessionCmd::Scratch),
        Some(&target),
        none,
        "first",
    );
    assert_eq!(receipt_of(&first).map(|r| r.outcome), Some(outcome::APPLIED.to_owned()));
    let published = subscribed(&mut fixture_server, false).snapshot();
    let named = chosen(&fixture_server).unwrap();
    let listed = entry(&published, fixture_server.sessions[0]).fields.scratch.unwrap();
    assert_eq!((listed.pane.identity, listed.shown), (named, true));

    let identity = Uuid::new_v4();
    scratch(&mut fixture, identity, true);
    let matching = args(|a| {
        a.mode = Some("off".into());
        a.scratch = Some(uuid_string(identity).to_lowercase());
    });
    let second = serve(
        &mut fixture,
        Command::Session(SessionCmd::Scratch),
        Some(&wire(session)),
        matching,
        "second",
    );

    assert_eq!(
        receipt_of(&second).map(|r| r.outcome),
        Some(outcome::APPLIED.to_owned())
    );
    assert!(!fixture.store().session(session).unwrap().scratch_active);
}
