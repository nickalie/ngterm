//! A shown pane's appearance (its answers to color scheme queries), the session links tapped in it, and the keys typed
//! in it that its origin hears of for its Badge reset rule.

use futures_lite::future::block_on;
use ngterm_core::live::daemon_name;
use uuid::Uuid;

use ngterm_core::remote::frames::{self, Body, PresentationFrame, PresentationHello, PresentationMode};
use ngterm_core::remote::hub::PANE_INPUT_KIND;
use ngterm_core::remote::ssh::{present_tail, tree_tail};

use super::pane_tests::{Fixture, MACHINE, PANE, SESSION, contains, inputs_until, listing, send};
use crate::api::{CellPoint, PaneColorScheme};
use crate::api::{PaneKey, PaneKeyAction};
use crate::transport::ChannelEnd;
use crate::transport::fake::Script;

const OTHER: &str = "00000000-0000-0000-0000-0000000000A4";
const UNLISTED: &str = "00000000-0000-0000-0000-0000000000FF";
const ENDED: &str = "00000000-0000-0000-0000-0000000000A5";

/// A listing of the shown session and `OTHER`, each with its one pane.
fn two_sessions() -> Script {
    sessions_listing(&[(SESSION, PANE), (OTHER, "00000000-0000-0000-0000-0000000000B4")])
}

