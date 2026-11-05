//! The presentation streams on the actor: attaching needs the machine's connection and its listing, the actor's
//! one-second tick drives staleness and retries on tokio's paused clock, and the ask, detach and disconnect commands
//! reach the streams. The actor's loop runs on the test's own `LocalSet`, so the paused clock is the actor's.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use ngterm_core::remote::client::STALE_AFTER;
use ngterm_core::remote::frames::{
    self, Body, PresentationAsk, PresentationAskAnswer, PresentationFrame, PresentationHello, PresentationMode,
    PresentationSnapshot,
};
use ngterm_core::remote::ssh::{present_tail, tree_tail};
use tokio::sync::{mpsc, oneshot};

use super::{Command, run};
use crate::api::{EventSink, Machine, MobileError, MobileEvent};
use crate::presentation::{AskReply, FLUSH_LIMIT, StreamState};
use crate::transport::fake::{FakeTransport, Script};
use crate::transport::{ChannelInput, ChannelPeer};

pub(super) const SESSION: &str = "00000000-0000-0000-0000-0000000000A1";

#[derive(Default)]
pub(super) struct Recorder(Mutex<Vec<MobileEvent>>);

impl EventSink for Recorder {
    fn on_event(&self, event: MobileEvent) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).push(event);
    }
}

impl Recorder {
    pub(super) fn take(&self) -> Vec<MobileEvent> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// The stream states reported since the last take.
    pub(super) fn states(&self) -> Vec<StreamState> {
        self.take()
            .into_iter()
            .filter_map(|event| match event {
                MobileEvent::SessionConnection { state, .. } => Some(state),
                _ => None,
            })
            .collect()
    }
}

pub(super) fn machine() -> Machine {
    Machine {
        id: "m1".to_owned(),
        label: "box".to_owned(),
        host: "box".to_owned(),
        port: 22,
        user: "nick".to_owned(),
        key_id: "key".to_owned(),
    }
}

/// A listing of the session, from an ngterm speaking presentation `version` when given.
pub(super) fn listing(version: Option<i64>) -> Script {
    let mut remote = serde_json::json!({
        "endpoint": {"executable": "/ngterm-session", "socketDirectory": "/tmp/ngterm-live-x"},
        "sessions": [{
            "id": SESSION, "name": "main", "windowID": "w1", "windowName": "main", "workspaceID": "ws1",
            "workspaceName": "work", "cwd": "/repo", "panes": [],
        }],
    });
    if let Some(version) = version {
        remote["presentation"] = version.into();
    }
    Script::ok(&serde_json::json!({"ok": true, "result": {"remote": remote}}).to_string())
}

/// The actor's loop on the test's `LocalSet`, its transport and its events.
pub(super) struct Actor {
    pub(super) commands: mpsc::UnboundedSender<Command>,
    pub(super) transport: FakeTransport,
    pub(super) sink: Arc<Recorder>,
}

impl Actor {
    pub(super) fn start() -> Self {
        let transport = FakeTransport::new();
        let sink = Arc::new(Recorder::default());
        let (commands, queue) = mpsc::unbounded_channel();
        let events: Arc<dyn EventSink> = Arc::clone(&sink) as Arc<dyn EventSink>;
        let wiring = super::Wiring {
            commands: commands.downgrade(),
            timing: crate::chat::input::InputTiming::default(),
        };
        tokio::task::spawn_local(run(transport.clone(), events, queue, wiring));
        Self {
            commands,
            transport,
            sink,
        }
    }

    pub(super) async fn connect(&self) {
        let (reply, answer) = oneshot::channel();
        assert!(
            self.commands
                .send(Command::Connect {
                    machine: machine(),
                    reply
                })
                .is_ok()
        );
        answer.await.unwrap().unwrap();
    }

    pub(super) async fn list(&self) {
        let (reply, answer) = oneshot::channel();
        let machine_id = "m1".to_owned();
        assert!(self.commands.send(Command::ListSessions { machine_id, reply }).is_ok());
        answer.await.unwrap().unwrap();
    }

    pub(super) async fn attach(&self) -> Result<(), MobileError> {
        let (reply, answer) = oneshot::channel();
        let command = Command::AttachSession {
            machine_id: "m1".to_owned(),
            session_id: SESSION.to_owned(),
            reply,
        };
        assert!(self.commands.send(command).is_ok());
        answer.await.unwrap()
    }

    pub(super) fn post(&self, command: Command) {
        assert!(self.commands.send(command).is_ok());
    }

    /// The stream's channel, opened since the last call.
    pub(super) async fn stream(&self) -> ChannelPeer {
        settle().await;
        let mut opened = self.transport.take_opened();
        assert_eq!(opened.len(), 1, "one stream opened");
        let opened = opened.remove(0);
        assert_eq!(opened.command, present_tail(SESSION).unwrap());
        opened.peer
    }
}

