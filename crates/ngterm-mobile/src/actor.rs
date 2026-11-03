//! The core's actor: one thread per [`crate::MobileCore`] running a current-thread tokio runtime and a `LocalSet`. It
//! owns the transport and the machines' connections (and, as the crate grows, the `!Send` presentation clients), so
//! none of them needs to be `Send` or locked. The facade posts `Command`s to it; a command with an answer carries a
//! oneshot the facade awaits, and the actor reports what it learns on its own through the [`EventSink`].
//!
//! Commands are handled in the order posted. A connect settles on the loop what it can at once (already connected,
//! joining a connect in flight, closing the connection of a machine since edited) and only waits on the network in a local task, so the queue keeps moving behind it
//! and a later disconnect cancels that task. A machine's session commands ([`crate::sessions`]) run as local tasks on
//! its connection as it was when they were posted: a disconnect closes it under them, which ends them with the
//! transport's text. They may overlap, so a listing changes the kept listing and is reported only while its connection
//! is still the machine's and nothing newer about its sessions is known: no later listing landed and no session ended
//! after it began.
//!
//! An attached session's presentation stream ([`crate::presentation`]) runs on its machine's current connection (none
//! while the machine is down); its links report back to the actor as local tasks, and a ticker task runs the streams'
//! one-second tick.
//!
//! A connection the core holds is watched: lost by itself, it is retried on the reconnect schedule as a connect
//! attempt that waits first ([`crate::reconnect`]) until it is back or refused for good
//! ([`crate::transport::TransportError::Refused`]), and the machine's sessions stay attached across it. Closing a
//! machine's connection (a disconnect, a connect of the machine as edited) ends its sessions' streams and its retries;
//! suspending closes every connection but keeps the streams to start again on resume.

#[cfg(test)]
mod presentation_tests;
#[cfg(test)]
mod reconnect_tests;
#[cfg(test)]
mod tests;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::thread::{self, JoinHandle, ThreadId};
use std::time::Duration;

use ngterm_proto::ControlRemoteTree;

use tokio::runtime::Builder;
use tokio::sync::{mpsc, oneshot};
use tokio::task::{AbortHandle, LocalSet};

use crate::api::{EventSink, Machine, MachineState, MobileError, MobileEvent};
use crate::presentation::{self, AskReply, ChannelLinks, LinkEvent, Presentations, Reporter};
use crate::reconnect::Retry;
use crate::sessions::{self, RemoteSession};
use crate::transport::{Connection, Transport, TransportError};

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
    ListSessions {
        machine_id: String,
        reply: oneshot::Sender<Result<Vec<RemoteSession>, MobileError>>,
    },
    CreateSession {
        machine_id: String,
        name: Option<String>,
        reply: oneshot::Sender<Result<RemoteSession, MobileError>>,
    },
    EndSession {
        machine_id: String,
        session_id: String,
        reply: Reply,
    },
    AttachSession {
        machine_id: String,
        session_id: String,
        reply: Reply,
    },
    DetachSession {
        machine_id: String,
        session_id: String,
    },
    AnswerAsk {
        machine_id: String,
        session_id: String,
        ask_id: String,
        reply: AskReply,
    },
    NetworkChanged,
    Suspend,
    Resume,
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
    /// What the actor knows of each connected machine's sessions.
    listings: RefCell<HashMap<String, Kept>>,
    /// The last ticket handed out ([`State::ticket`]).
    tickets: Cell<u64>,
    /// The attached sessions' presentation streams.
    presentations: RefCell<Presentations<T::Connection>>,
    /// The machines [`suspend`] set aside, to reconnect on [`resume`].
    suspended: RefCell<HashMap<String, SetAside>>,
}

/// What the actor knows of a connected machine's sessions.
#[derive(Default)]
struct Kept {
    /// Its last listing, whole (endpoint and panes included), for attaching its sessions, as the ends since left it;
    /// `None` until one is read on the connection.
    tree: Option<ControlRemoteTree>,
    /// The ticket of the newest news: the listing kept, or the last end. A listing that began before it is stale.
    as_of: u64,
}

