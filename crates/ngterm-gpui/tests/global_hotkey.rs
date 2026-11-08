//! keymap.conf's `global-hotkey` end to end, on the sandbox's own displays and bus (AGENTS.md > Never touch the
//! user's running ngterm): on a private Xvfb the chord's key is grabbed on the root window and a press from the X
//! server (XTEST, as any other client's keyboard) toggles the quick terminal, a keymap reload moving the grab, a chord
//! another client holds is refused and a keymap without the hotkey lets its key go; on a headless weston the chord is
//! bound through a fake global shortcuts portal on the private bus, as its preferred trigger, and the portal's
//! `Activated` toggles the panel; a reload closes the session and binds the new chord in a fresh one, and a refused
//! session or binding, or a portal without the interface, leaves it unbound.
//!
//! A test whose tool is missing prints `SKIPPED:` and passes, unless `NGTERM_REQUIRE_DISPLAY_TESTS` is set.
//!
//! Linux only: X11 key grabs on a private Xvfb and the global shortcuts portal on a private D-Bus bus.

#![cfg(target_os = "linux")]

#[macro_use]
#[expect(dead_code, reason = "the sandbox serves the process tests too")]
mod support;

use std::collections::HashMap;
use std::io::{BufRead as _, BufReader, Write as _};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde_json::{Value, json};
use support::{DEADLINE, Display, NO_SEATED_WESTON, Sandbox, TestResult, have, lavapipe_icd, seated_weston, wait_for};
use x11rb::connection::Connection as _;
use x11rb::protocol::xtest::ConnectionExt as _;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value as ZValue};

/// One control request with `args` to the sandbox's socket; its result, or the error it answered.
fn request(sandbox: &Sandbox, cmd: &str, args: &Value) -> TestResult<Value> {
    let mut socket = std::os::unix::net::UnixStream::connect(sandbox.path("state/ngterm.sock"))?;
    socket.set_read_timeout(Some(DEADLINE))?;
    writeln!(socket, "{}", json!({ "cmd": cmd, "args": args }))?;
    let mut reply = String::new();
    BufReader::new(socket).read_line(&mut reply)?;
    let reply: Value = serde_json::from_str(&reply)?;
    if reply["ok"] == json!(true) {
        Ok(reply.get("result").cloned().unwrap_or(Value::Null))
    } else {
        Err(format!("{cmd} {args}: {reply}").into())
    }
}

fn quick_visible(sandbox: &Sandbox) -> bool {
    request(sandbox, "tree", &json!({})).is_ok_and(|tree| tree["tree"]["quickVisible"] == json!(true))
}

fn wait_quick(sandbox: &Sandbox, visible: bool) -> TestResult {
    wait_for(|| (quick_visible(sandbox) == visible).then_some(())).map_err(|_| {
        format!(
            "the quick terminal never read visible = {visible}:\n{}",
            sandbox.log("primary.log")
        )
        .into()
    })
}

/// The keymap in the config directory a state dir override implies (`<state dir>/config`).
fn write_keymap(sandbox: &Sandbox, text: &str) -> TestResult {
    let dir = sandbox.path("state/config");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("keymap.conf"), text)?;
    Ok(())
}

/// Presses `keycodes` down in order and lets them up in reverse, as a keyboard of the X server.
fn press(conn: &x11rb::rust_connection::RustConnection, keycodes: &[u8]) -> TestResult {
    const KEY_PRESS: u8 = 2;
    const KEY_RELEASE: u8 = 3;
    for keycode in keycodes {
        conn.xtest_fake_input(KEY_PRESS, *keycode, 0, x11rb::NONE, 0, 0, 0)?;
    }
    for keycode in keycodes.iter().rev() {
        conn.xtest_fake_input(KEY_RELEASE, *keycode, 0, x11rb::NONE, 0, 0, 0)?;
    }
    conn.flush()?;
    Ok(())
}

/// Left Control, the key left of 1 (the backquote), T and left Alt: X keycodes on every evdev keymap.
const CONTROL: u8 = 37;
const BACKQUOTE: u8 = 49;
const T: u8 = 28;
const ALT: u8 = 64;

