//! The presentation streams over the scripted transport, one effect at a time: each [`ClientEffect`] variant and the
//! event (or answer) it makes, the adapter's line splitting and launches, and the tick's staleness and retries. The
//! clock is the harness's own `now`, passed in as the actor passes its clock.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use ngterm_core::hud::HudSpec;
use ngterm_core::model::{AgentStatus, AskAlignment, AskStyle};
use ngterm_core::remote::client::{PAGES_UNSUPPORTED, STALE_AFTER, viewer_kinds};
use ngterm_core::remote::frames::{
    self, Body, PresentationAsk, PresentationAskRef, PresentationFrame, PresentationHello, PresentationHud,
    PresentationLayout, PresentationMode, PresentationNotify, PresentationOverlay, PresentationOverlayChange,
    PresentationPage, PresentationPageChange, PresentationPageSource, PresentationRespawns, PresentationSnapshot,
    PresentationSound, PresentationStatus,
};
use ngterm_core::remote::hub::AGENT_KIND;
use ngterm_core::remote::ssh::present_tail;
use ngterm_proto::request::AskButton;
use uuid::Uuid;

use super::*;
use crate::transport::fake::{FakeConnection, FakeTransport};
use crate::transport::{ChannelEnd, ChannelInput, ChannelPeer, Transport};

const MACHINE: &str = "m1";
const SESSION: &str = "6F9619FF-8B86-D011-B42D-00C04FC964FF";
const PANE: &str = "0E984725-C51C-4BF4-9960-E1C80E27ABA0";
const GENERATION: i64 = 7;

/// Runs `test` on a `LocalSet`, as the actor runs the streams.
fn local(test: impl Future<Output = ()>) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    tokio::task::LocalSet::new().block_on(&runtime, test);
}

/// Lets the links' tasks run until they wait again.
async fn settle() {
    for _ in 0..32 {
        tokio::task::yield_now().await;
    }
}

/// What a link's peer received from the core.
#[derive(Debug, Default)]
struct Received {
    bodies: Vec<Body>,
    closed: bool,
}

/// One machine's presentations, its fake connection, the events they made and the stream's far end.
struct Harness {
    fake: FakeTransport,
    connection: Rc<FakeConnection>,
    presentations: Rc<RefCell<Presentations<FakeConnection>>>,
    events: Rc<RefCell<Vec<MobileEvent>>>,
    now: Rc<Cell<Instant>>,
    /// The key of each report the links made, in order.
    keys: Rc<RefCell<Vec<LinkKey>>>,
    peer: Option<ChannelPeer>,
    rev: i64,
}

impl Harness {
    async fn new() -> Self {
        let fake = FakeTransport::new();
        let machine = crate::api::Machine {
            id: MACHINE.to_owned(),
            label: "box".to_owned(),
            host: "box".to_owned(),
            port: 22,
            user: "nick".to_owned(),
            key_id: "key".to_owned(),
        };
        let connection = Rc::new(fake.connect(&machine).await.unwrap());
        Self {
            fake,
            connection,
            presentations: Rc::new(RefCell::new(Presentations::default())),
            events: Rc::new(RefCell::new(Vec::new())),
            now: Rc::new(Cell::new(Instant::now())),
            keys: Rc::new(RefCell::new(Vec::new())),
            peer: None,
            rev: 0,
        }
    }

    fn links(&self) -> ChannelLinks<FakeConnection> {
        let (presentations, events, now, keys) = (
            Rc::clone(&self.presentations),
            Rc::clone(&self.events),
            Rc::clone(&self.now),
            Rc::clone(&self.keys),
        );
        let report: Reporter = Rc::new(move |key, event| {
            keys.borrow_mut().push(key);
            let made = presentations.borrow_mut().link_event(key, event, now.get());
            events.borrow_mut().extend(made);
        });
        ChannelLinks::new(Rc::clone(&self.connection), report)
    }

    fn attach(&self, version: Option<i64>) -> Result<(), MobileError> {
        let made = self
            .presentations
            .borrow_mut()
            .attach(MACHINE, SESSION, version, self.links(), self.now.get())?;
        self.events.borrow_mut().extend(made);
        Ok(())
    }

    /// Attaches the session and takes the stream's channel.
    async fn attached(version: Option<i64>) -> Self {
        let mut harness = Self::new().await;
        harness.attach(version).unwrap();
        harness.open().await;
        harness
    }

