//! End-to-end dispatcher and model cases from upstream's `ControlDispatcherMessagesTests`,
//! `AppStoreMessagesTests` and `ControlServerMessagesTests`, over realized fake surfaces.

use ngterm_host::fakes::FakeSurface;
use ngterm_proto::command::MsgCmd;
use ngterm_proto::{Args, Command, ControlEvent, ControlResult, EventKind, Request, Response};
use serde_json::json;
use uuid::Uuid;

use super::{Fixture, wire};
use crate::messages::{MAX_BODY_BYTES, MessagePane};
use crate::model::{CompanionOptions, NewSession, StatusPane};

fn fixture() -> (Fixture, Uuid, [Uuid; 3]) {
    let mut fixture = Fixture::new(0);
    let id = fixture.sessions[0];
    fixture.store_mut().unwrap().set_split_visibility(id, true, None);
    let session = fixture.store_mut().unwrap().session_mut(id).unwrap();
    let panes = [
        session.pane_identity,
        session.split_pane_identity.unwrap(),
        Uuid::new_v4(),
    ];
    for (role, pane) in [StatusPane::Left, StatusPane::Right, StatusPane::Scratch]
        .into_iter()
        .zip(panes)
    {
        session
            .pane_slot_mut(role)
            .replace(Box::new(FakeSurface::new(wire(pane))));
    }
    (fixture, id, panes)
}

fn address(session: Uuid, pane: Uuid) -> String {
    format!("{}:{}", wire(session), wire(pane))
}

fn dispatch(fixture: &mut Fixture, command: MsgCmd, args: Args) -> Response {
    fixture.core.dispatch(&Request {
        cmd: Command::Msg(command),
        target: None,
        args: Some(args),
    })
}

fn result(response: Response) -> ControlResult {
    assert!(response.ok, "{response:?}");
    serde_json::from_value(response.result.unwrap()).unwrap()
}

fn send(fixture: &mut Fixture, targets: Vec<String>) -> ControlResult {
    result(dispatch(
        fixture,
        MsgCmd::Send,
        Args {
            body: Some("hi".into()),
            recipients: Some(targets),
            ..Args::default()
        },
    ))
}

fn pending(fixture: &Fixture, panes: [Uuid; 3]) -> [usize; 3] {
    panes.map(|pane| fixture.core.library.messages.pending(pane))
}

fn mounted_remote(fixture: &mut Fixture) -> (Uuid, Uuid) {
    let workspace = fixture.workspace;
    let store = fixture.store_mut().unwrap();
    let id = store
        .add_session(
            workspace,
            NewSession {
                remote_host: Some("host".into()),
                ..NewSession::at_cwd("/r")
            },
        )
        .unwrap();
    let session = store.session_mut(id).unwrap();
    let pane = session.pane_identity;
    session.surface.replace(Box::new(FakeSurface::new(wire(pane))));
    (id, pane)
}

fn mounted_other_window(fixture: &mut Fixture) -> (Uuid, Uuid, Uuid, Uuid) {
    let window = fixture.core.library_mut().new_window(None);
    let store = fixture.core.library_mut().store_mut(window).unwrap();
    let id = store.selected_session_id().unwrap();
    let workspace = store.workspace_for_session(id).unwrap().id();
    let session = store.session_mut(id).unwrap();
    let pane = session.pane_identity;
    session.surface.replace(Box::new(FakeSurface::new(wire(pane))));
    (window, workspace, id, pane)
}

fn message_events(fixture: &mut Fixture) -> Vec<ControlEvent> {
    fixture.core.commit();
    let run = wire(fixture.core.events().ring().run());
    result(fixture.core.dispatch(&Request {
        cmd: "events.read".parse().unwrap(),
        target: None,
        args: Some(Args {
            after: Some("0".into()),
            run: Some(run),
            ..Args::default()
        }),
    }))
    .events
    .unwrap()
    .items
    .into_iter()
    .filter(|event| event.kind == EventKind::Message)
    .collect()
}

