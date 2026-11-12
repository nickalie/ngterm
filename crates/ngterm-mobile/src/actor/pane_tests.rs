//! Shown panes through the whole core: the actor thread, the panes' real terminals, feeder and writer threads, over
//! the fake transport's channels driven from the test thread. Each wait is bounded; events are read as they arrive.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use futures_lite::future::block_on;
use ngterm_core::live::daemon_name;
use ngterm_core::live::lead::{LeadAttachment, LeadNotice};
use ngterm_core::remote::frames::{
    self, Body, PresentationFrame, PresentationHello, PresentationMode, PresentationRespawns, PresentationSnapshot,
};
use ngterm_core::remote::ssh::{SESSION_GONE, attach_tail, present_tail, tree_tail};
use ngterm_proto::ControlLiveEndpoint;
use uuid::Uuid;

use crate::api::{EventSink, Machine, MachineState, MobileCore, MobileEvent, Pane, PaneInputError, PaneSize};
use crate::chat::input::InputTiming;
use crate::pane::{ClipboardReply, PaneEnd, PaneRef, PaneRole};
use crate::transport::fake::{FakeTransport, Opened, Script};
use crate::transport::russh::CONNECTION_LOST;
use crate::transport::{ChannelEnd, ChannelInput, ChannelPeer};

pub(super) const WAIT: Duration = Duration::from_secs(10);
pub(super) const QUIET: Duration = Duration::from_millis(300);
pub(super) const MACHINE: &str = "m1";
pub(super) const SESSION: &str = "00000000-0000-0000-0000-0000000000A1";
pub(super) const PANE: &str = "00000000-0000-0000-0000-0000000000B1";
pub(super) const SIZE: PaneSize = PaneSize {
    cols: 80,
    rows: 24,
    cell_width: 9,
    cell_height: 18,
};

pub(super) struct Recorder(Mutex<Sender<MobileEvent>>);

impl EventSink for Recorder {
    fn on_event(&self, event: MobileEvent) {
        let _ = self.0.lock().unwrap_or_else(PoisonError::into_inner).send(event);
    }
}

pub(super) fn pane_uuid() -> Uuid {
    Uuid::parse_str(PANE).unwrap()
}

pub(super) fn daemon() -> String {
    daemon_name(pane_uuid())
}

pub(super) fn endpoint() -> ControlLiveEndpoint {
    ControlLiveEndpoint {
        attach_bridge: None,
        executable: "/ngterm-session".to_owned(),
        socket_directory: "/tmp/ngterm-live-x".to_owned(),
    }
}

/// The machine's listing, holding the session with its one pane or not.
pub(super) fn listing(with_session: bool) -> Script {
    let sessions = if with_session {
        serde_json::json!([{
            "id": SESSION, "name": "main", "windowID": "w1", "windowName": "main", "workspaceID": "ws1",
            "workspaceName": "work", "cwd": "/repo", "panes": [{"pane": "left", "daemon": daemon()}],
        }])
    } else {
        serde_json::json!([])
    };
    let remote = serde_json::json!({
        "endpoint": {"executable": "/ngterm-session", "socketDirectory": "/tmp/ngterm-live-x"},
        "sessions": sessions,
        "presentation": 1,
    });
    Script::ok(&serde_json::json!({"ok": true, "result": {"remote": remote}}).to_string())
}

pub(super) fn reference() -> PaneRef {
    PaneRef {
        machine_id: MACHINE.to_owned(),
        session_id: SESSION.to_owned(),
        pane_id: PANE.to_owned(),
    }
}

/// The lead words of an attach command: its nonce, and whether it claims.
pub(super) fn lead_of(command: &str) -> LeadAttachment {
    let at = command.find("NGTERM_LIVE_MANAGED=").unwrap() + "NGTERM_LIVE_MANAGED=".len();
    let nonce: String = command[at..].chars().take_while(char::is_ascii_hexdigit).collect();
    LeadAttachment {
        nonce,
        claim: command.contains("NGTERM_LIVE_MANAGED_CLAIM=1"),
    }
}

/// A role report as the attach client titles it.
pub(super) fn role_title(nonce: &str, role: &str, generation: u32) -> Vec<u8> {
    format!("\x1b]2;{}{nonce}:{role}:{generation}\x07", LeadNotice::PREFIX).into_bytes()
}