    /// Takes the one channel opened since the last call.
    async fn open(&mut self) {
        settle().await;
        let mut opened = self.fake.take_opened();
        assert_eq!(opened.len(), 1, "one stream opened");
        let opened = opened.remove(0);
        assert_eq!(
            opened.command,
            present_tail(ngterm_core::remote::origin::OriginShell::Posix, SESSION).unwrap()
        );
        assert_eq!(opened.pty, None);
        self.peer = Some(opened.peer);
        self.rev = 0;
    }

    /// What the core sent down the stream since the last call.
    async fn received(&mut self) -> Received {
        let mut received = Received::default();
        // A stopped link closes once what it wrote was taken: taking some lets more come.
        loop {
            settle().await;
            let peer = self.peer.as_mut().expect("a stream");
            let mut took = false;
            while let Some(input) = peer.try_input() {
                took = true;
                match input {
                    ChannelInput::Data(line) => {
                        let line = line.strip_suffix(b"\n").expect("a whole line");
                        received.bodies.push(frames::decode(line).unwrap().body);
                    }
                    ChannelInput::Close => received.closed = true,
                    other => panic!("unexpected input {other:?}"),
                }
            }
            if !took {
                return received;
            }
        }
    }

    /// The origin sends `bytes`; a stream the core has closed reads none of them.
    async fn raw(&mut self, bytes: &[u8]) {
        let _ = self.peer.as_ref().expect("a stream").data(bytes.to_vec()).await;
        settle().await;
    }

    /// The origin sends `body`, next in order.
    async fn deliver(&mut self, body: Body) {
        self.rev += 1;
        let line = frames::encode(&PresentationFrame {
            generation: GENERATION,
            rev: self.rev,
            body,
        })
        .unwrap();
        self.raw(&line).await;
    }

    /// The origin's hello offering `mode`, then its snapshot.
    async fn connect(&mut self, mode: PresentationMode, snapshot: PresentationSnapshot) {
        self.deliver(Body::Hello(PresentationHello {
            version: frames::VERSION,
            kinds: Vec::new(),
            mode,
        }))
        .await;
        self.deliver(Body::Snapshot(snapshot)).await;
    }

    async fn end(&mut self, end: ChannelEnd) {
        self.peer.take().expect("a stream").end(end).await;
        settle().await;
    }

    /// The key of the latest report.
    fn last_key(&self) -> LinkKey {
        *self.keys.borrow().last().expect("a report")
    }

    fn take_events(&self) -> Vec<MobileEvent> {
        std::mem::take(&mut *self.events.borrow_mut())
    }

    async fn tick_after(&self, elapsed: Duration) {
        self.now.set(self.now.get() + elapsed);
        let made = self.presentations.borrow_mut().tick(self.now.get());
        self.events.borrow_mut().extend(made);
        settle().await;
    }
}

fn connection(state: StreamState) -> MobileEvent {
    MobileEvent::SessionConnection {
        machine_id: MACHINE.to_owned(),
        session_id: SESSION.to_owned(),
        state,
    }
}

fn role(role: PresentationRole) -> MobileEvent {
    MobileEvent::SessionMode {
        machine_id: MACHINE.to_owned(),
        session_id: SESSION.to_owned(),
        role,
    }
}

fn failed(reason: &str) -> MobileEvent {
    connection(StreamState::Failed {
        reason: reason.to_owned(),
    })
}

fn blocked() -> PresentationStatus {
    PresentationStatus {
        status: AgentStatus::Blocked,
        blink: true,
        color: Some("#ff0000".to_owned()),
        shape: None,
        pane: None,
        changed_at: Some(12.5),
    }
}

fn remote_blocked() -> RemoteStatus {
    RemoteStatus {
        status: "blocked".to_owned(),
        blink: true,
        color: Some("#ff0000".to_owned()),
        shape: None,
        pane: None,
        changed_at: Some(12.5),
    }
}

fn status_event(status: Option<RemoteStatus>, changed: bool) -> MobileEvent {
    MobileEvent::SessionStatus {
        machine_id: MACHINE.to_owned(),
        session_id: SESSION.to_owned(),
        status,
        changed,
    }
}

fn layout() -> PresentationLayout {
    let pane = Uuid::parse_str(PANE).unwrap();
    PresentationLayout {
        panes: vec![pane],
        primary: pane,
        axis: None,
        shown: false,
    }
}

fn hud() -> PresentationHud {
    PresentationHud {
        spec: HudSpec::new("Building"),
        pane: None,
        generation: 3,
        remaining: Some(4.0),
    }
}