#[test]
fn recipients_resolve_once_and_unknowns_are_partial_rejections() {
    let (mut f, id, panes) = fixture();
    let unknown = format!("{}:right", wire(Uuid::new_v4()));
    let canonical = address(id, panes[0]);
    let sent = send(
        &mut f,
        vec![format!("{}:left", wire(id)), canonical.clone(), unknown.clone()],
    );
    assert_eq!(sent.delivered, Some(vec![canonical.clone()]));
    assert_eq!(
        serde_json::to_value(sent.rejected).unwrap(),
        json!([{"address":unknown,"reason":"unknown pane"}])
    );
    let read = result(dispatch(
        &mut f,
        MsgCmd::Read,
        Args {
            identity: Some(canonical),
            ..Args::default()
        },
    ));
    assert_eq!(read.messages.unwrap()[0].id, sent.message_id.unwrap());
    assert_eq!(pending(&f, panes), [1, 0, 0]);
    let none = send(&mut f, vec![unknown]);
    assert!(none.delivered.unwrap().is_empty());
}

#[test]
fn send_validation_order_refuses_before_queueing() {
    let (mut f, id, panes) = fixture();
    let cases = vec![
        (Args::default(), "msg.send requires a body".to_owned()),
        (
            Args {
                body: Some("hi".into()),
                all: Some(true),
                recipients: Some(vec!["bad".into()]),
                ..Args::default()
            },
            "msg.send takes --to or --all, not both".into(),
        ),
        (
            Args {
                body: Some("hi".into()),
                recipients: Some(vec![]),
                ..Args::default()
            },
            "msg.send requires --to or --all".into(),
        ),
        (
            Args {
                body: Some("hi".into()),
                recipients: Some(vec!["bad".into()]),
                workspace: Some("w".into()),
                ..Args::default()
            },
            "msg.send: --workspace narrows --all only".into(),
        ),
        (
            Args {
                body: Some("é".repeat(MAX_BODY_BYTES / 2 + 1)),
                all: Some(true),
                identity: Some("bad".into()),
                ..Args::default()
            },
            format!(
                "message body too large ({} bytes, max {MAX_BODY_BYTES})",
                MAX_BODY_BYTES + 2
            ),
        ),
        (
            Args {
                body: Some("hi".into()),
                all: Some(true),
                identity: Some(format!("{}:left", wire(id))),
                ..Args::default()
            },
            format!("identity \"{}:left\" must name a pane id, not a role", wire(id)),
        ),
        (
            Args {
                body: Some("hi".into()),
                recipients: Some(vec!["S".into()]),
                ..Args::default()
            },
            "address \"S\" names a session, not a pane".into(),
        ),
        (
            Args {
                body: Some("hi".into()),
                recipients: Some(vec!["S:nope".into()]),
                ..Args::default()
            },
            "invalid address \"S:nope\": expected SESSION:PANE".into(),
        ),
    ];
    for (args, error) in cases {
        assert_eq!(dispatch(&mut f, MsgCmd::Send, args).error, Some(error));
    }
    assert_eq!(pending(&f, panes), [0, 0, 0]);
}

#[test]
fn invalid_later_recipient_does_not_partially_deliver() {
    let (mut f, id, panes) = fixture();
    let response = dispatch(
        &mut f,
        MsgCmd::Send,
        Args {
            body: Some("hi".into()),
            recipients: Some(vec![address(id, panes[0]), "malformed".into()]),
            ..Args::default()
        },
    );
    assert!(!response.ok);
    assert_eq!(pending(&f, panes), [0, 0, 0]);
}

