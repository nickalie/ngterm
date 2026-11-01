//! Companions of a remote row (upstream's `ControlServerRemoteCompanionsTests.swift`): the origin creates them over a
//! scripted ssh runner, and they attach here under the row, never created here or twice.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ngterm_proto::command::{LiveCmd, SessionCmd};
use ngterm_proto::{Args, Command, ControlHtmlPageOutcomeState, Request, Response};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{Fixture, args, wire};
use crate::Core;
use crate::dispatch::Dispatched;
use crate::html::{HtmlHost, HtmlNavigation};
use crate::live::daemon_name;
use crate::model::{NewSession, RemoteBinding, RemoteOrigin, SessionContent, SidebarTint, TintColor};
use crate::remote::page_channel::{ChannelLink, Permits};
use crate::remote::replica_page::{HelperKey, PageHelperLauncher};
use crate::remote::tree::{RemoteCommandResult, RemoteCommandRunner};
use crate::sidebar_actions::SidebarAction;

const CREATED: &str = "0E2B7C1A-5D4F-4E3B-9A61-7C2D8F0B1A23";
const PAGE_ID: &str = "5A1E7C2B-3D4F-4E5A-8B6C-7D8E9F0A1B2C";
const ORIGIN: &str = "origin-1";

// MARK: - fakes

/// Answers each ssh call with the next scripted result (upstream's `ScriptedRunner`).
#[derive(Default)]
struct Scripted {
    results: Mutex<VecDeque<RemoteCommandResult>>,
    calls: Mutex<Vec<Vec<String>>>,
}

