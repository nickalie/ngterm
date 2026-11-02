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
use crate::transport::Transport;

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
    /// The host key pinned on first use (OpenSSH's `SHA256:` fingerprint), if any.
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
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum MobileEvent {
    /// A machine's connection changed.
    MachineState { machine_id: String, state: MachineState },
    /// A machine with no pinned key offered `fingerprint`: pinning it through [`HostKeyStore`] lets the connection
    /// proceed.
    HostKeyUnknown { machine_id: String, fingerprint: String },
    /// A machine offered a key other than the pinned one; the connection stopped.
    HostKeyChanged {
        machine_id: String,
        pinned: String,
        offered: String,
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
    /// flight is joined, not repeated). The state also arrives as a
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

    /// Closes the connection to `machine_id`, if any, and cancels a connect in flight.
    ///
    /// # Errors
    ///
    /// [`MobileError::Stopped`] when the core has stopped.
    pub fn disconnect(&self, machine_id: String) -> Result<(), MobileError> {
        self.actor.post(Command::Disconnect { machine_id })
    }
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
