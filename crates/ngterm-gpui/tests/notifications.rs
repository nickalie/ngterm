//! Desktop notifications on a private session bus: a `dbus-daemon` of the test's own (no service directories, so
//! nothing on it can be activated) with a fake `org.freedesktop.Notifications` server, never the user's bus. The
//! banner task connects to the session bus, `DBUS_SESSION_BUS_ADDRESS`, so the scenario runs in a child of this test
//! binary whose environment holds the private bus alone.
//!
//! The scenario: a repeat from one pane replaces its banner (`replaces_id`), another pane gets its own, a click on a
//! banner (`ActionInvoked` with the `default` key) reveals its identity with the activation token the server sent
//! before it (`ActivationToken`), a banner the server closed (`NotificationClosed`) is forgotten, a failed repeat keeps
//! the banner still on screen, signals from anyone but the server count for nothing, a clear withdraws the
//! session's banners still on screen (`CloseNotification`), and a server that took over the name (the old one
//! restarted) never gets the old one's ids: no `replaces_id` and no `CloseNotification` of them. A `notify` banner
//! takes the session's control slot (upstream #772): the main pane's OSC 9 beside it gets a banner of its own, a second
//! `notify` replaces the first, a clear withdraws the control banner with the panes', and a click on it reveals the
//! main pane.
//!
//! A second scenario, on a bus its child starts and restarts itself, meets a server's quirks: an activation token sent
//! before `Notify`'s reply counts for its banner, signals of another member or body count for nothing, a repeat the
//! server posts under a new id is clicked by that id alone, a refused `CloseNotification` still forgets its banner, and
//! a name left without an owner forgets the banners known; then the bus restarts under the banner task, whose banners
//! while it is down are dropped and whose next one reaches the new bus. Last, the app itself: its banner's click
//! raises its window with the server's token.
//!
//! Linux only: the freedesktop notification server on a private D-Bus bus.

#![cfg(target_os = "linux")]

#[macro_use]
#[expect(
    dead_code,
    unused_imports,
    reason = "the sandbox serves the process tests too; this one runs headless, without lavapipe"
)]
mod support;

