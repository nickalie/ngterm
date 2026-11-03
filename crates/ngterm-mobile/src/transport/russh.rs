//! The app's [`Transport`]: ssh through russh, one connection per machine, authenticated with the machine's key from
//! the [`KeyVault`] and checked against the host key pinned in the [`HostKeyStore`] (trust on first use).
//!
//! A connect resolves and reaches the machine within [`Timing::connect`], with keepalives once it is up. The host key
//! decides whether the handshake goes on:
//!
//! - pinned and offered: it goes on;
//! - pinned and another offered: [`MobileEvent::HostKeyChanged`], and the connect fails (removing the pin is the
//!   app's deliberate act, never this module's);
//! - none pinned: [`MobileEvent::HostKeyUnknown`], and the connect waits up to [`Timing::confirm`] for the app's
//!   [`Transport::answer_host_key`]. Trusting the offered fingerprint pins it through [`HostKeyStore::pin`] and the
//!   handshake goes on; a refusal, another fingerprint, the wait running out, or the connect being dropped (a
//!   disconnect, an edit) ends it. An answer is a statement about a key for a machine, not about one attempt: it
//!   settles whichever connect of that machine is waiting on that same key.
//!
//! [`HostKeyStore::pinned`] is the only authority on the pin: [`Machine::pinned_host_key`] is the app's copy for
//! showing, so removing a pin from the store is never undone by a stale record.
//!
//! Each exec channel is bridged to the core's [`Channel`] by two tasks: one copies the machine's output into the
//! [`ChannelPeer`] (waiting while the reader is behind, which holds back ssh's window), one copies the core's input to
//! ssh. A channel that closes after an exit status ends as [`ChannelEnd::Exit`]; one that closes without it ends as
//! [`ChannelEnd::Lost`] ([`PEER_GONE`]), one the core closed as [`ChannelEnd::Lost`] ([`CHANNEL_CLOSED`]: russh drops
//! the machine's answering close), one whose connection was closed as [`ChannelEnd::Lost`] with
//! [`TransportError::Closed`]'s text, and one whose connection went away as [`ChannelEnd::Lost`] ([`CONNECTION_LOST`]).
//! A channel whose reader and writers are all gone is closed.

#[cfg(test)]
mod tests;

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use ::russh::client::{self, AuthResult, Handle, Msg};
use ::russh::keys::{HashAlg, PrivateKeyWithHashAlg};
use ::russh::{ChannelMsg, Disconnect, Sig};
use ngterm_term::GridSize;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use zeroize::Zeroizing;

use super::{
    Channel, ChannelEnd, ChannelInput, ChannelOutput, ChannelPeer, Connection, PEER_GONE, Transport, TransportError,
};
use crate::api::{EventSink, HostKeyStore, KeyVault, Machine, MobileEvent};
use crate::keys;

/// Why a channel ended when its connection went away under it.
pub const CONNECTION_LOST: &str = "the connection was lost";
/// Why a channel ended when the core closed it.
pub const CHANNEL_CLOSED: &str = "the channel was closed";
/// The `TERM` a pane's pty is requested with: one every server's terminfo knows.
pub const PTY_TERM: &str = "xterm-256color";

/// How long a connect and a connection wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    /// Reaching the machine: resolving, connecting, the handshake and authentication, not counting the wait for the
    /// user's answer about an unknown host key.
    pub connect: Duration,
    /// How long a connect waits for the answer about an unknown host key.
    pub confirm: Duration,
    /// Quiet time after which a keepalive is sent; [`KEEPALIVE_MAX`] unanswered ones close the connection.
    pub keepalive: Duration,
}

/// Unanswered keepalives after which a connection is given up.
pub const KEEPALIVE_MAX: usize = 3;

impl Default for Timing {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(15),
            confirm: Duration::from_secs(120),
            keepalive: Duration::from_secs(15),
        }
    }
}

/// ssh through russh, with the app's keys and pins.
pub struct RusshTransport {
    vault: Arc<dyn KeyVault>,
    hosts: Arc<dyn HostKeyStore>,
    sink: Arc<dyn EventSink>,
    challenges: Arc<Challenges>,
    timing: Timing,
}

