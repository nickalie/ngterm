//! Reconnect and lifecycle on the actor, on tokio's paused clock: a lost connection retried on `retry_delay`,
//! `network_changed` retrying at once, the attached sessions' streams kept across the loss on their own backoff and
//! retried at once when the machine is back, and suspend/resume.

use std::time::Duration;

use ngterm_core::remote::ssh::{present_tail, tree_tail};
use tokio::sync::oneshot;

use super::Command;
use super::presentation_tests::{Actor, SESSION, deliver, listing, local, machine, received, settle};
use crate::api::{MachineState, MobileError, MobileEvent};
use crate::presentation::{CONNECTION_DOWN, StreamState};
use crate::transport::ChannelPeer;
use crate::transport::fake::Script;
use crate::transport::russh::CONNECTION_LOST;

const OTHER: &str = "00000000-0000-0000-0000-0000000000B2";

/// The machine states reported among `events`.
fn machine_states(events: &[MobileEvent]) -> Vec<MachineState> {
    events
        .iter()
        .filter_map(|event| match event {
            MobileEvent::MachineState { state, .. } => Some(state.clone()),
            _ => None,
        })
        .collect()
}

/// `session_id`'s stream states among `events`.
fn stream_states(events: &[MobileEvent], session_id: &str) -> Vec<StreamState> {
    events
        .iter()
        .filter_map(|event| match event {
            MobileEvent::SessionConnection {
                session_id: id, state, ..
            } if id == session_id => Some(state.clone()),
            _ => None,
        })
        .collect()
}

fn reconnecting(next_in_ms: u64) -> MachineState {
    MachineState::Reconnecting { next_in_ms }
}

fn failed(reason: &str) -> StreamState {
    StreamState::Failed {
        reason: reason.to_owned(),
    }
}

/// Advances the paused clock by `millis` and lets the actor run what came due.
async fn advance(millis: u64) {
    tokio::time::sleep(Duration::from_millis(millis)).await;
    settle().await;
}

/// A listing of two sessions, both speaking the presentation stream.
fn two_sessions() -> Script {
    let session = |id: &str| {
        serde_json::json!({
            "id": id, "name": id, "windowID": "w1", "windowName": "main", "workspaceID": "ws1",
            "workspaceName": "work", "cwd": "/repo", "panes": [],
        })
    };
    let remote = serde_json::json!({
        "endpoint": {"executable": "/ngterm-session", "socketDirectory": "/tmp/ngterm-live-x"},
        "sessions": [session(SESSION), session(OTHER)],
        "presentation": 1,
    });
    Script::ok(&serde_json::json!({"ok": true, "result": {"remote": remote}}).to_string())
}

async fn attach(actor: &Actor, session_id: &str) {
    let (reply, answer) = oneshot::channel();
    actor.post(Command::AttachSession {
        machine_id: "m1".to_owned(),
        session_id: session_id.to_owned(),
        reply,
    });
    answer.await.unwrap().unwrap();
}

/// The commands of the channels opened since the last call.
fn opened(actor: &Actor) -> Vec<String> {
    actor
        .transport
        .take_opened()
        .into_iter()
        .map(|opened| opened.command)
        .collect()
}

/// A connected actor with both sessions attached and their streams up, every event taken, and the streams' channels
/// (kept open: a peer dropped ends its channel).
async fn attached_both() -> (Actor, Vec<ChannelPeer>) {
    let actor = Actor::start();
    actor.transport.script(&tree_tail(), two_sessions());
    actor.connect().await;
    actor.list().await;
    attach(&actor, SESSION).await;
    attach(&actor, OTHER).await;
    settle().await;
    let mut peers = Vec::new();
    for opened in actor.transport.take_opened() {
        deliver(&opened.peer, Vec::new()).await;
        peers.push(opened.peer);
    }
    actor.sink.take();
    (actor, peers)
}

