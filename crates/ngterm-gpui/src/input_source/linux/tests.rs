//! The Linux backends against fake compositor servers speaking each protocol (framing, requests, replies), and the
//! Wayland probe's choice: a compositor socket only when the Wayland display's server serves it, GNOME's and any
//! other desktop's reasons otherwise. The X11 backend and niri's run against the real thing on a private display
//! (`tests/input_source.rs`).

use std::collections::HashMap;
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::*;
use crate::input_source::KeyboardLayout;

/// What a fake server was sent, in order.
type Log = Arc<Mutex<Vec<String>>>;

/// Serves `path` on a thread: each connection is handed to `answer`, which reads the request and writes the reply;
/// what it returns is logged.
fn serve(path: &Path, answer: impl Fn(&mut UnixStream) -> Option<String> + Send + 'static) -> Log {
    let listener = UnixListener::bind(path).unwrap();
    let log: Log = Arc::default();
    let logged = Arc::clone(&log);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            if let Some(request) = answer(&mut stream) {
                logged.lock().unwrap().push(request);
            }
        }
    });
    log
}

fn sent(log: &Log) -> Vec<String> {
    log.lock().unwrap().clone()
}

// MARK: - sway

/// One i3-ipc message read from `stream`: its type and payload.
fn read_i3(stream: &mut UnixStream) -> Option<(u32, String)> {
    let mut header = [0u8; 14];
    stream.read_exact(&mut header).ok()?;
    assert_eq!(&header[..6], b"i3-ipc");
    let length = u32::from_ne_bytes(header[6..10].try_into().unwrap());
    let kind = u32::from_ne_bytes(header[10..14].try_into().unwrap());
    let mut payload = vec![0u8; length as usize];
    stream.read_exact(&mut payload).ok()?;
    Some((kind, String::from_utf8(payload).unwrap()))
}

fn write_i3(stream: &mut UnixStream, kind: u32, payload: &Value) {
    let body = payload.to_string();
    let mut message = b"i3-ipc".to_vec();
    message.extend(u32::try_from(body.len()).unwrap().to_ne_bytes());
    message.extend(kind.to_ne_bytes());
    message.extend(body.as_bytes());
    stream.write_all(&message).unwrap();
}

/// A fake sway: a pointer, a keyboard without layouts, and two keyboards listing US and Russian in opposite orders
/// (per-device configs), each with its own active index moved by `input "<identifier>" xkb_switch_layout <index>`
/// commands, several in one message joined by `; `.
fn fake_sway(path: &Path) -> Log {
    let active = Arc::new(Mutex::new(HashMap::from([("kbd", 0usize), ("ext", 1usize)])));
    serve(path, move |stream| {
        let (kind, payload) = read_i3(stream)?;
        let mut active = active.lock().unwrap();
        match kind {
            100 => write_i3(
                stream,
                100,
                &json!([
                    { "type": "pointer", "identifier": "mouse" },
                    { "type": "keyboard", "identifier": "power", "xkb_layout_names": [] },
                    {
                        "type": "keyboard",
                        "identifier": "kbd",
                        "xkb_layout_names": ["English (US)", "Russian"],
                        "xkb_active_layout_index": active["kbd"],
                    },
                    {
                        "type": "keyboard",
                        "identifier": "ext",
                        "xkb_layout_names": ["Russian", "English (US)"],
                        "xkb_active_layout_index": active["ext"],
                    },
                ]),
            ),
            0 => {
                let replies: Vec<Value> = payload
                    .split("; ")
                    .map(|command| {
                        let words: Vec<&str> = command.split(' ').collect();
                        let keyboard = words.get(1).map(|word| word.trim_matches('"'));
                        let index = words.get(3).and_then(|index| index.parse::<usize>().ok());
                        let success = matches!(words.as_slice(), ["input", _, "xkb_switch_layout", _])
                            && keyboard.is_some_and(|keyboard| active.contains_key(keyboard))
                            && index.is_some_and(|index| index < 2);
                        if success && let (Some(keyboard), Some(index)) = (keyboard, index) {
                            *active.get_mut(keyboard).unwrap() = index;
                        }
                        json!({ "success": success, "error": "bad command" })
                    })
                    .collect();
                write_i3(stream, 0, &Value::Array(replies));
                return Some(payload);
            }
            _ => panic!("unexpected sway message {kind}"),
        }
        None
    })
}

