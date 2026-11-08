//! The keyboard-layout lock against real layouts on private displays, never the user's: a private Xvfb's XKB groups
//! (listed, locked and read back), a niri nested in a private headless weston (found by the Wayland probe through its
//! socket's peer, listed, switched and read back), the X backend failing without its server, a fake KDE keyboard
//! service on the sandbox's private session bus (alone, refusing, gone, reached on the session bus the environment
//! names, and found by the Wayland probe when Plasma's `kwin_wayland_wrapper` serves the display, which without the
//! service gives no backend), and the app end to end on the Xvfb: with "Lock in terminals" on Russian, the group
//! locks while the terminal holds the keyboard, a manual switch there is switched back, and the earlier group returns
//! when the keyboard goes elsewhere and on quit; and a keymap that only changes the layout list (`setxkbmap`, the
//! selected layout's name unchanged) is seen while the terminal keeps the keyboard: Russian enabled again locks it.
//!
//! A test whose tool is missing prints `SKIPPED:` and passes, unless `NGTERM_REQUIRE_DISPLAY_TESTS` is set. The
//! nested-niri check is `#[ignore]`d (CI has no niri): run it with `--ignored`.
//!
//! Linux only: XKB groups on a private Xvfb, Wayland compositors and KDE's D-Bus service.

#![cfg(target_os = "linux")]

#[macro_use]
#[expect(dead_code, reason = "the sandbox serves the process tests too")]
mod support;

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use ngterm_gpui::input_source::linux::{kde, wayland, x11};
use ngterm_gpui::input_source::{Backend, Failure, KeyboardLayout};
use support::{Display, Sandbox, TestResult, have, lavapipe_icd, seated_weston, wait_for, wait_for_socket};
use x11rb::connection::Connection as _;
use x11rb::protocol::xkb::ConnectionExt as _;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, InputFocus, MapState, Window};

/// A backend's result as the test's.
fn ok<T>(result: Result<T, Failure>) -> TestResult<T> {
    result.map_err(|error| error.to_string().into())
}

const US: &str = "English (US)";
const RU: &str = "Russian";

/// The sandbox's X display name.
fn display_name(sandbox: &Sandbox) -> TestResult<String> {
    match sandbox.display() {
        Display::X11(number) => Ok(format!(":{number}")),
        _ => Err("not an X11 sandbox".into()),
    }
}

/// A sandbox on a private Xvfb whose keyboard has XKB's `us` and `ru` layouts; with the test's connection, which must
/// stay open: the server resets its keymap when its last client disconnects.
fn two_layout_xvfb() -> TestResult<(Sandbox, X11)> {
    let mut sandbox = Sandbox::new()?;
    sandbox.start_xvfb()?;
    let display = display_name(&sandbox)?;
    let x11 = X11::connect(&display)?;
    set_layouts(&sandbox, "us,ru")?;
    Ok((sandbox, x11))
}

/// Loads the XKB `layouts` (comma-separated) into the sandbox's X server, as a user's `setxkbmap` would.
fn set_layouts(sandbox: &Sandbox, layouts: &str) -> TestResult {
    let display = display_name(sandbox)?;
    let status = sandbox
        .command("setxkbmap")
        .args(["-display", &display, "-layout", layouts])
        .status()?;
    assert!(status.success(), "setxkbmap -layout {layouts}: {status:?}");
    Ok(())
}

/// The X server as the test's window manager and second XKB client.
struct X11 {
    conn: x11rb::rust_connection::RustConnection,
    root: Window,
}

impl X11 {
    fn connect(display: &str) -> TestResult<Self> {
        let (conn, screen) = x11rb::connect(Some(display))?;
        conn.xkb_use_extension(1, 0)?.reply()?;
        let root = conn.setup().roots[screen].root;
        Ok(Self { conn, root })
    }

