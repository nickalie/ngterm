//! Finding and starting a namespace's server ([`connect_or_start`], [`connect_existing`], [`Client`]) against servers
//! that misbehave, on every OS: a fake server on a real endpoint of a private namespace, its greeting given by the
//! test (a refusal, a protocol it does not speak, bytes that are no frame, a connection closed unanswered) and its
//! answer to a handoff request scripted. A server of another protocol that takes no handoff, or answers one with
//! something else, fails the call with the reason and is not asked again; one gone before it could be asked, with an
//! executable that cannot start, fails with the start's error; a caller racing an attempt waits for it and reports its
//! outcome without asking again. On Unix the server executable is a script: one that exits before it is ready fails
//! the start with its status (or is started again, when it stopped by itself), and one that answers without running
//! detached is stopped.
//!
//! Every server here is the test's own fake or script: nothing reaches another one.

#[expect(
    dead_code,
    reason = "the helpers serve the pane, lead, snapshot, attach and connect tests alike"
)]
mod support;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use ngterm_session::client::{Client, ClientError};
use ngterm_session::endpoint::{Endpoint, Stream};
use ngterm_session::protocol::{
    CAPABILITY_HANDOFF_REQUEST, Control, ControlError, ErrorCode, HandoffRequest, HandoffResult, Hello,
    PROTOCOL_VERSION, Role, Welcome, read_frame, write_control,
};
use ngterm_session::server::spawn::{HandoffAttempt, StartError, StartOptions, connect_existing, connect_or_start};
use support::{PATIENCE, TestResult};

/// How the fake server answers a hello.
#[derive(Debug, Clone)]
enum Greeting {
    /// A welcome of `build` with `capabilities`, refusing the hello with `refusal` when given.
    Welcome {
        build: &'static str,
        capabilities: Vec<String>,
        refusal: Option<ErrorCode>,
    },
    /// Another control message in place of the welcome: the hello sent back.
    Other,
    /// Bytes that are no frame.
    Garbage,
    /// Nothing: the connection closed unanswered, as a stopping server's.
    Close,
}

impl Greeting {
    fn welcome(build: &'static str) -> Self {
        Self::Welcome {
            build,
            capabilities: Vec::new(),
            refusal: None,
        }
    }

    fn refusal(build: &'static str, code: ErrorCode, capabilities: Vec<String>) -> Self {
        Self::Welcome {
            build,
            capabilities,
            refusal: Some(code),
        }
    }
}

/// What the fake server does.
struct Script {
    greet: Box<dyn Fn(&Hello) -> Greeting + Send + Sync>,
    /// The answer to a handoff request on a welcomed control connection, written after the pause.
    answer: Option<(Control, Duration)>,
    /// The listener is dropped at the first connection, before it is answered: no server listens after it.
    once: bool,
}

/// What the fake server saw.
#[derive(Default)]
struct Seen {
    hellos: Vec<Hello>,
    requests: Vec<HandoffRequest>,
}

/// A fake server on a private namespace's endpoint.
struct Fake {
    _root: tempfile::TempDir,
    endpoint: Endpoint,
    seen: Arc<Mutex<Seen>>,
    /// Told of each handoff request as it comes.
    requested: Receiver<()>,
    stop: Arc<AtomicBool>,
    accept: Option<JoinHandle<()>>,
}

impl Fake {
    fn start(script: Script) -> TestResult<Self> {
        let (root, directory) = namespace()?;
        let endpoint = Endpoint::new(&directory);
        let listener = endpoint.bind()?;
        let seen = Arc::new(Mutex::new(Seen::default()));
        let (tell, requested) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let (script, recorded, stopping) = (Arc::new(script), Arc::clone(&seen), Arc::clone(&stop));
        let accept = thread::spawn(move || {
            let mut listener = Some(listener);
            while let Some(Ok(stream)) = listener.as_ref().map(ngterm_session::endpoint::Listener::accept) {
                if stopping.load(Ordering::SeqCst) {
                    return;
                }
                if script.once {
                    drop(listener.take());
                }
                let (script, recorded, tell) = (Arc::clone(&script), Arc::clone(&recorded), tell.clone());
                thread::spawn(move || serve(stream, &script, &recorded, &tell));
            }
        });
        Ok(Self {
            _root: root,
            endpoint,
            seen,
            requested,
            stop,
            accept: Some(accept),
        })
    }

