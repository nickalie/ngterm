//! Viewer refusals and HUD ownership over the host's fake terminals.

use std::collections::HashMap;

use ngterm_host::fakes::FakeSurface;
use ngterm_proto::AppIdentity;
use serde_json::json;

use super::*;
use crate::hud::HudSpec;
use crate::model::{NewSession, RemoteBinding, WindowLibrary};
use crate::{CoreConfig, CoreState};

fn core() -> (Core, Uuid, Uuid) {
    let mut library = WindowLibrary::new("/tmp");
    let window = library.new_window(None);
    let store = library.store_mut(window).unwrap();
    let workspace = store.workspaces()[0].id();
    let session = store
        .add_session(
            workspace,
            NewSession {
                remote_host: Some("origin".to_owned()),
                ..NewSession::at_cwd("/tmp")
            },
        )
        .unwrap();
    let pane = store.session(session).unwrap().pane_identity;
    store
        .session_mut(session)
        .unwrap()
        .surface
        .replace(Box::new(FakeSurface::new(uuid_string(pane))));
    store.bind_remote(
        RemoteBinding::new(
            uuid_string(Uuid::new_v4()),
            HashMap::from([(pane, crate::live::daemon_name(Uuid::new_v4()))]),
            Some(1),
            None,
        ),
        session,
    );
    let core = Core::new(
        CoreConfig {
            app: AppIdentity::from_recorded("0.1.0", None),
            capabilities: Vec::new(),
        },
        CoreState {
            library,
            settings: crate::persist::AppSettings::default(),
            persistence: None,
        },
    );
    (core, window, session)
}

fn hud(message: &str) -> PresentationHud {
    PresentationHud {
        spec: HudSpec::new(message),
        pane: None,
        generation: 1,
        remaining: Some(3.0),
    }
}

#[test]
fn a_mirrored_hud_updates_then_withdraws_its_own_panel() {
    let (mut core, _, session) = core();
    let dir = tempfile::tempdir().unwrap();
    core.set_hud_body_dir(dir.path().join("bodies"));
    core.show_remote_hud(Some(&hud("first")), session);
    let panel = core.library().held_session(session).unwrap();
    assert!(panel.hud_bridged());
    assert_eq!(panel.overlay.effective_hud_spec().unwrap().hide_after, Some(3.0));
    core.show_remote_hud(Some(&hud("second")), session);
    assert_eq!(
        core.library()
            .held_session(session)
            .unwrap()
            .overlay
            .effective_hud_spec()
            .unwrap()
            .message,
        "second"
    );
    core.show_remote_hud(
        Some(&PresentationHud {
            remaining: Some(0.0),
            ..hud("expired")
        }),
        session,
    );
    assert!(!core.library().held_session(session).unwrap().overlay.hud_active());
}

#[test]
fn remote_huds_leave_a_local_panel_alone() {
    let (mut core, _, session) = core();
    let dir = tempfile::tempdir().unwrap();
    core.set_hud_body_dir(dir.path().join("bodies"));
    crate::commands::hud::open(
        &mut core,
        Some(&uuid_string(session)),
        None,
        hud("local").spec,
        &Placement::default(),
    )
    .unwrap();
    core.show_remote_hud(Some(&hud("remote")), session);
    core.show_remote_hud(None, session);
    let panel = core.library().held_session(session).unwrap();
    assert!(!panel.hud_bridged());
    assert_eq!(panel.overlay.effective_hud_spec().unwrap().message, "local");
}

