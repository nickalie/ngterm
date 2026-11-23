//! What a headless origin (`ngterm-headless`) reports and refuses that an app with windows does not: `version`'s
//! `headless` and `versions`, `live.tree`'s `headless`, and the commands it cannot carry out (upstream's
//! `HeadlessCommandPolicy`, `ServerActions+Sessions.swift`, `ServerActions+Ask.swift`, `ServerActions+Pick.swift`).

use ngterm_proto::command::{AskCmd, LiveCmd, PickCmd, SessionCmd};
use ngterm_proto::{Args, AskButton, Command, PickItem, Response};
use serde_json::json;

use super::live::live;
use super::{Fixture, args, failure, wire};
use crate::commands::headless::PHYSICAL_WINDOW_COMMANDS;

fn headless(extra: usize) -> Fixture {
    let mut fixture = Fixture::new(extra);
    fixture.core.set_presenter_only();
    fixture
}

fn result(response: &Response) -> &serde_json::Value {
    assert!(response.ok, "{response:?}");
    response.result.as_ref().unwrap()
}

#[test]
fn version_reports_headless_and_the_versions_only_from_a_headless_origin() {
    let mut app = Fixture::new(0);
    let version = app.send(Command::Version, None, Args::default());
    let reported = result(&version);
    assert!(
        reported.get("headless").is_none() && reported.get("versions").is_none(),
        "{reported}"
    );
    let mut server = headless(0);
    let version = server.send(Command::Version, None, Args::default());
    let reported = result(&version);
    assert_eq!(reported["headless"], json!(true));
    assert_eq!(
        reported["versions"],
        json!({"server": "0.1.0", "protocol": 1, "catalog": 1})
    );
}

#[test]
fn live_tree_reports_a_headless_origin() {
    let mut live = live();
    let tree = live.fixture.send(Command::Live(LiveCmd::Tree), None, Args::default());
    assert!(result(&tree)["remote"].get("headless").is_none());
    live.fixture.core.set_presenter_only();
    let tree = live.fixture.send(Command::Live(LiveCmd::Tree), None, Args::default());
    assert_eq!(result(&tree)["remote"]["headless"], json!(true));
}

#[test]
fn a_command_that_drives_a_window_is_refused_by_name_on_a_headless_origin() {
    let mut server = headless(0);
    for cmd in PHYSICAL_WINDOW_COMMANDS {
        assert_eq!(
            server.send(cmd, None, Args::default()),
            failure(&format!(
                "{cmd} is not available on ngterm-headless: it drives a window on the machine showing it"
            )),
        );
    }
    let mut app = Fixture::new(0);
    let windows = app.send(
        Command::Window(ngterm_proto::command::WindowCmd::New),
        None,
        Args::default(),
    );
    assert!(windows.ok, "{windows:?}");
}

#[test]
fn wait_is_refused_on_a_headless_origin_after_the_commands_own_checks() {
    let mut server = headless(0);
    let session = wire(server.sessions[0]);
    let waiting = |a: &mut Args| {
        a.wait = Some(true);
        a.command = Some("true".to_owned());
    };
    assert_eq!(
        server.session(SessionCmd::New, None, args(waiting)),
        failure("session.new --wait is not available on ngterm-headless")
    );
    assert_eq!(
        server.session(SessionCmd::CompanionNew, Some(&session), args(waiting)),
        failure("session.companion.new --wait is not available on ngterm-headless")
    );
    assert_eq!(
        server.session(SessionCmd::New, None, args(|a| a.wait = Some(true))),
        failure("--wait requires --command")
    );
    assert!(
        server
            .session(SessionCmd::New, None, args(|a| a.command = Some("true".to_owned())))
            .ok
    );
    let mut app = Fixture::new(0);
    assert!(app.session(SessionCmd::New, None, args(waiting)).ok);
}

fn ask_args(style: &str) -> Args {
    args(|a| {
        a.title = Some("deploy?".to_owned());
        a.style = Some(style.to_owned());
        a.buttons = Some(vec![AskButton {
            id: "y".to_owned(),
            label: "Yes".to_owned(),
            hotkey: None,
        }]);
    })
}

#[test]
fn a_gui_ask_needs_a_target_on_a_headless_origin() {
    let mut server = headless(0);
    let session = wire(server.sessions[0]);
    assert_eq!(
        server.send(Command::Ask(AskCmd::Open), None, ask_args("gui")),
        failure("ask.open needs --target on ngterm-headless, which has no window to show it in")
    );
    assert_eq!(
        server.send(Command::Ask(AskCmd::Open), None, ask_args("nope")),
        failure("unknown style")
    );
    assert!(
        server
            .send(Command::Ask(AskCmd::Open), Some(&session), ask_args("gui"))
            .ok
    );
    let mut app = Fixture::new(0);
    assert!(app.send(Command::Ask(AskCmd::Open), None, ask_args("gui")).ok);
}

#[test]
fn a_pick_needs_a_session_on_a_headless_origin() {
    let items = || {
        args(|a| {
            a.items = Some(vec![PickItem {
                id: "a".to_owned(),
                label: "Alpha".to_owned(),
                ..PickItem::default()
            }]);
        })
    };
    let mut server = headless(0);
    let session = wire(server.sessions[0]);
    assert_eq!(
        server.send(Command::Pick(PickCmd::Open), None, items()),
        failure("pick.open needs --session on ngterm-headless, which has no window to show it in")
    );
    assert_eq!(
        server.send(Command::Pick(PickCmd::Open), None, Args::default()),
        failure("pick.open requires items")
    );
    assert!(server.send(Command::Pick(PickCmd::Open), Some(&session), items()).ok);
    let mut app = Fixture::new(0);
    assert!(app.send(Command::Pick(PickCmd::Open), None, items()).ok);
}

/// Upstream's `ServerActionsAskTests` and `ServerActionsPickTests` size cases: a request no frame can carry would wait
/// for a presenter forever.
#[test]
fn an_ask_or_a_pick_too_large_for_a_frame_is_refused_on_a_headless_origin() {
    let huge = "x".repeat(crate::remote::frames::MAX_FRAME_BYTES);
    let mut server = headless(0);
    let session = wire(server.sessions[0]);
    let mut ask = ask_args("gui");
    ask.message = Some(huge.clone());
    assert_eq!(
        server.send(Command::Ask(AskCmd::Open), Some(&session), ask),
        failure("ask is too large to send to a Mac")
    );
    let pick = args(|a| {
        a.items = Some(vec![PickItem {
            id: "a".to_owned(),
            label: huge,
            ..PickItem::default()
        }]);
    });
    assert_eq!(
        server.send(Command::Pick(PickCmd::Open), Some(&session), pick),
        failure("pick is too large to send to a Mac")
    );
}