    fn hellos(&self) -> Vec<Hello> {
        self.seen.lock().unwrap_or_else(PoisonError::into_inner).hellos.clone()
    }

    fn requests(&self) -> Vec<HandoffRequest> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .requests
            .clone()
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // the accept loop sees the stop with its next connection (none once it let its listener go)
        let _wake = self.endpoint.connect_timeout(Duration::from_secs(1));
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }
    }
}

/// A private namespace directory under a temporary root: short on Unix, where a socket path holds about a hundred
/// bytes.
fn namespace() -> TestResult<(tempfile::TempDir, PathBuf)> {
    #[cfg(unix)]
    let root = tempfile::Builder::new().prefix("agcn-").tempdir_in("/tmp")?;
    #[cfg(windows)]
    let root = tempfile::tempdir()?;
    let directory = root.path().join("live");
    Ok((root, directory))
}

fn serve(mut stream: Stream, script: &Script, seen: &Mutex<Seen>, requested: &Sender<()>) {
    let Ok(frame) = read_frame(&mut stream) else {
        return;
    };
    let Ok(Control::Hello(hello)) = frame.control() else {
        return;
    };
    seen.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .hellos
        .push(hello.clone());
    let (build, capabilities, refusal) = match (script.greet)(&hello) {
        Greeting::Close => return,
        Greeting::Garbage => {
            let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n");
            return;
        }
        Greeting::Other => {
            let _ = write_control(&mut stream, &Control::Hello(hello));
            return;
        }
        Greeting::Welcome {
            build,
            capabilities,
            refusal,
        } => (build, capabilities, refusal),
    };
    let welcome = Welcome {
        build: build.to_owned(),
        capabilities,
        pid: std::process::id(),
        error: refusal.map(|code| ControlError::new(code, "refused by the fake")),
    };
    if write_control(&mut stream, &Control::Welcome(welcome)).is_err() || refusal.is_some() {
        return;
    }
    if hello.role == Role::Control
        && let Ok(Control::HandoffRequest(request)) = read_frame(&mut stream).and_then(|frame| frame.control())
    {
        seen.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .requests
            .push(request);
        let _ = requested.send(());
        if let Some((answer, pause)) = &script.answer {
            thread::sleep(*pause);
            let _ = write_control(&mut stream, answer);
        }
    }
    // held open until the client goes
    while read_frame(&mut stream).is_ok() {}
}

fn hello(role: Role) -> Hello {
    Hello {
        role,
        build: "new".to_owned(),
        capabilities: Vec::new(),
    }
}

/// Options expecting the build `new` of `executable`, waiting `timeout` for a server.
fn expecting_new(executable: &Path, timeout: Duration) -> StartOptions {
    StartOptions {
        executable: executable.to_path_buf(),
        timeout,
        build: Some("new".to_owned()),
        handoff_timeout: PATIENCE,
    }
}

fn missing_executable() -> PathBuf {
    std::env::temp_dir().join("no-such-ngterm-session")
}

fn handoff_capability() -> Vec<String> {
    vec![CAPABILITY_HANDOFF_REQUEST.to_owned()]
}

