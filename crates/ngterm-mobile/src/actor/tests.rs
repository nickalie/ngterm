use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, ThreadId};
use std::time::Duration;

use futures_lite::future::{block_on, poll_once};

use super::THREAD_NAME;
use crate::api::{EventSink, Machine, MachineState, MobileCore, MobileError, MobileEvent};
use crate::transport::fake::FakeTransport;

const WAIT: Duration = Duration::from_secs(5);

/// An event with the id and name of the thread it arrived on.
type Received = (ThreadId, Option<String>, MobileEvent);

/// Records each event with the thread it arrived on.
struct Recorder {
    events: Mutex<Sender<Received>>,
}

impl EventSink for Recorder {
    fn on_event(&self, event: MobileEvent) {
        let current = thread::current();
        let sent = (current.id(), current.name().map(str::to_owned), event);
        let _ = self.events.lock().unwrap_or_else(PoisonError::into_inner).send(sent);
    }
}

fn recorder() -> (Arc<Recorder>, Receiver<Received>) {
    let (events, received) = mpsc::channel();
    (
        Arc::new(Recorder {
            events: Mutex::new(events),
        }),
        received,
    )
}

fn machine(id: &str, host: &str) -> Machine {
    Machine {
        id: id.to_owned(),
        label: id.to_owned(),
        host: host.to_owned(),
        port: 22,
        user: "nick".to_owned(),
        key_id: "key".to_owned(),
    }
}

fn core(transport: &FakeTransport, sink: &Arc<Recorder>) -> Arc<MobileCore> {
    let sink: Arc<dyn EventSink> = Arc::clone(sink) as Arc<dyn EventSink>;
    MobileCore::with_transport(transport.clone(), sink).expect("the actor starts")
}

#[test]
fn a_connect_is_answered_and_its_state_arrives_on_the_actor_thread() {
    let transport = FakeTransport::new();
    let (sink, events) = recorder();
    let core = core(&transport, &sink);

    assert_eq!(block_on(core.connect(machine("m1", "box"))), Ok(()));

    let (thread_id, name, event) = events.recv_timeout(WAIT).expect("an event");
    assert_ne!(thread_id, thread::current().id());
    assert_eq!(name.as_deref(), Some(THREAD_NAME));
    assert_eq!(
        event,
        MobileEvent::MachineState {
            machine_id: "m1".to_owned(),
            state: MachineState::Connected
        }
    );
    assert_eq!(transport.connects(), ["m1"]);
}

#[test]
fn a_refused_connect_fails_with_the_reason_and_reports_unreachable() {
    let transport = FakeTransport::new();
    transport.refuse("box", "connection refused");
    let (sink, events) = recorder();
    let core = core(&transport, &sink);

    let answer = block_on(core.connect(machine("m1", "box")));

    assert_eq!(
        answer,
        Err(MobileError::Unreachable {
            reason: "connection refused".to_owned()
        })
    );
    let (_, _, event) = events.recv_timeout(WAIT).expect("an event");
    assert_eq!(
        event,
        MobileEvent::MachineState {
            machine_id: "m1".to_owned(),
            state: MachineState::Unreachable {
                reason: "connection refused".to_owned()
            },
        }
    );
}

#[test]
fn connecting_a_connected_machine_again_reuses_its_connection() {
    let transport = FakeTransport::new();
    let (sink, events) = recorder();
    let core = core(&transport, &sink);

    assert_eq!(block_on(core.connect(machine("m1", "box"))), Ok(()));
    assert_eq!(block_on(core.connect(machine("m1", "box"))), Ok(()));

    assert_eq!(transport.connects(), ["m1"]);
    events.recv_timeout(WAIT).expect("the first connect's event");
    assert!(events.try_recv().is_err(), "the second connect changes nothing");
}

#[test]
fn connecting_a_connected_machine_as_edited_replaces_its_connection() {
    let transport = FakeTransport::new();
    let (sink, events) = recorder();
    let core = core(&transport, &sink);
    transport.refuse("new", "connection refused");

    assert_eq!(block_on(core.connect(machine("m1", "old"))), Ok(()));
    let answer = block_on(core.connect(machine("m1", "new")));

    assert!(matches!(answer, Err(MobileError::Unreachable { .. })), "{answer:?}");
    assert_eq!(transport.closed(), ["m1"], "the old host's connection is closed");
    assert_eq!(transport.connects(), ["m1", "m1"]);
    assert_eq!(events.recv_timeout(WAIT).expect("connected").2, connected("m1"));
    let unreachable = events.recv_timeout(WAIT).expect("unreachable").2;
    assert!(
        matches!(
            unreachable,
            MobileEvent::MachineState {
                state: MachineState::Unreachable { .. },
                ..
            }
        ),
        "{unreachable:?}"
    );
}