pub(super) struct Fixture {
    pub(super) core: Arc<MobileCore>,
    pub(super) transport: FakeTransport,
    pub(super) events: Receiver<MobileEvent>,
    /// Channels opened and not yet taken by a test.
    stash: Vec<Opened>,
}

impl Fixture {
    /// A core connected to the machine, its listing read.
    pub(super) fn connected() -> Self {
        Self::connected_with(vec![listing(true)])
    }

    pub(super) fn connected_with(listings: Vec<Script>) -> Self {
        Self::connected_timed(listings, InputTiming::default())
    }

    /// A core connected to the machine, its listing read from `listings`, its chat actions timed by `timing`.
    pub(super) fn connected_timed(listings: Vec<Script>, timing: InputTiming) -> Self {
        let transport = FakeTransport::new();
        transport.script_sequence(&tree_tail(ngterm_core::remote::origin::OriginShell::Posix), listings);
        let (events, received) = mpsc::channel();
        let sink: Arc<dyn EventSink> = Arc::new(Recorder(Mutex::new(events)));
        let core = MobileCore::with_timing(transport.clone(), sink, timing).unwrap();
        let machine = Machine {
            id: MACHINE.to_owned(),
            label: "box".to_owned(),
            host: "box".to_owned(),
            port: 22,
            user: "nick".to_owned(),
            key_id: "key".to_owned(),
        };
        block_on(core.connect(machine)).unwrap();
        block_on(core.list_sessions(MACHINE.to_owned())).unwrap();
        // the connect's and the listing's own events came before their answers
        received.try_iter().for_each(drop);
        Self {
            core,
            transport,
            events: received,
            stash: Vec::new(),
        }
    }

    pub(super) fn show(&self) -> Arc<Pane> {
        block_on(
            self.core
                .show_pane(MACHINE.to_owned(), SESSION.to_owned(), PANE.to_owned(), SIZE),
        )
        .unwrap()
    }

    /// The next channel opened that `matches`, waiting for it.
    pub(super) fn opened(&mut self, matches: impl Fn(&Opened) -> bool) -> Opened {
        let deadline = Instant::now() + WAIT;
        loop {
            self.stash.extend(self.transport.take_opened());
            if let Some(at) = self.stash.iter().position(&matches) {
                return self.stash.remove(at);
            }
            assert!(Instant::now() < deadline, "no channel opened");
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// The next pane attach opened.
    pub(super) fn attach(&mut self) -> Opened {
        self.opened(|opened| opened.pty.is_some())
    }

    /// Whether a pane attach opens within [`QUIET`].
    pub(super) fn attaches_soon(&mut self) -> bool {
        thread::sleep(QUIET);
        self.stash.extend(self.transport.take_opened());
        self.stash.iter().any(|opened| opened.pty.is_some())
    }

    /// The next event `matches` takes, skipping the others.
    pub(super) fn next<T>(&self, matches: impl Fn(MobileEvent) -> Option<T>) -> T {
        let deadline = Instant::now() + WAIT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.events.recv_timeout(left) {
                Ok(event) => {
                    if let Some(found) = matches(event) {
                        return found;
                    }
                }
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => panic!("no such event"),
            }
        }
    }

    /// Every event that arrives within [`QUIET`].
    pub(super) fn quiet(&self) -> Vec<MobileEvent> {
        thread::sleep(QUIET);
        self.events.try_iter().collect()
    }

    fn ended(&self) -> PaneEnd {
        self.next(|event| match event {
            MobileEvent::PaneEnded { pane, end } if pane == reference() => Some(end),
            _ => None,
        })
    }

    pub(super) fn lead(&self) -> (Option<PaneRole>, bool) {
        self.next(|event| match event {
            MobileEvent::PaneLead { pane, role, covered } if pane == reference() => Some((role, covered)),
            _ => None,
        })
    }

    pub(super) fn attached(&self) {
        self.next(|event| matches!(event, MobileEvent::PaneAttached { pane } if pane == reference()).then_some(()));
    }

    pub(super) fn dirty(&self) {
        self.next(|event| matches!(event, MobileEvent::PaneDirty { .. }).then_some(()));
    }
}

pub(super) fn send(peer: &ChannelPeer, bytes: &[u8]) {
    assert!(block_on(peer.data(bytes.to_vec())), "the feeder reads the channel");
}

/// What the core sent down `peer` until `done` holds for it: the bytes, and the other inputs in order.
pub(super) fn inputs_until(
    peer: &mut ChannelPeer,
    done: impl Fn(&[u8], &[ChannelInput]) -> bool,
) -> (Vec<u8>, Vec<ChannelInput>) {
    let deadline = Instant::now() + WAIT;
    let (mut bytes, mut others) = (Vec::new(), Vec::new());
    while !done(&bytes, &others) {
        match peer.try_input() {
            Some(ChannelInput::Data(data)) => bytes.extend(data),
            Some(other) => others.push(other),
            None => {
                assert!(
                    Instant::now() < deadline,
                    "waited for input; got {:?} and {others:?}",
                    String::from_utf8_lossy(&bytes)
                );
                thread::sleep(Duration::from_millis(5));
            }
        }
    }
    (bytes, others)
}

pub(super) fn contains(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|window| window == needle)
}