#[test]
fn removed_and_local_rows_refuse_remote_work() {
    let (mut core, window, remote) = core();
    let store = core.library().store(window).unwrap();
    let local = store.sessions().find(|row| row.id() != remote).unwrap().id();
    let ask: PresentationAsk = serde_json::from_value(json!({
        "id":"question", "owner":1, "title":"title", "message":"message",
        "buttons":[{"id":"ok","label":"OK"}], "style":"terminal", "align":"left", "width":50
    }))
    .unwrap();
    let overlay: PresentationOverlay =
        serde_json::from_value(json!({"job":"job", "wait":false, "follow":false})).unwrap();
    for session in [Uuid::new_v4(), local] {
        assert!(!core.show_replica_ask(&ask, session));
        assert!(!core.show_replica_overlay(&overlay, session));
        core.show_remote_hud(Some(&hud("ignored")), session);
        assert!(!core.reattach_remote_pane(session, Uuid::new_v4(), LeadAttachment::new(false), false));
        core.dismiss_replica_ask(
            &PresentationAskRef {
                id: "question".to_owned(),
                owner: 1,
            },
            session,
        );
        assert!(!core.replica_overlay_held(session, None, 1));
        assert!(!core.replica_overlay_held(session, Some(OverlayPane::Left), 1));
    }
    assert!(!core.library().held_session(local).unwrap().overlay.active);
    assert!(core.library().held_session(local).unwrap().ask().is_none());
    core.play_remote_sound(
        Uuid::new_v4(),
        PresentationSound {
            name: Some("default".to_owned()),
        },
    );
    assert!(core.take_status_sounds().is_empty());
}

#[test]
fn reconnects_require_a_current_remote_terminal() {
    let (mut core, _, session) = core();
    let pane = core.library().held_session(session).unwrap().pane_identity;
    assert!(!core.reconnect_waiting("not a uuid"));
    assert_eq!(core.pane_reconnect_note("not a uuid", Instant::now()), None);
    assert!(!core.remote_link_lost(
        session,
        "not a uuid",
        &RemoteLinkNotice {
            nonce: "old".to_owned()
        }
    ));
    assert!(!core.remote_link_lost(
        session,
        &uuid_string(pane),
        &RemoteLinkNotice {
            nonce: "old".to_owned()
        }
    ));
    assert!(!core.reattach_remote_pane(session, Uuid::new_v4(), LeadAttachment::new(false), false));
    // A binding without an origin endpoint cannot replace an existing terminal either.
    assert!(!core.reattach_remote_pane(session, pane, LeadAttachment::new(false), false));
    assert_eq!(core.remote_wake(), None);
}

#[test]
fn a_stale_pane_overlay_close_keeps_its_local_replacement() {
    let (mut core, _, session) = core();
    let found = core.library().held_session(session).unwrap();
    let remote = found
        .remote_presentation()
        .unwrap()
        .binding
        .remote_pane(found.pane_identity)
        .unwrap();
    let overlay = |job: &str| -> PresentationOverlay {
        serde_json::from_value(json!({
            "job": job,
            "pane": uuid_string(remote),
            "wait": false,
            "follow": false
        }))
        .unwrap()
    };
    assert!(core.show_replica_overlay(&overlay("live-job"), session));
    assert!(
        core.close_replica_overlay("live-job", session),
        "the origin closes the overlay its job still holds"
    );
    let left = |core: &Core| {
        core.library()
            .held_session(session)
            .unwrap()
            .pane_overlay(OverlayPane::Left)
            .is_some()
    };
    assert!(!left(&core));

    assert!(core.show_replica_overlay(&overlay("old-job"), session));

    let store = core.library_mut().store_for_session_mut(session).unwrap();
    assert!(store.close_pane_overlay(session, OverlayPane::Left));
    store
        .open_pane_overlay(
            session,
            OverlayPane::Left,
            OverlayProgram {
                command: "local-program".to_owned(),
                ..OverlayProgram::default()
            },
        )
        .unwrap();

    assert!(!core.close_replica_overlay("old-job", session));
    let replacement = core
        .library()
        .held_session(session)
        .unwrap()
        .pane_overlay(OverlayPane::Left);
    assert_eq!(
        replacement.map(|open| open.command.as_str()),
        Some("local-program"),
        "the old job's generation no longer owns the slot"
    );
}