#[test]
fn connecting_a_connected_machine_only_renamed_keeps_its_connection() {
    let transport = FakeTransport::new();
    let (sink, events) = recorder();
    let core = core(&transport, &sink);

    assert_eq!(block_on(core.connect(machine("m1", "host"))), Ok(()));
    let renamed = Machine {
        label: "build box".to_owned(),
        ..machine("m1", "host")
    };
    assert_eq!(block_on(core.connect(renamed)), Ok(()));

    assert!(transport.closed().is_empty(), "the connection stays");
    assert_eq!(transport.connects(), ["m1"]);
    assert_eq!(events.recv_timeout(WAIT).expect("connected").2, connected("m1"));
    assert!(events.try_recv().is_err(), "a rename changes no state");

    let moved = Machine {
        port: 2222,
        ..machine("m1", "host")
    };
    assert_eq!(block_on(core.connect(moved)), Ok(()));
    assert_eq!(transport.closed(), ["m1"], "another port is another machine");
}

#[test]
fn a_disconnect_closes_the_connection_and_a_later_connect_opens_a_new_one() {
    let transport = FakeTransport::new();
    let (sink, _events) = recorder();
    let core = core(&transport, &sink);

    assert_eq!(block_on(core.connect(machine("m1", "box"))), Ok(()));
    assert_eq!(core.disconnect("m1".to_owned()), Ok(()));
    // Commands run in order: the reconnect's answer means the disconnect ran.
    assert_eq!(block_on(core.connect(machine("m1", "box"))), Ok(()));

    assert_eq!(transport.closed(), ["m1"]);
    assert_eq!(transport.connects(), ["m1", "m1"]);
}

#[test]
fn dropping_the_core_joins_the_actor_and_closes_its_connections() {
    let transport = FakeTransport::new();
    let (sink, _events) = recorder();
    let core = core(&transport, &sink);
    assert_eq!(block_on(core.connect(machine("m1", "box"))), Ok(()));
    assert_eq!(Arc::strong_count(&sink), 2, "the actor holds the sink");

    drop(core);

    // Joined: the actor's state, its sink clone with it, is gone by the time drop returns.
    assert_eq!(Arc::strong_count(&sink), 1);
    assert_eq!(transport.closed(), ["m1"]);
}

fn connected(id: &str) -> MobileEvent {
    MobileEvent::MachineState {
        machine_id: id.to_owned(),
        state: MachineState::Connected,
    }
}

/// Starts `connect`: its first poll posts the command, then it waits on the actor's answer.
fn post<F: Future + Unpin>(connect: &mut F) {
    assert!(block_on(poll_once(connect)).is_none(), "the connect is held");
}

#[test]
fn a_disconnect_posted_behind_a_connect_cancels_it() {
    let transport = FakeTransport::new();
    transport.hold("box");
    let (sink, events) = recorder();
    let core = core(&transport, &sink);

    let mut connect = Box::pin(core.connect(machine("m1", "box")));
    post(&mut connect);
    assert_eq!(core.disconnect("m1".to_owned()), Ok(()));
    transport.release("box");

    assert_eq!(block_on(connect), Err(MobileError::Cancelled));
    // In order: the reconnect's answer means the cancelled connect is settled, and it kept nothing.
    assert_eq!(block_on(core.connect(machine("m1", "box"))), Ok(()));
    assert_eq!(
        events.recv_timeout(WAIT).expect("the reconnect's event").2,
        connected("m1")
    );
    assert!(events.try_recv().is_err(), "the cancelled connect reports nothing");
}

#[test]
fn a_connect_of_an_edited_machine_after_a_disconnect_wins_over_the_old_one_in_flight() {
    let transport = FakeTransport::new();
    transport.hold("old");
    let (sink, events) = recorder();
    let core = core(&transport, &sink);

    let mut old = Box::pin(core.connect(machine("m1", "old")));
    post(&mut old);
    assert_eq!(core.disconnect("m1".to_owned()), Ok(()));
    // The old host is still held: the answer comes from the new one.
    assert_eq!(block_on(core.connect(machine("m1", "new"))), Ok(()));
    assert_eq!(block_on(old), Err(MobileError::Cancelled));
    transport.release("old");

    // Still on the new host's connection: connecting again reuses it.
    assert_eq!(block_on(core.connect(machine("m1", "new"))), Ok(()));
    assert!(transport.closed().is_empty());
    assert_eq!(events.recv_timeout(WAIT).expect("an event").2, connected("m1"));
    assert!(events.try_recv().is_err(), "one connection, one event");
}

