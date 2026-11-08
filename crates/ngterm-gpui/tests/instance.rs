//! The GPUI build as a process: single instance over a private session bus, the activation token reaching the
//! primary and raising its window (Xvfb; a niri nested in a headless weston), the SIGTERM quit and its exit status,
//! and the app id fallback; and the library's registration on its own in a child on the private bus (`Open`,
//! `org.gtk.Actions`, an activation after the app is gone, ids the bus refuses). Every process runs in a [`Sandbox`]:
//! nothing here can reach the user's session bus, desktop or live ngterm.
//!
//! A test that needs a tool this machine lacks (weston, Xvfb, lavapipe) prints `SKIPPED:` with the reason (seen with
//! `--nocapture`) and passes, unless `NGTERM_REQUIRE_DISPLAY_TESTS` is set, as CI sets it: then it fails. The
//! nested-niri check is `#[ignore]`d (CI has no niri): run it with `--ignored`.
//!
//! Linux only: the D-Bus single instance and the X11/Wayland activation on a private bus, Xvfb and weston.

#![cfg(target_os = "linux")]

#[macro_use]
#[expect(dead_code, reason = "the sandbox serves the process tests too")]
mod support;

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use ngterm_gpui::instance::gapplication::object_path;
use ngterm_gpui::instance::{Activation, BusRequest, Registration, register};

use support::{
    DEADLINE, Display, NO_SEATED_WESTON, Sandbox, TestResult, have, lavapipe_icd, seated_weston, wait_for,
    wait_for_socket,
};

/// The token and startup id a forwarding launch is handed in these tests.
const TOKEN: &str = "ngterm-test-token";
const STARTUP_ID: &str = "ngterm-test-launch_TIME4242";
/// The startup id of a primary's own launch.
const PRIMARY_STARTUP_ID: &str = "ngterm-test-primary_TIME777";

/// Starts the primary on the sandbox's display (none: headless) and waits until it owns its name and serves.
fn start_primary(sandbox: &mut Sandbox) -> TestResult<usize> {
    let primary = sandbox.start_gpui_primary("primary.log")?;
    sandbox.wait_owner(&sandbox.app_id.clone())?;
    Ok(primary)
}

#[test]
fn a_second_launch_forwards_its_token_to_the_primary_and_leaves_the_state_alone() -> TestResult {
    let mut sandbox = Sandbox::new()?;
    let primary = start_primary(&mut sandbox)?;
    let activations = sandbox.monitor_activations()?;
    let before = sandbox.settled_state(Duration::from_secs(1))?;
    let second = sandbox.spawn_gpui(
        &[("XDG_ACTIVATION_TOKEN", TOKEN), ("DESKTOP_STARTUP_ID", STARTUP_ID)],
        "second.log",
    )?;
    let (status, took) = sandbox.wait_exit(second, DEADLINE)?;
    assert!(status.success(), "{status:?}\n{}", sandbox.log("second.log"));
    assert!(took < Duration::from_secs(5), "forwarding took {took:?}");

    let (member, data) = activations.recv_timeout(DEADLINE)?;
    assert_eq!(member, "Activate");
    assert_eq!(data.get("activation-token").map(String::as_str), Some(TOKEN));
    assert_eq!(data.get("desktop-startup-id").map(String::as_str), Some(STARTUP_ID));
    assert_eq!(
        data.get("cwd").map(String::as_str),
        Some(&*sandbox.path("home").to_string_lossy())
    );
    let log = sandbox.wait_log("primary.log", "activation requested over the bus")?;
    assert!(log.contains(&format!("token=\"{TOKEN}\"")), "{log}");
    sandbox.wait_log("primary.log", "activation: no display server to raise a window on")?;

    let second_log = sandbox.log("second.log");
    assert!(second_log.contains("forwarded to the running instance"), "{second_log}");
    assert!(
        !second_log.contains("control socket at"),
        "the second launch served nothing"
    );
    assert_eq!(sandbox.state_snapshot(), before, "the forwarding launch wrote no state");
    assert!(sandbox.child(primary).try_wait()?.is_none(), "the primary runs on");
    Ok(())
}