/// A listing of `sessions`, each with its one pane.
fn sessions_listing(sessions: &[(&str, &str)]) -> Script {
    let session = |id: &str, pane: &str| {
        serde_json::json!({
            "id": id, "name": id, "windowID": "w1", "windowName": "main", "workspaceID": "ws1",
            "workspaceName": "work", "cwd": "/repo",
            "panes": [{"pane": "left", "daemon": daemon_name(Uuid::parse_str(pane).unwrap())}],
        })
    };
    let remote = serde_json::json!({
        "endpoint": {"executable": "/ngterm-session", "socketDirectory": "/tmp/ngterm-live-x"},
        "sessions": sessions.iter().map(|(id, pane)| session(id, pane)).collect::<Vec<_>>(),
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

    pane.set_color_scheme(PaneColorScheme::Light);
    send(&again.peer, b"\x1b[?996n");
    let (bytes, _) = inputs_until(&mut again.peer, |bytes, _| contains(bytes, light));
    assert_eq!(bytes, light, "and goes back to light");
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

#[test]
fn a_link_to_a_session_made_after_the_listing_asks_the_machine_again() {
    let mut fixture = Fixture::connected_with(vec![listing(true), two_sessions()]);
    let pane = fixture.show();
    let opened = fixture.attach();
    fixture.attached();
    let lines = format!(
        "\x1b]8;;ngterm://session/{OTHER}\x1b\\new\x1b]8;;\x1b\\\r\n\
         \x1b]8;;https://example.com\x1b\\web\x1b]8;;\x1b\\"
    );
    send(&opened.peer, lines.as_bytes());
    fixture.dirty();
    let listings = || {
        let tree = tree_tail(ngterm_core::remote::origin::OriginShell::Posix);
        fixture
            .transport
            .execs()
            .iter()
            .filter(|exec| exec.command == tree)
            .count()
    };
    let before = listings();

    let at = |x: u16, y: u32| block_on(pane.session_link(CellPoint { x, y })).unwrap();
    assert_eq!(at(1, 1), None, "a web link");
    assert_eq!(listings(), before, "a web link asks the machine nothing");
    let found = at(1, 0).expect("the session made after the listing");
    assert_eq!(found.id, OTHER);
    assert_eq!(listings(), before + 1);
    assert!(
        fixture.events.try_iter().any(|event| matches!(
            event,
            crate::api::MobileEvent::Tree { sessions, .. } if sessions.iter().any(|session| session.id == OTHER)
        )),
        "the lists show the new session too"
    );
}

fn key(name: &str, mods: u8, text: &str) -> PaneKey {
    PaneKey {
        action: PaneKeyAction::Press,
        key: name.to_owned(),
        mods,
        consumed: 0,
        composing: false,
        text: text.to_owned(),
        unshifted: None,
    }
}

#[test]
fn keys_typed_in_the_pane_reach_an_origin_that_speaks_them() {
    let mut fixture = Fixture::connected();
    block_on(fixture.core.attach_session(MACHINE.to_owned(), SESSION.to_owned())).unwrap();
    let mut stream = fixture.opened(|opened| {
        opened.command == present_tail(ngterm_core::remote::origin::OriginShell::Posix, SESSION).unwrap()
    });
    let hello = Body::Hello(PresentationHello {
        version: frames::VERSION,
        kinds: vec![PANE_INPUT_KIND.to_owned()],
        mode: PresentationMode::Mirror,
    });
    let line = frames::encode(&PresentationFrame {
        generation: 1,
        rev: 1,
        body: hello,
    })
    .unwrap();
    send(&stream.peer, &line);
    let pane = fixture.show();
    fixture.attach();
    fixture.attached();

    assert!(pane.type_text("ls".to_owned()).unwrap());
    let plain = format!(r#""input":{{"pane":"{PANE}","key":"other"}}"#);
    inputs_until(&mut stream.peer, |bytes, _| contains(bytes, plain.as_bytes()));
    assert!(pane.key(key("enter", 0, "")).unwrap());
    let submit = format!(r#""input":{{"pane":"{PANE}","key":"submit"}}"#);
    inputs_until(&mut stream.peer, |bytes, _| contains(bytes, submit.as_bytes()));
}

#[test]
fn a_link_opens_its_session_from_a_fresh_listing_an_end_crossed_without_the_ended_one() {
    let ended = (ENDED, "00000000-0000-0000-0000-0000000000B5");
    let mut fixture = Fixture::connected_with(vec![sessions_listing(&[(SESSION, PANE), ended])]);
    let pane = fixture.show();
    let opened = fixture.attach();
    fixture.attached();
    send(
        &opened.peer,
        format!("\x1b]8;;ngterm://session/{OTHER}\x1b\\new\x1b]8;;\x1b\\").as_bytes(),
    );
    fixture.dirty();
    let tree = tree_tail(ngterm_core::remote::origin::OriginShell::Posix);
    // unscripted, the next listing waits for the test, which ends a session on the machine first
    fixture.transport.script_sequence(&tree, Vec::new());
    let close = ngterm_core::remote::ssh::control_tail(
        ngterm_core::remote::origin::OriginShell::Posix,
        &ngterm_core::remote::origin_replies::end_session_arguments(ENDED),
    );
    fixture
        .transport
        .script(&close, Script::ok(r#"{"ok": true, "result": {}}"#));

    let tapped = std::thread::spawn({
        let pane = std::sync::Arc::clone(&pane);
        move || block_on(pane.session_link(CellPoint { x: 1, y: 0 })).unwrap()
    });
    let relisting = fixture.opened(|opened| opened.command == tree);
    assert_eq!(
        block_on(fixture.core.end_session(MACHINE.to_owned(), ENDED.to_owned())),
        Ok(())
    );
    let fresh = sessions_listing(&[(SESSION, PANE), (OTHER, "00000000-0000-0000-0000-0000000000B4"), ended]);
    block_on(relisting.peer.data(fresh.stdout));
    block_on(relisting.peer.end(fresh.end));

    let found = tapped.join().unwrap().expect("the session the fresh listing holds");
    assert_eq!(found.id, OTHER);
    assert_eq!(
        block_on(fixture.core.attach_session(MACHINE.to_owned(), OTHER.to_owned())),
        Ok(()),
        "the app attaches the link's session by its id"
    );
    assert_eq!(
        block_on(fixture.core.attach_session(MACHINE.to_owned(), ENDED.to_owned())),
        Err(crate::MobileError::NotListed {
            session_id: ENDED.to_owned()
        }),
        "the session ended while the listing was in flight stays ended"
    );
}