#[test]
fn a_connect_of_an_edited_machine_replaces_the_one_in_flight() {
    let transport = FakeTransport::new();
    transport.hold("old");
    let (sink, _events) = recorder();
    let core = core(&transport, &sink);

    let mut old = Box::pin(core.connect(machine("m1", "old")));
    post(&mut old);

    assert_eq!(block_on(core.connect(machine("m1", "new"))), Ok(()));
    assert_eq!(block_on(old), Err(MobileError::Cancelled));
}

#[test]
fn a_failed_connect_overtaken_by_a_disconnect_is_cancelled_and_reports_nothing() {
    let transport = FakeTransport::new();
    transport.refuse("box", "connection refused");
    transport.hold("box");
    let (sink, events) = recorder();
    let core = core(&transport, &sink);

    let mut connect = Box::pin(core.connect(machine("m1", "box")));
    post(&mut connect);
    assert_eq!(core.disconnect("m1".to_owned()), Ok(()));
    transport.release("box");

    assert_eq!(block_on(connect), Err(MobileError::Cancelled));
    // In order: once this answers, the cancelled connect can't report any more.
    assert!(block_on(core.connect(machine("m2", "other"))).is_ok());
    assert_eq!(events.recv_timeout(WAIT).expect("m2's event").2, connected("m2"));
    assert!(events.try_recv().is_err(), "no Unreachable for a machine disconnected");
}

#[test]
fn connects_of_a_machine_in_flight_share_one_transport_connect() {
    let transport = FakeTransport::new();
    transport.hold("box");
    let (sink, events) = recorder();
    let core = core(&transport, &sink);

    let mut first = Box::pin(core.connect(machine("m1", "box")));
    post(&mut first);
    let mut second = Box::pin(core.connect(machine("m1", "box")));
    post(&mut second);
    transport.release("box");

    assert_eq!(block_on(first), Ok(()));
    assert_eq!(block_on(second), Ok(()));
    assert_eq!(transport.connects(), ["m1"]);
    assert_eq!(events.recv_timeout(WAIT).expect("an event").2, connected("m1"));
    assert!(events.try_recv().is_err(), "one connect, one event");
}

#[test]
fn a_failed_connect_answers_every_caller_waiting_on_it() {
    let transport = FakeTransport::new();
    transport.refuse("box", "connection refused");
    transport.hold("box");
    let (sink, _events) = recorder();
    let core = core(&transport, &sink);

    let mut first = Box::pin(core.connect(machine("m1", "box")));
    post(&mut first);
    let mut second = Box::pin(core.connect(machine("m1", "box")));
    post(&mut second);
    transport.release("box");

    let refused = Err(MobileError::Unreachable {
        reason: "connection refused".to_owned(),
    });
    assert_eq!(block_on(first), refused);
    assert_eq!(block_on(second), refused);
}

/// Drops the core it holds from its first event, on the actor's own thread.
struct Dropper {
    core: Mutex<Option<Arc<MobileCore>>>,
    done: Mutex<Sender<()>>,
}

impl EventSink for Dropper {
    fn on_event(&self, _event: MobileEvent) {
        let core = self.core.lock().unwrap_or_else(PoisonError::into_inner).take();
        drop(core);
        let _ = self.done.lock().unwrap_or_else(PoisonError::into_inner).send(());
    }
}

#[test]
fn dropping_the_core_from_an_event_on_the_actor_does_not_deadlock() {
    let transport = FakeTransport::new();
    let (done, finished) = mpsc::channel();
    let dropper = Arc::new(Dropper {
        core: Mutex::new(None),
        done: Mutex::new(done),
    });
    let sink: Arc<dyn EventSink> = Arc::clone(&dropper) as Arc<dyn EventSink>;
    let core = MobileCore::with_transport(transport.clone(), sink).expect("the actor starts");
    let connect = core.connect(machine("m1", "box"));
    *dropper.core.lock().unwrap_or_else(PoisonError::into_inner) = Some(Arc::clone(&core));

    assert_eq!(block_on(connect), Ok(()));
    drop(core);

    finished
        .recv_timeout(WAIT)
        .expect("the sink dropped the last core on the actor");
    // The detached actor finishes by itself: its sink clone goes away.
    let deadline = std::time::Instant::now() + WAIT;
    while Arc::strong_count(&dropper) > 1 {
        assert!(std::time::Instant::now() < deadline, "the actor never finished");
        thread::yield_now();
    }
}