#[test]
fn a_refused_hello_fails_the_connection_with_the_servers_welcome() -> TestResult {
    let fake = Fake::start(Script {
        greet: Box::new(|_| Greeting::refusal("fake", ErrorCode::UnsupportedRole, Vec::new())),
        answer: None,
        once: false,
    })?;
    let found = connect_existing(&fake.endpoint, &hello(Role::Api), PATIENCE);
    let Err(StartError::Refused { error, welcome, .. }) = &found else {
        return Err(format!("not refused: {found:?}").into());
    };
    assert_eq!(error.code, ErrorCode::UnsupportedRole);
    assert_eq!(welcome.build, "fake");
    let message = found.err().ok_or("refused")?.to_string();
    assert_eq!(
        message,
        format!(
            "the ngterm-session server at {} (build fake, pid {}) refused the connection: refused by the fake",
            fake.endpoint,
            std::process::id()
        )
    );
    // a client expecting a build is refused the same way: only another protocol is a reason to ask for a handoff
    let expecting = connect_or_start(
        &fake.endpoint,
        &hello(Role::Api),
        &expecting_new(&missing_executable(), PATIENCE),
    );
    assert!(
        matches!(&expecting, Err(StartError::Refused { error, .. }) if error.code == ErrorCode::UnsupportedRole),
        "{expecting:?}"
    );
    // an update is refused the same way, and asks nothing
    let update = Client::new(fake.endpoint.directory()).update(&expecting_new(&missing_executable(), PATIENCE));
    assert!(
        matches!(&update, Err(ClientError::Start(StartError::Refused { error, .. })) if error.code == ErrorCode::UnsupportedRole),
        "{update:?}"
    );
    assert!(fake.requests().is_empty());
    Ok(())
}

#[test]
fn a_server_that_answers_no_welcome_breaks_the_handshake() -> TestResult {
    for (greeting, broken) in [
        (Greeting::Garbage, "not an ngterm-session frame"),
        (Greeting::Other, "expected welcome.v1, got Hello("),
    ] {
        let fake = Fake::start(Script {
            greet: Box::new(move |_| greeting.clone()),
            answer: None,
            once: false,
        })?;
        let found = connect_existing(&fake.endpoint, &hello(Role::Api), PATIENCE);
        let Err(error @ StartError::Handshake { .. }) = found else {
            return Err(format!("the handshake did not break: {found:?}").into());
        };
        let message = error.to_string();
        let prefix = format!("the ngterm-session server at {} broke the handshake: ", fake.endpoint);
        assert!(message.starts_with(&prefix), "{message}");
        assert!(message.contains(broken), "{message}");
    }
    Ok(())
}

#[test]
fn a_server_that_closes_every_connection_is_waited_for_until_the_deadline() -> TestResult {
    let fake = Fake::start(Script {
        greet: Box::new(|_| Greeting::Close),
        answer: None,
        once: false,
    })?;
    let found = connect_existing(&fake.endpoint, &hello(Role::Api), Duration::from_millis(300));
    let Err(StartError::NotReady { last, .. }) = &found else {
        return Err(format!("not a wait that ran out: {found:?}").into());
    };
    assert!(last.starts_with(": the server closed the connection: "), "{last}");
    assert!(fake.hellos().len() > 1, "the server was probed again while it closed");
    Ok(())
}

/// A server of another protocol that takes no handoff requests is not asked; the call fails with why, and so does the
/// next one, from what this process remembers, without probing for a handoff again.
#[test]
fn a_server_of_another_protocol_without_handoffs_fails_the_call_and_is_not_asked() -> TestResult {
    let fake = Fake::start(Script {
        greet: Box::new(|_| Greeting::refusal("old", ErrorCode::ProtocolMismatch, Vec::new())),
        answer: None,
        once: false,
    })?;
    let options = expecting_new(&missing_executable(), PATIENCE);
    let expected = format!(
        "the ngterm-session server at {} (build old) speaks another protocol and could not hand its sessions over to \
         new: the server takes no handoff requests (no {CAPABILITY_HANDOFF_REQUEST})",
        fake.endpoint
    );
    let first = connect_or_start(&fake.endpoint, &hello(Role::Api), &options);
    assert!(matches!(&first, Err(StartError::HandoffFailed { .. })), "{first:?}");
    assert_eq!(first.err().ok_or("failed")?.to_string(), expected);
    let probes = fake.hellos().len();
    let again = connect_or_start(&fake.endpoint, &hello(Role::Api), &options);
    assert_eq!(again.err().ok_or("failed again")?.to_string(), expected);
    assert_eq!(fake.hellos().len(), probes + 1, "only the probe, no handoff attempt");
    // an update finds the same, and asks nothing either
    let update = Client::new(fake.endpoint.directory()).update(&options);
    assert!(
        matches!(&update, Err(ClientError::Start(StartError::HandoffFailed { .. }))),
        "{update:?}"
    );
    assert_eq!(update.err().ok_or("update failed")?.to_string(), expected);
    assert!(fake.hellos().iter().all(|hello| hello.role != Role::Control));
    assert!(fake.requests().is_empty());
    Ok(())
}