fn ask(id: &str, owner: i64) -> PresentationAsk {
    PresentationAsk {
        id: id.to_owned(),
        title: "Deploy?".to_owned(),
        message: Some("to production".to_owned()),
        buttons: vec![
            AskButton {
                id: "yes".to_owned(),
                label: "Yes".to_owned(),
                hotkey: Some("y".to_owned()),
            },
            AskButton {
                id: "no".to_owned(),
                label: "No".to_owned(),
                hotkey: None,
            },
        ],
        default_id: Some("yes".to_owned()),
        destructive_id: None,
        style: AskStyle::Gui,
        align: AskAlignment::Center,
        width: None,
        pane: None,
        owner,
        input: None,
        choices: None,
        cancel_id: Some("no".to_owned()),
    }
}

#[test]
fn attaching_opens_the_stream_with_a_hello_listing_no_page_kinds_and_the_agent_kind() {
    local(async {
        let mut harness = Harness::attached(Some(1)).await;

        assert_eq!(
            harness.take_events(),
            [role(PresentationRole::Mirror), connection(StreamState::Connecting)]
        );
        let received = harness.received().await;
        let mut kinds = viewer_kinds(false);
        kinds.push(AGENT_KIND.to_owned());
        assert_eq!(
            received.bodies,
            [Body::Hello(PresentationHello {
                version: frames::VERSION,
                kinds,
                mode: PresentationMode::Presenter,
            })]
        );
        assert!(!received.closed);
        // attaching again changes nothing
        harness.attach(Some(1)).unwrap();
        settle().await;
        assert!(harness.fake.take_opened().is_empty());
        assert!(harness.take_events().is_empty());
    });
}

#[test]
fn an_origin_without_the_stream_is_unsupported_and_never_opened() {
    local(async {
        let harness = Harness::new().await;
        harness.attach(None).unwrap();
        settle().await;

        assert_eq!(harness.take_events(), [connection(StreamState::Unsupported)]);
        assert!(harness.fake.execs().is_empty());
    });
}

#[test]
fn an_id_that_is_not_a_session_is_refused() {
    local(async {
        let harness = Harness::new().await;
        let made = harness.presentations.borrow_mut().attach(
            MACHINE,
            "not-a-session",
            Some(1),
            harness.links(),
            harness.now.get(),
        );
        assert_eq!(
            made.err(),
            Some(MobileError::Remote {
                reason: INVALID_REMOTE_SESSION.to_owned()
            })
        );
    });
}

#[test]
fn the_snapshot_connects_and_its_status_is_no_change() {
    local(async {
        let mut harness = Harness::attached(Some(1)).await;
        harness.take_events();
        let snapshot = PresentationSnapshot {
            status: Some(blocked()),
            hud: Some(hud()),
            context: Some("fixing the build".to_owned()),
            info: None,
            layout: Some(layout()),
            name: Some("Fix Linux".to_owned()),
            respawns: Some(PresentationRespawns {
                epoch: "e1".to_owned(),
                generations: std::collections::BTreeMap::new(),
            }),
        };
        harness.connect(PresentationMode::Mirror, snapshot).await;

        let (machine_id, session_id) = (MACHINE.to_owned(), SESSION.to_owned());
        assert_eq!(
            harness.take_events(),
            [
                connection(StreamState::Connected),
                MobileEvent::SessionLayout {
                    machine_id: machine_id.clone(),
                    session_id: session_id.clone(),
                    layout: RemoteLayout {
                        panes: vec![PANE.to_owned()],
                        primary: PANE.to_owned(),
                        axis: None,
                        shown: false,
                    },
                },
                status_event(Some(remote_blocked()), false),
                MobileEvent::Hud {
                    machine_id: machine_id.clone(),
                    session_id: session_id.clone(),
                    hud: Some(RemoteHud::from(hud())),
                },
                MobileEvent::SessionContext {
                    machine_id: machine_id.clone(),
                    session_id: session_id.clone(),
                    context: Some("fixing the build".to_owned()),
                },
                MobileEvent::SessionName {
                    machine_id,
                    session_id,
                    name: "Fix Linux".to_owned(),
                },
            ]
        );
    });
}

