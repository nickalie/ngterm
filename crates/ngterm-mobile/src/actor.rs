//! The core's actor: one thread per [`crate::MobileCore`] running a current-thread tokio runtime and a `LocalSet`. It
//! owns the transport and the machines' connections (and, as the crate grows, the `!Send` presentation clients), so
//! none of them needs to be `Send` or locked. The facade posts `Command`s to it; a command with an answer carries a
//! oneshot the facade awaits, and the actor reports what it learns on its own through the [`EventSink`].
//!
//! Commands are handled in the order posted. One that waits on the network (a connect) runs as a local task, so the
//! queue keeps moving behind it.

#[cfg(test)]
mod tests;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::thread::{self, JoinHandle, ThreadId};

use tokio::runtime::Builder;
use tokio::sync::{mpsc, oneshot};
use tokio::task::LocalSet;

use crate::api::{EventSink, Machine, MachineState, MobileError, MobileEvent};
use crate::transport::{Connection, Transport};

/// The actor thread's name.
pub const THREAD_NAME: &str = "ngterm-mobile";

/// What the facade asks of the actor.
pub(crate) enum Command {
    Connect {
        machine: Machine,
        reply: oneshot::Sender<Result<(), MobileError>>,
    },
    Disconnect {
        machine_id: String,
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
        let runtime = Builder::new_current_thread()
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
    connections: RefCell<HashMap<String, T::Connection>>,
    /// The machines a connect was asked for and no disconnect came after: a connect that completes for a machine no
    /// longer here is closed again.
    wanted: RefCell<HashSet<String>>,
}

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
        wanted: RefCell::new(HashSet::new()),
    });
    while let Some(command) = queue.recv().await {
        match command {
            Command::Connect { machine, reply } => {
                tokio::task::spawn_local(connect(Rc::clone(&state), machine, reply));
            }
            Command::Disconnect { machine_id } => disconnect(&state, &machine_id),
        }
    }
    let connections = std::mem::take(&mut *state.connections.borrow_mut());
    for connection in connections.values() {
        connection.close();
    }
}

async fn connect<T: Transport>(state: Rc<State<T>>, machine: Machine, reply: oneshot::Sender<Result<(), MobileError>>) {
    state.wanted.borrow_mut().insert(machine.id.clone());
    if state.connections.borrow().contains_key(&machine.id) {
        let _ = reply.send(Ok(()));
        return;
    }
    let result = state.transport.connect(&machine).await;
    let answer = match result {
        Ok(connection) => adopt(&state, &machine.id, connection),
        Err(error) => {
            let reason = error.to_string();
            state.emit(MobileEvent::MachineState {
                machine_id: machine.id.clone(),
                state: MachineState::Unreachable { reason: reason.clone() },
            });
            Err(MobileError::Unreachable { reason })
        }
    };
    // A caller that stopped waiting has nothing to be told.
    let _ = reply.send(answer);
}

/// Keeps a fresh `connection` unless a disconnect overtook it or another connect won the race.
fn adopt<T: Transport>(state: &State<T>, machine_id: &str, connection: T::Connection) -> Result<(), MobileError> {
    if !state.wanted.borrow().contains(machine_id) {
        connection.close();
        return Err(MobileError::Cancelled);
    }
    if state.connections.borrow().contains_key(machine_id) {
        connection.close();
        return Ok(());
    }
    state.connections.borrow_mut().insert(machine_id.to_owned(), connection);
    state.emit(MobileEvent::MachineState {
        machine_id: machine_id.to_owned(),
        state: MachineState::Connected,
    });
    Ok(())
}

fn disconnect<T: Transport>(state: &State<T>, machine_id: &str) {
    state.wanted.borrow_mut().remove(machine_id);
    let connection = state.connections.borrow_mut().remove(machine_id);
    if let Some(connection) = connection {
        connection.close();
    }
}
