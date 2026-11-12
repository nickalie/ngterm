//! The `UniFFI` facade the mobile apps call through their bindings (Kotlin on Android, Swift on iOS): [`MobileCore`]
//! and its handles (a shown pane's [`Pane`]), the records and events they carry, and the callback interfaces the app
//! implements ([`EventSink`], [`KeyVault`], [`HostKeyStore`]).
//!
//! Everything here is thin: a [`MobileCore`] call posts a command to the core's actor (`crate::actor`) and, when it
//! has an answer, awaits it (a Kotlin `suspend fun`, a Swift `async` function); what the actor learns on its own
//! reaches the app as [`MobileEvent`]s through the [`EventSink`], on the actor's thread. `UniFFI` passes owned
//! arguments, so a signature clippy would take by reference keeps its owned form under an item-level `#[expect]`.

mod pane;

use std::sync::Arc;

use tokio::sync::oneshot;

pub(crate) use self::pane::PaneSlot;
pub use self::pane::{
    CellPoint, Pane, PaneColors, PaneInputError, PaneKey, PaneKeyAction, PaneMouse, PaneMouseAction, PaneMouseButton,
    PaneSendOutcome, PaneSize, PaneWheel, PaneWheelOutcome,
};

use crate::actor::{Actor, Command};
use crate::chat::input::InputTiming;
use crate::chat::{AgentCommand, ChatEvent, ChatState, UploadCancel};
use crate::keys::{self, KeyError, KeyPair};
use crate::pane::{ClipboardPart, ClipboardReply, ClipboardWriteStatus, PaneClipboard, PaneEnd, PaneRef, PaneRole};
use crate::presentation::{
    AskReply, PresentationRole, RemoteAsk, RemoteHud, RemoteLayout, RemoteNotice, RemoteStatus, StreamState,
};
use crate::sessions::RemoteSession;
use crate::transport::Transport;
use crate::transport::russh::RusshTransport;

/// A machine the app knows, as it stores it and passes it in.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Machine {
    /// The app's stable id of the machine; every event about it names it.
    pub id: String,
    /// What the app shows for it.
    pub label: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    /// The [`KeyVault`] id of the private key the machine is reached with.
    pub key_id: String,
}

impl Machine {
    /// Whether `other` is reached as this one is: the same host, port, user and key. A record differing in its label
    /// alone is the same machine renamed.
    #[must_use]
    pub fn reached_as(&self, other: &Self) -> bool {
        self.host == other.host && self.port == other.port && self.user == other.user && self.key_id == other.key_id
    }
}

/// A machine's connection.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum MachineState {
    Connected,
    /// Down after being connected, retried in `next_in_ms`: every failure of a reconnect but a refusal is reported so,
    /// a changed host key arriving as [`MobileEvent::HostKeyChanged`] besides.
    Reconnecting {
        next_in_ms: u64,
    },
    /// Down until the app connects it again: a connect the app asked for that failed, or a reconnect refused for a
    /// reason a retry won't fix by itself (a changed host key, an unknown one not trusted, a key the machine does not
    /// accept).
    Unreachable {
        reason: String,
    },
}