/// A server of another protocol is asked for its handoff on a control connection; an answer that is no handoff
/// result, or a refusal of the control connection itself, is the reason the call fails with.
#[test]
fn a_handoff_request_answered_with_something_else_fails_with_that_answer() -> TestResult {
    let fake = Fake::start(Script {
        greet: Box::new(|hello| match hello.role {
            Role::Control => Greeting::welcome("old"),
            _ => Greeting::refusal("old", ErrorCode::ProtocolMismatch, handoff_capability()),
        }),
        answer: Some((Control::Hello(hello(Role::Api)), Duration::ZERO)),
        once: false,
    })?;
    let executable = missing_executable();
    let found = connect_or_start(
        &fake.endpoint,
        &hello(Role::Attach),
        &expecting_new(&executable, PATIENCE),
    );
    let Err(StartError::HandoffFailed {
        from, build, reason, ..
    }) = found
    else {
        return Err(format!("the handoff did not fail: {found:?}").into());
    };
    assert_eq!((from.as_str(), build.as_str()), ("old", "new"));
    assert!(
        reason.starts_with("the server answered the handoff request with Hello("),
        "{reason}"
    );
    assert_eq!(
        fake.requests(),
        [HandoffRequest {
            import_exe: executable.to_string_lossy().into_owned(),
            expected_build: "new".to_owned(),
            expected_protocol: PROTOCOL_VERSION,
        }]
    );

    let refusing = Fake::start(Script {
        greet: Box::new(|_| Greeting::refusal("old", ErrorCode::ProtocolMismatch, handoff_capability())),
        answer: None,
        once: false,
    })?;
    let found = connect_or_start(
        &refusing.endpoint,
        &hello(Role::Api),
        &expecting_new(&executable, PATIENCE),
    );
    let Err(StartError::HandoffFailed { reason, .. }) = found else {
        return Err(format!("the handoff did not fail: {found:?}").into());
    };
    assert_eq!(
        reason,
        format!(
            "the ngterm-session server at {} (build old, pid {}) refused the connection: refused by the fake",
            refusing.endpoint,
            std::process::id()
        )
    );
    assert!(refusing.requests().is_empty());
    Ok(())
}

/// A server gone before it could be asked for its handoff leaves nothing to ask, and the start that follows fails
/// with the start's own error when the executable cannot run.
#[test]
fn a_server_gone_before_its_handoff_is_asked_leaves_a_start_that_fails() -> TestResult {
    let fake = Fake::start(Script {
        greet: Box::new(|_| Greeting::refusal("old", ErrorCode::ProtocolMismatch, handoff_capability())),
        answer: None,
        once: true,
    })?;
    let executable = missing_executable();
    let found = connect_or_start(&fake.endpoint, &hello(Role::Api), &expecting_new(&executable, PATIENCE));
    let Err(error @ StartError::Spawn { .. }) = found else {
        return Err(format!("the start did not fail: {found:?}").into());
    };
    let message = error.to_string();
    let prefix = format!("could not start the ngterm-session server {}: ", executable.display());
    assert!(message.starts_with(&prefix), "{message}");
    assert!(fake.requests().is_empty());
    Ok(())
}