    /// The core keyboard's effective group, as any client reads it.
    fn group(&self) -> TestResult<u8> {
        Ok(u8::from(self.conn.xkb_get_state(256)?.reply()?.group))
    }

    /// Waits for the group to become `group`.
    fn wait_group(&self, group: u8, sandbox: &Sandbox) -> TestResult {
        wait_for(|| (self.group().ok()? == group).then_some(())).map_err(|_| {
            format!(
                "the group stayed {:?}, not {group}:\n{}",
                self.group(),
                sandbox.log("primary.log")
            )
            .into()
        })
    }

    /// Whether `window` is mapped and its `WM_CLASS` names `app_id`.
    fn is_app_window(&self, window: Window, app_id: &str) -> Option<bool> {
        let attributes = self.conn.get_window_attributes(window).ok()?.reply().ok()?;
        let class = self
            .conn
            .get_property(false, window, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 1024)
            .ok()?
            .reply()
            .ok()?;
        Some(attributes.map_state == MapState::VIEWABLE && String::from_utf8_lossy(&class.value).contains(app_id))
    }

    /// Waits for the app's window and gives it the keyboard focus, as a window manager would.
    fn focus_app_window(&self, app_id: &str) -> TestResult {
        let window = wait_for(|| {
            let tree = self.conn.query_tree(self.root).ok()?.reply().ok()?;
            tree.children
                .into_iter()
                .find(|window| self.is_app_window(*window, app_id) == Some(true))
        })
        .map_err(|_| "no ngterm window was mapped")?;
        self.conn
            .set_input_focus(InputFocus::PARENT, window, x11rb::CURRENT_TIME)?
            .check()?;
        Ok(())
    }

    /// Gives the keyboard to the root window: another application's, as far as the app can tell.
    fn focus_elsewhere(&self) -> TestResult {
        self.conn
            .set_input_focus(InputFocus::POINTER_ROOT, self.root, x11rb::CURRENT_TIME)?
            .check()?;
        Ok(())
    }
}

#[test]
fn xkb_groups_are_listed_locked_and_read_back() -> TestResult {
    if !have("Xvfb") || !have("setxkbmap") {
        skip!("no Xvfb or setxkbmap");
    }
    let (sandbox, x11) = two_layout_xvfb()?;
    let mut xkb = ok(x11::Xkb::connect(Some(&display_name(&sandbox)?)))?;
    assert_eq!(
        ok(xkb.layouts())?,
        [KeyboardLayout::named(US), KeyboardLayout::named(RU)]
    );
    assert_eq!(ok(xkb.current())?, US);
    ok(xkb.select(RU))?;
    assert_eq!(ok(xkb.current())?, RU);
    assert_eq!(x11.group()?, 1, "the lock is the server's");
    ok(xkb.select(US))?;
    assert_eq!(x11.group()?, 0);
    assert!(xkb.select("German").is_err());
    Ok(())
}