#[test]
fn stale_or_forged_identity_never_becomes_anonymous() {
    let (mut f, id, panes) = fixture();
    let forged = address(id, Uuid::new_v4());
    for cmd in [MsgCmd::Send, MsgCmd::Read, MsgCmd::Ack] {
        let response = dispatch(
            &mut f,
            cmd,
            Args {
                body: Some("hi".into()),
                all: Some(true),
                identity: Some(forged.clone()),
                ids: Some(vec![wire(Uuid::new_v4())]),
                ..Args::default()
            },
        );
        assert!(!response.ok);
        assert!(response.error.unwrap().contains("is not a live pane of session"));
    }
    assert_eq!(pending(&f, panes), [0, 0, 0]);
}

#[test]
fn read_ack_require_identity_then_ack_requires_valid_ids() {
    let (mut f, id, panes) = fixture();
    for cmd in [MsgCmd::Read, MsgCmd::Ack] {
        assert_eq!(
            dispatch(&mut f, cmd, Args::default()).error,
            Some(format!(
                "msg.{} requires an identity: --as SESSION:PANE-ID",
                if cmd == MsgCmd::Read { "read" } else { "ack" }
            ))
        );
    }
    for ids in [None, Some(vec![]), Some(vec!["nope".to_owned()])] {
        let expected = if ids.as_ref().is_some_and(|ids| !ids.is_empty()) {
            "invalid message id: nope"
        } else {
            "msg.ack requires message ids"
        };
        assert_eq!(
            dispatch(
                &mut f,
                MsgCmd::Ack,
                Args {
                    identity: Some(address(id, panes[0])),
                    ids,
                    ..Args::default()
                }
            )
            .error
            .as_deref(),
            Some(expected)
        );
    }
}

#[test]
fn read_ack_receipts_are_exact_and_ack_matches_ids_case_insensitively() {
    let (mut f, id, panes) = fixture();
    let target = address(id, panes[0]);
    let first = send(&mut f, vec![target.clone()]).message_id.unwrap();
    send(&mut f, vec![target.clone()]);
    let ack = result(dispatch(
        &mut f,
        MsgCmd::Ack,
        Args {
            identity: Some(target.clone()),
            ids: Some(vec![first.to_lowercase()]),
            ..Args::default()
        },
    ));
    assert_eq!(ack.acked, Some(vec![first]));
    let batch = result(dispatch(
        &mut f,
        MsgCmd::Read,
        Args {
            identity: Some(target.clone()),
            ack: Some(true),
            ..Args::default()
        },
    ));
    assert_eq!(
        batch.acked.unwrap(),
        batch.messages.unwrap().iter().map(|m| m.id.clone()).collect::<Vec<_>>()
    );
    let empty = result(dispatch(
        &mut f,
        MsgCmd::Read,
        Args {
            identity: Some(target),
            ack: Some(true),
            ..Args::default()
        },
    ));
    assert_eq!(empty.acked, Some(vec![]));
    assert_eq!(empty.messages, Some(vec![]));
    assert_eq!(empty.remaining, Some(0));
}

#[test]
fn broadcast_spans_windows_excludes_sender_and_obeys_workspace_scope() {
    let (mut f, id, panes) = fixture();
    let (_, _, other, other_pane) = mounted_other_window(&mut f);
    let (remote, remote_pane) = mounted_remote(&mut f);
    let identity = address(id, panes[0]);
    let sent = result(dispatch(
        &mut f,
        MsgCmd::Send,
        Args {
            body: Some("all".into()),
            all: Some(true),
            identity: Some(identity.clone()),
            window: Some("ignored when unscoped".into()),
            ..Args::default()
        },
    ));
    let delivered = sent.delivered.unwrap();
    assert_eq!(delivered.len(), 3);
    assert!(delivered.contains(&address(other, other_pane)));
    assert!(!delivered.contains(&address(remote, remote_pane)));
    assert_eq!(f.core.library.messages.pending(remote_pane), 0);
    assert_eq!(pending(&f, panes), [0, 1, 1]);
    assert_eq!(
        f.core.library.messages.read(other_pane, 32, false).messages.unwrap()[0]
            .from
            .as_deref(),
        Some(identity.as_str())
    );
    let workspace = wire(f.workspace);
    let window = wire(f.window);
    let sent = result(dispatch(
        &mut f,
        MsgCmd::Send,
        Args {
            body: Some("work".into()),
            all: Some(true),
            workspace: Some(workspace),
            window: Some(window),
            ..Args::default()
        },
    ));
    assert_eq!(sent.delivered.unwrap().len(), 3);
    assert_eq!(pending(&f, panes), [1, 2, 2]);
    assert_eq!(f.core.library.messages.pending(other_pane), 1);
    assert_eq!(f.core.library.messages.pending(remote_pane), 0);
}