impl Scripted {
    fn new(results: Vec<RemoteCommandResult>) -> Arc<Self> {
        Arc::new(Self {
            results: Mutex::new(results.into()),
            calls: Mutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> Vec<Vec<String>> {
        self.calls.lock().unwrap().clone()
    }

    /// The remote line of call `index`.
    fn line(&self, index: usize) -> String {
        self.calls()[index].last().cloned().unwrap_or_default()
    }
}

impl RemoteCommandRunner for Scripted {
    fn run(&self, argv: &[String], _deadline: Duration) -> RemoteCommandResult {
        self.calls.lock().unwrap().push(argv.to_vec());
        self.results
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| result(255, "", "unscripted call"))
    }
}

#[derive(Default)]
struct HostState {
    persistent_failure: Option<String>,
}

#[derive(Clone, Default)]
struct Host(Rc<RefCell<HostState>>);

impl HtmlHost for Host {
    fn persistent_store_failure(&self) -> Option<String> {
        self.0.borrow().persistent_failure.clone()
    }

    fn navigate(&self, _page: Uuid, _navigation: HtmlNavigation) -> Result<(), String> {
        Ok(())
    }
}

struct Helper;

impl ChannelLink for Helper {
    fn send(&self, _line: Vec<u8>) -> bool {
        true
    }

    fn stop(&self) {}
}

#[derive(Default)]
struct Launcher(RefCell<Vec<(Vec<String>, HelperKey)>>);

impl PageHelperLauncher for Launcher {
    fn launch(&self, argv: &[String], key: HelperKey, _permits: Arc<Permits>) -> Result<Box<dyn ChannelLink>, String> {
        self.0.borrow_mut().push((argv.to_vec(), key));
        Ok(Box::new(Helper))
    }
}

// MARK: - the origin's answers

fn result(status: i32, stdout: &str, stderr: &str) -> RemoteCommandResult {
    RemoteCommandResult {
        status,
        stdout: stdout.to_owned(),
        stderr: stderr.to_owned(),
    }
}

fn endpoint() -> Value {
    json!({"executable": "/opt/ngterm/bin/ngterm-session", "socketDirectory": "/tmp/ngterm-live-abc"})
}

fn session(id: &str) -> Value {
    json!({"id": id, "name": "c", "windowID": "w1", "windowName": "main", "workspaceID": "ws1",
        "workspaceName": "work", "cwd": "/repo", "panes": [{"pane": "left", "daemon": daemon_name(Uuid::new_v4())}]})
}

fn tree(sessions: &[Value]) -> RemoteCommandResult {
    let stdout = json!({"ok": true, "result": {"remote": {"endpoint": endpoint(), "sessions": sessions}}});
    result(0, &stdout.to_string(), "")
}

fn origin() -> RemoteCommandResult {
    tree(&[session(ORIGIN)])
}

fn reply(id: &str) -> RemoteCommandResult {
    result(
        0,
        &json!({"ok": true, "result": {"id": id, "parent": ORIGIN}}).to_string(),
        "",
    )
}

fn page_attach(id: &str) -> RemoteCommandResult {
    let page = json!({"page": PAGE_ID, "token": "tok-1",
        "source": {"file": {"path": "/srv/report.html", "grantRoot": "/srv"}}, "identity": "report.html",
        "chromeless": false, "javascript": true, "navigation": false, "browse": false, "persistent": false,
        "follow": false});
    let stdout = json!({"ok": true, "result": {"id": id, "parent": ORIGIN, "companionPage": page}});
    result(0, &stdout.to_string(), "")
}

fn submitted() -> RemoteCommandResult {
    let stdout = json!({"ok": true, "result": {"pageOutcome": {"pageID": PAGE_ID, "outcome": "submitted",
        "value": "yes"}}});
    result(0, &stdout.to_string(), "")
}

// MARK: - the viewer

/// A viewer with one row attached from `buildbox`, bound to the origin's session `origin-1`.
struct Remote {
    fixture: Fixture,
    runner: Arc<Scripted>,
    row: Uuid,
    host: Host,
    launcher: Rc<Launcher>,
}

impl Remote {
    fn new(results: Vec<RemoteCommandResult>) -> Self {
        let mut fixture = Fixture::new(0);
        let runner = Scripted::new(results);
        fixture.core.set_remote_runner(runner.clone());
        let host = Host::default();
        fixture.core.set_html_host(Box::new(host.clone()));
        let launcher = Rc::new(Launcher::default());
        fixture.core.set_page_helper_launcher(launcher.clone());
        let workspace = fixture.workspace;
        let store = fixture.store_mut().unwrap();
        let spec = NewSession {
            remote_host: Some("buildbox".to_owned()),
            ..NewSession::at_cwd("/home/u")
        };
        let row = store.add_session(workspace, spec).unwrap();
        store.bind_remote(RemoteBinding::new(ORIGIN, HashMap::new(), None, None), row);
        fixture.core.commit();
        Self {
            fixture,
            runner,
            row,
            host,
            launcher,
        }
    }

    /// Scripts the next calls.
    fn script(&self, results: Vec<RemoteCommandResult>) {
        self.runner.results.lock().unwrap().extend(results);
    }

    fn core(&mut self) -> &mut Core {
        &mut self.fixture.core
    }

    fn send(&mut self, cmd: SessionCmd, edit: impl FnOnce(&mut Args)) -> Response {
        let target = wire(self.row);
        self.fixture.session(cmd, Some(&target), args(edit))
    }

    fn new_companion(&mut self, edit: impl FnOnce(&mut Args)) -> Response {
        self.send(SessionCmd::CompanionNew, edit)
    }

    fn page_companion(&mut self, edit: impl FnOnce(&mut Args)) -> Response {
        self.send(SessionCmd::CompanionPage, edit)
    }

    fn companions(&self) -> Vec<Uuid> {
        self.fixture
            .store()
            .companions(self.row)
            .into_iter()
            .map(crate::model::Session::id)
            .collect()
    }

    /// Runs the remote tick until `done` or five seconds passed.
    fn tick_until(&mut self, mut done: impl FnMut(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done(self) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
            self.fixture.core.remote_tick(Instant::now() + Duration::from_secs(60));
            self.fixture.core.commit();
        }
    }
}

fn error(response: &Response) -> &str {
    response.error.as_deref().unwrap_or_default()
}

fn id_of(response: &Response) -> Uuid {
    super::answered_id(response).unwrap_or_else(|| panic!("{response:?}"))
}

fn page_file(a: &mut Args) {
    a.html = Some("/srv/report.html".to_owned());
    a.cwd = Some("/srv".to_owned());
}

// MARK: - terminal companions

#[test]
fn the_origin_creates_the_companion_which_attaches_under_the_row_once_its_panes_run() {
    let mut remote = Remote::new(vec![origin(), reply(CREATED), tree(&[]), tree(&[session(CREATED)])]);

    let response = remote.new_companion(|a| {
        a.command = Some("tail -f 'build log'".to_owned());
        a.name = Some("logs".to_owned());
        a.no_select = Some(true);
        a.tint = Some("red".to_owned());
    });

    assert!(response.ok, "{response:?}");
    assert_eq!(response.result.as_ref().unwrap()["parent"], json!(wire(remote.row)));
    let companion = id_of(&response);
    let found = remote.fixture.store().session(companion).unwrap();
    assert_eq!(found.parent_id(), Some(remote.row));
    assert_eq!(found.remote_host(), Some("buildbox"));
    assert_eq!(found.tint.map(SidebarTint::color), Some(TintColor::Red));
    assert_ne!(remote.fixture.selected(), Some(companion));
    assert_eq!(
        remote.runner.calls().len(),
        4,
        "a check, one create, then polls until the origin lists it, never a second create"
    );
    let create = remote.runner.line(1);
    for word in [
        "session",
        "new",
        "--parent",
        ORIGIN,
        "--no-select",
        "--command",
        "--name",
        "logs",
        "--json",
    ] {
        assert!(create.contains(word), "{word}: {create}");
    }
}

#[test]
fn duplicate_as_companion_on_a_remote_row_asks_the_origin_to_duplicate() {
    let mut remote = Remote::new(vec![origin(), reply(CREATED), tree(&[session(CREATED)])]);

    let response = remote.send(SessionCmd::CompanionDuplicate, |_| {});

    assert!(response.ok, "{response:?}");
    let create = remote.runner.line(1);
    for word in ["duplicate", "--companion", "--target", ORIGIN, "--no-select"] {
        assert!(create.contains(word), "{word}: {create}");
    }
    assert_eq!(remote.companions().len(), 1);
}

#[test]
fn an_origin_refusal_or_an_older_origin_creates_nothing_here() {
    let refusal = json!({"ok": false, "error": "no session matching origin-1"}).to_string();
    let mut remote = Remote::new(vec![origin(), result(1, &refusal, "")]);
    let response = remote.new_companion(|_| {});
    assert_eq!(error(&response), "no session matching origin-1");
    assert_eq!(remote.runner.calls().len(), 2);

    let older = json!({"ok": false,
        "error": "invalid request: unknown variant `session.companion.new`, expected one of …"})
    .to_string();
    remote.script(vec![origin(), result(1, &older, "")]);
    let old = remote.new_companion(|_| {});
    assert_eq!(error(&old), "the ngterm on buildbox is too old for companions");
    assert!(remote.companions().is_empty());
}

/// Runs one page attachment to its end.
fn attach_page(core: &mut Core, parent: Uuid) -> Result<ngterm_proto::ControlResult, String> {
    let mut attach = core.start_page_attach(CREATED, "buildbox", parent, false)?;
    loop {
        match core.step_page_attach(attach) {
            crate::remote::companions::Step::Pending(next, _) => attach = next,
            crate::remote::companions::Step::Done(done) => return *done,
        }
    }
}

#[test]
fn an_older_origin_cli_refusing_the_arguments_is_too_old_not_unknown() {
    let usage = result(64, "", "Error: Unknown option '--parent'\nUsage: ngtermctl session new");
    let mut remote = Remote::new(vec![origin(), usage]);

    let response = remote.new_companion(|_| {});

    assert_eq!(error(&response), "the ngterm on buildbox is too old for companions");
    assert!(remote.companions().is_empty());

    remote.script(vec![result(64, "", "Error: Unexpected argument 'page-attach'")]);
    let row = remote.row;
    assert_eq!(
        attach_page(remote.core(), row),
        Err("the ngterm on buildbox is too old for page companions".to_owned())
    );
}

#[test]
fn a_companion_that_never_becomes_attachable_within_the_budget_is_named_and_not_created_again() {
    let mut results = vec![origin(), reply(CREATED)];
    results.extend((0..50).map(|_| tree(&[])));
    let mut remote = Remote::new(results);
    let now = Rc::new(Cell::new(Instant::now()));
    let clock = Rc::clone(&now);
    remote.core().remote.clock = Box::new(move || {
        clock.set(clock.get() + Duration::from_secs(3));
        clock.get()
    });

    let response = remote.new_companion(|_| {});

    assert_eq!(
        error(&response),
        format!(
            "created companion {CREATED} on buildbox but could not attach it: it did not become attachable; \
             `ngtermctl live attach buildbox {CREATED}` attaches it"
        )
    );
    let creates = remote
        .runner
        .calls()
        .iter()
        .filter(|call| call.last().is_some_and(|line| line.contains("--parent")))
        .count();
    assert_eq!(creates, 1);
    assert!(
        remote.runner.calls().len() < 12,
        "the wall-clock budget ends the polling"
    );
    assert!(remote.companions().is_empty());
}

#[test]
fn a_create_with_no_answer_is_an_unknown_outcome_and_a_stale_binding_creates_nothing() {
    let mut remote = Remote::new(vec![origin(), result(-1, "", "the remote did not answer in time")]);
    let unknown = remote.new_companion(|_| {});
    assert_eq!(
        error(&unknown),
        "buildbox gave no answer (the remote did not answer in time); whether the companion was created there is \
         unknown"
    );

    remote.script(vec![tree(&[])]);
    let before = remote.runner.calls().len();
    let refused = remote.new_companion(|_| {});
    assert_eq!(
        error(&refused),
        "the session on buildbox is not attachable now; nothing was created"
    );
    assert_eq!(
        remote.runner.calls().len(),
        before + 1,
        "nothing is created on an origin that cannot attach it"
    );
    assert!(remote.companions().is_empty());
}

#[test]
fn a_full_receipt_over_a_failed_transport_names_the_companion_and_attaches_nothing() {
    let receipt = result(255, &reply(CREATED).stdout, "Connection reset");
    let mut remote = Remote::new(vec![origin(), receipt]);

    let response = remote.new_companion(|_| {});

    assert_eq!(
        error(&response),
        format!(
            "created companion {CREATED} on buildbox but the connection failed before attaching it; `ngtermctl live \
             attach buildbox {CREATED}` attaches it"
        )
    );
    assert_eq!(remote.runner.calls().len(), 2, "nothing is created again or attached");
    assert!(remote.companions().is_empty());
}

#[test]
fn a_parent_closed_during_the_wait_leaves_the_companion_named_and_no_orphan_row() {
    let mut remote = Remote::new(vec![origin(), reply(CREATED), tree(&[session(CREATED)])]);
    let request = Request {
        cmd: Command::Session(SessionCmd::CompanionNew),
        target: Some(wire(remote.row)),
        args: None,
    };
    let mut dispatched = remote.core().begin(&request);
    let mut closed = false;
    let response = loop {
        match dispatched {
            Dispatched::Done(response) => break response,
            Dispatched::Pending { resume, .. } => {
                if !closed && remote.runner.calls().len() >= 2 {
                    let row = remote.row;
                    remote.fixture.store_mut().unwrap().close_session(row);
                    closed = true;
                }
                dispatched = resume(remote.core());
            }
        }
    };

    assert_eq!(
        error(&response),
        format!(
            "created companion {CREATED} on buildbox but could not attach it: its parent row here was closed; \
             `ngtermctl live attach buildbox {CREATED}` attaches it"
        )
    );
    let row = remote.row;
    assert!(
        remote
            .fixture
            .store()
            .sessions()
            .all(|session| session.parent_id() != Some(row))
    );
}

#[test]
fn an_attach_failure_after_creation_names_the_created_companion() {
    let mut unaddressable = session(CREATED);
    unaddressable["panes"] = json!([{"pane": "middle", "daemon": daemon_name(Uuid::new_v4())}]);
    let mut remote = Remote::new(vec![origin(), reply(CREATED), tree(&[unaddressable])]);

    let response = remote.new_companion(|_| {});

    assert_eq!(
        error(&response),
        format!(
            "created companion {CREATED} on buildbox but could not attach it: buildbox reported panes ngterm cannot \
             address; `ngtermctl live attach buildbox {CREATED}` attaches it"
        )
    );
    assert!(remote.companions().is_empty());
}

#[test]
fn closing_a_remote_companion_here_only_detaches_it() {
    let mut remote = Remote::new(vec![origin(), reply(CREATED), tree(&[session(CREATED)])]);
    let companion = id_of(&remote.new_companion(|_| {}));

    let closed = remote
        .fixture
        .session(SessionCmd::Close, Some(&wire(companion)), Args::default());

    assert!(closed.ok, "{closed:?}");
    assert!(remote.companions().is_empty());
    assert!(remote.fixture.store().session(remote.row).is_some());
    assert_eq!(
        remote.runner.calls().len(),
        3,
        "the origin is never asked to end anything"
    );
}

// MARK: - page companions

#[test]
fn a_page_companion_of_a_remote_row_is_created_there_and_shown_here_through_its_own_channel() {
    let mut remote = Remote::new(vec![origin(), reply(CREATED), page_attach(CREATED)]);

    let response = remote.page_companion(|a| {
        page_file(a);
        a.javascript = Some(true);
    });

    assert!(response.ok, "{response:?}");
    let page = remote.companions()[0];
    let found = remote.fixture.store().session(page).unwrap();
    assert_eq!(found.content(), SessionContent::Page);
    assert_eq!(found.remote_host(), Some("buildbox"));
    let overlay = found.overlay.page().unwrap();
    assert_eq!(overlay.replica_of, Uuid::parse_str(PAGE_ID).ok());
    assert_eq!(
        found.remote_presentation().unwrap().binding.remote_session_id,
        CREATED,
        "New Companion on the row makes a sibling there"
    );
    let create = remote.runner.line(1);
    for word in [
        "overlay",
        "open",
        "--companion",
        "--html",
        "/srv/report.html",
        "--cwd",
        "/srv",
        "--js",
    ] {
        assert!(create.contains(word), "{word}: {create}");
    }
    assert!(remote.runner.line(2).contains("page-attach"));
    let serve = remote.launcher.0.borrow().last().unwrap().0.last().cloned().unwrap();
    assert!(
        serve.contains("page-serve") && serve.contains("tok-1"),
        "the channel claims the attachment's own token: {serve}"
    );
    let local = response.result.as_ref().unwrap()["pageID"].as_str().unwrap().to_owned();
    assert_eq!(local, wire(overlay.id), "a --block caller polls the view's own page");
    let outcome = remote.fixture.core.page_outcome(overlay.id).unwrap();
    assert_eq!(outcome.outcome, ControlHtmlPageOutcomeState::Pending);
}

/// Creates a page companion of the row and returns its row and local page id.
fn page_companion(remote: &mut Remote) -> (Uuid, Uuid) {
    let response = remote.page_companion(page_file);
    let row = id_of(&response);
    let local = Uuid::parse_str(response.result.as_ref().unwrap()["pageID"].as_str().unwrap()).unwrap();
    remote.core().commit();
    (row, local)
}

/// The page's helper ends: the page ended on its origin.
fn end_helper(remote: &mut Remote) {
    let key = remote.launcher.0.borrow().last().unwrap().1;
    remote.core().page_helper_closed(key, "exit 0");
    remote.core().commit();
}

#[test]
fn the_pages_outcome_there_becomes_the_views_outcome_when_that_mac_ends_it() {
    let mut remote = Remote::new(vec![origin(), reply(CREATED), page_attach(CREATED), submitted()]);
    let (_, local) = page_companion(&mut remote);

    end_helper(&mut remote);
    remote.tick_until(|remote| remote.companions().is_empty());

    assert!(remote.companions().is_empty(), "the page ending there closes the view");
    let outcome = remote.fixture.core.page_outcome(local).unwrap();
    assert_eq!(outcome.outcome, ControlHtmlPageOutcomeState::Submitted);
    assert_eq!(outcome.value.as_deref(), Some("yes"));
    assert!(remote.runner.calls().last().unwrap().last().unwrap().contains("result"));
}

#[test]
fn a_page_ending_there_while_parked_never_comes_back_on_undo() {
    for closes_parent in [false, true] {
        let mut remote = Remote::new(vec![origin(), reply(CREATED), page_attach(CREATED), submitted()]);
        let (page, _) = page_companion(&mut remote);
        let parked = if closes_parent { remote.row } else { page };
        assert!(remote.fixture.store_mut().unwrap().soft_close_session(parked));
        remote.core().commit();

        end_helper(&mut remote);
        remote.tick_until(|remote| remote.fixture.store().pending_close_session(page).is_none());

        let store = remote.fixture.store_mut().unwrap();
        assert!(
            store.pending_close_session(page).is_none(),
            "closes parent {closes_parent}"
        );
        assert_eq!(
            store.undo_pending_close(None),
            closes_parent,
            "the parent alone is still undoable"
        );
        assert!(store.session(page).is_none());
        assert!(store.session(remote.row).is_some());
    }
}

#[test]
fn a_persistent_page_needs_this_machines_browser_store_before_anything_is_created() {
    let mut remote = Remote::new(vec![]);
    remote.host.0.borrow_mut().persistent_failure = Some("the browser store cannot be read".to_owned());

    let response = remote.page_companion(|a| {
        a.url = Some("https://example.com/".to_owned());
        a.persistent = Some(true);
    });

    assert!(error(&response).starts_with("session.companion.page:"), "{response:?}");
    assert!(remote.runner.calls().is_empty());
    assert!(remote.companions().is_empty());
}

#[test]
fn a_file_pages_paths_are_the_origins_to_check() {
    // this machine's home is /home/u, a grant it refuses for its own pages; the origin's home is its own
    let mut remote = Remote::new(vec![origin(), reply(CREATED), page_attach(CREATED)]);

    let response = remote.page_companion(|a| {
        a.html = Some("/home/u/report.html".to_owned());
        a.cwd = Some("/home/u".to_owned());
    });

    assert!(response.ok, "{response:?}");
    let create = remote.runner.line(1);
    assert!(
        create.contains("/home/u/report.html") && create.contains("--cwd"),
        "sent as they came: {create}"
    );
}

#[test]
fn a_loopback_page_is_refused_before_the_origin_is_asked() {
    let mut remote = Remote::new(vec![]);

    let response = remote.page_companion(|a| a.url = Some("http://localhost:3000/".to_owned()));

    assert_eq!(
        error(&response),
        "a loopback page cannot be a companion of a remote session"
    );
    assert!(remote.runner.calls().is_empty());
    assert!(remote.companions().is_empty());
}

#[test]
fn a_listed_page_row_attaches_only_under_the_one_row_showing_its_parent() {
    let page_row = json!({"id": CREATED, "name": "report", "windowID": "w1", "windowName": "main",
        "workspaceID": "ws1", "workspaceName": "work", "cwd": "/srv", "panes": [], "parent": ORIGIN,
        "content": "page"});
    let listing = || tree(&[session(ORIGIN), page_row.clone()]);
    let mut remote = Remote::new(vec![listing(), page_attach(CREATED), listing()]);
    let endpoint: ngterm_proto::ControlLiveEndpoint = serde_json::from_value(endpoint()).unwrap();
    let bind = |remote: &mut Remote, row: Uuid| {
        let origin = RemoteOrigin {
            host: "buildbox".to_owned(),
            endpoint: endpoint.clone(),
            session_name: "c".to_owned(),
        };
        let binding = RemoteBinding::new(ORIGIN, HashMap::new(), None, Some(origin));
        remote.fixture.store_mut().unwrap().bind_remote(binding, row);
    };
    let row = remote.row;
    bind(&mut remote, row);
    let attach = |remote: &mut Remote| {
        let host = args(|a| a.host = Some("buildbox".to_owned()));
        remote.fixture.send(Command::Live(LiveCmd::Attach), Some(CREATED), host)
    };

    let attached = attach(&mut remote);

    assert!(attached.ok, "{attached:?}");
    let page = remote.companions()[0];
    assert_eq!(
        remote.fixture.store().session(page).unwrap().content(),
        SessionContent::Page
    );

    let workspace = remote.fixture.workspace;
    let spec = NewSession {
        remote_host: Some("buildbox".to_owned()),
        ..NewSession::at_cwd("/home/u")
    };
    let other = remote
        .fixture
        .store_mut()
        .unwrap()
        .add_session(workspace, spec)
        .unwrap();
    bind(&mut remote, other);
    let ambiguous = attach(&mut remote);
    assert_eq!(error(&ambiguous), "more than one row here shows the page's parent");
}

#[test]
fn a_gui_new_companion_on_a_remote_row_creates_it_through_the_origin() {
    let mut remote = Remote::new(vec![origin(), reply(CREATED), tree(&[session(CREATED)])]);
    let (window, row) = (remote.fixture.window, remote.row);

    assert!(remote.core().sidebar_action(window, SidebarAction::NewCompanion(row)));
    remote.core().commit();
    remote.tick_until(|remote| !remote.companions().is_empty());

    let companion = remote.companions()[0];
    assert_eq!(
        remote.fixture.store().session(companion).unwrap().remote_host(),
        Some("buildbox")
    );
    assert_eq!(
        remote.fixture.selected(),
        Some(companion),
        "the GUI selects the new companion"
    );
}

#[test]
fn a_gui_duplicate_whose_origin_refuses_shows_the_reason_over_the_row() {
    let refusal = json!({"ok": false, "error": "left pane is not realized; nothing was created"}).to_string();
    let mut remote = Remote::new(vec![origin(), result(1, &refusal, "")]);
    let (window, row) = (remote.fixture.window, remote.row);
    // the panels are HUDs, which need the painter
    let bodies = tempfile::tempdir().unwrap();
    remote.core().set_hud_body_dir(bodies.path().to_path_buf());

    assert!(
        remote
            .core()
            .sidebar_action(window, SidebarAction::DuplicateAsCompanion(row))
    );
    remote.core().commit();
    let hud = |remote: &Remote| {
        let found = remote.fixture.store().session(remote.row).unwrap();
        found
            .overlay
            .effective_hud_spec()
            .filter(|_| found.overlay.hud_active())
            .map(|spec| spec.message)
    };
    assert!(
        hud(&remote).is_some_and(|message| message.contains("creating a companion on buildbox")),
        "{:?}",
        hud(&remote)
    );
    remote.tick_until(|remote| hud(remote).is_some_and(|message| message.contains("not realized")));

    let message = hud(&remote).unwrap();
    assert!(message.starts_with("Duplicate as Companion"), "{message}");
    assert!(remote.companions().is_empty());
}
