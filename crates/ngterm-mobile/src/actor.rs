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
//! one-second tick. A shown pane ([`crate::pane`]) runs its attach on the same connection; its threads report back as
//! notes a local task hands to the panes, and the respawn counts a stream reports are handed to its session's panes.
//! A shown pane's chat ([`crate::chat`]) runs on the same connection too, learning each session's layout and its panes'
//! agents from what the streams report; hiding the pane or detaching its session closes it.
//!
//! A headless machine's catalog ([`catalogs`]) is followed on its connection once a listing says the machine is
//! headless: its rows take the place of the listing's own-app rows, with the other app's sessions and the listing's hint
//! beside them, and New Session, Rename Session and End Session go to it under an operation id while its stream is up;
//! while it is down, they run as plain commands and change the kept catalog as they change the kept listing.
//!
//! A connection the core holds is watched: lost by itself, it is retried on the reconnect schedule as a connect
//! attempt that waits first ([`crate::reconnect`]) until it is back or refused for good
//! ([`crate::transport::TransportError::Refused`]), and the machine's sessions stay attached across it. Closing a
//! machine's connection (a disconnect, a connect of the machine as edited) ends its sessions' streams and its retries;
//! suspending lets the streams' links write what they hold, then closes every connection but keeps the streams to start
//! again on resume.

mod catalogs;
mod chats;
mod inputs;

#[cfg(test)]
mod both_apps_tests;
#[cfg(test)]
mod catalog_tests;
#[cfg(test)]
mod chat_tests;
#[cfg(test)]
mod clipboard_tests;
#[cfg(test)]
mod input_tests;
#[cfg(test)]
mod link_tests;
#[cfg(test)]
mod log_tests;
#[cfg(test)]
mod pane_tests;
#[cfg(test)]
mod presentation_tests;
#[cfg(test)]
mod reconnect_tests;
#[cfg(test)]
mod tests;

use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::thread::{self, JoinHandle, ThreadId};
use std::time::Duration;

use ngterm_core::model::StatusKeystroke;
use ngterm_core::pane_env::uuid_string;
use ngterm_core::remote::origin::OriginForm;
use ngterm_core::remote::origin_replies::{NOT_ATTACHABLE, READY_BUDGET, SESSION_NOUN, unattached};
use ngterm_core::remote::peer::origin_pane_identity as pane_identity;
use ngterm_core::remote::tree::{INVALID_REMOTE_SESSION, is_plain};
use ngterm_proto::{ControlRemoteTree, ControlResult, RemotePeer, Response};
use uuid::Uuid;

use tokio::runtime::Builder;
use tokio::sync::{mpsc, oneshot};
use tokio::task::{AbortHandle, LocalSet};