#[test]
fn deltas_reach_the_app_as_their_events() {
    local(async {
        let mut harness = Harness::attached(Some(1)).await;
        harness
            .connect(PresentationMode::Mirror, PresentationSnapshot::default())
            .await;
        harness.take_events();
        let (machine_id, session_id) = (MACHINE.to_owned(), SESSION.to_owned());

        harness.deliver(Body::Status(Some(blocked()))).await;
        harness.deliver(Body::Context(None)).await;
        harness.deliver(Body::Layout(layout())).await;
        harness.deliver(Body::Hud(None)).await;
        let notify = PresentationNotify {
            title: "Done".to_owned(),
            body: "tests pass".to_owned(),
            pane: Some(frames::PresentationPane::Scratch),
            source: "agent".to_owned(),
        };
        harness.deliver(Body::Notify(notify)).await;
        harness.deliver(Body::Sound(PresentationSound { name: None })).await;
        harness.deliver(Body::Name("Auto Update".to_owned())).await;

        let events = harness.take_events();
        assert_eq!(events[0], status_event(Some(remote_blocked()), true));
        assert_eq!(
            events[1],
            MobileEvent::SessionContext {
                machine_id: machine_id.clone(),
                session_id: session_id.clone(),
                context: None,
            }
        );
        assert!(matches!(&events[2], MobileEvent::SessionLayout { layout, .. } if layout.primary == PANE));
        assert_eq!(
            events[3],
            MobileEvent::Hud {
                machine_id: machine_id.clone(),
                session_id: session_id.clone(),
                hud: None,
            }
        );
        assert_eq!(
            events[4],
            MobileEvent::Notify {
                machine_id: machine_id.clone(),
                session_id: session_id.clone(),
                notice: RemoteNotice {
                    title: "Done".to_owned(),
                    body: "tests pass".to_owned(),
                    pane: Some("scratch".to_owned()),
                    source: "agent".to_owned(),
                },
            }
        );
        assert_eq!(
            events[5],
            MobileEvent::Sound {
                machine_id: machine_id.clone(),
                session_id: session_id.clone(),
                name: None,
            }
        );
        assert_eq!(
            events[6],
            MobileEvent::SessionName {
                machine_id,
                session_id,
                name: "Auto Update".to_owned(),
            }
        );
        assert_eq!(events.len(), 7);
        // nothing went back but the hello
        assert_eq!(harness.received().await.bodies.len(), 1);
    });
}

#[test]
fn an_origin_offering_the_role_is_asked_and_a_grant_or_refusal_sets_it() {
    local(async {
        let mut harness = Harness::attached(Some(1)).await;
        harness.received().await;
        harness
            .connect(PresentationMode::Presenter, PresentationSnapshot::default())
            .await;
        assert_eq!(harness.received().await.bodies, [Body::PresenterAcquire]);
        harness.take_events();

        harness.deliver(Body::PresenterGranted).await;
        assert_eq!(harness.take_events(), [role(PresentationRole::Presenter)]);
        harness.deliver(Body::PresenterRefused).await;
        assert_eq!(harness.take_events(), [role(PresentationRole::Mirror)]);
    });
}

#[test]
fn an_ask_is_accepted_and_its_answer_echoes_the_owner() {
    local(async {
        let mut harness = Harness::attached(Some(1)).await;
        harness
            .connect(PresentationMode::Mirror, PresentationSnapshot::default())
            .await;
        harness.received().await;
        harness.take_events();

        harness.deliver(Body::AskRequest(ask("a1", 42))).await;
        let events = harness.take_events();
        let [MobileEvent::Ask { ask: shown, .. }] = events.as_slice() else {
            panic!("one ask: {events:?}");
        };
        assert_eq!(shown.id, "a1");
        assert_eq!((shown.style.as_str(), shown.align.as_str()), ("gui", "center"));
        assert_eq!(shown.buttons[0].hotkey.as_deref(), Some("y"));
        assert_eq!(shown.cancel_id.as_deref(), Some("no"));
        // nothing is refused
        assert!(harness.received().await.bodies.is_empty());

        let reply = AskReply {
            button: Some("yes".to_owned()),
            ..AskReply::default()
        };
        assert!(
            harness
                .presentations
                .borrow_mut()
                .answer_ask(MACHINE, SESSION, "a1", reply.clone())
        );
        assert_eq!(
            harness.received().await.bodies,
            [Body::AskResolve(frames::PresentationAskAnswer {
                id: "a1".to_owned(),
                owner: 42,
                button: Some("yes".to_owned()),
                text: None,
                selected: None,
            })]
        );
        // answered once
        assert!(
            !harness
                .presentations
                .borrow_mut()
                .answer_ask(MACHINE, SESSION, "a1", reply)
        );
    });
}