/// What the core tells the app on its own, through [`EventSink::on_event`].
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum MobileEvent {
    /// A machine's connection changed.
    MachineState { machine_id: String, state: MachineState },
    /// A machine with no pinned key offered `fingerprint`, and its connect waits for
    /// [`MobileCore::answer_host_key`]: trusting it pins it through [`HostKeyStore::pin`] and the connection
    /// proceeds.
    HostKeyUnknown { machine_id: String, fingerprint: String },
    /// A machine offered a key other than the pinned one; the connection stopped. Only removing the pin from the
    /// [`HostKeyStore`] lets a later connect ask about the new key.
    HostKeyChanged {
        machine_id: String,
        pinned: String,
        offered: String,
    },
    /// A machine asks the user to sign in at `url` (an http(s) link from its ssh banner, `message`, which is
    /// Tailscale SSH's way for a `check`) before it lets a connect in, and the connect waits for that up to ten minutes.
    /// The wait ends with the connect: the machine's next [`MobileEvent::MachineState`] (`Connected` once the user
    /// signed in, `Unreachable` or `Reconnecting` when it failed or the wait ran out), or the app's
    /// [`MobileCore::disconnect`]. A connect overtaken meanwhile (a suspend, an edit) reports nothing, as ever; one
    /// that takes its place asks again if the machine still wants a sign-in. A later banner with a link while it waits
    /// arrives as another `SignInRequested`.
    SignInRequested {
        machine_id: String,
        message: String,
        url: String,
    },
    /// A machine's sessions as it last listed them (after [`MobileCore::list_sessions`], a create, or an end), in the
    /// order to show them: no page companions, parents before their companions. A listing that finishes after its
    /// connection closed or was replaced, or after a later listing or an end landed, is stale and not reported.
    Tree {
        machine_id: String,
        sessions: Vec<RemoteSession>,
    },
    /// An attached session's presentation stream changed state ([`MobileCore::attach_session`]). Leaving `Connected`
    /// is followed by what that takes away: the open asks dismissed, the context, status and HUD cleared.
    SessionConnection {
        machine_id: String,
        session_id: String,
        state: StreamState,
    },
    /// An attached session's agent status. `changed` is false for the status a snapshot carries (one arrives with
    /// every reconnect) and for the clearing when the stream goes: no change on the origin, so nothing to notify.
    SessionStatus {
        machine_id: String,
        session_id: String,
        status: Option<RemoteStatus>,
        changed: bool,
    },
    /// An attached session's note of what it is for.
    SessionContext {
        machine_id: String,
        session_id: String,
        context: Option<String>,
    },
    /// An attached session's name as its origin shows it: with every snapshot (from an origin that sends one) and on
    /// each rename. It outlasts the stream's connection, as a session keeps its name while its link is down.
    SessionName {
        machine_id: String,
        session_id: String,
        name: String,
    },
    /// An attached session's panes and their arrangement.
    SessionLayout {
        machine_id: String,
        session_id: String,
        layout: RemoteLayout,
    },
    /// This app's role on an attached session.
    SessionMode {
        machine_id: String,
        session_id: String,
        role: PresentationRole,
    },
    /// The HUD panel over an attached session; `None` takes it down.
    Hud {
        machine_id: String,
        session_id: String,
        hud: Option<RemoteHud>,
    },
    /// A notification from an attached session, to post as the app's own.
    Notify {
        machine_id: String,
        session_id: String,
        notice: RemoteNotice,
    },
    /// A status sound an attached session's origin handed over instead of playing it: `name`, or for `None` the app's
    /// own blocked sound.
    Sound {
        machine_id: String,
        session_id: String,
        name: Option<String>,
    },
    /// An ask from an attached session: a dialog when the session is on screen, otherwise a notification. Answer it
    /// with [`MobileCore::answer_ask`].
    Ask {
        machine_id: String,
        session_id: String,
        ask: RemoteAsk,
    },
    /// An ask ended without this app's answer (answered elsewhere, ended on the origin, or its stream went): take it
    /// down.
    AskDismiss {
        machine_id: String,
        session_id: String,
        ask_id: String,
    },
    /// A shown pane has a fresh terminal (its first attach, or another after a lost link, a respawn or a takeover):
    /// draw it from scratch.
    PaneAttached { pane: PaneRef },
    /// A shown pane's terminal has a new frame to draw.
    PaneDirty { pane: PaneRef },
    /// A shown pane's program set its title (never the attach client's role reports, which arrive as
    /// [`MobileEvent::PaneLead`]).
    PaneTitle { pane: PaneRef, title: String },
    /// A shown pane's role at its session (`None` until its attach reports one) and whether it is covered: a
    /// follower's grid is another client's, and a pane attaching again has nothing to show yet. A covered pane offers
    /// [`MobileCore::take_lead`].
    PaneLead {
        pane: PaneRef,
        role: Option<PaneRole>,
        covered: bool,
    },
    /// A shown pane's attach ended, as [`PaneEnd`] says; a pane covered for its role is no longer covered.
    PaneEnded { pane: PaneRef, end: PaneEnd },
    /// A shown pane's program rang the bell.
    Bell { pane: PaneRef },
    /// A shown pane's program sets the device's clipboard to `contents`, every representation of one value (text as
    /// `text/plain`, at most once; none: clear it). The app keeps the types its clipboard can hold and answers
    /// [`MobileCore::complete_clipboard_write`] with `write_id` once the clipboard holds it (or refused it): the
    /// program's output waits for that answer. A write to another clipboard than the standard one (the device has
    /// none) is answered by the core and never reaches the app.
    ClipboardWrite {
        pane: PaneRef,
        write_id: u64,
        contents: Vec<ClipboardPart>,
    },
    /// A shown pane's program asks to read the clipboard: its output waits for [`MobileCore::answer_clipboard_read`].
    /// `mimes` are the types it wants (at most four, in its order of preference, text as `text/plain`; OSC 52 asks for
    /// text alone), and `list` whether it also wants the types the clipboard holds. `list_only` (the Kitty protocol's
    /// listing) and a read with no `mimes` disclose no data, so they need no consent; `granted` says a grant the
    /// program holds in this terminal (the Kitty protocol's password) already allows it; `can_remember` that an Allow
    /// may keep such a grant.
    ClipboardReadRequest {
        pane: PaneRef,
        read_id: u64,
        clipboard: PaneClipboard,
        list_only: bool,
        mimes: Vec<String>,
        list: bool,
        granted: bool,
        can_remember: bool,
    },
    /// A clipboard read was denied without the user's answer (the pane hidden, its link lost, its program ended):
    /// take its prompt down.
    ClipboardReadDismiss { pane: PaneRef, read_id: u64 },
    /// What a pane of an attached session runs, as its origin announces it: `agent` (`claude`, `codex`, or a name a
    /// newer origin knows) and its conversation, or `None` when the pane runs none. Every pane is announced when the
    /// stream connects, then on each change; an origin that predates the chat view announces none. A pane with an
    /// agent shows as a chat ([`MobileCore::open_chat`]) when the app's setting says so.
    PaneAgent {
        machine_id: String,
        session_id: String,
        /// The pane's identity, as the session's layout names it.
        pane_id: String,
        agent: Option<String>,
        conversation_id: Option<String>,
    },
    /// Events of a pane's open chat ([`MobileCore::open_chat`]), in order: the backlog, `live`, then each new one.
    /// A later event with an id already shown updates that item; a `reset` drops everything shown.
    Chat { pane: PaneRef, events: Vec<ChatEvent> },
    /// A pane's open chat is reconnecting, or ended for good (show the terminal).
    ChatState { pane: PaneRef, state: ChatState },
}

/// Why a [`MobileCore`] call failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum MobileError {
    /// The machine could not be reached.
    #[error("{reason}")]
    Unreachable { reason: String },
    /// The call was overtaken by a later one (a disconnect, or a connect of the machine as edited, while connecting).
    #[error("cancelled")]
    Cancelled,
    /// The core's actor has stopped (its runtime failed to start, or the core is shutting down).
    #[error("the mobile core has stopped")]
    Stopped,
    /// The machine has no connection: connect it first.
    #[error("{machine_id} is not connected")]
    NotConnected { machine_id: String },
    /// The machine answered, or failed to: why a session command did not do what was asked, in words for the user.
    #[error("{reason}")]
    Remote { reason: String },
    /// The session is not in the machine's last listing: list the machine's sessions first.
    #[error("{session_id} is not in the machine's session list")]
    NotListed { session_id: String },
}

/// Why a [`KeyVault`] could not hand over a key.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum VaultError {
    /// No key has that id.
    #[error("no key {key_id}")]
    Missing { key_id: String },
    /// The key exists but could not be read (the Android Keystore or the iOS Keychain refused, the app's callback
    /// failed).
    #[error("{reason}")]
    Unavailable { reason: String },
}

impl From<uniffi::UnexpectedUniFFICallbackError> for VaultError {
    fn from(error: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Unavailable { reason: error.reason }
    }
}

/// Where the core's events go: the app's, called on the core's actor thread, never the caller's. It hands each event
/// on and returns, since the actor waits for it.
#[uniffi::export(with_foreign)]
pub trait EventSink: Send + Sync {
    fn on_event(&self, event: MobileEvent);
}

