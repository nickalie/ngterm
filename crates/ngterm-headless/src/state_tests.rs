use std::collections::VecDeque;
use std::sync::Mutex;

use ngterm_core::model::WindowLibrary;
use ngterm_core::{CoreConfig, CoreState};
use ngterm_host::{MainLoopClosed, MainTask};
use ngterm_proto::{Args, command::SessionCmd};
use serde_json::{Value, json};

use super::*;
use crate::panes::PaneEnvironment;

/// A main-loop seam whose clock the test advances by taking the next task.
#[derive(Default)]
struct Loop {
    tasks: Mutex<VecDeque<MainTask<HeadlessState>>>,
    closed: std::sync::atomic::AtomicBool,
}

impl MainLoop<HeadlessState> for Loop {
    fn post(&self, task: MainTask<HeadlessState>) -> Result<(), MainLoopClosed> {
        self.post_after(Duration::ZERO, task)
    }

    fn post_after(&self, _: Duration, task: MainTask<HeadlessState>) -> Result<(), MainLoopClosed> {
        if self.closed.load(std::sync::atomic::Ordering::Acquire) {
            return Err(MainLoopClosed);
        }
        self.tasks.lock().unwrap().push_back(task);
        Ok(())
    }
}

impl Loop {
    fn drain(&self, state: &mut HeadlessState) {
        // Take a batch: a heartbeat may rearm itself for the next tick.
        let tasks = std::mem::take(&mut *self.tasks.lock().unwrap());
        for task in tasks {
            task(state);
        }
    }
}

fn fixture() -> (HeadlessState, Arc<Loop>, Uuid) {
    let library = WindowLibrary::seeded("/");
    let window = library.active_window_id().unwrap();
    let session = library.store(window).unwrap().sessions().next().unwrap().id();
    let core = Core::new(
        CoreConfig {
            app: crate::identity(),
            capabilities: Vec::new(),
        },
        CoreState {
            library,
            ..CoreState::default()
        },
    );
    let config = ngterm_core::ghostty_config::GhosttyConfig::default();
    let panes = Panes::new(
        PaneEnvironment {
            socket: "private-test.sock".into(),
            program_version: "test".into(),
            host: ngterm_term::Host::new(&config, None, None, None),
            config,
            grid: (80, 24),
            hostname: "box".into(),
        },
        Arc::new(|_, _| {}),
    );
    let main_loop = Arc::new(Loop::default());
    let posting: Arc<dyn MainLoop<HeadlessState>> = main_loop.clone();
    (
        HeadlessState::new(core, Arc::downgrade(&posting), panes),
        main_loop,
        session,
    )
}

fn request(cmd: Command, target: Option<Uuid>, args: Value) -> Request {
    Request {
        cmd,
        target: target.map(|id| id.to_string()),
        args: Some(serde_json::from_value::<Args>(args).unwrap()),
    }
}

fn session_request(cmd: SessionCmd, session: Uuid, args: Value) -> Request {
    request(Command::Session(cmd), Some(session), args)
}

/// The tree a fresh start over `env`'s state directory would serve.
fn saved_tree(env: &ngterm_core::paths::MapEnv) -> String {
    let mut core = Core::new(
        CoreConfig {
            app: crate::identity(),
            capabilities: Vec::new(),
        },
        crate::load_state(env),
    );
    serde_json::to_string(&core.dispatch(&Request::new(Command::Tree))).unwrap()
}

