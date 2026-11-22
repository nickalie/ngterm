use ngterm_core::remote::origin_replies::{
    self, NEW_SESSION_ARGUMENTS, NOT_ATTACHABLE, PAUSE, READY_BUDGET, RESTORE_MODE_ARGUMENTS, SESSION_NOUN,
    duplicate_session_arguments, end_session_arguments,
};
use ngterm_core::remote::ssh::{control_tail, tree_tail, tree_tail_for};
use ngterm_core::remote::tree::{RemoteCommandResult, TREE_DEADLINE};
use serde_json::json;
use tokio::time::Instant;

use super::*;
use crate::api::{Machine, MobileError};
use crate::transport::fake::{FakeConnection, FakeTransport, Script};
use crate::transport::{ChannelInput, Transport};

const HOST: &str = "box";
const CREATED: &str = "0E2B7C1A-5D4F-4E3B-9A61-7C2D8F0B1A23";
const PARENT: &str = "00000000-0000-0000-0000-0000000000A1";
const COMPANION: &str = "00000000-0000-0000-0000-0000000000A2";
const PAGE: &str = "00000000-0000-0000-0000-0000000000A3";
const MISSING_CLI: &str = "/bin/sh: 1: ngtermctl: not found\n";

fn machine() -> Machine {
    Machine {
        id: "m1".to_owned(),
        label: HOST.to_owned(),
        host: "box.example".to_owned(),
        port: 22,
        user: "nick".to_owned(),
        key_id: "key".to_owned(),
    }
}

/// A connection a listing told is ngterm's, as the app lists a machine before it creates or ends anything there.
async fn connected(transport: &FakeTransport) -> FakeConnection {
    let connection = fresh(transport).await;
    connection.set_origin_peer(ngterm_core::remote::peer::RemotePeer::Ngterm);
    connection
}

/// A connection nothing has been listed on yet.
async fn fresh(transport: &FakeTransport) -> FakeConnection {
    transport.connect(&machine()).await.expect("the fake connects")
}

fn session(id: &str, name: &str, parent: Option<&str>, content: Option<&str>) -> serde_json::Value {
    let mut session = json!({
        "id": id, "name": name, "windowID": "w1", "windowName": "main", "workspaceID": "ws1",
        "workspaceName": "work", "cwd": "/repo", "panes": [{"pane": "left", "daemon": "ngterm-live-0"}],
    });
    if let Some(parent) = parent {
        session["parent"] = json!(parent);
    }
    if let Some(content) = content {
        session["content"] = json!(content);
        session["panes"] = json!([]);
    }
    session
}

fn ok(body: &serde_json::Value) -> Script {
    Script::ok(&body.to_string())
}

fn exits(code: u32, stdout: &str, stderr: &str) -> Script {
    Script {
        stdout: stdout.as_bytes().to_vec(),
        stderr: stderr.as_bytes().to_vec(),
        end: ChannelEnd::Exit {
            code: Some(code),
            signal: None,
        },
    }
}

fn listing(sessions: &[serde_json::Value]) -> Script {
    ok(&json!({"ok": true, "result": {"remote": {
        "endpoint": {"executable": "/ngterm-session", "socketDirectory": "/tmp/ngterm-live-x"},
        "sessions": sessions,
    }}}))
}

fn restore_mode(active: &str) -> Script {
    ok(&json!({"ok": true, "result": {"restore": {
        "configured": active, "requestedAtLaunch": active, "active": active, "restartRequired": false,
    }}}))
}

fn created_receipt() -> serde_json::Value {
    json!({"ok": true, "result": {"id": CREATED}})
}

fn refusal(error: &str) -> Script {
    exits(1, &json!({"ok": false, "error": error}).to_string(), "")
}

/// The listing a create's wait runs on an ngterm machine: ngterm's alone, as it created there.
fn ready_tail() -> String {
    tree_tail_for(
        ngterm_core::remote::origin::OriginShell::Posix,
        Some(ngterm_core::remote::peer::RemotePeer::Ngterm),
    )
}

fn restore_line() -> String {
    control_tail(ngterm_core::remote::origin::OriginShell::Posix, &RESTORE_MODE_ARGUMENTS)
}

fn new_line(name: Option<&str>) -> String {
    control_tail(
        ngterm_core::remote::origin::OriginShell::Posix,
        &new_session_arguments(name),
    )
}

fn end_line(session: &str) -> String {
    control_tail(
        ngterm_core::remote::origin::OriginShell::Posix,
        &end_session_arguments(session),
    )
}

fn commands(transport: &FakeTransport) -> Vec<String> {
    transport.execs().into_iter().map(|exec| exec.command).collect()
}

/// A live machine whose `session new` answers with [`CREATED`].
fn creating(transport: &FakeTransport, name: Option<&str>) {
    transport.script(&restore_line(), restore_mode("live"));
    transport.script(&new_line(name), ok(&created_receipt()));
}

// MARK: - the list

#[tokio::test]
async fn a_windows_machine_is_learned_from_its_shells_refusal_and_keeps_its_form() {
    use ngterm_core::remote::origin::{self, OriginShell};
    let transport = FakeTransport::default();
    let connection = connected(&transport).await;
    transport.script(
        &tree_tail(OriginShell::Posix),
        exits(
            1,
            "",
            "\'\'\' is not recognized as an internal or external command,\r\n",
        ),
    );
    transport.script(origin::WINDOWS_PROBE, exits(0, "%OS%Windows_NT\r\n", ""));
    transport.script(
        &tree_tail(OriginShell::Windows),
        listing(&[session("A", "a", None, None)]),
    );
    let tree = list(&connection, "pc").await.unwrap();
    assert_eq!(tree.sessions.len(), 1);
    assert_eq!(connection.origin_shell(), OriginShell::Windows);
    // the next listing goes straight to the Windows form
    let asked = commands(&transport).len();
    list(&connection, "pc").await.unwrap();
    assert_eq!(commands(&transport)[asked..], [tree_tail(OriginShell::Windows)]);
}

