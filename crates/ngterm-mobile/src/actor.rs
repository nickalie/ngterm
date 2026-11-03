//! The core's actor: one thread per [`crate::MobileCore`] running a current-thread tokio runtime and a `LocalSet`. It
//! owns the transport and the machines' connections (and, as the crate grows, the `!Send` presentation clients), so
//! none of them needs to be `Send` or locked. The facade posts `Command`s to it; a command with an answer carries a
//! oneshot the facade awaits, and the actor reports what it learns on its own through the [`EventSink`].
//!
//! Commands are handled in the order posted. A connect settles on the loop what it can at once (already connected,
//! joining a connect in flight, closing the connection of a machine since edited) and only waits on the network in a local task, so the queue keeps moving behind it
//! and a later disconnect cancels that task.

#[cfg(test)]
mod tests;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::thread::{self, JoinHandle, ThreadId};

use tokio::runtime::Builder;
use tokio::sync::{mpsc, oneshot};
use tokio::task::{AbortHandle, LocalSet};

use crate::api::{EventSink, Machine, MachineState, MobileError, MobileEvent};
use crate::transport::{Connection, Transport};

/// The actor thread's name.
pub const THREAD_NAME: &str = "ngterm-mobile";

/// What the facade asks of the actor.
pub(crate) enum Command {
    Connect {
        machine: Machine,
        reply: Reply,
    },
    Disconnect {
        machine_id: String,
    },
    AnswerHostKey {
        machine_id: String,
        fingerprint: String,
        trust: bool,
    },
}

/// The facade's handle on the actor thread: dropping it stops the thread and joins it.
pub(crate) struct Actor {
    commands: Option<mpsc::UnboundedSender<Command>>,
    thread: Option<JoinHandle<()>>,
    thread_id: ThreadId,
}

impl Actor {
    /// Starts the actor over `transport`, reporting to `sink`.
    pub(crate) fn spawn<T>(transport: T, sink: Arc<dyn EventSink>) -> Result<Self, MobileError>
    where
        T: Transport + Send,
    {
        // Time and IO: the russh transport's sockets, timeouts and keepalives.
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| MobileError::Stopped)?;
        let (commands, queue) = mpsc::unbounded_channel();
        let thread = thread::Builder::new()
            .name(THREAD_NAME.to_owned())
            .spawn(move || {
                let local = LocalSet::new();
                local.block_on(&runtime, run(transport, sink, queue));
            })
            .map_err(|_| MobileError::Stopped)?;
        let thread_id = thread.thread().id();
        Ok(Self {
            commands: Some(commands),
            thread: Some(thread),
            thread_id,
        })
    }

    /// Queues `command`.
    pub(crate) fn post(&self, command: Command) -> Result<(), MobileError> {
        let commands = self.commands.as_ref().ok_or(MobileError::Stopped)?;
        commands.send(command).map_err(|_| MobileError::Stopped)
    }
}

impl Drop for Actor {
    fn drop(&mut self) {
        // Closing the queue ends the actor's loop.
        self.commands = None;
        let Some(thread) = self.thread.take() else { return };
        // Dropped from an event callback on the actor itself: joining would wait for this very call.
        if thread::current().id() != self.thread_id {
            // A panic on the actor has already been reported by its hook; nothing is left to clean up.
            let _ = thread.join();
        }
    }
}

/// The actor's state, shared by its loop and its local tasks. No borrow is held across an await.
struct State<T: Transport> {
    transport: T,
    sink: Arc<dyn EventSink>,
    /// The live connections, each with the machine record it was made from.
    connections: RefCell<HashMap<String, Link<T::Connection>>>,
    /// The connects in flight, one per machine: a later connect of the same machine joins it, a disconnect or a
    /// connect of the machine as edited cancels it.
    attempts: RefCell<HashMap<String, Attempt>>,
    /// The last attempt's number: an attempt that finishes no longer current was overtaken and changes nothing.
    generation: Cell<u64>,
}

/// A live connection and the machine record it was made from: a connect of the machine as edited replaces it.
struct Link<C> {
    machine: Machine,
    connection: C,
}

/// One connect in flight.
struct Attempt {
    generation: u64,
    machine: Machine,
    /// The callers waiting on it.
    waiters: Vec<Reply>,
    task: AbortHandle,
}

type Reply = oneshot::Sender<Result<(), MobileError>>;

impl<T: Transport> State<T> {
    fn emit(&self, event: MobileEvent) {
        self.sink.on_event(event);
    }
}