#[test]
fn requests_change_the_model_and_quit_saves_only_once() {
    let (mut state, main_loop, _) = fixture();
    let dir = tempfile::tempdir().unwrap();
    let env = ngterm_core::paths::MapEnv::new()
        .with("HOME", dir.path())
        .with("NGTERM_STATE_DIR", dir.path());
    state.core = Core::new(
        CoreConfig {
            app: crate::identity(),
            capabilities: Vec::new(),
        },
        crate::load_state(&env),
    );
    let window = state.core.library().active_window_id().unwrap();
    let session = state
        .core
        .library()
        .store(window)
        .unwrap()
        .active_session()
        .unwrap()
        .id();
    let reply = state.handle_request(&session_request(SessionCmd::Rename, session, json!({"name":"worker"})));
    assert!(reply.response.ok, "{reply:?}");
    assert!(
        serde_json::to_string(&state.handle_request(&Request::new(Command::Tree)).response)
            .unwrap()
            .contains("worker")
    );
    state.drain_pane(session, "missing");
    assert_eq!(state.terminals(), 0);
    state.request_quit();
    state.request_quit();
    assert!(state.quit_requested());
    // the rename's debounced save is still queued: only the quit save can have written it
    state.save_on_quit();
    assert!(!state.quit_requested());
    assert!(saved_tree(&env).contains("worker"));
    let reply = state.handle_request(&session_request(
        SessionCmd::Rename,
        session,
        json!({"name":"renamed-after-quit"}),
    ));
    assert!(reply.response.ok, "{reply:?}");
    state.request_quit();
    assert!(!state.quit_requested(), "a quit after the quit save asks for another");
    state.save_on_quit();
    state.settle();
    state.land();
    state.drain_pane(session, "missing");
    main_loop.drain(&mut state);
    let saved = saved_tree(&env);
    assert!(
        saved.contains("worker") && !saved.contains("renamed-after-quit"),
        "{saved}"
    );
    state.teardown();
    assert_eq!(state.terminals(), 0);
}

#[test]
fn a_deferred_reply_can_wait_again_and_a_panicking_step_answers_the_wire() {
    let (mut state, main_loop, _) = fixture();
    let request = Request::new(Command::Tree);
    let reply = state.reply(
        &request,
        Dispatched::Pending {
            after: Duration::ZERO,
            resume: Box::new(|_| Dispatched::Pending {
                after: Duration::ZERO,
                resume: Box::new(|_| Dispatched::Done(Response::with_result(json!({"done":true})))),
            }),
        },
    );
    main_loop.drain(&mut state);
    main_loop.drain(&mut state);
    assert_eq!(reply.settled().response.result, Some(json!({"done":true})));
    let (reply, sender) = Reply::deferred();
    state.resume(
        Command::Tree,
        Box::new(|_| panic!("test handler")),
        Waiter::Socket(sender),
    );
    let response = reply.settled().response;
    assert!(!response.ok);
    assert_eq!(response.error.as_deref(), Some("internal error handling tree"));
}

#[test]
fn posts_and_deferred_replies_are_dropped_when_the_loop_is_gone_or_closed() {
    let (mut state, main_loop, _) = fixture();
    main_loop.closed.store(true, std::sync::atomic::Ordering::Release);
    post(&state.poster, HeadlessState::request_quit);
    assert!(!state.after(Duration::ZERO, "test", HeadlessState::request_quit));
    let reply = state.reply(
        &Request::new(Command::Tree),
        Dispatched::Pending {
            after: Duration::ZERO,
            resume: Box::new(|_| panic!("must never run")),
        },
    );
    assert_eq!(
        reply.settled().response.error.as_deref(),
        Some("the app is shutting down")
    );
    drop(main_loop);
    post(&state.poster, HeadlessState::request_quit);
    assert!(!state.after(Duration::ZERO, "test", HeadlessState::request_quit));
    state.settle();
    state.schedule_live_quit(false);
    assert!(!state.quit_requested());
}

#[test]
fn scheduled_quit_waits_for_the_reply_task() {
    let (mut state, main_loop, _) = fixture();
    state.schedule_live_quit(true);
    assert!(!state.quit_requested());
    main_loop.drain(&mut state);
    assert!(state.quit_requested());
}

#[test]
fn info_and_hud_expiries_clear_the_user_visible_content() {
    let (mut state, main_loop, session) = fixture();
    let dir = tempfile::tempdir().unwrap();
    state.core.set_hud_body_dir(dir.path().to_path_buf());
    for (cmd, args) in [
        (
            SessionCmd::Info,
            json!({"mode":"set","text":"working","hideAfter":0.01}),
        ),
        (SessionCmd::HudOpen, json!({"message":"waiting","hideAfter":0.01})),
    ] {
        let reply = state.handle_request(&session_request(cmd, session, args));
        assert!(reply.response.ok, "{reply:?}");
    }
    main_loop.drain(&mut state);
    let window = state.core.library().active_window_id().unwrap();
    let found = state.core.library().store(window).unwrap().session(session).unwrap();
    assert!(found.effective_info().is_none());
    assert!(found.overlay.hud.is_none());
}