/// An agterm Mac answers the same listing under its marker line: the connection learns it, every session says so, its
/// later lines go to `agtermctl`, and the agent chat's calls are refused, as agterm has none.
#[tokio::test]
async fn an_agterm_machine_is_learned_from_its_listing_and_gets_agterms_lines() {
    use ngterm_core::remote::origin::{OriginForm, OriginShell};
    use ngterm_core::remote::peer::RemotePeer;
    let transport = FakeTransport::new();
    let body = json!({"ok": true, "result": {"remote": {
        "endpoint": {"executable": "/Applications/agterm.app/Contents/MacOS/zmx", "socketDirectory": "/tmp/agterm-zmx-1"},
        "sessions": [session(PARENT, "parent", None, None)],
    }}});
    transport.script(
        &tree_tail(OriginShell::Posix),
        Script::ok(&format!("{}\n{body}", ngterm_core::remote::peer::AGTERM_ANSWER)),
    );
    let agterm = OriginForm {
        shell: OriginShell::Posix,
        peer: RemotePeer::Agterm,
    };
    transport.script(
        &control_tail(agterm, &end_session_arguments(PARENT)),
        ok(&json!({"ok": true})),
    );
    let connection = connected(&transport).await;

    let tree = list(&connection, HOST).await.expect("a listing");

    assert_eq!(connection.origin_form(), agterm);
    assert_eq!(tree.endpoint.peer, Some(RemotePeer::Agterm));
    let listed = listed_sessions(&tree);
    assert_eq!(listed[0].peer, Some(crate::peer::PeerKind::Agterm));
    end(&connection, HOST, PARENT, None).await.expect("ended on agterm");
    let lines = commands(&transport);
    assert!(
        lines[1].contains("agtermctl") && !lines[1].contains("ngtermctl"),
        "{}",
        lines[1]
    );
    assert_eq!(
        crate::chat::offers_chat(&connection),
        Err(MobileError::Remote {
            reason: crate::chat::NO_CHAT_ON_AGTERM.to_owned()
        })
    );
}

/// An agterm Mac that lists nothing is asked, read-only, for its restore mode, so the listing says why; one in Live
/// sessions mode, or one that does not answer the ask, lists nothing and says nothing more.
#[tokio::test(start_paused = true)]
async fn an_agterm_that_lists_nothing_is_asked_its_mode_and_the_listing_says_why() {
    use ngterm_core::remote::origin::OriginShell;
    let empty = json!({"ok": true, "result": {"remote": {
        "endpoint": {"executable": "/Applications/agterm.app/Contents/MacOS/zmx", "socketDirectory": "/tmp/agterm-zmx-1"},
        "sessions": [],
    }}});
    let answer = Script::ok(&format!("{}\n{empty}", ngterm_core::remote::peer::AGTERM_ANSWER));
    let cases = [
        (
            Some(restore_mode("none")),
            Some(
                "agterm on box is not in Live sessions mode: set Settings > General > Restore sessions to Live \
                 sessions and restart agterm; only sessions started after that can be attached",
            ),
        ),
        (Some(restore_mode("live")), None),
        (Some(exits(1, "", "Error: agterm is not running\n")), None),
        (None, None),
    ];
    for (restore, said) in cases {
        let transport = FakeTransport::new();
        transport.script(&tree_tail(OriginShell::Posix), answer.clone());
        if let Some(restore) = restore {
            transport.script(&not_live::restore_tail(), restore);
        }
        let connection = fresh(&transport).await;

        let tree = list(&connection, HOST).await.expect("a listing");

        assert!(tree.sessions.is_empty());
        assert_eq!(not_live::hint(&tree).as_deref(), said);
        assert_eq!(
            commands(&transport),
            [tree_tail(OriginShell::Posix), not_live::restore_tail()]
        );
    }
}

/// A flow's listing on a fresh connection reads the machine plainly, as desktop's flows do: an agterm that lists
/// nothing is asked its restore mode once, by the flow that needs it, never also by the listing before it.
#[tokio::test(start_paused = true)]
async fn a_flows_listing_of_an_empty_agterm_does_not_ask_its_mode() {
    use ngterm_core::remote::origin::{OriginForm, OriginShell};
    use ngterm_core::remote::peer::{AGTERM_ANSWER, RemotePeer};
    let transport = FakeTransport::new();
    let empty = json!({"ok": true, "result": {"remote": {
        "endpoint": {"executable": "/Applications/agterm.app/Contents/MacOS/zmx", "socketDirectory": "/tmp/agterm-zmx-1"},
        "sessions": [],
    }}});
    transport.script(
        &tree_tail(OriginShell::Posix),
        Script::ok(&format!("{AGTERM_ANSWER}\n{empty}")),
    );
    let agterm = OriginForm {
        shell: OriginShell::Posix,
        peer: RemotePeer::Agterm,
    };
    transport.script(&control_tail(agterm, &RESTORE_MODE_ARGUMENTS), restore_mode("none"));
    let connection = fresh(&transport).await;

    let error = create(&connection, HOST, None, || {}).await.expect_err("not live");

    assert!(error.contains("restore mode none"), "{error}");
    assert_eq!(
        commands(&transport),
        [
            tree_tail(OriginShell::Posix),
            control_tail(agterm, &RESTORE_MODE_ARGUMENTS)
        ]
    );
}