pub(super) fn position(bytes: &[u8], needle: &[u8]) -> usize {
    bytes.windows(needle.len()).position(|window| window == needle).unwrap()
}

fn end(peer: ChannelPeer, end: ChannelEnd) {
    block_on(peer.end(end));
}

#[test]
fn a_shown_pane_attaches_on_a_pty_and_its_replies_and_input_go_back_in_order() {
    let mut fixture = Fixture::connected();
    let pane = fixture.show();
    assert_eq!(pane.id(), PANE);
    let mut opened = fixture.attach();
    assert_eq!(opened.pty, Some(SIZE.into()));
    let lead = lead_of(&opened.command);
    assert!(!lead.claim, "the first attach claims nothing");
    assert_eq!(
        opened.command,
        attach_tail(&endpoint(), &daemon(), Some(&lead)).unwrap()
    );
    fixture.attached();
    assert_eq!(fixture.lead(), (None, false));

    // the cursor report proves the bytes reached the terminal: column 6 after `hello`
    send(&opened.peer, b"hello\x1b[6n");
    fixture.dirty();
    assert!(pane.type_text("x".to_owned()).unwrap());
    let (bytes, _) = inputs_until(&mut opened.peer, |bytes, _| contains(bytes, b"x"));
    assert_eq!(bytes, b"\x1b[1;6Rx", "the reply, then the input");

    let wider = PaneSize { cols: 100, ..SIZE };
    pane.resize(wider).unwrap();
    let (_, others) = inputs_until(&mut opened.peer, |_, others| !others.is_empty());
    assert_eq!(others, [ChannelInput::Resize(wider.into())]);
    assert_eq!(pane.size(), wider);
}

#[test]
fn a_shown_pane_needs_a_listed_session_and_pane() {
    let fixture = Fixture::connected();
    let show = |session: &str, pane: &str| {
        block_on(
            fixture
                .core
                .show_pane(MACHINE.to_owned(), session.to_owned(), pane.to_owned(), SIZE),
        )
    };
    assert!(matches!(
        show("00000000-0000-0000-0000-0000000000C1", PANE),
        Err(crate::api::MobileError::NotListed { .. })
    ));
    assert!(matches!(
        show(SESSION, "00000000-0000-0000-0000-0000000000C2"),
        Err(crate::api::MobileError::Remote { reason }) if reason == crate::pane::UNKNOWN_PANE
    ));
}