#[test]
fn malformed_stream_results_remain_plain_replies() {
    let (mut state, _, _) = fixture();
    for result in [json!({}), json!({"id":42}), Value::Null] {
        let response = Response::with_result(result);
        assert!(state.job_stream(response.clone()).stream.is_none());
        assert!(state.presentation_stream(response.clone()).stream.is_none());
        assert!(state.page_serve_stream(response).stream.is_none());
    }
}

#[test]
fn page_helper_reports_for_an_unknown_page_leave_the_tree_unchanged() {
    let (mut state, _, _) = fixture();
    let helper = ngterm_core::remote::replica_page::HelperKey {
        page: Uuid::new_v4(),
        launch: 99,
    };
    let before = state.handle_request(&Request::new(Command::Tree)).response;
    for quitting in [false, true] {
        if quitting {
            state.save_on_quit();
        }
        state.page_helper_line(helper, b"{}");
        state.page_helper_closed(helper, "closed");
    }
    assert_eq!(state.handle_request(&Request::new(Command::Tree)).response, before);
}

/// A presentation stream's channel: the frames sent on it, and how often it was shut down.
#[cfg(unix)]
#[derive(Clone, Default)]
struct Lines {
    sent: std::rc::Rc<std::cell::RefCell<Vec<Value>>>,
    shutdowns: std::rc::Rc<std::cell::Cell<usize>>,
}
#[cfg(unix)]
impl ngterm_core::LineChannel for Lines {
    fn send(&self, line: Vec<u8>) -> bool {
        self.sent.borrow_mut().push(serde_json::from_slice(&line).unwrap());
        true
    }
    fn shutdown(&self) {
        self.shutdowns.set(self.shutdowns.get() + 1);
    }
}
#[cfg(unix)]
impl Lines {
    fn last(&self, kind: &str) -> Value {
        self.sent
            .borrow()
            .iter()
            .rev()
            .find(|frame| frame["kind"] == kind)
            .unwrap()
            .clone()
    }
}

/// The fixture's session presented live to one viewer holding the presenter role: the state, its loop, the session,
/// the viewer's channel and stream serial, and the directory the live install lives in.
#[cfg(unix)]
fn presented() -> (HeadlessState, Arc<Loop>, Uuid, Lines, u64, tempfile::TempDir) {
    use ngterm_host::fakes::{FakeSessionHost, FakeSurface};
    let (mut state, main_loop, session) = fixture();
    let dir = tempfile::tempdir().unwrap();
    let integration = dir.path().join("shell-integration/zsh");
    std::fs::create_dir_all(&integration).unwrap();
    std::fs::write(integration.join(".zshenv"), "").unwrap();
    state.core.force_restore_mode(ngterm_core::persist::RestoreMode::Live);
    state
        .core
        .set_restore_launch(ngterm_core::restore::mode::RestoreLaunchDecision::new(
            ngterm_core::persist::RestoreMode::Live,
            None,
        ));
    state.core.install_live_sessions(ngterm_core::LiveInstall {
        host: Arc::new(FakeSessionHost::new()),
        endpoint: ngterm_proto::ControlLiveEndpoint {
            executable: "/bin/sh".into(),
            socket_directory: dir.path().join("live").to_string_lossy().into_owned(),
        },
        wrap: ngterm_core::live::support::Inputs {
            executable: "/bin/sh".into(),
            login_shell: Some("/bin/zsh".into()),
            resources_dir: Some(dir.path().to_path_buf()),
            state_dir: dir.path().to_path_buf(),
            ..Default::default()
        },
        state_dir: dir.path().to_path_buf(),
        outdated_before: None,
    });
    let window = state.core.library().active_window_id().unwrap();
    let token = {
        let found = state
            .core
            .library_mut()
            .store_mut(window)
            .unwrap()
            .session_mut(session)
            .unwrap();
        let token = ngterm_core::pane_env::uuid_string(found.pane_identity);
        found
            .pane_slot_mut(ngterm_core::model::StatusPane::Left)
            .replace(Box::new(FakeSurface::new(&token).with_live_session(true)));
        token
    };
    state.core.wake_session(session);
    let seed = state.core.request_pane_spawn(session, &token).unwrap();
    let nonce = seed
        .env
        .iter()
        .find(|(name, _)| name == ngterm_core::live::lead::NONCE_VARIABLE)
        .unwrap()
        .1
        .clone();
    let title = format!("{}{nonce}:follower:1", ngterm_core::live::lead::LeadNotice::PREFIX);
    state
        .core
        .report_pane(session, &token, &ngterm_core::PaneReport::Title(title));
    let response = state
        .core
        .dispatch(&request(Command::Live(LiveCmd::Present), Some(session), json!({})));
    assert!(response.ok, "{response:?}");
    let lines = Lines::default();
    let serial = state
        .core
        .adopt_presentation_stream(&session.to_string(), Box::new(lines.clone()));
    state.core.presentation_line(serial, &serde_json::to_vec(&json!({"kind":"hello","gen":0,"rev":0,"hello":{"version":1,"mode":"presenter","kinds":ngterm_core::remote::hub::SUPPORTED_KINDS}})).unwrap());
    let gen_id = lines.last("hello")["gen"].clone();
    state.core.presentation_line(
        serial,
        &serde_json::to_vec(&json!({"kind":"presenter.acquire","gen":gen_id,"rev":1})).unwrap(),
    );
    state.settle();
    assert_eq!(lines.last("presenter.granted")["kind"], "presenter.granted");
    (state, main_loop, session, lines, serial, dir)
}

