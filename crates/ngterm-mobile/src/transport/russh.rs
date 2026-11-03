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
//! [`ChannelPeer`], one copies the core's input to ssh. russh hands every channel's messages over from one session loop
//! and opens ssh's window as data arrives, not as it is read, so the output task never stops taking them: a reader
//! that is behind would hold up the whole connection. It holds what the reader has not taken, up to
//! [`OUTPUT_BUDGET`], and closes a channel whose reader falls further behind ([`ChannelEnd::Lost`],
//! [`OUTPUT_OVERRUN`]).
//!
//! A channel that closes after an exit status ends as [`ChannelEnd::Exit`]; one that closes without it ends as
//! [`ChannelEnd::Lost`] ([`PEER_GONE`]), one the core closed as [`ChannelEnd::Lost`] ([`CHANNEL_CLOSED`]: russh drops
//! the machine's answering close), one whose connection was closed as [`ChannelEnd::Lost`] with
//! [`TransportError::Closed`]'s text, and one whose connection went away as [`ChannelEnd::Lost`] ([`CONNECTION_LOST`]).
//! A channel whose writers are all gone still delivers its output, and is closed once its reader is gone too.

#[cfg(test)]
mod tests;

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use ::russh::client::{self, AuthResult, Handle, Msg};
use ::russh::keys::{HashAlg, PrivateKeyWithHashAlg};
use ::russh::{ChannelMsg, Disconnect, Sig};
use ngterm_term::GridSize;
use tokio::net::TcpStream;
use tokio::sync::{Notify, mpsc, oneshot};
use tokio::time::Instant;
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
/// Why a channel ended when its reader fell more than [`OUTPUT_BUDGET`] behind.
pub const OUTPUT_OVERRUN: &str = "the channel's output was not read in time";
/// Output a channel holds for a reader that is behind, on top of its queue's [`super::CHANNEL_DEPTH`] chunks; past
/// it the channel is closed rather than hold up the connection's other channels.
pub const OUTPUT_BUDGET: usize = 8 * 1024 * 1024;
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
        attempt: &Withdraw,
    ) -> Result<RusshConnection, TransportError> {
        // One deadline for resolving, connecting, the handshake and authentication, held off only while the user is
        // asked about the host key.
        let deadline = Arc::new(Deadline::new(self.timing.connect));
        let refusal = Arc::new(Mutex::new(None));
        let client = Client {
            machine_id: machine.id.clone(),
            token: attempt.token,
            withdrawn: Arc::clone(&attempt.withdrawn),
            pinned: self.hosts.pinned(machine.id.clone()),
            hosts: Arc::clone(&self.hosts),
            sink: Arc::clone(&self.sink),
            challenges: Arc::clone(&self.challenges),
            confirm: self.timing.confirm,
            deadline: Arc::clone(&deadline),
            refusal: Arc::clone(&refusal),
        };
        let work = async {
            let socket = TcpStream::connect((machine.host.as_str(), machine.port))
                .await
                .map_err(|error| unreachable(machine, &error))?;
            // Keystrokes go out at once rather than waiting to fill a segment.
            let _ = socket.set_nodelay(true);
            let mut handle = client::connect_stream(self.config(), socket, client)
                .await
                .map_err(|error| refused(machine, &refusal, &error))?;
            authenticate(&mut handle, machine, key).await?;
            Ok::<_, TransportError>(handle)
        };
        let handle = tokio::select! {
            reached = work => reached?,
            () = deadline.passed() => return Err(timed_out(machine)),
        };
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
            withdrawn: Arc::new(AtomicBool::new(false)),
        };
        self.reach(machine, key, &withdraw).await
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

    /// Asks about `fingerprint` for `machine_id` under `token`; the answer arrives on the receiver. `None` when the
    /// connect was dropped already (its session task runs on after it): a question it could never withdraw.
    fn ask(
        &self,
        machine_id: &str,
        token: u64,
        fingerprint: &str,
        withdrawn: &AtomicBool,
    ) -> Option<oneshot::Receiver<bool>> {
        let (decide, decision) = oneshot::channel();
        let challenge = Challenge {
            token,
            fingerprint: fingerprint.to_owned(),
            decide,
        };
        let mut pending = self.lock();
        // Checked under the lock the withdrawal takes, so a question is either never asked or withdrawn.
        if withdrawn.load(Ordering::SeqCst) {
            return None;
        }
        // A question still open for the machine belongs to a connect that was overtaken: dropping it refuses it.
        pending.insert(machine_id.to_owned(), challenge);
        Some(decision)
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
    /// Set once the connect is over, so its handshake (which russh runs on in its own task) asks nothing after.
    withdrawn: Arc<AtomicBool>,
}