/// A first control line on a fresh connection is agterm's on an agterm-only Mac: the app is listed first, never guessed.
#[tokio::test]
async fn a_fresh_connections_first_control_line_lists_the_app_first() {
    use ngterm_core::remote::origin::{OriginForm, OriginShell};
    use ngterm_core::remote::peer::RemotePeer;
    let transport = FakeTransport::new();
    let body = json!({"ok": true, "result": {"remote": {
        "endpoint": {"executable": "/Applications/agterm.app/Contents/MacOS/zmx", "socketDirectory": "/tmp/agterm-zmx-1"},
        "sessions": [session(PARENT, "parent", None, None)],
    }}});
    transport.script(
        &tree_tail(OriginShell::Posix),
        Script::ok(&format!("{}\n{body}", ngterm_core::remote::peer::AGTERM_ANSWER)),
    );
    let agterm = OriginForm {
        shell: OriginShell::Posix,
        peer: RemotePeer::Agterm,
    };
    transport.script(
        &control_tail(agterm, &end_session_arguments(PARENT)),
        ok(&json!({"ok": true})),
    );
    let connection = fresh(&transport).await;

    end(&connection, HOST, PARENT, None).await.expect("ended on agterm");

    assert_eq!(
        commands(&transport),
        [
            tree_tail(OriginShell::Posix),
            control_tail(agterm, &end_session_arguments(PARENT))
        ]
    );
    assert!(connection.learned().peer_known);
}

/// A machine that cannot be listed gets no control line guessed at its app, and says why.
#[tokio::test]
async fn a_fresh_connection_whose_listing_fails_sends_no_control_line() {
    let transport = FakeTransport::new();
    transport.script(
        &tree_tail(ngterm_core::remote::origin::OriginShell::Posix),
        exits(127, "", MISSING_CLI),
    );
    let connection = fresh(&transport).await;

    let error = end(&connection, HOST, PARENT, None).await.expect_err("nothing answers");

    assert!(error.contains("ngtermctl: not found"), "{error}");
    assert_eq!(
        commands(&transport),
        [tree_tail(ngterm_core::remote::origin::OriginShell::Posix)]
    );
    assert!(!connection.learned().peer_known);
}

#[tokio::test]
async fn the_list_drops_page_companions_and_puts_parents_first() {
    let transport = FakeTransport::new();
    transport.script(
        &tree_tail(ngterm_core::remote::origin::OriginShell::Posix),
        listing(&[
            session(COMPANION, "companion", Some(PARENT), None),
            session(PAGE, "page", Some(PARENT), Some("page")),
            session(PARENT, "parent", None, None),
        ]),
    );
    let connection = connected(&transport).await;

    let tree = list(&connection, HOST).await.expect("a listing");

    let names: Vec<String> = listed_sessions(&tree).into_iter().map(|session| session.name).collect();
    assert_eq!(names, ["parent", "companion"]);
    assert_eq!(tree.sessions.len(), 3, "the whole listing is kept for attaching");
    let companion = &listed_sessions(&tree)[1];
    assert_eq!(companion.parent.as_deref(), Some(PARENT));
    assert_eq!(companion.peer, Some(crate::peer::PeerKind::Ngterm));
    assert!(crate::chat::offers_chat(&connection).is_ok());
    assert_eq!(companion.pane_count, 1);
    assert_eq!(
        commands(&transport),
        [tree_tail(ngterm_core::remote::origin::OriginShell::Posix)]
    );
    assert_eq!(transport.execs()[0].pty, None);
}

/// Convert to Companion on the machine: the fresh session it opened lists first as the parent, the converted session
/// and its old companion under it.
#[tokio::test]
async fn a_converted_group_lists_its_fresh_parent_first_with_both_under_it() {
    const FRESH: &str = "00000000-0000-0000-0000-0000000000A4";
    let transport = FakeTransport::new();
    transport.script(
        &tree_tail(ngterm_core::remote::origin::OriginShell::Posix),
        listing(&[
            session(PARENT, "converted", Some(FRESH), None),
            session(COMPANION, "companion", Some(FRESH), None),
            session(FRESH, "fresh", None, None),
        ]),
    );
    let connection = connected(&transport).await;

    let tree = list(&connection, HOST).await.expect("a listing");

    let listed: Vec<(String, Option<String>)> = listed_sessions(&tree)
        .into_iter()
        .map(|session| (session.name, session.parent))
        .collect();
    assert_eq!(
        listed,
        [
            ("fresh".to_owned(), None),
            ("converted".to_owned(), Some(FRESH.to_owned())),
            ("companion".to_owned(), Some(FRESH.to_owned())),
        ]
    );
}

#[tokio::test]
async fn a_list_the_far_side_refuses_says_why() {
    let transport = FakeTransport::new();
    transport.script(
        &tree_tail(ngterm_core::remote::origin::OriginShell::Posix),
        refusal("live sessions are off"),
    );
    let connection = connected(&transport).await;

    assert_eq!(list(&connection, HOST).await, Err("live sessions are off".to_owned()));
}

#[tokio::test]
async fn a_missing_ngtermctl_surfaces_the_shells_words() {
    let transport = FakeTransport::new();
    transport.script(
        &tree_tail(ngterm_core::remote::origin::OriginShell::Posix),
        exits(127, "", MISSING_CLI),
    );
    let connection = connected(&transport).await;

    assert_eq!(list(&connection, HOST).await, Err(MISSING_CLI.trim().to_owned()));
}

