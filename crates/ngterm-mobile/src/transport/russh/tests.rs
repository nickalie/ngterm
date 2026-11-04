//! The russh transport against an in-process russh server: keys from a test vault, host keys pinned in a test store,
//! and exec channels whose commands the server scripts.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use ::russh::keys::{Algorithm, HashAlg, PrivateKey, PublicKey};
use ::russh::server::{self, Auth, ChannelOpenHandle, Msg, Session, VerifiedPublicKey};
use ::russh::{ChannelId, Sig};
use futures_lite::future::zip;
use ngterm_term::GridSize;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::LocalSet;

use super::{
    CHANNEL_CLOSED, CONNECTION_LOST, Client, Deadline, HOST_KEY_CHANGED, HOST_KEY_REFUSED, HOST_KEY_UNCONFIRMED,
    HOST_KEY_WITHDRAWN, OUTPUT_BUDGET, OUTPUT_OVERRUN, RusshConnection, RusshTransport, SIGN_IN_UNFINISHED, Timing,
    banner_link, banner_text,
};
use crate::api::{EventSink, HostKeyStore, KeyVault, Machine, MobileEvent, VaultError};
use crate::keys;
use crate::transport::{
    Channel, ChannelEnd, ChannelInput, ChannelOutput, ChannelReader, Connection, PEER_GONE, Transport, TransportError,
};

const USER: &str = "nick";
const KEY_ID: &str = "phone";

// --- the server ---------------------------------------------------------------------------------------------------

/// What the server saw, in order: ptys, resizes, closes from the client.
type Log = Arc<Mutex<Vec<String>>>;

struct TestServer {
    port: u16,
    log: Log,
    host_key: PublicKey,
}

impl TestServer {
    /// A server with a new host key, accepting `authorized` for [`USER`], on a free local port.
    async fn start(authorized: &PublicKey) -> Self {
        Self::start_with(authorized, Script::default()).await
    }

    /// [`TestServer::start`], confirming each session channel `delay` after it is asked for.
    async fn start_opening_after(authorized: &PublicKey, delay: Duration) -> Self {
        Self::start_with(
            authorized,
            Script {
                open_delay: delay,
                ..Script::default()
            },
        )
        .await
    }

    /// [`TestServer::start`], sending `banner` as authentication starts and answering the client's key `hold` later,
    /// as a server that asks the user to sign in elsewhere does (Tailscale SSH's `check` mode).
    async fn start_holding_auth(authorized: &PublicKey, banner: &str, hold: Duration) -> Self {
        Self::start_with(
            authorized,
            Script {
                banner: Some(banner.to_owned()),
                auth_hold: hold,
                ..Script::default()
            },
        )
        .await
    }

    async fn start_with(authorized: &PublicKey, script: Script) -> Self {
        let host_key = PrivateKey::random(&mut ::russh::keys::key::safe_rng(), Algorithm::Ed25519).unwrap();
        let public = host_key.public_key().clone();
        let config = Arc::new(server::Config {
            keys: vec![host_key],
            auth_rejection_time: Duration::ZERO,
            // Small, so a command that stops reading its input soon leaves the client waiting for the window.
            window_size: 64 * 1024,
            auth_rejection_time_initial: Some(Duration::ZERO),
            ..server::Config::default()
        });
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let log = Log::default();
        let handler = Handler {
            authorized: authorized.clone(),
            log: Arc::clone(&log),
            commands: HashMap::new(),
            ptys: HashMap::new(),
            script,
        };
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let config = Arc::clone(&config);
                let handler = handler.fresh();
                tokio::spawn(serve(config, stream, handler));
            }
        });
        Self {
            port,
            log,
            host_key: public,
        }
    }

    fn fingerprint(&self) -> String {
        self.host_key.fingerprint(HashAlg::Sha256).to_string()
    }

    fn log(&self) -> Vec<String> {
        self.log.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

async fn serve(config: Arc<server::Config>, stream: tokio::net::TcpStream, handler: Handler) {
    if let Ok(running) = server::run_stream(config, stream, handler).await {
        let _ = running.await;
    }
}

/// One server connection. Its commands: `echo TEXT`, `chunks` (out, err, out, exit 3), `signal` (killed by TERM),
/// `vanish` (closes with no status), `cat` (echoes input until EOF, then exits 0), `pty` (reports its pty, and each
/// resize), `later` (`later` and exit 0, a moment after it starts), `flood` (twice [`OUTPUT_BUDGET`] as fast as it is
/// taken), `stall` (never reads its input), `hang`, `drop` (drops the whole connection) and `refuse` (refused).
/// A pty one column wide is refused.
struct Handler {
    authorized: PublicKey,
    log: Log,
    commands: HashMap<ChannelId, String>,
    ptys: HashMap<ChannelId, String>,
    script: Script,
}

/// How a test server behaves besides its commands.
#[derive(Clone, Default)]
struct Script {
    /// How long a session channel waits for its confirmation.
    open_delay: Duration,
    /// The banner sent as authentication starts.
    banner: Option<String>,
    /// How long the answer to the client's key waits.
    auth_hold: Duration,
}

impl Handler {
    fn fresh(&self) -> Self {
        Self {
            authorized: self.authorized.clone(),
            log: Arc::clone(&self.log),
            commands: HashMap::new(),
            ptys: HashMap::new(),
            script: self.script.clone(),
        }
    }

    fn record(&self, line: String) {
        self.log.lock().unwrap_or_else(PoisonError::into_inner).push(line);
    }

    fn exit(session: &mut Session, channel: ChannelId, code: u32) -> Result<(), ::russh::Error> {
        session.exit_status_request(channel, code)?;
        session.eof(channel)?;
        session.close(channel)
    }

    fn run(&self, session: &mut Session, channel: ChannelId, command: &str) -> Result<(), ::russh::Error> {
        match command.split_once(' ').unwrap_or((command, "")) {
            ("echo", text) => {
                session.data(channel, text.as_bytes().to_vec())?;
                Self::exit(session, channel, 0)
            }
            ("chunks", _) => {
                session.data(channel, b"one\n".to_vec())?;
                session.extended_data(channel, 1, b"err\n".to_vec())?;
                session.data(channel, b"two\n".to_vec())?;
                Self::exit(session, channel, 3)
            }
            ("signal", _) => {
                session.exit_signal_request(channel, Sig::TERM, false, "", "")?;
                session.close(channel)
            }
            ("vanish", _) => session.close(channel),
            ("pty", _) => {
                let pty = self.ptys.get(&channel).cloned().unwrap_or_else(|| "none".to_owned());
                session.data(channel, format!("pty {pty}\n").into_bytes())
            }
            ("later", _) => {
                tokio::spawn(later(session.handle(), channel));
                Ok(())
            }
            ("flood", _) => {
                tokio::spawn(flood(session.handle(), channel));
                Ok(())
            }
            ("drop", _) => Err(::russh::Error::Disconnect),
            _ => Ok(()),
        }
    }
}

/// `later`'s run: its output and exit a moment after it starts.
async fn later(handle: server::Handle, channel: ChannelId) {
    tokio::time::sleep(Duration::from_millis(100)).await;
    let _ = handle.data(channel, b"later".to_vec()).await;
    let _ = handle.exit_status_request(channel, 0).await;
    let _ = handle.close(channel).await;
}

/// `flood`'s run: twice [`OUTPUT_BUDGET`], as fast as the client takes it.
async fn flood(handle: server::Handle, channel: ChannelId) {
    let chunk = vec![b'x'; 32 * 1024];
    for _ in 0..2 * OUTPUT_BUDGET / chunk.len() {
        if handle.data(channel, chunk.clone()).await.is_err() {
            break;
        }
    }
}

#[expect(
    clippy::unused_async_trait_impl,
    reason = "russh's handler is async; the test server answers at once"
)]
impl server::Handler for Handler {
    type Error = ::russh::Error;