#[cfg(unix)]
fn pump(main_loop: &Loop, state: &mut HeadlessState) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while main_loop.tasks.lock().unwrap().is_empty() {
        assert!(Instant::now() < deadline, "no stream callback");
        std::thread::yield_now();
    }
    main_loop.drain(state);
}

#[cfg(unix)]
#[test]
fn a_job_stream_delivers_context_and_reports_the_helpers_exit() {
    use std::io::{BufRead as _, BufReader, Write as _};
    let (mut state, main_loop, session, lines, _, _dir) = presented();
    let opened = state.handle_request(&session_request(
        SessionCmd::OverlayOpen,
        session,
        json!({"command":"exit 7"}),
    ));
    assert!(opened.response.ok, "{opened:?}");
    let job = lines.last("overlay.request")["overlay"]["job"]
        .as_str()
        .unwrap()
        .to_owned();
    let reply = state.handle_request(&Request {
        cmd: Command::Session(SessionCmd::OverlayJobRun),
        target: Some(job),
        args: None,
    });
    assert!(reply.response.ok, "{reply:?}");
    let (server, mut client) = ngterm_server::endpoint::pair().unwrap();
    client.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let owner = reply.stream.unwrap();
    let worker = std::thread::spawn(move || owner.adopt(server, Vec::new()));
    let mut text = String::new();
    BufReader::new(client.try_clone().unwrap())
        .read_line(&mut text)
        .unwrap();
    let context: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(context["context"]["command"], "exit 7");
    main_loop.drain(&mut state);
    client
        .write_all(b"{\"kind\":\"started\"}\n{\"kind\":\"exited\",\"code\":7}\n")
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        main_loop.drain(&mut state);
        let reply = state.handle_request(&session_request(SessionCmd::OverlayResult, session, json!({})));
        if reply.response.ok {
            assert_eq!(reply.response.result.unwrap()["exitCode"], 7);
            break;
        }
        assert!(Instant::now() < deadline, "{reply:?}");
        std::thread::yield_now();
    }
    client.shutdown(std::net::Shutdown::Both).unwrap();
    worker.join().unwrap();
    pump(&main_loop, &mut state);
}