/// The app's private keys, kept by the platform (wrapped by the Android Keystore, held in the iOS Keychain):
/// [`KeyVault::load`] hands one over for a connection, which wipes the bytes once it has used them.
#[uniffi::export(with_foreign)]
pub trait KeyVault: Send + Sync {
    /// The OpenSSH private key bytes of `key_id`.
    ///
    /// # Errors
    ///
    /// [`VaultError::Missing`] when no key has the id, [`VaultError::Unavailable`] when it can't be unwrapped.
    fn load(&self, key_id: String) -> Result<Vec<u8>, VaultError>;
}

/// How far an upload ([`MobileCore::upload_file`]) has got, told on the core's actor thread: hand it on and return.
#[uniffi::export(with_foreign)]
pub trait UploadProgress: Send + Sync {
    /// `sent` of the file's `total` bytes have gone to the machine.
    fn sent(&self, sent: u64, total: u64);
}

/// The app's pinned host keys (trust on first use).
#[uniffi::export(with_foreign)]
pub trait HostKeyStore: Send + Sync {
    /// The fingerprint pinned for `machine_id`, if any.
    fn pinned(&self, machine_id: String) -> Option<String>;
    /// Pins `fingerprint` for `machine_id`, as the user accepted it.
    fn pin(&self, machine_id: String, fingerprint: String);
}

/// The app's core: the machines' connections and everything on them, run by one actor thread. Dropping the last
/// reference stops the actor and waits for it, unless the drop happens on the actor's own thread (from an
/// [`EventSink`] callback), where it lets the thread finish by itself.
#[derive(uniffi::Object)]
pub struct MobileCore {
    actor: Actor,
}

#[uniffi::export]
impl MobileCore {
    /// The app's core: ssh through russh, with keys from `vault` and host keys pinned in `hosts`, its events going to
    /// `sink`.
    ///
    /// # Errors
    ///
    /// [`MobileError::Stopped`] when the actor's runtime or thread can't start.
    #[uniffi::constructor]
    pub fn new(
        sink: Arc<dyn EventSink>,
        vault: Arc<dyn KeyVault>,
        hosts: Arc<dyn HostKeyStore>,
    ) -> Result<Arc<Self>, MobileError> {
        let transport = RusshTransport::new(vault, hosts, Arc::clone(&sink));
        Self::with_transport(transport, sink)
    }
}

impl MobileCore {
    /// A core over `transport`, its events going to `sink`.
    ///
    /// # Errors
    ///
    /// [`MobileError::Stopped`] when the actor's runtime or thread can't start.
    pub fn with_transport<T>(transport: T, sink: Arc<dyn EventSink>) -> Result<Arc<Self>, MobileError>
    where
        T: Transport + Send,
    {
        Self::with_timing(transport, sink, InputTiming::default())
    }

    /// A core over `transport` whose chat actions settle, wait and look as `timing` says (the tests' shorter waits).
    pub(crate) fn with_timing<T>(
        transport: T,
        sink: Arc<dyn EventSink>,
        timing: InputTiming,
    ) -> Result<Arc<Self>, MobileError>
    where
        T: Transport + Send,
    {
        Ok(Arc::new(Self {
            actor: Actor::spawn(transport, sink, timing)?,
        }))
    }
}

#[uniffi::export]
impl MobileCore {
    /// Connects `machine`, answered once it is connected (at once when it already is; a connect of the same machine in
    /// flight is joined, not repeated, and a machine waiting to reconnect is retried at once). Once connected, a lost
    /// connection is retried on its own, reported as [`MachineState::Reconnecting`], until
    /// [`MobileCore::disconnect`]. A connect of the machine as edited (another host, port, user or key:
    /// [`Machine::reached_as`]) closes its connection and connects anew; one only renamed keeps it and takes the new
    /// label. The state also arrives as a
    /// [`MobileEvent::MachineState`].
    ///
    /// # Errors
    ///
    /// [`MobileError::Unreachable`] when the machine can't be reached, [`MobileError::Cancelled`] when a disconnect
    /// or a connect of the machine as edited (another host, port, user or key) overtook the connect, [`MobileError::Stopped`] when the core has stopped.
    pub async fn connect(&self, machine: Machine) -> Result<(), MobileError> {
        let (reply, answer) = oneshot::channel();
        self.actor.post(Command::Connect { machine, reply })?;
        answer.await.unwrap_or(Err(MobileError::Stopped))
    }

    /// Closes the connection to `machine_id`, if any, cancels a connect or reconnect in flight, ends its attached
    /// sessions' streams and hides its shown panes.
    ///
    /// # Errors
    ///
    /// [`MobileError::Stopped`] when the core has stopped.
    pub fn disconnect(&self, machine_id: String) -> Result<(), MobileError> {
        self.actor.post(Command::Disconnect { machine_id })
    }

    /// `machine_id` signs in with the key `key_id` from its next connection attempt (a reconnect included): a
    /// connection it has stays, with its attached sessions, and a connect already reaching the machine goes on with
    /// the key it started with. The machine's record is then the one with `key_id`, so a connect of it with that key
    /// is no edit. Nothing happens for a machine the core doesn't hold.
    ///
    /// # Errors
    ///
    /// [`MobileError::Stopped`] when the core has stopped.
    pub fn use_key(&self, machine_id: String, key_id: String) -> Result<(), MobileError> {
        self.actor.post(Command::UseKey { machine_id, key_id })
    }

    /// The user's answer about the unknown host key `fingerprint` of `machine_id` (a
    /// [`MobileEvent::HostKeyUnknown`]): `trust` pins it and the waiting connect goes on; a refusal, or an answer
    /// about a key other than the one on offer, ends that connect. An answer with no connect waiting changes
    /// nothing.
    ///
    /// # Errors
    ///
    /// [`MobileError::Stopped`] when the core has stopped.
    pub fn answer_host_key(&self, machine_id: String, fingerprint: String, trust: bool) -> Result<(), MobileError> {
        self.actor.post(Command::AnswerHostKey {
            machine_id,
            fingerprint,
            trust,
        })
    }

    /// `machine_id`'s sessions, as its `ngtermctl live tree` lists them for a picker: no page companions, parents
    /// before their companions. They also arrive as a [`MobileEvent::Tree`] unless stale by then (see there).
    ///
    /// # Errors
    ///
    /// [`MobileError::NotConnected`], [`MobileError::Remote`] with why there is no listing, or
    /// [`MobileError::Stopped`].
    pub async fn list_sessions(&self, machine_id: String) -> Result<Vec<RemoteSession>, MobileError> {
        let (reply, answer) = oneshot::channel();
        self.actor.post(Command::ListSessions { machine_id, reply })?;
        answer.await.unwrap_or(Err(MobileError::Stopped))
    }

