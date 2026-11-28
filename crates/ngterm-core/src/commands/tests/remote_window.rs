//! Remote windows (upstream's `RemoteCatalogClientTests`, `RemoteCatalogMirrorTests` and `WindowLibraryRemoteTests`):
//! `window.remote.open`, the `version` probe, the catalog stream and the rows it builds, over a fake ssh runner and a
//! fake stream transport.

use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ngterm_proto::command::{LiveCmd, RecentCmd, SessionCmd, WindowCmd, WorkspaceCmd};
use ngterm_proto::{Args, Command, Request, Response};
use serde_json::{Value, json};
use uuid::Uuid;

use super::viewer::{Link, Transport};
use super::{Fixture, answered_id, args, wire};
use crate::Dispatched;
use crate::app_actions::ActionOrigin;
use crate::catalog::fixture::{self as catalog, pane, session, session_with, snapshot, workspace};
use crate::catalog::frames::{self, CatalogChange, CatalogFrame, CatalogScratch, CatalogSnapshot, Lifecycle};
use crate::keymap::BuiltinAction;
use crate::model::{AgentIndicator, AgentStatus, AppStore, Session, SplitAxis};
use crate::palette::{PaletteAction, PaletteMode};
use crate::palette_catalog::PaletteCommand;
use crate::pane_env::StatusPane;
use crate::persist::LibraryPersistence;
use crate::remote::catalog::CatalogState;
use crate::remote::catalog::{RemoteWindowViewState, SessionView};
use crate::remote::tree::{RemoteCommandResult, RemoteCommandRunner};
use crate::remote::viewer_run::readable;
use crate::sidebar_actions::SidebarAction;

/// Answers `version` with `version`, an operation sent with an id with an applied receipt at the next revision (the
/// created row's id for a new session), and records every run. With `lose` set, operations and lookups get no answer;
/// with `refuse` set, an operation gets a refused receipt with that error.
struct Runner {
    version: Mutex<RemoteCommandResult>,
    runs: Mutex<Vec<Vec<String>>>,
    rev: Mutex<i64>,
    created: Mutex<Option<Uuid>>,
    lose: AtomicBool,
    refuse: Mutex<Option<String>>,
}

impl Runner {
    /// The remote lines run so far, quotes left out.
    fn lines(&self) -> Vec<String> {
        let runs = self.runs.lock().unwrap();
        runs.iter()
            .map(|argv| {
                argv.last()
                    .map_or_else(String::new, |line| line.replace(['\'', '\\'], ""))
            })
            .collect()
    }

    /// The lines that ran an operation.
    fn operations(&self) -> Vec<String> {
        self.lines()
            .into_iter()
            .filter(|line| line.contains("--op-id "))
            .collect()
    }

    /// The lines that asked about an operation.
    fn lookups(&self) -> Vec<String> {
        self.lines()
            .into_iter()
            .filter(|line| line.contains(" catalog op "))
            .collect()
    }
}

impl RemoteCommandRunner for Runner {
    fn run(&self, argv: &[String], _deadline: Duration) -> RemoteCommandResult {
        self.runs.lock().unwrap().push(argv.to_vec());
        let line = argv
            .last()
            .map_or_else(String::new, |line| line.replace(['\'', '\\'], ""));
        let operation = line.contains("--op-id ") || line.contains(" catalog op ");
        if !operation {
            return self.version.lock().unwrap().clone();
        }
        if self.lose.load(Ordering::SeqCst) {
            return answer(&json!(null));
        }
        let at = line.find("--op-id ").map(|at| at + 8).unwrap_or_default();
        let op_id: String = line[at..].chars().take(36).collect();
        let mut rev = self.rev.lock().unwrap();
        *rev += 1;
        if let Some(error) = self.refuse.lock().unwrap().clone() {
            return answer(&json!({"ok": false, "error": error, "result": {
                "receipt": {"opID": op_id, "epoch": "e1", "outcome": "refused", "rev": *rev, "error": error}
            }}));
        }
        let created = line
            .contains("session new")
            .then(|| *self.created.lock().unwrap())
            .flatten();
        answer(&json!({"ok": true, "result": {
            "id": created.map(wire),
            "receipt": {"opID": op_id, "epoch": "e1", "outcome": "applied", "rev": *rev}
        }}))
    }
}

fn answer(stdout: &Value) -> RemoteCommandResult {
    RemoteCommandResult {
        status: 0,
        stdout: stdout.to_string(),
        stderr: String::new(),
    }
}

fn server_version() -> Value {
    json!({"ok": true, "result": {
        "app": {"version": "0.9.0"}, "headless": true,
        "versions": {"server": "1.2.3", "protocol": 1, "catalog": 1},
        "conformance": {"parityLevel": "0.42.3", "capabilities": []}
    }})
}

fn app_version() -> Value {
    json!({"ok": true, "result": {
        "app": {"version": "0.9.0"},
        "conformance": {"parityLevel": "0.42.3", "capabilities": []}
    }})
}

/// A viewer with a local window, its runner and its stream transport.
struct Remote {
    fixture: Fixture,
    runner: Arc<Runner>,
    transport: Rc<Transport>,
    /// The remote window `window.remote.open` answered.
    window: Uuid,
}