// MARK: - sessions

const PARENT: &str = "00000000-0000-0000-0000-0000000000A1";
const COMPANION: &str = "00000000-0000-0000-0000-0000000000A2";
const OTHER: &str = "00000000-0000-0000-0000-0000000000A3";

fn listing(sessions: &[(&str, Option<&str>)]) -> crate::transport::fake::Script {
    let sessions: Vec<serde_json::Value> = sessions
        .iter()
        .map(|(id, parent)| {
            serde_json::json!({
                "id": id, "name": id, "windowID": "w1", "windowName": "main", "workspaceID": "ws1",
                "workspaceName": "work", "cwd": "/repo", "panes": [], "parent": parent,
            })
        })
        .collect();
    let body = serde_json::json!({"ok": true, "result": {"remote": {
        "endpoint": {"executable": "/ngterm-session", "socketDirectory": "/tmp/ngterm-live-x"},
        "sessions": sessions,
    }}});
    crate::transport::fake::Script::ok(&body.to_string())
}

fn ids(sessions: &[crate::sessions::RemoteSession]) -> Vec<&str> {
    sessions.iter().map(|session| session.id.as_str()).collect()
}

/// The next event, skipping the machine's connection state.
fn next_tree(events: &Receiver<Received>) -> (String, Vec<crate::sessions::RemoteSession>) {
    loop {
        match events.recv_timeout(WAIT).expect("an event").2 {
            MobileEvent::Tree { machine_id, sessions } => return (machine_id, sessions),
            MobileEvent::MachineState { .. } => {}
            other => panic!("unexpected {other:?}"),
        }
    }
}

#[test]
fn session_commands_need_a_connection() {
    let transport = FakeTransport::new();
    let (sink, _events) = recorder();
    let core = core(&transport, &sink);
    let not_connected = Err(MobileError::NotConnected {
        machine_id: "m1".to_owned(),
    });

    assert_eq!(block_on(core.list_sessions("m1".to_owned())).map(|_| ()), not_connected);
    assert_eq!(
        block_on(core.create_session("m1".to_owned(), None)).map(|_| ()),
        not_connected
    );
    assert_eq!(
        block_on(core.duplicate_session("m1".to_owned(), PARENT.to_owned())).map(|_| ()),
        not_connected
    );
    assert_eq!(
        block_on(core.end_session("m1".to_owned(), PARENT.to_owned())),
        not_connected
    );
    assert!(transport.execs().is_empty());
}

#[test]
fn a_listing_is_answered_and_arrives_as_a_tree() {
    let transport = FakeTransport::new();
    let tree = ngterm_core::remote::ssh::tree_tail(ngterm_core::remote::origin::OriginShell::Posix);
    transport.script(&tree, listing(&[(COMPANION, Some(PARENT)), (PARENT, None)]));
    let (sink, events) = recorder();
    let core = core(&transport, &sink);
    block_on(core.connect(machine("m1", "box"))).expect("connected");

    let sessions = block_on(core.list_sessions("m1".to_owned())).expect("listed");

    assert_eq!(ids(&sessions), [PARENT, COMPANION]);
    let (machine_id, event) = next_tree(&events);
    assert_eq!(machine_id, "m1");
    assert_eq!(event, sessions);
}

#[test]
fn a_remote_failure_names_the_machine_by_its_label() {
    let transport = FakeTransport::new();
    let tree = ngterm_core::remote::ssh::tree_tail(ngterm_core::remote::origin::OriginShell::Posix);
    transport.script(&tree, crate::transport::fake::Script::failing(1, ""));
    let (sink, _events) = recorder();
    let core = core(&transport, &sink);
    let mut studio = machine("m1", "box.example");
    studio.label = "Studio".to_owned();
    block_on(core.connect(studio)).expect("connected");

    assert_eq!(
        block_on(core.list_sessions("m1".to_owned())),
        Err(MobileError::Remote {
            reason: "the remote command failed on Studio".to_owned()
        })
    );
}

