//! The `UniFFI` facade the Kotlin app calls: [`MobileCore`] and its handles, the records and events they carry, and the
//! callback interfaces the app implements ([`EventSink`], [`KeyVault`], [`HostKeyStore`]).
//!
//! Everything here is thin: a [`MobileCore`] call posts a command to the core's actor (`crate::actor`) and, when it
//! has an answer, awaits it (a Kotlin `suspend fun`); what the actor learns on its own reaches the app as
//! [`MobileEvent`]s through the [`EventSink`], on the actor's thread. `UniFFI` passes owned arguments, so a signature
//! clippy would take by reference keeps its owned form under an item-level `#[expect]`.

use std::sync::Arc;

use tokio::sync::oneshot;

use crate::actor::{Actor, Command};
use crate::keys::{self, KeyError, KeyPair};
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
    /// The host key pinned on first use (OpenSSH's `SHA256:` fingerprint), if any: the app's copy, for showing. The
    /// core trusts only [`HostKeyStore::pinned`], so a pin removed from the store stays removed.
    pub pinned_host_key: Option<String>,
}

/// A machine's connection.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum MachineState {
    Connected,
    /// Down, retried in `next_in_ms`.
    Reconnecting {
        next_in_ms: u64,
    },
    /// Down for a reason a retry won't fix by itself (refused, authentication, a changed host key).
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
    /// [`MobileCore::disconnect`]. A connect of the machine as edited (another host, user, key or pin) closes its
    /// connection and connects anew. The state also arrives as a
    /// [`MobileEvent::MachineState`].
    ///
    /// # Errors
    ///
    /// [`MobileError::Unreachable`] when the machine can't be reached, [`MobileError::Cancelled`] when a disconnect
    /// or a connect of the machine as edited (another host, user, key or pin) overtook the connect, [`MobileError::Stopped`] when the core has stopped.
    pub async fn connect(&self, machine: Machine) -> Result<(), MobileError> {
        let (reply, answer) = oneshot::channel();
        self.actor.post(Command::Connect { machine, reply })?;
        answer.await.unwrap_or(Err(MobileError::Stopped))
    }

    /// Closes the connection to `machine_id`, if any, cancels a connect or reconnect in flight, and ends its attached
    /// sessions' streams.
    ///
    /// # Errors
    ///
    /// [`MobileError::Stopped`] when the core has stopped.
    pub fn disconnect(&self, machine_id: String) -> Result<(), MobileError> {
        self.actor.post(Command::Disconnect { machine_id })
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

    /// Detaches `session_id` of `machine_id`: its stream ends, with no further events about it. The session goes on
    /// running there.
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

    /// Brings back what [`MobileCore::suspend`] closed: each machine reconnects at once (and keeps retrying, reported
    /// as [`MachineState::Reconnecting`]), and its attached sessions' streams start again once it is connected.
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

/// A shown pane of an attached session: a handle onto the terminal the actor feeds from its ssh channel.
#[derive(Debug, uniffi::Object)]
pub struct Pane {
    id: u64,
}

#[uniffi::export]
impl Pane {
    /// The pane's id within its session, as the origin numbers it.
    #[must_use]
    pub fn id(&self) -> u64 {
        self.id
    }
}