    async fn authentication_banner(&mut self) -> Result<Option<String>, Self::Error> {
        Ok(self.script.banner.clone())
    }

    async fn auth_publickey(&mut self, user: &str, key: VerifiedPublicKey<'_>) -> Result<Auth, Self::Error> {
        tokio::time::sleep(self.script.auth_hold).await;
        if user == USER && key.public_key.key_data() == self.authorized.key_data() {
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
        }
    }

    async fn channel_open_session(
        &mut self,
        _channel: ::russh::Channel<Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.script.open_delay.is_zero() {
            reply.accept().await;
        } else {
            let delay = self.script.open_delay;
            tokio::spawn(async move {
                tokio::time::sleep(delay).await;
                reply.accept().await;
            });
        }
        Ok(())
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        term: &str,
        cols: u32,
        rows: u32,
        width: u32,
        height: u32,
        _modes: &[(::russh::Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let pty = format!("{term} {cols}x{rows} {width}x{height}");
        self.record(format!("pty {pty}"));
        if cols == 1 {
            return session.channel_failure(channel);
        }
        self.ptys.insert(channel, pty);
        session.channel_success(channel)
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let command = String::from_utf8_lossy(data).into_owned();
        if command == "refuse" {
            return session.channel_failure(channel);
        }
        session.channel_success(channel)?;
        self.run(session, channel, &command)?;
        self.commands.insert(channel, command);
        Ok(())
    }

    async fn data(&mut self, channel: ChannelId, data: &[u8], session: &mut Session) -> Result<(), Self::Error> {
        if self.commands.get(&channel).map(String::as_str) == Some("stall") {
            // The connection stops here: no more input is taken, and the window is never opened again.
            std::future::pending::<()>().await;
        }
        session.data(channel, data.to_vec())
    }

    async fn channel_close(&mut self, _channel: ChannelId, _session: &mut Session) -> Result<(), Self::Error> {
        self.record("close".to_owned());
        Ok(())
    }

    async fn channel_eof(&mut self, channel: ChannelId, session: &mut Session) -> Result<(), Self::Error> {
        if self.commands.get(&channel).map(String::as_str) == Some("cat") {
            Self::exit(session, channel, 0)?;
        }
        Ok(())
    }

    async fn window_change_request(
        &mut self,
        channel: ChannelId,
        cols: u32,
        rows: u32,
        width: u32,
        height: u32,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let size = format!("{cols}x{rows} {width}x{height}");
        self.record(format!("resize {size}"));
        session.data(channel, format!("resize {size}\n").into_bytes())
    }
}

// --- the app's side -----------------------------------------------------------------------------------------------

struct Vault {
    keys: HashMap<String, Vec<u8>>,
}

impl KeyVault for Vault {
    fn load(&self, key_id: String) -> Result<Vec<u8>, VaultError> {
        self.keys.get(&key_id).cloned().ok_or(VaultError::Missing { key_id })
    }
}

#[derive(Default)]
struct Hosts {
    pins: Mutex<HashMap<String, String>>,
}

impl Hosts {
    fn pins(&self) -> HashMap<String, String> {
        self.pins.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

impl HostKeyStore for Hosts {
    fn pinned(&self, machine_id: String) -> Option<String> {
        self.pins().get(&machine_id).cloned()
    }

    fn pin(&self, machine_id: String, fingerprint: String) {
        self.pins
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(machine_id, fingerprint);
    }
}

struct Sink {
    events: mpsc::UnboundedSender<MobileEvent>,
}

impl EventSink for Sink {
    fn on_event(&self, event: MobileEvent) {
        let _ = self.events.send(event);
    }
}

/// A transport with a fresh key in its vault, the key's public half, and where its events arrive.
struct Fixture {
    transport: RusshTransport,
    hosts: Arc<Hosts>,
    events: mpsc::UnboundedReceiver<MobileEvent>,
    public: PublicKey,
}

fn fixture(timing: Timing) -> Fixture {
    let pair = keys::generate("test").unwrap();
    let public = PublicKey::from_openssh(&pair.authorized_key).unwrap();
    let vault = Arc::new(Vault {
        keys: HashMap::from([(KEY_ID.to_owned(), pair.private_key)]),
    });
    let hosts = Arc::new(Hosts::default());
    let (sender, events) = mpsc::unbounded_channel();
    let sink = Arc::new(Sink { events: sender });
    let transport = RusshTransport::with_timing(vault, Arc::clone(&hosts) as Arc<dyn HostKeyStore>, sink, timing);
    Fixture {
        transport,
        hosts,
        events,
        public,
    }
}

fn quick() -> Timing {
    Timing {
        connect: Duration::from_secs(10),
        confirm: Duration::from_secs(10),
        sign_in: Duration::from_secs(10),
        sign_in_grace: Duration::from_millis(50),
        keepalive: Duration::from_secs(10),
    }
}

fn machine(port: u16) -> Machine {
    Machine {
        id: "m1".to_owned(),
        label: "box".to_owned(),
        host: "127.0.0.1".to_owned(),
        port,
        user: USER.to_owned(),
        key_id: KEY_ID.to_owned(),
    }
}

/// Runs `test` on a current-thread runtime inside a `LocalSet`, as the core's actor does.
fn run<F: Future>(test: F) -> F::Output {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    LocalSet::new().block_on(&runtime, async move {
        tokio::time::timeout(Duration::from_secs(30), test)
            .await
            .expect("the test finishes in time")
    })
}

/// Connects with the server's key already pinned.
async fn connected(fixture: &Fixture, server: &TestServer) -> RusshConnection {
    fixture.hosts.pin("m1".to_owned(), server.fingerprint());
    match fixture.transport.connect(&machine(server.port)).await {
        Ok(connection) => connection,
        Err(error) => panic!("connect: {error}"),
    }
}

/// Everything the channel delivers up to and including its end.
async fn drain(channel: &mut Channel) -> (Vec<u8>, Vec<u8>, ChannelEnd) {
    drain_reader(&mut channel.reader).await
}

/// [`drain`] for a reader on its own.
async fn drain_reader(reader: &mut ChannelReader) -> (Vec<u8>, Vec<u8>, ChannelEnd) {
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    loop {
        match reader.read().await {
            ChannelOutput::Data(bytes) => stdout.extend(bytes),
            ChannelOutput::Stderr(bytes) => stderr.extend(bytes),
            ChannelOutput::End(end) => return (stdout, stderr, end),
        }
    }
}

/// Waits until the server has seen the client close a channel.
async fn closed_by_client(server: &TestServer) {
    while !server.log().iter().any(|line| line == "close") {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Reads stdout until it holds `wanted`.
async fn read_until(channel: &mut Channel, wanted: &str) -> String {
    let mut seen = Vec::new();
    while !String::from_utf8_lossy(&seen).contains(wanted) {
        match channel.reader.read().await {
            ChannelOutput::Data(bytes) => seen.extend(bytes),
            other => panic!(
                "expected {wanted:?}, got {other:?} after {:?}",
                String::from_utf8_lossy(&seen)
            ),
        }
    }
    String::from_utf8_lossy(&seen).into_owned()
}

fn exited(code: u32) -> ChannelEnd {
    ChannelEnd::Exit {
        code: Some(code),
        signal: None,
    }
}

// --- connecting ---------------------------------------------------------------------------------------------------

#[test]
fn a_pinned_host_connects_with_the_key_from_the_vault() {
    run(async {
        let mut fixture = fixture(quick());
        let server = TestServer::start(&fixture.public).await;

        let connection = connected(&fixture, &server).await;

        let mut channel = connection.exec("echo hi", None).await.unwrap();
        assert_eq!(drain(&mut channel).await, (b"hi".to_vec(), Vec::new(), exited(0)));
        assert!(fixture.events.try_recv().is_err(), "a pinned key asks nothing");
    });
}

#[test]
fn an_unknown_host_asks_and_proceeds_once_trusted_and_pinned() {
    run(async {
        let mut fixture = fixture(quick());
        let server = TestServer::start(&fixture.public).await;
        let transport = &fixture.transport;

        let answer = async {
            let event = fixture.events.recv().await.unwrap();
            assert_eq!(
                event,
                MobileEvent::HostKeyUnknown {
                    machine_id: "m1".to_owned(),
                    fingerprint: server.fingerprint(),
                }
            );
            assert!(fixture.hosts.pins().is_empty(), "nothing is pinned before the answer");
            transport.answer_host_key("m1", &server.fingerprint(), true);
        };
        let (connected, ()) = zip(transport.connect(&machine(server.port)), answer).await;

        assert!(connected.is_ok());
        assert_eq!(
            fixture.hosts.pins(),
            HashMap::from([("m1".to_owned(), server.fingerprint())])
        );

        // A later connect with the same key passes without asking.
        assert!(transport.connect(&machine(server.port)).await.is_ok());
        assert!(fixture.events.try_recv().is_err());
    });
}

#[test]
fn a_refused_or_mismatched_answer_ends_the_connect_and_pins_nothing() {
    run(async {
        let mut fixture = fixture(quick());
        let server = TestServer::start(&fixture.public).await;
        let transport = &fixture.transport;

        for (fingerprint, trust) in [(server.fingerprint(), false), ("SHA256:other".to_owned(), true)] {
            let answer = async {
                fixture.events.recv().await.unwrap();
                transport.answer_host_key("m1", &fingerprint, trust);
            };
            let (connected, ()) = zip(transport.connect(&machine(server.port)), answer).await;

            assert_eq!(
                connected.err(),
                Some(TransportError::Refused(HOST_KEY_REFUSED.to_owned()))
            );
            assert!(fixture.hosts.pins().is_empty());
        }
    });
}

#[test]
fn an_unanswered_question_runs_out() {
    run(async {
        let fixture = fixture(Timing {
            confirm: Duration::from_millis(200),
            ..quick()
        });
        let server = TestServer::start(&fixture.public).await;

        let connected = fixture.transport.connect(&machine(server.port)).await;

        assert_eq!(
            connected.err(),
            Some(TransportError::Unreachable(HOST_KEY_UNCONFIRMED.to_owned()))
        );
    });
}

#[test]
fn a_dropped_connect_withdraws_its_question() {
    run(async {
        let mut fixture = fixture(quick());
        let server = TestServer::start(&fixture.public).await;

        let machine = machine(server.port);
        let connect = fixture.transport.connect(&machine);
        let asked = async {
            fixture.events.recv().await.unwrap();
        };
        // The connect is dropped once the question is out.
        futures_lite::future::or(async { drop(connect.await) }, asked).await;

        assert!(fixture.transport.challenges.lock().is_empty());
    });
}

#[test]
fn a_changed_host_key_fails_with_both_fingerprints() {
    run(async {
        let mut fixture = fixture(quick());
        let server = TestServer::start(&fixture.public).await;
        let impostor = TestServer::start(&fixture.public).await;
        fixture.hosts.pin("m1".to_owned(), server.fingerprint());

        let connected = fixture.transport.connect(&machine(impostor.port)).await;

        assert_eq!(
            connected.err(),
            Some(TransportError::Refused(HOST_KEY_CHANGED.to_owned()))
        );
        assert_eq!(
            fixture.events.try_recv().unwrap(),
            MobileEvent::HostKeyChanged {
                machine_id: "m1".to_owned(),
                pinned: server.fingerprint(),
                offered: impostor.fingerprint(),
            }
        );
        assert_eq!(fixture.hosts.pins()["m1"], server.fingerprint(), "the pin stays");
    });
}

#[test]
fn the_store_is_the_only_authority_on_the_pin() {
    run(async {
        let mut fixture = fixture(quick());
        let server = TestServer::start(&fixture.public).await;
        // A pin removed from the store is not trusted: the store is all the transport asks.
        let machine = machine(server.port);
        let transport = &fixture.transport;

        let answer = async {
            fixture.events.recv().await.unwrap();
            transport.answer_host_key("m1", &server.fingerprint(), false);
        };
        let (connected, ()) = zip(transport.connect(&machine), answer).await;

        assert!(connected.is_err());
    });
}

#[test]
fn a_key_the_machine_does_not_accept_fails_authentication() {
    run(async {
        let fixture = fixture(quick());
        let stranger = keys::generate("stranger").unwrap();
        let server = TestServer::start(&PublicKey::from_openssh(&stranger.authorized_key).unwrap()).await;
        fixture.hosts.pin("m1".to_owned(), server.fingerprint());

        let connected = fixture.transport.connect(&machine(server.port)).await;

        assert_eq!(
            connected.err(),
            Some(TransportError::Refused(format!(
                "{USER}@127.0.0.1 did not accept the key"
            )))
        );
    });
}

#[test]
fn a_missing_key_fails_before_reaching_the_machine() {
    run(async {
        let fixture = fixture(quick());
        let machine = Machine {
            key_id: "gone".to_owned(),
            ..machine(1)
        };

        let connected = fixture.transport.connect(&machine).await;

        assert_eq!(
            connected.err(),
            Some(TransportError::Unreachable("no key gone".to_owned()))
        );
    });
}

#[test]
fn a_machine_that_never_answers_times_out() {
    run(async {
        let fixture = fixture(Timing {
            connect: Duration::from_millis(200),
            ..quick()
        });
        // Accepts the connection but never speaks ssh.
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        fixture.hosts.pin("m1".to_owned(), "SHA256:any".to_owned());

        let connected = fixture.transport.connect(&machine(port)).await;

        assert_eq!(
            connected.err(),
            Some(TransportError::Unreachable(format!(
                "127.0.0.1:{port} did not answer in time"
            )))
        );
        drop(listener);
    });
}

#[test]
fn an_unknown_machine_that_never_answers_times_out_without_the_confirm_wait() {
    run(async {
        let fixture = fixture(Timing {
            connect: Duration::from_millis(200),
            ..quick()
        });
        // Accepts the connection but never speaks ssh, so nothing is ever asked.
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let started = std::time::Instant::now();

        let connected = fixture.transport.connect(&machine(port)).await;

        assert!(connected.is_err());
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "only the connect's own time"
        );
        drop(listener);
    });
}

#[test]
fn the_wait_for_an_answer_does_not_count_against_the_connect() {
    run(async {
        let mut fixture = fixture(Timing {
            connect: Duration::from_secs(1),
            ..quick()
        });
        let server = TestServer::start(&fixture.public).await;
        let transport = &fixture.transport;

        let answer = async {
            fixture.events.recv().await.unwrap();
            tokio::time::sleep(Duration::from_millis(1500)).await;
            transport.answer_host_key("m1", &server.fingerprint(), true);
        };
        let (connected, ()) = zip(transport.connect(&machine(server.port)), answer).await;

        assert!(connected.is_ok());
    });
}

// --- signing in ---------------------------------------------------------------------------------------------------

/// What Tailscale SSH sends a node its policy holds for a `check`: the link to sign in at.
const SIGN_IN_BANNER: &str =
    "# Tailscale SSH requires an additional check.\n# To authenticate, visit: https://login.tailscale.com/a/l1nk\n";

/// The sign-in link in [`SIGN_IN_BANNER`].
const SIGN_IN_LINK: &str = "https://login.tailscale.com/a/l1nk";

fn sign_in_requested() -> MobileEvent {
    MobileEvent::SignInRequested {
        machine_id: "m1".to_owned(),
        message: SIGN_IN_BANNER.trim().to_owned(),
        url: SIGN_IN_LINK.to_owned(),
    }
}

/// [`quick`] with a short network deadline, which a sign-in outlasts.
fn signing_in() -> Timing {
    Timing {
        connect: Duration::from_millis(300),
        ..quick()
    }
}

#[test]
fn a_machine_holding_authentication_for_a_sign_in_keeps_the_connect_waiting() {
    run(async {
        let mut fixture = fixture(signing_in());
        let server = TestServer::start_holding_auth(&fixture.public, SIGN_IN_BANNER, Duration::from_millis(1200)).await;
        fixture.hosts.pin("m1".to_owned(), server.fingerprint());
        let started = std::time::Instant::now();

        let connected = fixture.transport.connect(&machine(server.port)).await;

        assert!(connected.is_ok(), "{:?}", connected.err());
        assert!(
            started.elapsed() >= Duration::from_millis(1200),
            "it waited for the machine"
        );
        assert_eq!(fixture.events.try_recv().ok(), Some(sign_in_requested()));
        assert!(fixture.events.try_recv().is_err(), "asked once");
    });
}

#[test]
fn a_sign_in_not_completed_in_time_fails_the_connect() {
    run(async {
        let mut fixture = fixture(Timing {
            sign_in: Duration::from_millis(700),
            ..signing_in()
        });
        let server = TestServer::start_holding_auth(&fixture.public, SIGN_IN_BANNER, Duration::from_secs(60)).await;
        fixture.hosts.pin("m1".to_owned(), server.fingerprint());
        let started = std::time::Instant::now();

        let connected = fixture.transport.connect(&machine(server.port)).await;

        assert_eq!(
            connected.err(),
            Some(TransportError::Unreachable(SIGN_IN_UNFINISHED.to_owned()))
        );
        assert!(
            started.elapsed() >= Duration::from_millis(700),
            "the sign-in's wait, not the connect's"
        );
        assert_eq!(fixture.events.try_recv().ok(), Some(sign_in_requested()));
    });
}

#[test]
fn a_dropped_connect_stops_waiting_for_the_sign_in_and_raises_nothing_after() {
    run(async {
        let mut fixture = fixture(Timing {
            sign_in_grace: Duration::from_millis(400),
            ..signing_in()
        });
        let server = TestServer::start_holding_auth(&fixture.public, SIGN_IN_BANNER, Duration::from_secs(60)).await;
        fixture.hosts.pin("m1".to_owned(), server.fingerprint());
        let machine = machine(server.port);

        // Dropped once the banner is in, before its grace is over (a disconnect, an edit, a suspend).
        let connect = fixture.transport.connect(&machine);
        let dropped = futures_lite::future::or(async { Some(connect.await.err()) }, async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            None
        })
        .await;
        assert!(dropped.is_none(), "the connect was still waiting: {dropped:?}");
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert!(
            fixture.events.try_recv().is_err(),
            "a connect that is over asks nothing"
        );

        // Dropped once it asked: nothing follows.
        let connect = fixture.transport.connect(&machine);
        let asked = futures_lite::future::or(async { Err(connect.await.err()) }, async {
            Ok(fixture.events.recv().await)
        })
        .await;
        assert_eq!(asked, Ok(Some(sign_in_requested())));
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert!(fixture.events.try_recv().is_err());
    });
}

#[test]
fn a_banner_without_a_link_changes_nothing() {
    run(async {
        let mut fixture = fixture(signing_in());
        let banner = "Authorized use only.\n";
        let held = TestServer::start_holding_auth(&fixture.public, banner, Duration::from_millis(1200)).await;
        let prompt = TestServer::start_holding_auth(&fixture.public, banner, Duration::ZERO).await;

        fixture.hosts.pin("m1".to_owned(), held.fingerprint());
        let connected = fixture.transport.connect(&machine(held.port)).await;
        assert_eq!(
            connected.err(),
            Some(TransportError::Unreachable(format!(
                "127.0.0.1:{} did not answer in time",
                held.port
            ))),
            "the network deadline still counts"
        );

        fixture.hosts.pin("m1".to_owned(), prompt.fingerprint());
        assert!(fixture.transport.connect(&machine(prompt.port)).await.is_ok());
        assert!(fixture.events.try_recv().is_err(), "nothing is asked");
    });
}

#[test]
fn a_link_the_machine_does_not_wait_on_asks_nothing() {
    run(async {
        // quick()'s connect deadline, not signing_in()'s 300 ms: the handshake before the banner counts against it,
        // and a debug build's on a busy simulator can take that long; this test is about the grace alone
        let mut fixture = fixture(Timing {
            sign_in_grace: Duration::from_millis(500),
            ..quick()
        });
        let banner = "Authorized use only: see https://example.com/policy.\n";
        let server = TestServer::start_holding_auth(&fixture.public, banner, Duration::from_millis(100)).await;
        fixture.hosts.pin("m1".to_owned(), server.fingerprint());

        let connected = fixture.transport.connect(&machine(server.port)).await;
        assert!(connected.is_ok(), "{:?}", connected.err());
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert!(
            fixture.events.try_recv().is_err(),
            "a link answered within the grace asks nothing"
        );
    });
}

#[test]
fn a_banner_link_is_its_first_http_link_without_the_sentence_around_it() {
    assert_eq!(banner_link(SIGN_IN_BANNER).as_deref(), Some(SIGN_IN_LINK));
    assert_eq!(
        banner_link("Sign in (at HTTPS://Example.com/a?b=c#d), then wait.").as_deref(),
        Some("HTTPS://Example.com/a?b=c#d")
    );
    assert_eq!(
        banner_link("see http://[::1]:8080/x. or https://later.example").as_deref(),
        Some("http://[::1]:8080/x")
    );
    assert_eq!(
        banner_link("wiki: https://en.wikipedia.org/wiki/Foo_(bar)!").as_deref(),
        Some("https://en.wikipedia.org/wiki/Foo_(bar)")
    );
    assert_eq!(
        banner_link("<https://example.com/q>").as_deref(),
        Some("https://example.com/q")
    );
    assert_eq!(banner_link("Authorized use only."), None);
    assert_eq!(banner_link("ftp://example.com or https:// alone"), None);
    assert_eq!(banner_link("javascript:alert(1) https:/x"), None);
}

#[test]
fn a_banner_is_shown_without_control_characters_or_blank_edges() {
    assert_eq!(
        banner_text("\n\u{1b}[31m# visit:\thttps://x.example\u{1b}[0m\r\n\n"),
        "[31m# visit:\thttps://x.example[0m"
    );
}

#[test]
fn a_connect_already_dropped_asks_nothing() {
    run(async {
        let mut fixture = fixture(quick());
        let refusal = Arc::new(Mutex::new(None));
        let client = Client {
            _alive: tokio::sync::watch::channel(()).0,
            machine_id: "m1".to_owned(),
            token: 7,
            withdrawn: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            pinned: None,
            hosts: Arc::clone(&fixture.transport.hosts),
            sink: Arc::clone(&fixture.transport.sink),
            challenges: Arc::clone(&fixture.transport.challenges),
            confirm: Duration::from_secs(10),
            sign_in_wait: Duration::from_secs(10),
            deadline: Arc::new(Deadline::new(Duration::from_secs(10))),
            refusal: Arc::clone(&refusal),
            sign_in: mpsc::unbounded_channel().0,
        };

        assert!(!client.ask("SHA256:offered".to_owned()).await);

        assert_eq!(
            *refusal.lock().unwrap(),
            Some(TransportError::Unreachable(HOST_KEY_WITHDRAWN.to_owned()))
        );
        assert!(fixture.transport.challenges.lock().is_empty());
        assert!(fixture.events.try_recv().is_err(), "nothing is shown");
    });
}

// --- channels -----------------------------------------------------------------------------------------------------

#[test]
fn exec_without_a_pty_delivers_bytes_in_order_and_then_the_exit() {
    run(async {
        let fixture = fixture(quick());
        let server = TestServer::start(&fixture.public).await;
        let connection = connected(&fixture, &server).await;

        let mut channel = connection.exec("chunks", None).await.unwrap();

        assert_eq!(
            drain(&mut channel).await,
            (b"one\ntwo\n".to_vec(), b"err\n".to_vec(), exited(3))
        );
        assert!(server.log().is_empty(), "no pty was asked for");
    });
}

#[test]
fn input_reaches_the_command_and_eof_ends_it() {
    run(async {
        let fixture = fixture(quick());
        let server = TestServer::start(&fixture.public).await;
        let connection = connected(&fixture, &server).await;

        let mut channel = connection.exec("cat", None).await.unwrap();
        assert!(channel.writer.write(b"abc"));
        assert!(channel.writer.write(b"def"));
        assert!(channel.writer.eof());

        assert_eq!(drain(&mut channel).await, (b"abcdef".to_vec(), Vec::new(), exited(0)));
    });
}

#[test]
fn exec_with_a_pty_sends_its_size_and_resizes() {
    run(async {
        let fixture = fixture(quick());
        let server = TestServer::start(&fixture.public).await;
        let connection = connected(&fixture, &server).await;
        let size = GridSize {
            cols: 80,
            rows: 24,
            cell_width: 10,
            cell_height: 20,
        };

        let mut channel = connection.exec("pty", Some(size)).await.unwrap();
        read_until(&mut channel, "pty xterm-256color 80x24 800x480\n").await;
        assert!(channel.writer.resize(GridSize {
            cols: 100,
            rows: 30,
            ..size
        }));
        read_until(&mut channel, "resize 100x30 1000x600\n").await;

        assert_eq!(
            server.log(),
            ["pty xterm-256color 80x24 800x480", "resize 100x30 1000x600"]
        );
    });
}

#[test]
fn a_signal_ends_the_channel_with_its_name() {
    run(async {
        let fixture = fixture(quick());
        let server = TestServer::start(&fixture.public).await;
        let connection = connected(&fixture, &server).await;

        let mut channel = connection.exec("signal", None).await.unwrap();

        let end = ChannelEnd::Exit {
            code: None,
            signal: Some("TERM".to_owned()),
        };
        assert_eq!(drain(&mut channel).await, (Vec::new(), Vec::new(), end));
    });
}

#[test]
fn a_channel_closed_without_a_status_is_lost() {
    run(async {
        let fixture = fixture(quick());
        let server = TestServer::start(&fixture.public).await;
        let connection = connected(&fixture, &server).await;

        let mut channel = connection.exec("vanish", None).await.unwrap();

        assert_eq!(drain(&mut channel).await.2, ChannelEnd::Lost(PEER_GONE.to_owned()));
    });
}

#[test]
fn a_refused_command_is_rejected() {
    run(async {
        let fixture = fixture(quick());
        let server = TestServer::start(&fixture.public).await;
        let connection = connected(&fixture, &server).await;

        let refused = connection.exec("refuse", None).await;

        assert!(matches!(refused, Err(TransportError::Rejected(_))));
        closed_by_client(&server).await;
        assert_eq!(server.log(), ["close"]);
    });
}

#[test]
fn a_refused_pty_is_rejected_and_its_channel_closed() {
    run(async {
        let fixture = fixture(quick());
        let server = TestServer::start(&fixture.public).await;
        let connection = connected(&fixture, &server).await;
        let size = GridSize {
            cols: 1,
            rows: 24,
            cell_width: 10,
            cell_height: 20,
        };

        let refused = connection.exec("pty", Some(size)).await;

        assert!(matches!(refused, Err(TransportError::Rejected(reason)) if reason == "the machine refused a pty"));
        closed_by_client(&server).await;
        assert_eq!(server.log(), ["pty xterm-256color 1x24 10x480", "close"]);
    });
}

#[test]
fn an_exec_dropped_before_its_channel_opens_closes_it() {
    run(async {
        let fixture = fixture(quick());
        let server = TestServer::start_opening_after(&fixture.public, Duration::from_millis(200)).await;
        let connection = connected(&fixture, &server).await;

        let dropped = tokio::time::timeout(Duration::from_millis(50), connection.exec("hang", None)).await;

        assert!(dropped.is_err(), "the exec is dropped while its channel is opening");
        closed_by_client(&server).await;
        assert_eq!(server.log(), ["close"]);
    });
}

#[test]
fn a_dropped_connection_ends_every_channel_as_lost() {
    run(async {
        let fixture = fixture(quick());
        let server = TestServer::start(&fixture.public).await;
        let connection = connected(&fixture, &server).await;
        let mut first = connection.exec("hang", None).await.unwrap();
        let mut second = connection.exec("hang", None).await.unwrap();

        assert!(connection.exec("drop", None).await.is_err());

        let lost = ChannelEnd::Lost(CONNECTION_LOST.to_owned());
        assert_eq!(drain(&mut first).await.2, lost);
        assert_eq!(drain(&mut second).await.2, lost);
        // And the connection says it is gone.
        connection.lost().await;
    });
}

#[test]
fn closing_the_connection_ends_its_channels_and_refuses_new_ones() {
    run(async {
        let fixture = fixture(quick());
        let server = TestServer::start(&fixture.public).await;
        let connection = connected(&fixture, &server).await;
        let mut channel = connection.exec("hang", None).await.unwrap();

        connection.close();

        assert_eq!(
            drain(&mut channel).await.2,
            ChannelEnd::Lost(TransportError::Closed.to_string())
        );
        connection.lost().await;
        assert_eq!(
            connection.exec("echo hi", None).await.err(),
            Some(TransportError::Closed)
        );
    });
}

#[test]
fn closing_a_channel_ends_it() {
    run(async {
        let fixture = fixture(quick());
        let server = TestServer::start(&fixture.public).await;
        let connection = connected(&fixture, &server).await;
        let mut channel = connection.exec("hang", None).await.unwrap();

        assert!(channel.writer.send(ChannelInput::Close));

        assert_eq!(drain(&mut channel).await.2, ChannelEnd::Lost(CHANNEL_CLOSED.to_owned()));
        // The connection itself stays up, and the server saw the close before the next command.
        let mut other = connection.exec("echo still", None).await.unwrap();
        assert_eq!(drain(&mut other).await.0, b"still");
        assert_eq!(server.log(), ["close"]);
    });
}

#[test]
fn closing_a_channel_cancels_a_write_waiting_for_the_window() {
    run(async {
        let fixture = fixture(quick());
        let server = TestServer::start(&fixture.public).await;
        let connection = connected(&fixture, &server).await;
        let mut channel = connection.exec("stall", None).await.unwrap();

        // Far more than the machine's window, which it never opens again.
        assert!(channel.writer.write(&vec![b'x'; 1024 * 1024]));
        assert!(channel.writer.close());

        assert_eq!(drain(&mut channel).await.2, ChannelEnd::Lost(CHANNEL_CLOSED.to_owned()));
    });
}

#[test]
fn a_channel_dropped_by_the_core_is_closed() {
    run(async {
        let fixture = fixture(quick());
        let server = TestServer::start(&fixture.public).await;
        let connection = connected(&fixture, &server).await;
        let channel = connection.exec("hang", None).await.unwrap();

        drop(channel);

        while server.log().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(server.log(), ["close"]);
    });
}

#[test]
fn a_channel_whose_writers_are_gone_still_delivers_and_is_closed_with_its_reader() {
    run(async {
        let fixture = fixture(quick());
        let server = TestServer::start(&fixture.public).await;
        let connection = connected(&fixture, &server).await;

        let Channel { mut reader, writer } = connection.exec("later", None).await.unwrap();
        drop(writer);
        assert_eq!(
            drain_reader(&mut reader).await,
            (b"later".to_vec(), Vec::new(), exited(0))
        );

        let Channel { reader, writer } = connection.exec("hang", None).await.unwrap();
        drop(writer);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(server.log().is_empty(), "the reader still holds the channel open");
        drop(reader);
        closed_by_client(&server).await;
        assert_eq!(server.log(), ["close"]);
    });
}

#[test]
fn a_reader_far_behind_loses_its_channel_but_holds_up_no_other() {
    run(async {
        let fixture = fixture(quick());
        let server = TestServer::start(&fixture.public).await;
        let connection = connected(&fixture, &server).await;

        // Nobody reads the flood until the transport has given it up.
        let mut flood = connection.exec("flood", None).await.unwrap();
        closed_by_client(&server).await;

        let mut other = connection.exec("echo still", None).await.unwrap();
        assert_eq!(drain(&mut other).await.0, b"still");
        let (stdout, _, end) = drain(&mut flood).await;
        assert_eq!(end, ChannelEnd::Lost(OUTPUT_OVERRUN.to_owned()));
        assert!(stdout.len() > OUTPUT_BUDGET);
    });
}

// --- through the facade -------------------------------------------------------------------------------------------

/// Events for a test thread outside any runtime, which waits for them with a deadline.
struct ThreadSink {
    events: Mutex<std::sync::mpsc::Sender<MobileEvent>>,
}

impl EventSink for ThreadSink {
    fn on_event(&self, event: MobileEvent) {
        let _ = self.events.lock().unwrap_or_else(PoisonError::into_inner).send(event);
    }
}

/// A server on a thread of its own, for a core whose actor has its own runtime.
fn server_thread(authorized: PublicKey) -> (u16, String) {
    server_thread_with(authorized, Script::default())
}

/// [`server_thread`] with `script`.
fn server_thread_with(authorized: PublicKey, script: Script) -> (u16, String) {
    let (started, port) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let server = TestServer::start_with(&authorized, script).await;
            started.send((server.port, server.fingerprint())).unwrap();
            std::future::pending::<()>().await;
        });
    });
    port.recv().unwrap()
}