#[test]
fn role_reports_become_the_panes_lead_never_its_title_and_take_lead_reattaches_claiming() {
    let mut fixture = Fixture::connected();
    let pane = fixture.show();
    let mut first = fixture.attach();
    let lead = lead_of(&first.command);
    fixture.attached();
    assert_eq!(fixture.lead(), (None, false));

    send(&first.peer, &role_title(&lead.nonce, "follower", 1));
    assert_eq!(fixture.lead(), (Some(PaneRole::Follower), true));
    send(
        &first.peer,
        &role_title("0123456789abcdef0123456789abcdef", "leader", 2),
    );
    send(&first.peer, b"\x1b]2;vim\x07");
    let title = fixture.next(|event| match event {
        MobileEvent::PaneTitle { title, .. } => Some(title),
        MobileEvent::PaneLead { .. } => panic!("a report for another nonce changes nothing"),
        _ => None,
    });
    assert_eq!(title, "vim", "only the program's own title shows");

    let old = pane.terminal().unwrap();
    fixture
        .core
        .take_lead(MACHINE.to_owned(), SESSION.to_owned(), PANE.to_owned())
        .unwrap();
    let mut second = fixture.attach();
    let claim = lead_of(&second.command);
    assert!(claim.claim, "take control claims");
    assert_ne!(claim.nonce, lead.nonce);
    fixture.attached();
    assert_eq!(fixture.lead(), (None, true), "covered until the takeover reports");
    assert!(!Arc::ptr_eq(&old, &pane.terminal().unwrap()), "a fresh terminal");
    inputs_until(&mut first.peer, |_, others| others.contains(&ChannelInput::Close));

    send(&second.peer, &role_title(&claim.nonce, "leader", 3));
    assert_eq!(fixture.lead(), (Some(PaneRole::Leader), false));

    // the leader left: the pane attaches again without the claim
    send(&second.peer, &role_title(&claim.nonce, "unowned", 4));
    assert_eq!(fixture.lead(), (Some(PaneRole::Unowned), true));
    let third = fixture.attach();
    assert!(!lead_of(&third.command).claim);
    inputs_until(&mut second.peer, |_, others| others.contains(&ChannelInput::Close));
}

#[test]
fn a_lost_link_covers_the_pane_and_it_reattaches_into_a_fresh_terminal_after_the_reconnect() {
    let mut fixture = Fixture::connected();
    let pane = fixture.show();
    let first = fixture.attach();
    let lead = lead_of(&first.command);
    let old = pane.terminal().unwrap();

    fixture.transport.refuse("box", "the network is down");
    fixture.transport.drop_connections(MACHINE);
    assert_eq!(
        fixture.ended(),
        PaneEnd::Lost {
            reason: CONNECTION_LOST.to_owned()
        }
    );
    fixture.next(|event| {
        matches!(
            event,
            MobileEvent::MachineState {
                state: MachineState::Reconnecting { .. },
                ..
            }
        )
        .then_some(())
    });
    assert!(!fixture.attaches_soon(), "nothing attaches while the machine is down");

    fixture.transport.accept("box");
    fixture.core.network_changed().unwrap();
    let again = fixture.attach();
    let relead = lead_of(&again.command);
    assert!(!relead.claim, "a reattach claims nothing");
    assert_ne!(relead.nonce, lead.nonce);
    fixture.attached();
    assert_eq!(fixture.lead(), (None, true), "covered until the new attach reports");
    assert!(
        !Arc::ptr_eq(&old, &pane.terminal().unwrap()),
        "never the stale terminal"
    );
}

#[test]
fn an_exit_is_reported_and_attaches_again_only_after_the_origin_respawned_the_pane() {
    let mut fixture = Fixture::connected();
    block_on(fixture.core.attach_session(MACHINE.to_owned(), SESSION.to_owned())).unwrap();
    let stream = fixture.opened(|opened| {
        opened.command == present_tail(ngterm_core::remote::origin::OriginShell::Posix, SESSION).unwrap()
    });
    let counts = |generation: i64| PresentationRespawns {
        epoch: "e1".to_owned(),
        generations: std::collections::BTreeMap::from([(daemon(), generation)]),
    };
    let snapshot = PresentationSnapshot {
        respawns: Some(counts(0)),
        ..PresentationSnapshot::default()
    };
    let hello = Body::Hello(PresentationHello {
        version: frames::VERSION,
        kinds: Vec::new(),
        mode: PresentationMode::Mirror,
    });
    let frame = |rev: i64, body: Body| {
        frames::encode(&PresentationFrame {
            generation: 1,
            rev,
            body,
        })
        .unwrap()
    };
    send(&stream.peer, &frame(1, hello));
    send(&stream.peer, &frame(2, Body::Snapshot(snapshot)));

    fixture.show();
    let first = fixture.attach();
    send(&first.peer, b"bye\r\n");
    end(
        first.peer,
        ChannelEnd::Exit {
            code: Some(0),
            signal: None,
        },
    );
    assert_eq!(
        fixture.ended(),
        PaneEnd::Exited {
            code: Some(0),
            signal: None
        }
    );
    assert!(
        !fixture.attaches_soon(),
        "an exit with no respawn keeps the last screen"
    );

    send(&stream.peer, &frame(3, Body::Respawn(counts(1))));
    let again = fixture.attach();
    assert!(!lead_of(&again.command).claim, "attach-only");
    fixture.attached();
    assert_eq!(fixture.lead(), (None, true));
}