impl RusshTransport {
    /// A transport loading keys from `vault`, pins from `hosts`, and telling `sink` about host keys.
    #[must_use]
    pub fn new(vault: Arc<dyn KeyVault>, hosts: Arc<dyn HostKeyStore>, sink: Arc<dyn EventSink>) -> Self {
        Self::with_timing(vault, hosts, sink, Timing::default())
    }

    /// [`RusshTransport::new`] with other waits.
    #[must_use]
    pub fn with_timing(
        vault: Arc<dyn KeyVault>,
        hosts: Arc<dyn HostKeyStore>,
        sink: Arc<dyn EventSink>,
        timing: Timing,
    ) -> Self {
        Self {
            vault,
            hosts,
            sink,
            challenges: Arc::new(Challenges::default()),
            timing,
        }
    }

    fn config(&self) -> Arc<client::Config> {
        Arc::new(client::Config {
            keepalive_interval: Some(self.timing.keepalive),
            keepalive_max: KEEPALIVE_MAX,
            nodelay: true,
            ..client::Config::default()
        })
    }

    async fn reach(
        &self,
        machine: &Machine,
        key: ::russh::keys::PrivateKey,
        token: u64,
    ) -> Result<RusshConnection, TransportError> {
        let pinned = self.hosts.pinned(machine.id.clone());
        // An unknown host may wait for the user on top of the connect's own time.
        let budget = if pinned.is_some() {
            self.timing.connect
        } else {
            self.timing.connect + self.timing.confirm
        };
        let refusal = Arc::new(Mutex::new(None));
        let client = Client {
            machine_id: machine.id.clone(),
            token,
            pinned,
            hosts: Arc::clone(&self.hosts),
            sink: Arc::clone(&self.sink),
            challenges: Arc::clone(&self.challenges),
            confirm: self.timing.confirm,
            refusal: Arc::clone(&refusal),
        };
        let mut handle = tokio::time::timeout(budget, async {
            let socket = tokio::time::timeout(
                self.timing.connect,
                TcpStream::connect((machine.host.as_str(), machine.port)),
            )
            .await
            .map_err(|_| timed_out(machine))?
            .map_err(|error| unreachable(machine, &error))?;
            // Keystrokes go out at once rather than waiting to fill a segment.
            let _ = socket.set_nodelay(true);
            client::connect_stream(self.config(), socket, client)
                .await
                .map_err(|error| refused(machine, &refusal, &error))
        })
        .await
        .map_err(|_| timed_out(machine))??;
        tokio::time::timeout(self.timing.connect, authenticate(&mut handle, machine, key))
            .await
            .map_err(|_| timed_out(machine))??;
        Ok(RusshConnection {
            handle: RefCell::new(Some(Rc::new(handle))),
            closed: Arc::new(AtomicBool::new(false)),
        })
    }
}

impl Transport for RusshTransport {
    type Connection = RusshConnection;

    async fn connect(&self, machine: &Machine) -> Result<RusshConnection, TransportError> {
        let bytes = self
            .vault
            .load(machine.key_id.clone())
            .map_err(|error| TransportError::Unreachable(error.to_string()))?;
        let bytes = Zeroizing::new(bytes);
        let key = keys::load(&bytes).map_err(|error| TransportError::Unreachable(error.to_string()))?;
        drop(bytes);
        // Dropped with the connect (a disconnect, an edit), it withdraws the question this connect asked, if any.
        let withdraw = Withdraw {
            challenges: Arc::clone(&self.challenges),
            machine_id: machine.id.clone(),
            token: self.challenges.token(),
        };
        self.reach(machine, key, withdraw.token).await
    }

    fn answer_host_key(&self, machine_id: &str, fingerprint: &str, trust: bool) {
        self.challenges.answer(machine_id, fingerprint, trust);
    }
}

/// The questions about unknown host keys waiting for the app's answer, one per machine.
#[derive(Default)]
struct Challenges {
    pending: Mutex<HashMap<String, Challenge>>,
    next: AtomicU64,
}

struct Challenge {
    /// Which ask this is: a withdrawal removes only its own.
    token: u64,
    fingerprint: String,
    decide: oneshot::Sender<bool>,
}

impl Challenges {
    fn lock(&self) -> MutexGuard<'_, HashMap<String, Challenge>> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A new connect's token: its question is asked and withdrawn under it.
    fn token(&self) -> u64 {
        self.next.fetch_add(1, Ordering::Relaxed)
    }