#[cfg(unix)]
#[test]
fn page_channels_answer_bridge_calls_and_park_on_disconnect() {
    use std::io::{BufRead as _, BufReader, Write as _};
    let (mut state, main_loop, session, lines, _, dir) = presented();
    let file = dir.path().join("index.html");
    std::fs::write(&file, "<p>hello</p>").unwrap();
    let opened = state.handle_request(&session_request(
        SessionCmd::OverlayOpen,
        session,
        json!({"html":file,"javascript":true,"cwd":dir.path()}),
    ));
    assert!(opened.response.ok, "{opened:?}");
    let token = lines.last("page.request")["page"]["token"].as_str().unwrap().to_owned();
    let reply = state.handle_request(&Request {
        cmd: Command::Session(SessionCmd::OverlayPageServe),
        target: Some(token),
        args: None,
    });
    assert!(reply.response.ok, "{reply:?}");
    let (server, mut client) = ngterm_server::endpoint::pair().unwrap();
    client.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let worker = std::thread::spawn(move || reply.stream.unwrap().adopt(server, Vec::new()));
    main_loop.drain(&mut state);
    let frame = ngterm_core::remote::page_serve::PageServeFrame::Bridge {
        id: 1,
        request: "{\"cmd\":\"tree\"}".into(),
    };
    client
        .write_all(&ngterm_core::remote::page_serve::encode(&frame).unwrap())
        .unwrap();
    pump(&main_loop, &mut state);
    let mut text = String::new();
    BufReader::new(client.try_clone().unwrap())
        .read_line(&mut text)
        .unwrap();
    let frame = ngterm_core::remote::page_serve::decode(text.as_bytes()).unwrap();
    let ngterm_core::remote::page_serve::PageServeFrame::Reply { id, response } = frame else {
        panic!("{frame:?}");
    };
    assert_eq!(id, 1);
    let response: Value = serde_json::from_str(&response).unwrap();
    assert_eq!(response["ok"], true, "{response}");
    let resource = ngterm_core::remote::page_serve::PageServeFrame::Resource {
        id: 2,
        path: file.to_string_lossy().into_owned(),
    };
    client
        .write_all(&ngterm_core::remote::page_serve::encode(&resource).unwrap())
        .unwrap();
    let mut reader = BufReader::new(client.try_clone().unwrap());
    let mut body = Vec::new();
    loop {
        let mut line = String::new();
        assert_ne!(reader.read_line(&mut line).unwrap(), 0);
        match ngterm_core::remote::page_serve::decode(line.as_bytes()).unwrap() {
            ngterm_core::remote::page_serve::PageServeFrame::Head { id, .. } => assert_eq!(id, 2),
            ngterm_core::remote::page_serve::PageServeFrame::Chunk { id, data } => {
                assert_eq!(id, 2);
                body.extend(data);
            }
            ngterm_core::remote::page_serve::PageServeFrame::End { id } => {
                assert_eq!(id, 2);
                break;
            }
            frame => panic!("unexpected resource frame: {frame:?}"),
        }
    }
    assert_eq!(body, b"<p>hello</p>");
    client.shutdown(std::net::Shutdown::Both).unwrap();
    worker.join().unwrap();
    pump(&main_loop, &mut state);
    assert!(
        state
            .handle_request(&session_request(SessionCmd::OverlayClose, session, json!({})))
            .response
            .ok
    );
}

#[cfg(unix)]
#[test]
fn presentation_adoption_arms_hello_deadline_and_abandon_releases_the_stream() {
    use std::io::{BufRead as _, BufReader};
    let (mut state, main_loop, session, _, serial, _dir) = presented();
    state.core.presentation_closed(serial);
    assert!(!state.core.has_presentation_streams());
    let present = request(Command::Live(LiveCmd::Present), Some(session), json!({}));
    let reply = state.handle_request(&present);
    assert!(reply.response.ok, "{reply:?}");
    assert!(state.core.has_presentation_streams());
    reply.stream.unwrap().abandon();
    main_loop.drain(&mut state);
    assert!(!state.core.has_presentation_streams());
    // a viewer that sends no hello is closed at the deadline, which this loop runs at once
    let reply = state.handle_request(&present);
    assert!(reply.response.ok, "{reply:?}");
    main_loop.drain(&mut state);
    let (server, client) = ngterm_server::endpoint::pair().unwrap();
    client.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let owner = reply.stream.unwrap();
    let worker = std::thread::spawn(move || owner.adopt(server, Vec::new()));
    let mut line = String::new();
    let read = BufReader::new(client).read_line(&mut line).unwrap();
    assert_eq!(read, 0, "the stream outlived its hello deadline: {line:?}");
    worker.join().unwrap();
    pump(&main_loop, &mut state);
    assert!(!state.core.has_presentation_streams());
}