pub(super) async fn settle() {
    for _ in 0..32 {
        tokio::task::yield_now().await;
    }
}

/// What the core sent down `peer`: its frames' bodies, and whether it closed the channel.
pub(super) async fn received(peer: &mut ChannelPeer) -> (Vec<Body>, bool) {
    let (mut bodies, mut closed) = (Vec::new(), false);
    // A stopped link closes once what it wrote was taken: taking some lets more come.
    loop {
        settle().await;
        let mut took = false;
        while let Some(input) = peer.try_input() {
            took = true;
            match input {
                ChannelInput::Data(line) => {
                    bodies.push(frames::decode(line.strip_suffix(b"\n").unwrap()).unwrap().body);
                }
                ChannelInput::Close => closed = true,
                other => panic!("unexpected input {other:?}"),
            }
        }
        if !took {
            return (bodies, closed);
        }
    }
}

/// The origin's hello and snapshot, then `bodies`, in order.
pub(super) async fn deliver(peer: &ChannelPeer, bodies: Vec<Body>) {
    let hello = Body::Hello(PresentationHello {
        version: frames::VERSION,
        kinds: Vec::new(),
        mode: PresentationMode::Mirror,
    });
    let frames = [hello, Body::Snapshot(PresentationSnapshot::default())]
        .into_iter()
        .chain(bodies);
    for (rev, body) in (1..).zip(frames) {
        let line = frames::encode(&PresentationFrame {
            generation: 1,
            rev,
            body,
        })
        .unwrap();
        assert!(peer.data(line).await);
    }
    settle().await;
}

pub(super) fn local(test: impl Future<Output = ()>) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()
        .unwrap();
    tokio::task::LocalSet::new().block_on(&runtime, test);
}

#[test]
fn attaching_needs_the_connection_and_a_listing_holding_the_session() {
    local(async {
        let actor = Actor::start();
        actor.transport.script(&tree_tail(), listing(Some(1)));

        assert_eq!(
            actor.attach().await,
            Err(MobileError::NotConnected {
                machine_id: "m1".to_owned()
            })
        );
        actor.connect().await;
        assert_eq!(
            actor.attach().await,
            Err(MobileError::NotListed {
                session_id: SESSION.to_owned()
            })
        );
        actor.list().await;
        actor.sink.take();

        assert_eq!(actor.attach().await, Ok(()));
        let mut peer = actor.stream().await;
        assert_eq!(actor.sink.states(), [StreamState::Connecting]);
        assert!(matches!(received(&mut peer).await.0.as_slice(), [Body::Hello(_)]));
    });
}

#[test]
fn a_listing_from_an_ngterm_without_the_stream_attaches_unsupported() {
    local(async {
        let actor = Actor::start();
        actor.transport.script(&tree_tail(), listing(None));
        actor.connect().await;
        actor.list().await;
        actor.sink.take();

        assert_eq!(actor.attach().await, Ok(()));
        settle().await;

        assert_eq!(actor.sink.states(), [StreamState::Unsupported]);
        assert!(actor.transport.take_opened().is_empty());
    });
}

#[test]
fn the_actors_tick_drops_a_quiet_stream_and_reopens_it_on_its_backoff() {
    local(async {
        let actor = Actor::start();
        actor.transport.script(&tree_tail(), listing(Some(1)));
        actor.connect().await;
        actor.list().await;
        actor.attach().await.unwrap();
        let mut peer = actor.stream().await;
        deliver(&peer, Vec::new()).await;
        received(&mut peer).await;
        assert_eq!(actor.sink.states(), [StreamState::Connecting, StreamState::Connected]);

        tokio::time::sleep(STALE_AFTER).await;
        settle().await;
        assert!(actor.sink.states().is_empty(), "quiet, not stale yet");

        tokio::time::sleep(Duration::from_secs(1)).await;
        settle().await;
        let reason = format!("no frames for {} seconds", STALE_AFTER.as_secs());
        assert_eq!(actor.sink.states(), [StreamState::Failed { reason }]);
        assert!(received(&mut peer).await.1, "the quiet stream is closed");

        tokio::time::sleep(Duration::from_secs(1)).await;
        let mut peer = actor.stream().await;
        assert_eq!(actor.sink.states(), [StreamState::Connecting]);
        assert!(matches!(received(&mut peer).await.0.as_slice(), [Body::Hello(_)]));
    });
}

