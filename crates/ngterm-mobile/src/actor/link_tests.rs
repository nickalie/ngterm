//! A shown pane's appearance (its answers to color scheme queries) and the session links tapped in it.

use futures_lite::future::block_on;
use ngterm_core::live::daemon_name;
use uuid::Uuid;

use super::pane_tests::{Fixture, PANE, SESSION, contains, inputs_until, send};
use crate::api::{CellPoint, PaneColorScheme};
use crate::transport::ChannelEnd;
use crate::transport::fake::Script;

const OTHER: &str = "00000000-0000-0000-0000-0000000000A4";
const UNLISTED: &str = "00000000-0000-0000-0000-0000000000FF";

/// A listing of the shown session and `OTHER`, each with its one pane.
fn two_sessions() -> Script {
    let session = |id: &str, pane: &str| {
        serde_json::json!({
            "id": id, "name": id, "windowID": "w1", "windowName": "main", "workspaceID": "ws1",
            "workspaceName": "work", "cwd": "/repo",
            "panes": [{"pane": "left", "daemon": daemon_name(Uuid::parse_str(pane).unwrap())}],
        })
    };
    let remote = serde_json::json!({
        "endpoint": {"executable": "/ngterm-session", "socketDirectory": "/tmp/ngterm-live-x"},
        "sessions": [session(SESSION, PANE), session(OTHER, "00000000-0000-0000-0000-0000000000B4")],
        "presentation": 1,
    });
    Script::ok(&serde_json::json!({"ok": true, "result": {"remote": remote}}).to_string())
}

#[test]
fn the_pane_answers_and_reports_the_apps_appearance_and_keeps_it_for_the_next_attach() {
    let mut fixture = Fixture::connected();
    let pane = fixture.show();
    let mut opened = fixture.attach();
    fixture.attached();

    send(&opened.peer, b"\x1b[?996n");
    let light = b"\x1b[?997;2n";
    let (bytes, _) = inputs_until(&mut opened.peer, |bytes, _| contains(bytes, light));
    assert_eq!(bytes, light, "light until the app says otherwise");

    // the status report's answer proves the mode is on before the flip
    send(&opened.peer, b"\x1b[?2031h\x1b[5n");
    let (bytes, _) = inputs_until(&mut opened.peer, |bytes, _| contains(bytes, b"\x1b[0n"));
    assert_eq!(bytes, b"\x1b[0n");
    pane.set_color_scheme(PaneColorScheme::Dark);
    let dark = b"\x1b[?997;1n";
    let (bytes, _) = inputs_until(&mut opened.peer, |bytes, _| contains(bytes, dark));
    assert_eq!(bytes, dark, "a program in mode 2031 gets the flip");

    block_on(opened.peer.end(ChannelEnd::Lost("the channel was reset".to_owned())));
    let mut again = fixture.attach();
    fixture.attached();
    send(&again.peer, b"\x1b[?996n");
    let (bytes, _) = inputs_until(&mut again.peer, |bytes, _| contains(bytes, dark));
    assert_eq!(bytes, dark, "a fresh terminal starts on the kept appearance");
}

#[test]
fn a_session_link_in_the_pane_names_a_session_the_machine_lists() {
    let mut fixture = Fixture::connected_with(vec![two_sessions()]);
    let pane = fixture.show();
    let opened = fixture.attach();
    fixture.attached();
    let lines = format!(
        "\x1b]8;;ngterm://session/{}\x1b\\other\x1b]8;;\x1b\\\r\n\
         \x1b]8;;ngterm://session/{UNLISTED}\x1b\\gone\x1b]8;;\x1b\\\r\n\
         \x1b]8;;https://example.com\x1b\\web\x1b]8;;\x1b\\\r\nplain",
        OTHER.to_ascii_lowercase()
    );
    send(&opened.peer, lines.as_bytes());
    fixture.dirty();

    let at = |x: u16, y: u32| block_on(pane.session_link(CellPoint { x, y })).unwrap();
    let found = at(2, 0).expect("the listed session");
    assert_eq!((found.id.as_str(), found.name.as_str()), (OTHER, OTHER));
    for (x, y) in [(1, 1), (1, 2), (1, 3), (10, 0)] {
        assert_eq!(at(x, y), None, "cell {x},{y}");
    }
}