    /// New Session on `machine_id`, named `name` when given (a blank name is none): answered with the session once
    /// the machine lists it as attachable, the listing also arriving as a [`MobileEvent::Tree`] unless stale by then. The machine keeps
    /// its own selection.
    ///
    /// # Errors
    ///
    /// [`MobileError::NotConnected`], [`MobileError::Remote`] with why it was not created (the machine runs no live
    /// sessions, its ngterm is too old, it refused) or was created but not attachable in time, or
    /// [`MobileError::Stopped`].
    pub async fn create_session(&self, machine_id: String, name: Option<String>) -> Result<RemoteSession, MobileError> {
        let (reply, answer) = oneshot::channel();
        self.actor.post(Command::CreateSession {
            machine_id,
            name,
            reply,
        })?;
        answer.await.unwrap_or(Err(MobileError::Stopped))
    }

    /// Duplicate: `machine_id` copies its session `session_id` as desktop's Duplicate Session does (a fresh shell in
    /// the source's focused directory, right after it, with a name of its own; a companion's copy stands on its own,
    /// and companions are not copied), answered as [`MobileCore::create_session`] is. The machine selects the copy.
    ///
    /// # Errors
    ///
    /// As [`MobileCore::create_session`], with [`MobileError::Remote`] also for an id that is not a session's.
    pub async fn duplicate_session(
        &self,
        machine_id: String,
        session_id: String,
    ) -> Result<RemoteSession, MobileError> {
        let (reply, answer) = oneshot::channel();
        self.actor.post(Command::DuplicateSession {
            machine_id,
            session_id,
            reply,
        })?;
        answer.await.unwrap_or(Err(MobileError::Stopped))
    }

    /// End Session: `machine_id` closes its session `session_id`, its programs and companions with it, with no undo
    /// (the app confirms first). When a listing of the machine is kept (one was read on this connection), the listing
    /// without it arrives as a [`MobileEvent::Tree`].
    ///
    /// # Errors
    ///
    /// [`MobileError::NotConnected`], [`MobileError::Remote`] with why it did not end, or [`MobileError::Stopped`].
    pub async fn end_session(&self, machine_id: String, session_id: String) -> Result<(), MobileError> {
        let (reply, answer) = oneshot::channel();
        self.actor.post(Command::EndSession {
            machine_id,
            session_id,
            reply,
        })?;
        answer.await.unwrap_or(Err(MobileError::Stopped))
    }

    /// Attaches `session_id` of `machine_id`: its presentation stream opens and stays up, reconnecting on its own
    /// backoff, until [`MobileCore::detach_session`]. What it shows arrives as events
    /// ([`MobileEvent::SessionConnection`] and the session events after it). Attaching an attached session changes
    /// nothing.
    ///
    /// # Errors
    ///
    /// [`MobileError::NotConnected`], [`MobileError::NotListed`] when the machine's last listing doesn't hold the
    /// session, [`MobileError::Remote`] for an id that is not a session's, or [`MobileError::Stopped`].
    pub async fn attach_session(&self, machine_id: String, session_id: String) -> Result<(), MobileError> {
        let (reply, answer) = oneshot::channel();
        self.actor.post(Command::AttachSession {
            machine_id,
            session_id,
            reply,
        })?;
        answer.await.unwrap_or(Err(MobileError::Stopped))
    }

    /// Detaches `session_id` of `machine_id`: its stream ends and its shown panes are hidden
    /// ([`MobileCore::hide_pane`]), with no further events about them. The session goes on running there.
    ///
    /// # Errors
    ///
    /// [`MobileError::Stopped`] when the core has stopped.
    pub fn detach_session(&self, machine_id: String, session_id: String) -> Result<(), MobileError> {
        self.actor.post(Command::DetachSession { machine_id, session_id })
    }

    /// The device's network changed (another Wi-Fi, mobile data, back online): every machine waiting to reconnect is
    /// retried at once, its backoff starting over.
    ///
    /// # Errors
    ///
    /// [`MobileError::Stopped`] when the core has stopped.
    pub fn network_changed(&self) -> Result<(), MobileError> {
        self.actor.post(Command::NetworkChanged)
    }

    /// The app has nothing to keep connected (in the background with no attached session to watch): every machine's
    /// connection closes and every connect in flight is cancelled ([`MobileError::Cancelled`]), with no state
    /// reported, and the attached sessions' streams stop quietly. An answer posted before ([`MobileCore::answer_ask`])
    /// is written to its origin before the connection closes. [`MobileCore::resume`] brings them back.
    ///
    /// # Errors
    ///
    /// [`MobileError::Stopped`] when the core has stopped.
    pub fn suspend(&self) -> Result<(), MobileError> {
        self.actor.post(Command::Suspend)
    }

    /// Brings back what [`MobileCore::suspend`] closed: each machine that had been connected reconnects at once (and
    /// keeps retrying, reported as [`MachineState::Reconnecting`]), one whose first connect was cancelled is connected
    /// again as that connect (its failure reported as [`MachineState::Unreachable`]), and the attached sessions' streams
    /// start again once their machine is connected.
    ///
    /// # Errors
    ///
    /// [`MobileError::Stopped`] when the core has stopped.
    pub fn resume(&self) -> Result<(), MobileError> {
        self.actor.post(Command::Resume)
    }

    /// The user's answer to the ask `ask_id` of an attached session (a [`MobileEvent::Ask`]). An answer to an ask no
    /// longer open (dismissed, or its stream went) is dropped: the origin took the ask back.
    ///
    /// # Errors
    ///
    /// [`MobileError::Stopped`] when the core has stopped.
    pub fn answer_ask(
        &self,
        machine_id: String,
        session_id: String,
        ask_id: String,
        reply: AskReply,
    ) -> Result<(), MobileError> {
        self.actor.post(Command::AnswerAsk {
            machine_id,
            session_id,
            ask_id,
            reply,
        })
    }