    /// Asks about `fingerprint` for `machine_id` under `token`; the answer arrives on the receiver.
    fn ask(&self, machine_id: &str, token: u64, fingerprint: &str) -> oneshot::Receiver<bool> {
        let (decide, decision) = oneshot::channel();
        let challenge = Challenge {
            token,
            fingerprint: fingerprint.to_owned(),
            decide,
        };
        // A question still open for the machine belongs to a connect that was overtaken: dropping it refuses it.
        self.lock().insert(machine_id.to_owned(), challenge);
        decision
    }

    fn answer(&self, machine_id: &str, fingerprint: &str, trust: bool) {
        let mut pending = self.lock();
        // An answer about another key (a sheet left open while the machine changed keys) refuses: the user never saw
        // the key on offer.
        let trust = trust
            && pending
                .get(machine_id)
                .is_some_and(|challenge| challenge.fingerprint == fingerprint);
        if let Some(challenge) = pending.remove(machine_id) {
            let _ = challenge.decide.send(trust);
        }
    }

    /// Removes `machine_id`'s question if it is still `token`'s, never a later connect's.
    fn withdraw(&self, machine_id: &str, token: u64) {
        let mut pending = self.lock();
        let ours = pending
            .get(machine_id)
            .is_some_and(|challenge| challenge.token == token);
        if ours {
            pending.remove(machine_id);
        }
    }
}

/// Withdraws the question of the connect it guards when that connect ends or is dropped.
struct Withdraw {
    challenges: Arc<Challenges>,
    machine_id: String,
    token: u64,
}

impl Drop for Withdraw {
    fn drop(&mut self) {
        self.challenges.withdraw(&self.machine_id, self.token);
    }
}

/// russh's side of one connection: it checks the host key.
struct Client {
    machine_id: String,
    /// The connect's token, which its question is asked under.
    token: u64,
    pinned: Option<String>,
    hosts: Arc<dyn HostKeyStore>,
    sink: Arc<dyn EventSink>,
    challenges: Arc<Challenges>,
    confirm: Duration,
    /// Why the host key was refused, for the connect's error.
    refusal: Arc<Mutex<Option<String>>>,
}

impl Client {
    fn refuse(&self, reason: &str) -> bool {
        *self.refusal.lock().unwrap_or_else(PoisonError::into_inner) = Some(reason.to_owned());
        false
    }

    async fn ask(&self, offered: String) -> bool {
        let decision = self.challenges.ask(&self.machine_id, self.token, &offered);
        self.sink.on_event(MobileEvent::HostKeyUnknown {
            machine_id: self.machine_id.clone(),
            fingerprint: offered.clone(),
        });
        let answer = tokio::time::timeout(self.confirm, decision).await;
        self.challenges.withdraw(&self.machine_id, self.token);
        match answer {
            Ok(Ok(true)) => {
                self.hosts.pin(self.machine_id.clone(), offered);
                true
            }
            Ok(Ok(false)) => self.refuse(HOST_KEY_REFUSED),
            Ok(Err(_)) => self.refuse(HOST_KEY_WITHDRAWN),
            Err(_) => self.refuse(HOST_KEY_UNCONFIRMED),
        }
    }
}

/// Why a connect failed when its unknown host key was refused.
pub const HOST_KEY_REFUSED: &str = "the host key was not trusted";
/// Why a connect failed when the answer about its host key never came.
pub const HOST_KEY_UNCONFIRMED: &str = "the host key was not confirmed in time";
/// Why a connect failed when its question about the host key was overtaken by another connect.
pub const HOST_KEY_WITHDRAWN: &str = "the host key question was withdrawn";
/// Why a connect failed when the machine offered another key than the pinned one.
pub const HOST_KEY_CHANGED: &str = "the host key changed";

impl client::Handler for Client {
    type Error = ::russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &::russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let offered = fingerprint(server_public_key);
        let trusted = match self.pinned.clone() {
            Some(pinned) if pinned == offered => true,
            Some(pinned) => {
                self.sink.on_event(MobileEvent::HostKeyChanged {
                    machine_id: self.machine_id.clone(),
                    pinned,
                    offered,
                });
                self.refuse(HOST_KEY_CHANGED)
            }
            None => self.ask(offered).await,
        };
        Ok(trusted)
    }
}