#[test]
fn sigterm_quits_a_headless_primary_with_status_0_within_10_s() -> TestResult {
    let mut sandbox = Sandbox::new()?;
    let primary = start_primary(&mut sandbox)?;
    sandbox.terminate(primary)?;
    let (status, took) = sandbox.wait_exit(primary, Duration::from_secs(10))?;
    assert_eq!(status.code(), Some(0), "{status:?}\n{}", sandbox.log("primary.log"));
    assert!(took < Duration::from_secs(10));
    let log = sandbox.log("primary.log");
    assert!(
        log.contains("SIGTERM: quitting") && log.contains("quit: the state is saved"),
        "{log}"
    );
    assert!(!sandbox.path("state/ngterm.sock").exists(), "the socket is removed");
    assert!(
        sandbox.path("state/windows.json").is_file(),
        "the quit save wrote the index"
    );
    Ok(())
}

#[test]
fn the_launchs_token_and_startup_id_reach_no_child() -> TestResult {
    let mut sandbox = Sandbox::new()?;
    let seen = sandbox.path("hook-env");
    let config = sandbox.path("state/config");
    std::fs::create_dir_all(&config)?;
    std::fs::write(
        config.join("hooks.conf"),
        format!("on tree.changed env > {}\n", seen.display()),
    )?;
    let primary = sandbox.spawn_gpui(
        &[("XDG_ACTIVATION_TOKEN", TOKEN), ("DESKTOP_STARTUP_ID", STARTUP_ID)],
        "primary.log",
    )?;
    wait_for_socket(&sandbox.path("state/ngterm.sock"))?;
    let reply = sandbox.request("session.new")?;
    assert!(reply.contains("\"ok\":true"), "{reply}");
    let env = wait_for(|| {
        std::fs::read_to_string(&seen)
            .ok()
            .filter(|text| text.contains("AGT_EVENT_KIND=tree.changed"))
    })?;
    assert!(
        env.contains(&format!("NGTERM_APP_ID={}", sandbox.app_id)),
        "the app's environment: {env}"
    );
    assert!(!env.contains("XDG_ACTIVATION_TOKEN"), "{env}");
    assert!(!env.contains("DESKTOP_STARTUP_ID"), "{env}");
    sandbox.terminate(primary)?;
    assert_eq!(sandbox.wait_exit(primary, Duration::from_secs(10))?.0.code(), Some(0));
    Ok(())
}

/// The status of a quit with a wgpu device. It cannot catch the exit crash `main`'s `_exit` guards against: that one
/// needs NVIDIA's EGL (`docs/upstream-issues/gpui-wgpu-gles-exit-segfault.md`), and these tests render on Mesa only.
#[test]
fn sigterm_quits_a_windowed_primary_with_status_0() -> TestResult {
    if seated_weston().is_none() {
        skip!("{NO_SEATED_WESTON}");
    }
    let Some(lavapipe) = lavapipe_icd() else {
        skip!("no lavapipe ICD (scripts/fetch-lavapipe.sh)");
    };
    let mut sandbox = Sandbox::new()?;
    sandbox.start_weston()?;
    sandbox.use_software_rendering(&lavapipe);
    let primary = start_primary(&mut sandbox)?;
    // GPUI logs the adapter once its device is created and the surface configured with it (`select_adapter_and_device`
    // tests each adapter that way): on lavapipe, never a hardware driver
    sandbox.wait_log("primary.log", "Selected GPU adapter: \"llvmpipe")?;
    sandbox.terminate(primary)?;
    let (status, _) = sandbox.wait_exit(primary, Duration::from_secs(10))?;
    assert_eq!(status.code(), Some(0), "{status:?}\n{}", sandbox.log("primary.log"));
    Ok(())
}