#[test]
fn a_dismissed_ask_is_taken_down_and_no_longer_answered() {
    local(async {
        let mut harness = Harness::attached(Some(1)).await;
        harness
            .connect(PresentationMode::Mirror, PresentationSnapshot::default())
            .await;
        harness.received().await;
        harness.deliver(Body::AskRequest(ask("a1", 42))).await;
        harness.take_events();

        harness
            .deliver(Body::AskDismiss(PresentationAskRef {
                id: "a1".to_owned(),
                owner: 42,
            }))
            .await;
        assert_eq!(
            harness.take_events(),
            [MobileEvent::AskDismiss {
                machine_id: MACHINE.to_owned(),
                session_id: SESSION.to_owned(),
                ask_id: "a1".to_owned(),
            }]
        );
        // a dismissal of an ask not shown says nothing
        harness
            .deliver(Body::AskDismiss(PresentationAskRef {
                id: "a9".to_owned(),
                owner: 1,
            }))
            .await;
        assert!(harness.take_events().is_empty());
        assert!(
            !harness
                .presentations
                .borrow_mut()
                .answer_ask(MACHINE, SESSION, "a1", AskReply::default())
        );
        assert!(harness.received().await.bodies.is_empty());
    });
}

#[test]
fn an_overlay_is_refused_and_its_closes_change_nothing() {
    local(async {
        let mut harness = Harness::attached(Some(1)).await;
        harness
            .connect(PresentationMode::Mirror, PresentationSnapshot::default())
            .await;
        harness.received().await;
        harness.take_events();

        harness
            .deliver(Body::OverlayRequest(PresentationOverlay {
                width: None,
                height: None,
                job: "j1".to_owned(),
                pane: None,
                size_percent: None,
                background_color: None,
                follow: false,
                wait: true,
            }))
            .await;
        harness
            .deliver(Body::OverlayResize(PresentationOverlayChange {
                width: None,
                height: None,
                job: "j1".to_owned(),
                size_percent: Some(50),
            }))
            .await;
        harness
            .deliver(Body::OverlayClose(PresentationOverlayChange::job("j1")))
            .await;

        assert!(harness.take_events().is_empty());
        assert_eq!(
            harness.received().await.bodies,
            [Body::OverlayRejected(PresentationOverlayChange::job("j1"))]
        );
    });
}

#[test]
fn a_page_is_refused_as_unsupported_and_its_closes_change_nothing() {
    local(async {
        let mut harness = Harness::attached(Some(1)).await;
        harness
            .connect(PresentationMode::Mirror, PresentationSnapshot::default())
            .await;
        harness.received().await;
        harness.take_events();

        harness
            .deliver(Body::PageRequest(PresentationPage {
                width: None,
                height: None,
                page: "p1".to_owned(),
                token: "t".to_owned(),
                source: PresentationPageSource::Url {
                    address: "https://example.com".to_owned(),
                },
                identity: "example.com".to_owned(),
                chromeless: false,
                javascript: true,
                navigation: true,
                browse: false,
                persistent: false,
                background_color: None,
                pane: None,
                size_percent: None,
                follow: false,
            }))
            .await;
        harness
            .deliver(Body::PageClose(PresentationPageChange::page("p1")))
            .await;

        assert!(harness.take_events().is_empty());
        assert_eq!(
            harness.received().await.bodies,
            [Body::PageRejected(PresentationPageChange {
                page: "p1".to_owned(),
                reason: Some(PAGES_UNSUPPORTED.to_owned()),
            })]
        );
    });
}

#[test]
fn respawn_counts_are_handed_to_the_panes_and_warnings_make_no_event() {
    local(async {
        let mut harness = Harness::attached(Some(1)).await;
        let counts = |generation: i64| PresentationRespawns {
            epoch: "e1".to_owned(),
            generations: std::collections::BTreeMap::from([(PANE.to_owned(), generation)]),
        };
        let snapshot = PresentationSnapshot {
            respawns: Some(counts(1)),
            ..PresentationSnapshot::default()
        };
        harness.connect(PresentationMode::Mirror, snapshot).await;
        harness.take_events();

        harness.deliver(Body::Respawn(counts(2))).await;
        // an invalid layout is only a warning
        harness.deliver(Body::Layout(PresentationLayout::invalid())).await;

        assert!(harness.take_events().is_empty());
        let respawns = |generation: i64, seed: bool| Respawns {
            machine_id: MACHINE.to_owned(),
            session_id: SESSION.to_owned(),
            counts: counts(generation),
            seed,
        };
        assert_eq!(
            harness.presentations.borrow_mut().take_respawns(),
            [respawns(1, true), respawns(2, false)],
            "the snapshot's counts seed, the frame's follow"
        );
        assert!(
            harness.presentations.borrow_mut().take_respawns().is_empty(),
            "each handed on once"
        );
    });
}