#[test]
fn a_duplicate_answers_the_copy_and_its_listing_arrives_as_a_tree() {
    use ngterm_core::remote::origin::OriginShell;
    use ngterm_core::remote::origin_replies::{RESTORE_MODE_ARGUMENTS, duplicate_session_arguments};
    use ngterm_core::remote::ssh::{control_tail, tree_tail, tree_tail_for};
    let transport = FakeTransport::new();
    let live = r#"{"ok": true, "result": {"restore": {"configured": "live", "requestedAtLaunch": "live",
        "active": "live", "restartRequired": false}}}"#;
    transport.script(
        &control_tail(OriginShell::Posix, &RESTORE_MODE_ARGUMENTS),
        crate::transport::fake::Script::ok(live),
    );
    transport.script(
        &control_tail(OriginShell::Posix, &duplicate_session_arguments(PARENT)),
        crate::transport::fake::Script::ok(&format!(r#"{{"ok": true, "result": {{"id": "{OTHER}"}}}}"#)),
    );
    let attachable = |id: &str| {
        serde_json::json!({
            "id": id, "name": id, "windowID": "w1", "windowName": "main", "workspaceID": "ws1",
            "workspaceName": "work", "cwd": "/repo", "panes": [{"pane": "left", "daemon": "ngterm-live-0"}],
        })
    };
    let body = serde_json::json!({"ok": true, "result": {"remote": {
        "endpoint": {"executable": "/ngterm-session", "socketDirectory": "/tmp/ngterm-live-x"},
        "sessions": [attachable(PARENT), attachable(OTHER)],
    }}});
    for tail in [
        tree_tail(OriginShell::Posix),
        tree_tail_for(OriginShell::Posix, Some(ngterm_core::remote::peer::RemotePeer::Ngterm)),
    ] {
        transport.script(&tail, crate::transport::fake::Script::ok(&body.to_string()));
    }
    let (sink, events) = recorder();
    let core = core(&transport, &sink);
    block_on(core.connect(machine("m1", "box"))).expect("connected");

    let copy = block_on(core.duplicate_session("m1".to_owned(), PARENT.to_owned())).expect("duplicated");

    assert_eq!(copy.id, OTHER);
    assert_eq!(copy.peer, Some(crate::peer::PeerKind::Ngterm));
    let (machine_id, sessions) = next_tree(&events);
    assert_eq!(machine_id, "m1");
    assert_eq!(ids(&sessions), [PARENT, OTHER]);
}

/// A session made on an agterm Mac comes back marked agterm's, as its listing marks it.
#[test]
fn a_create_on_an_agterm_mac_answers_a_session_marked_agterm() {
    use ngterm_core::remote::origin::{OriginForm, OriginShell};
    use ngterm_core::remote::origin_replies::{NEW_SESSION_ARGUMENTS, RESTORE_MODE_ARGUMENTS};
    use ngterm_core::remote::peer::{AGTERM_ANSWER, RemotePeer};
    use ngterm_core::remote::ssh::{control_tail, tree_tail, tree_tail_for};
    let transport = FakeTransport::new();
    let agterm = OriginForm {
        shell: OriginShell::Posix,
        peer: RemotePeer::Agterm,
    };
    let live = r#"{"ok": true, "result": {"restore": {"configured": "live", "requestedAtLaunch": "live",
        "active": "live", "restartRequired": false}}}"#;
    transport.script(
        &control_tail(agterm, &RESTORE_MODE_ARGUMENTS),
        crate::transport::fake::Script::ok(live),
    );
    transport.script(
        &control_tail(agterm, &NEW_SESSION_ARGUMENTS),
        crate::transport::fake::Script::ok(&format!(r#"{{"ok": true, "result": {{"id": "{OTHER}"}}}}"#)),
    );
    let body = serde_json::json!({"ok": true, "result": {"remote": {
        "endpoint": {"executable": "/Applications/agterm.app/Contents/MacOS/zmx", "socketDirectory": "/tmp/z"},
        "sessions": [{"id": OTHER, "name": "new", "windowID": "w1", "windowName": "main", "workspaceID": "ws1",
            "workspaceName": "work", "cwd": "/repo",
            "panes": [{"pane": "left", "daemon": format!("agterm-{}", "0".repeat(32))}]}],
    }}});
    // the wait for the created session lists agterm alone
    for tail in [
        tree_tail(OriginShell::Posix),
        tree_tail_for(OriginShell::Posix, Some(RemotePeer::Agterm)),
    ] {
        transport.script(
            &tail,
            crate::transport::fake::Script::ok(&format!("{AGTERM_ANSWER}\n{body}")),
        );
    }
    let (sink, _events) = recorder();
    let core = core(&transport, &sink);
    block_on(core.connect(machine("m1", "box"))).expect("connected");

    let created = block_on(core.create_session("m1".to_owned(), None)).expect("created");

    assert_eq!(created.id, OTHER);
    assert_eq!(created.peer, Some(crate::peer::PeerKind::Agterm));
}

#[test]
fn an_ended_session_leaves_the_tree_with_its_companions() {
    let transport = FakeTransport::new();
    let tree = ngterm_core::remote::ssh::tree_tail(ngterm_core::remote::origin::OriginShell::Posix);
    transport.script(
        &tree,
        listing(&[(PARENT, None), (COMPANION, Some(PARENT)), (OTHER, None)]),
    );
    let close = ngterm_core::remote::ssh::control_tail(
        ngterm_core::remote::origin::OriginShell::Posix,
        &ngterm_core::remote::origin_replies::end_session_arguments(PARENT),
    );
    transport.script(
        &close,
        crate::transport::fake::Script::ok(r#"{"ok": true, "result": {}}"#),
    );
    let (sink, events) = recorder();
    let core = core(&transport, &sink);
    block_on(core.connect(machine("m1", "box"))).expect("connected");
    block_on(core.list_sessions("m1".to_owned())).expect("listed");
    next_tree(&events);

    assert_eq!(block_on(core.end_session("m1".to_owned(), PARENT.to_owned())), Ok(()));

    let (_, sessions) = next_tree(&events);
    assert_eq!(ids(&sessions), [OTHER]);
}

#[test]
fn a_disconnect_ends_a_session_command_running_on_the_connection() {
    let transport = FakeTransport::new();
    let (sink, _events) = recorder();
    let core = core(&transport, &sink);
    block_on(core.connect(machine("m1", "box"))).expect("connected");
    // the listing is unscripted: it runs until the connection closes under it
    let listing = core.list_sessions("m1".to_owned());
    let mut listing = std::pin::pin!(listing);
    assert!(block_on(poll_once(listing.as_mut())).is_none());
    let deadline = std::time::Instant::now() + WAIT;
    while transport.execs().is_empty() {
        assert!(std::time::Instant::now() < deadline, "the listing never ran");
        thread::yield_now();
    }
    let opened = transport.take_opened();

    core.disconnect("m1".to_owned()).expect("posted");

    assert_eq!(
        block_on(listing),
        Err(MobileError::Remote {
            reason: "the connection is closed".to_owned()
        })
    );
    drop(opened);
}

#[test]
fn a_listing_that_began_before_an_end_answers_but_is_neither_kept_nor_reported() {
    let transport = FakeTransport::new();
    let close = ngterm_core::remote::ssh::control_tail(
        ngterm_core::remote::origin::OriginShell::Posix,
        &ngterm_core::remote::origin_replies::end_session_arguments(PARENT),
    );
    transport.script(
        &close,
        crate::transport::fake::Script::ok(r#"{"ok": true, "result": {}}"#),
    );
    let (sink, events) = recorder();
    let core = core(&transport, &sink);
    block_on(core.connect(machine("m1", "box"))).expect("connected");
    // the first listing is unscripted: it reads the tree while PARENT still exists, and answers after the end
    let first = core.list_sessions("m1".to_owned());
    let mut first = std::pin::pin!(first);
    assert!(block_on(poll_once(first.as_mut())).is_none());
    let deadline = std::time::Instant::now() + WAIT;
    let opened = loop {
        let mut opened = transport.take_opened();
        if let Some(opened) = opened.pop() {
            break opened;
        }
        assert!(std::time::Instant::now() < deadline, "the listing never ran");
        thread::yield_now();
    };

    // nothing has told this fresh connection's app yet, so the end lists it first, a listing no one reports
    transport.script(
        &ngterm_core::remote::ssh::tree_tail(ngterm_core::remote::origin::OriginShell::Posix),
        listing(&[(PARENT, None), (OTHER, None)]),
    );
    assert_eq!(block_on(core.end_session("m1".to_owned(), PARENT.to_owned())), Ok(()));
    let old = listing(&[(PARENT, None), (OTHER, None)]);
    block_on(opened.peer.data(old.stdout));
    block_on(opened.peer.end(old.end));

    let answered = block_on(first).expect("listed");
    assert_eq!(ids(&answered), [PARENT, OTHER]);
    // a later listing is kept and reported, and it is the first tree the app sees
    transport.script(
        &ngterm_core::remote::ssh::tree_tail(ngterm_core::remote::origin::OriginShell::Posix),
        listing(&[(OTHER, None)]),
    );
    block_on(core.list_sessions("m1".to_owned())).expect("listed again");
    let (_, sessions) = next_tree(&events);
    assert_eq!(ids(&sessions), [OTHER]);
}

/// A listing of `sessions` as the far side answers it.
fn tree_of(sessions: &[(&str, Option<&str>)]) -> ngterm_proto::ControlRemoteTree {
    let script = listing(sessions);
    let result = ngterm_core::remote::tree::RemoteCommandResult {
        status: 0,
        stdout: String::from_utf8(script.stdout).expect("utf-8"),
        stderr: String::new(),
    };
    ngterm_core::remote::origin_replies::read_remote_tree(&result, "box").expect("a tree")
}

#[test]
fn a_listing_read_on_a_replaced_connection_answers_but_is_neither_kept_nor_reported() {
    use crate::transport::Transport as _;

    let transport = FakeTransport::new();
    let (sink, events) = recorder();
    let state = super::State::new(
        transport.clone(),
        Arc::clone(&sink) as Arc<dyn EventSink>,
        tokio::sync::mpsc::unbounded_channel().0,
        super::Wiring::detached(),
    );
    let machine = machine("m1", "box");
    let old = std::rc::Rc::new(block_on(transport.connect(&machine)).expect("connected"));
    let new = std::rc::Rc::new(block_on(transport.connect(&machine)).expect("connected"));
    state.connections.borrow_mut().insert(
        "m1".to_owned(),
        super::Link {
            machine,
            connection: std::rc::Rc::clone(&new),
        },
    );
    let on = |connection: &std::rc::Rc<_>| super::OnMachine {
        connection: std::rc::Rc::clone(connection),
        host: "m1".to_owned(),
    };

    let overtaken = super::listed(
        &state,
        &on(&old),
        "m1".to_owned(),
        tree_of(&[(PARENT, None)]),
        state.ticket(),
    );

    assert_eq!(ids(&overtaken), [PARENT]);
    assert!(events.try_recv().is_err());
    assert!(state.listings.borrow().is_empty());

    let current = super::listed(
        &state,
        &on(&new),
        "m1".to_owned(),
        tree_of(&[(OTHER, None)]),
        state.ticket(),
    );

    assert_eq!(ids(&current), [OTHER]);
    let (_, sessions) = next_tree(&events);
    assert_eq!(sessions, current);
}

#[test]
fn a_listing_overtaken_by_a_later_one_is_neither_kept_nor_reported() {
    use crate::transport::Transport as _;

    let transport = FakeTransport::new();
    let (sink, events) = recorder();
    let state = super::State::new(
        transport.clone(),
        Arc::clone(&sink) as Arc<dyn EventSink>,
        tokio::sync::mpsc::unbounded_channel().0,
        super::Wiring::detached(),
    );
    let machine = machine("m1", "box");
    let connection = std::rc::Rc::new(block_on(transport.connect(&machine)).expect("connected"));
    state.connections.borrow_mut().insert(
        "m1".to_owned(),
        super::Link {
            machine,
            connection: std::rc::Rc::clone(&connection),
        },
    );
    let on = super::OnMachine {
        connection,
        host: "m1".to_owned(),
    };
    let first = state.ticket();
    let second = state.ticket();

    super::listed(&state, &on, "m1".to_owned(), tree_of(&[(OTHER, None)]), second);
    let overtaken = super::listed(&state, &on, "m1".to_owned(), tree_of(&[(PARENT, None)]), first);

    assert_eq!(ids(&overtaken), [PARENT]);
    let (_, sessions) = next_tree(&events);
    assert_eq!(ids(&sessions), [OTHER]);
    assert!(events.try_recv().is_err());
    let kept = state.listings.borrow();
    let tree = kept.get("m1").and_then(|kept| kept.tree.as_ref()).expect("kept");
    assert_eq!(ids(&crate::sessions::listed_sessions(tree)), [OTHER]);
}