/// The OpenSSH `SHA256:` fingerprint of a host key, or of a host certificate's key.
fn fingerprint(key: &::russh::keys::PublicKeyOrCertificate) -> String {
    match key {
        ::russh::keys::PublicKeyOrCertificate::PublicKey { key, .. } => key.fingerprint(HashAlg::Sha256).to_string(),
        ::russh::keys::PublicKeyOrCertificate::Certificate(certificate) => {
            certificate.public_key().fingerprint(HashAlg::Sha256).to_string()
        }
    }
}

async fn authenticate(
    handle: &mut Handle<Client>,
    machine: &Machine,
    key: ::russh::keys::PrivateKey,
) -> Result<(), TransportError> {
    let key = Arc::new(key);
    let hash = if key.algorithm().is_rsa() {
        handle
            .best_supported_rsa_hash()
            .await
            .map_err(|error| unreachable(machine, &error))?
            .flatten()
    } else {
        None
    };
    let result = handle
        .authenticate_publickey(machine.user.clone(), PrivateKeyWithHashAlg::new(key, hash))
        .await
        .map_err(|error| unreachable(machine, &error))?;
    match result {
        AuthResult::Success => Ok(()),
        AuthResult::Failure { .. } => Err(TransportError::Unreachable(format!(
            "{}@{} did not accept the key",
            machine.user, machine.host
        ))),
    }
}

fn timed_out(machine: &Machine) -> TransportError {
    TransportError::Unreachable(format!("{}:{} did not answer in time", machine.host, machine.port))
}

fn unreachable(machine: &Machine, error: &dyn std::fmt::Display) -> TransportError {
    TransportError::Unreachable(format!("{}:{}: {error}", machine.host, machine.port))
}

/// A handshake that failed: the host key's refusal when that is why, else russh's error.
fn refused(machine: &Machine, refusal: &Mutex<Option<String>>, error: &::russh::Error) -> TransportError {
    match refusal.lock().unwrap_or_else(PoisonError::into_inner).take() {
        Some(reason) => TransportError::Unreachable(reason),
        None => unreachable(machine, error),
    }
}

/// One machine's ssh connection.
pub struct RusshConnection {
    /// `None` once closed.
    handle: RefCell<Option<Rc<Handle<Client>>>>,
    /// Set by [`Connection::close`], so the channels it ends say so.
    closed: Arc<AtomicBool>,
}

impl Connection for RusshConnection {
    async fn exec(&self, command: &str, pty: Option<GridSize>) -> Result<Channel, TransportError> {
        let handle = self.handle.borrow().clone().ok_or(TransportError::Closed)?;
        let mut channel = handle.channel_open_session().await.map_err(|error| {
            if handle.is_closed() {
                TransportError::Closed
            } else {
                TransportError::Rejected(error.to_string())
            }
        })?;
        let mut early = VecDeque::new();
        if let Some(size) = pty {
            let (width, height) = pixels(size);
            channel
                .request_pty(true, PTY_TERM, size.cols.into(), size.rows.into(), width, height, &[])
                .await
                .map_err(|_| TransportError::Closed)?;
            replied(&mut channel, &mut early, "the machine refused a pty").await?;
        }
        channel.exec(true, command).await.map_err(|_| TransportError::Closed)?;
        replied(&mut channel, &mut early, "the machine refused to run the command").await?;
        let (core, peer) = Channel::pair();
        let ChannelPeer { output, input } = peer;
        let (read, write) = channel.split();
        let (closing, closed) = oneshot::channel();
        let ending = Ending {
            closed,
            connection: Arc::clone(&self.closed),
        };
        tokio::task::spawn_local(copy_output(read, early, output.clone(), ending));
        tokio::task::spawn_local(copy_input(write, input, output, closing));
        Ok(core)
    }

    fn close(&self) {
        let Some(handle) = self.handle.borrow_mut().take() else {
            return;
        };
        self.closed.store(true, Ordering::Relaxed);
        // The session ends once it has said goodbye, and every channel with it.
        tokio::task::spawn_local(async move {
            let _ = handle.disconnect(Disconnect::ByApplication, "", "en").await;
        });
    }
}

/// A pty's size in pixels, as ssh carries it.
fn pixels(size: GridSize) -> (u32, u32) {
    (
        u32::from(size.cols).saturating_mul(size.cell_width),
        u32::from(size.rows).saturating_mul(size.cell_height),
    )
}