#[test]
fn an_empty_grid_is_refused_and_a_pane_shown_at_one_attaches_as_soon_as_the_view_sizes_it() {
    let mut fixture = Fixture::connected();
    let empty = PaneSize {
        cols: 0,
        rows: 0,
        ..SIZE
    };
    let pane = block_on(
        fixture
            .core
            .show_pane(MACHINE.to_owned(), SESSION.to_owned(), PANE.to_owned(), empty),
    )
    .unwrap();
    assert!(
        !fixture
            .quiet()
            .iter()
            .any(|event| matches!(event, MobileEvent::PaneEnded { .. })),
        "a pane with no grid waits for one, not lost"
    );
    assert!(!fixture.attaches_soon(), "nothing attaches without a grid");
    assert!(matches!(
        pane.resize(PaneSize { cols: 0, ..SIZE }),
        Err(PaneInputError::Refused { .. })
    ));
    assert_eq!(pane.size(), empty, "a refused grid is not kept");

    // well before the backoff's first retry (1 s after the show)
    pane.resize(SIZE).unwrap();
    assert_eq!(pane.size(), SIZE);
    assert!(fixture.attaches_soon(), "the grid wakes the attach at once");
    let opened = fixture.attach();
    assert_eq!(opened.pty, Some(SIZE.into()), "it attaches at the view's grid");
    fixture.attached();
    assert!(!fixture.attaches_soon(), "one attach");
}

#[test]
fn a_pane_shown_after_its_sessions_snapshot_finds_a_respawn_missed_while_the_stream_was_down() {
    let mut fixture = Fixture::connected();
    block_on(fixture.core.attach_session(MACHINE.to_owned(), SESSION.to_owned())).unwrap();
    let stream = fixture.opened(|opened| {
        opened.command == present_tail(ngterm_core::remote::origin::OriginShell::Posix, SESSION).unwrap()
    });
    let snapshot = |generation: i64| {
        Body::Snapshot(PresentationSnapshot {
            respawns: Some(PresentationRespawns {
                epoch: "e1".to_owned(),
                generations: std::collections::BTreeMap::from([(daemon(), generation)]),
            }),
            ..PresentationSnapshot::default()
        })
    };
    let hello = Body::Hello(PresentationHello {
        version: frames::VERSION,
        kinds: Vec::new(),
        mode: PresentationMode::Mirror,
    });
    let frame = |rev: i64, body: Body| {
        frames::encode(&PresentationFrame {
            generation: 1,
            rev,
            body,
        })
        .unwrap()
    };
    send(&stream.peer, &frame(1, hello));
    // the session's pane respawned once already, before the pane is shown
    send(&stream.peer, &frame(2, snapshot(1)));
    fixture.quiet();

    fixture.show();
    let first = fixture.attach();
    end(
        first.peer,
        ChannelEnd::Exit {
            code: Some(0),
            signal: None,
        },
    );
    fixture.ended();
    assert!(!fixture.attaches_soon(), "no respawn since the pane was shown");

    // the respawn's frame was missed; the next snapshot (a reconnect's) carries it
    send(&stream.peer, &frame(3, snapshot(2)));
    let again = fixture.attach();
    assert!(!lead_of(&again.command).claim, "attach-only");
}