#[test]
fn the_grabbed_chord_toggles_the_quick_terminal_from_the_x_server() -> TestResult {
    if !have("Xvfb") {
        skip!("no Xvfb");
    }
    let Some(lavapipe) = lavapipe_icd() else {
        skip!("no lavapipe ICD (scripts/fetch-lavapipe.sh)");
    };
    let mut sandbox = Sandbox::new()?;
    write_keymap(&sandbox, "global-hotkey ctrl+`\n")?;
    sandbox.start_xvfb()?;
    sandbox.use_software_rendering(&lavapipe);
    let primary = sandbox.start_gpui_primary("primary.log")?;
    sandbox.wait_log("primary.log", "global hotkey: grabbed")?;
    let Display::X11(number) = sandbox.display() else {
        return Err("not an X11 sandbox".into());
    };
    let (conn, _) = x11rb::connect(Some(&format!(":{number}")))?;
    assert!(!quick_visible(&sandbox));

    press(&conn, &[CONTROL, BACKQUOTE])?;
    wait_quick(&sandbox, true)?;
    press(&conn, &[CONTROL, BACKQUOTE])?;
    wait_quick(&sandbox, false)?;

    // a reload moves the grab: the old chord does nothing, the new one toggles
    write_keymap(&sandbox, "global-hotkey ctrl+alt+t\n")?;
    request(&sandbox, "keymap.reload", &json!({}))?;
    wait_for(|| (sandbox.log("primary.log").matches("global hotkey: grabbed").count() >= 2).then_some(()))
        .map_err(|_| "the reload grabbed nothing new")?;
    press(&conn, &[CONTROL, BACKQUOTE])?;
    std::thread::sleep(Duration::from_millis(300));
    assert!(!quick_visible(&sandbox), "the old chord is let go");
    press(&conn, &[CONTROL, ALT, T])?;
    wait_quick(&sandbox, true)?;

    sandbox.terminate(primary)?;
    assert_eq!(sandbox.wait_exit(primary, Duration::from_secs(10))?.0.code(), Some(0));
    Ok(())
}

/// What the fake portal was asked.
#[derive(Default)]
struct Asked {
    /// The session last created.
    session: Option<OwnedObjectPath>,
    /// Each bound shortcut's id and preferred trigger.
    bound: Vec<(String, Option<String>)>,
    /// The sessions closed, in order.
    closed: Vec<OwnedObjectPath>,
    /// The method the portal refuses (`CreateSession`, `BindShortcuts`), if any.
    refuse: Option<&'static str>,
}

/// A session the portal created, which records its closing.
struct PortalSession {
    path: OwnedObjectPath,
    asked: Arc<Mutex<Asked>>,
}

#[zbus::interface(name = "org.freedesktop.portal.Session")]
impl PortalSession {
    fn close(&self) {
        let mut asked = self.asked.lock().unwrap_or_else(PoisonError::into_inner);
        asked.closed.push(self.path.clone());
    }
}

/// A global shortcuts portal answering through `org.freedesktop.portal.Request` as the real one does.
struct Portal {
    asked: Arc<Mutex<Asked>>,
    version: u32,
    connection: Arc<std::sync::OnceLock<zbus::Connection>>,
}

/// The `Request` or `Session` object path the portal derives from the caller and its token.
fn handle(kind: &str, sender: &str, token: &str) -> String {
    let caller = sender.trim_start_matches(':').replace('.', "_");
    format!("/org/freedesktop/portal/desktop/{kind}/{caller}/{token}")
}

fn token(options: &HashMap<String, OwnedValue>, key: &str) -> String {
    options
        .get(key)
        .and_then(|value| String::try_from(value.clone()).ok())
        .unwrap_or_default()
}

/// Answers request `path` of `sender` with `results`, a little later, as a portal answers once its dialog is done.
fn respond(connection: zbus::Connection, sender: String, path: String, results: HashMap<String, OwnedValue>) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        let sent = zbus::block_on(connection.emit_signal(
            Some(sender.as_str()),
            path.as_str(),
            "org.freedesktop.portal.Request",
            "Response",
            &(0_u32, results),
        ));
        if let Err(error) = sent {
            eprintln!("the fake portal's response failed: {error}");
        }
    });
}

fn failed(error: impl std::fmt::Display) -> zbus::fdo::Error {
    zbus::fdo::Error::Failed(error.to_string())
}

impl Portal {
    /// The portal's own connection, which answers the requests.
    fn connection(&self) -> zbus::fdo::Result<zbus::Connection> {
        self.connection
            .get()
            .cloned()
            .ok_or_else(|| failed("the portal is not up yet"))
    }

