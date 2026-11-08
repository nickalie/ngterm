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

// MARK: - refusals and bad replies

/// A backend's failure, as the picker shows it.
fn failure<T>(result: Result<T, crate::input_source::Failure>) -> String {
    match result {
        Ok(_) => panic!("the call succeeded"),
        Err(error) => error.to_string(),
    }
}

/// A fake sway answering `GET_INPUTS` with `inputs` and every `RUN_COMMAND` with `replies`.
fn sway_answering(path: &Path, inputs: Value, replies: Value) -> Log {
    serve(path, move |stream| {
        let (kind, payload) = read_i3(stream)?;
        write_i3(stream, kind, if kind == 100 { &inputs } else { &replies });
        (kind == 0).then_some(payload)
    })
}

#[test]
fn sways_refused_switch_and_a_reply_of_another_protocol_are_failures() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sway.sock");
    let keyboard = json!([{ "type": "keyboard", "identifier": "kbd \"1\"", "xkb_layout_names": ["Russian"] }]);
    let commands = sway_answering(&path, keyboard, json!([{ "success": false, "error": "denied" }]));
    let mut sway = sway::Sway::new(path);
    assert_eq!(
        sway.current().unwrap(),
        "Russian",
        "a keyboard without an active index reads its first layout"
    );
    assert_eq!(failure(sway.select("Russian")), r#"sway refused the switch: "denied""#);
    // the server accepts this connection only after recording the refused command
    sway.layouts().unwrap();
    assert_eq!(
        sent(&commands),
        [r#"input "kbd \"1\"" xkb_switch_layout 0"#],
        "the identifier quoted as a sway argument"
    );

    let other = dir.path().join("other.sock");
    serve(&other, |stream| {
        read_i3(stream)?;
        stream.write_all(b"i3-ipX\0\0\0\0\0\0\0\0").unwrap();
        None
    });
    assert_eq!(failure(sway::Sway::new(other).layouts()), "not an i3-ipc reply");
}

#[test]
fn sway_without_a_keyboard_with_layouts_or_its_active_one_fails() {
    let dir = tempfile::tempdir().unwrap();
    let none = dir.path().join("none.sock");
    sway_answering(
        &none,
        json!([{ "type": "keyboard", "identifier": "power", "xkb_layout_names": [] }]),
        json!([]),
    );
    let mut sway = sway::Sway::new(none);
    assert_eq!(failure(sway.layouts()), "no keyboard with layouts");
    assert_eq!(failure(sway.select("Russian")), r#"no layout named "Russian""#);

    let out_of_range = dir.path().join("range.sock");
    sway_answering(
        &out_of_range,
        json!([{ "type": "keyboard", "identifier": "kbd", "xkb_layout_names": ["Russian"], "xkb_active_layout_index": 3 }]),
        json!([]),
    );
    assert_eq!(failure(sway::Sway::new(out_of_range).current()), "no active layout");
    assert!(
        sway::Sway::new(dir.path().join("gone.sock")).layouts().is_err(),
        "no compositor at the socket"
    );
}

/// A fake niri answering every request with `reply`, a line of its own; a connection that asks nothing (the Wayland
/// probe's look at the socket's peer) gets nothing.
fn niri_answering(path: &Path, reply: &'static str) {
    serve(path, move |stream| {
        let mut line = String::new();
        if BufReader::new(&*stream).read_line(&mut line).ok()? == 0 {
            return None;
        }
        writeln!(stream, "{reply}").unwrap();
        None
    });
}

#[test]
fn niris_refusals_and_replies_without_a_layout_are_failures() {
    let dir = tempfile::tempdir().unwrap();
    let cases = [
        (r#"{"Err":"denied"}"#, r#"niri refused "KeyboardLayouts": "denied""#),
        (r#"{"Unexpected":1}"#, r#"niri's reply: {"Unexpected":1}"#),
        (
            r#"{"Ok":{"KeyboardLayouts":{"names":["Russian"]}}}"#,
            "no current layout",
        ),
        (
            r#"{"Ok":{"KeyboardLayouts":{"names":["Russian"],"current_idx":4}}}"#,
            "no current layout",
        ),
    ];
    for (index, (reply, expected)) in cases.into_iter().enumerate() {
        let path = dir.path().join(format!("niri-{index}.sock"));
        niri_answering(&path, reply);
        let got = failure(niri::Niri::new(path).current());
        assert_eq!(got.trim_end(), expected, "{reply}");
    }
    let path = dir.path().join("niri-select.sock");
    niri_answering(
        &path,
        r#"{"Ok":{"KeyboardLayouts":{"names":["Russian"],"current_idx":0}}}"#,
    );
    assert_eq!(
        failure(niri::Niri::new(path).select("German")),
        r#"no layout named "German""#
    );
    assert!(
        niri::Niri::new(dir.path().join("gone.sock")).layouts().is_err(),
        "no compositor at the socket"
    );
}

/// A fake Hyprland answering `j/devices` with `devices` and any other command with `reply`; a connection that asks
/// nothing (the Wayland probe's look at the socket's peer) gets nothing.
fn hyprland_answering(path: &Path, devices: Value, reply: &'static str) -> Log {
    serve(path, move |stream| {
        let mut request = [0u8; 256];
        let read = stream.read(&mut request).ok().filter(|read| *read > 0)?;
        let request = String::from_utf8_lossy(&request[..read]).into_owned();
        let answer = if request == "j/devices" {
            devices.to_string()
        } else {
            reply.to_owned()
        };
        stream.write_all(answer.as_bytes()).unwrap();
        Some(request)
    })
}

/// A Hyprland keyboard on `layout`, active on `active`.
fn hyprland_keyboard(name: &str, layout: &str, active: &str) -> Value {
    json!({ "name": name, "rules": "", "model": "", "layout": layout, "variant": "", "options": "",
            "active_keymap": active })
}

#[test]
fn hyprland_without_a_keyboard_fails_and_one_not_marked_main_is_read() {
    let dir = tempfile::tempdir().unwrap();
    let none = dir.path().join("none.sock");
    hyprland_answering(&none, json!({ "mice": [] }), "ok");
    assert_eq!(failure(hyprland::Hyprland::new(none).current()), "no keyboards");
    let empty = dir.path().join("empty.sock");
    hyprland_answering(&empty, json!({ "keyboards": [] }), "ok");
    assert_eq!(failure(hyprland::Hyprland::new(empty).layouts()), "no keyboard");
    let unmarked = dir.path().join("unmarked.sock");
    hyprland_answering(
        &unmarked,
        json!({ "keyboards": [hyprland_keyboard("first", "us", "Russian"), hyprland_keyboard("second", "us", "")] }),
        "ok",
    );
    assert_eq!(
        hyprland::Hyprland::new(unmarked).current().unwrap(),
        "Russian",
        "none is marked main: the first keyboard"
    );
    assert!(
        hyprland::Hyprland::new(dir.path().join("gone.sock")).current().is_err(),
        "no compositor at the socket"
    );
}

#[test]
fn hyprlands_refused_switch_and_layouts_that_do_not_compile_are_failures() {
    let us = [
        String::new(),
        String::new(),
        "us".to_owned(),
        String::new(),
        String::new(),
    ];
    if let Err(error) = hyprland::layout_names(&us) {
        eprintln!("SKIPPED: xkbcommon finds no XKB data here: {error}");
        return;
    }
    let bogus = [
        String::new(),
        String::new(),
        "ngterm-no-such-layout".to_owned(),
        String::new(),
        "ngterm:no-such-option".to_owned(),
    ];
    assert_eq!(
        failure(hyprland::layout_names(&bogus)),
        r#"the layouts "ngterm-no-such-layout" ("") do not compile"#
    );

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".socket.sock");
    let keyboard = hyprland_keyboard("at-kbd", "us,ru", "English (US)");
    let requests = hyprland_answering(&path, json!({ "keyboards": [keyboard] }), "error: no such device\n");
    let mut hyprland = hyprland::Hyprland::new(path);
    assert_eq!(
        failure(hyprland.select("Russian")),
        "Hyprland refused the switch: error: no such device"
    );
    assert_eq!(failure(hyprland.select("German")), r#"no layout named "German""#);
    assert!(sent(&requests).contains(&"switchxkblayout at-kbd 1".to_owned()));
}

#[test]
fn hyprlands_and_niris_sockets_the_wayland_server_serves_are_used() {
    let dir = tempfile::tempdir().unwrap();
    // this process serves the display and both compositors' sockets: the same peer
    let _display = UnixListener::bind(dir.path().join("wayland-1")).unwrap();
    let hypr = dir.path().join("hypr/sig");
    std::fs::create_dir_all(&hypr).unwrap();
    let keyboard = hyprland_keyboard("at-kbd", "us", "English (US)");
    hyprland_answering(&hypr.join(".socket.sock"), json!({ "keyboards": [keyboard] }), "ok");
    let niri = dir.path().join("niri.sock");
    niri_answering(
        &niri,
        r#"{"Ok":{"KeyboardLayouts":{"names":["English (US)","Russian"],"current_idx":1}}}"#,
    );
    let display = [
        ("XDG_RUNTIME_DIR", dir.path()),
        ("WAYLAND_DISPLAY", Path::new("wayland-1")),
    ];
    let hyprland_vars = env(&[
        display[0],
        display[1],
        ("HYPRLAND_INSTANCE_SIGNATURE", Path::new("sig")),
    ]);
    assert_eq!(
        probe_in(&hyprland_vars).unwrap().current().unwrap(),
        "English (US)",
        "Hyprland's request socket under the runtime dir"
    );
    let niri_vars = env(&[
        display[0],
        display[1],
        ("HYPRLAND_INSTANCE_SIGNATURE", Path::new("ngterm-test-no-such-instance")),
        ("NIRI_SOCKET", &niri),
    ]);
    assert_eq!(
        probe_in(&niri_vars).unwrap().layouts().unwrap(),
        [KeyboardLayout::named("English (US)"), KeyboardLayout::named("Russian")],
        "no Hyprland instance of that signature: niri's socket"
    );
}
