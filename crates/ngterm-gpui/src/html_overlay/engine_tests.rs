//! The forwarding adapter against the real worker, with isolated storage and process exit.

use std::process::Stdio;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use ngterm_wpe_client::{Event, FrameMode, HostConfig, InjectionTime, MessageHandler, Settings};

use super::*;

const CHILD: &str = "html_overlay::engine::tests::real_engine_child";
const WAIT: Duration = Duration::from_secs(30);
/// The child's exit code when it ran and passed: a stale `CHILD` selects no test, and that exits 0.
const PASSED: i32 = 77;

#[test]
fn engine_commands_reach_webkit_and_its_replies_reach_the_page() {
    let home = tempfile::tempdir().unwrap();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", CHILD, "--ignored", "--nocapture"])
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for key in [
        "PATH",
        "LD_LIBRARY_PATH",
        "LLVM_PROFILE_FILE",
        "DISPLAY",
        "DBUS_SESSION_BUS_ADDRESS",
    ] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    for (key, directory) in [
        ("HOME", "home"),
        ("XDG_DATA_HOME", "data"),
        ("XDG_CACHE_HOME", "cache"),
        ("XDG_CONFIG_HOME", "config"),
        ("XDG_RUNTIME_DIR", "run"),
    ] {
        let path = home.path().join(directory);
        std::fs::create_dir(&path).unwrap();
        command.env(key, path);
    }
    command.env("HTML_ENGINE_CHILD", "1").env("LIBGL_ALWAYS_SOFTWARE", "1");
    let child = command.spawn().unwrap();
    let pid = i32::try_from(child.id()).unwrap();
    let (send, received) = mpsc::channel();
    let waiter = std::thread::spawn(move || send.send(child.wait_with_output()).unwrap());
    let output = received.recv_timeout(Duration::from_secs(120));
    if output.is_err() {
        // SAFETY: the pid is our unreaped child; this ends only that isolated test process (bwrap ends it with itself).
        unsafe { libc::kill(pid, libc::SIGKILL) };
    }
    waiter.join().unwrap();
    let output = output.expect("the engine child exceeded its deadline").unwrap();
    assert_eq!(
        output.status.code(),
        Some(PASSED),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

struct Fixture {
    engine: Engine,
    queue: async_channel::Receiver<Event>,
    events: mpsc::Receiver<Event>,
    pump: std::thread::JoinHandle<()>,
}

impl Fixture {
    fn start() -> Self {
        let worker = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("ngterm-web");
        ngterm_wpe_client::worker::pin_worker_path(&worker).unwrap();
        let (host, queue) = Host::start_worker(
            &HostConfig {
                frames: FrameMode::Shm,
                schemes: vec!["overlay-test".to_owned()],
            },
            &ngterm_wpe_client::EngineSource::System,
        )
        .unwrap();
        let (send, events) = mpsc::channel();
        let input = queue.clone();
        let pump = std::thread::spawn(move || {
            while let Ok(event) = input.recv_blocking() {
                if send.send(event).is_err() {
                    break;
                }
            }
        });
        Self {
            engine: Engine(Arc::new(host)),
            queue,
            events,
            pump,
        }
    }

    fn wait(&self, wanted: impl Fn(&Event) -> bool) -> Event {
        let deadline = Instant::now() + WAIT;
        loop {
            let event = self
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("the engine event did not arrive");
            match &event {
                Event::Policy(request) => self.engine.decide_policy(request.token, true),
                Event::Frame(frame) => self.engine.presenter()(frame.view, frame.buffer),
                Event::SchemeRequest(request) => self.engine.finish_scheme_request(request.token, SchemeResponse::Bytes {
                    data: b"<title>Loaded</title><input id='field'><script>window.keys=[];document.onkeydown=e=>keys.push(e.key)</script>".to_vec(),
                    mime: "text/html".to_owned(),
                }),
                Event::ScriptMessage(message) => self.engine.reply_script_message(message.token, Ok("{\"answer\":42}".to_owned())),
                _ => {}
            }
            if wanted(&event) {
                return event;
            }
        }
    }

    fn loaded(&self, view: ViewId) {
        self.wait(
            |event| matches!(event, Event::Load { view: id, event: ngterm_wpe_client::LoadEvent::Finished } if *id == view),
        );
    }

    fn evaluate(&self, view: ViewId, script: &str) -> String {
        self.evaluate_answer(view, script)
            .unwrap()
            .unwrap_or_else(|| panic!("script returned undefined: {script}"))
    }

    fn evaluate_answer(&self, view: ViewId, script: &str) -> ngterm_wpe_client::Evaluation {
        let queue = self.engine.0.evaluate(view, script.to_owned(), None);
        let input = queue.clone();
        let (send, receive) = mpsc::channel();
        let task = std::thread::spawn(move || {
            let _ = send.send(input.recv_blocking());
        });
        let answer = receive.recv_timeout(WAIT);
        queue.close();
        task.join().unwrap();
        answer.expect("the script exceeded its deadline").unwrap()
    }

    fn stop(self) {
        self.engine.shut_down();
        assert!(self.queue.is_closed(), "shutdown ends the engine event stream");
        self.queue.close();
        self.pump.join().unwrap();
    }
}

#[test]
#[ignore = "run by engine_commands_reach_webkit_and_its_replies_reach_the_page in an isolated process"]
fn real_engine_child() {
    if std::env::var_os("HTML_ENGINE_CHILD").is_none() {
        return;
    }
    let result = std::panic::catch_unwind(|| {
        let fixture = Fixture::start();
        let session = fixture.engine.create_session(SessionSpec::Ephemeral);
        let spec = ViewSpec {
            session,
            geometry: Geometry {
                width: 300,
                height: 200,
                scale: 1.0,
            },
            settings: Settings::overlay(true),
            scripts: Vec::new(),
            message_handlers: vec![MessageHandler {
                name: "test".to_owned(),
                world: None,
            }],
            zoom: 1.0,
            background: None,
            block_network: false,
        };
        let view = fixture.engine.create_view(spec.clone());
        history_and_scripts(&fixture, view);
        input_and_geometry(&fixture, view);
        fixture.engine.clear_session(session);
        let cleared =
            fixture.wait(|event| matches!(event, Event::SessionCleared { session: id, .. } if *id == session));
        assert!(matches!(cleared, Event::SessionCleared { result: Ok(()), .. }));
        fixture.engine.close_view(view);
        assert_eq!(fixture.evaluate_answer(view, "1"), Err("no such page".to_owned()));
        fixture.engine.drop_session(session);
        let refused = fixture.engine.create_view(spec);
        let error = fixture.wait(|event| matches!(event, Event::ViewFailed { view, .. } if *view == refused));
        assert!(matches!(error, Event::ViewFailed { message, .. } if message == "no such browser store"));
        fixture.stop();
    });
    std::process::exit(if result.is_ok() { PASSED } else { 101 });
}

fn history_and_scripts(fixture: &Fixture, view: ViewId) {
    let web = &fixture.engine;
    web.load(view, Load::Uri("overlay-test:///first".to_owned()));
    fixture.loaded(view);
    assert_eq!(fixture.evaluate(view, "document.title"), "\"Loaded\"");
    web.load(view, Load::Uri("overlay-test:///second".to_owned()));
    fixture.loaded(view);
    web.go_back(view);
    fixture.loaded(view);
    assert_eq!(fixture.evaluate(view, "location.pathname"), "\"/first\"");
    web.go_forward(view);
    fixture.loaded(view);
    assert_eq!(fixture.evaluate(view, "location.pathname"), "\"/second\"");
    web.set_scripts(
        view,
        vec![UserScript {
            source: "window.injected=73".to_owned(),
            time: InjectionTime::Start,
            world: None,
        }],
    );
    web.reload(view);
    fixture.loaded(view);
    assert_eq!(fixture.evaluate(view, "window.injected"), "73");
    fixture.evaluate(
        view,
        "window.webkit.messageHandlers.test.postMessage({}).then(x=>window.webkit.messageHandlers.test.postMessage({resolved:x}));0",
    );
    fixture.wait(|event| matches!(event, Event::ScriptMessage(_)));
    let resolved = fixture.wait(|event| matches!(event, Event::ScriptMessage(_)));
    assert!(
        matches!(resolved, Event::ScriptMessage(message) if message.body.as_deref() == Some("{\"resolved\":{\"answer\":42}}"))
    );
}

fn input_and_geometry(fixture: &Fixture, view: ViewId) {
    let web = &fixture.engine;
    web.set_geometry(
        view,
        Geometry {
            width: 600,
            height: 300,
            scale: 1.0,
        },
    );
    web.set_zoom(view, 2.0);
    web.set_visible(view, false);
    web.set_visible(view, true);
    web.set_focused(view, true);
    fixture.evaluate(view, "document.getElementById('field').focus();0");
    fixture.evaluate(view, "field.addEventListener('compositionstart',()=>document.title='composing');field.addEventListener('compositionend',e=>document.title='composed:'+e.data);0");
    for kind in [
        ngterm_wpe_client::PointerKind::Down(ngterm_wpe_client::Button::Primary),
        ngterm_wpe_client::PointerKind::Up(ngterm_wpe_client::Button::Primary),
    ] {
        web.input(
            view,
            Input::Pointer(ngterm_wpe_client::Pointer {
                kind,
                x: 20.,
                y: 20.,
                modifiers: ngterm_wpe_client::Modifiers::empty(),
            }),
        );
    }
    web.ime_preedit(view, "preedit".to_owned(), 7);
    fixture.wait(|event| matches!(event, Event::Page { page, .. } if page.title.as_deref() == Some("composing")));
    web.ime_commit(view, "committed".to_owned());
    fixture
        .wait(|event| matches!(event, Event::Page { page, .. } if page.title.as_deref() == Some("composed:committed")));
    assert_eq!(
        fixture.evaluate(view, "document.getElementById('field').value"),
        "\"committed\""
    );
    fixture.evaluate(
        view,
        "field.addEventListener('input',()=>document.title='input:'+field.value);0",
    );
    web.set_clipboard("pasted".to_owned());
    web.editing_command(view, EditingCommand::SelectAll);
    web.editing_command(view, EditingCommand::Paste);
    fixture.wait(|event| matches!(event, Event::Page { page, .. } if page.title.as_deref() == Some("input:pasted")));
    assert_eq!(
        fixture.evaluate(view, "document.getElementById('field').value"),
        "\"pasted\""
    );
    fixture.evaluate(view, "window.keys=[];0");
    web.input(
        view,
        Input::Key(ngterm_wpe_client::Key {
            pressed: true,
            keyval: 0xff1b,
            keycode: None,
            modifiers: ngterm_wpe_client::Modifiers::empty(),
        }),
    );
    assert_eq!(
        fixture.evaluate(view, "JSON.stringify(window.keys)"),
        "\"[\\\"Escape\\\"]\""
    );
    assert_eq!(fixture.evaluate(view, "innerWidth"), "300");
    // an SHM engine leaves the page alone (a dmabuf one would reload it): its script state stays
    web.fall_back_to_shm(view);
    assert_eq!(
        fixture.evaluate(view, "JSON.stringify(window.keys)"),
        "\"[\\\"Escape\\\"]\""
    );
}