#[test]
fn the_core_asks_about_an_unknown_host_and_connects_once_answered() {
    let pair = keys::generate("test").unwrap();
    let (port, fingerprint) = server_thread(PublicKey::from_openssh(&pair.authorized_key).unwrap());
    let vault = Arc::new(Vault {
        keys: HashMap::from([(KEY_ID.to_owned(), pair.private_key)]),
    });
    let hosts = Arc::new(Hosts::default());
    let (sender, events) = std::sync::mpsc::channel();
    let next = || events.recv_timeout(Duration::from_secs(10)).ok();
    let core = crate::MobileCore::new(
        Arc::new(ThreadSink {
            events: Mutex::new(sender),
        }),
        vault,
        Arc::clone(&hosts) as Arc<dyn HostKeyStore>,
    )
    .unwrap();

    let connecting = {
        let core = Arc::clone(&core);
        std::thread::spawn(move || futures_lite::future::block_on(core.connect(machine(port))))
    };
    assert_eq!(
        next(),
        Some(MobileEvent::HostKeyUnknown {
            machine_id: "m1".to_owned(),
            fingerprint: fingerprint.clone(),
        })
    );
    core.answer_host_key("m1".to_owned(), fingerprint.clone(), true)
        .unwrap();

    assert_eq!(connecting.join().unwrap(), Ok(()));
    assert_eq!(
        next(),
        Some(MobileEvent::MachineState {
            machine_id: "m1".to_owned(),
            state: crate::MachineState::Connected,
        })
    );
    assert_eq!(hosts.pins(), HashMap::from([("m1".to_owned(), fingerprint)]));
}