impl Drop for Withdraw {
    fn drop(&mut self) {
        self.withdrawn.store(true, Ordering::SeqCst);
        self.challenges.withdraw(&self.machine_id, self.token);
    }
}

/// A connect's network deadline, held off while the user is asked about the host key.
struct Deadline {
    at: Mutex<Instant>,
    /// Asks under way: the deadline waits for them.
    asking: AtomicUsize,
    answered: Notify,
}

impl Deadline {
    fn new(budget: Duration) -> Self {
        Self {
            at: Mutex::new(Instant::now() + budget),
            asking: AtomicUsize::new(0),
            answered: Notify::new(),
        }
    }

    fn at(&self) -> Instant {
        *self.at.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Runs `ask`, moving the deadline out by the time it took.
    async fn hold<T>(&self, ask: impl Future<Output = T>) -> T {
        let _asking = Asking {
            deadline: self,
            since: Instant::now(),
        };
        self.asking.fetch_add(1, Ordering::SeqCst);
        ask.await
    }

    /// Resolves once the deadline has passed with no ask under way.
    async fn passed(&self) {
        loop {
            tokio::time::sleep_until(self.at()).await;
            if self.asking.load(Ordering::SeqCst) > 0 {
                self.answered.notified().await;
            } else if self.at() <= Instant::now() {
                return;
            }
        }
    }
}

/// One ask holding a [`Deadline`] off; ended (answered or dropped), it moves the deadline out.
struct Asking<'a> {
    deadline: &'a Deadline,
    since: Instant,
}

impl Drop for Asking<'_> {
    fn drop(&mut self) {
        *self.deadline.at.lock().unwrap_or_else(PoisonError::into_inner) += self.since.elapsed();
        self.deadline.asking.fetch_sub(1, Ordering::SeqCst);
        self.deadline.answered.notify_one();
    }
}

/// russh's side of one connection: it checks the host key.
struct Client {
    machine_id: String,
    /// The connect's token, which its question is asked under.
    token: u64,
    /// Set once the connect is over: no question is asked after.
    withdrawn: Arc<AtomicBool>,
    pinned: Option<String>,
    hosts: Arc<dyn HostKeyStore>,
    sink: Arc<dyn EventSink>,
    challenges: Arc<Challenges>,
    confirm: Duration,
    /// The connect's deadline, held off while the user is asked.
    deadline: Arc<Deadline>,
    /// Why the host key was refused, for the connect's error.
    refusal: Arc<Mutex<Option<String>>>,
}

impl Client {
    fn refuse(&self, reason: &str) -> bool {
        *self.refusal.lock().unwrap_or_else(PoisonError::into_inner) = Some(reason.to_owned());
        false
    }

