use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, ThreadId};
use std::time::Duration;

use futures_lite::future::block_on;

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