    /// Fails `method` when the test asked the portal to refuse it.
    fn refusing(&self, method: &str) -> zbus::fdo::Result<()> {
        let asked = self.asked.lock().unwrap_or_else(PoisonError::into_inner);
        if asked.refuse == Some(method) {
            return Err(failed(format!("{method} refused")));
        }
        Ok(())
    }
}

#[zbus::interface(name = "org.freedesktop.portal.GlobalShortcuts")]
impl Portal {
    #[zbus(property)]
    fn version(&self) -> u32 {
        self.version
    }

    #[expect(clippy::needless_pass_by_value, reason = "zbus hands a method its arguments owned")]
    fn create_session(
        &self,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        self.refusing("CreateSession")?;
        let sender = header.sender().map(ToString::to_string).unwrap_or_default();
        let request = handle("request", &sender, &token(&options, "handle_token"));
        let session = handle("session", &sender, &token(&options, "session_handle_token"));
        let session_path = OwnedObjectPath::try_from(session.clone()).map_err(failed)?;
        self.asked.lock().unwrap_or_else(PoisonError::into_inner).session = Some(session_path.clone());
        let results = HashMap::from([(
            "session_handle".to_owned(),
            OwnedValue::try_from(ZValue::from(session)).map_err(failed)?,
        )]);
        // the session object first, so it can be closed once its caller knows it
        let connection = self.connection()?;
        let answered = request.clone();
        let served = PortalSession {
            path: session_path.clone(),
            asked: Arc::clone(&self.asked),
        };
        std::thread::spawn(move || {
            if let Err(error) = zbus::block_on(connection.object_server().at(session_path, served)) {
                eprintln!("the fake portal's session was not served: {error}");
            }
            respond(connection, sender, answered, results);
        });
        OwnedObjectPath::try_from(request).map_err(failed)
    }

    #[expect(clippy::needless_pass_by_value, reason = "zbus hands a method its arguments owned")]
    fn bind_shortcuts(
        &self,
        session: OwnedObjectPath,
        shortcuts: Vec<(String, HashMap<String, OwnedValue>)>,
        parent_window: String,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        self.refusing("BindShortcuts")?;
        let sender = header.sender().map(ToString::to_string).unwrap_or_default();
        let request = handle("request", &sender, &token(&options, "handle_token"));
        let mut asked = self.asked.lock().unwrap_or_else(PoisonError::into_inner);
        if asked.session.as_ref() != Some(&session) {
            return Err(failed(format!("no such session: {session} (parent {parent_window:?})")));
        }
        for (id, info) in shortcuts {
            let trigger = info
                .get("preferred_trigger")
                .and_then(|value| String::try_from(value.clone()).ok());
            asked.bound.push((id, trigger));
        }
        respond(self.connection()?, sender, request.clone(), HashMap::new());
        OwnedObjectPath::try_from(request).map_err(failed)
    }
}

#[test]
fn the_portal_binds_the_chord_and_its_activation_toggles_the_quick_terminal() -> TestResult {
    if seated_weston().is_none() {
        skip!("{NO_SEATED_WESTON}");
    }
    let Some(lavapipe) = lavapipe_icd() else {
        skip!("no lavapipe ICD (scripts/fetch-lavapipe.sh)");
    };
    let mut sandbox = Sandbox::new()?;
    let asked = Arc::new(Mutex::new(Asked::default()));
    let connection = Arc::new(std::sync::OnceLock::new());
    let portal = zbus::blocking::connection::Builder::address(sandbox.bus_address())?
        .name("org.freedesktop.portal.Desktop")?
        .serve_at(
            "/org/freedesktop/portal/desktop",
            Portal {
                asked: Arc::clone(&asked),
                version: 1,
                connection: Arc::clone(&connection),
            },
        )?
        .build()?;
    let _ = connection.set(portal.inner().clone());
    write_keymap(&sandbox, "global-hotkey ctrl+`\n")?;
    sandbox.start_weston()?;
    sandbox.use_software_rendering(&lavapipe);
    let primary = sandbox.start_gpui_primary("primary.log")?;
    sandbox.wait_log("primary.log", "global hotkey: bound through the portal")?;
    let session = {
        let asked = asked.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(
            asked.bound,
            [("quick-terminal".to_owned(), Some("CTRL+grave".to_owned()))],
            "the chord offered as the preferred trigger"
        );
        asked.session.clone().ok_or("no session was created")?
    };
    let activate = |id: &str| -> TestResult {
        portal.emit_signal(
            None::<()>,
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.GlobalShortcuts",
            "Activated",
            &(session.clone(), id, 0_u64, HashMap::<String, OwnedValue>::new()),
        )?;
        Ok(())
    };
    activate("something-else")?;
    std::thread::sleep(Duration::from_millis(300));
    assert!(!quick_visible(&sandbox), "another shortcut's activation does nothing");
    activate("quick-terminal")?;
    wait_quick(&sandbox, true)?;
    activate("quick-terminal")?;
    wait_quick(&sandbox, false)?;

    sandbox.terminate(primary)?;
    assert_eq!(sandbox.wait_exit(primary, Duration::from_secs(10))?.0.code(), Some(0));
    Ok(())
}