#[test]
fn a_ping_is_acked() {
    local(async {
        let mut harness = Harness::attached(Some(1)).await;
        harness
            .connect(PresentationMode::Mirror, PresentationSnapshot::default())
            .await;
        harness.received().await;

        harness.deliver(Body::Ping).await;
        assert_eq!(harness.received().await.bodies, [Body::Ack]);
    });
}

#[test]
fn lines_are_split_across_chunks_and_a_partial_line_at_the_end_is_dropped() {
    local(async {
        let mut harness = Harness::attached(Some(1)).await;
        harness.take_events();
        let hello = frames::encode(&PresentationFrame {
            generation: GENERATION,
            rev: 1,
            body: Body::Hello(PresentationHello {
                version: frames::VERSION,
                kinds: Vec::new(),
                mode: PresentationMode::Mirror,
            }),
        })
        .unwrap();
        let snapshot = frames::encode(&PresentationFrame {
            generation: GENERATION,
            rev: 2,
            body: Body::Snapshot(PresentationSnapshot::default()),
        })
        .unwrap();
        let mut bytes = hello;
        bytes.extend_from_slice(&snapshot);
        let (first, rest) = bytes.split_at(bytes.len() - 5);
        harness.raw(first).await;
        assert!(harness.take_events().is_empty(), "the snapshot is not whole yet");
        harness.raw(rest).await;
        assert_eq!(harness.take_events()[0], connection(StreamState::Connected));

        harness.raw(b"{\"partial").await;
        harness
            .end(ChannelEnd::Exit {
                code: Some(255),
                signal: None,
            })
            .await;
        let events = harness.take_events();
        assert_eq!(events[0], failed("exit 255"), "the partial line is not a bad frame");
    });
}

#[test]
fn an_oversize_line_ends_the_stream() {
    local(async {
        let mut harness = Harness::attached(Some(1)).await;
        harness.received().await;
        harness.take_events();

        harness.raw(&vec![b'x'; MAX_FRAME_BYTES + 1]).await;

        assert!(harness.received().await.closed);
        assert_eq!(harness.take_events()[0], failed(OVERSIZE_LINE));
    });
}

#[test]
fn an_undecodable_line_drops_the_stream_and_it_reopens_after_its_backoff() {
    local(async {
        let mut harness = Harness::attached(Some(1)).await;
        harness.received().await;
        harness.take_events();

        harness.raw(b"not a frame\n").await;
        let first = harness.last_key();
        assert!(harness.received().await.closed);
        assert_eq!(
            harness.take_events(),
            [failed("bad frame")],
            "the role was already mirror"
        );

        harness.tick_after(Duration::from_millis(500)).await;
        assert!(harness.fake.take_opened().is_empty(), "within the backoff");
        harness.tick_after(Duration::from_millis(600)).await;
        harness.open().await;
        assert_eq!(harness.take_events(), [connection(StreamState::Connecting)]);
        assert!(matches!(harness.received().await.bodies.as_slice(), [Body::Hello(_)]));

        // The second launch's frames reach the client under that launch's key.
        harness
            .connect(PresentationMode::Mirror, PresentationSnapshot::default())
            .await;
        let second = harness.last_key();
        assert_eq!((second.session, second.client), (first.session, first.client));
        assert_eq!(second.launch, first.launch + 1);
        assert_eq!(harness.take_events()[0], connection(StreamState::Connected));
        harness.received().await;

        // A late report of the first launch changes nothing.
        let late = frames::encode(&PresentationFrame {
            generation: GENERATION,
            rev: harness.rev + 1,
            body: Body::Snapshot(PresentationSnapshot {
                context: Some("stale".to_owned()),
                ..PresentationSnapshot::default()
            }),
        })
        .unwrap();
        let late = late.strip_suffix(b"\n").unwrap().to_vec();
        let now = harness.now.get();
        let presentations = &harness.presentations;
        assert!(
            presentations
                .borrow_mut()
                .link_event(first, LinkEvent::Line(late), now)
                .is_empty()
        );
        let closed = LinkEvent::Closed("exit 1".to_owned());
        assert!(presentations.borrow_mut().link_event(first, closed, now).is_empty());
        harness.deliver(Body::Ping).await;
        assert!(harness.take_events().is_empty(), "the second launch is still connected");
        assert_eq!(
            harness.received().await.bodies,
            [Body::Ack],
            "the second launch still answers"
        );
    });
}

