//! One machine that runs both apps, ngterm and agterm: its listing holds the sessions of both, and each session's
//! stream, panes and commands go to the app that listed it.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use futures_lite::future::block_on;
use ngterm_core::remote::origin::{OriginForm, OriginShell};
use ngterm_core::remote::origin_replies::{RESTORE_MODE_ARGUMENTS, duplicate_session_arguments, end_session_arguments};
use ngterm_core::remote::peer::{AGTERM_ANSWER, RemotePeer, daemon_name, skipped_line};
use ngterm_core::remote::ssh::{control_tail, present_tail, tree_tail, tree_tail_for};
use uuid::Uuid;

use crate::api::{EventSink, Machine, MobileCore, MobileError, MobileEvent, PaneSize};
use crate::peer::PeerKind;
use crate::sessions::RemoteSession;
use crate::transport::fake::{FakeTransport, Script};

const WAIT: Duration = Duration::from_secs(5);
const OWN: &str = "00000000-0000-0000-0000-0000000000B1";
const THEIRS: &str = "00000000-0000-0000-0000-0000000000B2";
const COPY: &str = "00000000-0000-0000-0000-0000000000B3";
const THEIR_PANE: &str = "00000000-0000-0000-0000-0000000000C2";
const SIZE: PaneSize = PaneSize {
    cols: 80,
    rows: 24,
    cell_width: 8,
    cell_height: 16,
};
const AGTERM: OriginForm = OriginForm {
    shell: OriginShell::Posix,
    peer: RemotePeer::Agterm,
};

struct Recorder(Mutex<Sender<MobileEvent>>);

impl EventSink for Recorder {
    fn on_event(&self, event: MobileEvent) {
        let _ = self.0.lock().unwrap_or_else(PoisonError::into_inner).send(event);
    }
}

fn session(id: &str, daemon: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id, "name": id, "windowID": "w1", "windowName": "main", "workspaceID": "ws1",
        "workspaceName": "work", "cwd": "/repo", "panes": [{"pane": "left", "daemon": daemon}],
    })
}

fn answer(executable: &str, directory: &str, sessions: &[serde_json::Value]) -> String {
    serde_json::json!({"ok": true, "result": {"remote": {
        "endpoint": {"executable": executable, "socketDirectory": directory},
        "sessions": sessions,
        "presentation": 1,
    }}})
    .to_string()
}

fn ngterms_answer() -> String {
    let daemon = daemon_name(RemotePeer::Ngterm, Uuid::from_u128(0xc1));
    answer("/ngterm-session", "/tmp/ngterm-live-x", &[session(OWN, &daemon)])
}

/// agterm's answer under its marker line, holding `ids`; the first has the pane the tests show.
fn agterms_answer(ids: &[&str]) -> String {
    let sessions: Vec<serde_json::Value> = ids
        .iter()
        .zip([THEIR_PANE, "00000000-0000-0000-0000-0000000000C3"])
        .map(|(id, pane)| session(id, &daemon_name(RemotePeer::Agterm, Uuid::parse_str(pane).unwrap())))
        .collect();
    let body = answer(
        "/Applications/agterm.app/Contents/MacOS/zmx",
        "/tmp/agterm-zmx-x",
        &sessions,
    );
    format!("{AGTERM_ANSWER}\n{body}")
}

struct Both {
    transport: FakeTransport,
    core: Arc<MobileCore>,
    events: Receiver<MobileEvent>,
}

impl Both {
    /// A machine running both apps, connected and listed.
    fn listed() -> (Self, Vec<RemoteSession>) {
        let transport = FakeTransport::new();
        transport.script(
            &tree_tail(OriginShell::Posix),
            Script::ok(&format!("{}\n{}\n", ngterms_answer(), agterms_answer(&[THEIRS]))),
        );
        let (events, received) = mpsc::channel();
        let sink: Arc<dyn EventSink> = Arc::new(Recorder(Mutex::new(events)));
        let core = MobileCore::with_transport(transport.clone(), sink).expect("the actor starts");
        let machine = Machine {
            id: "m1".to_owned(),
            label: "studio".to_owned(),
            host: "studio".to_owned(),
            port: 22,
            user: "nick".to_owned(),
            key_id: "key".to_owned(),
            connect_timeout_secs: None,
        };
        block_on(core.connect(machine)).expect("connected");
        let sessions = block_on(core.list_sessions("m1".to_owned())).expect("listed");
        let both = Self {
            transport,
            core,
            events: received,
        };
        assert_eq!(both.next_tree(), apps(&sessions));
        (both, sessions)
    }