/// A core over the real transport, reaching a server on its own thread run by `script`, with its host key pinned.
fn signing_in_core(script: Script) -> (Arc<crate::MobileCore>, u16, std::sync::mpsc::Receiver<MobileEvent>) {
    let pair = keys::generate("test").unwrap();
    let (port, fingerprint) = server_thread_with(PublicKey::from_openssh(&pair.authorized_key).unwrap(), script);
    let vault = Arc::new(Vault {
        keys: HashMap::from([(KEY_ID.to_owned(), pair.private_key)]),
    });
    let hosts = Arc::new(Hosts::default());
    hosts.pin("m1".to_owned(), fingerprint);
    let (sender, events) = std::sync::mpsc::channel();
    let core = crate::MobileCore::new(
        Arc::new(ThreadSink {
            events: Mutex::new(sender),
        }),
        vault,
        hosts,
    )
    .unwrap();
    (core, port, events)
}

#[test]
fn the_core_asks_for_a_sign_in_and_the_connected_state_ends_it() {
    let (core, port, events) = signing_in_core(Script {
        banner: Some(SIGN_IN_BANNER.to_owned()),
        auth_hold: Duration::from_secs(2),
        ..Script::default()
    });
    let next = || events.recv_timeout(Duration::from_secs(10)).ok();

    let connecting = {
        let core = Arc::clone(&core);
        std::thread::spawn(move || futures_lite::future::block_on(core.connect(machine(port))))
    };

    assert_eq!(next(), Some(sign_in_requested()));
    assert_eq!(
        next(),
        Some(MobileEvent::MachineState {
            machine_id: "m1".to_owned(),
            state: crate::MachineState::Connected,
        })
    );
    assert_eq!(connecting.join().unwrap(), Ok(()));
}

#[test]
fn a_disconnect_while_the_core_waits_for_a_sign_in_cancels_it_quietly() {
    let (core, port, events) = signing_in_core(Script {
        banner: Some(SIGN_IN_BANNER.to_owned()),
        auth_hold: Duration::from_secs(60),
        ..Script::default()
    });

    let connecting = {
        let core = Arc::clone(&core);
        std::thread::spawn(move || futures_lite::future::block_on(core.connect(machine(port))))
    };
    assert_eq!(
        events.recv_timeout(Duration::from_secs(10)).ok(),
        Some(sign_in_requested())
    );
    core.disconnect("m1".to_owned()).unwrap();

    assert_eq!(connecting.join().unwrap(), Err(crate::MobileError::Cancelled));
    assert_eq!(
        events.recv_timeout(Duration::from_millis(1500)).ok(),
        None,
        "nothing follows"
    );
}