#[test]
fn two_machines_showing_one_origin_pane_each_attach_again_after_its_respawn() {
    const OTHER: &str = "m2";
    let mut fixture = Fixture::connected();
    // a second record of the same origin (another address), showing the same pane
    let other = Machine {
        id: OTHER.to_owned(),
        label: "box over tailscale".to_owned(),
        host: "box.tail".to_owned(),
        port: 22,
        user: "nick".to_owned(),
        key_id: "key".to_owned(),
    };
    block_on(fixture.core.connect(other)).unwrap();
    block_on(fixture.core.list_sessions(OTHER.to_owned())).unwrap();
    let counts = |generation: i64| PresentationRespawns {
        epoch: "e1".to_owned(),
        generations: std::collections::BTreeMap::from([(daemon(), generation)]),
    };
    let frame = |rev: i64, body: Body| {
        frames::encode(&PresentationFrame {
            generation: 1,
            rev,
            body,
        })
        .unwrap()
    };
    let mut streams = Vec::new();
    for machine in [MACHINE, OTHER] {
        block_on(fixture.core.attach_session(machine.to_owned(), SESSION.to_owned())).unwrap();
        let stream = fixture.opened(|opened| {
            opened.machine_id == machine
                && opened.command == present_tail(ngterm_core::remote::origin::OriginShell::Posix, SESSION).unwrap()
        });
        let hello = Body::Hello(PresentationHello {
            version: frames::VERSION,
            kinds: Vec::new(),
            mode: PresentationMode::Mirror,
        });
        send(&stream.peer, &frame(1, hello));
        let snapshot = PresentationSnapshot {
            respawns: Some(counts(0)),
            ..PresentationSnapshot::default()
        };
        send(&stream.peer, &frame(2, Body::Snapshot(snapshot)));
        streams.push(stream);
    }
    fixture.quiet();

    for machine in [MACHINE, OTHER] {
        block_on(
            fixture
                .core
                .show_pane(machine.to_owned(), SESSION.to_owned(), PANE.to_owned(), SIZE),
        )
        .unwrap();
        let attach = fixture.opened(|opened| opened.machine_id == machine && opened.pty.is_some());
        end(
            attach.peer,
            ChannelEnd::Exit {
                code: Some(0),
                signal: None,
            },
        );
    }
    let exited = |events: &[MobileEvent], machine: &str| {
        events.iter().any(|event| {
            matches!(event, MobileEvent::PaneEnded { pane, end: PaneEnd::Exited { .. } } if pane.machine_id == machine)
        })
    };
    let events = fixture.quiet();
    assert!(
        exited(&events, MACHINE) && exited(&events, OTHER),
        "both exits are reported"
    );
    assert!(!fixture.attaches_soon(), "no respawn yet");

    for stream in &streams {
        send(&stream.peer, &frame(3, Body::Respawn(counts(1))));
    }
    for machine in [MACHINE, OTHER] {
        let again = fixture.opened(|opened| opened.machine_id == machine && opened.pty.is_some());
        assert!(!lead_of(&again.command).claim, "attach-only");
    }
}

#[test]
fn a_signal_exit_is_reported_with_its_signal() {
    let mut fixture = Fixture::connected();
    fixture.show();
    let opened = fixture.attach();
    end(
        opened.peer,
        ChannelEnd::Exit {
            code: None,
            signal: Some("TERM".to_owned()),
        },
    );
    assert_eq!(
        fixture.ended(),
        PaneEnd::Exited {
            code: None,
            signal: Some("TERM".to_owned())
        }
    );
}

#[test]
fn the_gone_guards_exit_ends_the_pane_for_good_once_the_listing_confirms_it() {
    let mut fixture = Fixture::connected_with(vec![listing(true), listing(false)]);
    fixture.show();
    let opened = fixture.attach();
    send(&opened.peer, format!("{SESSION_GONE}\r\n").as_bytes());
    end(
        opened.peer,
        ChannelEnd::Exit {
            code: Some(1),
            signal: None,
        },
    );
    let sessions = fixture.next(|event| match event {
        MobileEvent::Tree { sessions, .. } => Some(sessions),
        _ => None,
    });
    assert!(sessions.is_empty(), "the session's row leaves");
    assert_eq!(fixture.ended(), PaneEnd::Gone);
    assert!(!fixture.attaches_soon(), "final");
}

#[test]
fn the_gone_guards_line_with_the_session_still_listed_is_a_plain_exit() {
    let mut fixture = Fixture::connected_with(vec![listing(true), listing(true)]);
    fixture.show();
    let opened = fixture.attach();
    send(&opened.peer, format!("{SESSION_GONE}\r\n").as_bytes());
    end(
        opened.peer,
        ChannelEnd::Exit {
            code: Some(1),
            signal: None,
        },
    );
    assert_eq!(
        fixture.ended(),
        PaneEnd::Exited {
            code: Some(1),
            signal: None
        }
    );
}

#[test]
fn the_bell_reaches_the_app_for_its_pane() {
    let mut fixture = Fixture::connected();
    let _pane = fixture.show();
    let opened = fixture.attach();

    send(&opened.peer, b"\x07");
    fixture.next(|event| matches!(event, MobileEvent::Bell { pane } if pane == reference()).then_some(()));
}