/// A machine [`suspend`] set aside.
struct SetAside {
    machine: Machine,
    /// Whether it had been connected (connected or being reconnected then): it comes back as a reconnect, which keeps
    /// retrying. One never connected comes back as the connect the app asked for, whose failure is final.
    was_connected: bool,
}

impl SetAside {
    /// How it is connected again: as a reconnect at once, or as a first connect.
    fn retry(&self) -> Option<Retry> {
        self.was_connected.then(Retry::at_once)
    }
}

/// A live connection and the machine record it was made from: a connect of the machine as edited replaces it.
struct Link<C> {
    machine: Machine,
    /// Shared with the session commands running on it.
    connection: Rc<C>,
}

/// One connect in flight.
struct Attempt {
    generation: u64,
    machine: Machine,
    /// The callers waiting on it.
    waiters: Vec<Reply>,
    task: AbortHandle,
    /// A reconnect's place in its schedule; `None` for a connect the app asked for, whose failure is final.
    retry: Option<Retry>,
    /// Whether it is still waiting out its retry's delay, not yet reaching the machine.
    waiting: bool,
}

type Reply = oneshot::Sender<Result<(), MobileError>>;

impl<T: Transport> State<T> {
    fn new(transport: T, sink: Arc<dyn EventSink>) -> Self {
        Self {
            transport,
            sink,
            connections: RefCell::new(HashMap::new()),
            attempts: RefCell::new(HashMap::new()),
            generation: Cell::new(0),
            listings: RefCell::new(HashMap::new()),
            tickets: Cell::new(0),
            presentations: RefCell::new(Presentations::default()),
            suspended: RefCell::new(HashMap::new()),
        }
    }

    fn emit(&self, event: MobileEvent) {
        self.sink.on_event(event);
    }

    fn emit_all(&self, events: Vec<MobileEvent>) {
        for event in events {
            self.emit(event);
        }
    }

    /// A ticket later than every one before: taken as a listing begins and as an end is applied, it orders what the
    /// actor learns of a machine's sessions.
    fn ticket(&self) -> u64 {
        let ticket = self.tickets.get() + 1;
        self.tickets.set(ticket);
        ticket
    }

    /// Whether `connection` is still `machine_id`'s: a result read on a connection since closed or replaced changes
    /// nothing.
    fn is_current(&self, machine_id: &str, connection: &Rc<T::Connection>) -> bool {
        self.connections
            .borrow()
            .get(machine_id)
            .is_some_and(|current| Rc::ptr_eq(&current.connection, connection))
    }
}