use crate::api::PaneInputError;
use crate::api::{
    EventSink, Machine, MachineState, MobileError, MobileEvent, Pane, PaneSize, PaneSlot, UploadProgress,
};
use crate::applog::{self, tag};
use crate::catalog::operations::{self, Operation, Settled};
use crate::chat::input::{Action, AgentInputOutcome, InputTiming};
use crate::chat::{AgentCommand, ChatEvent, ChatKey, Chats, UploadCancel};
use crate::note;
use crate::pane::{self, ClipboardReply, ClipboardWriteStatus, GoneCheck, Note, PaneRef, Panes, Target};
use crate::presentation::{self, AskReply, ChannelLinks, LinkEvent, PickReply, Presentations, Reporter};
use crate::reconnect::Retry;
use crate::sessions::{self, RemoteSession};
use crate::transport::{Connection, Learned, Transport, TransportError};

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
    UseKey {
        machine_id: String,
        key_id: String,
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
    DuplicateSession {
        machine_id: String,
        session_id: String,
        reply: oneshot::Sender<Result<RemoteSession, MobileError>>,
    },
    EndSession {
        machine_id: String,
        session_id: String,
        reply: Reply,
    },
    RenameSession {
        machine_id: String,
        session_id: String,
        name: String,
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
    AnswerPick {
        machine_id: String,
        session_id: String,
        pick_id: String,
        reply: PickReply,
    },
    ShowPane {
        machine_id: String,
        session_id: String,
        pane_id: String,
        size: PaneSize,
        reply: oneshot::Sender<Result<Arc<Pane>, MobileError>>,
    },
    HidePane(PaneRef),
    TakeLead(PaneRef),
    AnswerClipboardRead {
        pane: PaneRef,
        read_id: u64,
        reply: ClipboardReply,
    },
    CompleteClipboardWrite {
        pane: PaneRef,
        write_id: u64,
        status: ClipboardWriteStatus,
    },
    OpenChat {
        pane: PaneRef,
        reply: Reply,
    },
    CloseChat(PaneRef),
    LoadOlder {
        pane: PaneRef,
        reply: oneshot::Sender<Result<Vec<ChatEvent>, MobileError>>,
    },
    AgentCommands {
        pane: PaneRef,
        reply: oneshot::Sender<Result<Vec<AgentCommand>, MobileError>>,
    },
    /// A file the chat composer attaches, sent to the machine ([`crate::chat::upload`]); the reply dropped, or the
    /// `cancel` token cancelled, cancels it.
    Upload {
        machine_id: String,
        name: String,
        bytes: Vec<u8>,
        progress: Arc<dyn UploadProgress>,
        cancel: Option<Arc<UploadCancel>>,
        reply: oneshot::Sender<Result<String, MobileError>>,
    },
    /// A chat action on a shown pane's agent ([`crate::chat::input`]), from the handle whose slot is `slot`.
    PaneInput {
        pane: PaneRef,
        slot: Arc<PaneSlot>,
        action: Action,
        reply: oneshot::Sender<Result<AgentInputOutcome, PaneInputError>>,
    },
    /// The labels an approval card offers for a shown pane's dialog, asked by the handle whose slot is `slot`.
    ApprovalLabels {
        pane: PaneRef,
        slot: Arc<PaneSlot>,
        reply: oneshot::Sender<Result<Vec<String>, PaneInputError>>,
    },
    /// A key the user typed into a shown pane, for its origin's Badge reset rule ([`crate::presentation`]).
    Typed {
        pane: PaneRef,
        key: StatusKeystroke,
    },
    /// The listed session a session link tapped in a shown pane names ([`Pane::session_link`]).
    SessionLink {
        pane: PaneRef,
        raw: String,
        reply: oneshot::Sender<Result<Option<RemoteSession>, PaneInputError>>,
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
    /// Starts the actor over `transport`, reporting to `sink`, its chat actions waiting as `timing` says.
    pub(crate) fn spawn<T>(transport: T, sink: Arc<dyn EventSink>, timing: InputTiming) -> Result<Self, MobileError>
    where
        T: Transport + Send,
    {
        // Time and IO: the russh transport's sockets, timeouts and keepalives.
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| MobileError::Stopped)?;
        let (commands, queue) = mpsc::unbounded_channel();
        let wiring = Wiring {
            commands: commands.downgrade(),
            timing,
        };
        let log = applog::inherited();
        let thread = thread::Builder::new()
            .name(THREAD_NAME.to_owned())
            .spawn(move || {
                let _log = log.as_ref().map(tracing::dispatcher::set_default);
                let local = LocalSet::new();
                local.block_on(&runtime, run(transport, sink, queue, wiring));
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

/// What the actor hands out besides its state: its own queue, for the pane handles it makes (held weakly, so a handle
/// left in the app never keeps the actor running), and its chat actions' timing.
pub(crate) struct Wiring {
    pub(crate) commands: mpsc::WeakUnboundedSender<Command>,
    pub(crate) timing: InputTiming,
}

#[cfg(test)]
impl Wiring {
    /// Wiring to no actor's queue, at the default timing (state built by a test, with no loop).
    pub(crate) fn detached() -> Self {
        Self {
            commands: mpsc::unbounded_channel().0.downgrade(),
            timing: InputTiming::default(),
        }
    }
}

/// The actor's state, shared by its loop and its local tasks. No borrow is held across an await.
struct State<T: Transport> {
    transport: T,
    /// The actor's queue and its chat actions' timing.
    wiring: Wiring,
    /// The panes a chat action is running on: one at a time each.
    acting: RefCell<HashSet<ChatKey>>,
    sink: Arc<dyn EventSink>,
    /// The live connections, each with the machine record it was made from.
    connections: RefCell<HashMap<String, Link<T::Connection>>>,
    /// What each machine's last lost or suspended connection had learned of it ([`Learned`]: its shell and its app),
    /// for its reconnect, whose streams start before any new listing, with the machine as it was then: an entry since
    /// edited to reach another machine learns its own.
    origin_forms: RefCell<HashMap<String, (Machine, Learned)>>,
    /// The connects in flight, one per machine: a later connect of the same machine joins it, a disconnect or a
    /// connect of the machine as edited cancels it.
    attempts: RefCell<HashMap<String, Attempt>>,
    /// The last attempt's number: an attempt that finishes no longer current was overtaken and changes nothing.
    generation: Cell<u64>,
    /// What the actor knows of each connected machine's sessions.
    listings: RefCell<HashMap<String, Kept>>,
    /// The last ticket handed out ([`State::ticket`]).
    tickets: Cell<u64>,
    /// The tickets of the listings in flight ([`State::begin_listing`]).
    open_listings: Rc<RefCell<BTreeSet<u64>>>,
    /// The attached sessions' presentation streams.
    presentations: RefCell<Presentations<T::Connection>>,
    /// The shown panes.
    panes: RefCell<Panes<T::Connection>>,
    /// The open chats, and what they know of the attached sessions.
    chats: RefCell<Chats<T::Connection>>,
    /// The machines [`suspend`] set aside, to reconnect on [`resume`].
    suspended: RefCell<HashMap<String, SetAside>>,
    /// The catalogs of the headless machines ([`catalogs`]).
    catalogs: RefCell<HashMap<String, catalogs::Follow>>,
}

/// What the actor knows of a connected machine's sessions.
#[derive(Default)]
struct Kept {
    /// Its last listing, whole (endpoint and panes included), for attaching its sessions, as the ends since left it;
    /// `None` until one is read on the connection.
    tree: Option<ControlRemoteTree>,
    /// The ticket of the listing kept. A listing that began before it is stale.
    as_of: u64,
    /// The sessions ended since that listing or one still in flight began, with their tickets: a listing that began
    /// before an end loses the session again.
    ended: Vec<(u64, String)>,
}

/// A listing in flight, from [`State::begin_listing`] until it drops.
struct Listing {
    began: u64,
    open: Rc<RefCell<BTreeSet<u64>>>,
}

impl Drop for Listing {
    fn drop(&mut self) {
        self.open.borrow_mut().remove(&self.began);
    }
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
    fn new(transport: T, sink: Arc<dyn EventSink>, notes: pane::Notes, wiring: Wiring) -> Self {
        Self {
            transport,
            wiring,
            acting: RefCell::new(HashSet::new()),
            sink,
            connections: RefCell::new(HashMap::new()),
            origin_forms: RefCell::new(HashMap::new()),
            attempts: RefCell::new(HashMap::new()),
            generation: Cell::new(0),
            listings: RefCell::new(HashMap::new()),
            tickets: Cell::new(0),
            open_listings: Rc::default(),
            presentations: RefCell::new(Presentations::default()),
            panes: RefCell::new(Panes::new(notes)),
            chats: RefCell::new(Chats::default()),
            suspended: RefCell::new(HashMap::new()),
            catalogs: RefCell::new(HashMap::new()),
        }
    }

    fn emit(&self, event: MobileEvent) {
        self.sink.on_event(event);
    }

    /// What `read` makes of the answer of the app that lists `session_id` in `machine_id`'s kept listing: a machine
    /// that runs both apps keeps both answers. `None` for a session the listing does not hold.
    fn listing_of<R>(
        &self,
        machine_id: &str,
        session_id: &str,
        read: impl FnOnce(&ControlRemoteTree) -> R,
    ) -> Option<R> {
        let listings = self.listings.borrow();
        let tree = listings.get(machine_id)?.tree.as_ref()?;
        let app = tree
            .apps()
            .find(|app| app.sessions.iter().any(|listed| listed.id == session_id));
        // a session the catalog brought before any listing showed it is the headless machine's own
        app.or_else(|| {
            let catalogs = self.catalogs.borrow();
            let catalog = catalogs.get(machine_id)?.mirror.catalog()?;
            crate::catalog::session(catalog, session_id)?;
            tree.apps().find(|app| app.headless == Some(true))
        })
        .map(read)
    }

    /// The app that lists `session_id` on `machine_id`; `None` for a session the kept listing does not hold.
    fn app_of(&self, machine_id: &str, session_id: &str) -> Option<RemotePeer> {
        self.listing_of(machine_id, session_id, |app| app.endpoint.peer())
    }

    fn emit_all(&self, events: Vec<MobileEvent>) {
        for event in events {
            self.emit(event);
        }
    }

    /// Emits what the streams made, which the chats learn from first, then hands the respawn counts they reported to
    /// their sessions' panes.
    fn presented(&self, events: Vec<MobileEvent>) {
        for event in &events {
            self.chats.borrow_mut().observe(event);
        }
        self.emit_all(events);
        let respawns = self.presentations.borrow_mut().take_respawns();
        for reported in respawns {
            let events = self.panes.borrow_mut().respawns(&reported);
            self.emit_all(events);
        }
    }

    /// A ticket later than every one before: taken as a listing begins and as an end is applied, it orders what the
    /// actor learns of a machine's sessions.
    fn ticket(&self) -> u64 {
        let ticket = self.tickets.get() + 1;
        self.tickets.set(ticket);
        ticket
    }

    /// A listing begins: its ticket, held as in flight until the [`Listing`] drops.
    fn begin_listing(&self) -> Listing {
        let began = self.ticket();
        self.open_listings.borrow_mut().insert(began);
        Listing {
            began,
            open: Rc::clone(&self.open_listings),
        }
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

async fn run<T: Transport>(
    transport: T,
    sink: Arc<dyn EventSink>,
    mut queue: mpsc::UnboundedReceiver<Command>,
    wiring: Wiring,
) {
    let (notes, inbox) = mpsc::unbounded_channel();
    let state = Rc::new(State::new(transport, sink, notes, wiring));
    let ticker = tokio::task::spawn_local(tick(Rc::downgrade(&state))).abort_handle();
    let pane_notes = tokio::task::spawn_local(pane_notes(Rc::downgrade(&state), inbox)).abort_handle();
    while let Some(command) = queue.recv().await {
        match command {
            Command::Connect { machine, reply } => connect(&state, machine, reply),
            Command::Disconnect { machine_id } => {
                note!(INFO, tag::LINK, "disconnect asked", machine = machine_id.as_str());
                disconnect(&state, &machine_id);
            }
            Command::UseKey { machine_id, key_id } => use_key(&state, &machine_id, &key_id),
            Command::AnswerHostKey {
                machine_id,
                fingerprint,
                trust,
            } => state.transport.answer_host_key(&machine_id, &fingerprint, trust),
            command @ (Command::ListSessions { .. }
            | Command::CreateSession { .. }
            | Command::DuplicateSession { .. }
            | Command::EndSession { .. }
            | Command::RenameSession { .. }
            | Command::AttachSession { .. }
            | Command::DetachSession { .. }) => session_command(&state, command),
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
            Command::AnswerPick {
                machine_id,
                session_id,
                pick_id,
                reply,
            } => {
                state
                    .presentations
                    .borrow_mut()
                    .answer_pick(&machine_id, &session_id, &pick_id, reply);
            }
            Command::Typed { pane, key } => {
                state.presentations.borrow_mut().typed(&pane, key);
            }
            Command::SessionLink { pane, raw, reply } => session_link(&state, pane, raw, reply),
            Command::NetworkChanged => network_changed(&state),
            Command::Suspend => suspend(&state).await,
            Command::Resume => resume(&state),
            command if chats::handles(&command) => chats::command(&state, command),
            command @ (Command::PaneInput { .. } | Command::ApprovalLabels { .. }) => inputs::command(&state, command),
            command => pane_command(&state, command),
        }
    }
    ticker.abort();
    pane_notes.abort();
    state.catalogs.borrow_mut().clear();
    *state.presentations.borrow_mut() = Presentations::default();
    state.chats.borrow_mut().clear();
    // Nobody is left to tell: the panes' threads stop and their channels close.
    state.panes.borrow_mut().clear();
    // Waiters dropped unanswered read as `Stopped`.
    for (_, attempt) in state.attempts.borrow_mut().drain() {
        attempt.task.abort();
    }
    let connections = std::mem::take(&mut *state.connections.borrow_mut());
    for link in connections.values() {
        link.connection.close();
    }
}

/// A command about a machine's sessions: listing, New Session, Duplicate, End Session, attaching and detaching.
fn session_command<T: Transport>(state: &Rc<State<T>>, command: Command) {
    match command {
        Command::ListSessions { machine_id, reply } => {
            spawn_on_machine(state, machine_id, reply, list_sessions);
        }
        Command::CreateSession {
            machine_id,
            name,
            reply,
        } => spawn_on_machine(state, machine_id, reply, |state, link, id| async move {
            let created = create_session(state, link, id, name).await;
            note!(INFO, tag::SESSIONS, "new session", result = result_class(&created));
            created
        }),
        Command::DuplicateSession {
            machine_id,
            session_id,
            reply,
        } => spawn_on_machine(state, machine_id, reply, |state, link, id| async move {
            let created = duplicate_session(state, link, id, session_id).await;
            note!(
                INFO,
                tag::SESSIONS,
                "duplicate session",
                result = result_class(&created)
            );
            created
        }),
        Command::EndSession {
            machine_id,
            session_id,
            reply,
        } => spawn_on_machine(state, machine_id, reply, |state, link, id| async move {
            let ended = end_session(state, link, id, session_id).await;
            note!(INFO, tag::SESSIONS, "end session", result = result_class(&ended));
            ended
        }),
        Command::RenameSession {
            machine_id,
            session_id,
            name,
            reply,
        } => spawn_on_machine(state, machine_id, reply, |state, link, id| async move {
            let renamed = rename_session(state, link, id, session_id, name).await;
            note!(INFO, tag::SESSIONS, "rename session", result = result_class(&renamed));
            renamed
        }),
        Command::AttachSession {
            machine_id,
            session_id,
            reply,
        } => {
            let attached = attach_session(state, &machine_id, &session_id);
            note!(
                INFO,
                tag::STREAM,
                "attach asked",
                result = result_class(&attached),
                machine = machine_id.as_str(),
                session = session_id.as_str()
            );
            let _ = reply.send(attached);
        }
        Command::DetachSession { machine_id, session_id } => {
            note!(
                INFO,
                tag::STREAM,
                "detach asked",
                machine = machine_id.as_str(),
                session = session_id.as_str()
            );
            state.presentations.borrow_mut().detach(&machine_id, &session_id);
            state.chats.borrow_mut().close_session(&machine_id, &session_id);
            let events = state.panes.borrow_mut().hide_session(&machine_id, &session_id);
            state.emit_all(events);
        }
        _ => {}
    }
}

/// Settles what a connect can at once, on the loop, so a command posted after it sees its effect; only the network
/// part runs as a local task.
fn connect<T: Transport>(state: &Rc<State<T>>, machine: Machine, reply: Reply) {
    // A connect of the machine as edited replaces whatever it had (a connection, a connect or retry in flight, a
    // suspension), and its sessions' streams with it: the new connection reports its own state. One only renamed
    // keeps it all and takes the new label.
    note!(
        INFO,
        tag::LINK,
        "connect asked",
        machine = machine.id.as_str(),
        label = machine.label.as_str(),
        user = machine.user.as_str(),
        host = machine.host.as_str(),
        port = machine.port
    );
    match known(state, &machine.id) {
        Some(known) if !known.reached_as(&machine) => disconnect(state, &machine.id),
        Some(known) if known.label != machine.label => relabel(state, &machine.id, &machine.label),
        _ => {}
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

/// `machine_id`'s record, connected, being connected or set aside, takes the key `key_id` for its next attempt; its
/// connection stays.
fn use_key<T: Transport>(state: &State<T>, machine_id: &str, key_id: &str) {
    if let Some(link) = state.connections.borrow_mut().get_mut(machine_id) {
        key_id.clone_into(&mut link.machine.key_id);
    }
    if let Some(attempt) = state.attempts.borrow_mut().get_mut(machine_id) {
        key_id.clone_into(&mut attempt.machine.key_id);
    }
    if let Some(set_aside) = state.suspended.borrow_mut().get_mut(machine_id) {
        key_id.clone_into(&mut set_aside.machine.key_id);
    }
}

/// `machine_id`'s record, connected, being connected or set aside, takes the label `label`; its connection stays.
fn relabel<T: Transport>(state: &State<T>, machine_id: &str, label: &str) {
    if let Some(link) = state.connections.borrow_mut().get_mut(machine_id) {
        label.clone_into(&mut link.machine.label);
    }
    if let Some(attempt) = state.attempts.borrow_mut().get_mut(machine_id) {
        label.clone_into(&mut attempt.machine.label);
    }
    if let Some(set_aside) = state.suspended.borrow_mut().get_mut(machine_id) {
        label.clone_into(&mut set_aside.machine.label);
    }
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

/// The network part of a connect, after a reconnect's `delay`: aborted when the attempt is cancelled. The machine is
/// reached with the attempt's record as it stands then, and kept or retried with it as it stands after ([`use_key`]
/// may change it meanwhile).
async fn reach<T: Transport>(state: Rc<State<T>>, mut machine: Machine, generation: u64, delay: Duration) {
    if !delay.is_zero() {
        tokio::time::sleep(delay).await;
        let mut attempts = state.attempts.borrow_mut();
        if let Some(attempt) = attempts
            .get_mut(&machine.id)
            .filter(|attempt| attempt.generation == generation)
        {
            attempt.waiting = false;
            machine.clone_from(&attempt.machine);
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
    let machine = attempt.machine;
    let answer = match result {
        Ok(connection) => {
            connected(&state, machine, connection);
            Ok(())
        }
        Err(error) => {
            let reason = error.to_string();
            note!(
                WARN,
                tag::LINK,
                "connect failed",
                machine = machine.id.as_str(),
                class = error.class(),
                reconnect = attempt.retry.is_some()
            );
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
    if let Some((recorded, learned)) = state.origin_forms.borrow().get(&id)
        && recorded.reached_as(&machine)
    {
        // what its last connection learned: a reconnect's streams start before any new listing
        connection.set_learned(*learned);
    }
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
    state.presented(events);
    let events = state.panes.borrow_mut().machine_up(&id, &connection);
    state.emit_all(events);
    state.chats.borrow_mut().machine_up(&id, &connection);
    catalogs::start(state, &id);
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
    note!(WARN, tag::LINK, "connection lost", machine = machine_id.as_str());
    state
        .origin_forms
        .borrow_mut()
        .insert(machine_id.clone(), (link.machine.clone(), link.connection.learned()));
    state.presentations.borrow_mut().machine_down(&machine_id);
    state.panes.borrow_mut().machine_down(&machine_id);
    let events = state.chats.borrow_mut().machine_down(&machine_id);
    state.emit_all(events);
    let lost = format!("the connection to {} was lost", shown_name(&link.machine));
    catalogs::stop(&state, &machine_id, &lost);
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
    note!(INFO, tag::LINK, "network changed", retried = waiting.len());
    for machine_id in waiting {
        retry_at_once(state, &machine_id);
    }
}

/// The app went to the background with nothing to keep: every connection closes and every connect in flight is
/// cancelled, with no state reported, and each machine is set aside for [`resume`] with its streams stopped.
///
/// The connected machines' streams stop first, and the loop waits for their links to write what they hold and the
/// transport to send it (each for [`FLUSH_LIMIT`] at most, all at once): an answer posted just before (`answer_ask` is
/// fire-and-forget) reaches the origin before the connection closes under it.
///
/// [`FLUSH_LIMIT`]: crate::presentation::FLUSH_LIMIT
async fn suspend<T: Transport>(state: &State<T>) {
    let mut connected: Vec<String> = state.connections.borrow().keys().cloned().collect();
    connected.sort();
    note!(INFO, tag::LINK, "suspend", connected = connected.len());
    let mut tasks = Vec::new();
    for machine_id in connected {
        tasks.extend(state.presentations.borrow_mut().stop_machine(&machine_id));
    }
    for task in tasks {
        // A link that panicked has nothing left to write.
        let _ = task.await;
    }
    let connections = std::mem::take(&mut *state.connections.borrow_mut());
    let attempts = std::mem::take(&mut *state.attempts.borrow_mut());
    let mut set_aside = Vec::new();
    for (machine_id, link) in connections {
        state
            .origin_forms
            .borrow_mut()
            .insert(machine_id.clone(), (link.machine.clone(), link.connection.learned()));
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
        state.panes.borrow_mut().machine_down(&machine_id);
        state.chats.borrow_mut().suspend_machine(&machine_id);
        catalogs::stop(state, &machine_id, "the app is in the background");
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
    note!(INFO, tag::LINK, "resume", machines = machines.len());
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
    // a machine disconnected or edited may be another machine now: it is learned again
    state.origin_forms.borrow_mut().remove(machine_id);
    state.listings.borrow_mut().remove(machine_id);
    catalogs::forget(state, machine_id);
    state.presentations.borrow_mut().forget_machine(machine_id);
    state.chats.borrow_mut().forget_machine(machine_id);
    let events = state.panes.borrow_mut().forget_machine(machine_id);
    state.emit_all(events);
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

/// How a call ended, for the log: `ok`, or the error's class.
pub(crate) fn result_class<R>(result: &Result<R, MobileError>) -> &'static str {
    result.as_ref().map_or_else(MobileError::class, |_| "ok")
}

fn remote(reason: String) -> MobileError {
    MobileError::Remote { reason }
}

/// The sessions of `tree`, a listing of `machine_id` that began at ticket `began`, an app it went on without as last
/// listed ([`sessions::skipped_kept`]). It is kept and reported as a
/// [`MobileEvent::Tree`] only while its connection is still the machine's and no newer listing is kept; a stale listing
/// still answers its caller. The sessions ended since it began are not in what it answers or keeps.
fn listed<T: Transport>(
    state: &Rc<State<T>>,
    link: &OnMachine<T::Connection>,
    machine_id: &str,
    tree: ControlRemoteTree,
    began: u64,
) -> Vec<RemoteSession> {
    let mut tree = with_skipped_app(state, machine_id, tree);
    let current = state.is_current(machine_id, &link.connection);
    let answer = |tree: &ControlRemoteTree| {
        catalogs::rows(state, machine_id, Some(tree)).unwrap_or_else(|| sessions::listed_sessions(tree))
    };
    {
        let mut listings = state.listings.borrow_mut();
        let ended = listings
            .get(machine_id)
            .map(|kept| kept.ended.iter())
            .into_iter()
            .flatten();
        for (_, session_id) in ended.filter(|(ticket, _)| *ticket > began) {
            sessions::forget(&mut tree, session_id);
        }
        if !current {
            return answer(&tree);
        }
        let kept = listings.entry(machine_id.to_owned()).or_default();
        if kept.as_of > began {
            return answer(&tree);
        }
        let oldest = state
            .open_listings
            .borrow()
            .first()
            .map_or(began, |open| began.min(*open));
        kept.ended.retain(|(ticket, _)| *ticket > oldest);
        kept.tree = Some(tree.clone());
        kept.as_of = began;
    }
    catalogs::listed(state, machine_id, &link.host, &tree);
    catalogs::report(state, machine_id);
    answer(&tree)
}

async fn list_sessions<T: Transport>(
    state: Rc<State<T>>,
    link: OnMachine<T::Connection>,
    machine_id: String,
) -> Result<Vec<RemoteSession>, MobileError> {
    let listing = state.begin_listing();
    let tree = sessions::list(&*link.connection, &link.host).await.map_err(remote)?;
    Ok(listed(&state, &link, &machine_id, tree, listing.began))
}

async fn create_session<T: Transport>(
    state: Rc<State<T>>,
    link: OnMachine<T::Connection>,
    machine_id: String,
    name: Option<String>,
) -> Result<RemoteSession, MobileError> {
    let operation = Operation::NewSession { name: name.clone() };
    if let Some(created) = catalog_action(&state, &link, &machine_id, &operation).await {
        return created_in_catalog(&state, &link, &machine_id, created?).await;
    }
    let listing = Cell::new(None);
    let (created, tree) = sessions::create(&*link.connection, &link.host, name.as_deref(), || {
        listing.set(Some(state.begin_listing()));
    })
    .await
    .map_err(remote)?;
    let receipt = RemoteSession::listed(&created, tree.endpoint.peer());
    catalogs::add_session(&state, &machine_id, &created.id);
    let tree = with_other_app(&state, &machine_id, tree);
    let listing = listing.take();
    listed(
        &state,
        &link,
        &machine_id,
        tree,
        listing.as_ref().map_or(0, |listing| listing.began),
    );
    Ok(receipt)
}

/// `tree`, a listing of the machine, with an app it went on without as last listed: that app's sessions stay.
fn with_skipped_app<T: Transport>(state: &State<T>, machine_id: &str, tree: ControlRemoteTree) -> ControlRemoteTree {
    let listings = state.listings.borrow();
    sessions::skipped_kept(tree, listings.get(machine_id).and_then(|kept| kept.tree.as_ref()))
}

/// `tree`, the listing of the one app a session was made on, with the machine's other app as last listed.
fn with_other_app<T: Transport>(state: &State<T>, machine_id: &str, tree: ControlRemoteTree) -> ControlRemoteTree {
    let listings = state.listings.borrow();
    sessions::beside_kept(tree, listings.get(machine_id).and_then(|kept| kept.tree.as_ref()))
}

async fn duplicate_session<T: Transport>(
    state: Rc<State<T>>,
    link: OnMachine<T::Connection>,
    machine_id: String,
    source: String,
) -> Result<RemoteSession, MobileError> {
    let listing = Cell::new(None);
    let source = (source.as_str(), state.app_of(&machine_id, &source));
    let (created, tree) = sessions::duplicate(&*link.connection, &link.host, source, || {
        listing.set(Some(state.begin_listing()));
    })
    .await
    .map_err(remote)?;
    let receipt = RemoteSession::listed(&created, tree.endpoint.peer());
    catalogs::add_session(&state, &machine_id, &created.id);
    let tree = with_other_app(&state, &machine_id, tree);
    let listing = listing.take();
    listed(
        &state,
        &link,
        &machine_id,
        tree,
        listing.as_ref().map_or(0, |listing| listing.began),
    );
    Ok(receipt)
}

/// Ends the session there; while the connection is still the machine's, the kept listing, if any, loses the session
/// and is reported, and so does a listing that began before and is kept later.
async fn end_session<T: Transport>(
    state: Rc<State<T>>,
    link: OnMachine<T::Connection>,
    machine_id: String,
    session_id: String,
) -> Result<(), MobileError> {
    let operation = Operation::EndSession { id: session_id.clone() };
    if !is_plain(&session_id) {
        return Err(remote(INVALID_REMOTE_SESSION.to_owned()));
    }
    if let Some(ended) = catalog_action(&state, &link, &machine_id, &operation).await {
        ended?;
    } else {
        let app = state.app_of(&machine_id, &session_id);
        sessions::end(&*link.connection, &link.host, &session_id, app)
            .await
            .map_err(remote)?;
        catalogs::forget_session(&state, &machine_id, &session_id);
    }
    if !state.is_current(&machine_id, &link.connection) {
        return Ok(());
    }
    let ticket = state.ticket();
    {
        let mut listings = state.listings.borrow_mut();
        let kept = listings.entry(machine_id.clone()).or_default();
        kept.ended.push((ticket, session_id.clone()));
        if let Some(tree) = kept.tree.as_mut() {
            sessions::forget(tree, &session_id);
        }
    }
    catalogs::report(&state, &machine_id);
    Ok(())
}

/// Renames the session there; while the connection is still the machine's, the kept listing takes the name and is
/// reported. On a headless machine with its catalog up, the catalog brings the name instead; with it down, the kept
/// catalog takes the name too.
async fn rename_session<T: Transport>(
    state: Rc<State<T>>,
    link: OnMachine<T::Connection>,
    machine_id: String,
    session_id: String,
    name: String,
) -> Result<(), MobileError> {
    if !is_plain(&session_id) {
        return Err(remote(INVALID_REMOTE_SESSION.to_owned()));
    }
    let operation = Operation::RenameSession {
        id: session_id.clone(),
        name: name.clone(),
    };
    if let Some(renamed) = catalog_action(&state, &link, &machine_id, &operation).await {
        return renamed.map(drop);
    }
    let app = state.app_of(&machine_id, &session_id);
    sessions::rename(&*link.connection, &link.host, (&session_id, app), &name)
        .await
        .map_err(remote)?;
    catalogs::rename_session(&state, &machine_id, &session_id, &name);
    if !state.is_current(&machine_id, &link.connection) {
        return Ok(());
    }
    if let Some(tree) = state
        .listings
        .borrow_mut()
        .get_mut(&machine_id)
        .and_then(|kept| kept.tree.as_mut())
    {
        sessions::renamed(tree, &session_id, &name);
    }
    catalogs::report(&state, &machine_id);
    Ok(())
}

/// Sends `operation` under an operation id when `machine_id`'s catalog is up, and answers once the catalog shows what
/// it applied, or [`READY_BUDGET`] runs out (New Session at once, as its caller waits for the row): the command's own
/// reply, `None` when only a lookup told or nothing changed. `None` while the machine has no live catalog: the caller
/// sends it as a plain command, as it does for a session the catalog does not hold.
async fn catalog_action<T: Transport>(
    state: &Rc<State<T>>,
    link: &OnMachine<T::Connection>,
    machine_id: &str,
    operation: &Operation,
) -> Option<Result<Option<Box<Response<ControlResult>>>, MobileError>> {
    let position = catalogs::position(state, machine_id)?;
    // a session of the machine's other app is not the catalog's to act on
    if let Operation::RenameSession { id, .. } | Operation::EndSession { id } = operation
        && !catalogs::holds(state, machine_id, id)
    {
        return None;
    }
    let op_id = uuid_string(Uuid::new_v4());
    let form = OriginForm {
        peer: RemotePeer::Ngterm,
        ..link.connection.origin_form()
    };
    let settled = operations::send(&*link.connection, (&link.host, form), operation, &op_id, &position).await;
    note!(
        INFO,
        tag::SESSIONS,
        "catalog operation",
        operation = operation.label(),
        outcome = match &settled {
            Settled::Applied { .. } => "applied",
            Settled::NoChange => "noop",
            Settled::Failed(_) => "failed",
        }
    );
    Some(match settled {
        Settled::Failed(reason) => Err(remote(reason)),
        Settled::NoChange => Ok(None),
        Settled::Applied { epoch, rev, reply } => {
            // New Session's caller waits for its row instead
            let waits = !matches!(operation, Operation::NewSession { .. });
            if let Some(rev) = rev.filter(|_| waits) {
                // the answer waits for the rows to show the change; a catalog that never gets there still answers
                let _ = catalogs::wait_for(state, machine_id, READY_BUDGET, |mirror| {
                    mirror.reaches(&epoch, rev).then_some(())
                })
                .await;
            }
            Ok(reply)
        }
    })
}

/// New Session's answer once the catalog applied it: the created row once the catalog shows it attachable, with the
/// machine listed again so that its panes show.
async fn created_in_catalog<T: Transport>(
    state: &Rc<State<T>>,
    link: &OnMachine<T::Connection>,
    machine_id: &str,
    reply: Option<Box<Response<ControlResult>>>,
) -> Result<RemoteSession, MobileError> {
    let id = reply
        .and_then(|reply| reply.result)
        .and_then(|result| result.id)
        .filter(|id| is_plain(id));
    let Some(id) = id else {
        return Err(remote(format!(
            "{} made the session; {}",
            link.host,
            sessions::ATTACH_FROM_LIST
        )));
    };
    let row = catalogs::shown(state, machine_id, &id, READY_BUDGET)
        .await
        .ok_or_else(|| {
            remote(sessions::phone_wording(unattached(
                SESSION_NOUN,
                &id,
                &link.host,
                NOT_ATTACHABLE,
            )))
        })?;
    // panes are shown from the listing, so it must hold the session the catalog made
    let listing = state.begin_listing();
    if let Ok(tree) = sessions::list(&*link.connection, &link.host).await {
        listed(state, link, machine_id, tree, listing.began);
    } else {
        note!(
            WARN,
            tag::SESSIONS,
            "listing after new session failed",
            machine = machine_id
        );
    }
    Ok(row)
}

/// Opens the presentation stream of `session_id`, a session of `machine_id`'s last listing, on the machine's
/// connection.
fn attach_session<T: Transport>(state: &Rc<State<T>>, machine_id: &str, session_id: &str) -> Result<(), MobileError> {
    let connection = connection_of(state, machine_id)?;
    if let Some(reason) = catalogs::attach_refusal(state, machine_id, session_id) {
        return Err(remote(reason));
    }
    let (version, app) = state
        .listing_of(machine_id, session_id, |app| (app.presentation, app.endpoint.peer()))
        .ok_or_else(|| MobileError::NotListed {
            session_id: session_id.to_owned(),
        })?;
    let events = state.presentations.borrow_mut().attach(
        machine_id,
        session_id,
        version,
        ChannelLinks::new(connection, reporter(state)).of_app(app),
        sessions::now(),
    )?;
    state.presented(events);
    Ok(())
}

/// `machine_id`'s connection.
fn connection_of<T: Transport>(state: &State<T>, machine_id: &str) -> Result<Rc<T::Connection>, MobileError> {
    state
        .connections
        .borrow()
        .get(machine_id)
        .map(|link| Rc::clone(&link.connection))
        .ok_or_else(|| MobileError::NotConnected {
            machine_id: machine_id.to_owned(),
        })
}

/// Where the streams' links report: the actor's [`Presentations`], once the link's task has the report.
fn reporter<T: Transport>(state: &Rc<State<T>>) -> Reporter {
    let state = Rc::downgrade(state);
    Rc::new(move |key, event: LinkEvent| {
        let Some(state) = state.upgrade() else { return };
        let events = state.presentations.borrow_mut().link_event(key, event, sessions::now());
        state.presented(events);
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
        state.presented(events);
        state.chats.borrow_mut().tick(sessions::now());
    }
}

/// The listed session `raw`, a link tapped in `pane`, names. A session link the machine's kept listing misses is
/// looked for again in a fresh listing: a program can make the session after the last one (upstream's viewer keeps
/// its listing live, `WindowLibrary+SessionLink.swift`).
fn session_link<T: Transport>(
    state: &Rc<State<T>>,
    pane: PaneRef,
    raw: String,
    reply: oneshot::Sender<Result<Option<RemoteSession>, PaneInputError>>,
) {
    let target = kept_link_target(state, &pane, &raw);
    if target.is_some() || !sessions::is_session_link(&raw) {
        noted_link(&pane, target.as_ref(), false);
        let _ = reply.send(Ok(target));
        return;
    }
    let (listed, listing) = oneshot::channel();
    spawn_on_machine(state, pane.machine_id.clone(), listed, list_sessions);
    let state = Rc::clone(state);
    tokio::task::spawn_local(async move {
        // a failed listing leaves the kept one, which misses the session
        let _ = listing.await;
        let target = kept_link_target(&state, &pane, &raw);
        noted_link(&pane, target.as_ref(), true);
        let _ = reply.send(Ok(target));
    });
}

/// The listed session `raw`, a link tapped in `pane`, names in its machine's kept listing.
fn kept_link_target<T: Transport>(state: &State<T>, pane: &PaneRef, raw: &str) -> Option<RemoteSession> {
    state
        .listings
        .borrow()
        .get(&pane.machine_id)
        .and_then(|kept| kept.tree.as_ref())
        .and_then(|tree| sessions::session_link_target(tree, &pane.session_id, raw))
}

fn noted_link(pane: &PaneRef, target: Option<&RemoteSession>, relisted: bool) {
    note!(
        INFO,
        tag::PANE,
        "session link tapped",
        found = target.is_some(),
        relisted = relisted,
        session = pane.session_id.as_str(),
        pane = pane.pane_id.as_str()
    );
}

/// The pane commands.
fn pane_command<T: Transport>(state: &Rc<State<T>>, command: Command) {
    match command {
        Command::ShowPane {
            machine_id,
            session_id,
            pane_id,
            size,
            reply,
        } => {
            let pane = PaneRef {
                machine_id,
                session_id,
                pane_id,
            };
            let shown = show_pane(state, &pane, size);
            note!(
                INFO,
                tag::PANE,
                "show asked",
                result = result_class(&shown),
                cols = size.cols,
                rows = size.rows,
                session = pane.session_id.as_str(),
                pane = pane.pane_id.as_str()
            );
            let _ = reply.send(shown);
        }
        Command::HidePane(pane) => {
            note!(INFO, tag::PANE, "hide asked", pane = pane.pane_id.as_str());
            let Some(id) = pane_uuid(&pane) else { return };
            if let Ok(key) = ChatKey::of(&pane) {
                state.chats.borrow_mut().close(&key);
            }
            let events = state.panes.borrow_mut().hide(&pane.machine_id, &pane.session_id, id);
            state.emit_all(events);
        }
        Command::TakeLead(pane) => {
            note!(
                INFO,
                tag::PANE,
                "take control asked",
                session = pane.session_id.as_str(),
                pane = pane.pane_id.as_str()
            );
            let Some(id) = pane_uuid(&pane) else { return };
            let events = state
                .panes
                .borrow_mut()
                .take_lead(&pane.machine_id, &pane.session_id, id);
            state.emit_all(events);
        }
        Command::AnswerClipboardRead { pane, read_id, reply } => {
            let Some(id) = pane_uuid(&pane) else { return };
            state.panes.borrow_mut().answer_read(&pane, id, read_id, reply);
        }
        Command::CompleteClipboardWrite { pane, write_id, status } => {
            let Some(id) = pane_uuid(&pane) else { return };
            state.panes.borrow_mut().complete_write(&pane, id, write_id, status);
        }
        // The loop handles every other command itself.
        _ => {}
    }
}

fn pane_uuid(pane: &PaneRef) -> Option<Uuid> {
    Uuid::parse_str(&pane.pane_id).ok()
}

/// Shows a pane of a session of the machine's last listing, on the machine's connection.
fn show_pane<T: Transport>(state: &Rc<State<T>>, pane: &PaneRef, size: PaneSize) -> Result<Arc<Pane>, MobileError> {
    let connection = connection_of(state, &pane.machine_id)?;
    let unknown = || MobileError::Remote {
        reason: pane::UNKNOWN_PANE.to_owned(),
    };
    let id = pane_uuid(pane);
    let found = state.listing_of(&pane.machine_id, &pane.session_id, |app| {
        let session = app.sessions.iter().find(|listed| listed.id == pane.session_id)?;
        let listed = session
            .panes
            .iter()
            .find(|listed| id.is_some() && pane_identity(&listed.daemon) == id)?;
        Some((listed.daemon.clone(), app.endpoint.clone()))
    });
    let (daemon, endpoint) = found
        .ok_or_else(|| MobileError::NotListed {
            session_id: pane.session_id.clone(),
        })?
        .ok_or_else(unknown)?;
    let target = Target {
        machine_id: pane.machine_id.clone(),
        session_id: pane.session_id.clone(),
        pane: id.ok_or_else(unknown)?,
        daemon,
        endpoint,
    };
    let (slot, events) = state.panes.borrow_mut().show(target, connection, size.into())?;
    state.emit_all(events);
    let handle = Pane::new(pane.pane_id.to_uppercase(), slot).acting(pane.clone(), state.wiring.commands.clone());
    Ok(Arc::new(handle))
}

/// Hands each note of the panes' threads and tasks to the panes, while the actor runs.
async fn pane_notes<T: Transport>(state: Weak<State<T>>, mut inbox: mpsc::UnboundedReceiver<Note>) {
    while let Some(note) = inbox.recv().await {
        let Some(state) = state.upgrade() else { return };
        let handled = state.panes.borrow_mut().note(note);
        state.emit_all(handled.events);
        if let Some(check) = handled.check {
            check_gone(&state, check);
        }
    }
}

/// Reads the machine's listing to tell whether a pane whose attach printed the gone guard's line lost its session:
/// gone when the listing no longer holds it. The listing is kept and reported as any other; a machine with no
/// connection, or a listing that fails, leaves the pane's exit a plain one.
fn check_gone<T: Transport>(state: &Rc<State<T>>, check: GoneCheck) {
    let link = state.connections.borrow().get(&check.machine_id).map(|link| OnMachine {
        connection: Rc::clone(&link.connection),
        host: shown_name(&link.machine),
    });
    let Some(link) = link else {
        let events = state.panes.borrow_mut().gone_checked(check.key, false);
        state.emit_all(events);
        return;
    };
    let state = Rc::clone(state);
    tokio::task::spawn_local(async move {
        let listing = state.begin_listing();
        let gone = match sessions::list(&*link.connection, &link.host).await {
            Ok(tree) => {
                let tree = with_skipped_app(&state, &check.machine_id, tree);
                let gone = !tree
                    .apps()
                    .any(|app| app.sessions.iter().any(|listed| listed.id == check.session_id));
                listed(&state, &link, &check.machine_id, tree, listing.began);
                gone
            }
            Err(_) => false,
        };
        let events = state.panes.borrow_mut().gone_checked(check.key, gone);
        state.emit_all(events);
    });
}