#[test]
fn sway_lists_reads_and_switches_each_keyboard_at_its_own_index() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sway.sock");
    let commands = fake_sway(&path);
    let mut sway = sway::Sway::new(path);
    assert_eq!(
        sway.layouts().unwrap(),
        [KeyboardLayout::named("English (US)"), KeyboardLayout::named("Russian")]
    );
    assert_eq!(sway.current().unwrap(), "English (US)");
    sway.select("Russian").unwrap();
    assert_eq!(sway.current().unwrap(), "Russian");
    sway.select("English (US)").unwrap();
    assert!(sway.select("German").is_err());
    assert_eq!(
        sent(&commands),
        [
            r#"input "kbd" xkb_switch_layout 1; input "ext" xkb_switch_layout 0"#,
            r#"input "kbd" xkb_switch_layout 0; input "ext" xkb_switch_layout 1"#,
        ],
        "each keyboard switched at its own layout's index, not one broadcast index"
    );
}

// MARK: - niri

/// A fake niri: US and Russian, `SwitchLayout` by index.
fn fake_niri(path: &Path) -> Log {
    let current = Arc::new(Mutex::new(0u64));
    serve(path, move |stream| {
        let mut line = String::new();
        BufReader::new(&*stream).read_line(&mut line).ok()?;
        let request: Value = serde_json::from_str(&line).unwrap();
        let mut current = current.lock().unwrap();
        let reply = if request == json!("KeyboardLayouts") {
            json!({ "Ok": { "KeyboardLayouts": { "names": ["English (US)", "Russian"], "current_idx": *current } } })
        } else if let Some(index) = request["Action"]["SwitchLayout"]["layout"]["Index"].as_u64() {
            *current = index;
            json!({ "Ok": "Handled" })
        } else {
            json!({ "Err": "unknown request" })
        };
        writeln!(stream, "{reply}").unwrap();
        Some(line.trim_end().to_owned())
    })
}

#[test]
fn niri_lists_reads_and_switches_its_layouts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("niri.sock");
    let requests = fake_niri(&path);
    let mut niri = niri::Niri::new(path);
    assert_eq!(
        niri.layouts().unwrap(),
        [KeyboardLayout::named("English (US)"), KeyboardLayout::named("Russian")]
    );
    niri.select("Russian").unwrap();
    assert_eq!(niri.current().unwrap(), "Russian");
    assert!(sent(&requests).contains(&r#"{"Action":{"SwitchLayout":{"layout":{"Index":1}}}}"#.to_owned()));
}

// MARK: - Hyprland

/// A fake Hyprland: a main keyboard on `us,ru`, `switchxkblayout` moving its active keymap.
fn fake_hyprland(path: &Path) -> Log {
    let active = Arc::new(Mutex::new("English (US)".to_owned()));
    serve(path, move |stream| {
        let mut request = [0u8; 256];
        let read = stream.read(&mut request).ok()?;
        let request = String::from_utf8_lossy(&request[..read]).into_owned();
        let mut active = active.lock().unwrap();
        let reply = if request == "j/devices" {
            json!({ "mice": [], "keyboards": [
                { "name": "power-button", "rules": "", "model": "", "layout": "us", "variant": "", "options": "",
                  "active_keymap": "English (US)", "main": false },
                { "name": "at-kbd", "rules": "", "model": "", "layout": "us,ru", "variant": "", "options": "",
                  "active_keymap": *active, "main": true },
            ] })
            .to_string()
        } else if request == "switchxkblayout at-kbd 1" {
            "Russian".clone_into(&mut active);
            "ok".to_owned()
        } else {
            "unknown request".to_owned()
        };
        stream.write_all(reply.as_bytes()).unwrap();
        Some(request)
    })
}

#[test]
fn hyprland_names_the_main_keyboards_layouts_and_switches_them() {
    let names = hyprland::layout_names(&[
        String::new(),
        String::new(),
        "us,ru".to_owned(),
        String::new(),
        String::new(),
    ]);
    let Ok(names) = names else {
        eprintln!("SKIPPED: xkbcommon finds no XKB data here: {names:?}");
        return;
    };
    assert_eq!(names, ["English (US)", "Russian"]);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".socket.sock");
    let requests = fake_hyprland(&path);
    let mut hyprland = hyprland::Hyprland::new(path);
    assert_eq!(
        hyprland.layouts().unwrap(),
        [KeyboardLayout::named("English (US)"), KeyboardLayout::named("Russian")]
    );
    assert_eq!(hyprland.current().unwrap(), "English (US)");
    hyprland.select("Russian").unwrap();
    assert_eq!(hyprland.current().unwrap(), "Russian");
    assert!(sent(&requests).contains(&"switchxkblayout at-kbd 1".to_owned()));
}