#[test]
fn a_lost_connection_is_retried_on_retry_delay() {
    local(async {
        let actor = Actor::start();
        actor.connect().await;
        actor.sink.take();
        actor.transport.refuse("box", "no route to host");

        actor.transport.drop_connections("m1");
        settle().await;
        assert_eq!(machine_states(&actor.sink.take()), [reconnecting(1000)]);
        assert_eq!(actor.transport.connects().len(), 1, "it waits before the first retry");

        advance(999).await;
        assert_eq!(actor.transport.connects().len(), 1);
        advance(1).await;
        assert_eq!(actor.transport.connects().len(), 2, "retried after a second");
        assert_eq!(machine_states(&actor.sink.take()), [reconnecting(2000)]);

        advance(2000).await;
        assert_eq!(actor.transport.connects().len(), 3);
        assert_eq!(machine_states(&actor.sink.take()), [reconnecting(4000)]);

        actor.transport.accept("box");
        advance(3999).await;
        assert!(actor.sink.take().is_empty());
        advance(1).await;
        assert_eq!(actor.transport.connects().len(), 4);
        assert_eq!(machine_states(&actor.sink.take()), [MachineState::Connected]);

        // Connected again, it is watched again.
        actor.transport.drop_connections("m1");
        settle().await;
        assert_eq!(machine_states(&actor.sink.take()), [reconnecting(1000)]);
    });
}

#[test]
fn network_changed_retries_at_once_and_starts_the_backoff_over() {
    local(async {
        let actor = Actor::start();
        actor.connect().await;
        actor.transport.refuse("box", "no route to host");
        actor.transport.drop_connections("m1");
        advance(1000).await;
        advance(2000).await;
        actor.sink.take();
        assert_eq!(actor.transport.connects().len(), 3, "now waiting four seconds");

        actor.post(Command::NetworkChanged);
        settle().await;
        assert_eq!(actor.transport.connects().len(), 4, "retried at once");
        assert_eq!(
            machine_states(&actor.sink.take()),
            [reconnecting(0), reconnecting(1000)]
        );

        actor.transport.accept("box");
        actor.post(Command::NetworkChanged);
        settle().await;
        assert_eq!(
            machine_states(&actor.sink.take()),
            [reconnecting(0), MachineState::Connected]
        );

        // Nothing waits to reconnect any more: a network change does nothing.
        actor.post(Command::NetworkChanged);
        settle().await;
        assert_eq!(actor.transport.connects().len(), 5);
        assert!(actor.sink.take().is_empty());
    });
}

#[test]
fn a_connect_of_a_machine_waiting_to_reconnect_retries_at_once_and_is_answered() {
    local(async {
        let actor = Actor::start();
        actor.connect().await;
        actor.transport.drop_connections("m1");
        settle().await;
        actor.sink.take();

        actor.connect().await;
        assert_eq!(actor.transport.connects().len(), 2);
        assert_eq!(
            machine_states(&actor.sink.take()),
            [reconnecting(0), MachineState::Connected]
        );
    });
}

#[test]
fn a_disconnect_ends_the_retries_and_the_streams() {
    local(async {
        let (actor, _peers) = attached_both().await;
        actor.transport.refuse("box", "no route to host");
        actor.transport.drop_connections("m1");
        settle().await;

        actor.post(Command::Disconnect {
            machine_id: "m1".to_owned(),
        });
        settle().await;
        actor.sink.take();
        advance(60_000).await;
        assert_eq!(actor.transport.connects().len(), 1, "no retry after the disconnect");
        assert!(actor.sink.take().is_empty(), "and no stream retries either");
    });
}

#[test]
fn while_the_connection_is_down_links_close_at_once_and_each_stream_keeps_its_own_backoff() {
    local(async {
        let (actor, _peers) = attached_both().await;
        actor.transport.refuse("box", "no route to host");

        actor.transport.drop_connections("m1");
        settle().await;
        let events = actor.sink.take();
        assert_eq!(stream_states(&events, SESSION), [failed(CONNECTION_LOST)]);
        assert_eq!(stream_states(&events, OTHER), [failed(CONNECTION_LOST)]);

        // The streams' retries run on the actor's tick: a second after the loss, then two, then four.
        let retried = [StreamState::Connecting, failed(CONNECTION_DOWN)];
        advance(1500).await;
        assert_eq!(stream_states(&actor.sink.take(), SESSION), retried);
        advance(1000).await;
        assert!(stream_states(&actor.sink.take(), SESSION).is_empty());
        advance(1000).await;
        assert_eq!(stream_states(&actor.sink.take(), SESSION), retried);

        // The machine's own retry, sped up by a network change, leaves the streams' backoff alone.
        actor.post(Command::NetworkChanged);
        advance(900).await;
        let events = actor.sink.take();
        assert_eq!(machine_states(&events), [reconnecting(0), reconnecting(1000)]);
        assert!(stream_states(&events, SESSION).is_empty());
        advance(2000).await;
        assert!(stream_states(&actor.sink.take(), SESSION).is_empty());
        advance(1000).await;
        let events = actor.sink.take();
        assert_eq!(stream_states(&events, SESSION), retried);
        assert_eq!(stream_states(&events, OTHER), retried);

        assert!(opened(&actor).is_empty(), "no channel opens while the machine is down");
    });
}