#[tokio::test]
async fn an_unreadable_list_and_a_silent_failure_read_as_desktops_strings() {
    let transport = FakeTransport::new();
    transport.script(
        &tree_tail(ngterm_core::remote::origin::OriginShell::Posix),
        Script::ok("not json"),
    );
    let connection = connected(&transport).await;
    assert_eq!(
        list(&connection, HOST).await,
        Err("the remote did not return a readable session list".to_owned())
    );

    transport.script(
        &tree_tail(ngterm_core::remote::origin::OriginShell::Posix),
        exits(1, "", ""),
    );
    assert_eq!(
        list(&connection, HOST).await,
        Err(format!("the remote command failed on {HOST}"))
    );
}

#[tokio::test]
async fn a_lost_link_reads_as_its_reason_and_a_closed_connection_as_the_transports() {
    let transport = FakeTransport::new();
    transport.script(
        &tree_tail(ngterm_core::remote::origin::OriginShell::Posix),
        Script {
            stdout: Vec::new(),
            stderr: Vec::new(),
            end: ChannelEnd::Lost("the connection was lost".to_owned()),
        },
    );
    let connection = connected(&transport).await;
    assert_eq!(list(&connection, HOST).await, Err("the connection was lost".to_owned()));

    connection.close();
    assert_eq!(
        list(&connection, HOST).await,
        Err("the connection is closed".to_owned())
    );
}

#[tokio::test(start_paused = true)]
async fn a_list_that_never_ends_runs_out_of_time_and_closes_its_channel() {
    let transport = FakeTransport::new();
    let connection = connected(&transport).await;
    let started = Instant::now();

    assert_eq!(list(&connection, HOST).await, Err(NO_ANSWER_IN_TIME.to_owned()));

    assert_eq!(started.elapsed(), TREE_DEADLINE);
    let mut opened = transport.take_opened();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].peer.try_input(), Some(ChannelInput::Close));
}