impl Remote {
    fn new(version: &Value) -> Self {
        let mut fixture = Fixture::new(0);
        let runner = Arc::new(Runner {
            version: Mutex::new(answer(version)),
            runs: Mutex::new(Vec::new()),
            rev: Mutex::new(0),
            created: Mutex::new(None),
            lose: AtomicBool::new(false),
            refuse: Mutex::new(None),
        });
        fixture.core.set_remote_runner(runner.clone());
        let transport = Rc::new(Transport::default());
        fixture.core.set_presentation_transport(transport.clone());
        Self {
            fixture,
            runner,
            transport,
            window: Uuid::nil(),
        }
    }

    /// A viewer whose `window.remote.open box` was answered and whose probe settled.
    fn opened(version: &Value) -> Self {
        let mut remote = Self::new(version);
        let response = remote.open("box");
        remote.window = answered_id(&response).unwrap_or_else(|| panic!("{response:?}"));
        remote.settle();
        remote
    }

    fn open(&mut self, host: &str) -> Response {
        self.fixture.send(
            Command::Window(WindowCmd::RemoteOpen),
            None,
            args(|a| a.host = Some(host.to_owned())),
        )
    }

    /// Ticks until the probe answered.
    fn settle(&mut self) {
        for _ in 0..400 {
            self.fixture.core.remote_tick(Instant::now());
            self.fixture.core.commit();
            if !self.fixture.core.catalog_wake().0 {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("the probe never answered");
    }

    fn links(&self) -> Vec<Link> {
        self.transport.0.borrow().clone()
    }

    /// The window's catalog streams, oldest first; the rows' presentation streams are left out.
    fn streams(&self) -> Vec<Link> {
        self.links()
            .into_iter()
            .filter(|link| link.0.borrow().key.is_some_and(|key| key.session == self.window))
            .collect()
    }

    fn link(&self) -> Link {
        self.streams().last().cloned().expect("a stream")
    }

    fn state(&self) -> CatalogState {
        self.fixture
            .core
            .remote_window_state(self.window)
            .cloned()
            .expect("a connection")
    }

    /// Delivers `frame` on the current stream.
    fn frame(&mut self, frame: &CatalogFrame) {
        let link = self.link();
        self.line_on(&link, &frames::encode(frame).unwrap());
    }

    fn line_on(&mut self, stream: &Link, text: &[u8]) {
        let key = stream.0.borrow().key.unwrap();
        let text = text.strip_suffix(b"\n").unwrap_or(text);
        self.fixture.core.viewer_line(key, text);
        self.fixture.core.commit();
    }

    fn snapshot(&mut self, snapshot: CatalogSnapshot) {
        self.frame(&CatalogFrame::Snapshot(snapshot));
    }

    fn delta(&mut self, rev: i64, change: CatalogChange) {
        self.frame(&CatalogFrame::Delta {
            epoch: "e1".to_owned(),
            rev,
            change,
        });
    }

    fn close(&mut self, link: &Link, reason: &str) {
        let key = link.0.borrow().key.unwrap();
        self.fixture.core.viewer_closed(key, reason);
        self.fixture.core.commit();
    }

    fn store(&self) -> &AppStore {
        self.fixture
            .core
            .library()
            .store(self.window)
            .expect("the remote window")
    }

    fn row(&self, id: Uuid) -> &Session {
        self.store().session(id).expect("the row")
    }

    fn rows(&self) -> Vec<Uuid> {
        self.store().sessions().map(Session::id).collect()
    }

    fn tree(&mut self) -> Value {
        let window = wire(self.window);
        let response = self
            .fixture
            .send(Command::Tree, None, args(|a| a.window = Some(window)));
        response.result.expect("a tree")["tree"].clone()
    }

    fn window_node(&mut self) -> Value {
        let response = self
            .fixture
            .send(Command::Window(WindowCmd::List), None, Args::default());
        let windows = response.result.expect("a list")["windows"].clone();
        windows
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["id"] == json!(wire(self.window)))
            .cloned()
            .expect("the remote window")
    }
}

// MARK: - opening

#[test]
fn opening_a_remote_window_needs_a_host_ssh_can_be_given() {
    let mut remote = Remote::new(&server_version());
    let windows = remote.fixture.core.library().windows().len();
    let missing = remote
        .fixture
        .send(Command::Window(WindowCmd::RemoteOpen), None, Args::default());
    assert_eq!(missing.error.as_deref(), Some("window.remote.open requires a host"));
    let bad = remote.open("-oProxyCommand=x");
    assert_eq!(
        bad.error.as_deref(),
        Some("window.remote.open: not a host ssh can be given")
    );
    assert_eq!(remote.fixture.core.library().windows().len(), windows);
    assert!(remote.runner.runs.lock().unwrap().is_empty());
}

#[test]
fn a_server_that_advertises_a_catalog_gets_the_stream_opened_with_hello() {
    let remote = Remote::opened(&server_version());
    let runs = remote.runner.runs.lock().unwrap().clone();
    assert_eq!(runs.len(), 1);
    assert!(runs[0].last().unwrap().contains("version"), "{runs:?}");
    let link = remote.link();
    let link = link.0.borrow();
    assert_eq!(
        link.argv.last(),
        Some(&crate::remote::ssh::catalog_tail(
            crate::remote::origin::OriginShell::Posix
        ))
    );
    assert_eq!(link.key.unwrap().session, remote.window);
    assert_eq!(link.sent, vec![json!({"kind": "hello", "hello": {"version": 1}})]);
    assert_eq!(remote.state(), CatalogState::Connecting);
}

#[test]
fn a_remote_window_is_never_seeded_and_is_frontmost() {
    let remote = Remote::opened(&server_version());
    let library = remote.fixture.core.library();
    assert!(library.is_open(remote.window));
    assert_eq!(library.active_window_id(), Some(remote.window));
    assert!(remote.store().workspaces().is_empty());
    assert_eq!(library.windows().last().map(|info| info.name.as_str()), Some("box"));
}

#[test]
fn opening_a_host_already_open_answers_its_window() {
    let mut remote = Remote::opened(&server_version());
    remote.snapshot(snapshot(vec![workspace(Uuid::new_v4(), "w", vec![session("a")])]));
    let windows = remote.fixture.core.library().windows().len();
    let again = remote.open("box");
    assert_eq!(answered_id(&again), Some(remote.window));
    assert_eq!(remote.fixture.core.library().windows().len(), windows);
    assert_eq!(remote.streams().len(), 1, "the open stream is kept");
}

#[test]
fn an_origin_with_no_catalog_is_refused_and_no_stream_is_opened() {
    let mut remote = Remote::opened(&app_version());
    assert!(remote.streams().is_empty());
    let reason = "box runs ngterm 0.9.0 without a catalog, an app with windows rather than ngterm-headless; a remote \
                  window needs ngterm-headless";
    assert_eq!(remote.state(), CatalogState::Refused(reason.to_owned()));
    let node = remote.tree()["remote"].clone();
    assert_eq!(node, json!({"host": "box", "state": "refused", "reason": reason}));
    // refused is final: no backoff retries it
    remote
        .fixture
        .core
        .remote_tick(Instant::now() + Duration::from_secs(120));
    assert_eq!(remote.runner.runs.lock().unwrap().len(), 1);
}

#[test]
fn a_newer_catalog_version_is_refused_naming_both() {
    let mut version = server_version();
    version["result"]["versions"]["catalog"] = json!(9);
    let remote = Remote::opened(&version);
    assert_eq!(
        remote.state(),
        CatalogState::Refused("box speaks catalog 9, this Mac speaks catalog 1".to_owned())
    );
}

#[test]
fn an_unreachable_server_stays_disconnected_and_retries_after_the_backoff() {
    let mut remote = Remote::new(&server_version());
    *remote.runner.version.lock().unwrap() = RemoteCommandResult {
        status: 255,
        stdout: String::new(),
        stderr: "ssh: connect to host box port 22: Connection refused".to_owned(),
    };
    remote.window = answered_id(&remote.open("box")).unwrap();
    remote.settle();
    assert_eq!(
        remote.state(),
        CatalogState::Disconnected("box did not answer".to_owned())
    );
    assert!(remote.streams().is_empty());
    *remote.runner.version.lock().unwrap() = answer(&server_version());
    remote.fixture.core.remote_tick(Instant::now() + Duration::from_secs(2));
    remote.settle();
    assert_eq!(remote.runner.runs.lock().unwrap().len(), 2);
    assert_eq!(remote.streams().len(), 1, "the retry's probe opened the stream");
}

// MARK: - the stream

#[test]
fn the_snapshot_builds_every_row_with_its_catalog_identities_and_attaches() {
    let mut remote = Remote::opened(&server_version());
    let (left, right) = (pane(), pane());
    let split = session_with(Uuid::new_v4(), "build", vec![left.clone(), right.clone()]);
    let plain = session("logs");
    let (w1, w2) = (Uuid::new_v4(), Uuid::new_v4());
    remote.snapshot(snapshot(vec![
        workspace(w1, "dev", vec![split.clone()]),
        workspace(w2, "ops", vec![plain.clone()]),
    ]));
    assert_eq!(remote.state(), CatalogState::Connected);
    let store = remote.store();
    let workspaces: Vec<(Uuid, &str)> = store
        .workspaces()
        .iter()
        .map(|workspace| (workspace.id(), workspace.name.as_str()))
        .collect();
    assert_eq!(workspaces, vec![(w1, "dev"), (w2, "ops")]);
    assert_eq!(remote.rows(), vec![split.id(), plain.id()]);
    let row = remote.row(split.id());
    assert_eq!(row.pane_identity, left.identity);
    assert_eq!(row.split_pane_identity, Some(right.identity));
    assert!(row.split.shown);
    assert_eq!(row.remote_host(), Some("box"));
    assert_eq!(row.remote_session_id(), Some(wire(split.id()).as_str()));
    for (command, daemon) in [
        (&row.launch.command, &left.daemon),
        (&row.split_launch.command, &right.daemon),
    ] {
        let command = readable(command.as_deref().unwrap());
        assert!(command.contains(daemon.as_str()), "{command}");
        assert!(command.contains("ngterm-session"), "{command}");
    }
    let binding = &row.remote_presentation().unwrap().binding;
    assert!(binding.catalog_owned && binding.origin_headless);
    assert_eq!(binding.local_pane(left.identity), Some(left.identity));
    assert_eq!(
        store.selected_session_id(),
        Some(split.id()),
        "a fresh window selects its first row"
    );
}

#[test]
fn tree_and_window_list_report_the_server_its_catalog_and_the_connection() {
    let mut remote = Remote::opened(&server_version());
    assert_eq!(remote.tree()["remote"], json!({"host": "box", "state": "connecting"}));
    remote.snapshot(snapshot(vec![workspace(Uuid::new_v4(), "w", vec![session("a")])]));
    let connected = json!({"host": "box", "catalogID": wire(catalog::CATALOG), "state": "connected"});
    assert_eq!(remote.tree()["remote"], connected);
    assert_eq!(remote.window_node()["remote"], connected);
}

#[test]
fn a_ping_is_answered_with_an_ack() {
    let mut remote = Remote::opened(&server_version());
    remote.frame(&CatalogFrame::Ping);
    assert_eq!(remote.link().0.borrow().sent.last(), Some(&json!({"kind": "ack"})));
}

#[test]
fn a_removed_session_loses_its_row_and_an_added_one_gains_one() {
    let mut remote = Remote::opened(&server_version());
    let (a, b) = (session("a"), session("b"));
    let w = Uuid::new_v4();
    remote.snapshot(snapshot(vec![workspace(w, "w", vec![a.clone()])]));
    remote.delta(
        1,
        CatalogChange::SessionAdded {
            session: b.clone(),
            workspace: w,
            index: 1,
        },
    );
    assert_eq!(remote.rows(), vec![a.id(), b.id()]);
    remote.delta(2, CatalogChange::SessionRemoved(a.id()));
    assert_eq!(remote.rows(), vec![b.id()]);
    assert!(
        remote.fixture.core.library().recent_closed_items().is_empty(),
        "a row the server removed is no close of this machine's"
    );
}

#[test]
fn a_rename_and_a_reorder_move_the_rows_in_place() {
    let mut remote = Remote::opened(&server_version());
    let (a, b) = (session("a"), session("b"));
    let w = Uuid::new_v4();
    remote.snapshot(snapshot(vec![workspace(w, "w", vec![a.clone(), b.clone()])]));
    let mut renamed = a.fields.clone();
    renamed.custom_name = Some("named".to_owned());
    remote.delta(1, CatalogChange::SessionChanged(renamed));
    remote.delta(
        2,
        CatalogChange::SessionOrder {
            ids: vec![b.id(), a.id()],
            workspace: w,
        },
    );
    assert_eq!(remote.rows(), vec![b.id(), a.id()]);
    assert_eq!(remote.row(a.id()).custom_name.as_deref(), Some("named"));
}

#[test]
fn an_unnamed_row_follows_its_title_and_a_named_one_keeps_its_name() {
    let mut remote = Remote::opened(&server_version());
    let (mut unnamed, mut named) = (session("zsh"), session("build"));
    unnamed.fields.custom_name = Some(String::new());
    named.fields.custom_name = Some("build".to_owned());
    let older = session("from an older server");
    remote.snapshot(snapshot(vec![workspace(
        Uuid::new_v4(),
        "w",
        vec![unnamed.clone(), named.clone(), older.clone()],
    )]));
    assert_eq!(remote.row(unnamed.id()).custom_name, None);
    assert_eq!(remote.row(named.id()).custom_name.as_deref(), Some("build"));
    assert_eq!(
        remote.row(older.id()).custom_name.as_deref(),
        Some("from an older server"),
        "a server without customName keeps the catalog name over the title"
    );
}

#[test]
fn a_gap_or_another_epoch_asks_for_a_snapshot_and_applies_nothing() {
    let mut remote = Remote::opened(&server_version());
    let a = session("a");
    let w = Uuid::new_v4();
    remote.snapshot(snapshot(vec![workspace(w, "w", vec![a.clone()])]));
    let first = remote.link();
    remote.delta(5, CatalogChange::SessionRemoved(a.id()));
    assert_eq!(remote.rows(), vec![a.id()], "a gap applies nothing");
    assert!(first.0.borrow().stopped, "the stream is replaced");
    assert_eq!(remote.streams().len(), 2);
    assert_eq!(remote.state(), CatalogState::Connecting);
    remote.snapshot(snapshot(vec![workspace(w, "w", vec![a.clone()])]));
    remote.frame(&CatalogFrame::Delta {
        epoch: "e2".to_owned(),
        rev: 1,
        change: CatalogChange::SessionRemoved(a.id()),
    });
    assert_eq!(remote.rows(), vec![a.id()], "another epoch applies nothing");
    assert_eq!(remote.streams().len(), 3);
}

#[test]
fn a_bad_line_subscribes_again() {
    let mut remote = Remote::opened(&server_version());
    let first = remote.link();
    remote.line_on(&first, b"{not json");
    assert!(first.0.borrow().stopped);
    assert_eq!(remote.streams().len(), 2);
}

#[test]
fn an_unknown_change_advances_the_cursor_so_the_next_one_applies() {
    let mut remote = Remote::opened(&server_version());
    let a = session("a");
    let w = Uuid::new_v4();
    remote.snapshot(snapshot(vec![workspace(w, "w", vec![a.clone()])]));
    let link = remote.link();
    remote.line_on(&link, br#"{"kind":"future.change","epoch":"e1","rev":1}"#);
    remote.delta(2, CatalogChange::SessionRemoved(a.id()));
    assert!(remote.rows().is_empty());
    assert_eq!(remote.streams().len(), 1);
}

#[test]
fn a_lost_stream_keeps_the_rows_shows_disconnected_and_reconnects_after_the_backoff() {
    let mut remote = Remote::opened(&server_version());
    let a = session("a");
    remote.snapshot(snapshot(vec![workspace(Uuid::new_v4(), "w", vec![a.clone()])]));
    let first = remote.link();
    remote.close(&first, "exit 255");
    assert_eq!(
        remote.state(),
        CatalogState::Disconnected("the catalog stream from box ended: exit 255".to_owned())
    );
    assert_eq!(remote.rows(), vec![a.id()], "the last rows stay");
    assert_eq!(
        remote.window_node()["remote"]["reason"],
        json!("the catalog stream from box ended: exit 255")
    );
    remote
        .fixture
        .core
        .remote_tick(Instant::now() + Duration::from_millis(1500));
    remote.settle();
    assert_eq!(remote.streams().len(), 2);
    assert_eq!(remote.state(), CatalogState::Connecting);
}

#[test]
fn a_replaced_stream_closing_later_changes_nothing() {
    let mut remote = Remote::opened(&server_version());
    let first = remote.link();
    remote.line_on(&first, b"{not json");
    remote.close(&first, "signal 15");
    assert_eq!(remote.state(), CatalogState::Connecting);
    remote.snapshot(snapshot(vec![workspace(Uuid::new_v4(), "w", vec![session("a")])]));
    remote.line_on(&first, &frames::encode(&CatalogFrame::Ping).unwrap());
    assert_eq!(
        first.0.borrow().sent.len(),
        1,
        "a late line of the old stream is not answered"
    );
}

#[test]
fn a_silent_stream_is_dropped_and_retried() {
    let mut remote = Remote::opened(&server_version());
    let first = remote.link();
    remote
        .fixture
        .core
        .remote_tick(Instant::now() + Duration::from_secs(40));
    assert!(first.0.borrow().stopped);
    assert_eq!(
        remote.state(),
        CatalogState::Disconnected("no word from box for 35 seconds".to_owned())
    );
}

// MARK: - rows

#[test]
fn a_lost_daemon_keeps_the_row_and_its_screen_and_marks_it_ended() {
    let mut remote = Remote::opened(&server_version());
    let a = session("a");
    remote.snapshot(snapshot(vec![workspace(Uuid::new_v4(), "w", vec![a.clone()])]));
    let identity = a.panes[0].identity;
    remote
        .fixture
        .core
        .library_mut()
        .store_mut(remote.window)
        .unwrap()
        .session_mut(a.id())
        .unwrap()
        .surface
        .replace(Box::new(ngterm_host::fakes::FakeSurface::new(wire(identity))));
    let before = remote.row(a.id()).launch.command.clone();
    let mut ended = a.fields.clone();
    ended.lifecycle = Lifecycle::Ended;
    remote.delta(
        1,
        CatalogChange::PaneRemoved {
            identity,
            session: a.id(),
        },
    );
    remote.delta(2, CatalogChange::SessionChanged(ended));
    let row = remote.row(a.id());
    assert!(row.ended);
    assert!(row.surface.is_present(), "its last screen stays");
    assert_eq!(row.launch.command, before);
}

#[test]
fn an_ended_session_this_machine_never_showed_gets_the_ended_line_and_no_stream() {
    let mut remote = Remote::opened(&server_version());
    let mut ended = session("gone");
    ended.fields.lifecycle = Lifecycle::Ended;
    ended.panes.clear();
    let streams = remote.links().len();
    remote.snapshot(snapshot(vec![workspace(Uuid::new_v4(), "w", vec![ended.clone()])]));
    let row = remote.row(ended.id());
    assert!(row.ended);
    assert_eq!(
        row.launch.command.as_deref(),
        Some(crate::remote::ssh::ended_command("box").as_str())
    );
    assert!(row.launch.wait);
    assert_eq!(
        remote.links().len(),
        streams,
        "no presentation stream for a session that ended"
    );
}

#[test]
fn a_live_row_starts_its_presentation_stream() {
    let mut remote = Remote::opened(&server_version());
    let a = session("a");
    remote.snapshot(snapshot(vec![workspace(Uuid::new_v4(), "w", vec![a.clone()])]));
    let present = remote
        .links()
        .into_iter()
        .find(|link| link.0.borrow().key.unwrap().session == a.id());
    assert!(present.is_some(), "the row's presentation stream opens");
}

#[test]
fn a_split_added_and_closed_on_the_server_opens_and_closes_here() {
    let mut remote = Remote::opened(&server_version());
    let a = session("a");
    remote.snapshot(snapshot(vec![workspace(Uuid::new_v4(), "w", vec![a.clone()])]));
    let right = pane();
    remote.delta(
        1,
        CatalogChange::PaneAdded {
            pane: right.clone(),
            session: a.id(),
        },
    );
    let mut shown = a.fields.clone();
    shown.shown = true;
    shown.axis = Some("vertical".to_owned());
    remote.delta(2, CatalogChange::SessionChanged(shown));
    let row = remote.row(a.id());
    assert_eq!(row.split_pane_identity, Some(right.identity));
    assert!(row.split.exists && row.split.shown);
    assert!(readable(row.split_launch.command.as_deref().unwrap()).contains(&right.daemon));
    remote.delta(
        3,
        CatalogChange::PaneRemoved {
            identity: right.identity,
            session: a.id(),
        },
    );
    let row = remote.row(a.id());
    assert!(!row.split.exists);
    assert_eq!(row.split_pane_identity, None);
}

#[test]
fn a_swap_the_server_made_swaps_the_slots_even_while_an_attach_is_starting() {
    let mut remote = Remote::opened(&server_version());
    let (left, right) = (pane(), pane());
    let split = session_with(Uuid::new_v4(), "s", vec![left.clone(), right.clone()]);
    remote.snapshot(snapshot(vec![workspace(Uuid::new_v4(), "w", vec![split.clone()])]));
    let commands = (
        remote.row(split.id()).launch.command.clone(),
        remote.row(split.id()).split_launch.command.clone(),
    );
    let mut swapped = split.fields.clone();
    swapped.primary = right.identity;
    remote.delta(1, CatalogChange::SessionChanged(swapped));
    let row = remote.row(split.id());
    assert_eq!(row.pane_identity, right.identity);
    assert_eq!(row.split_pane_identity, Some(left.identity));
    assert_eq!(
        (row.launch.command.clone(), row.split_launch.command.clone()),
        (commands.1, commands.0)
    );
}

#[test]
fn a_killed_primary_promotes_a_split_whose_attach_has_not_started() {
    let mut remote = Remote::opened(&server_version());
    let (left, right) = (pane(), pane());
    let split = session_with(Uuid::new_v4(), "s", vec![left.clone(), right.clone()]);
    remote.snapshot(snapshot(vec![workspace(Uuid::new_v4(), "w", vec![split.clone()])]));
    let split_command = remote.row(split.id()).split_launch.command.clone();
    remote.delta(
        1,
        CatalogChange::PaneRemoved {
            identity: left.identity,
            session: split.id(),
        },
    );
    let row = remote.row(split.id());
    assert_eq!(row.pane_identity, right.identity);
    assert!(!row.split.exists);
    assert_eq!(row.launch.command, split_command);
}

#[test]
fn a_session_the_server_moves_into_a_new_workspace_keeps_its_row() {
    let mut remote = Remote::opened(&server_version());
    let a = session("a");
    let (w1, w2) = (Uuid::new_v4(), Uuid::new_v4());
    remote.snapshot(snapshot(vec![workspace(w1, "one", vec![a.clone()])]));
    let identity = remote.row(a.id()).pane_identity;
    remote.delta(
        1,
        CatalogChange::WorkspaceAdded {
            workspace: workspace(w2, "two", Vec::new()),
            index: 1,
        },
    );
    remote.delta(
        2,
        CatalogChange::SessionMoved {
            id: a.id(),
            workspace: w2,
            index: 0,
        },
    );
    remote.delta(3, CatalogChange::WorkspaceRemoved(w1));
    let store = remote.store();
    assert_eq!(store.workspaces().len(), 1);
    assert_eq!(store.workspaces()[0].id(), w2);
    assert_eq!(
        store.workspace_for_session(a.id()).map(crate::model::Workspace::id),
        Some(w2)
    );
    assert_eq!(remote.row(a.id()).pane_identity, identity);
}

#[test]
fn removing_a_parent_leaves_its_companion_until_its_own_removal() {
    let mut remote = Remote::opened(&server_version());
    let parent = session("parent");
    let companion = catalog::companion("worker", parent.id());
    remote.snapshot(snapshot(vec![workspace(
        Uuid::new_v4(),
        "w",
        vec![parent.clone(), companion.clone()],
    )]));
    assert_eq!(remote.row(companion.id()).parent_id(), Some(parent.id()));
    remote.delta(1, CatalogChange::SessionRemoved(parent.id()));
    assert_eq!(remote.rows(), vec![companion.id()]);
    remote.delta(2, CatalogChange::SessionRemoved(companion.id()));
    assert!(remote.rows().is_empty());
}

#[test]
fn an_endpoint_change_reattaches_a_running_pane() {
    let mut remote = Remote::opened(&server_version());
    let a = session("a");
    remote.snapshot(snapshot(vec![workspace(Uuid::new_v4(), "w", vec![a.clone()])]));
    let identity = a.panes[0].identity;
    remote
        .fixture
        .core
        .library_mut()
        .store_mut(remote.window)
        .unwrap()
        .session_mut(a.id())
        .unwrap()
        .surface
        .replace(Box::new(ngterm_host::fakes::FakeSurface::new(wire(identity))));
    let mut moved = a.panes[0].clone();
    moved.endpoint.socket_directory = "/run/live-2".to_owned();
    remote.delta(
        1,
        CatalogChange::PaneChanged {
            pane: moved,
            session: a.id(),
        },
    );
    let row = remote.row(a.id());
    assert!(!row.surface.is_present(), "the running attach is replaced");
    assert!(readable(row.launch.command.as_deref().unwrap()).contains("/run/live-2"));
}

#[test]
fn a_shown_server_scratch_attaches_and_a_gone_one_closes() {
    let mut remote = Remote::opened(&server_version());
    let mut a = session("a");
    let scratch = pane();
    a.fields.scratch = Some(CatalogScratch {
        pane: scratch.clone(),
        shown: true,
    });
    remote.snapshot(snapshot(vec![workspace(Uuid::new_v4(), "w", vec![a.clone()])]));
    let row = remote.row(a.id());
    assert!(row.scratch_active);
    let command = readable(row.scratch_command.as_deref().unwrap());
    assert!(command.contains(&scratch.daemon), "{command}");
    let mut hidden = a.fields.clone();
    hidden.scratch = Some(CatalogScratch {
        pane: scratch.clone(),
        shown: false,
    });
    remote.delta(1, CatalogChange::SessionChanged(hidden));
    assert!(!remote.row(a.id()).scratch_active);
    let mut gone = a.fields.clone();
    gone.scratch = None;
    remote.delta(2, CatalogChange::SessionChanged(gone));
    let row = remote.row(a.id());
    assert!(row.catalog_scratch.is_none());
    assert!(row.scratch_command.is_none());
}

/// Upstream's `catalogScratchStopped` and `replaceScratchAttachment`: the attach can start before the server's scratch
/// daemon does, and its end must not hide the server's scratch here.
#[test]
fn a_server_scratch_whose_attach_ends_stays_shown_and_attaches_again() {
    a_stopped_server_scratch_stays_shown_and_attaches_again(crate::Core::pane_exited);
}

/// Upstream's `PaneLead.linkLost` parks a scratch before it checks the notice's nonce.
#[test]
fn a_server_scratch_whose_link_is_lost_stays_shown_and_attaches_again() {
    a_stopped_server_scratch_stays_shown_and_attaches_again(|core, session, token| {
        let lost = crate::PaneReport::Title(crate::remote::reconnect::RemoteLinkNotice::title("any"));
        core.report_pane(session, token, &lost)
    });
}

fn a_stopped_server_scratch_stays_shown_and_attaches_again(stop: impl FnOnce(&mut crate::Core, Uuid, &str) -> bool) {
    let mut remote = Remote::opened(&server_version());
    let a = session("a");
    remote.snapshot(snapshot(vec![workspace(Uuid::new_v4(), "w", vec![a.clone()])]));
    let scratch = pane();
    let mut shown = a.fields.clone();
    shown.scratch = Some(CatalogScratch {
        pane: scratch.clone(),
        shown: true,
    });
    remote.delta(1, CatalogChange::SessionChanged(shown));
    let token = wire(Uuid::new_v4());
    let row = remote
        .fixture
        .core
        .library_mut()
        .store_mut(remote.window)
        .unwrap()
        .session_mut(a.id())
        .unwrap();
    row.scratch_command = None;
    row.scratch_surface
        .replace(Box::new(ngterm_host::fakes::FakeSurface::new(token.clone())));

    assert!(stop(&mut remote.fixture.core, a.id(), &token));
    remote.fixture.core.commit();
    let row = remote.row(a.id());
    assert!(row.scratch_active, "the server's scratch stays shown");
    assert!(
        row.scratch_surface.is_present(),
        "its screen stays until it attaches again"
    );
    assert_eq!(remote.tree()["workspaces"][0]["sessions"][0]["scratch"], json!(true));

    for _ in 0..400 {
        remote.fixture.core.remote_tick(Instant::now());
        remote.fixture.core.commit();
        if remote.row(a.id()).scratch_command.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let row = remote.row(a.id());
    assert!(row.scratch_active);
    assert!(!row.scratch_surface.is_present(), "the stopped attach is replaced");
    let command = readable(row.scratch_command.as_deref().expect("a fresh attach"));
    assert!(command.contains(&scratch.daemon), "{command}");
}

#[test]
fn an_ended_row_with_a_running_scratch_keeps_its_presentation() {
    let mut remote = Remote::opened(&server_version());
    let mut a = session("a");
    a.fields.lifecycle = Lifecycle::Ended;
    a.panes.clear();
    a.fields.scratch = Some(CatalogScratch {
        pane: pane(),
        shown: false,
    });
    remote.snapshot(snapshot(vec![workspace(Uuid::new_v4(), "w", vec![a.clone()])]));
    assert!(remote.row(a.id()).presents_catalog());
    let stream = |remote: &Remote| {
        remote
            .links()
            .into_iter()
            .find(|link| link.0.borrow().key.unwrap().session == a.id())
    };
    let present = stream(&remote).expect("a running scratch keeps the stream");
    let mut gone = a.fields.clone();
    gone.scratch = None;
    remote.delta(1, CatalogChange::SessionChanged(gone));
    assert!(present.0.borrow().stopped, "its stream ends with the scratch");
}

#[test]
fn a_catalog_another_window_shows_settles_on_that_window() {
    let mut remote = Remote::opened(&server_version());
    remote.snapshot(snapshot(vec![workspace(Uuid::new_v4(), "w", vec![session("a")])]));
    let first = remote.window;
    let response = remote.open("alias");
    let alias = answered_id(&response).unwrap();
    assert_ne!(alias, first);
    remote.window = alias;
    remote.settle();
    remote.snapshot(snapshot(vec![workspace(Uuid::new_v4(), "w", vec![session("a")])]));
    let library = remote.fixture.core.library();
    assert!(
        !library.windows().iter().any(|info| info.id == alias),
        "the alias's window goes"
    );
    assert_eq!(library.active_window_id(), Some(first));
    assert!(remote.link().0.borrow().stopped);
}

#[test]
fn closing_a_remote_window_ends_its_stream_and_its_rows_leave_running() {
    let mut remote = Remote::opened(&server_version());
    remote.snapshot(snapshot(vec![workspace(Uuid::new_v4(), "w", vec![session("a")])]));
    let stream = remote.link();
    remote.fixture.core.close_window(remote.window);
    remote.fixture.core.commit();
    assert!(stream.0.borrow().stopped);
    assert_eq!(
        remote.window_node()["remote"],
        json!({"host": "box", "catalogID": wire(catalog::CATALOG)})
    );
}

#[test]
fn a_local_window_has_no_remote_node() {
    let mut remote = Remote::opened(&server_version());
    let local = wire(remote.fixture.window);
    let response = remote
        .fixture
        .send(Command::Tree, None, args(|a| a.window = Some(local)));
    assert!(response.result.unwrap()["tree"].get("remote").is_none());
}

#[test]
fn a_pane_ngterm_cannot_address_says_so_and_starts_no_shell() {
    let mut remote = Remote::opened(&server_version());
    let mut a = session("a");
    a.panes[0].daemon = "not one word".to_owned();
    remote.snapshot(snapshot(vec![workspace(Uuid::new_v4(), "w", vec![a.clone()])]));
    let command = readable(remote.row(a.id()).launch.command.as_deref().unwrap());
    assert!(
        command.contains("ngterm: box listed a pane ngterm cannot address"),
        "{command}"
    );
}

// MARK: - row commands

#[test]
fn a_row_command_in_a_local_window_still_runs_while_a_remote_one_is_open() {
    let mut remote = Remote::opened(&server_version());
    let local = wire(remote.fixture.window);
    let response = remote.fixture.send(
        Command::Session(SessionCmd::New),
        None,
        args(|a| a.window = Some(local)),
    );
    assert!(response.ok, "{response:?}");
}

#[test]
fn an_open_that_settles_after_the_duplicate_window_went_names_the_first_window() {
    let mut remote = Remote::opened(&server_version());
    remote.snapshot(snapshot(vec![workspace(Uuid::new_v4(), "w", vec![session("a")])]));
    let first = remote.window;
    let request = Request {
        cmd: Command::Window(WindowCmd::RemoteOpen),
        target: None,
        args: Some(args(|a| a.host = Some("alias".to_owned()))),
    };
    let Dispatched::Pending { resume, .. } = remote.fixture.core.begin(&request) else {
        panic!("the open waits for its connection");
    };
    remote.window = remote.fixture.core.library().active_window_id().unwrap();
    assert_ne!(remote.window, first);
    remote.settle();
    remote.snapshot(snapshot(vec![workspace(Uuid::new_v4(), "w", vec![session("a")])]));
    let response = resume(&mut remote.fixture.core).finish_now(&mut remote.fixture.core);
    assert_eq!(answered_id(&response), Some(first));
    assert_eq!(remote.fixture.core.library().active_window_id(), Some(first));
}

#[test]
fn a_late_close_of_a_replaced_stream_announces_no_tree_change() {
    let mut remote = Remote::opened(&server_version());
    let first = remote.link();
    remote.line_on(&first, b"{not json");
    remote.snapshot(snapshot(vec![workspace(Uuid::new_v4(), "w", vec![session("a")])]));
    let _ = remote.fixture.core.commit();
    let key = first.0.borrow().key.unwrap();
    remote.fixture.core.viewer_closed(key, "signal 15");
    assert!(remote.fixture.core.commit().tree_changed.is_empty());
}

#[test]
fn a_saved_view_comes_back_on_the_rows_of_a_reopened_window() {
    let dir = tempfile::tempdir().unwrap();
    let mut remote = Remote::new(&server_version());
    remote.fixture.core.persistence = Some(LibraryPersistence::new(dir.path()));
    remote.window = answered_id(&remote.open("box")).unwrap();
    remote.settle();
    remote.fixture.core.close_window(remote.window);
    let _ = remote.fixture.core.commit();
    let (a, b) = (session("a"), session("b"));
    let mut view = RemoteWindowViewState {
        selected_session_id: Some(b.id()),
        ..RemoteWindowViewState::default()
    };
    view.sessions.insert(
        a.id(),
        SessionView {
            font_size: Some(17.0),
            ..SessionView::default()
        },
    );
    let path = remote
        .fixture
        .core
        .persistence
        .as_ref()
        .unwrap()
        .remote_view_path(remote.window);
    view.save(&path).unwrap();
    assert_eq!(answered_id(&remote.open("box")), Some(remote.window));
    remote.settle();
    remote.snapshot(snapshot(vec![workspace(
        Uuid::new_v4(),
        "w",
        vec![a.clone(), b.clone()],
    )]));
    assert_eq!(remote.row(a.id()).font_size, Some(17.0));
    assert_eq!(remote.store().selected_session_id(), Some(b.id()));
}

#[test]
fn a_closed_local_item_reopens_in_a_local_window_while_a_remote_one_is_frontmost() {
    let mut remote = Remote::opened(&server_version());
    remote.snapshot(snapshot(vec![workspace(Uuid::new_v4(), "w", vec![session("a")])]));
    let local = remote.fixture.window;
    let (gone, sessions) = remote.fixture.add_workspace("gone", 1);
    let store = remote.fixture.core.library_mut().store_mut(local).unwrap();
    assert!(store.remove_workspace(gone).is_some());
    let _ = remote.fixture.core.commit();
    assert_eq!(remote.fixture.core.library().active_window_id(), Some(remote.window));
    let rows = remote.rows();
    remote.fixture.core.perform_builtin(
        ActionOrigin {
            window: remote.window,
            quick: false,
        },
        BuiltinAction::ReopenRecent,
    );
    assert_eq!(remote.rows(), rows);
    assert_eq!(remote.store().workspaces().len(), 1);
    let restored = remote.fixture.core.library().store(local).unwrap().session(sessions[0]);
    assert!(restored.is_some(), "the local window takes it back");
}

#[path = "remote_window_actions.rs"]
mod actions;

#[path = "remote_window_detach.rs"]
mod detach;