/// The X modifier masks a grab of a Control chord takes, under every Caps Lock and Num Lock state.
fn control_masks(extra: x11rb::protocol::xproto::ModMask) -> [x11rb::protocol::xproto::ModMask; 4] {
    use x11rb::protocol::xproto::ModMask;
    let base = ModMask::CONTROL | extra;
    [
        base,
        base | ModMask::LOCK,
        base | ModMask::M2,
        base | ModMask::LOCK | ModMask::M2,
    ]
}

/// Whether this client could grab `keycode` under `masks` on `root`; what it grabbed is let go again.
fn could_grab(
    conn: &x11rb::rust_connection::RustConnection,
    root: u32,
    keycode: u8,
    masks: &[x11rb::protocol::xproto::ModMask],
) -> TestResult<bool> {
    use x11rb::protocol::xproto::{ConnectionExt as _, GrabMode};
    let mut free = true;
    for mask in masks {
        let grabbed = conn
            .grab_key(false, root, *mask, keycode, GrabMode::ASYNC, GrabMode::ASYNC)?
            .check();
        free &= grabbed.is_ok();
        conn.ungrab_key(keycode, root, *mask)?;
    }
    conn.flush()?;
    Ok(free)
}

/// Reloads the keymap as `text`.
fn reload(sandbox: &Sandbox, text: &str) -> TestResult {
    write_keymap(sandbox, text)?;
    request(sandbox, "keymap.reload", &json!({}))?;
    Ok(())
}

#[test]
fn a_chord_another_client_holds_is_refused_and_a_cleared_chord_lets_its_key_go() -> TestResult {
    use x11rb::protocol::xproto::{ConnectionExt as _, GrabMode, ModMask};
    if !have("Xvfb") {
        skip!("no Xvfb");
    }
    let Some(lavapipe) = lavapipe_icd() else {
        skip!("no lavapipe ICD (scripts/fetch-lavapipe.sh)");
    };
    let mut sandbox = Sandbox::new()?;
    write_keymap(&sandbox, "global-hotkey ctrl+`\n")?;
    let number = sandbox.start_xvfb()?;
    sandbox.use_software_rendering(&lavapipe);
    let (conn, screen) = x11rb::connect(Some(&format!(":{number}")))?;
    let root = conn.setup().roots[screen].root;
    // another application holds the chord first, whatever the locks
    let held = control_masks(ModMask::from(0_u16));
    for mask in held {
        conn.grab_key(false, root, mask, BACKQUOTE, GrabMode::ASYNC, GrabMode::ASYNC)?
            .check()?;
    }
    let primary = sandbox.start_gpui_primary("primary.log")?;
    sandbox.wait_log("primary.log", "the global hotkey is taken by another application")?;
    press(&conn, &[CONTROL, BACKQUOTE])?;
    std::thread::sleep(Duration::from_millis(300));
    assert!(!quick_visible(&sandbox), "the press is the other application's");

    for mask in held {
        conn.ungrab_key(BACKQUOTE, root, mask)?;
    }
    conn.flush()?;
    reload(&sandbox, "global-hotkey ctrl+alt+t\n")?;
    sandbox.wait_log("primary.log", "global hotkey: grabbed")?;
    let alt_t = control_masks(ModMask::M1);
    assert!(!could_grab(&conn, root, T, &alt_t)?, "the app holds its chord now");
    reload(&sandbox, "")?;
    wait_for(|| could_grab(&conn, root, T, &alt_t).ok().filter(|free| *free))
        .map_err(|_| "a keymap without a global hotkey kept the key grabbed")?;

    sandbox.terminate(primary)?;
    assert_eq!(sandbox.wait_exit(primary, Duration::from_secs(10))?.0.code(), Some(0));
    Ok(())
}