use std::collections::HashMap;
use std::io::{BufRead as _, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ngterm_core::{
    BannerSlot, NotificationEffect, NotificationPost, PaneRole, control_notification_identity, notification_identity,
    parse_notification_identity,
};
use ngterm_gpui::notifications::{BannerSink as _, Click, Desktop, deliver, run};
use uuid::Uuid;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::OwnedValue;

/// Names the private bus the child runs on; the child refuses any other.
const PRIVATE_BUS: &str = "NGTERM_GPUI_TEST_PRIVATE_BUS";
const PATH: &str = "/org/freedesktop/Notifications";
const DEADLINE: Duration = Duration::from_secs(20);
const APP_ID: &str = "com.nickalie.ngterm.test";
/// The body the fake server refuses to post.
const FAILING: &str = "fail";

/// A session bus that listens on `socket` only, activates nothing and lets every client talk to every other.
fn bus_config(socket: &Path) -> String {
    format!(
        r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:path={}</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
"#,
        socket.display()
    )
}

fn dbus_daemon() -> Option<std::path::PathBuf> {
    ["/usr/bin/dbus-daemon", "/bin/dbus-daemon"]
        .into_iter()
        .map(std::path::PathBuf::from)
        .find(|path| path.is_file())
}

#[test]
fn banners_reach_a_private_bus_replace_click_and_withdraw() {
    let Some(daemon) = dbus_daemon() else {
        eprintln!("skipped: no dbus-daemon to run a private session bus on");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("bus.conf");
    std::fs::write(&config, bus_config(&dir.path().join("bus"))).unwrap();
    let mut bus = Command::new(daemon)
        .arg(format!("--config-file={}", config.display()))
        .args(["--nofork", "--print-address=1"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut address = String::new();
    BufReader::new(bus.stdout.take().unwrap())
        .read_line(&mut address)
        .unwrap();
    let address = address.trim().to_owned();
    assert!(address.starts_with("unix:"), "{address}");
    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "on_the_private_bus",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env_clear()
        // a coverage run's profile destination (cargo llvm-cov), where this child's coverage goes
        .envs(std::env::var_os("LLVM_PROFILE_FILE").map(|profile| ("LLVM_PROFILE_FILE", profile)))
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", dir.path())
        .env("DBUS_SESSION_BUS_ADDRESS", &address)
        .env(PRIVATE_BUS, &address)
        .status();
    let _ = bus.kill();
    let _ = bus.wait();
    assert!(status.unwrap().success(), "the scenario failed on the private bus");
}

/// What the fake server was asked, in order.
#[derive(Debug, Clone, PartialEq)]
enum Call {
    Notify {
        app: String,
        icon: String,
        replaces: u32,
        id: u32,
        body: String,
        actions: Vec<String>,
        desktop_entry: Option<String>,
    },
    Close(u32),
}

/// A notification server that records its calls and numbers banners from 1.
struct Server {
    calls: Arc<Mutex<Vec<Call>>>,
    last: u32,
}

/// The notification server's bus name.
const SERVER_NAME: &str = "org.freedesktop.Notifications";

/// A fake server recording into `calls`, serving on the bus at `address` under the server's name.
fn serve(address: &str, calls: &Arc<Mutex<Vec<Call>>>) -> TestResult<zbus::blocking::Connection> {
    Ok(zbus::blocking::connection::Builder::address(address)?
        .serve_at(
            PATH,
            Server {
                calls: Arc::clone(calls),
                last: 0,
            },
        )?
        .name(SERVER_NAME)?
        .build()?)
}

/// The `Notify` call as the server reads it.
struct Notify {
    app: String,
    icon: String,
    replaces: u32,
    body: String,
    actions: Vec<String>,
    hints: HashMap<String, OwnedValue>,
}

impl Server {
    fn notify(&mut self, request: Notify) -> zbus::fdo::Result<u32> {
        if request.body == FAILING {
            return Err(zbus::fdo::Error::Failed("refused".to_owned()));
        }
        let id = if request.replaces == 0 {
            self.last += 1;
            self.last
        } else {
            request.replaces
        };
        let desktop_entry = request
            .hints
            .get("desktop-entry")
            .and_then(|value| String::try_from(value.try_clone().ok()?).ok());
        record(
            &self.calls,
            Call::Notify {
                app: request.app,
                icon: request.icon,
                replaces: request.replaces,
                id,
                body: request.body,
                actions: request.actions,
                desktop_entry,
            },
        );
        Ok(id)
    }
}

fn record(calls: &Mutex<Vec<Call>>, call: Call) {
    if let Ok(mut calls) = calls.lock() {
        calls.push(call);
    }
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Server {
    #[zbus(name = "Notify")]
    #[expect(
        clippy::too_many_arguments,
        reason = "the freedesktop `Notify` method's eight arguments"
    )]
    fn notify_method(
        &mut self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
    ) -> zbus::fdo::Result<u32> {
        drop((summary, expire_timeout));
        self.notify(Notify {
            app: app_name,
            icon: app_icon,
            replaces: replaces_id,
            body,
            actions,
            hints,
        })
    }

    fn close_notification(&mut self, id: u32) {
        record(&self.calls, Call::Close(id));
    }

    #[zbus(signal)]
    async fn action_invoked(emitter: &SignalEmitter<'_>, id: u32, action_key: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn activation_token(emitter: &SignalEmitter<'_>, id: u32, activation_token: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn notification_closed(emitter: &SignalEmitter<'_>, id: u32, reason: u32) -> zbus::Result<()>;
}

/// Waits until `done` holds of the recorded calls, or fails.
fn wait_for(calls: &Mutex<Vec<Call>>, what: &str, done: impl Fn(&[Call]) -> bool) -> Vec<Call> {
    let deadline = Instant::now() + DEADLINE;
    loop {
        let seen = calls.lock().map(|calls| calls.clone()).unwrap_or_default();
        if done(&seen) {
            return seen;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}: {seen:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn notifies(calls: &[Call]) -> Vec<(u32, u32, String)> {
    calls
        .iter()
        .filter_map(|call| match call {
            Call::Notify { replaces, id, body, .. } => Some((*replaces, *id, body.clone())),
            Call::Close(_) => None,
        })
        .collect()
}

/// What a scenario step returns: a failed bus call fails the scenario.
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// The scenario's parts: the fake server on the private bus, what it was asked, the banner task's sink and clicks, and
/// the three panes' identities of one session.
struct Scenario {
    address: String,
    calls: Arc<Mutex<Vec<Call>>>,
    server: zbus::blocking::Connection,
    sink: Desktop,
    clicks: std::sync::mpsc::Receiver<Click>,
    window: Uuid,
    session: Uuid,
    main: String,
    split: String,
    overlay: String,
}

impl Scenario {
    /// The fake server serving on the bus at `address`, and the banner task running beside it.
    fn start(address: String) -> TestResult<Self> {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let server = serve(&address, &calls)?;
        let (sink, queue) = Desktop::new();
        let (clicked, clicks) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            zbus::block_on(run(queue, APP_ID.to_owned(), move |click| {
                let _ = clicked.send(click);
            }));
        });
        let (window, session) = (Uuid::new_v4(), Uuid::new_v4());
        Ok(Self {
            address,
            calls,
            server,
            sink,
            clicks,
            window,
            session,
            main: notification_identity(window, session, PaneRole::Main),
            split: notification_identity(window, session, PaneRole::Split),
            overlay: notification_identity(window, session, PaneRole::Overlay),
        })
    }

    /// The fake server's interface, whose emitter sends its signals.
    fn interface(&self) -> TestResult<zbus::blocking::object_server::InterfaceRef<Server>> {
        Ok(self.server.object_server().interface::<_, Server>(PATH)?)
    }

    /// The next click, or a failure.
    fn click(&self) -> TestResult<Click> {
        Ok(self.clicks.recv_timeout(DEADLINE)?)
    }
}

/// Sends a signal of the fake server's.
fn emit(signal: impl std::future::Future<Output = zbus::Result<()>>) -> TestResult {
    Ok(zbus::block_on(signal)?)
}

#[test]
#[ignore = "run by banners_reach_a_private_bus_replace_click_and_withdraw, on its private bus"]
fn on_the_private_bus() -> TestResult {
    let address = std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap_or_default();
    // only ever on the private bus the parent test started and named: never the user's
    if address.is_empty() || std::env::var(PRIVATE_BUS).ok().as_deref() != Some(address.as_str()) {
        eprintln!("skipped: not on the parent test's private bus");
        return Ok(());
    }
    let scenario = Scenario::start(address)?;
    repeats_replace_their_banner(&scenario);
    a_click_carries_the_servers_token(&scenario)?;
    refused_and_closed_banners(&scenario)?;

    let clear = NotificationEffect::Clear {
        window: scenario.window,
        session: scenario.session,
    };
    deliver(&scenario.sink, &[clear]);
    let seen = wait_for(&scenario.calls, "the withdrawal", |calls| {
        calls.contains(&Call::Close(2))
    });
    let closes: Vec<_> = seen.iter().filter(|call| matches!(call, Call::Close(_))).collect();
    assert_eq!(
        closes,
        [&Call::Close(2)],
        "only the split's banner is still up, kept through its refused repeat"
    );
    assert_eq!(notifies(&seen).len(), 5, "a clear posts nothing");
    let (successor, restarted) = a_restarted_server_gets_none_of_the_old_ids(&scenario)?;
    a_notify_has_its_own_slot(&scenario, &successor, &restarted)
}

/// The server's name passes to another server that numbers its banners from 1 again (the old one restarted without
/// closing its banners): the new one gets none of the old one's ids, neither as a repeat's `replaces_id` nor in a
/// withdrawal, and its signals are the ones that count. The server now serving, and what it was asked.
fn a_restarted_server_gets_none_of_the_old_ids(
    scenario: &Scenario,
) -> TestResult<(zbus::blocking::Connection, Arc<Mutex<Vec<Call>>>)> {
    let Scenario {
        calls,
        sink,
        main,
        split,
        overlay,
        ..
    } = scenario;
    sink.send(main, "t", "six");
    sink.send(split, "t", "seven");
    let before = wait_for(calls, "the banners before the restart", |calls| {
        notifies(calls).len() == 7
    });
    assert_eq!(
        notifies(&before)[5..],
        [(0, 5, "six".to_owned()), (0, 6, "seven".to_owned())]
    );

    scenario.server.release_name(SERVER_NAME)?;
    let restarted = Arc::new(Mutex::new(Vec::new()));
    let successor = serve(&scenario.address, &restarted)?;
    sink.send(main, "t", "eight");
    deliver(
        sink,
        &[NotificationEffect::Clear {
            window: scenario.window,
            session: scenario.session,
        }],
    );
    sink.send(overlay, "t", "nine");
    sink.send(split, "t", "ten");
    let seen = wait_for(&restarted, "the banners after the restart", |calls| {
        notifies(calls).len() == 3
    });
    let summary: Vec<_> = seen
        .iter()
        .map(|call| match call {
            Call::Notify { replaces, id, body, .. } => format!("notify {replaces} {id} {body}"),
            Call::Close(id) => format!("close {id}"),
        })
        .collect();
    assert_eq!(
        summary,
        ["notify 0 1 eight", "close 1", "notify 0 2 nine", "notify 0 3 ten"],
        "the new server replaces and withdraws none of the old one's banners, only its own"
    );
    assert_eq!(
        calls.lock().map(|calls| calls.len()).unwrap_or_default(),
        before.len(),
        "the old server gets nothing more"
    );

    // the old server's click counts for nothing now; the new one's does
    let old = scenario.interface()?;
    emit(Server::action_invoked(old.signal_emitter(), 3, "default"))?;
    let new = successor.object_server().interface::<_, Server>(PATH)?;
    emit(Server::action_invoked(new.signal_emitter(), 3, "default"))?;
    assert_eq!(
        scenario.click()?,
        Click {
            identity: split.clone(),
            token: None
        },
        "the new server's click reveals its banner's pane; the old server's counts for nothing"
    );
    Ok((successor, restarted))
}

/// The core's banner for `session` of `window` in `slot`.
fn post(window: Uuid, session: Uuid, slot: BannerSlot, body: &str) -> NotificationEffect {
    NotificationEffect::Post(NotificationPost {
        window,
        session,
        slot,
        title: "t".to_owned(),
        body: body.to_owned(),
    })
}

/// Upstream #772, on another session served by `server`, which has handed out ids 1 to 3 (`calls`): a `notify` and the
/// main pane's OSC 9 give two banners, no `replaces_id` between them; a second `notify` replaces the first; a clear
/// (the session took focus) closes the control banner too; a click on a control banner reveals the main pane.
fn a_notify_has_its_own_slot(
    scenario: &Scenario,
    server: &zbus::blocking::Connection,
    calls: &Mutex<Vec<Call>>,
) -> TestResult {
    let (window, session) = (Uuid::new_v4(), Uuid::new_v4());
    let control = control_notification_identity(window, session);
    let sink = &scenario.sink;
    deliver(
        sink,
        &[
            post(window, session, BannerSlot::Control, "note"),
            post(window, session, BannerSlot::Pane(PaneRole::Main), "osc"),
            post(window, session, BannerSlot::Control, "again"),
        ],
    );
    let seen = wait_for(calls, "the note, the terminal's banner and the repeat", |calls| {
        notifies(calls).len() == 6
    });
    assert_eq!(
        notifies(&seen)[3..],
        [
            (0, 4, "note".to_owned()),
            (0, 5, "osc".to_owned()),
            (4, 4, "again".to_owned())
        ],
        "the terminal's banner replaces no note; a second note replaces the first"
    );

    deliver(sink, &[NotificationEffect::Clear { window, session }]);
    let seen = wait_for(calls, "the session's withdrawal", |calls| {
        calls.contains(&Call::Close(4))
    });
    let closes: Vec<_> = seen
        .iter()
        .filter(|call| matches!(call, Call::Close(id) if *id >= 4))
        .collect();
    assert_eq!(
        closes,
        [&Call::Close(5), &Call::Close(4)],
        "a clear withdraws the main pane's banner and the control one"
    );

    deliver(sink, &[post(window, session, BannerSlot::Control, "later")]);
    let seen = wait_for(calls, "the next note", |calls| notifies(calls).len() == 7);
    assert_eq!(
        notifies(&seen)[6],
        (0, 6, "later".to_owned()),
        "a withdrawn note is replaced by none"
    );
    let interface = server.object_server().interface::<_, Server>(PATH)?;
    emit(Server::action_invoked(interface.signal_emitter(), 6, "default"))?;
    let click = scenario.click()?;
    assert_eq!(click.identity, control, "the click names the control slot");
    assert_eq!(
        parse_notification_identity(&click.identity),
        Some((window, session, PaneRole::Main)),
        "a click on a note reveals the main pane"
    );
    Ok(())
}

/// A banner the server can click, under the app's desktop entry and icon; a repeat from one pane replaces its banner, another
/// pane gets its own.
fn repeats_replace_their_banner(scenario: &Scenario) {
    let Scenario {
        calls,
        sink,
        main,
        split,
        ..
    } = scenario;
    sink.send(main, "t", "one");
    let seen = wait_for(calls, "the first banner", |calls| notifies(calls).len() == 1);
    assert_eq!(
        seen[0],
        Call::Notify {
            app: "ngterm".to_owned(),
            icon: ngterm_gpui::instance::DEFAULT_APP_ID.to_owned(),
            replaces: 0,
            id: 1,
            body: "one".to_owned(),
            actions: vec!["default".to_owned(), String::new()],
            desktop_entry: Some(APP_ID.to_owned()),
        },
        "a banner the server can click, under the app's desktop entry and icon"
    );
    sink.send(main, "t", "two");
    sink.send(split, "t", "three");
    let seen = wait_for(calls, "the repeat and the split's banner", |calls| {
        notifies(calls).len() == 3
    });
    assert_eq!(
        notifies(&seen),
        [
            (0, 1, "one".to_owned()),
            (1, 1, "two".to_owned()),
            (0, 2, "three".to_owned())
        ],
        "a repeat from one pane replaces its banner; another pane gets its own"
    );
}

/// A click reveals the pane its banner names, with the activation token the server sent before it.
fn a_click_carries_the_servers_token(scenario: &Scenario) -> TestResult {
    let interface = scenario.interface()?;
    let emitter = interface.signal_emitter();
    emit(Server::activation_token(emitter, 1, "token-1"))?;
    emit(Server::action_invoked(emitter, 1, "default"))?;
    assert_eq!(
        scenario.click()?,
        Click {
            identity: scenario.main.clone(),
            token: Some("token-1".to_owned())
        },
        "a click reveals the pane its banner names, with the server's activation token"
    );
    Ok(())
}

/// A repeat the server refuses leaves the split's banner known; a clicked banner and one the server closed are
/// forgotten; a signal from a peer that is not the server counts for nothing.
fn refused_and_closed_banners(scenario: &Scenario) -> TestResult {
    let Scenario {
        calls,
        sink,
        main,
        split,
        overlay,
        ..
    } = scenario;
    sink.send(split, "t", FAILING);
    sink.send(main, "t", "four");
    sink.send(overlay, "t", "five");
    let seen = wait_for(calls, "the banners after the refused one", |calls| {
        notifies(calls).len() == 5
    });
    assert_eq!(
        notifies(&seen)[3..],
        [(0, 3, "four".to_owned()), (0, 4, "five".to_owned())],
        "a clicked banner is forgotten: its pane's next banner replaces nothing"
    );

    // the server closed the main pane's banner: a click on it after reveals nothing; nor does a peer's signal; the
    // overlay's click, sent last, is the next one seen
    let interface = scenario.interface()?;
    let emitter = interface.signal_emitter();
    emit(Server::notification_closed(emitter, 3, 2))?;
    emit(Server::action_invoked(emitter, 3, "default"))?;
    let peer = zbus::blocking::connection::Builder::address(scenario.address.as_str())?.build()?;
    peer.emit_signal(
        None::<&str>,
        PATH,
        "org.freedesktop.Notifications",
        "ActionInvoked",
        &(2_u32, "default"),
    )?;
    emit(Server::action_invoked(emitter, 4, "default"))?;
    assert_eq!(
        scenario.click()?,
        Click {
            identity: overlay.clone(),
            token: None
        },
        "a closed banner and a peer's signal reveal nothing; a click without a token carries none"
    );
    Ok(())
}

/// Names the configuration the second scenario's child starts its own private bus daemons from, and the daemon.
const BUS_CONFIG: &str = "NGTERM_GPUI_TEST_BUS_CONFIG";
const BUS_DAEMON: &str = "NGTERM_GPUI_TEST_BUS_DAEMON";
/// A body the quirky server answers only after sending its banner's activation token.
const EARLY: &str = "early";
/// A body whose repeat the quirky server posts under a new id.
const RENUMBER: &str = "renumber";
/// A body whose banner the quirky server refuses to close.
const STUCK: &str = "stuck";
const INTERFACE: &str = "org.freedesktop.Notifications";

/// The second scenario, in a child whose session bus is a daemon it starts, stops and starts again itself: a server's
/// quirks (an activation token sent before `Notify`'s reply, signals the specification does not name or that carry
/// another body, a repeat posted under a new id, a refused `CloseNotification`), the server's name left without an
/// owner, and the bus restarting under the banner task.
#[test]
fn a_quirky_server_and_a_bus_restart_on_a_private_bus() {
    let Some(daemon) = dbus_daemon() else {
        eprintln!("skipped: no dbus-daemon to run a private session bus on");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("bus.conf");
    std::fs::write(&config, bus_config(&dir.path().join("bus"))).unwrap();
    let address = format!("unix:path={}", dir.path().join("bus").display());
    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "quirks_on_the_private_bus",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env_clear()
        // a coverage run's profile destination (cargo llvm-cov), where this child's coverage goes
        .envs(std::env::var_os("LLVM_PROFILE_FILE").map(|profile| ("LLVM_PROFILE_FILE", profile)))
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", dir.path())
        .env("DBUS_SESSION_BUS_ADDRESS", &address)
        .env(PRIVATE_BUS, &address)
        .env(BUS_CONFIG, &config)
        .env(BUS_DAEMON, &daemon)
        .status();
    assert!(status.unwrap().success(), "the scenario failed on the private bus");
}

/// A private bus daemon at the scenario's socket, killed when dropped.
struct Daemon(std::process::Child);

impl Daemon {
    /// The daemon `program` serving `config`, once it listens; a stopped daemon's socket at `socket` is removed first.
    fn start(program: &Path, config: &Path, socket: &Path) -> TestResult<Self> {
        let _ = std::fs::remove_file(socket);
        let mut child = Command::new(program)
            .arg(format!("--config-file={}", config.display()))
            .args(["--nofork", "--print-address=1"])
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .stdout(Stdio::piped())
            .spawn()?;
        let mut address = String::new();
        BufReader::new(child.stdout.take().ok_or("no daemon output")?).read_line(&mut address)?;
        let daemon = Self(child);
        if !address.starts_with("unix:") {
            return Err(format!("the daemon gave no address: {address:?}").into());
        }
        Ok(daemon)
    }

    fn stop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A notification server with real servers' quirks (the bodies [`EARLY`], [`RENUMBER`] and [`STUCK`] ask for them):
/// records its calls and numbers banners from 1.
struct Quirky {
    calls: Arc<Mutex<Vec<Call>>>,
    last: u32,
    bodies: HashMap<u32, String>,
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Quirky {
    #[zbus(name = "Notify")]
    #[expect(
        clippy::too_many_arguments,
        reason = "the freedesktop `Notify` method's eight arguments and the emitter"
    )]
    async fn notify_method(
        &mut self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<u32> {
        drop((summary, expire_timeout));
        let id = if replaces_id == 0 || body == RENUMBER {
            self.last += 1;
            self.last
        } else {
            replaces_id
        };
        if body == EARLY {
            Self::activation_token(&emitter, id, "early-token")
                .await
                .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;
        }
        self.bodies.insert(id, body.clone());
        let desktop_entry = hints
            .get("desktop-entry")
            .and_then(|value| String::try_from(value.try_clone().ok()?).ok());
        record(
            &self.calls,
            Call::Notify {
                app: app_name,
                icon: app_icon,
                replaces: replaces_id,
                id,
                body,
                actions,
                desktop_entry,
            },
        );
        Ok(id)
    }

    fn close_notification(&mut self, id: u32) -> zbus::fdo::Result<()> {
        record(&self.calls, Call::Close(id));
        if self.bodies.get(&id).is_some_and(|body| body == STUCK) {
            return Err(zbus::fdo::Error::Failed("stuck".to_owned()));
        }
        Ok(())
    }

    #[zbus(signal)]
    async fn action_invoked(emitter: &SignalEmitter<'_>, id: u32, action_key: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn activation_token(emitter: &SignalEmitter<'_>, id: u32, activation_token: &str) -> zbus::Result<()>;
}

/// A quirky server recording into `calls`, serving on the bus at `address` under the server's name.
fn serve_quirky(address: &str, calls: &Arc<Mutex<Vec<Call>>>) -> TestResult<zbus::blocking::Connection> {
    Ok(zbus::blocking::connection::Builder::address(address)?
        .serve_at(
            PATH,
            Quirky {
                calls: Arc::clone(calls),
                last: 0,
                bodies: HashMap::new(),
            },
        )?
        .name(SERVER_NAME)?
        .build()?)
}

/// Clicks banner `id` on `server`, as its user would.
fn click_on(server: &zbus::blocking::Connection, id: u32) -> TestResult {
    let interface = server.object_server().interface::<_, Quirky>(PATH)?;
    emit(Quirky::action_invoked(interface.signal_emitter(), id, "default"))
}

/// Waits until `done` holds, or fails.
fn wait_until(what: &str, done: impl Fn() -> bool) -> TestResult {
    let deadline = Instant::now() + DEADLINE;
    while !done() {
        if Instant::now() > deadline {
            return Err(format!("timed out waiting for {what}").into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

/// The second scenario's parts: the quirky server, what it was asked, the banner task's sink, its queue as the task
/// takes from it, and its clicks, with the three panes' identities of one session.
struct Quirks {
    address: String,
    calls: Arc<Mutex<Vec<Call>>>,
    server: zbus::blocking::Connection,
    sink: Desktop,
    pending: async_channel::Receiver<ngterm_gpui::notifications::Banner>,
    clicks: std::sync::mpsc::Receiver<Click>,
    window: Uuid,
    session: Uuid,
    main: String,
    split: String,
    overlay: String,
}

impl Quirks {
    fn start(address: String) -> TestResult<Self> {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let server = serve_quirky(&address, &calls)?;
        let (sink, queue) = Desktop::new();
        let pending = queue.clone();
        let (clicked, clicks) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            zbus::block_on(run(queue, APP_ID.to_owned(), move |click| {
                let _ = clicked.send(click);
            }));
        });
        let (window, session) = (Uuid::new_v4(), Uuid::new_v4());
        Ok(Self {
            address,
            calls,
            server,
            sink,
            pending,
            clicks,
            window,
            session,
            main: notification_identity(window, session, PaneRole::Main),
            split: notification_identity(window, session, PaneRole::Split),
            overlay: notification_identity(window, session, PaneRole::Overlay),
        })
    }

    fn click(&self) -> TestResult<Click> {
        Ok(self.clicks.recv_timeout(DEADLINE)?)
    }

    /// The core's clear of the scenario's session: every pane's and the control slot's withdrawal.
    fn clear(&self) {
        deliver(
            &self.sink,
            &[NotificationEffect::Clear {
                window: self.window,
                session: self.session,
            }],
        );
    }

    /// Waits until the task took every banner queued: all but the last are handled.
    fn taken(&self) -> TestResult {
        wait_until("the task to take the queued banners", || self.pending.is_empty())
    }
}

#[test]
#[ignore = "run by a_quirky_server_and_a_bus_restart_on_a_private_bus, on its private bus"]
fn quirks_on_the_private_bus() -> TestResult {
    let address = std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap_or_default();
    // only ever on the private bus the parent test named, which this child starts itself: never the user's
    let socket = address.strip_prefix("unix:path=").map(PathBuf::from);
    let (Some(socket), Some(config), Some(program)) = (
        socket.filter(|_| std::env::var(PRIVATE_BUS).ok().as_deref() == Some(address.as_str())),
        std::env::var_os(BUS_CONFIG).map(PathBuf::from),
        std::env::var_os(BUS_DAEMON).map(PathBuf::from),
    ) else {
        eprintln!("skipped: not on the parent test's private bus");
        return Ok(());
    };
    let mut daemon = Daemon::start(&program, &config, &socket)?;
    let scenario = Quirks::start(address)?;
    an_early_token_and_stray_signals(&scenario)?;
    a_renumbered_repeat_and_a_refused_close(&scenario)?;
    let successor = nobody_serving_forgets_the_banners(&scenario)?;
    daemon.stop();
    drop(successor);
    the_task_reconnects_after_its_bus_restarted(&scenario, || Daemon::start(&program, &config, &socket))
}

/// A token the server sent before its `Notify` reply still counts for that banner; a signal the specification does not
/// name, and its own signals with another body, change nothing.
fn an_early_token_and_stray_signals(scenario: &Quirks) -> TestResult {
    let Quirks {
        calls,
        server,
        sink,
        main,
        split,
        ..
    } = scenario;
    sink.send(main, "t", EARLY);
    wait_for(calls, "the early banner", |calls| notifies(calls).len() == 1);
    click_on(server, 1)?;
    assert_eq!(
        scenario.click()?,
        Click {
            identity: main.clone(),
            token: Some("early-token".to_owned())
        },
        "the token sent while the post waited for its reply raises the window"
    );

    sink.send(split, "t", "two");
    let seen = wait_for(calls, "the split's banner", |calls| notifies(calls).len() == 2);
    assert_eq!(notifies(&seen)[1], (0, 2, "two".to_owned()));
    server.emit_signal(None::<&str>, PATH, INTERFACE, "Unexpected", &(2_u32,))?;
    server.emit_signal(None::<&str>, PATH, INTERFACE, "ActionInvoked", &("2",))?;
    server.emit_signal(None::<&str>, PATH, INTERFACE, "ActivationToken", &(2_u32,))?;
    server.emit_signal(None::<&str>, PATH, INTERFACE, "NotificationClosed", &(2_u32, "gone"))?;
    click_on(server, 2)?;
    assert_eq!(
        scenario.click()?,
        Click {
            identity: split.clone(),
            token: None
        },
        "a stray member and malformed bodies neither click, nor close, nor hand a token"
    );
    Ok(())
}

/// A repeat the server posts under a new id: the old id names nothing, the new one the pane. A banner the server
/// refuses to close is still forgotten.
fn a_renumbered_repeat_and_a_refused_close(scenario: &Quirks) -> TestResult {
    let Quirks {
        calls,
        server,
        sink,
        main,
        overlay,
        ..
    } = scenario;
    sink.send(main, "t", "three");
    sink.send(main, "t", RENUMBER);
    let seen = wait_for(calls, "the renumbered repeat", |calls| notifies(calls).len() == 4);
    assert_eq!(
        notifies(&seen)[2..],
        [(0, 3, "three".to_owned()), (3, 4, RENUMBER.to_owned())]
    );
    let interface = server.object_server().interface::<_, Quirky>(PATH)?;
    emit(Quirky::activation_token(interface.signal_emitter(), 4, "token-4"))?;
    click_on(server, 3)?;
    click_on(server, 4)?;
    assert_eq!(
        scenario.click()?,
        Click {
            identity: main.clone(),
            token: Some("token-4".to_owned())
        },
        "the replaced id reveals nothing; the new one reveals the pane, with its token"
    );

    sink.send(overlay, "t", STUCK);
    wait_for(calls, "the stuck banner", |calls| notifies(calls).len() == 5);
    scenario.clear();
    wait_for(calls, "the refused withdrawal", |calls| calls.contains(&Call::Close(5)));
    sink.send(overlay, "t", "again");
    let seen = wait_for(calls, "the overlay's next banner", |calls| notifies(calls).len() == 6);
    assert_eq!(
        notifies(&seen)[5],
        (0, 6, "again".to_owned()),
        "a banner whose close was refused is forgotten all the same"
    );
    Ok(())
}

/// The server's name left without an owner: a withdrawal finds nobody to ask, and the banners known are forgotten, so
/// the server that takes the name next gets none of their ids. That server, serving on.
fn nobody_serving_forgets_the_banners(scenario: &Quirks) -> TestResult<zbus::blocking::Connection> {
    let Quirks {
        calls,
        sink,
        overlay,
        split,
        ..
    } = scenario;
    sink.send(split, "t", "seven");
    wait_for(calls, "the split's banner", |calls| notifies(calls).len() == 7);
    scenario.server.release_name(SERVER_NAME)?;
    scenario.clear();
    scenario.taken()?;
    let asked = calls.lock().map(|calls| calls.clone()).unwrap_or_default();

    let successor_calls = Arc::new(Mutex::new(Vec::new()));
    let successor = serve_quirky(&scenario.address, &successor_calls)?;
    sink.send(overlay, "t", "eight");
    sink.send(split, "t", "nine");
    let seen = wait_for(&successor_calls, "the successor's banners", |calls| {
        notifies(calls).len() == 2
    });
    assert_eq!(
        seen,
        [notify_call(0, 1, "eight"), notify_call(0, 2, "nine"),],
        "no replaces_id and no withdrawal of the ids the old server handed out"
    );
    assert_eq!(
        calls.lock().map(|calls| calls.clone()).unwrap_or_default(),
        asked,
        "the old server, no longer named, is asked nothing"
    );
    assert_eq!(
        asked.last(),
        Some(&notify_call(0, 7, "seven")),
        "the clear closed nothing"
    );
    Ok(successor)
}

/// The quirky server's record of a `Notify`.
fn notify_call(replaces: u32, id: u32, body: &str) -> Call {
    Call::Notify {
        app: "ngterm".to_owned(),
        icon: ngterm_gpui::instance::DEFAULT_APP_ID.to_owned(),
        replaces,
        id,
        body: body.to_owned(),
        actions: vec!["default".to_owned(), String::new()],
        desktop_entry: Some(APP_ID.to_owned()),
    }
}

/// The bus went away under the task and came back: the banners while it was gone are dropped, the next one opens a
/// connection to the new bus, whose server gets none of the old ids and whose clicks count.
fn the_task_reconnects_after_its_bus_restarted(
    scenario: &Quirks,
    restart: impl FnOnce() -> TestResult<Daemon>,
) -> TestResult {
    let Quirks {
        address,
        sink,
        main,
        split,
        ..
    } = scenario;
    // the bus is down: these find none to post on
    sink.send(split, "t", "lost");
    sink.send(main, "t", "lost too");
    sink.withdraw("no banner of the task's");
    scenario.taken()?;
    let _daemon = restart()?;
    let calls = Arc::new(Mutex::new(Vec::new()));
    let server = serve_quirky(address, &calls)?;
    sink.send(split, "t", "after");
    let seen = wait_for(&calls, "the banner after the restart", |calls| {
        notifies(calls).len() == 1
    });
    assert_eq!(
        seen,
        [notify_call(0, 1, "after")],
        "the banners while the bus was down were dropped; the old server's id is not reused"
    );
    click_on(&server, 1)?;
    assert_eq!(
        scenario.click()?,
        Click {
            identity: split.clone(),
            token: None
        },
        "the new connection hears the new server's clicks"
    );
    Ok(())
}

/// One control request to the sandbox's app; its reply.
fn control(sandbox: &support::Sandbox, request: &serde_json::Value) -> TestResult<serde_json::Value> {
    use std::io::Write as _;
    let mut socket = std::os::unix::net::UnixStream::connect(sandbox.path("state/ngterm.sock"))?;
    socket.set_read_timeout(Some(DEADLINE))?;
    writeln!(socket, "{request}")?;
    let mut reply = String::new();
    BufReader::new(socket).read_line(&mut reply)?;
    Ok(serde_json::from_str(&reply)?)
}

/// The app end to end, headless on the sandbox's private bus with the fake server: a `notify` posts its banner, and the
/// server's click raises the app's window with the activation token the server sent first.
#[test]
fn the_apps_banner_click_raises_its_window_with_the_servers_token() -> TestResult {
    if dbus_daemon().is_none() {
        skip!("no dbus-daemon to run a private session bus on");
    }
    let mut sandbox = support::Sandbox::new()?;
    let calls = Arc::new(Mutex::new(Vec::new()));
    let server = serve(sandbox.bus_address(), &calls)?;
    let primary = sandbox.start_gpui_primary("primary.log")?;
    let reply = control(
        &sandbox,
        &serde_json::json!({ "cmd": "notify", "args": { "title": "Build", "body": "tests passed" } }),
    )?;
    assert_eq!(reply["ok"], serde_json::json!(true), "{reply}");
    wait_for(&calls, "the app's banner", |calls| notifies(calls).len() == 1);
    let raised = |sandbox: &support::Sandbox| {
        sandbox
            .log("primary.log")
            .matches("activation: no display server to raise a window on")
            .count()
    };
    let before = raised(&sandbox);
    let interface = server.object_server().interface::<_, Server>(PATH)?;
    emit(Server::activation_token(interface.signal_emitter(), 1, "token-1"))?;
    emit(Server::action_invoked(interface.signal_emitter(), 1, "default"))?;
    support::wait_for(|| (raised(&sandbox) > before).then_some(()))
        .map_err(|_| format!("the click raised nothing:\n{}", sandbox.log("primary.log")))?;
    sandbox.terminate(primary)?;
    assert_eq!(sandbox.wait_exit(primary, Duration::from_secs(10))?.0.code(), Some(0));
    Ok(())
}