#[test]
fn xkb_without_a_display_or_after_it_is_gone_fails() -> TestResult {
    if !have("Xvfb") {
        skip!("no Xvfb");
    }
    assert!(x11::Xkb::connect(Some("no display name")).is_err());
    let sandbox = Sandbox::new()?;
    // an X server of this test's own, to stop under the backend
    let mut xvfb = sandbox
        .command("Xvfb")
        .args(["-displayfd", "1", "-nolisten", "tcp"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    let mut number = String::new();
    std::io::BufRead::read_line(
        &mut std::io::BufReader::new(xvfb.stdout.take().ok_or("no Xvfb output")?),
        &mut number,
    )?;
    let xvfb = Killed(xvfb);
    let mut xkb = ok(x11::Xkb::connect(Some(&format!(":{}", number.trim()))))?;
    assert!(!ok(xkb.layouts())?.is_empty());
    drop(xvfb);
    assert!(xkb.layouts().is_err(), "the server is gone");
    assert!(xkb.current().is_err());
    assert!(xkb.select(US).is_err());
    assert!(xkb.lock_group(0).is_err());
    Ok(())
}

#[test]
#[ignore = "needs niri (none in CI): niri nested in a private headless weston; run with --ignored"]
fn a_nested_niri_is_found_by_its_socket_and_switched() -> TestResult {
    if !have("niri") {
        skip!("no niri");
    }
    if seated_weston().is_none() {
        skip!("{}", support::NO_SEATED_WESTON);
    }
    let mut sandbox = Sandbox::new()?;
    sandbox.start_nested_niri_with_layouts("us,ru")?;
    let var = |name: &str| sandbox.env_var(name).map(PathBuf::from);
    let mut niri = wayland(&var).map_err(|reason| format!("no backend for the nested niri: {reason}"))?;
    assert_eq!(
        ok(niri.layouts())?,
        [KeyboardLayout::named(US), KeyboardLayout::named(RU)]
    );
    assert_eq!(ok(niri.current())?, US);
    ok(niri.select(RU))?;
    assert_eq!(ok(niri.current())?, RU);
    ok(niri.select(US))?;
    assert_eq!(ok(niri.current())?, US);
    Ok(())
}

/// A fake KDE keyboard service: its layouts and the current index.
struct Layouts {
    list: Vec<(String, String, String)>,
    current: Arc<Mutex<u32>>,
}

#[zbus::interface(name = "org.kde.KeyboardLayouts")]
impl Layouts {
    #[zbus(name = "getLayoutsList")]
    fn get_layouts_list(&self) -> Vec<(String, String, String)> {
        self.list.clone()
    }

    #[zbus(name = "getLayout")]
    fn get_layout(&self) -> u32 {
        *self.current.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Selects the first two layouts, refuses the third and fails past it.
    #[zbus(name = "setLayout")]
    fn set_layout(&self, index: u32) -> zbus::fdo::Result<bool> {
        if index > 2 {
            return Err(zbus::fdo::Error::Failed(format!("no layout {index}")));
        }
        if index > 1 {
            return Ok(false);
        }
        *self.current.lock().unwrap_or_else(PoisonError::into_inner) = index;
        Ok(true)
    }
}

/// The fake KDE keyboard service on the sandbox's session bus, US and Russian with US current; the index it holds.
fn serve_kde(sandbox: &Sandbox) -> TestResult<(zbus::blocking::Connection, Arc<Mutex<u32>>)> {
    // (short name, display name, long name): the long name is the XKB layout name
    serve_kde_layouts(
        sandbox,
        vec![
            ("us".to_owned(), "EN".to_owned(), US.to_owned()),
            ("ru".to_owned(), String::new(), RU.to_owned()),
        ],
    )
}

/// The fake KDE keyboard service on the sandbox's session bus with `list`, the first current; the index it holds.
fn serve_kde_layouts(
    sandbox: &Sandbox,
    list: Vec<(String, String, String)>,
) -> TestResult<(zbus::blocking::Connection, Arc<Mutex<u32>>)> {
    let current = Arc::new(Mutex::new(0));
    let service = zbus::blocking::connection::Builder::address(sandbox.bus_address())?
        .name("org.kde.keyboard")?
        .serve_at(
            "/Layouts",
            Layouts {
                list,
                current: Arc::clone(&current),
            },
        )?
        .build()?;
    Ok((service, current))
}

#[test]
fn kdes_keyboard_service_lists_reads_and_switches() -> TestResult {
    if !have("dbus-daemon") {
        skip!("no dbus-daemon");
    }
    let sandbox = Sandbox::new()?;
    let (_service, current) = serve_kde(&sandbox)?;
    let client = zbus::blocking::connection::Builder::address(sandbox.bus_address())?.build()?;
    let mut kde = ok(kde::Kde::over(client))?;
    assert_eq!(
        ok(kde.layouts())?,
        [KeyboardLayout::named(US), KeyboardLayout::named(RU)]
    );
    ok(kde.select(RU))?;
    assert_eq!(*current.lock().unwrap_or_else(PoisonError::into_inner), 1);
    assert_eq!(ok(kde.current())?, RU);
    Ok(())
}

/// A child process killed when dropped.
struct Killed(Child);

impl Drop for Killed {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Plasma's `kwin_wayland_wrapper` serving a Wayland display socket in the sandbox, and that socket; `None` without the
/// system's python to stand in for it.
fn kwin_wrapper(sandbox: &Sandbox) -> TestResult<Option<(Killed, PathBuf)>> {
    // the system's python, from the sandbox's PATH: a version manager's shim on the user's PATH execs by its own name
    let Some(python) = sandbox.env_var("PATH").and_then(|path| {
        std::env::split_paths(path)
            .map(|dir| dir.join("python3"))
            .find(|path| path.is_file())
    }) else {
        return Ok(None);
    };
    // Plasma's `kwin_wayland_wrapper` listens on the display's socket and hands it to `kwin_wayland`, so the socket's
    // peer is the wrapper: a listener exec'd under that name (its command name cut to `kwin_wayland_wr`)
    let wrapper = sandbox.path("kwin_wayland_wrapper");
    std::os::unix::fs::symlink(python.canonicalize()?, &wrapper)?;
    let display = sandbox.path("run/wayland-kde");
    let kwin = Killed(
        Command::new(&wrapper)
            .args([
                "-I",
                "-c",
                "import socket, sys, time\n\
                 s = socket.socket(socket.AF_UNIX)\n\
                 s.bind(sys.argv[1])\n\
                 s.listen(16)\n\
                 time.sleep(600)",
            ])
            .arg(&display)
            .env_clear()
            .spawn()?,
    );
    wait_for_socket(&display)?;
    Ok(Some((kwin, display)))
}

#[test]
fn kde_is_found_when_plasmas_kwin_wrapper_serves_the_display() -> TestResult {
    if !have("dbus-daemon") {
        skip!("no dbus-daemon");
    }
    let sandbox = Sandbox::new()?;
    let Some((_kwin, display)) = kwin_wrapper(&sandbox)? else {
        skip!("no python3");
    };
    let (_service, current) = serve_kde(&sandbox)?;
    let var = |name: &str| match name {
        "WAYLAND_DISPLAY" => Some(display.clone()),
        "XDG_CURRENT_DESKTOP" => Some(PathBuf::from("KDE")),
        _ => sandbox.env_var(name).map(PathBuf::from),
    };
    let mut kde = wayland(&var).map_err(|reason| format!("no backend under the KWin wrapper: {reason}"))?;
    assert_eq!(
        ok(kde.layouts())?,
        [KeyboardLayout::named(US), KeyboardLayout::named(RU)]
    );
    ok(kde.select(RU))?;
    assert_eq!(*current.lock().unwrap_or_else(PoisonError::into_inner), 1);
    Ok(())
}

/// A backend's failure, as the picker shows it.
fn failure<T>(result: Result<T, Failure>) -> TestResult<String> {
    match result {
        Ok(_) => Err("the call succeeded".into()),
        Err(error) => Ok(error.to_string()),
    }
}

#[test]
fn kdes_refusals_and_a_missing_service_are_failures() -> TestResult {
    if !have("dbus-daemon") {
        skip!("no dbus-daemon");
    }
    let sandbox = Sandbox::new()?;
    let client = || zbus::blocking::connection::Builder::address(sandbox.bus_address())?.build();
    assert!(kde::Kde::over(client()?).is_err(), "nobody serves the keyboard service");
    // a third layout the service refuses to select, known by its short name alone, and a fourth it fails to
    let (service, current) = serve_kde_layouts(
        &sandbox,
        vec![
            ("us".to_owned(), "EN".to_owned(), US.to_owned()),
            ("ru".to_owned(), String::new(), RU.to_owned()),
            ("de".to_owned(), String::new(), String::new()),
            ("fr".to_owned(), String::new(), "French".to_owned()),
        ],
    )?;
    let mut kde = ok(kde::Kde::over(client()?))?;
    assert_eq!(
        ok(kde.layouts())?,
        [
            KeyboardLayout::named(US),
            KeyboardLayout::named(RU),
            KeyboardLayout::named("de"),
            KeyboardLayout::named("French")
        ],
        "a layout without a long name goes by its short one"
    );
    assert_eq!(failure(kde.select("de"))?, "KDE refused layout 2");
    assert!(
        failure(kde.select("French"))?.contains("no layout 3"),
        "the service's own failure"
    );
    assert_eq!(failure(kde.select("fr"))?, r#"no layout named "fr""#);
    *current.lock().unwrap_or_else(PoisonError::into_inner) = 5;
    assert_eq!(failure(kde.current())?, "no layout 5");

    // the service gone: every call fails
    service.release_name("org.kde.keyboard")?;
    assert!(kde.layouts().is_err() && kde.current().is_err() && kde.select(US).is_err());
    assert!(kde::Kde::connect(Some("no bus address")).is_err());
    let nobody = format!("unix:path={}", sandbox.path("run/no-bus").display());
    assert!(kde::Kde::connect(Some(&nobody)).is_err(), "no bus at the address");
    Ok(())
}

/// Names the private bus the KDE child runs on; the child refuses any other.
const PRIVATE_BUS: &str = "NGTERM_GPUI_TEST_PRIVATE_BUS";

#[test]
fn kde_is_reached_on_the_session_bus_the_environment_names() -> TestResult {
    if !have("dbus-daemon") {
        skip!("no dbus-daemon");
    }
    let sandbox = Sandbox::new()?;
    let (_service, _) = serve_kde(&sandbox)?;
    let status = sandbox
        .command(std::env::current_exe()?)
        .args([
            "--exact",
            "kde_on_the_session_bus",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(PRIVATE_BUS, sandbox.bus_address())
        .status()?;
    assert!(status.success(), "the KDE service was not reached on the session bus");
    Ok(())
}

/// [`kde::Kde::connect`] without an address, in a child of this test whose session bus is the sandbox's.
#[test]
#[ignore = "run by kde_is_reached_on_the_session_bus_the_environment_names, on its private bus"]
fn kde_on_the_session_bus() -> TestResult {
    let address = std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap_or_default();
    // only ever on the sandbox's private bus the parent named: never the user's
    if address.is_empty() || std::env::var(PRIVATE_BUS).ok().as_deref() != Some(address.as_str()) {
        eprintln!("skipped: not on the parent test's private bus");
        return Ok(());
    }
    let mut kde = ok(kde::Kde::connect(None))?;
    assert_eq!(
        ok(kde.layouts())?,
        [KeyboardLayout::named(US), KeyboardLayout::named(RU)]
    );
    Ok(())
}

#[test]
fn a_kwin_without_its_keyboard_service_has_no_backend() -> TestResult {
    if !have("dbus-daemon") {
        skip!("no dbus-daemon");
    }
    let sandbox = Sandbox::new()?;
    let Some((_kwin, display)) = kwin_wrapper(&sandbox)? else {
        skip!("no python3");
    };
    let var = |name: &str| match name {
        "WAYLAND_DISPLAY" => Some(display.clone()),
        "XDG_CURRENT_DESKTOP" => Some(PathBuf::from("KDE")),
        _ => sandbox.env_var(name).map(PathBuf::from),
    };
    assert_eq!(
        wayland(&var).err().as_deref(),
        Some(ngterm_gpui::input_source::linux::DESKTOP_UNAVAILABLE),
        "KWin serves the display, but nothing answers for its layouts"
    );
    Ok(())
}

/// The app on a private Xvfb with US and Russian, its lock on Russian; with the test's X connection.
fn start_locked_app(sandbox_and_x11: (Sandbox, X11), lavapipe: &Path) -> TestResult<(Sandbox, X11, usize)> {
    let (mut sandbox, x11) = sandbox_and_x11;
    // a state a launch left before (no welcome dialog takes the keyboard), the lock on Russian
    std::fs::create_dir_all(sandbox.path("state/windows"))?;
    std::fs::write(
        sandbox.path("state/settings.json"),
        format!(r#"{{ "terminalInputSource": "{RU}" }}"#),
    )?;
    sandbox.use_software_rendering(lavapipe);
    let primary = sandbox.start_gpui_primary("primary.log")?;
    Ok((sandbox, x11, primary))
}

#[test]
fn the_app_locks_the_layout_while_its_terminal_has_the_keyboard() -> TestResult {
    if !have("Xvfb") || !have("setxkbmap") {
        skip!("no Xvfb or setxkbmap");
    }
    let Some(lavapipe) = lavapipe_icd() else {
        skip!("no lavapipe ICD (scripts/fetch-lavapipe.sh)");
    };
    let (mut sandbox, x11, primary) = start_locked_app(two_layout_xvfb()?, &lavapipe)?;
    assert_eq!(x11.group()?, 0, "nothing locks before a terminal has the keyboard");

    x11.focus_app_window(&sandbox.app_id)?;
    x11.wait_group(1, &sandbox)?;
    // a switch made while the terminal has the keyboard is switched back
    ok(ok(x11::Xkb::connect(Some(&display_name(&sandbox)?)))?.lock_group(0))?;
    x11.wait_group(1, &sandbox)?;
    // the keyboard goes to another application: the earlier layout returns
    x11.focus_elsewhere()?;
    x11.wait_group(0, &sandbox)?;
    x11.focus_app_window(&sandbox.app_id)?;
    x11.wait_group(1, &sandbox)?;

    // the quit restores it
    sandbox.terminate(primary)?;
    assert_eq!(sandbox.wait_exit(primary, Duration::from_secs(10))?.0.code(), Some(0));
    assert_eq!(x11.group()?, 0, "the quit restored the earlier layout");
    Ok(())
}

#[test]
fn the_app_sees_a_layout_list_change_while_its_terminal_keeps_the_keyboard() -> TestResult {
    if !have("Xvfb") || !have("setxkbmap") {
        skip!("no Xvfb or setxkbmap");
    }
    let Some(lavapipe) = lavapipe_icd() else {
        skip!("no lavapipe ICD (scripts/fetch-lavapipe.sh)");
    };
    let (mut sandbox, x11, primary) = start_locked_app(two_layout_xvfb()?, &lavapipe)?;
    x11.focus_app_window(&sandbox.app_id)?;
    x11.wait_group(1, &sandbox)?;

    // Russian is dropped: US is the one layout left, which the lock reads as selected (the server keeps group 1 until
    // a state change, and wraps it into range) and leaves as it is; the user's own switch puts the server on group 0
    set_layouts(&sandbox, "us")?;
    let mut xkb = ok(x11::Xkb::connect(Some(&display_name(&sandbox)?)))?;
    assert_eq!(ok(xkb.current())?, US, "a group out of range wraps into range");
    ok(xkb.lock_group(0))?;
    x11.wait_group(0, &sandbox)?;
    // Russian is enabled again while the terminal keeps the keyboard; the selected layout's name stays US, so only the
    // new keymap tells the app, and nothing else (no focus or activation change) re-evaluates the lock
    set_layouts(&sandbox, "us,ru")?;
    x11.wait_group(1, &sandbox)?;

    sandbox.terminate(primary)?;
    assert_eq!(sandbox.wait_exit(primary, Duration::from_secs(10))?.0.code(), Some(0));
    assert_eq!(x11.group()?, 0, "the quit restored the earlier layout");
    Ok(())
}