#[test]
fn showing_a_shown_pane_hands_back_the_same_pane_and_attaches_nothing_more() {
    let mut fixture = Fixture::connected();
    let first = fixture.show();
    let _attach = fixture.attach();
    let second = fixture.show();
    assert!(Arc::ptr_eq(&first.terminal().unwrap(), &second.terminal().unwrap()));
    assert!(!fixture.attaches_soon(), "no second attach");
}

#[test]
fn commands_about_a_pane_not_shown_change_nothing() {
    let mut fixture = Fixture::connected();
    let pane = fixture.show();
    let mut opened = fixture.attach();
    fixture.attached();
    fixture.lead();
    let other = "00000000-0000-0000-0000-0000000000C2".to_owned();
    let core = &fixture.core;
    core.hide_pane(MACHINE.to_owned(), SESSION.to_owned(), other.clone())
        .unwrap();
    core.take_lead(MACHINE.to_owned(), SESSION.to_owned(), other.clone())
        .unwrap();
    core.answer_clipboard_read(MACHINE.to_owned(), SESSION.to_owned(), other, 1, ClipboardReply::Deny)
        .unwrap();
    // a read the shown pane never asked for
    core.answer_clipboard_read(
        MACHINE.to_owned(),
        SESSION.to_owned(),
        PANE.to_owned(),
        99,
        ClipboardReply::Allow {
            contents: vec![crate::ClipboardPart {
                mime: "text/plain".to_owned(),
                data: b"secret".to_vec(),
            }],
            available: Vec::new(),
            remember: false,
        },
    )
    .unwrap();
    core.complete_clipboard_write(
        MACHINE.to_owned(),
        SESSION.to_owned(),
        PANE.to_owned(),
        99,
        crate::ClipboardWriteStatus::Done,
    )
    .unwrap();

    assert!(fixture.quiet().is_empty(), "nothing is reported");
    assert!(!fixture.attaches_soon(), "nothing attaches again");
    assert!(pane.type_text("x".to_owned()).unwrap(), "the shown pane goes on");
    let (bytes, others) = inputs_until(&mut opened.peer, |bytes, _| contains(bytes, b"x"));
    assert_eq!((bytes.as_slice(), others.as_slice()), (b"x".as_slice(), [].as_slice()));
}