    /// Shows the pane `pane_id` (its identity, as the session's layout names it) of `session_id`, a session of
    /// `machine_id`'s last listing, at `size`: its terminal attaches on a channel with a pty of that size, claiming
    /// nothing (it leads only a session nobody leads; [`MobileCore::take_lead`] claims). What happens to it arrives
    /// as pane events ([`MobileEvent::PaneAttached`] and the ones after). Showing a shown pane hands back a handle on
    /// the same pane.
    ///
    /// # Errors
    ///
    /// [`MobileError::NotConnected`], [`MobileError::NotListed`] when the machine's last listing doesn't hold the
    /// session, [`MobileError::Remote`] when the session has no such pane, or [`MobileError::Stopped`].
    pub async fn show_pane(
        &self,
        machine_id: String,
        session_id: String,
        pane_id: String,
        size: PaneSize,
    ) -> Result<Arc<Pane>, MobileError> {
        let (reply, answer) = oneshot::channel();
        self.actor.post(Command::ShowPane {
            machine_id,
            session_id,
            pane_id,
            size,
            reply,
        })?;
        answer.await.unwrap_or(Err(MobileError::Stopped))
    }

    /// Hides a shown pane: its channel closes and any clipboard read it holds open is denied. Its handles go dead
    /// ([`PaneInputError::Closed`]).
    ///
    /// # Errors
    ///
    /// [`MobileError::Stopped`] when the core has stopped.
    pub fn hide_pane(&self, machine_id: String, session_id: String, pane_id: String) -> Result<(), MobileError> {
        self.actor.post(Command::HidePane(PaneRef {
            machine_id,
            session_id,
            pane_id,
        }))
    }

    /// Take control: a shown pane attaches again claiming the lead, so its grid sizes the machine's pty and the
    /// client that led follows. A pane whose program ended, or whose takeover is on its way, stays as it is.
    ///
    /// # Errors
    ///
    /// [`MobileError::Stopped`] when the core has stopped.
    pub fn take_lead(&self, machine_id: String, session_id: String, pane_id: String) -> Result<(), MobileError> {
        self.actor.post(Command::TakeLead(PaneRef {
            machine_id,
            session_id,
            pane_id,
        }))
    }

    /// The user's answer to a shown pane's clipboard read `read_id` (a [`MobileEvent::ClipboardReadRequest`]); an
    /// answer to a read no longer open is dropped.
    ///
    /// # Errors
    ///
    /// [`MobileError::Stopped`] when the core has stopped.
    pub fn answer_clipboard_read(
        &self,
        machine_id: String,
        session_id: String,
        pane_id: String,
        read_id: u64,
        reply: ClipboardReply,
    ) -> Result<(), MobileError> {
        let pane = PaneRef {
            machine_id,
            session_id,
            pane_id,
        };
        self.actor.post(Command::AnswerClipboardRead { pane, read_id, reply })
    }

    /// How the app's write `write_id` of a shown pane's program (a [`MobileEvent::ClipboardWrite`]) ended, once the
    /// device's clipboard took it or refused it; the program learns it (the Kitty protocol's DONE, EPERM, ENOSYS or
    /// EIO) and its output goes on. An answer to a write no longer open is dropped.
    ///
    /// # Errors
    ///
    /// [`MobileError::Stopped`] when the core has stopped.
    pub fn complete_clipboard_write(
        &self,
        machine_id: String,
        session_id: String,
        pane_id: String,
        write_id: u64,
        status: ClipboardWriteStatus,
    ) -> Result<(), MobileError> {
        let pane = PaneRef {
            machine_id,
            session_id,
            pane_id,
        };
        self.actor
            .post(Command::CompleteClipboardWrite { pane, write_id, status })
    }
}

#[uniffi::export]
impl MobileCore {
    /// Opens the chat of a shown pane ([`MobileCore::show_pane`]) of an attached session, one the origin announced
    /// an agent for ([`MobileEvent::PaneAgent`]): its events arrive as [`MobileEvent::Chat`], and what becomes of it
    /// as [`MobileEvent::ChatState`]. Reading the chat claims nothing: the pane's own attach keeps its role. Opening an
    /// open chat changes nothing; hiding the pane closes it.
    ///
    /// # Errors
    ///
    /// [`MobileError::NotConnected`], [`MobileError::Remote`] when the pane is not shown or its session's layout
    /// does not hold it, or [`MobileError::Stopped`].
    pub async fn open_chat(&self, machine_id: String, session_id: String, pane_id: String) -> Result<(), MobileError> {
        let (reply, answer) = oneshot::channel();
        let pane = PaneRef {
            machine_id,
            session_id,
            pane_id,
        };
        self.actor.post(Command::OpenChat { pane, reply })?;
        answer.await.unwrap_or(Err(MobileError::Stopped))
    }

    /// Closes a pane's chat (the pane shows its terminal again, or leaves the screen), reporting nothing.
    ///
    /// # Errors
    ///
    /// [`MobileError::Stopped`] when the core has stopped.
    pub fn close_chat(&self, machine_id: String, session_id: String, pane_id: String) -> Result<(), MobileError> {
        self.actor.post(Command::CloseChat(PaneRef {
            machine_id,
            session_id,
            pane_id,
        }))
    }

    /// The page of a pane's open chat before the oldest event it holds, oldest first: empty once the conversation's
    /// start is reached. The page's events are not repeated as [`MobileEvent::Chat`].
    ///
    /// # Errors
    ///
    /// [`MobileError::Remote`] when the chat is not open or the origin failed to read the page,
    /// [`MobileError::NotConnected`] while the machine is down, [`MobileError::Cancelled`] when the chat closed or its
    /// conversation switched meanwhile, or [`MobileError::Stopped`].
    pub async fn load_older(
        &self,
        machine_id: String,
        session_id: String,
        pane_id: String,
    ) -> Result<Vec<ChatEvent>, MobileError> {
        let (reply, answer) = oneshot::channel();
        let pane = PaneRef {
            machine_id,
            session_id,
            pane_id,
        };
        self.actor.post(Command::LoadOlder { pane, reply })?;
        answer.await.unwrap_or(Err(MobileError::Stopped))
    }