#[test]
fn split_without_a_mounted_surface_is_neither_recipient_nor_identity() {
    let mut f = Fixture::new(0);
    let id = f.sessions[0];
    f.store_mut().unwrap().set_split_visibility(id, true, None);
    let split = f.store().session(id).unwrap().split_pane_identity.unwrap();
    for shown in [true, false] {
        f.store_mut().unwrap().set_split_visibility(id, shown, None);
        let target = format!("{}:right", wire(id));
        let sent = send(&mut f, vec![target.clone()]);
        assert_eq!(sent.delivered, Some(vec![]));
        assert_eq!(
            serde_json::to_value(sent.rejected).unwrap(),
            json!([{"address": target, "reason": "unknown pane"}])
        );
        for command in [MsgCmd::Send, MsgCmd::Read, MsgCmd::Ack] {
            let response = dispatch(
                &mut f,
                command,
                Args {
                    body: Some("hi".into()),
                    all: Some(true),
                    identity: Some(address(id, split)),
                    ids: Some(vec![wire(Uuid::new_v4())]),
                    ..Args::default()
                },
            );
            assert_eq!(
                response.error,
                Some(format!(
                    "pane {} is not a live pane of session {}",
                    wire(split),
                    wire(id)
                ))
            );
        }
        assert_eq!(f.core.library.messages.pending(split), 0);
    }
}

#[test]
fn a_send_delivering_nothing_emits_no_message_event() {
    let (mut f, id, _) = fixture();
    let sent = send(&mut f, vec![address(id, Uuid::new_v4())]);
    assert_eq!(sent.delivered, Some(vec![]));
    assert_eq!(sent.rejected.unwrap().len(), 1);
    assert!(message_events(&mut f).is_empty());
}

#[test]
fn a_cross_window_send_emits_once_from_the_recipient_window() {
    let (mut f, id, panes) = fixture();
    let (window, workspace, recipient, pane) = mounted_other_window(&mut f);
    let sender = address(id, panes[0]);
    let sent = result(dispatch(
        &mut f,
        MsgCmd::Send,
        Args {
            body: Some("hi".into()),
            recipients: Some(vec![address(recipient, pane)]),
            identity: Some(sender.clone()),
            ..Args::default()
        },
    ));
    let events = message_events(&mut f);
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(
        (&event.window, &event.workspace, &event.session),
        (&Some(wire(window)), &Some(wire(workspace)), &Some(wire(recipient)))
    );
    assert_eq!(event.payload.message_id, sent.message_id);
    assert_eq!(event.payload.pane_id, Some(wire(pane)));
    assert_eq!(event.payload.from, Some(sender));
    assert!(event.payload.body.is_none());
}

#[test]
fn only_realized_local_panes_qualify_including_hidden_splits_and_scratch() {
    let (mut f, id, panes) = fixture();
    let stranger = f.add_session(f.workspace);
    let stranger_pane = f.store().session(stranger).unwrap().pane_identity;
    f.store_mut()
        .unwrap()
        .session_mut(stranger)
        .unwrap()
        .surface
        .replace(Box::new(FakeSurface::new(wire(stranger_pane)).with_realized(false)));
    f.store_mut().unwrap().set_split_visibility(id, false, None);
    assert_eq!(
        send(
            &mut f,
            vec![format!("{}:right", wire(id)), format!("{}:scratch", wire(id))]
        )
        .delivered
        .unwrap()
        .len(),
        2
    );
    assert!(
        f.store()
            .message_recipient(id, MessagePane::Id(stranger_pane))
            .is_none()
    );
    assert!(
        f.store()
            .message_recipient(stranger, MessagePane::Role(StatusPane::Left))
            .is_none()
    );
    let (remote, _) = mounted_remote(&mut f);
    assert!(
        f.store()
            .message_recipient(remote, MessagePane::Role(StatusPane::Left))
            .is_none()
    );
    assert_eq!(pending(&f, panes), [0, 1, 1]);
}