#[cfg(unix)]
#[test]
fn presentation_callbacks_obey_shutdown_and_release_failed_posts() {
    let (mut state, main_loop, _, lines, serial, _dir) = presented();
    let mut reader = presentation_reader(state.poster.clone(), serial);
    // a bad frame shuts the viewer's channel: the reader's task reaches the core
    reader(b"not a frame".to_vec());
    main_loop.drain(&mut state);
    assert_eq!(lines.shutdowns.get(), 1);
    state.save_on_quit();
    let shutdowns = lines.shutdowns.get();
    reader(b"not a frame".to_vec());
    presentation_closer(state.poster.clone(), serial)();
    main_loop.drain(&mut state);
    assert_eq!(
        lines.shutdowns.get(),
        shutdowns,
        "a line after the quit save reached the core"
    );
    assert!(
        state.core.has_presentation_streams(),
        "a close after the quit save reached the core"
    );
    // each failed post gives its inbound slot back, or the reader would block past the limit
    main_loop.closed.store(true, std::sync::atomic::Ordering::Release);
    for _ in 0..=ngterm_core::remote::INBOUND_LIMIT {
        reader(b"{}".to_vec());
    }
    drop(main_loop);
    reader(b"{}".to_vec());
}

#[test]
fn closing_a_session_arms_and_finalizes_its_pending_close() {
    let (mut state, main_loop, session) = fixture();
    let reply = state.handle_request(&session_request(SessionCmd::Close, session, json!({"undoable":true})));
    assert!(reply.response.ok, "{reply:?}");
    let window = state.core.library().active_window_id().unwrap();
    assert!(
        state
            .core
            .library()
            .store(window)
            .unwrap()
            .pending_close_summary()
            .is_some()
    );
    main_loop.drain(&mut state);
    assert!(
        state
            .core
            .library()
            .store(window)
            .unwrap()
            .pending_close_summary()
            .is_none()
    );
}

#[test]
fn a_pending_close_timer_does_nothing_once_quitting() {
    let (mut state, main_loop, session) = fixture();
    let reply = state.handle_request(&session_request(SessionCmd::Close, session, json!({"undoable":true})));
    assert!(reply.response.ok, "{reply:?}");
    // the quit save itself makes every pending close final, so only the phase stands in for it here
    state.phase = Phase::Quitting;
    main_loop.drain(&mut state);
    let window = state.core.library().active_window_id().unwrap();
    assert!(
        state
            .core
            .library()
            .store(window)
            .unwrap()
            .pending_close_summary()
            .is_some()
    );
}

fn active_session(state: &HeadlessState) -> Option<Uuid> {
    let window = state.core.library().active_window_id()?;
    state
        .core
        .library()
        .store(window)?
        .active_session()
        .map(ngterm_core::model::Session::id)
}