    /// The slash commands and skills of a pane's agent, for the composer's `/` dropdown ([`filter_commands`]): read
    /// once per conversation and kept until the pane's conversation changes or its chat reads a `reset`.
    ///
    /// [`filter_commands`]: crate::chat::filter_commands
    ///
    /// # Errors
    ///
    /// [`MobileError::NotConnected`], [`MobileError::Remote`] when the session's layout does not hold the pane or the
    /// origin gave no catalog (it runs no agent there, or predates the command), or [`MobileError::Stopped`].
    pub async fn agent_commands(
        &self,
        machine_id: String,
        session_id: String,
        pane_id: String,
    ) -> Result<Vec<AgentCommand>, MobileError> {
        let (reply, answer) = oneshot::channel();
        let pane = PaneRef {
            machine_id,
            session_id,
            pane_id,
        };
        self.actor.post(Command::AgentCommands { pane, reply })?;
        answer.await.unwrap_or(Err(MobileError::Stopped))
    }

    /// Sends a file the chat composer attaches to the machine, which stores it for its agents to read
    /// (`ngtermctl agent upload`: a fresh directory under its state directory's `uploads/`, kept a week), and returns
    /// the stored path to put into the prompt. `progress` hears how much has gone. Cancelling the call (dropping it, a
    /// cancelled coroutine) while bytes are still to go stops the upload, and the machine keeps nothing of it; once
    /// `progress` has heard every byte sent the machine has the whole file and stores it, so a cancel then is too late
    /// (the stored copy goes with the week's prune) and the app no longer offers one.
    ///
    /// # Errors
    ///
    /// [`MobileError::NotConnected`], [`MobileError::Remote`] with the machine's words when it stored nothing (the
    /// file is over its 32 MiB cap, an origin that predates the command) or the transport's when the link failed or
    /// stalled, [`MobileError::Cancelled`], or [`MobileError::Stopped`].
    pub async fn upload_file(
        &self,
        machine_id: String,
        name: String,
        bytes: Vec<u8>,
        progress: Arc<dyn UploadProgress>,
    ) -> Result<String, MobileError> {
        self.upload(machine_id, name, bytes, progress, None).await
    }

    /// [`MobileCore::upload_file`] that `cancel` stops as well as a dropped call: for the iOS app, whose async calls
    /// can't drop the core's future, so a cancelled Swift task alone would send the whole file.
    ///
    /// # Errors
    ///
    /// As [`MobileCore::upload_file`]; [`MobileError::Cancelled`] once `cancel` was cancelled while bytes were still
    /// to go.
    pub async fn upload_file_cancellable(
        &self,
        machine_id: String,
        name: String,
        bytes: Vec<u8>,
        progress: Arc<dyn UploadProgress>,
        cancel: Arc<UploadCancel>,
    ) -> Result<String, MobileError> {
        self.upload(machine_id, name, bytes, progress, Some(cancel)).await
    }
}

impl MobileCore {
    /// Posts an upload and waits for its answer: the call [`MobileCore::upload_file`] and
    /// [`MobileCore::upload_file_cancellable`] share.
    async fn upload(
        &self,
        machine_id: String,
        name: String,
        bytes: Vec<u8>,
        progress: Arc<dyn UploadProgress>,
        cancel: Option<Arc<UploadCancel>>,
    ) -> Result<String, MobileError> {
        let (reply, answer) = oneshot::channel();
        self.actor.post(Command::Upload {
            machine_id,
            name,
            bytes,
            progress,
            cancel,
            reply,
        })?;
        answer.await.unwrap_or(Err(MobileError::Stopped))
    }
}

/// The core's version (the workspace's), which the app shows and its smoke test reads to prove the library loaded.
#[uniffi::export]
#[must_use]
pub fn core_version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

/// A new ed25519 key for the app, its public half commented `comment` (shown in the machine's `authorized_keys`).
///
/// # Errors
///
/// [`KeyError::Invalid`] when the key can't be generated.
#[uniffi::export]
#[expect(
    clippy::needless_pass_by_value,
    reason = "UniFFI passes the foreign side's arguments owned"
)]
pub fn generate_key(comment: String) -> Result<KeyPair, KeyError> {
    keys::generate(&comment)
}

/// A key the user imports (OpenSSH, PKCS#1 or PKCS#8 PEM), opened with `passphrase` when it is encrypted, as the
/// unencrypted OpenSSH key the app stores. The bytes and the passphrase are wiped once read.
///
/// # Errors
///
/// [`KeyError::PassphraseRequired`], [`KeyError::WrongPassphrase`] or [`KeyError::Invalid`].
#[uniffi::export]
pub fn import_key(private_key: Vec<u8>, passphrase: Option<String>) -> Result<KeyPair, KeyError> {
    let private_key = zeroize::Zeroizing::new(private_key);
    let passphrase = passphrase.map(zeroize::Zeroizing::new);
    keys::import(&private_key, passphrase.as_deref().map(String::as_str))
}

#[cfg(test)]
mod tests {
    //! The facade over the scripted fake transport, its actor on its own thread as in the app: each call reaches the
    //! actor and comes back with its answer or its events. What each command does is the actor's tests'; these hold the
    //! calls the apps make to it.

    use std::sync::mpsc::{self, Receiver, Sender};
    use std::sync::{Arc, Mutex, PoisonError};
    use std::thread;
    use std::time::{Duration, Instant};

    use futures_lite::future::{block_on, poll_once};
    use ngterm_core::model::{AskAlignment, AskStyle};
    use ngterm_core::remote::frames::{
        self, Body, PresentationAsk, PresentationAskAnswer, PresentationFrame, PresentationHello, PresentationMode,
        PresentationSnapshot,
    };
    use ngterm_core::remote::ssh::{present_tail, tree_tail};

    use super::*;
    use crate::chat::NOT_OPEN;
    use crate::chat::upload::upload_tail;
    use crate::transport::fake::{FakeTransport, Opened, Script};
    use crate::transport::{ChannelEnd, ChannelInput, ChannelPeer};

    const WAIT: Duration = Duration::from_secs(10);
    const SESSION: &str = "00000000-0000-0000-0000-0000000000A1";
    const PANE: &str = "00000000-0000-0000-0000-0000000000B1";

    struct Recorder(Mutex<Sender<MobileEvent>>);

    impl EventSink for Recorder {
        fn on_event(&self, event: MobileEvent) {
            let _ = self.0.lock().unwrap_or_else(PoisonError::into_inner).send(event);
        }
    }

    /// The progress an upload told, in order.
    #[derive(Default)]
    struct Told(Mutex<Vec<(u64, u64)>>);