#[test]
fn tree_counts_omit_zero_and_events_have_recipient_location_without_body() {
    let (mut f, id, panes) = fixture();
    let sent = send(&mut f, vec![address(id, panes[0])]);
    f.core.commit();
    let tree = serde_json::to_value(f.core.tree(Some(&wire(f.window))).unwrap()).unwrap();
    let surfaces = tree["workspaces"][0]["sessions"][0]["surfaces"].as_array().unwrap();
    assert_eq!(
        surfaces.iter().find(|p| p["kind"] == "left").unwrap()["pendingMessages"],
        1
    );
    assert!(
        surfaces
            .iter()
            .find(|p| p["kind"] == "right")
            .unwrap()
            .get("pendingMessages")
            .is_none()
    );
    let run = wire(f.core.events().ring().run());
    let events = result(f.core.dispatch(&Request {
        cmd: "events.read".parse().unwrap(),
        target: None,
        args: Some(Args {
            after: Some("0".into()),
            run: Some(run),
            ..Args::default()
        }),
    }));
    let event = events
        .events
        .unwrap()
        .items
        .into_iter()
        .find(|event| event.kind == ngterm_proto::EventKind::Message)
        .unwrap();
    assert_eq!(
        (event.window, event.workspace, event.session),
        (Some(wire(f.window)), Some(wire(f.workspace)), Some(wire(id)))
    );
    assert_eq!(event.payload.message_id, sent.message_id);
    assert_eq!(event.payload.pane_id, Some(wire(panes[0])));
    assert!(event.payload.body.is_none());
}

#[test]
fn swaps_hide_and_promotion_preserve_surviving_pane_mail() {
    let (mut f, id, panes) = fixture();
    send(&mut f, vec![address(id, panes[0]), address(id, panes[1])]);
    f.store_mut().unwrap().swap_panes(id).unwrap();
    assert_eq!(
        f.store()
            .message_recipient(id, MessagePane::Role(StatusPane::Right))
            .unwrap()
            .pane,
        panes[0]
    );
    f.store_mut().unwrap().set_split_visibility(id, false, None);
    assert_eq!(pending(&f, panes), [1, 1, 0]);
    f.store_mut().unwrap().close_primary_pane(id);
    assert_eq!(f.store().session(id).unwrap().pane_identity, panes[0]);
    assert_eq!(pending(&f, panes), [1, 0, 0]);
}

#[test]
fn each_pane_close_disposes_only_its_own_mail() {
    let (mut f, id, panes) = fixture();
    send(&mut f, panes.map(|p| address(id, p)).to_vec());
    f.store_mut().unwrap().close_split(id);
    assert_eq!(pending(&f, panes), [1, 0, 1]);
    f.store_mut().unwrap().close_scratch(id, true);
    assert_eq!(pending(&f, panes), [1, 0, 0]);
    f.store_mut().unwrap().close_session(id);
    assert_eq!(pending(&f, panes), [0, 0, 0]);
}

