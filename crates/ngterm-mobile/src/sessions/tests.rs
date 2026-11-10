use ngterm_core::remote::origin_replies::{
    self, NEW_SESSION_ARGUMENTS, NOT_ATTACHABLE, PAUSE, READY_BUDGET, RESTORE_MODE_ARGUMENTS, SESSION_NOUN,
    end_session_arguments,
};
use ngterm_core::remote::ssh::{control_tail, tree_tail};
use ngterm_core::remote::tree::{RemoteCommandResult, TREE_DEADLINE};
use serde_json::json;
use tokio::time::Instant;

use super::*;
use crate::api::Machine;
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

async fn connected(transport: &FakeTransport) -> FakeConnection {
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
    assert_eq!(companion.pane_count, 1);
    assert_eq!(
        commands(&transport),
        [tree_tail(ngterm_core::remote::origin::OriginShell::Posix)]
    );
    assert_eq!(transport.execs()[0].pty, None);
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

// MARK: - New Session

#[tokio::test(start_paused = true)]
async fn create_checks_live_creates_and_lists_until_the_session_is_there() {
    let transport = FakeTransport::new();
    creating(&transport, None);
    let parent = session(PARENT, "parent", None, None);
    transport.script_sequence(
        &tree_tail(ngterm_core::remote::origin::OriginShell::Posix),
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
            tree_tail(ngterm_core::remote::origin::OriginShell::Posix),
            tree_tail(ngterm_core::remote::origin::OriginShell::Posix)
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn create_with_a_name_passes_it_as_one_value() {
    let transport = FakeTransport::new();
    creating(&transport, Some("-build logs"));
    transport.script(
        &tree_tail(ngterm_core::remote::origin::OriginShell::Posix),
        listing(&[session(CREATED, "-build logs", None, None)]),
    );
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
        assert!(
            !commands(&transport).contains(&tree_tail(ngterm_core::remote::origin::OriginShell::Posix)),
            "nothing is listed"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn create_gives_up_once_the_budget_runs_out() {
    let transport = FakeTransport::new();
    creating(&transport, None);
    transport.script(
        &tree_tail(ngterm_core::remote::origin::OriginShell::Posix),
        listing(&[session(PARENT, "parent", None, None)]),
    );
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
        .filter(|command| **command == tree_tail(ngterm_core::remote::origin::OriginShell::Posix))
        .count();
    assert_eq!(listings, 20, "one listing a second within the budget");
}

#[tokio::test(start_paused = true)]
async fn a_failed_listing_is_the_reason_a_give_up_names() {
    let transport = FakeTransport::new();
    creating(&transport, None);
    transport.script(
        &tree_tail(ngterm_core::remote::origin::OriginShell::Posix),
        refusal("the store is busy"),
    );
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

// MARK: - End Session

#[tokio::test]
async fn end_closes_the_session_there() {
    let transport = FakeTransport::new();
    transport.script(&end_line(PARENT), ok(&json!({"ok": true, "result": {}})));
    let connection = connected(&transport).await;

    assert_eq!(end(&connection, HOST, PARENT).await, Ok(()));
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

        assert_eq!(end(&connection, HOST, PARENT).await, Err(expected));
    }
}

#[tokio::test]
async fn end_refuses_an_id_that_is_not_one_token_before_running_anything() {
    let transport = FakeTransport::new();
    let connection = connected(&transport).await;

    assert_eq!(
        end(&connection, HOST, "a b").await,
        Err(INVALID_REMOTE_SESSION.to_owned())
    );
    assert_eq!(end(&connection, HOST, "").await, Err(INVALID_REMOTE_SESSION.to_owned()));
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