    impl UploadProgress for Told {
        fn sent(&self, sent: u64, total: u64) {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((sent, total));
        }
    }

    impl Told {
        fn last(&self) -> Option<(u64, u64)> {
            self.0.lock().unwrap_or_else(PoisonError::into_inner).last().copied()
        }
    }

    fn machine(key_id: &str) -> Machine {
        Machine {
            id: "m1".to_owned(),
            label: "box".to_owned(),
            host: "box".to_owned(),
            port: 22,
            user: "nick".to_owned(),
            key_id: key_id.to_owned(),
        }
    }

    /// A core over a fake transport, the machine `m1` connected, the events after the connect's.
    fn connected() -> (Arc<MobileCore>, FakeTransport, Receiver<MobileEvent>) {
        let transport = FakeTransport::new();
        let (events, received) = mpsc::channel();
        let sink: Arc<dyn EventSink> = Arc::new(Recorder(Mutex::new(events)));
        let core = MobileCore::with_transport(transport.clone(), sink).unwrap();
        block_on(core.connect(machine("key"))).unwrap();
        received.try_iter().for_each(drop);
        (core, transport, received)
    }

    /// The next event `matches` takes, skipping the others.
    fn next<T>(events: &Receiver<MobileEvent>, matches: impl Fn(MobileEvent) -> Option<T>) -> T {
        let deadline = Instant::now() + WAIT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let event = events.recv_timeout(left).expect("the event arrives");
            if let Some(found) = matches(event) {
                return found;
            }
        }
    }

    fn state(events: &Receiver<MobileEvent>) -> MachineState {
        next(events, |event| match event {
            MobileEvent::MachineState { state, .. } => Some(state),
            _ => None,
        })
    }

    /// The next unscripted channel the core opened.
    fn opened(transport: &FakeTransport) -> Opened {
        let deadline = Instant::now() + WAIT;
        loop {
            if let Some(opened) = transport.take_opened().pop() {
                return opened;
            }
            assert!(Instant::now() < deadline, "no channel opened");
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// What the core sent down `peer` until `done` is among it: the bytes, and whether it closed the channel.
    fn inputs_until(peer: &mut ChannelPeer, done: &ChannelInput) -> (Vec<u8>, Vec<ChannelInput>) {
        let deadline = Instant::now() + WAIT;
        let (mut bytes, mut others) = (Vec::new(), Vec::new());
        while !others.contains(done) {
            match peer.try_input() {
                Some(ChannelInput::Data(data)) => bytes.extend(data),
                Some(other) => others.push(other),
                None => {
                    assert!(Instant::now() < deadline, "waited for {done:?}; got {others:?}");
                    thread::sleep(Duration::from_millis(5));
                }
            }
        }
        (bytes, others)
    }

    #[test]
    fn core_version_is_the_crate_version() {
        assert_eq!(super::core_version(), env!("CARGO_PKG_VERSION"));
        assert!(!super::core_version().is_empty());
    }

    #[test]
    fn a_generated_key_imports_back_as_itself_and_a_non_key_is_invalid() {
        let pair = generate_key("phone".to_owned()).unwrap();
        assert!(pair.authorized_key.ends_with(" phone"), "{}", pair.authorized_key);
        assert_eq!(import_key(pair.private_key.clone(), None), Ok(pair));
        assert!(matches!(
            import_key(b"not a key".to_vec(), Some("secret".to_owned())),
            Err(KeyError::Invalid { .. })
        ));
    }

    #[test]
    fn a_failed_vault_callback_is_an_unavailable_key_with_its_reason() {
        let failed = uniffi::UnexpectedUniFFICallbackError::new("the keystore is locked");
        let error = VaultError::from(failed);
        assert_eq!(
            error,
            VaultError::Unavailable {
                reason: "the keystore is locked".to_owned()
            }
        );
        assert_eq!(error.to_string(), "the keystore is locked");
    }

    #[test]
    fn a_key_switch_keeps_the_connection_and_makes_a_connect_with_that_key_no_edit() {
        let (core, transport, _events) = connected();
        assert_eq!(core.use_key("m1".to_owned(), "new".to_owned()), Ok(()));
        assert_eq!(block_on(core.connect(machine("new"))), Ok(()));
        assert!(transport.closed().is_empty(), "the connection stays");
        assert_eq!(transport.connects(), ["m1"]);
    }

    #[test]
    fn suspend_closes_the_connections_quietly_and_resume_reconnects_them() {
        let (core, transport, events) = connected();
        assert_eq!(core.suspend(), Ok(()));
        // commands run in order: the listing's answer means the suspend ran
        assert_eq!(
            block_on(core.list_sessions("m1".to_owned())),
            Err(MobileError::NotConnected {
                machine_id: "m1".to_owned()
            })
        );
        assert_eq!(transport.closed(), ["m1"]);
        assert!(events.try_recv().is_err(), "no state reported");

        assert_eq!(core.resume(), Ok(()));
        assert_eq!(state(&events), MachineState::Reconnecting { next_in_ms: 0 });
        assert_eq!(state(&events), MachineState::Connected);
        assert_eq!(transport.connects(), ["m1", "m1"]);
    }

    /// The session attached on `core`, its stream's far end.
    fn attached(core: &MobileCore, transport: &FakeTransport) -> ChannelPeer {
        let remote = serde_json::json!({
            "endpoint": {"executable": "/ngterm-session", "socketDirectory": "/tmp/ngterm-live-x"},
            "sessions": [{"id": SESSION, "name": "main", "windowID": "w1", "windowName": "main",
                          "workspaceID": "ws1", "workspaceName": "work", "cwd": "/repo", "panes": []}],
            "presentation": 1,
        });
        let listing = serde_json::json!({"ok": true, "result": {"remote": remote}});
        transport.script(
            &tree_tail(ngterm_core::remote::origin::OriginShell::Posix),
            Script::ok(&listing.to_string()),
        );
        block_on(core.list_sessions("m1".to_owned())).unwrap();
        block_on(core.attach_session("m1".to_owned(), SESSION.to_owned())).unwrap();
        let Opened { command, peer, .. } = opened(transport);
        assert_eq!(
            command,
            present_tail(ngterm_core::remote::origin::OriginShell::Posix, SESSION).unwrap()
        );
        peer
    }

    /// The origin's hello (this app presenting) and snapshot, then `ask`.
    fn present_ask(peer: &ChannelPeer, ask: PresentationAsk) {
        let hello = Body::Hello(PresentationHello {
            version: frames::VERSION,
            kinds: Vec::new(),
            mode: PresentationMode::Presenter,
        });
        let bodies = [
            hello,
            Body::Snapshot(PresentationSnapshot::default()),
            Body::AskRequest(ask),
        ];
        for (rev, body) in (1..).zip(bodies) {
            let line = frames::encode(&PresentationFrame {
                generation: 1,
                rev,
                body,
            })
            .unwrap();
            assert!(block_on(peer.data(line)));
        }
    }

    /// The first ask answer the core wrote down `peer`.
    fn answer_written(peer: &mut ChannelPeer) -> PresentationAskAnswer {
        let deadline = Instant::now() + WAIT;
        loop {
            match peer.try_input() {
                Some(ChannelInput::Data(line)) => {
                    let body = frames::decode(line.strip_suffix(b"\n").unwrap()).unwrap().body;
                    if let Body::AskResolve(answer) = body {
                        return answer;
                    }
                }
                Some(_) => {}
                None => {
                    assert!(Instant::now() < deadline, "no answer written");
                    thread::sleep(Duration::from_millis(5));
                }
            }
        }
    }

    #[test]
    fn an_asks_answer_goes_back_down_its_sessions_stream_with_the_asks_owner() {
        let (core, transport, events) = connected();
        let mut peer = attached(&core, &transport);
        let ask = PresentationAsk {
            id: "a1".to_owned(),
            title: "Deploy?".to_owned(),
            message: None,
            buttons: vec![ngterm_proto::request::AskButton {
                id: "ok".to_owned(),
                label: "OK".to_owned(),
                hotkey: None,
            }],
            default_id: None,
            destructive_id: None,
            style: AskStyle::Gui,
            align: AskAlignment::Center,
            width: None,
            pane: None,
            owner: 42,
            input: None,
            choices: None,
            cancel_id: None,
        };
        present_ask(&peer, ask);
        let asked = next(&events, |event| match event {
            MobileEvent::Ask { ask, .. } => Some(ask.id),
            _ => None,
        });
        assert_eq!(asked, "a1");

        let reply = AskReply {
            text: Some("prod".to_owned()),
            ..AskReply::default()
        };
        assert_eq!(
            core.answer_ask("m1".to_owned(), SESSION.to_owned(), "a1".to_owned(), reply),
            Ok(())
        );
        assert_eq!(
            answer_written(&mut peer),
            PresentationAskAnswer {
                id: "a1".to_owned(),
                owner: 42,
                button: None,
                text: Some("prod".to_owned()),
                selected: None,
            }
        );
    }

    #[test]
    fn an_older_page_of_a_chat_not_open_is_refused() {
        let (core, _transport, _events) = connected();
        let older = block_on(core.load_older("m1".to_owned(), SESSION.to_owned(), PANE.to_owned()));
        assert_eq!(
            older,
            Err(MobileError::Remote {
                reason: NOT_OPEN.to_owned()
            })
        );
    }

    #[test]
    fn an_upload_sends_the_file_and_answers_the_stored_path() {
        let (core, transport, _events) = connected();
        let told = Arc::new(Told::default());
        let progress: Arc<dyn UploadProgress> = Arc::clone(&told) as Arc<dyn UploadProgress>;
        let uploading = {
            let core = Arc::clone(&core);
            thread::spawn(move || {
                block_on(core.upload_file("m1".to_owned(), "notes.txt".to_owned(), b"hello".to_vec(), progress))
            })
        };
        let mut origin = opened(&transport);
        assert_eq!(
            origin.command,
            upload_tail(ngterm_core::remote::origin::OriginShell::Posix, "notes.txt", 5)
        );
        let (bytes, _) = inputs_until(&mut origin.peer, &ChannelInput::Eof);
        assert_eq!(bytes, b"hello");
        assert!(block_on(origin.peer.data(b"/state/uploads/1/notes.txt\n".to_vec())));
        block_on(origin.peer.end(ChannelEnd::Exit {
            code: Some(0),
            signal: None,
        }));
        assert_eq!(uploading.join().unwrap(), Ok("/state/uploads/1/notes.txt".to_owned()));
        assert_eq!(told.last(), Some((5, 5)));
    }

    #[test]
    fn an_upload_to_a_machine_not_connected_is_refused() {
        let (core, transport, _events) = connected();
        let progress: Arc<dyn UploadProgress> = Arc::new(Told::default());
        let cancel = UploadCancel::new();
        let answer = block_on(core.upload_file_cancellable(
            "m9".to_owned(),
            "a.txt".to_owned(),
            b"abc".to_vec(),
            progress,
            cancel,
        ));
        assert_eq!(
            answer,
            Err(MobileError::NotConnected {
                machine_id: "m9".to_owned()
            })
        );
        assert!(transport.take_opened().is_empty());
    }

    #[test]
    fn cancelling_an_uploads_token_closes_its_channel_and_answers_cancelled() {
        let (core, transport, _events) = connected();
        let cancel = UploadCancel::new();
        let uploading = {
            let (core, cancel) = (Arc::clone(&core), Arc::clone(&cancel));
            let progress: Arc<dyn UploadProgress> = Arc::new(Told::default());
            thread::spawn(move || {
                block_on(core.upload_file_cancellable(
                    "m1".to_owned(),
                    "big.bin".to_owned(),
                    vec![1; 1024],
                    progress,
                    cancel,
                ))
            })
        };
        let mut origin = opened(&transport);
        cancel.cancel();
        assert_eq!(uploading.join().unwrap(), Err(MobileError::Cancelled));
        inputs_until(&mut origin.peer, &ChannelInput::Close);
    }

    #[test]
    fn dropping_an_upload_call_closes_its_channel() {
        let (core, transport, _events) = connected();
        let progress: Arc<dyn UploadProgress> = Arc::new(Told::default());
        let mut call = Box::pin(core.upload_file("m1".to_owned(), "big.bin".to_owned(), vec![1; 1024], progress));
        assert_eq!(block_on(poll_once(&mut call)), None, "the upload waits for the machine");
        let mut origin = opened(&transport);
        drop(call);
        inputs_until(&mut origin.peer, &ChannelInput::Close);
    }
}
