//! What the `ngtermctl agent` integration tests share: the built binary, a short temp dir for sockets, a control
//! server on the fake main loop (answering through a handler, or through a core over a seeded library), one-shot
//! requests, and a run whose environment is the test's own (no `NGTERM_*`, `CLAUDE*` or `CODEX*` of the caller's).

use std::error::Error;
use std::ffi::OsStr;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use ngterm_core::model::WindowLibrary;
use ngterm_core::{Core, CoreConfig, CoreState};
use ngterm_host::MainLoop;
use ngterm_host::fakes::FakeMainLoop;
use ngterm_proto::{AppIdentity, Request, Response};
use ngterm_server::{ControlServer, Reply};
use serde_json::{Value, json};
use tempfile::TempDir;

pub type TestResult<T = ()> = Result<T, Box<dyn Error>>;

pub const NGTERMCTL: &str = env!("CARGO_BIN_EXE_ngtermctl");

/// A short temp dir: socket paths must stay under 104 bytes whatever `TMPDIR` is.
pub fn short_dir() -> io::Result<TempDir> {
    #[cfg(unix)]
    let root = PathBuf::from("/tmp");
    #[cfg(windows)]
    let root = std::env::temp_dir();
    tempfile::Builder::new().prefix("agh").tempdir_in(root)
}

/// A server on `socket` answering through `handler` on a fake main loop; it stops before its main loop goes.
pub struct Server<S: 'static> {
    _server: ControlServer,
    _main_loop: Arc<FakeMainLoop<S>>,
}

pub fn serve_on<S, H>(socket: &Path, main_loop: FakeMainLoop<S>, handler: H) -> TestResult<Server<S>>
where
    S: 'static,
    H: Fn(&mut S, Request) -> Reply + Send + Sync + 'static,
{
    let main_loop = Arc::new(main_loop);
    let mut server = ControlServer::new(socket);
    server.start(Arc::clone(&main_loop) as Arc<dyn MainLoop<S>>, handler)?;
    Ok(Server {
        _server: server,
        _main_loop: main_loop,
    })
}

/// A core over a seeded library, built on its main loop's thread, which it never leaves.
pub fn serve_core(socket: &Path) -> TestResult<Server<Core>> {
    let main_loop = FakeMainLoop::spawn_with(|| {
        Core::new(
            CoreConfig {
                app: AppIdentity::from_recorded("0.1.0", Some("abc1234")),
                capabilities: vec!["posix-shell".to_owned()],
            },
            CoreState {
                library: WindowLibrary::seeded("/home/u"),
                ..CoreState::default()
            },
        )
    })?;
    serve_on(socket, main_loop, |core: &mut Core, request: Request| {
        core.dispatch(&request).into()
    })
}

/// Sends one request and returns its response.
pub fn request(socket: &Path, request: &Value) -> TestResult<Response> {
    let mut stream = ngterm_server::connect(socket)?;
    stream.write_all(format!("{request}\n").as_bytes())?;
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply)?;
    Ok(serde_json::from_str(&reply)?)
}

/// The seeded session's id.
pub fn session_id(socket: &Path) -> TestResult<String> {
    let tree = request(socket, &json!({"cmd": "tree"}))?;
    let id = tree.result.ok_or("no tree")?["tree"]["workspaces"][0]["sessions"][0]["id"]
        .as_str()
        .ok_or("no session id")?
        .to_owned();
    Ok(id)
}

/// Whether `name` is a variable of the caller's that must not reach a run: a live instance's, an agent's.
pub fn foreign_var(name: &OsStr) -> bool {
    let name = name.to_string_lossy();
    ["NGTERM_", "AGTERM_", "CLAUDE", "CODEX"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

/// `program` with the caller's foreign variables removed and a home under `root`.
pub fn isolated(program: impl AsRef<OsStr>, root: &Path) -> Command {
    let mut command = Command::new(program);
    for (name, _) in std::env::vars_os() {
        if foreign_var(&name) {
            command.env_remove(name);
        }
    }
    command.env("HOME", root.join("home"));
    command
}

/// `ngterm-core`'s recorded agent fixtures.
pub fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../ngterm-core/tests/fixtures/agent_chat")
}