async fn run<T: Transport>(transport: T, sink: Arc<dyn EventSink>, mut queue: mpsc::UnboundedReceiver<Command>) {
    let state = Rc::new(State::new(transport, sink));
    let ticker = tokio::task::spawn_local(tick(Rc::downgrade(&state))).abort_handle();
    while let Some(command) = queue.recv().await {
        match command {
            Command::Connect { machine, reply } => connect(&state, machine, reply),
            Command::Disconnect { machine_id } => disconnect(&state, &machine_id),
            Command::AnswerHostKey {
                machine_id,
                fingerprint,
                trust,
            } => state.transport.answer_host_key(&machine_id, &fingerprint, trust),
            Command::ListSessions { machine_id, reply } => {
                spawn_on_machine(&state, machine_id, reply, list_sessions);
            }
            Command::CreateSession {
                machine_id,
                name,
                reply,
            } => spawn_on_machine(&state, machine_id, reply, |state, link, id| {
                create_session(state, link, id, name)
            }),
            Command::EndSession {
                machine_id,
                session_id,
                reply,
            } => spawn_on_machine(&state, machine_id, reply, |state, link, id| {
                end_session(state, link, id, session_id)
            }),
            Command::AttachSession {
                machine_id,
                session_id,
                reply,
            } => {
                let _ = reply.send(attach_session(&state, &machine_id, &session_id));
            }
            Command::DetachSession { machine_id, session_id } => {
                state.presentations.borrow_mut().detach(&machine_id, &session_id);
            }
            Command::AnswerAsk {
                machine_id,
                session_id,
                ask_id,
                reply,
            } => {
                state
                    .presentations
                    .borrow_mut()
                    .answer_ask(&machine_id, &session_id, &ask_id, reply);
            }
            Command::NetworkChanged => network_changed(&state),
            Command::Suspend => suspend(&state),
            Command::Resume => resume(&state),
        }
    }
    ticker.abort();
    *state.presentations.borrow_mut() = Presentations::default();
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
    // A connect of the machine as edited replaces whatever it had (a connection, a connect or retry in flight, a
    // suspension), and its sessions' streams with it: the new connection reports its own state.
    if known(state, &machine.id).is_some_and(|known| known != machine) {
        disconnect(state, &machine.id);
    }
    if state.connections.borrow().contains_key(&machine.id) {
        let _ = reply.send(Ok(()));
        return;
    }
    let mut attempts = state.attempts.borrow_mut();
    if let Some(attempt) = attempts.get_mut(&machine.id) {
        attempt.waiters.push(reply);
        let waiting = attempt.waiting;
        drop(attempts);
        // A machine being reconnected is retried at once for the caller.
        if waiting {
            retry_at_once(state, &machine.id);
        }
        return;
    }
    drop(attempts);
    // A machine set aside by a suspend comes back as resume brings it.
    let retry = state.suspended.borrow().get(&machine.id).and_then(SetAside::retry);
    start_attempt(state, machine, vec![reply], retry);
}

/// The machine record `machine_id` is connected, being connected, or set aside with.
fn known<T: Transport>(state: &State<T>, machine_id: &str) -> Option<Machine> {
    let connected = state
        .connections
        .borrow()
        .get(machine_id)
        .map(|link| link.machine.clone());
    connected
        .or_else(|| {
            let attempts = state.attempts.borrow();
            attempts.get(machine_id).map(|attempt| attempt.machine.clone())
        })
        .or_else(|| {
            let suspended = state.suspended.borrow();
            suspended.get(machine_id).map(|set_aside| set_aside.machine.clone())
        })
}

/// Starts a connect of `machine` for `waiters`: a reconnect waits out its `retry`'s delay first, reported as
/// [`MachineState::Reconnecting`].
fn start_attempt<T: Transport>(state: &Rc<State<T>>, machine: Machine, waiters: Vec<Reply>, retry: Option<Retry>) {
    state.suspended.borrow_mut().remove(&machine.id);
    let generation = state.generation.get() + 1;
    state.generation.set(generation);
    let delay = retry.map_or(Duration::ZERO, Retry::delay);
    if retry.is_some() {
        state.emit(MobileEvent::MachineState {
            machine_id: machine.id.clone(),
            state: MachineState::Reconnecting {
                next_in_ms: u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
            },
        });
    }
    let task = tokio::task::spawn_local(reach(Rc::clone(state), machine.clone(), generation, delay)).abort_handle();
    let id = machine.id.clone();
    let attempt = Attempt {
        generation,
        machine,
        waiters,
        task,
        retry,
        waiting: !delay.is_zero(),
    };
    state.attempts.borrow_mut().insert(id, attempt);
}

/// Whether `generation` is still `machine_id`'s attempt.
fn is_attempt<T: Transport>(state: &State<T>, machine_id: &str, generation: u64) -> bool {
    state
        .attempts
        .borrow()
        .get(machine_id)
        .is_some_and(|attempt| attempt.generation == generation)
}