#[test]
fn soft_close_undo_and_workspace_closes_do_not_resurrect_mail() {
    for soft_workspace in [None, Some(false), Some(true)] {
        let (mut f, id, panes) = fixture();
        send(&mut f, panes.map(|p| address(id, p)).to_vec());
        if let Some(soft) = soft_workspace {
            f.add_workspace("other", 0);
            let work = f.workspace;
            if soft {
                assert!(f.store_mut().unwrap().soft_remove_workspace(work));
            } else {
                f.store_mut().unwrap().remove_workspace(work);
            }
        } else {
            assert!(f.store_mut().unwrap().soft_close_session(id));
            assert!(f.store_mut().unwrap().undo_pending_close(None));
        }
        assert_eq!(pending(&f, panes), [0, 0, 0]);
    }
}

#[test]
fn parent_close_includes_companions_and_respawn_drops_old_program_mail() {
    let (mut f, id, panes) = fixture();
    let child = f
        .store_mut()
        .unwrap()
        .add_companion(id, CompanionOptions::default(), "/home/u")
        .unwrap();
    let child_pane = f.store().session(child).unwrap().pane_identity;
    f.store_mut()
        .unwrap()
        .session_mut(child)
        .unwrap()
        .surface
        .replace(Box::new(FakeSurface::new(wire(child_pane))));
    send(&mut f, vec![address(id, panes[0]), address(child, child_pane)]);
    f.store_mut().unwrap().close_split(id);
    f.store_mut().unwrap().close_primary_pane(id);
    assert_eq!(pending(&f, panes), [0, 0, 0]);
    assert_eq!(f.core.library.messages.pending(child_pane), 1);
    f.store_mut().unwrap().close_session(id);
    assert_eq!(f.core.library.messages.pending(child_pane), 0);
}

#[test]
fn window_close_and_restore_drop_mail() {
    let (mut f, id, panes) = fixture();
    send(&mut f, panes.map(|p| address(id, p)).to_vec());
    let snapshot = f.store().snapshot();
    f.store_mut().unwrap().restore(&snapshot, false);
    assert_eq!(pending(&f, panes), [0, 0, 0]);
    let bus = f.core.library.messages.clone();
    bus.send("x", None, &[crate::messages::MessageRecipient::new(id, panes[0])], 1.0)
        .unwrap();
    f.core.library_mut().close_window(f.window);
    assert_eq!(bus.pending(panes[0]), 0);
}

#[test]
fn restart_discards_old_mail_after_shutdown_immediately_before_replacement() {
    use crate::Dispatched;
    use ngterm_host::ProcessRecord;
    use ngterm_proto::command::SessionCmd;
    let mut live = super::live::live();
    let session = live.session;
    let pane = live.left;
    let daemon = crate::live::daemon_name(pane);
    live.host.set_record(&daemon, Some(1), Some(100), None);
    live.host.set_job(
        100,
        vec![ProcessRecord {
            pid: 101,
            started: 1,
            group: 101,
            foreground: 101,
        }],
    );
    send(&mut live.fixture, vec![address(session, pane)]);
    let mut state = live.fixture.core.begin(&Request {
        cmd: Command::Session(SessionCmd::Restart),
        target: Some(wire(session)),
        args: Some(Args {
            pane: Some("left".into()),
            command: Some("cat".into()),
            ..Args::default()
        }),
    });
    let mut step = 0;
    loop {
        match state {
            Dispatched::Done(response) => {
                assert!(response.ok, "{response:?}");
                break;
            }
            Dispatched::Pending { resume, .. } => {
                assert!(step < 10, "restart did not settle");
                if step == 0 {
                    assert_eq!(live.fixture.core.library.messages.pending(pane), 1);
                    send(&mut live.fixture, vec![address(session, pane)]);
                    assert_eq!(live.fixture.core.library.messages.pending(pane), 2);
                    live.host.finish_job_process(101);
                }
                if !live.fixture.core.take_pane_replacements().is_empty() {
                    assert_eq!(live.fixture.core.library.messages.pending(pane), 0);
                    live.host.set_record(&daemon, Some(1), Some(200), None);
                }
                step += 1;
                state = resume(&mut live.fixture.core);
            }
        }
    }
    assert_eq!(live.fixture.core.library.messages.pending(pane), 0);
}
