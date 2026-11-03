//! The `UniFFI` facade the Kotlin app calls: [`MobileCore`] and its handles (a shown pane's [`Pane`]), the records and
//! events they carry, and the callback interfaces the app implements ([`EventSink`], [`KeyVault`], [`HostKeyStore`]).
//!
//! Everything here is thin: a [`MobileCore`] call posts a command to the core's actor (`crate::actor`) and, when it
//! has an answer, awaits it (a Kotlin `suspend fun`); what the actor learns on its own reaches the app as
//! [`MobileEvent`]s through the [`EventSink`], on the actor's thread. `UniFFI` passes owned arguments, so a signature
//! clippy would take by reference keeps its owned form under an item-level `#[expect]`.

mod pane;

use std::sync::Arc;

use tokio::sync::oneshot;

pub(crate) use self::pane::PaneSlot;
pub use self::pane::{
    CellPoint, Pane, PaneColors, PaneInputError, PaneKey, PaneKeyAction, PaneMouse, PaneMouseAction, PaneMouseButton,
    PaneSendOutcome, PaneSize, PaneWheel, PaneWheelOutcome,
};

use crate::actor::{Actor, Command};
use crate::keys::{self, KeyError, KeyPair};
use crate::pane::{ClipboardReply, PaneClipboard, PaneEnd, PaneRef, PaneRole};
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
    /// A shown pane's program set the clipboard (`None` clears it).
    ClipboardWrite {
        pane: PaneRef,
        clipboard: PaneClipboard,
        text: Option<String>,
    },
    /// A shown pane's program asks to read the clipboard (`list_only`: only which kinds it holds): its output waits
    /// for [`MobileCore::answer_clipboard_read`].
    ClipboardReadRequest {
        pane: PaneRef,
        read_id: u64,
        clipboard: PaneClipboard,
        list_only: bool,
    },
    /// A clipboard read was denied without the user's answer (the pane hidden, its link lost, its program ended):
    /// take its prompt down.
    ClipboardReadDismiss { pane: PaneRef, read_id: u64 },
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
    /// The key exists but could not be unwrapped (the Keystore refused, the app's callback failed).
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

/// The app's private keys, wrapped by the Android Keystore: [`KeyVault::load`] unwraps one for a connection, which
/// wipes the bytes once it has used them.
#[uniffi::export(with_foreign)]
pub trait KeyVault: Send + Sync {
    /// The OpenSSH private key bytes of `key_id`.
    ///
    /// # Errors
    ///
    /// [`VaultError::Missing`] when no key has the id, [`VaultError::Unavailable`] when it can't be unwrapped.
    fn load(&self, key_id: String) -> Result<Vec<u8>, VaultError>;
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
        Ok(Arc::new(Self {
            actor: Actor::spawn(transport, sink)?,
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
    /// reported, and the attached sessions' streams stop quietly. [`MobileCore::resume`] brings them back.
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
    #[test]
    fn core_version_is_the_crate_version() {
        assert_eq!(super::core_version(), env!("CARGO_PKG_VERSION"));
        assert!(!super::core_version().is_empty());
    }
}