    /// The next listing reported, as each session's id and app.
    fn next_tree(&self) -> Vec<(String, Option<PeerKind>)> {
        loop {
            if let MobileEvent::Tree { sessions, .. } = self.events.recv_timeout(WAIT).expect("an event") {
                return apps(&sessions);
            }
        }
    }

    /// Waits for a command holding `needle` to run on the machine.
    fn ran(&self, needle: &str) -> String {
        let deadline = Instant::now() + WAIT;
        loop {
            let found = self
                .transport
                .execs()
                .into_iter()
                .find(|exec| exec.command.contains(needle));
            if let Some(exec) = found {
                return exec.command;
            }
            assert!(
                Instant::now() < deadline,
                "nothing ran {needle}: {:?}",
                self.transport.execs()
            );
            thread::sleep(Duration::from_millis(5));
        }
    }
}

fn apps(sessions: &[RemoteSession]) -> Vec<(String, Option<PeerKind>)> {
    sessions
        .iter()
        .map(|session| (session.id.clone(), session.peer))
        .collect()
}

fn app(id: &str, peer: PeerKind) -> (String, Option<PeerKind>) {
    (id.to_owned(), Some(peer))
}

#[test]
fn a_machine_running_both_apps_lists_the_sessions_of_both_each_naming_its_app() {
    let (_both, sessions) = Both::listed();
    assert_eq!(
        apps(&sessions),
        [app(OWN, PeerKind::Ngterm), app(THEIRS, PeerKind::Agterm)]
    );
}

#[test]
fn each_sessions_stream_opens_with_the_cli_of_the_app_that_listed_it() {
    let (both, _) = Both::listed();
    block_on(both.core.attach_session("m1".to_owned(), THEIRS.to_owned())).expect("attached agterm's");
    assert_eq!(both.ran(THEIRS), present_tail(AGTERM, THEIRS).unwrap());
    block_on(both.core.attach_session("m1".to_owned(), OWN.to_owned())).expect("attached ngterm's");
    assert_eq!(both.ran(OWN), present_tail(OriginShell::Posix, OWN).unwrap());
}

#[test]
fn an_agterm_sessions_pane_attaches_through_agterms_zmx_and_offers_no_chat() {
    let (both, _) = Both::listed();
    let pane = block_on(
        both.core
            .show_pane("m1".to_owned(), THEIRS.to_owned(), THEIR_PANE.to_owned(), SIZE),
    )
    .expect("shown");
    assert_eq!(pane.id(), THEIR_PANE.to_uppercase());
    let attach = both.ran(" 'attach' ");
    assert!(attach.contains("'ZMX_DIR=/tmp/agterm-zmx-x'"), "{attach}");
    assert!(
        attach.contains("/Applications/agterm.app/Contents/MacOS/zmx"),
        "{attach}"
    );
    let refused = MobileError::Remote {
        reason: crate::chat::NO_CHAT_ON_AGTERM.to_owned(),
    };
    let pane = || ("m1".to_owned(), THEIRS.to_owned(), THEIR_PANE.to_owned());
    let (machine, session, id) = pane();
    assert_eq!(
        block_on(both.core.open_chat(machine, session, id)),
        Err(refused.clone())
    );
    let (machine, session, id) = pane();
    assert_eq!(
        block_on(both.core.agent_commands(machine, session, id)).map(|_| ()),
        Err(refused)
    );
}