async fn run<T: Transport>(transport: T, sink: Arc<dyn EventSink>, mut queue: mpsc::UnboundedReceiver<Command>) {
    let state = Rc::new(State {
        transport,
        sink,
        connections: RefCell::new(HashMap::new()),
        attempts: RefCell::new(HashMap::new()),
        generation: Cell::new(0),
    });
    while let Some(command) = queue.recv().await {
        match command {
            Command::Connect { machine, reply } => connect(&state, machine, reply),
            Command::Disconnect { machine_id } => disconnect(&state, &machine_id),
            Command::AnswerHostKey {
                machine_id,
                fingerprint,
                trust,
            } => state.transport.answer_host_key(&machine_id, &fingerprint, trust),
        }
    }
    // Waiters dropped unanswered read as `Stopped`.
    for (_, attempt) in state.attempts.borrow_mut().drain() {
        attempt.task.abort();
    }
    let connections = std::mem::take(&mut *state.connections.borrow_mut());
    for link in connections.values() {
        link.connection.close();
    }
}

/// Settles what a connect can at once, on the loop, so a command posted after it sees its effect; only the network
/// part runs as a local task.
fn connect<T: Transport>(state: &Rc<State<T>>, machine: Machine, reply: Reply) {
    let connected = state
        .connections
        .borrow()
        .get(&machine.id)
        .map(|link| link.machine == machine);
    match connected {
        Some(true) => {
            let _ = reply.send(Ok(()));
            return;
        }
        // A connect of the machine as edited replaces its connection: the new one reports its own state.
        Some(false) => close(state, &machine.id),
        None => {}
    }
    if let Some(attempt) = state.attempts.borrow_mut().get_mut(&machine.id)
        && attempt.machine == machine
    {
        attempt.waiters.push(reply);
        return;
    }
    // A connect of the machine as edited replaces the one in flight.
    cancel(state, &machine.id);
    let generation = state.generation.get() + 1;
    state.generation.set(generation);
    let task = tokio::task::spawn_local(reach(Rc::clone(state), machine.clone(), generation)).abort_handle();
    let id = machine.id.clone();
    let attempt = Attempt {
        generation,
        machine,
        waiters: vec![reply],
        task,
    };
    state.attempts.borrow_mut().insert(id, attempt);
}

/// The network part of a connect: aborted when the attempt is cancelled.
async fn reach<T: Transport>(state: Rc<State<T>>, machine: Machine, generation: u64) {
    let result = state.transport.connect(&machine).await;
    let current = state
        .attempts
        .borrow()
        .get(&machine.id)
        .map(|attempt| attempt.generation)
        == Some(generation);
    if !current {
        // Overtaken: its callers were answered when it was.
        if let Ok(connection) = result {
            connection.close();
        }
        return;
    }
    let Some(attempt) = state.attempts.borrow_mut().remove(&machine.id) else {
        return;
    };
    let answer = match result {
        Ok(connection) => {
            let link = Link {
                machine: machine.clone(),
                connection,
            };
            state.connections.borrow_mut().insert(machine.id.clone(), link);
            state.emit(MobileEvent::MachineState {
                machine_id: machine.id.clone(),
                state: MachineState::Connected,
            });
            Ok(())
        }
        Err(error) => {
            let reason = error.to_string();
            state.emit(MobileEvent::MachineState {
                machine_id: machine.id.clone(),
                state: MachineState::Unreachable { reason: reason.clone() },
            });
            Err(MobileError::Unreachable { reason })
        }
    };
    for waiter in attempt.waiters {
        // A caller that stopped waiting has nothing to be told.
        let _ = waiter.send(answer.clone());
    }
}

/// Cancels `machine_id`'s connect in flight, if any: its network work stops and its callers are answered
/// [`MobileError::Cancelled`].
fn cancel<T: Transport>(state: &State<T>, machine_id: &str) {
    let attempt = state.attempts.borrow_mut().remove(machine_id);
    if let Some(attempt) = attempt {
        attempt.task.abort();
        for waiter in attempt.waiters {
            let _ = waiter.send(Err(MobileError::Cancelled));
        }
    }
}

fn disconnect<T: Transport>(state: &State<T>, machine_id: &str) {
    cancel(state, machine_id);
    close(state, machine_id);
}

/// Closes `machine_id`'s connection, if any.
fn close<T: Transport>(state: &State<T>, machine_id: &str) {
    let link = state.connections.borrow_mut().remove(machine_id);
    if let Some(link) = link {
        link.connection.close();
    }
}