/// The network part of a connect, after a reconnect's `delay`: aborted when the attempt is cancelled.
async fn reach<T: Transport>(state: Rc<State<T>>, machine: Machine, generation: u64, delay: Duration) {
    if !delay.is_zero() {
        tokio::time::sleep(delay).await;
        if let Some(attempt) = state.attempts.borrow_mut().get_mut(&machine.id) {
            attempt.waiting = false;
        }
    }
    let result = state.transport.connect(&machine).await;
    if !is_attempt(&state, &machine.id, generation) {
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
            connected(&state, machine, connection);
            Ok(())
        }
        Err(error) => {
            let reason = error.to_string();
            // A refusal ends a reconnect too: retrying won't change the key, or the user's or the machine's answer.
            match attempt.retry.filter(|_| !matches!(error, TransportError::Refused(_))) {
                Some(retry) => start_attempt(&state, machine, Vec::new(), Some(retry.failed())),
                None => state.emit(MobileEvent::MachineState {
                    machine_id: machine.id.clone(),
                    state: MachineState::Unreachable { reason: reason.clone() },
                }),
            }
            Err(MobileError::Unreachable { reason })
        }
    };
    for waiter in attempt.waiters {
        // A caller that stopped waiting has nothing to be told.
        let _ = waiter.send(answer.clone());
    }
}

/// `machine` is connected on `connection`: it is kept and watched, and the machine's streams start again.
fn connected<T: Transport>(state: &Rc<State<T>>, machine: Machine, connection: T::Connection) {
    let id = machine.id.clone();
    let connection = Rc::new(connection);
    let link = Link {
        machine,
        connection: Rc::clone(&connection),
    };
    state.connections.borrow_mut().insert(id.clone(), link);
    tokio::task::spawn_local(watch(Rc::clone(state), id.clone(), Rc::clone(&connection)));
    state.emit(MobileEvent::MachineState {
        machine_id: id.clone(),
        state: MachineState::Connected,
    });
    let events = state
        .presentations
        .borrow_mut()
        .machine_up(&id, &connection, sessions::now());
    state.emit_all(events);
}

/// Waits for `connection` to go; still the machine's then, it was lost, and the machine is reconnected.
async fn watch<T: Transport>(state: Rc<State<T>>, machine_id: String, connection: Rc<T::Connection>) {
    connection.lost().await;
    if !state.is_current(&machine_id, &connection) {
        return;
    }
    let Some(link) = state.connections.borrow_mut().remove(&machine_id) else {
        return;
    };
    state.presentations.borrow_mut().machine_down(&machine_id);
    start_attempt(&state, link.machine, Vec::new(), Some(Retry::after_loss()));
}

/// A reconnect of `machine_id` waiting out its delay runs at once, its backoff over, for the same callers.
fn retry_at_once<T: Transport>(state: &Rc<State<T>>, machine_id: &str) {
    let attempt = state.attempts.borrow_mut().remove(machine_id);
    let Some(attempt) = attempt else { return };
    attempt.task.abort();
    start_attempt(state, attempt.machine, attempt.waiters, Some(Retry::at_once()));
}

/// The device's network changed: every reconnect waiting out its delay runs at once.
fn network_changed<T: Transport>(state: &Rc<State<T>>) {
    let mut waiting: Vec<String> = state
        .attempts
        .borrow()
        .iter()
        .filter(|(_, attempt)| attempt.waiting)
        .map(|(id, _)| id.clone())
        .collect();
    waiting.sort();
    for machine_id in waiting {
        retry_at_once(state, &machine_id);
    }
}

/// The app went to the background with nothing to keep: every connection closes and every connect in flight is
/// cancelled, with no state reported, and each machine is set aside for [`resume`] with its streams stopped.
fn suspend<T: Transport>(state: &State<T>) {
    let connections = std::mem::take(&mut *state.connections.borrow_mut());
    let attempts = std::mem::take(&mut *state.attempts.borrow_mut());
    let mut set_aside = Vec::new();
    for (machine_id, link) in connections {
        link.connection.close();
        let machine = SetAside {
            machine: link.machine,
            was_connected: true,
        };
        set_aside.push((machine_id, machine));
    }
    for (machine_id, attempt) in attempts {
        attempt.task.abort();
        for waiter in attempt.waiters {
            let _ = waiter.send(Err(MobileError::Cancelled));
        }
        let machine = SetAside {
            machine: attempt.machine,
            was_connected: attempt.retry.is_some(),
        };
        set_aside.push((machine_id, machine));
    }
    for (machine_id, machine) in set_aside {
        state.presentations.borrow_mut().suspend_machine(&machine_id);
        state.suspended.borrow_mut().insert(machine_id, machine);
    }
}