/// The fake portal on the sandbox's bus, recording into `asked`.
fn serve_portal(sandbox: &Sandbox, asked: &Arc<Mutex<Asked>>) -> TestResult<zbus::blocking::Connection> {
    let connection = Arc::new(std::sync::OnceLock::new());
    let portal = zbus::blocking::connection::Builder::address(sandbox.bus_address())?
        .name("org.freedesktop.portal.Desktop")?
        .serve_at(
            "/org/freedesktop/portal/desktop",
            Portal {
                asked: Arc::clone(asked),
                version: 1,
                connection: Arc::clone(&connection),
            },
        )?
        .build()?;
    let _ = connection.set(portal.inner().clone());
    Ok(portal)
}

/// A primary on a headless weston with `keymap`, its portal (if any) already on the bus.
fn start_on_weston(sandbox: &mut Sandbox, keymap: &str, lavapipe: &std::path::Path) -> TestResult<usize> {
    write_keymap(sandbox, keymap)?;
    sandbox.start_weston()?;
    sandbox.use_software_rendering(lavapipe);
    sandbox.start_gpui_primary("primary.log")
}

/// Waits until what the portal was asked satisfies `done`.
fn wait_asked(asked: &Mutex<Asked>, what: &str, done: impl Fn(&Asked) -> bool) -> TestResult {
    wait_for(|| done(&asked.lock().unwrap_or_else(PoisonError::into_inner)).then_some(()))
        .map_err(|_| format!("the portal was never asked for {what}").into())
}

#[test]
fn a_reload_closes_the_portal_session_and_binds_the_new_chord_in_a_fresh_one() -> TestResult {
    if seated_weston().is_none() {
        skip!("{NO_SEATED_WESTON}");
    }
    let Some(lavapipe) = lavapipe_icd() else {
        skip!("no lavapipe ICD (scripts/fetch-lavapipe.sh)");
    };
    let mut sandbox = Sandbox::new()?;
    let asked = Arc::new(Mutex::new(Asked::default()));
    let _portal = serve_portal(&sandbox, &asked)?;
    let primary = start_on_weston(&mut sandbox, "global-hotkey ctrl+`\n", &lavapipe)?;
    wait_asked(&asked, "the first binding", |asked| asked.bound.len() == 1)?;
    let session = |asked: &Mutex<Asked>| asked.lock().unwrap_or_else(PoisonError::into_inner).session.clone();
    let first = session(&asked).ok_or("no session was created")?;

    reload(&sandbox, "global-hotkey ctrl+alt+t\n")?;
    wait_asked(&asked, "the second binding", |asked| asked.bound.len() == 2)?;
    let second = session(&asked).ok_or("no session was created")?;
    reload(&sandbox, "")?;
    wait_asked(&asked, "the last session's close", |asked| asked.closed.len() == 2)?;
    {
        let asked = asked.lock().unwrap_or_else(PoisonError::into_inner);
        let id = "quick-terminal".to_owned();
        assert_eq!(
            asked.bound,
            [
                (id.clone(), Some("CTRL+grave".to_owned())),
                (id, Some("CTRL+ALT+t".to_owned())),
            ],
            "each chord bound in a session of its own"
        );
        assert_eq!(
            asked.closed,
            [first, second],
            "each new chord closes the old session; a keymap without one closes the last and binds nothing"
        );
    }
    sandbox.terminate(primary)?;
    assert_eq!(sandbox.wait_exit(primary, Duration::from_secs(10))?.0.code(), Some(0));
    Ok(())
}