#[test]
fn a_quiet_stream_goes_stale_and_reconnects() {
    local(async {
        let mut harness = Harness::attached(Some(1)).await;
        harness
            .connect(PresentationMode::Mirror, PresentationSnapshot::default())
            .await;
        harness.received().await;
        harness.take_events();

        harness.tick_after(STALE_AFTER).await;
        assert!(harness.take_events().is_empty(), "not quiet for longer yet");
        harness.deliver(Body::Ping).await;
        harness.tick_after(STALE_AFTER).await;
        assert!(harness.take_events().is_empty(), "a ping keeps it fresh");

        harness.tick_after(Duration::from_secs(1)).await;
        let reason = format!("no frames for {} seconds", STALE_AFTER.as_secs());
        let events = harness.take_events();
        assert_eq!(events[0], failed(&reason));
        assert!(harness.received().await.closed);

        harness.tick_after(Duration::from_secs(1)).await;
        harness.open().await;
        assert_eq!(harness.take_events(), [connection(StreamState::Connecting)]);
    });
}

#[test]
fn leaving_connected_dismisses_open_asks_and_clears_what_the_stream_showed() {
    local(async {
        let mut harness = Harness::attached(Some(1)).await;
        harness
            .connect(PresentationMode::Mirror, PresentationSnapshot::default())
            .await;
        harness.deliver(Body::AskRequest(ask("a1", 42))).await;
        harness.take_events();

        harness.end(ChannelEnd::Lost("connection lost".to_owned())).await;

        let (machine_id, session_id) = (MACHINE.to_owned(), SESSION.to_owned());
        assert_eq!(
            harness.take_events(),
            [
                failed("connection lost"),
                MobileEvent::AskDismiss {
                    machine_id: machine_id.clone(),
                    session_id: session_id.clone(),
                    ask_id: "a1".to_owned(),
                },
                MobileEvent::SessionContext {
                    machine_id: machine_id.clone(),
                    session_id: session_id.clone(),
                    context: None,
                },
                status_event(None, false),
                MobileEvent::Hud {
                    machine_id,
                    session_id,
                    hud: None,
                },
            ]
        );
        assert!(
            !harness
                .presentations
                .borrow_mut()
                .answer_ask(MACHINE, SESSION, "a1", AskReply::default())
        );
    });
}

#[test]
fn a_stream_that_cannot_open_fails_with_the_transports_text() {
    local(async {
        let harness = Harness::new().await;
        harness.connection.close();
        harness.attach(Some(1)).unwrap();
        settle().await;

        let events = harness.take_events();
        assert_eq!(
            events.last(),
            Some(&failed(&crate::transport::TransportError::Closed.to_string()))
        );
    });
}

#[test]
fn detaching_ends_the_stream_and_reports_nothing() {
    local(async {
        let mut harness = Harness::attached(Some(1)).await;
        harness
            .connect(PresentationMode::Mirror, PresentationSnapshot::default())
            .await;
        harness.received().await;
        harness.take_events();

        harness.presentations.borrow_mut().detach(MACHINE, SESSION);

        assert!(harness.received().await.closed);
        harness.deliver(Body::Status(Some(blocked()))).await;
        harness.tick_after(Duration::from_secs(60)).await;
        assert!(harness.take_events().is_empty());
        assert!(harness.fake.take_opened().is_empty());
    });
}

#[test]
fn forgetting_a_machine_ends_its_streams() {
    local(async {
        let mut harness = Harness::attached(Some(1)).await;
        harness.received().await;
        harness.take_events();

        harness.presentations.borrow_mut().forget_machine("other");
        assert!(!harness.received().await.closed);
        harness.presentations.borrow_mut().forget_machine(MACHINE);
        assert!(harness.received().await.closed);
        assert!(harness.take_events().is_empty());
    });
}

/// An ask with a multi-line text field, over a pane, its buttons on the left of a terminal-style dialog.
fn typed_ask() -> PresentationAsk {
    use ngterm_core::model::{AskInputForm, AskInputMode};

    PresentationAsk {
        style: AskStyle::Terminal,
        align: AskAlignment::Left,
        width: Some(60),
        pane: Some(frames::PresentationPane::Identity(Uuid::parse_str(PANE).unwrap())),
        input: Some(AskInputForm {
            mode: AskInputMode::Multiline,
            value: "draft".to_owned(),
            placeholder: Some("Notes".to_owned()),
        }),
        ..ask("a1", 42)
    }
}