#[test]
fn on_reconnect_every_stream_retries_at_once() {
    local(async {
        let (actor, _peers) = attached_both().await;
        actor.transport.refuse("box", "no route to host");
        actor.transport.drop_connections("m1");
        // The streams' last retries were at 1, 3, 7 and 15 seconds: the next is sixteen seconds off.
        advance(15_500).await;
        actor.sink.take();

        actor.transport.accept("box");
        actor.post(Command::NetworkChanged);
        settle().await;

        let events = actor.sink.take();
        assert_eq!(machine_states(&events), [reconnecting(0), MachineState::Connected]);
        assert_eq!(stream_states(&events, SESSION), [StreamState::Connecting]);
        assert_eq!(stream_states(&events, OTHER), [StreamState::Connecting]);
        let tail = |id: &str| present_tail(id).unwrap();
        let mut commands = opened(&actor);
        commands.sort();
        let mut expected = vec![tail(SESSION), tail(OTHER)];
        expected.sort();
        assert_eq!(commands, expected, "both streams reopen on the new connection");
    });
}

#[test]
fn suspend_closes_quietly_and_resume_reattaches_everything() {
    local(async {
        let actor = Actor::start();
        actor.transport.script(&tree_tail(), listing(Some(1)));
        actor.connect().await;
        actor.list().await;
        actor.attach().await.unwrap();
        let mut peer = actor.stream().await;
        deliver(&peer, Vec::new()).await;
        received(&mut peer).await;
        actor.sink.take();

        actor.post(Command::Suspend);
        settle().await;
        assert!(received(&mut peer).await.1, "the stream is closed");
        assert_eq!(actor.transport.closed(), ["m1"], "and the connection");
        assert!(actor.sink.take().is_empty(), "with no state reported");
        advance(600_000).await;
        assert!(actor.sink.take().is_empty(), "nothing retries while suspended");
        assert_eq!(actor.transport.connects().len(), 1);

        actor.post(Command::Resume);
        settle().await;
        let events = actor.sink.take();
        assert_eq!(machine_states(&events), [reconnecting(0), MachineState::Connected]);
        assert_eq!(stream_states(&events, SESSION), [StreamState::Connecting]);
        let peer = actor.stream().await;
        deliver(&peer, Vec::new()).await;
        assert_eq!(actor.sink.states(), [StreamState::Connected]);
    });
}

#[test]
fn suspend_cancels_a_connect_in_flight_and_resume_connects_it() {
    local(async {
        let actor = Actor::start();
        actor.transport.hold("box");
        let (reply, answer) = oneshot::channel();
        actor.post(Command::Connect {
            machine: machine(),
            reply,
        });
        settle().await;

        actor.post(Command::Suspend);
        assert_eq!(answer.await.unwrap(), Err(MobileError::Cancelled));
        actor.transport.release("box");
        settle().await;
        assert!(actor.sink.take().is_empty());

        actor.post(Command::Resume);
        settle().await;
        assert_eq!(
            machine_states(&actor.sink.take()),
            [reconnecting(0), MachineState::Connected]
        );
    });
}

#[test]
fn a_failed_resume_keeps_reconnecting() {
    local(async {
        let actor = Actor::start();
        actor.connect().await;
        actor.post(Command::Suspend);
        actor.transport.refuse("box", "no route to host");
        actor.post(Command::Resume);
        settle().await;
        assert_eq!(
            machine_states(&actor.sink.take()),
            [MachineState::Connected, reconnecting(0), reconnecting(1000)]
        );
    });
}

#[test]
fn a_disconnect_while_suspended_forgets_the_machine() {
    local(async {
        let actor = Actor::start();
        actor.connect().await;
        actor.post(Command::Suspend);
        actor.post(Command::Disconnect {
            machine_id: "m1".to_owned(),
        });
        actor.post(Command::Resume);
        settle().await;
        assert_eq!(machine_states(&actor.sink.take()), [MachineState::Connected]);
        assert_eq!(actor.transport.connects().len(), 1);
    });
}