    async fn ask(&self, offered: String) -> bool {
        let Some(decision) = self
            .challenges
            .ask(&self.machine_id, self.token, &offered, &self.withdrawn)
        else {
            return self.refuse(HOST_KEY_WITHDRAWN);
        };
        self.sink.on_event(MobileEvent::HostKeyUnknown {
            machine_id: self.machine_id.clone(),
            fingerprint: offered.clone(),
        });
        let answer = self.deadline.hold(tokio::time::timeout(self.confirm, decision)).await;
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
        let write = Rc::new(write);
        let (closing, closed) = oneshot::channel();
        let ending = Ending {
            closed,
            connection: Arc::clone(&self.closed),
        };
        tokio::task::spawn_local(copy_output(read, early, output.clone(), Rc::clone(&write), ending));
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
    /// Fires when the core closes the channel; dropped unfired when the input side stops without closing it.
    closed: oneshot::Receiver<()>,
    /// Set when the core closes the connection.
    connection: Arc<AtomicBool>,
}

/// The machine's output on its way to the core: what the reader has not taken yet, and how the channel ended.
#[derive(Default)]
struct Pending {
    chunks: VecDeque<ChannelOutput>,
    /// The bytes in `chunks`.
    bytes: usize,
    /// The exit status or signal, which the machine's close turns into the end.
    exit: Option<ChannelEnd>,
    /// Whether the end is in `chunks`: nothing more is taken from the machine.
    ended: bool,
}

impl Pending {
    /// Takes the machine's next message; `None` is its connection gone, closed by the core when `closed`.
    fn take(&mut self, message: Option<ChannelMsg>, closed: bool) {
        match message {
            Some(ChannelMsg::Data { data }) => self.push(ChannelOutput::Data(data.to_vec())),
            Some(ChannelMsg::ExtendedData { data, ext: 1 }) => self.push(ChannelOutput::Stderr(data.to_vec())),
            Some(ChannelMsg::ExitStatus { exit_status }) => {
                self.exit = Some(ChannelEnd::Exit {
                    code: Some(exit_status),
                    signal: None,
                });
            }
            Some(ChannelMsg::ExitSignal { signal_name, .. }) => {
                self.exit = Some(ChannelEnd::Exit {
                    code: None,
                    signal: Some(signal(signal_name)),
                });
            }
            Some(ChannelMsg::Close) => {
                let end = self
                    .exit
                    .take()
                    .unwrap_or_else(|| ChannelEnd::Lost(PEER_GONE.to_owned()));
                self.end(end);
            }
            None => {
                let reason = if closed {
                    TransportError::Closed.to_string()
                } else {
                    CONNECTION_LOST.to_owned()
                };
                let end = self.exit.take().unwrap_or(ChannelEnd::Lost(reason));
                self.end(end);
            }
            Some(_) => {}
        }
    }

    fn push(&mut self, chunk: ChannelOutput) {
        self.bytes += size(&chunk);
        self.chunks.push_back(chunk);
    }

    fn end(&mut self, end: ChannelEnd) {
        self.chunks.push_back(ChannelOutput::End(end));
        self.ended = true;
    }