#[test]
fn hyprlands_sockets_live_under_the_runtime_dir_then_tmp() {
    assert_eq!(
        hyprland::sockets(Path::new("abc"), Some(Path::new("/run/user/1000"))),
        [
            PathBuf::from("/run/user/1000/hypr/abc/.socket.sock"),
            PathBuf::from("/tmp/hypr/abc/.socket.sock")
        ]
    );
}

// MARK: - the Wayland probe

/// An environment reader over `vars`.
fn env(vars: &[(&str, &Path)]) -> HashMap<String, PathBuf> {
    vars.iter()
        .map(|(name, value)| ((*name).to_owned(), value.to_path_buf()))
        .collect()
}

fn probe_in(vars: &HashMap<String, PathBuf>) -> Probe {
    wayland(&|name| vars.get(name).cloned())
}

#[test]
fn a_compositor_socket_the_wayland_server_serves_is_used() {
    let dir = tempfile::tempdir().unwrap();
    // this process serves both: the same peer
    let _display = UnixListener::bind(dir.path().join("wayland-1")).unwrap();
    let sway = dir.path().join("sway.sock");
    fake_sway(&sway);
    let vars = env(&[
        ("XDG_RUNTIME_DIR", dir.path()),
        ("WAYLAND_DISPLAY", Path::new("wayland-1")),
        ("SWAYSOCK", &sway),
    ]);
    let mut backend = probe_in(&vars).unwrap();
    assert_eq!(backend.current().unwrap(), "English (US)");
}

#[test]
fn a_compositor_socket_without_the_wayland_server_is_not_trusted() {
    let dir = tempfile::tempdir().unwrap();
    let sway = dir.path().join("sway.sock");
    fake_sway(&sway);
    let niri = dir.path().join("niri.sock");
    fake_niri(&niri);
    // no Wayland server answers at the display: an inherited socket of another session
    let vars = env(&[
        ("XDG_RUNTIME_DIR", dir.path()),
        ("WAYLAND_DISPLAY", Path::new("wayland-9")),
        ("SWAYSOCK", &sway),
        ("NIRI_SOCKET", &niri),
        ("XDG_CURRENT_DESKTOP", Path::new("ubuntu:GNOME")),
    ]);
    assert_eq!(probe_in(&vars).err().as_deref(), Some(GNOME_UNAVAILABLE));
    let vars = env(&[("XDG_RUNTIME_DIR", dir.path()), ("SWAYSOCK", &sway)]);
    assert_eq!(probe_in(&vars).err().as_deref(), Some(DESKTOP_UNAVAILABLE));
}

#[test]
fn a_display_server_gpui_does_not_name_has_no_backend() {
    assert_eq!(probe("").err().as_deref(), Some(DESKTOP_UNAVAILABLE));
}

#[test]
fn kwin_is_known_by_its_own_name_and_its_wrappers() {
    // Plasma 5.23+ starts KWin through `kwin_wayland_wrapper`, the display socket's peer, its name cut to 15 bytes
    assert!(is_kwin("kwin_wayland"));
    assert!(is_kwin("kwin_wayland_wr"));
    assert!(!is_kwin("kwin_x11"));
    assert!(!is_kwin("sway"));
}