#[test]
fn an_answer_echoes_the_owner_and_detaching_or_disconnecting_ends_the_stream() {
    local(async {
        let actor = Actor::start();
        actor.transport.script(&tree_tail(), listing(Some(1)));
        actor.connect().await;
        actor.list().await;
        actor.attach().await.unwrap();
        let mut peer = actor.stream().await;
        let ask: PresentationAsk = serde_json::from_value(serde_json::json!({
            "id": "a1", "title": "Deploy?", "buttons": [{"id": "yes", "label": "Yes"}],
            "style": "terminal", "align": "right", "owner": 9,
        }))
        .unwrap();
        deliver(&peer, vec![Body::AskRequest(ask)]).await;
        received(&mut peer).await;
        assert!(
            actor
                .sink
                .take()
                .iter()
                .any(|event| matches!(event, MobileEvent::Ask { ask, .. } if ask.id == "a1"))
        );

        actor.post(Command::AnswerAsk {
            machine_id: "m1".to_owned(),
            session_id: SESSION.to_owned(),
            ask_id: "a1".to_owned(),
            reply: AskReply {
                button: Some("yes".to_owned()),
                ..AskReply::default()
            },
        });
        assert_eq!(
            received(&mut peer).await.0,
            [Body::AskResolve(PresentationAskAnswer {
                id: "a1".to_owned(),
                owner: 9,
                button: Some("yes".to_owned()),
                text: None,
                selected: None,
            })]
        );

        actor.post(Command::DetachSession {
            machine_id: "m1".to_owned(),
            session_id: SESSION.to_owned(),
        });
        assert!(received(&mut peer).await.1, "detaching closes the stream");
        assert!(actor.sink.take().is_empty(), "and reports nothing");

        actor.attach().await.unwrap();
        let mut peer = actor.stream().await;
        assert!(matches!(received(&mut peer).await.0.as_slice(), [Body::Hello(_)]));
        actor.post(Command::Disconnect {
            machine_id: "m1".to_owned(),
        });
        assert_eq!(
            received(&mut peer).await,
            (Vec::new(), false),
            "disconnecting closes the connection under the stream"
        );
        assert_eq!(actor.transport.closed(), ["m1"]);
        tokio::time::sleep(Duration::from_secs(60)).await;
        settle().await;
        assert!(actor.transport.take_opened().is_empty(), "and nothing reopens it");
    });
}

#[test]
fn an_answer_posted_just_before_a_suspend_is_written_before_the_connection_closes() {
    local(async {
        let actor = Actor::start();
        actor.transport.script(&tree_tail(), listing(Some(1)));
        actor.connect().await;
        actor.list().await;
        actor.attach().await.unwrap();
        let mut peer = actor.stream().await;
        let ask: PresentationAsk = serde_json::from_value(serde_json::json!({
            "id": "a1", "title": "Deploy?", "buttons": [{"id": "yes", "label": "Yes"}],
            "style": "terminal", "align": "right", "owner": 9,
        }))
        .unwrap();
        deliver(&peer, vec![Body::AskRequest(ask)]).await;
        received(&mut peer).await;
        actor.sink.take();

        actor.post(Command::AnswerAsk {
            machine_id: "m1".to_owned(),
            session_id: SESSION.to_owned(),
            ask_id: "a1".to_owned(),
            reply: AskReply {
                button: Some("yes".to_owned()),
                ..AskReply::default()
            },
        });
        actor.post(Command::Suspend);
        settle().await;
        assert!(
            actor.transport.closed().is_empty(),
            "the connection stays while the answer waits to be sent"
        );

        let Some(ChannelInput::Data(line)) = peer.try_input() else {
            panic!("the answer was queued first");
        };
        assert_eq!(
            frames::decode(line.strip_suffix(b"\n").unwrap()).unwrap().body,
            Body::AskResolve(PresentationAskAnswer {
                id: "a1".to_owned(),
                owner: 9,
                button: Some("yes".to_owned()),
                text: None,
                selected: None,
            }),
            "the answer reached the machine"
        );
        assert_eq!(
            received(&mut peer).await,
            (Vec::new(), true),
            "and the stream was closed once it was sent"
        );
        settle().await;
        assert_eq!(actor.transport.closed(), ["m1"], "then the connection");
    });
}

#[test]
fn a_suspend_waits_for_a_machine_that_takes_no_input_only_so_long() {
    local(async {
        let actor = Actor::start();
        actor.transport.script(&tree_tail(), listing(Some(1)));
        actor.connect().await;
        actor.list().await;
        actor.attach().await.unwrap();
        let peer = actor.stream().await;
        deliver(&peer, Vec::new()).await;
        actor.sink.take();

        // the hello the client sent back is never taken
        actor.post(Command::Suspend);
        settle().await;
        assert!(
            actor.transport.closed().is_empty(),
            "the connection waits for the hello"
        );
        tokio::time::sleep(FLUSH_LIMIT).await;
        settle().await;
        assert_eq!(actor.transport.closed(), ["m1"], "until the flush limit");
    });
}