    fn pop(&mut self) -> Option<ChannelOutput> {
        let chunk = self.chunks.pop_front()?;
        self.bytes -= size(&chunk);
        Some(chunk)
    }
}

/// The bytes a chunk of output holds.
fn size(chunk: &ChannelOutput) -> usize {
    match chunk {
        ChannelOutput::Data(bytes) | ChannelOutput::Stderr(bytes) => bytes.len(),
        ChannelOutput::End(_) => 0,
    }
}

/// Copies the machine's output to the core until the channel ends or the core stops reading.
///
/// It takes the machine's messages as they come: russh hands them over from the session's one loop, which waits on a
/// channel that is not read, and with it every other channel of the connection. What the reader has not taken is held
/// here, up to [`OUTPUT_BUDGET`] bytes; past it the channel is closed and ends as [`ChannelEnd::Lost`]
/// ([`OUTPUT_OVERRUN`]).
async fn copy_output(
    mut read: ::russh::ChannelReadHalf,
    early: VecDeque<ChannelMsg>,
    output: mpsc::Sender<ChannelOutput>,
    write: Rc<::russh::ChannelWriteHalf<Msg>>,
    ending: Ending,
) {
    let Ending { mut closed, connection } = ending;
    let mut pending = Pending::default();
    for message in early {
        pending.take(Some(message), false);
    }
    // Whether the core can still close the channel: the input side gone without closing it cannot.
    let mut closable = true;
    while !pending.ended {
        // Delivering first, then the machine's messages, so what is queued wins over the core's close.
        tokio::select! {
            biased;
            permit = output.reserve(), if !pending.chunks.is_empty() => {
                // Nobody reads any more: the core closes the channel through its writer when it means to.
                let Ok(permit) = permit else { return };
                if let Some(chunk) = pending.pop() {
                    permit.send(chunk);
                }
            }
            message = read.wait() => pending.take(message, connection.load(Ordering::Relaxed)),
            signal = &mut closed, if closable => match signal {
                Ok(()) => pending.end(ChannelEnd::Lost(CHANNEL_CLOSED.to_owned())),
                Err(_) => closable = false,
            },
        }
        if !pending.ended && pending.bytes > OUTPUT_BUDGET {
            let _ = write.close().await;
            pending.end(ChannelEnd::Lost(OUTPUT_OVERRUN.to_owned()));
        }
    }
    // Nothing more comes from the machine: russh drops what it would hand a reader that is gone.
    drop(read);
    while let Some(chunk) = pending.pop() {
        if output.send(chunk).await.is_err() {
            return;
        }
    }
}

/// Copies the core's input to the machine until the channel closes or every writer is gone; a channel left with no
/// reader and no writer is closed.
///
/// A write can wait long for ssh's window (a command that stops reading its input), so the core's next inputs are
/// taken while it waits: a close, or the channel's reader and writers all going, cancels it.
async fn copy_input(
    write: Rc<::russh::ChannelWriteHalf<Msg>>,
    mut input: mpsc::UnboundedReceiver<ChannelInput>,
    output: mpsc::Sender<ChannelOutput>,
    closing: oneshot::Sender<()>,
) {
    let mut queued = VecDeque::new();
    let mut writers = true;
    loop {
        let next = match queued.pop_front() {
            Some(next) => next,
            None if writers => match input.recv().await {
                Some(next) => next,
                None => break,
            },
            None => break,
        };
        let step = Step {
            write: &write,
            input: &mut input,
            output: &output,
            queued: &mut queued,
            writers: &mut writers,
        };
        match step.run(next).await {
            Sent::Done => {}
            // The channel or its connection is gone; the output side tells the reader how it ended.
            Sent::Failed => return,
            Sent::Close => return close(&write, closing).await,
        }
    }
    // Every writer is gone: the reader still gets what comes, and the channel is closed once it is gone too.
    output.closed().await;
    close(&write, closing).await;
}

/// How one input fared.
enum Sent {
    Done,
    Failed,
    /// The core closed the channel, or left it with no reader and no writer.
    Close,
}

/// One input on its way to the machine, and what the core sends meanwhile.
struct Step<'a> {
    write: &'a ::russh::ChannelWriteHalf<Msg>,
    input: &'a mut mpsc::UnboundedReceiver<ChannelInput>,
    output: &'a mpsc::Sender<ChannelOutput>,
    /// Inputs taken while a write waits, sent after it.
    queued: &'a mut VecDeque<ChannelInput>,
    /// Whether any writer is left.
    writers: &'a mut bool,
}

impl Step<'_> {
    async fn run(self, next: ChannelInput) -> Sent {
        let sending = match next {
            ChannelInput::Close => return Sent::Close,
            other => send(self.write, other),
        };
        tokio::pin!(sending);
        loop {
            tokio::select! {
                biased;
                sent = &mut sending => return if sent.is_ok() { Sent::Done } else { Sent::Failed },
                more = self.input.recv(), if *self.writers => match more {
                    Some(ChannelInput::Close) => return Sent::Close,
                    Some(other) => self.queued.push_back(other),
                    None => *self.writers = false,
                },
                () = self.output.closed(), if !*self.writers => return Sent::Close,
            }
        }
    }
}

/// Sends one input other than a close.
async fn send(write: &::russh::ChannelWriteHalf<Msg>, input: ChannelInput) -> Result<(), ::russh::Error> {
    match input {
        ChannelInput::Data(bytes) => write.data_bytes(bytes).await,
        ChannelInput::Resize(size) => {
            let (width, height) = pixels(size);
            write
                .window_change(size.cols.into(), size.rows.into(), width, height)
                .await
        }
        ChannelInput::Eof => write.eof().await,
        ChannelInput::Close => write.close().await,
    }
}

/// Closes the channel and tells the output side the core closed it.
async fn close(write: &::russh::ChannelWriteHalf<Msg>, closing: oneshot::Sender<()>) {
    let _ = write.close().await;
    let _ = closing.send(());
}

/// A signal's name as `StreamFeed::exited` takes it (`TERM`, `KILL`, …).
fn signal(signal: Sig) -> String {
    match signal {
        Sig::Custom(name) => name,
        named => format!("{named:?}"),
    }
}