/// Of two callers finding a server of another build, the second waits for the first one's attempt and reports its
/// outcome: the server is asked once. So for a server of another protocol, which fails both calls.
#[test]
fn a_caller_racing_a_handoff_attempt_waits_for_it_and_asks_nothing() -> TestResult {
    let refused = || {
        Some((
            Control::HandoffResult(HandoffResult(Err(ControlError::new(
                ErrorCode::HandoffFailed,
                "refused by the fake",
            )))),
            Duration::from_millis(300),
        ))
    };
    let failed = HandoffAttempt::Failed {
        from: "old".to_owned(),
        reason: "refused by the fake".to_owned(),
    };
    let of_another_build = Fake::start(Script {
        greet: Box::new(|_| Greeting::Welcome {
            build: "old",
            capabilities: handoff_capability(),
            refusal: None,
        }),
        answer: refused(),
        once: false,
    })?;
    let (first, second) = race(&of_another_build)?;
    assert_eq!(first, Ok(Some(failed.clone())));
    assert_eq!(second, Ok(Some(failed)));
    assert_eq!(of_another_build.requests().len(), 1, "asked once");

    let of_another_protocol = Fake::start(Script {
        greet: Box::new(|hello| match hello.role {
            Role::Control => Greeting::welcome("old"),
            _ => Greeting::refusal("old", ErrorCode::ProtocolMismatch, handoff_capability()),
        }),
        answer: refused(),
        once: false,
    })?;
    let (first, second) = race(&of_another_protocol)?;
    let expected = format!(
        "the ngterm-session server at {} (build old) speaks another protocol and could not hand its sessions over to \
         new: refused by the fake",
        of_another_protocol.endpoint
    );
    assert_eq!(first, Err(expected.clone()));
    assert_eq!(second, Err(expected));
    assert_eq!(of_another_protocol.requests().len(), 1, "asked once");
    Ok(())
}

/// What a call expecting the build `new` came to.
type Outcome = Result<Option<HandoffAttempt>, String>;

/// Two callers expecting the build `new` of `fake`'s server, the second starting once the first one's handoff request
/// reached it; what each came to.
fn race(fake: &Fake) -> TestResult<(Outcome, Outcome)> {
    let options = expecting_new(&missing_executable(), PATIENCE);
    let caller = || {
        let (endpoint, options) = (fake.endpoint.clone(), options.clone());
        thread::spawn(move || {
            connect_or_start(&endpoint, &hello(Role::Api), &options)
                .map(|connected| connected.handoff)
                .map_err(|error| error.to_string())
        })
    };
    let first = caller();
    fake.requested.recv_timeout(PATIENCE)?;
    let second = caller();
    Ok((
        first.join().map_err(|_| "the first caller panicked")?,
        second.join().map_err(|_| "the second caller panicked")?,
    ))
}

#[test]
fn a_clients_hello_names_its_build() -> TestResult {
    let fake = Fake::start(Script {
        greet: Box::new(|_| Greeting::welcome("fake")),
        answer: None,
        once: false,
    })?;
    let client = Client::new(fake.endpoint.directory())
        .with_timeout(PATIENCE)
        .with_build("mine");
    assert_eq!(client.timeout(), PATIENCE);
    let api = client.api()?.ok_or("no server")?;
    assert_eq!(api.welcome.build, "fake");
    let hellos = fake.hellos();
    assert_eq!(
        hellos
            .iter()
            .map(|hello| (hello.role, hello.build.as_str()))
            .collect::<Vec<_>>(),
        [(Role::Api, "mine")]
    );
    assert_eq!(
        Client::new(fake.endpoint.directory()).timeout(),
        ngterm_session::client::DEFAULT_TIMEOUT
    );
    Ok(())
}

/// A socket path the OS cannot take is no missing server: the connection fails with the reason.
#[cfg(unix)]
#[test]
fn an_endpoint_the_os_cannot_reach_fails_the_connection() -> TestResult {
    let root = tempfile::Builder::new().prefix("agcn-").tempdir_in("/tmp")?;
    let endpoint = Endpoint::new(root.path().join("d".repeat(200)));
    let found = connect_existing(&endpoint, &hello(Role::Api), PATIENCE);
    let Err(error @ StartError::Connect { .. }) = found else {
        return Err(format!("not a failed connection: {found:?}").into());
    };
    let message = error.to_string();
    let prefix = format!("could not connect to the ngterm-session server at {endpoint}: ");
    assert!(message.starts_with(&prefix), "{message}");
    Ok(())
}

/// A server executable for the tests below: a shell script, copied into place and made executable by a child process
/// (a script written from here could be held open by a child another test's thread forks meanwhile, until its exec,
/// and starting it then fails with ETXTBSY).
#[cfg(unix)]
fn script_executable(root: &Path, body: &str) -> TestResult<PathBuf> {
    let source = root.join("script.txt");
    std::fs::write(&source, format!("#!/bin/sh\n{body}\n"))?;
    let executable = root.join("server.sh");
    let status = std::process::Command::new("/bin/sh")
        .args(["-c", "cp \"$1\" \"$2\" && chmod 755 \"$2\"", "sh"])
        .arg(&source)
        .arg(&executable)
        .status()?;
    assert!(status.success(), "{status}");
    Ok(executable)
}