#[test]
fn an_auto_follow_timer_selects_a_blocked_session_only_until_the_quit() {
    use ngterm_core::model::{AgentIndicator, AgentStatus};
    let (mut state, _, first) = fixture();
    let created = state.handle_request(&request(
        Command::Session(SessionCmd::New),
        None,
        json!({"name":"waiting"}),
    ));
    assert!(created.response.ok, "{created:?}");
    let blocked: Uuid = created.response.result.unwrap()["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let window = state.core.library().active_window_id().unwrap();
    let store = state.core.library_mut().store_mut(window).unwrap();
    store.select_session(Some(first));
    let indicator = AgentIndicator {
        status: AgentStatus::Blocked,
        ..AgentIndicator::default()
    };
    store.set_agent_indicator(indicator, blocked);
    // the first take applies the (off) settings, so the explicit config stands from then on
    drop(state.core.take_auto_follow_timers());
    state.core.set_auto_follow(Some(ngterm_core::auto_follow::FollowConfig {
        timeout: Duration::ZERO,
        stay_on_active: false,
    }));
    let timers = state.core.take_auto_follow_timers();
    assert!(!timers.is_empty());
    state.phase = Phase::Quitting;
    for timer in timers.iter().copied() {
        state.fire_auto_follow(timer);
    }
    assert_eq!(active_session(&state), Some(first));
    state.phase = Phase::Running;
    for timer in timers {
        state.fire_auto_follow(timer);
    }
    assert_eq!(active_session(&state), Some(blocked));
}

/// A remote runner answering every command with one output.
struct Runner(String);

impl ngterm_core::remote::tree::RemoteCommandRunner for Runner {
    fn run(&self, _: &[String], _: Duration) -> ngterm_core::remote::tree::RemoteCommandResult {
        ngterm_core::remote::tree::RemoteCommandResult {
            status: 0,
            stdout: self.0.clone(),
            stderr: String::new(),
        }
    }
}

/// A presentation transport whose links note the key the last one opened with, and whether one was stopped.
#[derive(Clone, Default)]
struct Transport {
    key: std::rc::Rc<std::cell::Cell<Option<ngterm_core::remote::LinkKey>>>,
    stopped: std::rc::Rc<std::cell::Cell<bool>>,
}

impl ngterm_core::remote::PresentationTransport for Transport {
    fn open(&self, _: &[String], key: ngterm_core::remote::LinkKey) -> Box<dyn ngterm_core::remote::PresentationLink> {
        self.key.set(Some(key));
        Box::new(self.clone())
    }
}

impl ngterm_core::remote::PresentationLink for Transport {
    fn send(&self, _: &[u8]) {}
    fn stop(&self) {
        self.stopped.set(true);
    }
}

/// The fixture with another machine's session attached as a row, its presentation bridged over `transport`.
fn attached(transport: &Transport) -> (HeadlessState, Arc<Loop>) {
    let (mut state, main_loop, _) = fixture();
    let origin = Uuid::new_v4();
    let pane = Uuid::new_v4();
    let tree = json!({"ok":true,"result":{"remote":{
        "endpoint":{"executable":"/ngterm-session","socketDirectory":"/private"},"presentation":1,
        "sessions":[{"id":origin,"name":"remote build","windowID":"W","windowName":"main","workspaceID":"WS","workspaceName":"dev","cwd":"/","panes":[{"pane":"left","daemon":ngterm_core::live::daemon_name(pane)}]}]
    }}});
    state.core.set_remote_runner(Arc::new(Runner(tree.to_string())));
    state
        .core
        .set_presentation_transport(std::rc::Rc::new(transport.clone()));
    let response = state.core.dispatch(&request(
        Command::Live(LiveCmd::Attach),
        Some(origin),
        json!({"host":"origin"}),
    ));
    assert!(response.ok, "{response:?}");
    state.settle();
    main_loop.drain(&mut state);
    (state, main_loop)
}

#[test]
fn remote_ticks_keep_an_attached_viewer_alive_and_quit_stops_its_transport() {
    let transport = Transport::default();
    let (mut state, main_loop) = attached(&transport);
    assert!(!transport.stopped.get());
    assert!(state.core.remote_wake().is_some());
    state.save_on_quit();
    main_loop.drain(&mut state);
    assert!(transport.stopped.get());
}

#[test]
fn a_viewers_reports_reach_the_core_only_until_the_quit() {
    let reports: [fn(&mut HeadlessState, ngterm_core::remote::LinkKey); 2] = [
        |state, key| state.viewer_line(key, b"not a frame"),
        |state, key| state.viewer_closed(key, "closed"),
    ];
    for report in reports {
        let transport = Transport::default();
        let (mut state, _main_loop) = attached(&transport);
        let key = transport.key.get().expect("the attach opened a link");
        // the quit save stops every link itself, so only the phase stands in for it here
        state.phase = Phase::Quitting;
        report(&mut state, key);
        assert!(!transport.stopped.get(), "a report after the quit reached the core");
        // a bad frame or a close fails the link, which stops it
        state.phase = Phase::Running;
        report(&mut state, key);
        assert!(transport.stopped.get());
    }
}

#[test]
fn focus_timers_and_notification_effects_are_drained_without_a_display() {
    let (mut state, main_loop, session) = fixture();
    let response = state.handle_request(&session_request(SessionCmd::Split, session, json!({})));
    assert!(response.response.ok, "{response:?}");
    let response = state.handle_request(&session_request(SessionCmd::Focus, session, json!({"pane":"right"})));
    assert!(response.response.ok, "{response:?}");
    let response = state.handle_request(&request(
        Command::Notify,
        Some(session),
        json!({"title":"done","body":"build complete"}),
    ));
    assert!(response.response.ok, "{response:?}");
    main_loop.drain(&mut state);
    assert!(state.core.take_notifications().is_empty());
    state.save_on_quit();
    main_loop.drain(&mut state);
}

#[cfg(unix)]
#[test]
fn land_and_drain_deliver_real_terminal_reports_then_remove_closed_panes() {
    let (mut state, _, session) = fixture();
    let window = state.core.library().active_window_id().unwrap();
    {
        let found = state
            .core
            .library_mut()
            .store_mut(window)
            .unwrap()
            .session_mut(session)
            .unwrap();
        found.launch.command = Some("printf '\\033]2;worker-title\\007\\033]7;file://box/var/tmp\\007\\033]777;notify;task;complete\\007'; exit 0".into());
        found.launch.wait = true;
    }
    let config = ngterm_core::ghostty_config::GhosttyConfig::default();
    let (sender, wake) = std::sync::mpsc::channel();
    state.panes = Panes::new(
        PaneEnvironment {
            socket: "private.sock".into(),
            program_version: "test".into(),
            host: ngterm_term::Host::new(&config, Some("/bin/sh".into()), None, None),
            config,
            grid: (80, 24),
            hostname: "box".into(),
        },
        Arc::new(move |session, token| {
            let _ = sender.send((session, token));
        }),
    );
    state.land();
    assert_eq!(state.terminals(), 1);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let (session, token) = wake
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap();
        state.drain_pane(session, &token);
        let found = state.core.library().store(window).unwrap().session(session).unwrap();
        if found.osc_title.as_deref() == Some("worker-title") && found.current_cwd.as_deref() == Some("/var/tmp") {
            break;
        }
    }
    assert!(state.core.take_notifications().is_empty());
    state.land();
    assert_eq!(state.terminals(), 1);
    let reply = state.handle_request(&session_request(SessionCmd::Close, session, json!({})));
    assert!(reply.response.ok, "{reply:?}");
    state.land();
    assert_eq!(state.terminals(), 0);
    state.teardown();
}