#[test]
fn a_hidden_panes_handle_takes_a_grid_and_attaches_nothing() {
    let mut fixture = Fixture::connected();
    let pane = fixture.show();
    let _attach = fixture.attach();
    fixture.attached();
    fixture.lead();
    fixture
        .core
        .hide_pane(MACHINE.to_owned(), SESSION.to_owned(), PANE.to_owned())
        .unwrap();
    // the hide ran: the handle has no terminal
    let deadline = Instant::now() + WAIT;
    while pane.terminal().is_some() {
        assert!(Instant::now() < deadline, "the pane is never hidden");
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(pane.resize(PaneSize { cols: 100, ..SIZE }), Ok(()));
    assert!(!fixture.attaches_soon());
    assert!(fixture.quiet().is_empty());
}

#[test]
fn a_channel_lost_on_a_connection_still_up_attaches_again_on_the_backoff() {
    let mut fixture = Fixture::connected();
    let pane = fixture.show();
    let first = fixture.attach();
    fixture.attached();
    let old = pane.terminal().unwrap();

    end(first.peer, ChannelEnd::Lost("the channel was reset".to_owned()));
    assert_eq!(
        fixture.ended(),
        PaneEnd::Lost {
            reason: "the channel was reset".to_owned()
        }
    );
    // the first loss waits a second, then the same connection attaches it again
    let again = fixture.attach();
    assert!(!lead_of(&again.command).claim);
    fixture.attached();
    assert!(!Arc::ptr_eq(&old, &pane.terminal().unwrap()), "a fresh terminal");
    assert_eq!(fixture.transport.connects(), [MACHINE], "on the same connection");
}

/// A listing of the session under `session_id`, its one pane, its endpoint's executable at `executable`.
fn listing_of(session_id: &str, executable: &str) -> Script {
    let remote = serde_json::json!({
        "endpoint": {"executable": executable, "socketDirectory": "/tmp/ngterm-live-x"},
        "sessions": [{
            "id": session_id, "name": "main", "windowID": "w1", "windowName": "main", "workspaceID": "ws1",
            "workspaceName": "work", "cwd": "/repo", "panes": [{"pane": "left", "daemon": daemon()}],
        }],
        "presentation": 1,
    });
    Script::ok(&serde_json::json!({"ok": true, "result": {"remote": remote}}).to_string())
}

#[test]
fn a_pane_whose_attach_cant_be_named_is_refused_as_unknown() {
    let unknown = Err(crate::api::MobileError::Remote {
        reason: crate::pane::UNKNOWN_PANE.to_owned(),
    });
    // a listed session whose id is not an identity
    let mut fixture = Fixture::connected_with(vec![listing_of("main", "/ngterm-session")]);
    let shown = block_on(
        fixture
            .core
            .show_pane(MACHINE.to_owned(), "main".to_owned(), PANE.to_owned(), SIZE),
    );
    assert_eq!(shown.map(|_| ()), unknown);
    assert!(!fixture.attaches_soon());

    // an endpoint no command line can carry
    let mut fixture = Fixture::connected_with(vec![listing_of(SESSION, "/ngterm\nsession")]);
    let shown = block_on(
        fixture
            .core
            .show_pane(MACHINE.to_owned(), SESSION.to_owned(), PANE.to_owned(), SIZE),
    );
    assert_eq!(shown.map(|_| ()), unknown);
    assert!(!fixture.attaches_soon());
}

#[test]
fn take_lead_does_nothing_while_a_takeover_is_on_its_way_or_after_the_program_ended() {
    let mut fixture = Fixture::connected();
    let _pane = fixture.show();
    let _first = fixture.attach();
    fixture.attached();
    let take_lead = |fixture: &Fixture| {
        fixture
            .core
            .take_lead(MACHINE.to_owned(), SESSION.to_owned(), PANE.to_owned())
            .unwrap();
    };

    take_lead(&fixture);
    let second = fixture.attach();
    assert!(lead_of(&second.command).claim);
    // the takeover has not reported its role yet
    take_lead(&fixture);
    assert!(!fixture.attaches_soon(), "one takeover at a time");

    end(
        second.peer,
        ChannelEnd::Exit {
            code: Some(0),
            signal: None,
        },
    );
    assert_eq!(
        fixture.ended(),
        PaneEnd::Exited {
            code: Some(0),
            signal: None
        }
    );
    take_lead(&fixture);
    assert!(!fixture.attaches_soon(), "an ended program has no lead to take");
}

#[test]
fn shown_panes_gate_the_bridge_on_the_endpoint_and_keep_old_peer_pastes_plain() {
    use ngterm_core::attach_bridge::{Envelope, Message, VERSION};
    for capability in [None, Some(VERSION)] {
        let mut script = listing(true);
        if let Some(version) = capability {
            let mut body: serde_json::Value = serde_json::from_slice(&script.stdout).unwrap();
            body["result"]["remote"]["endpoint"]["attachBridge"] = serde_json::json!(version);
            script.stdout = body.to_string().into_bytes();
        }
        let mut fixture = Fixture::connected_with(vec![script]);
        let pane = fixture.show();
        let mut opened = fixture.attach();
        let lead = lead_of(&opened.command);
        assert_eq!(opened.command.contains("attach-bridge"), capability.is_some());
        fixture.attached();
        let nonce = if capability.is_some() {
            lead.nonce.as_str()
        } else {
            "forged"
        };
        send(
            &opened.peer,
            &Envelope {
                version: VERSION,
                nonce: nonce.into(),
                message: Message::Hello,
            }
            .encode(),
        );
        send(&opened.peer, b"ready\x1b[6n");
        inputs_until(&mut opened.peer, |bytes, _| contains(bytes, b"R"));
        pane.paste("pasted".into(), false).unwrap();
        let (bytes, _) = inputs_until(&mut opened.peer, |bytes, _| {
            if capability.is_some() {
                contains(bytes, b"\x1b\\")
            } else {
                contains(bytes, b"pasted")
            }
        });
        if capability.is_some() {
            let decoded = ngterm_core::attach_bridge::Decoder::default().feed(&bytes);
            assert!(decoded.iter().any(|part| matches!(part, ngterm_core::attach_bridge::Part::Envelope(Envelope { nonce, message: Message::Paste(_), .. }) if nonce == &lead.nonce)));
        } else {
            assert_eq!(bytes, b"pasted");
        }
    }
}