/// A server started here that exits before it is ready fails the start with its status, unless it stopped by itself,
/// when it is started again until the wait runs out.
#[cfg(unix)]
#[test]
fn a_server_that_exits_before_it_is_ready_fails_the_start_or_is_started_again() -> TestResult {
    let (root, directory) = namespace()?;
    let endpoint = Endpoint::new(&directory);
    let options = |executable: PathBuf, timeout: Duration| StartOptions {
        executable,
        timeout,
        build: None,
        handoff_timeout: PATIENCE,
    };
    let failing = script_executable(root.path(), "exit 1")?;
    let found = connect_or_start(&endpoint, &hello(Role::Api), &options(failing, PATIENCE));
    let Err(error @ StartError::Exited { .. }) = found else {
        return Err(format!("the start did not fail: {found:?}").into());
    };
    assert_eq!(
        error.to_string(),
        format!(
            "the ngterm-session server started for {endpoint} exited before it was ready (exit status: 1); run \
             `ngterm-session server <directory>` to see why"
        )
    );

    let stopping = script_executable(root.path(), "exit 0")?;
    let found = connect_or_start(
        &endpoint,
        &hello(Role::Api),
        &options(stopping, Duration::from_millis(500)),
    );
    let Err(StartError::NotReady { last, .. }) = &found else {
        return Err(format!("the wait did not run out: {found:?}").into());
    };
    assert!(!last.is_empty(), "{found:?}");
    Ok(())
}

/// A server started here that answers without running detached from its launcher would end with it: it is stopped,
/// and the start fails.
#[cfg(unix)]
#[test]
fn a_server_started_here_that_is_not_detached_is_stopped() -> TestResult {
    let (root, directory) = namespace()?;
    std::fs::create_dir_all(&directory)?;
    // the started server writes its pid where the test reads it, then waits; the test answers in its place
    let executable = script_executable(root.path(), "echo $$ > \"$2/pid\"\nexec sleep 60")?;
    let endpoint = Endpoint::new(&directory);
    let options = StartOptions {
        executable,
        timeout: PATIENCE,
        build: None,
        handoff_timeout: PATIENCE,
    };
    let starting = {
        let endpoint = endpoint.clone();
        thread::spawn(move || connect_or_start(&endpoint, &hello(Role::Api), &options).map(drop))
    };
    let pid_file = directory.join("pid");
    let pid: u32 = support::wait_for(
        "the started server's pid",
        || {
            std::fs::read_to_string(&pid_file)
                .ok()
                .and_then(|pid| pid.trim().parse().ok())
        },
        Option::is_some,
    )
    .ok_or("no pid")?;
    // a server of that pid that does not report itself detached
    let listener = endpoint.bind()?;
    let answering = thread::spawn(move || {
        let mut stream = listener.accept().unwrap();
        read_frame(&mut stream).unwrap();
        let welcome = Welcome {
            build: "script".to_owned(),
            capabilities: Vec::new(),
            pid,
            error: None,
        };
        write_control(&mut stream, &Control::Welcome(welcome)).unwrap();
        // held until the client goes
        while read_frame(&mut stream).is_ok() {}
    });
    let found = starting.join().map_err(|_| "the start panicked")?;
    let Err(error @ StartError::NotDetached { .. }) = found else {
        return Err(format!("the start did not fail: {found:?}").into());
    };
    assert_eq!(
        error.to_string(),
        format!(
            "the ngterm-session server started for {endpoint} (pid {pid}, {}) is not detached from its launcher and \
             would end with it; it was stopped",
            ngterm_session::server::spawn::SpawnPath::Detached
        )
    );
    support::wait_for_exit(pid, "the server that is not detached");
    answering.join().map_err(|_| "the answer panicked")?;
    Ok(())
}