/// Waits for the machine's answer to the request just sent, keeping what arrives before it for the reader.
async fn replied(
    channel: &mut ::russh::Channel<Msg>,
    early: &mut VecDeque<ChannelMsg>,
    refusal: &str,
) -> Result<(), TransportError> {
    loop {
        match channel.wait().await {
            Some(ChannelMsg::Success) => return Ok(()),
            Some(ChannelMsg::Failure) => return Err(TransportError::Rejected(refusal.to_owned())),
            Some(ChannelMsg::Close) | None => return Err(TransportError::Closed),
            Some(message) => early.push_back(message),
        }
    }
}

/// What ends a channel besides the machine: the core closing it, or closing its connection.
struct Ending {
    /// Fires when the core closes the channel.
    closed: oneshot::Receiver<()>,
    /// Set when the core closes the connection.
    connection: Arc<AtomicBool>,
}

/// Copies the machine's output to the core until the channel ends or the core stops reading.
async fn copy_output(
    mut read: ::russh::ChannelReadHalf,
    mut early: VecDeque<ChannelMsg>,
    output: mpsc::Sender<ChannelOutput>,
    mut ending: Ending,
) {
    let mut exit = None;
    loop {
        let message = if let Some(message) = early.pop_front() {
            Some(message)
        } else {
            tokio::select! {
                message = read.wait() => message,
                _ = &mut ending.closed => {
                    let _ = output.send(ChannelOutput::End(ChannelEnd::Lost(CHANNEL_CLOSED.to_owned()))).await;
                    return;
                }
            }
        };
        let chunk = match message {
            Some(ChannelMsg::Data { data }) => ChannelOutput::Data(data.to_vec()),
            Some(ChannelMsg::ExtendedData { data, ext: 1 }) => ChannelOutput::Stderr(data.to_vec()),
            Some(ChannelMsg::ExitStatus { exit_status }) => {
                exit = Some(ChannelEnd::Exit {
                    code: Some(exit_status),
                    signal: None,
                });
                continue;
            }
            Some(ChannelMsg::ExitSignal { signal_name, .. }) => {
                exit = Some(ChannelEnd::Exit {
                    code: None,
                    signal: Some(signal(signal_name)),
                });
                continue;
            }
            Some(ChannelMsg::Close) => {
                let end = exit.take().unwrap_or_else(|| ChannelEnd::Lost(PEER_GONE.to_owned()));
                let _ = output.send(ChannelOutput::End(end)).await;
                return;
            }
            None => {
                let reason = if ending.connection.load(Ordering::Relaxed) {
                    TransportError::Closed.to_string()
                } else {
                    CONNECTION_LOST.to_owned()
                };
                let end = exit.take().unwrap_or(ChannelEnd::Lost(reason));
                let _ = output.send(ChannelOutput::End(end)).await;
                return;
            }
            Some(_) => continue,
        };
        if output.send(chunk).await.is_err() {
            // Nobody reads any more: the core closes the channel through its writer when it means to.
            return;
        }
    }
}

/// Copies the core's input to the machine until the channel closes or every writer is gone; a channel left with no
/// reader and no writer is closed.
async fn copy_input(
    write: ::russh::ChannelWriteHalf<Msg>,
    mut input: mpsc::UnboundedReceiver<ChannelInput>,
    output: mpsc::Sender<ChannelOutput>,
    closing: oneshot::Sender<()>,
) {
    while let Some(next) = input.recv().await {
        let sent = match next {
            ChannelInput::Data(bytes) => write.data_bytes(bytes).await,
            ChannelInput::Resize(size) => {
                let (width, height) = pixels(size);
                write
                    .window_change(size.cols.into(), size.rows.into(), width, height)
                    .await
            }
            ChannelInput::Eof => write.eof().await,
            ChannelInput::Close => {
                let _ = write.close().await;
                let _ = closing.send(());
                return;
            }
        };
        if sent.is_err() {
            return;
        }
    }
    if output.is_closed() {
        let _ = write.close().await;
    }
}

/// A signal's name as `StreamFeed::exited` takes it (`TERM`, `KILL`, …).
fn signal(signal: Sig) -> String {
    match signal {
        Sig::Custom(name) => name,
        named => format!("{named:?}"),
    }
}