/// The app is back: each machine [`suspend`] set aside is connected again at once, one that had been connected as a
/// reconnect (on the reconnect schedule), one never connected as the connect the app asked for.
fn resume<T: Transport>(state: &Rc<State<T>>) {
    let mut machines: Vec<SetAside> = std::mem::take(&mut *state.suspended.borrow_mut())
        .into_values()
        .collect();
    machines.sort_by(|left, right| left.machine.id.cmp(&right.machine.id));
    for set_aside in machines {
        let retry = set_aside.retry();
        start_attempt(state, set_aside.machine, Vec::new(), retry);
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

/// Closes `machine_id`'s connection, if any, and forgets its listing, its sessions' streams and its suspension.
fn close<T: Transport>(state: &State<T>, machine_id: &str) {
    state.suspended.borrow_mut().remove(machine_id);
    state.listings.borrow_mut().remove(machine_id);
    state.presentations.borrow_mut().forget_machine(machine_id);
    let link = state.connections.borrow_mut().remove(machine_id);
    if let Some(link) = link {
        link.connection.close();
    }
}

/// What a session command runs with: the machine's connection and the name its messages give the machine.
struct OnMachine<C> {
    connection: Rc<C>,
    host: String,
}

/// Runs `job` as a local task on `machine_id`'s connection, answering `reply` with what it returns;
/// [`MobileError::NotConnected`] at once when the machine has no connection.
fn spawn_on_machine<T, R, F, Fut>(
    state: &Rc<State<T>>,
    machine_id: String,
    reply: oneshot::Sender<Result<R, MobileError>>,
    job: F,
) where
    T: Transport,
    R: 'static,
    F: FnOnce(Rc<State<T>>, OnMachine<T::Connection>, String) -> Fut,
    Fut: Future<Output = Result<R, MobileError>> + 'static,
{
    let link = state.connections.borrow().get(&machine_id).map(|link| OnMachine {
        connection: Rc::clone(&link.connection),
        host: shown_name(&link.machine),
    });
    let Some(link) = link else {
        let _ = reply.send(Err(MobileError::NotConnected { machine_id }));
        return;
    };
    let work = job(Rc::clone(state), link, machine_id);
    tokio::task::spawn_local(async move {
        // A caller that stopped waiting has nothing to be told.
        let _ = reply.send(work.await);
    });
}

/// The name the session messages give `machine`: what the app shows for it, or its host when it has no label.
fn shown_name(machine: &Machine) -> String {
    let label = machine.label.trim();
    if label.is_empty() {
        machine.host.clone()
    } else {
        label.to_owned()
    }
}

fn remote(reason: String) -> MobileError {
    MobileError::Remote { reason }
}

/// The sessions of `tree`, a listing of `machine_id` that began at ticket `began`. It is kept and reported as a
/// [`MobileEvent::Tree`] only while its connection is still the machine's and nothing newer about the machine's
/// sessions is known; a stale listing still answers its caller.
fn listed<T: Transport>(
    state: &State<T>,
    link: &OnMachine<T::Connection>,
    machine_id: String,
    tree: ControlRemoteTree,
    began: u64,
) -> Vec<RemoteSession> {
    let sessions = sessions::listed_sessions(&tree);
    if !state.is_current(&machine_id, &link.connection) {
        return sessions;
    }
    {
        let mut listings = state.listings.borrow_mut();
        let kept = listings.entry(machine_id.clone()).or_default();
        if kept.as_of > began {
            return sessions;
        }
        kept.tree = Some(tree);
        kept.as_of = began;
    }
    state.emit(MobileEvent::Tree {
        machine_id,
        sessions: sessions.clone(),
    });
    sessions
}

async fn list_sessions<T: Transport>(
    state: Rc<State<T>>,
    link: OnMachine<T::Connection>,
    machine_id: String,
) -> Result<Vec<RemoteSession>, MobileError> {
    let began = state.ticket();
    let tree = sessions::list(&*link.connection, &link.host).await.map_err(remote)?;
    Ok(listed(&state, &link, machine_id, tree, began))
}

async fn create_session<T: Transport>(
    state: Rc<State<T>>,
    link: OnMachine<T::Connection>,
    machine_id: String,
    name: Option<String>,
) -> Result<RemoteSession, MobileError> {
    let began = Cell::new(0);
    let (created, tree) = sessions::create(&*link.connection, &link.host, name.as_deref(), || {
        began.set(state.ticket());
    })
    .await
    .map_err(remote)?;
    listed(&state, &link, machine_id, tree, began.get());
    Ok(RemoteSession::from(&created))
}

/// Ends the session there; while the connection is still the machine's, a listing that began before is stale from now
/// on, and the kept listing, if any, loses the session and is reported.
async fn end_session<T: Transport>(
    state: Rc<State<T>>,
    link: OnMachine<T::Connection>,
    machine_id: String,
    session_id: String,
) -> Result<(), MobileError> {
    sessions::end(&*link.connection, &link.host, &session_id)
        .await
        .map_err(remote)?;
    if !state.is_current(&machine_id, &link.connection) {
        return Ok(());
    }
    let ticket = state.ticket();
    let sessions = {
        let mut listings = state.listings.borrow_mut();
        let kept = listings.entry(machine_id.clone()).or_default();
        kept.as_of = ticket;
        kept.tree.as_mut().map(|tree| {
            sessions::forget(tree, &session_id);
            sessions::listed_sessions(tree)
        })
    };
    if let Some(sessions) = sessions {
        state.emit(MobileEvent::Tree { machine_id, sessions });
    }
    Ok(())
}

/// Opens the presentation stream of `session_id`, a session of `machine_id`'s last listing, on the machine's
/// connection.
fn attach_session<T: Transport>(state: &Rc<State<T>>, machine_id: &str, session_id: &str) -> Result<(), MobileError> {
    let connection = state
        .connections
        .borrow()
        .get(machine_id)
        .map(|link| Rc::clone(&link.connection))
        .ok_or_else(|| MobileError::NotConnected {
            machine_id: machine_id.to_owned(),
        })?;
    let version = state
        .listings
        .borrow()
        .get(machine_id)
        .and_then(|kept| kept.tree.as_ref())
        .filter(|tree| tree.sessions.iter().any(|listed| listed.id == session_id))
        .map(|tree| tree.presentation)
        .ok_or_else(|| MobileError::NotListed {
            session_id: session_id.to_owned(),
        })?;
    let events = state.presentations.borrow_mut().attach(
        machine_id,
        session_id,
        version,
        ChannelLinks::new(connection, reporter(state)),
        sessions::now(),
    )?;
    state.emit_all(events);
    Ok(())
}

/// Where the streams' links report: the actor's [`Presentations`], once the link's task has the report.
fn reporter<T: Transport>(state: &Rc<State<T>>) -> Reporter {
    let state = Rc::downgrade(state);
    Rc::new(move |key, event: LinkEvent| {
        let Some(state) = state.upgrade() else { return };
        let events = state.presentations.borrow_mut().link_event(key, event, sessions::now());
        state.emit_all(events);
    })
}

/// The streams' tick, every [`presentation::TICK`] while the actor runs.
async fn tick<T: Transport>(state: Weak<State<T>>) {
    let mut interval = tokio::time::interval(presentation::TICK);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        let Some(state) = state.upgrade() else { return };
        let events = state.presentations.borrow_mut().tick(sessions::now());
        state.emit_all(events);
    }
}