#[test]
fn a_portal_that_refuses_the_session_or_the_binding_leaves_the_chord_unbound() -> TestResult {
    if seated_weston().is_none() {
        skip!("{NO_SEATED_WESTON}");
    }
    let Some(lavapipe) = lavapipe_icd() else {
        skip!("no lavapipe ICD (scripts/fetch-lavapipe.sh)");
    };
    let mut sandbox = Sandbox::new()?;
    let asked = Arc::new(Mutex::new(Asked {
        refuse: Some("CreateSession"),
        ..Asked::default()
    }));
    let portal = serve_portal(&sandbox, &asked)?;
    let primary = start_on_weston(&mut sandbox, "global-hotkey ctrl+`\n", &lavapipe)?;
    sandbox.wait_log("primary.log", "the global shortcuts portal opened no session")?;

    asked.lock().unwrap_or_else(PoisonError::into_inner).refuse = Some("BindShortcuts");
    reload(&sandbox, "global-hotkey ctrl+alt+t\n")?;
    sandbox.wait_log(
        "primary.log",
        "the global shortcuts portal did not bind the global hotkey",
    )?;
    wait_asked(&asked, "the refused session's close", |asked| asked.closed.len() == 1)?;
    let session = {
        let asked = asked.lock().unwrap_or_else(PoisonError::into_inner);
        assert!(asked.bound.is_empty(), "nothing was bound");
        let session = asked.session.clone().ok_or("no session was created")?;
        assert_eq!(
            asked.closed,
            std::slice::from_ref(&session),
            "the session of a refused binding is closed"
        );
        session
    };
    portal.emit_signal(
        None::<()>,
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.GlobalShortcuts",
        "Activated",
        &(session, "quick-terminal", 0_u64, HashMap::<String, OwnedValue>::new()),
    )?;
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        !quick_visible(&sandbox),
        "an unbound shortcut's activation does nothing"
    );

    sandbox.terminate(primary)?;
    assert_eq!(sandbox.wait_exit(primary, Duration::from_secs(10))?.0.code(), Some(0));
    Ok(())
}

/// The desktop portal without the global shortcuts interface (a wlroots desktop's), answering as xdg-desktop-portal
/// does: the properties of an interface it lacks are `InvalidArgs` naming that interface, any other call
/// `UnknownMethod`.
fn portal_without_global_shortcuts(sandbox: &Sandbox) -> TestResult<zbus::blocking::Connection> {
    let portal = zbus::blocking::connection::Builder::address(sandbox.bus_address())?
        .name("org.freedesktop.portal.Desktop")?
        .build()?;
    let answering = portal.clone();
    std::thread::spawn(move || {
        for message in zbus::blocking::MessageIterator::from(&answering).flatten() {
            if message.message_type() != zbus::message::Type::MethodCall {
                continue;
            }
            let header = message.header();
            let properties = header
                .interface()
                .is_some_and(|interface| interface == "org.freedesktop.DBus.Properties");
            // `Get` names the interface and the property, `GetAll` the interface alone
            let body = message.body();
            let asked = body
                .deserialize::<(String, String)>()
                .map(|(interface, _)| interface)
                .or_else(|_| body.deserialize::<(String,)>().map(|(interface,)| interface))
                .ok();
            let (name, text) = match asked {
                Some(interface) if properties => (
                    "org.freedesktop.DBus.Error.InvalidArgs",
                    format!("No such interface \u{201c}{interface}\u{201d}"),
                ),
                _ => ("org.freedesktop.DBus.Error.UnknownMethod", "no such method".to_owned()),
            };
            if answering.reply_error(&header, name, &text).is_err() {
                return;
            }
        }
    });
    Ok(portal)
}

#[test]
fn a_desktop_portal_without_global_shortcuts_leaves_the_chord_unbound() -> TestResult {
    if seated_weston().is_none() {
        skip!("{NO_SEATED_WESTON}");
    }
    let Some(lavapipe) = lavapipe_icd() else {
        skip!("no lavapipe ICD (scripts/fetch-lavapipe.sh)");
    };
    let mut sandbox = Sandbox::new()?;
    let _portal = portal_without_global_shortcuts(&sandbox)?;
    let primary = start_on_weston(&mut sandbox, "global-hotkey ctrl+`\n", &lavapipe)?;
    sandbox.wait_log(
        "primary.log",
        "no global shortcuts portal: global-hotkey is ignored on this desktop",
    )?;
    // the binding has ended: a new chord is handed to nobody
    reload(&sandbox, "global-hotkey ctrl+alt+t\n")?;
    sandbox.terminate(primary)?;
    assert_eq!(sandbox.wait_exit(primary, Duration::from_secs(10))?.0.code(), Some(0));
    let log = sandbox.log("primary.log");
    assert_eq!(
        log.matches("no global shortcuts portal").count(),
        1,
        "said once:\n{log}"
    );
    assert!(!log.contains("global hotkey: bound"), "nothing was bound:\n{log}");
    Ok(())
}