#[tokio::test]
async fn a_run_reports_status_output_and_a_signal_as_desktops_runner_does() {
    let transport = FakeTransport::new();
    transport.script(
        "signalled",
        Script {
            stdout: b"out".to_vec(),
            stderr: b"err".to_vec(),
            end: ChannelEnd::Exit {
                code: None,
                signal: Some("KILL".to_owned()),
            },
        },
    );
    let connection = connected(&transport).await;

    let result = run(&connection, "signalled", TREE_DEADLINE).await;

    assert_eq!(
        result,
        Ok(RemoteCommandResult {
            status: -1,
            stdout: "out".to_owned(),
            stderr: "err".to_owned(),
        })
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_remote_command_keeps_apostrophes_with_marks_as_literal_bytes() {
    use std::os::unix::fs::PermissionsExt as _;

    let arguments = [
        "session",
        "new",
        "--name",
        "it'\u{301}s; echo INJECTED",
        "x'\u{fe0f}",
        "y'\u{1f3fb}",
    ];
    let transport = FakeTransport::new();
    let line = control_tail(OriginShell::Posix, &arguments);
    transport.script(&line, Script::ok("{}"));
    let connection = connected(&transport).await;
    control_in(&connection, connection.origin_form(), &arguments)
        .await
        .unwrap();
    let home = tempfile::tempdir().unwrap();
    let bin = home.path().join(".local/bin");
    std::fs::create_dir_all(&bin).unwrap();
    let cli = bin.join("ngtermctl");
    std::fs::write(&cli, "#!/bin/sh\nprintf '%s\\0' \"$@\"\n").unwrap();
    std::fs::set_permissions(cli, std::fs::Permissions::from_mode(0o755)).unwrap();
    let output = std::process::Command::new("/bin/sh")
        .args(["-c", &commands(&transport)[0]])
        .env_clear()
        .env("HOME", home.path())
        .env(
            "PATH",
            std::env::join_paths([bin, "/usr/bin".into(), "/bin".into()]).unwrap(),
        )
        .output()
        .unwrap();
    let mut expected = arguments.to_vec();
    expected.push("--json");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, format!("{}\0", expected.join("\0")).into_bytes());
}

// MARK: - New Session

#[tokio::test(start_paused = true)]
async fn create_checks_live_creates_and_lists_until_the_session_is_there() {
    let transport = FakeTransport::new();
    creating(&transport, None);
    let parent = session(PARENT, "parent", None, None);
    transport.script_sequence(
        &ready_tail(),
        vec![
            listing(std::slice::from_ref(&parent)),
            listing(&[parent.clone(), session(CREATED, "new", None, None)]),
        ],
    );
    let connection = connected(&transport).await;
    let started = Instant::now();

    let (created, tree) = create(&connection, HOST, None, || {}).await.expect("created");

    assert_eq!(created.id, CREATED);
    assert_eq!(tree.sessions.len(), 2);
    assert_eq!(started.elapsed(), PAUSE, "one pause between the two listings");
    assert_eq!(
        commands(&transport),
        [
            restore_line(),
            control_tail(ngterm_core::remote::origin::OriginShell::Posix, &NEW_SESSION_ARGUMENTS),
            ready_tail(),
            ready_tail()
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn create_with_a_name_passes_it_as_one_value() {
    let transport = FakeTransport::new();
    creating(&transport, Some("-build logs"));
    transport.script(&ready_tail(), listing(&[session(CREATED, "-build logs", None, None)]));
    let connection = connected(&transport).await;

    let (created, _) = create(&connection, HOST, Some("  -build logs "), || {})
        .await
        .expect("created");

    assert_eq!(created.name, "-build logs");
    assert_eq!(
        new_session_arguments(Some("  -build logs ")),
        ["session", "new", "--no-select", "--name=-build logs"]
    );
    assert_eq!(commands(&transport)[1], new_line(Some("-build logs")));
    assert_eq!(new_session_arguments(Some("  ")), NEW_SESSION_ARGUMENTS);
}

#[tokio::test]
async fn create_on_a_machine_without_live_sessions_creates_nothing() {
    let transport = FakeTransport::new();
    transport.script(&restore_line(), restore_mode("off"));
    let connection = connected(&transport).await;

    let error = create(&connection, HOST, None, || {}).await.expect_err("not live");

    assert_eq!(
        error,
        format!(
            "{HOST} does not run live sessions (restore mode off), so a session made there could not attach; nothing \
             was created"
        )
    );
    assert_eq!(commands(&transport), [restore_line()], "no session new");
}

#[tokio::test]
async fn create_fails_on_an_old_ngterm_a_missing_ngtermctl_and_a_refusal() {
    let cases = [
        (exits(64, "", "Error: Unknown option\n"), origin_replies::too_old(HOST)),
        (
            exits(127, "", MISSING_CLI),
            format!("{HOST} gave no answer ({}); nothing was created", MISSING_CLI.trim()),
        ),
        (
            refusal("the app is quitting"),
            "the app is quitting; nothing was created".to_owned(),
        ),
    ];
    for (answer, expected) in cases {
        let transport = FakeTransport::new();
        transport.script(&restore_line(), answer);
        let connection = connected(&transport).await;

        assert_eq!(create(&connection, HOST, None, || {}).await.map(|_| ()), Err(expected));
    }
}

#[tokio::test]
async fn create_reports_session_new_failures() {
    let half_done = format!(
        "created session {CREATED} on {HOST} but the connection failed before attaching it; {ATTACH_FROM_LIST}"
    );
    let cases = [
        (
            exits(64, "", "Error: Unknown option '--no-select'\n"),
            origin_replies::too_old(HOST),
        ),
        (
            refusal("no window to create it in"),
            "no window to create it in".to_owned(),
        ),
        (exits(255, &created_receipt().to_string(), "ssh: lost\n"), half_done),
        (
            ok(&json!({"ok": true, "result": {}})),
            format!("{HOST} created a session but did not name it"),
        ),
    ];
    for (answer, expected) in cases {
        let transport = FakeTransport::new();
        transport.script(&restore_line(), restore_mode("live"));
        transport.script(&new_line(None), answer);
        let connection = connected(&transport).await;

        assert_eq!(create(&connection, HOST, None, || {}).await.map(|_| ()), Err(expected));
        assert!(!commands(&transport).contains(&ready_tail()), "nothing is listed");
    }
}

#[tokio::test(start_paused = true)]
async fn create_gives_up_once_the_budget_runs_out() {
    let transport = FakeTransport::new();
    creating(&transport, None);
    transport.script(&ready_tail(), listing(&[session(PARENT, "parent", None, None)]));
    let connection = connected(&transport).await;
    let started = Instant::now();

    let error = create(&connection, HOST, None, || {}).await.expect_err("never listed");

    assert_eq!(
        error,
        format!("created session {CREATED} on {HOST} but could not attach it: {NOT_ATTACHABLE}; {ATTACH_FROM_LIST}")
    );
    assert_eq!(started.elapsed(), READY_BUDGET);
    let listings = commands(&transport)
        .iter()
        .filter(|command| **command == ready_tail())
        .count();
    assert_eq!(listings, 20, "one listing a second within the budget");
}

#[tokio::test(start_paused = true)]
async fn a_failed_listing_is_the_reason_a_give_up_names() {
    let transport = FakeTransport::new();
    creating(&transport, None);
    transport.script(&ready_tail(), refusal("the store is busy"));
    let connection = connected(&transport).await;

    let error = create(&connection, HOST, None, || {}).await.expect_err("never listed");

    assert_eq!(
        error,
        format!("created session {CREATED} on {HOST} but could not attach it: the store is busy; {ATTACH_FROM_LIST}")
    );
}

#[tokio::test(start_paused = true)]
async fn create_stops_waiting_once_the_connection_is_gone() {
    let transport = FakeTransport::new();
    creating(&transport, None);
    let connection = connected(&transport).await;
    let started = Instant::now();
    // the first listing is left open: closing the connection under it ends the wait
    let waiting = create(&connection, HOST, None, || {});
    let closing = async {
        tokio::task::yield_now().await;
        while transport.take_opened().is_empty() {
            tokio::task::yield_now().await;
        }
        connection.close();
    };

    let (error, ()) = tokio::join!(waiting, closing);

    let error = error.expect_err("gone");
    assert!(
        error.starts_with(&format!(
            "created session {CREATED} on {HOST} but could not attach it: "
        )),
        "{error}"
    );
    assert!(error.ends_with(ATTACH_FROM_LIST), "{error}");
    assert!(started.elapsed() < READY_BUDGET);
}

/// A Mac running both apps, created on agterm: the wait lists agterm alone, so ngterm answering the machine meanwhile
/// never hides the session agterm created.
#[tokio::test(start_paused = true)]
async fn a_create_on_agterm_waits_on_agterms_listing_whichever_app_answers_the_machine() {
    use ngterm_core::remote::origin::{OriginForm, OriginShell};
    use ngterm_core::remote::peer::{AGTERM_ANSWER, RemotePeer};
    let transport = FakeTransport::new();
    let agterm = OriginForm {
        shell: OriginShell::Posix,
        peer: RemotePeer::Agterm,
    };
    transport.script(&control_tail(agterm, &RESTORE_MODE_ARGUMENTS), restore_mode("live"));
    transport.script(&control_tail(agterm, &NEW_SESSION_ARGUMENTS), ok(&created_receipt()));
    transport.script(&tree_tail(OriginShell::Posix), listing(&[]));
    let body = json!({"ok": true, "result": {"remote": {
        "endpoint": {"executable": "/Applications/agterm.app/Contents/MacOS/zmx", "socketDirectory": "/tmp/agterm-zmx-1"},
        "sessions": [session(CREATED, "new", None, None)],
    }}});
    let agterm_tree = tree_tail_for(OriginShell::Posix, Some(RemotePeer::Agterm));
    transport.script(&agterm_tree, Script::ok(&format!("{AGTERM_ANSWER}\n{body}")));
    let connection = fresh(&transport).await;
    connection.set_origin_peer(RemotePeer::Agterm);

    let (created, tree) = create(&connection, HOST, None, || {}).await.expect("created on agterm");

    assert_eq!(created.id, CREATED);
    assert_eq!(tree.endpoint.peer(), RemotePeer::Agterm);
    assert_eq!(
        commands(&transport),
        [
            control_tail(agterm, &RESTORE_MODE_ARGUMENTS),
            control_tail(agterm, &NEW_SESSION_ARGUMENTS),
            agterm_tree
        ]
    );
}

// MARK: - Duplicate

fn duplicate_line(source: &str) -> String {
    control_tail(
        ngterm_core::remote::origin::OriginShell::Posix,
        &duplicate_session_arguments(source),
    )
}

#[tokio::test(start_paused = true)]
async fn duplicate_checks_live_duplicates_the_source_and_lists_until_the_copy_is_there() {
    let transport = FakeTransport::new();
    transport.script(&restore_line(), restore_mode("live"));
    transport.script(&duplicate_line(PARENT), ok(&created_receipt()));
    let parent = session(PARENT, "parent", None, None);
    transport.script_sequence(
        &ready_tail(),
        vec![
            listing(std::slice::from_ref(&parent)),
            listing(&[parent.clone(), session(CREATED, "parent 2", None, None)]),
        ],
    );
    let connection = connected(&transport).await;
    let mut listings = 0;

    let (copy, tree) = duplicate(&connection, HOST, (PARENT, None), || listings += 1)
        .await
        .expect("duplicated");

    assert_eq!((copy.id.as_str(), copy.name.as_str()), (CREATED, "parent 2"));
    assert_eq!(tree.sessions.len(), 2);
    assert_eq!(listings, 2);
    assert_eq!(
        commands(&transport),
        [
            restore_line(),
            control_tail(
                ngterm_core::remote::origin::OriginShell::Posix,
                &["session", "duplicate", "--target", PARENT]
            ),
            ready_tail(),
            ready_tail()
        ]
    );
}

#[tokio::test]
async fn duplicate_failures_say_why_and_nothing_is_duplicated_off_live_sessions() {
    let transport = FakeTransport::new();
    transport.script(&restore_line(), restore_mode("off"));
    let connection = connected(&transport).await;
    let error = duplicate(&connection, HOST, (PARENT, None), || {})
        .await
        .expect_err("not live");
    assert!(
        error.starts_with(&format!("{HOST} does not run live sessions")),
        "{error}"
    );
    assert_eq!(commands(&transport), [restore_line()]);

    let transport = FakeTransport::new();
    transport.script(&restore_line(), restore_mode("live"));
    transport.script(&duplicate_line(PARENT), refusal("no session matches"));
    let connection = connected(&transport).await;
    assert_eq!(
        duplicate(&connection, HOST, (PARENT, None), || {}).await.map(|_| ()),
        Err("no session matches".to_owned())
    );
}

#[tokio::test]
async fn duplicate_refuses_an_id_that_is_not_one_token_before_running_anything() {
    let transport = FakeTransport::new();
    let connection = connected(&transport).await;

    assert_eq!(
        duplicate(&connection, HOST, ("a; b", None), || {}).await.map(|_| ()),
        Err(INVALID_REMOTE_SESSION.to_owned())
    );
    assert!(transport.execs().is_empty());
}

// MARK: - End Session

#[tokio::test]
async fn end_closes_the_session_there() {
    let transport = FakeTransport::new();
    transport.script(&end_line(PARENT), ok(&json!({"ok": true, "result": {}})));
    let connection = connected(&transport).await;

    assert_eq!(end(&connection, HOST, PARENT, None).await, Ok(()));
    assert_eq!(commands(&transport), [end_line(PARENT)]);
}

#[tokio::test]
async fn end_failures_say_why() {
    let cases = [
        (refusal("no session matches"), "no session matches".to_owned()),
        (
            exits(255, "", "ssh: lost\n"),
            format!("{HOST} gave no answer (ssh: lost); whether the session ended there is unknown"),
        ),
        (
            exits(127, "", MISSING_CLI),
            format!(
                "{HOST} gave no answer ({}); whether the session ended there is unknown",
                MISSING_CLI.trim()
            ),
        ),
    ];
    for (answer, expected) in cases {
        let transport = FakeTransport::new();
        transport.script(&end_line(PARENT), answer);
        let connection = connected(&transport).await;

        assert_eq!(end(&connection, HOST, PARENT, None).await, Err(expected));
    }
}

#[tokio::test]
async fn end_refuses_an_id_that_is_not_one_token_before_running_anything() {
    let transport = FakeTransport::new();
    let connection = connected(&transport).await;

    assert_eq!(
        end(&connection, HOST, "a b", None).await,
        Err(INVALID_REMOTE_SESSION.to_owned())
    );
    assert_eq!(
        end(&connection, HOST, "", None).await,
        Err(INVALID_REMOTE_SESSION.to_owned())
    );
    assert!(transport.execs().is_empty());
}

#[test]
fn forget_drops_the_session_and_its_companions() {
    let script = listing(&[
        session(PARENT, "parent", None, None),
        session(COMPANION, "companion", Some(PARENT), None),
        session(CREATED, "other", None, None),
    ]);
    let result = RemoteCommandResult {
        status: 0,
        stdout: String::from_utf8(script.stdout).expect("utf-8"),
        stderr: String::new(),
    };
    let mut tree = read_remote_tree(&result, HOST).expect("a listing");

    forget(&mut tree, PARENT);

    let ids: Vec<&str> = tree.sessions.iter().map(|session| session.id.as_str()).collect();
    assert_eq!(ids, [CREATED]);
}

/// A listing of `ids` as `peer` answers it alone.
fn app_tree(peer: RemotePeer, ids: &[&str]) -> ControlRemoteTree {
    let sessions: Vec<serde_json::Value> = ids.iter().map(|id| session(id, id, None, None)).collect();
    let script = listing(&sessions);
    let stdout = String::from_utf8(script.stdout).expect("utf-8");
    let stdout = match peer {
        RemotePeer::Ngterm => stdout,
        RemotePeer::Agterm => format!("{}\n{stdout}", ngterm_core::remote::peer::AGTERM_ANSWER),
    };
    let result = RemoteCommandResult {
        status: 0,
        stdout,
        stderr: String::new(),
    };
    origin_replies::read_peer_tree(&result, HOST).expect("a listing")
}

fn listed_ids(tree: &ControlRemoteTree) -> Vec<(String, Option<crate::peer::PeerKind>)> {
    listed_sessions(tree)
        .into_iter()
        .map(|session| (session.id, session.peer))
        .collect()
}

/// New Session and Duplicate read one app's listing: on a machine that runs both, the other app's sessions stay as
/// last listed, and an ended session leaves whichever app listed it.
#[test]
fn one_apps_listing_keeps_the_other_apps_sessions_of_a_machine_running_both() {
    use crate::peer::PeerKind::{Agterm, Ngterm};

    let both = ControlRemoteTree {
        beside: Some(Box::new(app_tree(RemotePeer::Agterm, &[COMPANION]))),
        ..app_tree(RemotePeer::Ngterm, &[PARENT])
    };
    let own = |id: &str| (id.to_owned(), Some(Ngterm));
    let theirs = |id: &str| (id.to_owned(), Some(Agterm));

    let made_on_own = beside_kept(app_tree(RemotePeer::Ngterm, &[PARENT, CREATED]), Some(&both));
    assert_eq!(listed_ids(&made_on_own), [own(PARENT), own(CREATED), theirs(COMPANION)]);
    let made_on_theirs = beside_kept(app_tree(RemotePeer::Agterm, &[COMPANION, CREATED]), Some(&both));
    assert_eq!(
        listed_ids(&made_on_theirs),
        [own(PARENT), theirs(COMPANION), theirs(CREATED)]
    );

    // a machine that runs one app, or one never listed, keeps the fresh listing as it is
    let alone = app_tree(RemotePeer::Agterm, &[CREATED]);
    assert_eq!(beside_kept(alone.clone(), None), alone);
    assert_eq!(
        beside_kept(alone.clone(), Some(&app_tree(RemotePeer::Ngterm, &[PARENT]))),
        alone
    );
    // a whole listing replaces what was kept
    assert_eq!(beside_kept(both.clone(), Some(&made_on_theirs)), both);

    let mut ended = made_on_theirs;
    forget(&mut ended, COMPANION);
    assert_eq!(listed_ids(&ended), [own(PARENT), theirs(CREATED)]);
}

/// An app that failed or stalled beside the one that answered has not ended its sessions: they stay as last listed,
/// whichever of the two it is, and the app that answered is listed afresh.
#[test]
fn a_listing_that_went_on_without_an_app_keeps_that_apps_sessions_as_last_listed() {
    use crate::peer::PeerKind::{Agterm, Ngterm};

    let both = ControlRemoteTree {
        beside: Some(Box::new(app_tree(RemotePeer::Agterm, &[COMPANION]))),
        ..app_tree(RemotePeer::Ngterm, &[PARENT])
    };
    let without = |peer: RemotePeer, skipped: RemotePeer, ids: &[&str]| ControlRemoteTree {
        skipped: Some(skipped),
        ..app_tree(peer, ids)
    };
    let own = |id: &str| (id.to_owned(), Some(Ngterm));
    let theirs = |id: &str| (id.to_owned(), Some(Agterm));

    let no_agterm = skipped_kept(without(RemotePeer::Ngterm, RemotePeer::Agterm, &[CREATED]), Some(&both));
    assert_eq!(listed_ids(&no_agterm), [own(CREATED), theirs(COMPANION)]);
    assert_eq!(no_agterm.listing(COMPANION).endpoint, both.listing(COMPANION).endpoint);
    let own_kept = skipped_kept(without(RemotePeer::Agterm, RemotePeer::Ngterm, &[CREATED]), Some(&both));
    assert_eq!(listed_ids(&own_kept), [own(PARENT), theirs(CREATED)]);
    assert_eq!(own_kept.endpoint.peer(), RemotePeer::Ngterm);
    // the result is a whole listing: a second pass changes nothing
    assert_eq!(skipped_kept(own_kept.clone(), Some(&both)), own_kept);

    // the app is kept from a listing that had it alone too
    let alone = app_tree(RemotePeer::Agterm, &[COMPANION]);
    let beside_alone = skipped_kept(without(RemotePeer::Ngterm, RemotePeer::Agterm, &[PARENT]), Some(&alone));
    assert_eq!(listed_ids(&beside_alone), [own(PARENT), theirs(COMPANION)]);

    // an app never listed has nothing to keep, and a listing that skipped nothing replaces what was kept
    let never = without(RemotePeer::Ngterm, RemotePeer::Agterm, &[PARENT]);
    assert_eq!(skipped_kept(never.clone(), None), never);
    assert_eq!(
        skipped_kept(never.clone(), Some(&app_tree(RemotePeer::Ngterm, &[CREATED]))),
        never
    );
    let whole = app_tree(RemotePeer::Ngterm, &[CREATED]);
    assert_eq!(skipped_kept(whole.clone(), Some(&both)), whole);
}

/// A machine a listing has told is ngterm's stays ngterm's when one listing goes on without ngterm; a machine no
/// listing has told yet is the app that answered.
#[tokio::test]
async fn a_listing_that_went_on_without_ngterm_does_not_hand_a_known_machine_to_agterm() {
    use ngterm_core::remote::peer::{AGTERM_ANSWER, skipped_line};

    let own = String::from_utf8(listing(&[session(PARENT, "own", None, None)]).stdout).expect("utf-8");
    let theirs = String::from_utf8(listing(&[session(COMPANION, "theirs", None, None)]).stdout).expect("utf-8");
    let no_ngterm = format!("{}\n{AGTERM_ANSWER}\n{theirs}\n", skipped_line(RemotePeer::Ngterm));
    let tail = tree_tail(OriginShell::Posix);

    let transport = FakeTransport::new();
    transport.script(&tail, Script::ok(&format!("{own}\n{AGTERM_ANSWER}\n{theirs}\n")));
    let connection = fresh(&transport).await;
    list(&connection, HOST).await.expect("both listed");
    assert_eq!(connection.origin_form().peer, RemotePeer::Ngterm);
    transport.script(&tail, Script::ok(&no_ngterm));
    let partial = list(&connection, HOST).await.expect("agterm listed");
    assert_eq!(partial.skipped, Some(RemotePeer::Ngterm));
    assert_eq!(connection.origin_form().peer, RemotePeer::Ngterm);

    let transport = FakeTransport::new();
    transport.script(&tail, Script::ok(&no_ngterm));
    let connection = fresh(&transport).await;
    list(&connection, HOST).await.expect("agterm listed");
    assert_eq!(connection.origin_form().peer, RemotePeer::Agterm);
    assert!(connection.learned().peer_known);
}

// MARK: - phone wording

#[test]
fn desktops_cli_advice_becomes_phone_wording() {
    let unattached = origin_replies::unattached(SESSION_NOUN, CREATED, HOST, NOT_ATTACHABLE);
    assert!(unattached.contains("`ngtermctl live attach"), "{unattached}");
    assert_eq!(
        phone_wording(unattached),
        format!("created session {CREATED} on {HOST} but could not attach it: {NOT_ATTACHABLE}; {ATTACH_FROM_LIST}")
    );

    let half_done = RemoteCommandResult {
        status: 255,
        stdout: created_receipt().to_string(),
        stderr: String::new(),
    };
    let desktop = origin_replies::origin_created(&half_done, HOST, SESSION_NOUN).expect_err("half done");
    assert!(desktop.contains("`ngtermctl live attach"), "{desktop}");
    assert!(phone_wording(desktop).ends_with(&format!("before attaching it; {ATTACH_FROM_LIST}")));
}

#[test]
fn other_messages_keep_desktops_words() {
    for message in [
        origin_replies::too_old(HOST),
        "no session matches".to_owned(),
        "it said `ngtermctl live attach` once".to_owned(),
    ] {
        assert_eq!(phone_wording(message.clone()), message);
    }
}

#[test]
fn the_log_names_the_cli_a_listing_asks_and_why_an_answer_gave_nothing() {
    use ngterm_core::remote::tree::NO_ANSWER_IN_TIME;

    assert_eq!(asked_cli(OriginShell::Posix, None), "ngtermctl-and-agtermctl");
    assert_eq!(asked_cli(OriginShell::Posix, Some(RemotePeer::Ngterm)), "ngtermctl");
    assert_eq!(asked_cli(OriginShell::Posix, Some(RemotePeer::Agterm)), "agtermctl");
    assert_eq!(asked_cli(OriginShell::Windows, None), "ngtermctl");
    assert_eq!(shell_class(OriginShell::Windows), "windows");
    assert_eq!(peer_class(RemotePeer::Agterm), "agterm");
    let result = |status: i32, stderr: &str| RemoteCommandResult {
        status,
        stdout: String::new(),
        stderr: stderr.to_owned(),
    };
    assert_eq!(failure_class(&result(0, "")), "unreadable-answer");
    assert_eq!(failure_class(&result(-1, NO_ANSWER_IN_TIME)), "no-answer-in-time");
    assert_eq!(failure_class(&result(-1, "the connection was lost")), "lost");
    assert_eq!(failure_class(&result(127, "command not found")), "exit-status");
}