/// An ask with a checkbox list, one item checked and a free-form row, its buttons on the right.
fn picked_ask() -> PresentationAsk {
    use ngterm_core::model::AskChoiceForm;
    use ngterm_proto::AskChoice;

    PresentationAsk {
        align: AskAlignment::Right,
        choices: Some(AskChoiceForm {
            choices: vec![
                AskChoice {
                    id: "eu".to_owned(),
                    label: "Europe".to_owned(),
                    subtitle: Some("Frankfurt".to_owned()),
                },
                AskChoice {
                    id: "us".to_owned(),
                    label: "US".to_owned(),
                    subtitle: None,
                },
            ],
            multiple: true,
            checked: vec!["us".to_owned()],
            other: Some("Elsewhere".to_owned()),
        }),
        ..ask("a2", 43)
    }
}

#[test]
fn an_asks_text_field_and_choice_list_reach_the_app_as_the_origin_sent_them() {
    local(async {
        let mut harness = Harness::attached(Some(1)).await;
        harness
            .connect(PresentationMode::Mirror, PresentationSnapshot::default())
            .await;
        harness.received().await;
        harness.take_events();

        harness.deliver(Body::AskRequest(typed_ask())).await;
        harness.deliver(Body::AskRequest(picked_ask())).await;

        let asks: Vec<RemoteAsk> = harness
            .take_events()
            .into_iter()
            .filter_map(|event| match event {
                MobileEvent::Ask { ask, .. } => Some(ask),
                _ => None,
            })
            .collect();
        let [typed, picked] = asks.as_slice() else {
            panic!("two asks: {asks:?}");
        };
        assert_eq!(
            (typed.style.as_str(), typed.align.as_str(), typed.width),
            ("terminal", "left", Some(60))
        );
        assert_eq!(typed.pane.as_deref(), Some(PANE), "a pane as its upper-case identity");
        assert_eq!(
            typed.input,
            Some(RemoteAskInput {
                multiline: true,
                value: "draft".to_owned(),
                placeholder: Some("Notes".to_owned()),
            })
        );
        assert_eq!(typed.choices, None);

        assert_eq!(picked.align, "right");
        assert_eq!(picked.input, None);
        assert_eq!(
            picked.choices,
            Some(RemoteAskChoices {
                choices: vec![
                    RemoteAskChoice {
                        id: "eu".to_owned(),
                        label: "Europe".to_owned(),
                        subtitle: Some("Frankfurt".to_owned()),
                    },
                    RemoteAskChoice {
                        id: "us".to_owned(),
                        label: "US".to_owned(),
                        subtitle: None,
                    },
                ],
                multiple: true,
                checked: vec!["us".to_owned()],
                other: Some("Elsewhere".to_owned()),
            })
        );
    });
}

#[test]
fn a_status_shape_and_a_huds_spinner_reach_the_app_by_their_wire_names() {
    use ngterm_core::hud::{HudPosition, HudSpinner};
    use ngterm_core::model::StatusShape;

    local(async {
        let mut harness = Harness::attached(Some(1)).await;
        harness
            .connect(PresentationMode::Mirror, PresentationSnapshot::default())
            .await;
        harness.take_events();

        let pane = Uuid::parse_str(PANE).unwrap();
        let shaped = PresentationStatus {
            shape: Some(StatusShape::Diamond),
            pane: Some(frames::PresentationPane::Identity(pane)),
            ..blocked()
        };
        let mut spec = HudSpec::new("Deploying");
        spec.spinner = Some(HudSpinner::Braille);
        spec.position = HudPosition::BottomRight;
        let spinning = PresentationHud {
            spec,
            pane: Some(frames::PresentationPane::Identity(pane)),
            ..hud()
        };
        harness.deliver(Body::Status(Some(shaped))).await;
        harness.deliver(Body::Hud(Some(spinning))).await;

        let events = harness.take_events();
        let status = events.iter().find_map(|event| match event {
            MobileEvent::SessionStatus { status, .. } => status.clone(),
            _ => None,
        });
        assert_eq!(
            status,
            Some(RemoteStatus {
                shape: Some("diamond".to_owned()),
                pane: Some(PANE.to_owned()),
                ..remote_blocked()
            })
        );
        let shown = events.iter().find_map(|event| match event {
            MobileEvent::Hud { hud, .. } => hud.clone(),
            _ => None,
        });
        let shown = shown.expect("the HUD goes up");
        assert_eq!(
            (
                shown.message.as_str(),
                shown.spinner.as_deref(),
                shown.position.as_str()
            ),
            ("Deploying", Some("braille"), "bottom-right")
        );
        assert_eq!(shown.pane.as_deref(), Some(PANE));
    });
}