#[test]
fn an_invalid_app_id_falls_back_to_the_default_one() -> TestResult {
    let mut sandbox = Sandbox::new()?;
    // the private bus only: the default name is the installed app's on the user's bus
    sandbox.set_env("NGTERM_APP_ID", "io.github.9invalid");
    let primary = sandbox.start_gpui_primary("primary.log")?;
    sandbox.wait_owner(ngterm_gpui::instance::DEFAULT_APP_ID)?;
    sandbox.terminate(primary)?;
    let (status, _) = sandbox.wait_exit(primary, Duration::from_secs(10))?;
    assert_eq!(status.code(), Some(0));
    Ok(())
}

#[test]
fn without_a_session_bus_the_launch_runs_alone() -> TestResult {
    let mut sandbox = Sandbox::new()?;
    let missing = sandbox.path("run/no-bus");
    sandbox.set_env("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={}", missing.display()));
    let primary = sandbox.start_gpui_primary("primary.log")?;
    sandbox.wait_log("primary.log", "no session bus; running without single-instance")?;
    sandbox.terminate(primary)?;
    let (status, _) = sandbox.wait_exit(primary, Duration::from_secs(10))?;
    assert_eq!(status.code(), Some(0));
    Ok(())
}

/// Names the private bus the registration child runs on; the child refuses any other.
const PRIVATE_BUS: &str = "NGTERM_GPUI_TEST_PRIVATE_BUS";