#[test]
fn ending_an_agterm_session_asks_agterm_and_keeps_ngterms_sessions_listed() {
    let (both, _) = Both::listed();
    let close = control_tail(AGTERM, &end_session_arguments(THEIRS));
    both.transport
        .script(&close, Script::ok(r#"{"ok": true, "result": {}}"#));
    block_on(both.core.end_session("m1".to_owned(), THEIRS.to_owned())).expect("ended on agterm");
    assert_eq!(both.ran("close"), close);
    assert_eq!(both.next_tree(), [app(OWN, PeerKind::Ngterm)]);
}

#[test]
fn duplicating_an_agterm_session_asks_agterm_and_keeps_ngterms_sessions_listed() {
    let (both, _) = Both::listed();
    let live = r#"{"ok": true, "result": {"restore": {"configured": "live", "requestedAtLaunch": "live",
        "active": "live", "restartRequired": false}}}"#;
    both.transport
        .script(&control_tail(AGTERM, &RESTORE_MODE_ARGUMENTS), Script::ok(live));
    both.transport.script(
        &control_tail(AGTERM, &duplicate_session_arguments(THEIRS)),
        Script::ok(&format!(r#"{{"ok": true, "result": {{"id": "{COPY}"}}}}"#)),
    );
    both.transport.script(
        &tree_tail_for(OriginShell::Posix, Some(RemotePeer::Agterm)),
        Script::ok(&agterms_answer(&[THEIRS, COPY])),
    );
    let copy = block_on(both.core.duplicate_session("m1".to_owned(), THEIRS.to_owned())).expect("duplicated");
    assert_eq!((copy.id.as_str(), copy.peer), (COPY, Some(PeerKind::Agterm)));
    assert_eq!(
        both.next_tree(),
        [
            app(OWN, PeerKind::Ngterm),
            app(THEIRS, PeerKind::Agterm),
            app(COPY, PeerKind::Agterm)
        ]
    );
    let ran: Vec<String> = both.transport.execs().into_iter().map(|exec| exec.command).collect();
    assert!(!ran.iter().any(|line| line.contains("ngtermctl session")), "{ran:?}");
}

/// A listing that goes on without agterm (it stalled or failed there) says nothing of agterm's sessions: they stay
/// listed and attachable, so the app has no reason to let an attached one go.
#[test]
fn a_listing_that_went_on_without_agterm_keeps_its_sessions_listed_and_attachable() {
    let (both, whole) = Both::listed();
    both.transport.script(
        &tree_tail(OriginShell::Posix),
        Script::ok(&format!("{}\n{}\n", ngterms_answer(), skipped_line(RemotePeer::Agterm))),
    );
    let partial = block_on(both.core.list_sessions("m1".to_owned())).expect("listed");
    assert_eq!(apps(&partial), apps(&whole));
    assert_eq!(both.next_tree(), apps(&whole));
    block_on(both.core.attach_session("m1".to_owned(), THEIRS.to_owned())).expect("still attachable");
    assert_eq!(both.ran(THEIRS), present_tail(AGTERM, THEIRS).unwrap());
    block_on(
        both.core
            .show_pane("m1".to_owned(), THEIRS.to_owned(), THEIR_PANE.to_owned(), SIZE),
    )
    .expect("its pane still shows");
}

/// The same for ngterm: its sessions stay, the machine stays ngterm's, and a later whole listing replaces them.
#[test]
fn a_listing_that_went_on_without_ngterm_keeps_its_sessions_until_a_whole_listing() {
    let (both, whole) = Both::listed();
    both.transport.script(
        &tree_tail(OriginShell::Posix),
        Script::ok(&format!(
            "{}\n{}\n",
            skipped_line(RemotePeer::Ngterm),
            agterms_answer(&[THEIRS, COPY])
        )),
    );
    let partial = block_on(both.core.list_sessions("m1".to_owned())).expect("listed");
    let kept = [
        app(OWN, PeerKind::Ngterm),
        app(THEIRS, PeerKind::Agterm),
        app(COPY, PeerKind::Agterm),
    ];
    assert_eq!(apps(&partial), kept);
    assert_eq!(both.next_tree(), kept);
    block_on(both.core.attach_session("m1".to_owned(), OWN.to_owned())).expect("still attachable");
    assert_eq!(both.ran(OWN), present_tail(OriginShell::Posix, OWN).unwrap());

    both.transport.script(
        &tree_tail(OriginShell::Posix),
        Script::ok(&format!("{}\n", agterms_answer(&[THEIRS]))),
    );
    block_on(both.core.list_sessions("m1".to_owned())).expect("listed");
    assert_eq!(both.next_tree(), [app(THEIRS, PeerKind::Agterm)]);
    drop(whole);
}