#[test]
fn a_debounced_font_change_also_saves_the_latest_terminal_directory() {
    let (mut state, main_loop, _) = fixture();
    let dir = tempfile::tempdir().unwrap();
    let env = ngterm_core::paths::MapEnv::new()
        .with("HOME", dir.path())
        .with("NGTERM_STATE_DIR", dir.path());
    state.core = Core::new(
        CoreConfig {
            app: crate::identity(),
            capabilities: Vec::new(),
        },
        crate::load_state(&env),
    );
    let window = state.core.library().active_window_id().unwrap();
    let session = state
        .core
        .library()
        .store(window)
        .unwrap()
        .sessions()
        .next()
        .unwrap()
        .id();
    let token = {
        let found = state
            .core
            .library_mut()
            .store_mut(window)
            .unwrap()
            .session_mut(session)
            .unwrap();
        let token = ngterm_core::pane_env::uuid_string(found.pane_identity);
        found
            .pane_slot_mut(ngterm_core::model::StatusPane::Left)
            .replace(Box::new(ngterm_host::fakes::FakeSurface::new(&token)));
        token
    };
    assert!(
        state
            .core
            .report_pane(session, &token, &ngterm_core::PaneReport::Pwd("/var/tmp".into()))
    );
    state
        .core
        .library_mut()
        .store_mut(window)
        .unwrap()
        .set_font_size(session, 16.0);
    state.settle();
    main_loop.drain(&mut state);
    let restored = crate::load_state(&env);
    assert_eq!(
        restored
            .library
            .store(window)
            .unwrap()
            .session(session)
            .unwrap()
            .effective_cwd(),
        "/var/tmp"
    );
    state.save_on_quit();
    main_loop.drain(&mut state);
}
