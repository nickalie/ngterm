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
        pinned_host_key: None,
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