#[test]
fn a_registration_serves_gapplications_interfaces_and_refuses_an_id_the_bus_cannot_take() -> TestResult {
    let sandbox = Sandbox::new()?;
    let status = sandbox
        .command(std::env::current_exe()?)
        .args([
            "--exact",
            "registration_on_the_private_bus",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(PRIVATE_BUS, sandbox.bus_address())
        .status()?;
    assert!(status.success(), "the registration failed on the private bus");
    Ok(())
}

/// The next request the primary's handlers queued, or a failure.
fn next_request(requests: &async_channel::Receiver<BusRequest>) -> TestResult<BusRequest> {
    wait_for(|| requests.try_recv().ok()).map_err(|_| "no request reached the primary".into())
}

/// The library's registration on the session bus, in a child of this test whose session bus is the sandbox's: the
/// primary, a second registration of the id forwarding to it, `Open` raising as `Activate` does, `org.gtk.Actions`
/// with no actions, an activation answered after the app is gone, and ids the bus refuses.
#[test]
#[ignore = "run by a_registration_serves_gapplications_interfaces_and_refuses_an_id_the_bus_cannot_take"]
fn registration_on_the_private_bus() -> TestResult {
    let address = std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap_or_default();
    // only ever on the sandbox's private bus the parent named: never the user's
    if address.is_empty() || std::env::var(PRIVATE_BUS).ok().as_deref() != Some(address.as_str()) {
        eprintln!("skipped: not on the parent test's private bus");
        return Ok(());
    }
    let app_id = std::env::var("NGTERM_APP_ID")?;
    let state = std::env::temp_dir();
    let (requests, received) = async_channel::unbounded();
    let Registration::Primary(_primary) = register(&app_id, &state, requests)? else {
        return Err("the first registration is not the primary".into());
    };
    let Registration::Remote(remote) = register(&app_id, &state, async_channel::unbounded().0)? else {
        return Err("a second registration of the id does not find the primary".into());
    };
    let forwarded = Activation {
        token: Some(TOKEN.to_owned()),
        startup_id: None,
    };
    remote.activate(&forwarded, Some(std::path::Path::new("/srv")))?;
    assert_eq!(next_request(&received)?, BusRequest::Activate(forwarded));

    let client = zbus::blocking::connection::Builder::address(address.as_str())?.build()?;
    let path = object_path(&app_id);
    let data = HashMap::from([("desktop-startup-id", zbus::zvariant::Value::from(STARTUP_ID))]);
    client.call_method(
        Some(app_id.as_str()),
        path.as_str(),
        Some("org.gtk.Application"),
        "Open",
        &(vec!["file:///srv/notes.txt"], "", data),
    )?;
    assert_eq!(
        next_request(&received)?,
        BusRequest::Activate(Activation {
            token: None,
            startup_id: Some(STARTUP_ID.to_owned()),
        }),
        "an Open raises the window as an Activate does"
    );
    gapplications_actions_are_none(&client, &app_id)?;
    assert!(received.is_empty(), "an action asks the app nothing");

    // the app is gone: the activation is dropped, the call still answered
    drop(received);
    let empty = HashMap::<&str, zbus::zvariant::Value<'_>>::new();
    client.call_method(
        Some(app_id.as_str()),
        path.as_str(),
        Some("org.gtk.Application"),
        "Activate",
        &(empty,),
    )?;

    for (id, why) in [("io..ngterm", "no object path"), ("io.9ngterm", "no bus name")] {
        let Err(error) = register(id, &state, async_channel::unbounded().0) else {
            return Err(format!("{id} registered: {why}").into());
        };
        let message = error.to_string();
        assert!(
            message.starts_with(&format!("failed to register {id} on the session bus: ")),
            "{why}: {message}"
        );
    }
    Ok(())
}

/// `org.gtk.Actions` on the primary of `app_id`: no actions to list or describe, and an unknown one's activation is
/// answered.
fn gapplications_actions_are_none(client: &zbus::blocking::Connection, app_id: &str) -> TestResult {
    let path = object_path(app_id);
    let call = |member: &str| -> TestResult<zbus::Message> {
        Ok(client.call_method(Some(app_id), path.as_str(), Some("org.gtk.Actions"), member, &())?)
    };
    assert!(call("List")?.body().deserialize::<Vec<String>>()?.is_empty());
    let described = call("DescribeAll")?;
    let described: HashMap<String, (bool, zbus::zvariant::Signature, Vec<zbus::zvariant::OwnedValue>)> =
        described.body().deserialize()?;
    assert!(described.is_empty());
    let platform_data = HashMap::<&str, zbus::zvariant::Value<'_>>::new();
    client.call_method(
        Some(app_id),
        path.as_str(),
        Some("org.gtk.Actions"),
        "Activate",
        &("quit", Vec::<zbus::zvariant::Value<'_>>::new(), platform_data),
    )?;
    Ok(())
}

/// X11's activation message, `_NET_WM_USER_TIME` and the startup sequence's end, as a window manager would see them.
mod x11 {
    use x11rb::connection::Connection as _;
    use x11rb::protocol::Event;
    use x11rb::protocol::xproto::{AtomEnum, ChangeWindowAttributesAux, ConnectionExt as _, EventMask};

    use super::{DEADLINE, Display, Sandbox, TestResult};

    /// What the root window saw.
    #[derive(Debug, Default)]
    pub struct Seen {
        /// `_NET_ACTIVE_WINDOW`: the window, the source indication and the timestamp.
        pub active: Option<(u32, u32, u32)>,
        /// The startup-notification message, reassembled.
        pub startup: String,
    }

    pub struct Root {
        conn: x11rb::rust_connection::RustConnection,
        root: u32,
    }

    impl Root {
        /// Watches the sandbox's Xvfb root for client messages, as a window manager selects them.
        pub fn watch(sandbox: &Sandbox) -> TestResult<Self> {
            let Display::X11(number) = sandbox.display() else {
                return Err("not an X11 sandbox".into());
            };
            let (conn, screen) = x11rb::connect(Some(&format!(":{number}")))?;
            let root = conn.setup().roots[screen].root;
            let mask = EventMask::SUBSTRUCTURE_NOTIFY | EventMask::PROPERTY_CHANGE;
            conn.change_window_attributes(root, &ChangeWindowAttributesAux::new().event_mask(mask))?
                .check()?;
            Ok(Self { conn, root })
        }

        fn atom(&self, name: &str) -> TestResult<u32> {
            Ok(self.conn.intern_atom(false, name.as_bytes())?.reply()?.atom)
        }

        /// Collects until a NUL-terminated startup message (and, `with_active`, the activation) arrived, or the
        /// deadline.
        pub fn collect(&self, with_active: bool) -> TestResult<Seen> {
            let (active, begin, more) = (
                self.atom("_NET_ACTIVE_WINDOW")?,
                self.atom("_NET_STARTUP_INFO_BEGIN")?,
                self.atom("_NET_STARTUP_INFO")?,
            );
            let mut seen = Seen::default();
            let mut bytes = Vec::new();
            let started = std::time::Instant::now();
            while started.elapsed() < DEADLINE && ((with_active && seen.active.is_none()) || !bytes.contains(&0)) {
                let Some(Event::ClientMessage(event)) = self.conn.poll_for_event()? else {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                    continue;
                };
                if event.type_ == active {
                    let data = event.data.as_data32();
                    seen.active = Some((event.window, data[0], data[1]));
                } else if event.type_ == begin {
                    bytes = event.data.as_data8().to_vec();
                } else if event.type_ == more {
                    bytes.extend_from_slice(&event.data.as_data8());
                }
            }
            let end = bytes.iter().position(|&byte| byte == 0).unwrap_or(bytes.len());
            seen.startup = String::from_utf8_lossy(&bytes[..end]).into_owned();
            Ok(seen)
        }

        /// A CARDINAL property of `window`.
        pub fn cardinal(&self, window: u32, name: &str) -> TestResult<Option<u32>> {
            let reply = self
                .conn
                .get_property(false, window, self.atom(name)?, AtomEnum::CARDINAL, 0, 1)?
                .reply()?;
            Ok(reply.value32().and_then(|mut values| values.next()))
        }

        /// `WM_CLASS` of `window`, as text.
        pub fn class(&self, window: u32) -> TestResult<String> {
            let reply = self
                .conn
                .get_property(false, window, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 1024)?
                .reply()?;
            Ok(String::from_utf8_lossy(&reply.value).replace('\0', " "))
        }

        pub fn root(&self) -> u32 {
            self.root
        }

        /// The top-level window whose `WM_CLASS` names `app_id`.
        pub fn window_of(&self, app_id: &str) -> TestResult<u32> {
            let tree = self.conn.query_tree(self.root)?.reply()?;
            for window in tree.children {
                if self.class(window)?.contains(app_id) {
                    return Ok(window);
                }
            }
            Err(format!("no window of {app_id}").into())
        }
    }
}

/// Asserts what a forwarded launch with [`STARTUP_ID`] did on X11.
fn assert_x11_activation(root: &x11::Root, app_id: &str) -> TestResult {
    let seen = root.collect(true)?;
    let (window, source, time) = seen.active.ok_or("no _NET_ACTIVE_WINDOW reached the root")?;
    assert_ne!(window, root.root());
    assert!(root.class(window)?.contains(app_id), "the primary's window");
    assert_eq!(
        (source, time),
        (1, 4242),
        "an application's request with the launch's time"
    );
    assert_eq!(root.cardinal(window, "_NET_WM_USER_TIME")?, Some(4242));
    assert_eq!(seen.startup, format!("remove: ID={STARTUP_ID}"));
    Ok(())
}

#[test]
fn x11_launches_carry_their_startup_time_and_end_their_sequences() -> TestResult {
    if !have("Xvfb") {
        skip!("no Xvfb");
    }
    let Some(lavapipe) = lavapipe_icd() else {
        skip!("no lavapipe ICD (scripts/fetch-lavapipe.sh)");
    };
    let mut sandbox = Sandbox::new()?;
    sandbox.start_xvfb()?;
    sandbox.use_software_rendering(&lavapipe);
    let app_id = sandbox.app_id.clone();
    // the primary's own launch: its sequence ends once its window exists, with the launch's time on the window
    let root = x11::Root::watch(&sandbox)?;
    let primary = sandbox.spawn_gpui(&[("DESKTOP_STARTUP_ID", PRIMARY_STARTUP_ID)], "primary.log")?;
    let seen = root.collect(false)?;
    assert_eq!(
        seen.startup,
        format!("remove: ID={PRIMARY_STARTUP_ID}"),
        "{}",
        sandbox.log("primary.log")
    );
    assert!(seen.active.is_none(), "a new window is the window manager's to focus");
    let window = root.window_of(&app_id)?;
    assert_eq!(root.cardinal(window, "_NET_WM_USER_TIME")?, Some(777));
    wait_for_socket(&sandbox.path("state/ngterm.sock"))?;
    sandbox.wait_owner(&app_id)?;

    let root = x11::Root::watch(&sandbox)?;
    let second = sandbox.spawn_gpui(&[("DESKTOP_STARTUP_ID", STARTUP_ID)], "second.log")?;
    assert!(sandbox.wait_exit(second, DEADLINE)?.0.success());
    assert_x11_activation(&root, &app_id)?;

    sandbox.terminate(primary)?;
    assert_eq!(sandbox.wait_exit(primary, Duration::from_secs(10))?.0.code(), Some(0));
    Ok(())
}

/// The nested niri's focused window's app id.
fn niri_focused(sandbox: &Sandbox) -> Option<String> {
    let output = sandbox
        .command("niri")
        .args(["msg", "--json", "focused-window"])
        .output()
        .ok()?;
    let window: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    window.get("app_id")?.as_str().map(str::to_owned)
}

/// Waits until niri focuses the window of `app_id`.
fn wait_focused(sandbox: &Sandbox, app_id: &str) -> TestResult {
    wait_for(|| (niri_focused(sandbox).as_deref() == Some(app_id)).then_some(()))
        .map_err(|_| format!("niri never focused {app_id} (focused: {:?})", niri_focused(sandbox)).into())
}

/// Has niri spawn `argv`, so it hands the child an activation token of its own making, as a launcher does.
fn niri_spawn(sandbox: &Sandbox, argv: &[&str]) -> TestResult {
    let status = sandbox
        .command("niri")
        .args(["msg", "action", "spawn", "--"])
        .args(argv)
        .status()?;
    assert!(status.success());
    Ok(())
}

#[test]
#[ignore = "needs niri (none in CI): niri nested in a private headless weston; run with --ignored"]
fn a_launchers_token_raises_the_primary_under_a_nested_niri() -> TestResult {
    if !have("niri") || seated_weston().is_none() {
        skip!("no niri, or {NO_SEATED_WESTON}: the nested-niri activation check needs both");
    }
    let Some(lavapipe) = lavapipe_icd() else {
        skip!("no lavapipe ICD (scripts/fetch-lavapipe.sh)");
    };
    let mut sandbox = Sandbox::new()?;
    sandbox.use_software_rendering(&lavapipe);
    sandbox.start_nested_niri()?;
    let app_id = sandbox.app_id.clone();
    let _primary = start_primary(&mut sandbox)?;
    wait_focused(&sandbox, &app_id)?;

    // a decoy of another id, with a state of its own, takes the focus
    let decoy_id = format!("{app_id}.decoy");
    let decoy_state = sandbox.path("decoy-state");
    let decoy_socket = decoy_state.join("ngterm.sock");
    let decoy_env: BTreeMap<&str, String> = BTreeMap::from([
        ("NGTERM_APP_ID", decoy_id.clone()),
        ("NGTERM_STATE_DIR", decoy_state.to_string_lossy().into_owned()),
        ("NGTERM_CONTROL_SOCKET", decoy_socket.to_string_lossy().into_owned()),
    ]);
    let decoy_pairs: Vec<(&str, &str)> = decoy_env.iter().map(|(key, value)| (*key, value.as_str())).collect();
    sandbox.spawn_gpui(&decoy_pairs, "decoy.log")?;
    wait_focused(&sandbox, &decoy_id)?;

    niri_spawn(&sandbox, &[support::GPUI])?;
    wait_focused(&sandbox, &app_id)?;
    let log = sandbox.wait_log("primary.log", "activation: window raised")?;
    assert!(log.contains("token=true"), "the token arrived: {log}");
    Ok(())
}
