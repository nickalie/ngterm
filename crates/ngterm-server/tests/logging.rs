//! What the control server logs, through a subscriber of this test binary's own that records every event: the
//! operator's view of what the wire cannot show, such as the panic behind a handler's `internal error`.

use std::error::Error;
use std::fmt::{self, Write as _};
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, Once, PoisonError};
use std::time::Duration;

use ngterm_host::MainLoop;
use ngterm_host::fakes::FakeMainLoop;
use ngterm_proto::{Command, Request, Response};
use ngterm_server::{ControlServer, Reply};
use tempfile::TempDir;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Metadata, Subscriber};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

const CLIENT_TIMEOUT: Duration = Duration::from_secs(20);

/// Every event logged in this binary, as `LEVEL name=value ...`, the message first.
static EVENTS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn events() -> MutexGuard<'static, Vec<String>> {
    EVENTS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Records every event into [`EVENTS`]; spans are not used by the server and get one shared id.
struct Recorder;

impl Subscriber for Recorder {
    fn enabled(&self, _: &Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, _: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }

    fn record(&self, _: &Id, _: &Record<'_>) {}

    fn record_follows_from(&self, _: &Id, _: &Id) {}

    fn event(&self, event: &Event<'_>) {
        let mut line = Fields(event.metadata().level().to_string());
        event.record(&mut line);
        events().push(line.0);
    }

    fn enter(&self, _: &Id) {}

    fn exit(&self, _: &Id) {}
}

struct Fields(String);

impl Visit for Fields {
    fn record_str(&mut self, field: &Field, value: &str) {
        write!(self.0, " {}={value}", field.name()).ok();
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        write!(self.0, " {}={value:?}", field.name()).ok();
    }
}

fn record_events() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        tracing::subscriber::set_global_default(Recorder).ok();
    });
}

/// Whether an event was logged that holds every one of `parts`.
fn logged(parts: &[&str]) -> bool {
    events()
        .iter()
        .any(|event| parts.iter().all(|part| event.contains(part)))
}

/// A short temp dir: socket paths must stay under 104 bytes whatever `TMPDIR` is (Unix; Windows has no such limit).
fn short_dir() -> io::Result<TempDir> {
    let builder = tempfile::Builder::new().prefix("ngt").clone();
    if cfg!(unix) {
        builder.tempdir_in("/tmp")
    } else {
        builder.tempdir()
    }
}

fn answer(socket: &Path, line: &[u8]) -> TestResult<Response> {
    let mut stream = ngterm_server::connect(socket)?;
    stream.set_read_timeout(Some(CLIENT_TIMEOUT))?;
    stream.write_all(line)?;
    stream.write_all(b"\n")?;
    let mut response = String::new();
    BufReader::new(&stream).read_line(&mut response)?;
    Ok(serde_json::from_str(&response)?)
}

/// A handler panicking with a `&str`, a `String` or a payload that is neither, by command.
#[expect(clippy::panic, reason = "its panics are what the test logs")]
fn panicking(request: &Request) -> Reply {
    match request.cmd {
        Command::Tree => panic!("tree blew up"),
        Command::Version => panic!("version blew up: {}", 42),
        _ => std::panic::panic_any(7_u32),
    }
}

#[test]
fn a_handler_panic_is_logged_with_its_message() -> TestResult {
    record_events();
    let dir = short_dir()?;
    let socket = dir.path().join("ngterm.sock");
    let main_loop: Arc<dyn MainLoop<()>> = Arc::new(FakeMainLoop::spawn(())?);
    let mut server = ControlServer::new(&socket);
    server.start(main_loop, |(): &mut (), request: Request| panicking(&request))?;

    for (line, cmd, panic) in [
        (&br#"{"cmd":"tree"}"#[..], "tree", "tree blew up"),
        (br#"{"cmd":"version"}"#, "version", "version blew up: 42"),
        (br#"{"cmd":"dashboard"}"#, "dashboard", "non-string panic payload"),
    ] {
        assert_eq!(
            answer(&socket, line)?,
            Response::failure(format!("internal error handling {cmd}"))
        );
        let fields = [
            "ERROR message=control handler panicked".to_owned(),
            format!("cmd={cmd}"),
            format!("panic={panic}"),
        ];
        let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
        assert!(logged(&fields), "{fields:?} not in {:?}", *events());
    }
    Ok(())
}

#[test]
fn a_refused_instance_logs_why_it_does_not_bind() -> TestResult {
    record_events();
    let dir = short_dir()?;
    let socket = dir.path().join("ngterm.sock");
    let _owner = ControlServer::new(&socket);
    assert!(ControlServer::new(&socket).is_refused());
    let warning = format!(
        "WARN message=control socket {} is already served by another instance — not binding",
        socket.display()
    );
    assert!(logged(&[&warning]), "{warning:?} not in {:?}", *events());
    Ok(())
}
