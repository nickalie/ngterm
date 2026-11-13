//! The server's connections and its command line, on every OS: a connection that does not start with a hello is
//! refused with the server's welcome, and a control connection that asks no handoff (another message, or bytes that
//! are no frame) is closed, the server serving on; `server` with arguments it does not take is a usage error, and a
//! handoff's replacement whose source is not there fails. The server runs in the test process (`server::run`) on a
//! private namespace and stops by itself once its clients left; the binary runs with the test's environment without
//! the live ngterm's `NGTERM_*` and `AGTERM_*` variables.

#[expect(
    dead_code,
    reason = "the helpers serve the pane, lead, snapshot, attach and server tests alike"
)]
mod support;

use std::ffi::OsString;
use std::io::Write as _;
use std::process::{Command, ExitCode, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use ngterm_session::endpoint::{self, Endpoint, Stream};
use ngterm_session::protocol::{
    Control, ErrorCode, FramingError, HandoffRequest, Hello, PROTOCOL_VERSION, Role, read_frame, write_control,
};
use ngterm_session::server::handoff::IMPORT_FLAG;
use ngterm_session::server::lifecycle::{Graces, StopReason};
use ngterm_session::server::{self, EXIT_FAILED, EXIT_USAGE, ServerConfig};
use support::{PATIENCE, TestResult, exe};

const BUILD: &str = "server-test";

/// A server of a private namespace, run on a thread of the test.
struct Server {
    _root: tempfile::TempDir,
    endpoint: Endpoint,
    thread: Option<JoinHandle<std::io::Result<StopReason>>>,
}

impl Server {
    fn start() -> TestResult<Self> {
        // short on Unix, where a socket path holds about a hundred bytes
        #[cfg(unix)]
        let root = tempfile::Builder::new().prefix("agsv-").tempdir_in("/tmp")?;
        #[cfg(windows)]
        let root = tempfile::tempdir()?;
        let directory = root.path().join("live");
        let endpoint = Endpoint::new(&directory);
        let config = ServerConfig {
            build: BUILD.to_owned(),
            graces: Graces {
                startup: PATIENCE,
                idle: Duration::from_millis(100),
            },
            ..ServerConfig::default()
        };
        let thread = thread::spawn(move || server::run(&directory, &config));
        Ok(Self {
            _root: root,
            endpoint,
            thread: Some(thread),
        })
    }

    /// A connection, nothing sent on it yet.
    fn connect(&self) -> TestResult<Stream> {
        let deadline = Instant::now() + PATIENCE;
        loop {
            match self.endpoint.connect() {
                Ok(stream) => return Ok(stream),
                Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
                Err(err) => return Err(err.into()),
            }
        }
    }

    /// A control connection, its hello welcomed.
    fn control(&self) -> TestResult<Stream> {
        let mut stream = self.connect()?;
        write_control(&mut stream, &Control::Hello(hello(Role::Control)))?;
        let Control::Welcome(welcome) = read_frame(&mut stream)?.control()? else {
            return Err("no welcome".into());
        };
        assert_eq!(welcome.error, None);
        Ok(stream)
    }

    /// Wait for the server to stop by itself; why it did.
    fn stopped(mut self) -> TestResult<StopReason> {
        let thread = self.thread.take().ok_or("no server thread")?;
        let deadline = Instant::now() + PATIENCE;
        while !thread.is_finished() {
            assert!(Instant::now() < deadline, "the server never stopped");
            thread::sleep(Duration::from_millis(20));
        }
        Ok(thread.join().map_err(|_| "the server panicked")??)
    }
}

fn hello(role: Role) -> Hello {
    Hello {
        role,
        build: BUILD.to_owned(),
        capabilities: Vec::new(),
    }
}

/// Whether the server closed `stream` without a word more (reset, when it left unread what was sent).
fn closed(stream: &mut Stream) -> bool {
    match read_frame(stream) {
        Err(FramingError::Closed) => true,
        Err(FramingError::Io(error)) => endpoint::is_connection_closed_error(&error),
        _ => false,
    }
}

#[test]
fn a_connection_that_does_not_start_with_a_hello_is_refused_with_the_welcome() -> TestResult {
    let server = Server::start()?;
    let mut stream = server.connect()?;
    let request = HandoffRequest {
        import_exe: exe().to_owned(),
        expected_build: "other".to_owned(),
        expected_protocol: PROTOCOL_VERSION,
    };
    write_control(&mut stream, &Control::HandoffRequest(request))?;
    let Control::Welcome(welcome) = read_frame(&mut stream)?.control()? else {
        return Err("no welcome".into());
    };
    assert_eq!(welcome.build, BUILD);
    let error = welcome.error.ok_or("not refused")?;
    assert_eq!(
        (error.code, error.message.as_str()),
        (ErrorCode::UnknownOp, "a connection starts with hello.v1")
    );
    assert!(closed(&mut stream));
    drop(stream);
    assert_eq!(server.stopped()?, StopReason::Idle);
    Ok(())
}

#[test]
fn a_control_connection_that_asks_no_handoff_is_closed_and_the_server_serves_on() -> TestResult {
    let server = Server::start()?;
    let mut asked_else = server.control()?;
    write_control(&mut asked_else, &Control::Hello(hello(Role::Api)))?;
    assert!(closed(&mut asked_else), "another message closes the connection");
    let mut broken = server.control()?;
    broken.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n")?;
    assert!(closed(&mut broken), "bytes that are no frame close the connection");
    // the server serves on
    drop(server.control()?);
    drop((asked_else, broken));
    assert_eq!(server.stopped()?, StopReason::Idle);
    Ok(())
}

#[test]
fn server_arguments_it_does_not_take_are_a_usage_error() {
    let usage = ExitCode::from(EXIT_USAGE);
    assert_eq!(server::command(&[]), usage);
    let args = ["live", "--handoff-export", "channel", "token"].map(OsString::from);
    assert_eq!(server::command(&args), usage);
    let args = ["live", "extra"].map(OsString::from);
    assert_eq!(server::command(&args), usage);
}

/// A replacement started for a handoff whose source's channel is not there takes nothing and fails.
#[test]
fn a_replacement_without_its_source_fails() -> TestResult {
    let root = tempfile::tempdir()?;
    let output = Command::new(exe())
        .arg("server")
        .arg(root.path().join("live"))
        .arg(IMPORT_FLAG)
        .arg(root.path().join("no-channel"))
        .arg("token")
        .env_clear()
        .envs(std::env::vars_os().filter(|(name, _)| {
            let name = name.to_string_lossy().to_ascii_uppercase();
            !name.starts_with("NGTERM_") && !name.starts_with("AGTERM_")
        }))
        .stdin(Stdio::null())
        .output()?;
    assert_eq!(output.status.code(), Some(i32::from(EXIT_FAILED)));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.starts_with("ngterm-session server: "), "{stderr}");
    assert!(!root.path().join("live").exists(), "nothing was taken over");
    Ok(())
}
