use std::collections::VecDeque;

use clap::Parser as _;
use ngterm_core::paths::MapEnv;

use super::*;
use crate::{Command as Cli, Ngtermctl};

fn mail(argv: &[&str]) -> Mail {
    let argv = ["ngtermctl", "agent", "mail"].iter().chain(argv).copied();
    let Cli::Agent(super::super::Agent {
        command: super::super::AgentCommand::Mail(mail),
    }) = Ngtermctl::try_parse_from(argv).unwrap().command
    else {
        panic!("not agent mail")
    };
    mail
}

fn message(id: &str, body: &str, from: Option<&str>) -> BusMessage {
    BusMessage {
        id: id.to_owned(),
        from: from.map(str::to_owned),
        to: "S:P".to_owned(),
        ts: 1.0,
        body: body.to_owned(),
    }
}

/// A read+ack reply: `messages` taken, each acknowledged unless `confirmed` is false.
fn reply(messages: &[BusMessage], confirmed: bool) -> Reply {
    let ids: Vec<&str> = messages.iter().map(|message| message.id.as_str()).collect();
    let mut result = json!({"messages": messages, "remaining": 0});
    if confirmed {
        result["acked"] = json!(ids);
    }
    let response = ngterm_proto::Response {
        ok: true,
        result: Some(result),
        error: None,
    };
    Reply {
        raw: serde_json::to_vec(&response).unwrap(),
        response,
    }
}

fn refusal(error: &str) -> Reply {
    let response = ngterm_proto::Response {
        ok: false,
        result: None,
        error: Some(error.to_owned()),
    };
    Reply {
        raw: serde_json::to_vec(&response).unwrap(),
        response,
    }
}

#[test]
fn wait_is_claude_codes_alone() {
    assert_eq!(mail(&["claude", "--wait"]).validate(), Ok(()));
    assert_eq!(mail(&["codex"]).validate(), Ok(()));
    assert_eq!(
        mail(&["codex", "--wait"]).validate(),
        Err("--wait is Claude Code's asyncRewake hook: codex has no way to be woken".to_owned())
    );
    assert_eq!(
        mail(&["gemini"]).validate(),
        Err("unknown agent: gemini (claude|codex)".to_owned())
    );
}

#[test]
fn only_the_panes_interactive_agent_takes_mail() {
    let cli = MapEnv::default().with("CLAUDE_CODE_ENTRYPOINT", "cli");
    let headless = MapEnv::default().with("CLAUDE_CODE_ENTRYPOINT", "sdk-cli");
    let main = json!({"hook_event_name": "Stop", "session_id": "s"});
    let subagent = json!({"hook_event_name": "PostToolUse", "agent_id": "a1"});
    assert!(takes_mail(AgentKind::Claude, &main, &cli));
    assert!(!takes_mail(AgentKind::Claude, &main, &headless));
    assert!(!takes_mail(AgentKind::Claude, &main, &MapEnv::default()));
    assert!(!takes_mail(AgentKind::Claude, &subagent, &cli));
    let empty_agent = json!({"hook_event_name": "Stop", "agent_id": ""});
    assert!(takes_mail(AgentKind::Claude, &empty_agent, &cli));
}

/// A Codex rollout in `directory` whose first line is `first`, and a payload naming it.
fn codex_payload(directory: &Path, name: &str, first: &str, extra: Value) -> Value {
    let path = directory.join(name);
    fs::write(&path, format!("{first}\n{{\"type\":\"turn_context\"}}\n")).unwrap();
    let mut payload = json!({"hook_event_name": "Stop", "transcript_path": path.to_string_lossy()});
    if let (Value::Object(payload), Value::Object(extra)) = (&mut payload, extra) {
        payload.extend(extra);
    }
    payload
}

fn session_meta(source: &Value) -> String {
    json!({"type": "session_meta", "payload": {"originator": "codex-tui", "source": source}}).to_string()
}

/// The rollout half of Codex's check; the process half is `codex_process`'s, and `takes_mail` refuses a subagent's
/// event before either (a test run under a `codex exec` would otherwise find that run above it).
#[test]
fn only_a_rollout_codexs_tui_created_takes_mail() {
    let directory = tempfile::tempdir().unwrap();
    let tui = codex_payload(directory.path(), "tui.jsonl", &session_meta(&json!("cli")), json!({}));
    assert!(created_by_codexs_tui(&tui));
    for (name, source) in [
        ("exec", json!("exec")),
        ("app-server", json!("vscode")),
        ("mcp", json!("mcp")),
        ("subagent", json!({"subagent": {"thread_spawn": {"depth": 1}}})),
    ] {
        let payload = codex_payload(directory.path(), name, &session_meta(&source), json!({}));
        assert!(!created_by_codexs_tui(&payload), "{name}");
        assert!(!takes_mail(AgentKind::Codex, &payload, &MapEnv::default()), "{name}");
    }
    let subagent = codex_payload(
        directory.path(),
        "sub.jsonl",
        &session_meta(&json!("cli")),
        json!({"agent_id": "a1"}),
    );
    assert!(!takes_mail(AgentKind::Codex, &subagent, &MapEnv::default()));
    let not_meta = codex_payload(
        directory.path(),
        "turn.jsonl",
        r#"{"type":"turn_context","payload":{"source":"cli"}}"#,
        json!({}),
    );
    assert!(!created_by_codexs_tui(&not_meta));
    let garbled = codex_payload(directory.path(), "garbled.jsonl", "{not json", json!({}));
    assert!(!created_by_codexs_tui(&garbled));
    let unfinished = directory.path().join("unfinished.jsonl");
    fs::write(&unfinished, session_meta(&json!("cli"))).unwrap();
    for payload in [
        json!({"hook_event_name": "Stop", "transcript_path": unfinished.to_string_lossy()}),
        json!({"hook_event_name": "Stop", "transcript_path": directory.path().join("missing.jsonl").to_string_lossy()}),
        json!({"hook_event_name": "Stop", "transcript_path": null}),
        json!({"hook_event_name": "Stop"}),
    ] {
        assert!(!created_by_codexs_tui(&payload), "{payload}");
    }
}

#[test]
fn a_wake_turns_the_panes_status_back_to_active() {
    let env = MapEnv::default()
        .with("NGTERM_SESSION_ID", "S")
        .with("NGTERM_PANE", "right")
        .with("NGTERM_PANE_ID", "p7");
    assert_eq!(
        serde_json::to_value(wake_status(&env).unwrap()).unwrap(),
        json!({"cmd": "session.status", "target": "S",
               "args": {"pane": "right", "paneID": "p7", "status": "active", "blink": true}})
    );
    let bare = MapEnv::default().with("NGTERM_SESSION_ID", "S");
    assert_eq!(
        serde_json::to_value(wake_status(&bare).unwrap()).unwrap(),
        json!({"cmd": "session.status", "target": "S", "args": {"status": "active", "blink": true}})
    );
    assert!(wake_status(&MapEnv::default()).is_none());
    assert!(wake_status(&MapEnv::default().with("NGTERM_SESSION_ID", "")).is_none());
}

#[test]
fn the_read_takes_the_panes_mail_in_one_step() {
    assert_eq!(
        serde_json::to_value(read_request("S:P".to_owned())).unwrap(),
        json!({"cmd": "msg.read", "args": {"identity": "S:P", "ack": true}})
    );
}

#[test]
fn each_event_answers_in_its_own_hook_shape() {
    let messages = [message("m1", "rebase on main first\n", Some("D:p1"))];
    let context = hook_output("PostToolUse", &messages).unwrap();
    assert_eq!(context["hookSpecificOutput"]["hookEventName"], "PostToolUse");
    assert_eq!(
        context["hookSpecificOutput"]["additionalContext"],
        json!(render(&messages))
    );
    let prompt = hook_output("UserPromptSubmit", &messages).unwrap();
    assert_eq!(prompt["hookSpecificOutput"]["hookEventName"], "UserPromptSubmit");
    assert_eq!(
        hook_output("Stop", &messages).unwrap(),
        json!({"decision": "block", "reason": render(&messages)})
    );
    assert_eq!(hook_output("Stop", &[]), None);
    assert_eq!(hook_output("SessionStart", &messages), None);
}

#[test]
fn the_mail_names_each_sender_and_how_to_answer() {
    assert_eq!(
        render(&[message("m1", "status?\n", Some("D:p1"))]),
        "ngterm mail: a message arrived in this pane's mailbox (sent with `ngtermctl msg send`).\n\n\
         Message 1 of 1, from D:p1 (answer with `ngtermctl msg send --to D:p1 --stdin`):\nstatus?"
    );
    assert_eq!(
        render(&[message("m1", "one", None), message("m2", "two", Some("D:p1"))]),
        "ngterm mail: 2 messages arrived in this pane's mailbox (sent with `ngtermctl msg send`).\n\n\
         Message 1 of 2, from outside ngterm:\none\n\n\
         Message 2 of 2, from D:p1 (answer with `ngtermctl msg send --to D:p1 --stdin`):\ntwo"
    );
}

#[test]
fn a_read_that_does_not_confirm_its_removal_takes_nothing() {
    let messages = [message("m1", "hi", None)];
    assert_eq!(taken(&reply(&messages, true)).unwrap(), messages);
    assert!(taken(&reply(&messages, false)).is_err());
    assert!(taken(&refusal("unknown pane")).is_err());
    assert_eq!(taken(&reply(&[], true)).unwrap(), []);
}

/// A scripted waiter: replies in order, whether it is still current before each read, and the sleeps it took.
#[derive(Default)]
struct Script {
    replies: VecDeque<Result<Reply, ClientError>>,
    current: VecDeque<bool>,
    sleeps: Vec<Duration>,
    reads: usize,
}

impl Waiting for Script {
    fn read(&mut self) -> Result<Reply, ClientError> {
        self.reads += 1;
        self.replies.pop_front().expect("an unscripted read")
    }

    fn sleep(&mut self, delay: Duration) {
        self.sleeps.push(delay);
    }

    fn current(&mut self) -> bool {
        self.current.pop_front().unwrap_or(true)
    }
}

fn gone() -> ClientError {
    ClientError {
        message: "connection refused".to_owned(),
        connect_error: Some(std::io::ErrorKind::ConnectionRefused),
    }
}

#[test]
fn a_waiter_returns_the_first_mail_after_holding_its_first_read() {
    let messages = vec![message("m1", "go", Some("D:p1"))];
    let mut script = Script {
        replies: VecDeque::from([Ok(reply(&[], true)), Ok(reply(&messages, true))]),
        ..Script::default()
    };
    assert_eq!(wait_for_mail(&mut script), Some(messages));
    assert_eq!(script.sleeps, [FIRST_READ_DELAY, POLL_INTERVAL]);
}

#[test]
fn a_superseded_waiter_stops_without_reading() {
    let mut script = Script {
        replies: VecDeque::from([Ok(reply(&[], true))]),
        current: VecDeque::from([true, false]),
        ..Script::default()
    };
    assert_eq!(wait_for_mail(&mut script), None);
    assert_eq!(script.reads, 1);
}

#[test]
fn a_waiter_rides_out_a_failed_read_but_not_a_gone_app() {
    let messages = vec![message("m1", "go", None)];
    let mut script = Script {
        replies: VecDeque::from([Err(gone()), Ok(refusal("busy")), Ok(reply(&messages, true))]),
        ..Script::default()
    };
    assert_eq!(wait_for_mail(&mut script), Some(messages));

    let mut script = Script {
        replies: VecDeque::from([Err(gone()), Err(gone()), Err(gone())]),
        ..Script::default()
    };
    assert_eq!(wait_for_mail(&mut script), None);
    assert_eq!(script.reads, FAILED_READS as usize);
}

#[test]
fn the_newest_claim_supersedes_the_last_and_only_its_owner_releases_it() {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("ngterm.sock");
    let first = Claim::take(&socket, "S-1:pane/2").unwrap();
    assert!(first.path.ends_with("agent-mail/S-1_pane_2.waiter"));
    assert!(first.held());
    std::thread::sleep(Duration::from_millis(2));
    let second = Claim::take(&socket, "S-1:pane/2").unwrap();
    assert!(!first.held());
    assert!(second.held());
    first.release();
    assert!(
        second.held(),
        "a superseded waiter must not remove its successor's claim"
    );
    second.release();
    assert!(!second.path.exists());
}

#[test]
fn a_claim_taken_between_a_release_check_and_its_removal_survives() {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("ctl.sock");
    let first = Claim::take(&socket, "S:P").unwrap();
    std::thread::scope(|scope| {
        let mut taker = None;
        first.release_checked(|| {
            let (observed, observation) = std::sync::mpsc::sync_channel(0);
            let lock_path = &first.lock;
            let socket = &socket;
            let handle = scope.spawn(move || {
                let lock = fs::OpenOptions::new().read(true).write(true).open(lock_path).unwrap();
                let blocked = matches!(lock.try_lock(), Err(fs::TryLockError::WouldBlock));
                drop(lock);
                observed.send(blocked).unwrap();
                Claim::take(socket, "S:P").unwrap()
            });
            taker = Some(handle);
            // the release cannot continue until the competing attempt has observed its lock
            assert!(
                observation.recv().unwrap(),
                "the claims lock was not held between the ownership check and the removal"
            );
        });
        let second = taker.expect("the release never checked").join().unwrap();
        assert!(
            second.held(),
            "a waiter's release removed the claim a newer waiter took meanwhile"
        );
    });
}

#[test]
fn a_claim_cannot_be_taken_under_a_file_and_a_removed_claim_is_not_held() {
    let directory = tempfile::tempdir().unwrap();
    let blocked = directory.path().join("blocked");
    fs::write(&blocked, b"file").unwrap();
    assert!(Claim::take(&blocked.join("ctl.sock"), "S:P").is_none());
    let claim = Claim::take(&directory.path().join("ctl.sock"), "S:P").unwrap();
    fs::remove_file(&claim.path).unwrap();
    assert!(!claim.held());
    claim.release();
}

#[test]
fn a_live_waiter_stops_when_its_parent_or_claim_changes() {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("ctl.sock");
    let claim = Claim::take(&socket, "S:P").unwrap();
    let mut waiter = LiveWaiting {
        socket,
        request: read_request("S:P".to_owned()),
        claim: Some(&claim),
        parent: parent_pid(),
    };
    assert!(waiter.current());
    fs::write(&claim.path, "another waiter").unwrap();
    assert!(!waiter.current());
    waiter.claim = None;
    assert!(waiter.current());
    #[cfg(unix)]
    {
        waiter.parent = None;
        assert!(!waiter.current());
    }
}

#[cfg(unix)]
#[test]
fn synchronous_mail_reads_and_acks_only_supported_events_for_the_interactive_agent() {
    use crate::messages::tests::run_command;
    let environment = [("CLAUDE_CODE_ENTRYPOINT", "cli")];
    for event in ["Stop", "PostToolUse", "UserPromptSubmit"] {
        let payload = json!({"hook_event_name":event}).to_string();
        let (output, requests) = run_command(
            &["agent", "mail", "claude", "--as", "S:P"],
            payload.as_bytes(),
            &environment,
            &[r#"{"ok":true,"result":{"messages":[{"id":"m","to":"S:P","ts":1,"body":"hello"}],"acked":["m"]}}"#],
        );
        assert_eq!(
            requests,
            [json!({"cmd":"msg.read","args":{"identity":"S:P","ack":true}})]
        );
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.contains("hello"), "{stdout}");
        assert!(
            stdout.contains(if event == "Stop" {
                "\"decision\":\"block\""
            } else {
                "\"additionalContext\":"
            }),
            "{stdout}"
        );
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains("PROBE_RESULT:Ok(())")
        );
    }
    for (payload, env) in [
        (json!({}), environment.to_vec()),
        (json!({"hook_event_name":"SessionStart"}), environment.to_vec()),
        (
            json!({"hook_event_name":"Stop","agent_id":"child"}),
            environment.to_vec(),
        ),
        (
            json!({"hook_event_name":"Stop"}),
            vec![("CLAUDE_CODE_ENTRYPOINT", "sdk-cli")],
        ),
    ] {
        let (output, requests) = run_command(
            &["agent", "mail", "claude", "--as", "S:P"],
            payload.to_string().as_bytes(),
            &env,
            &[],
        );
        assert!(requests.is_empty());
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains("PROBE_RESULT:Ok(())")
        );
    }
}

#[cfg(unix)]
#[test]
fn mail_outside_a_pane_and_empty_mail_are_quiet_and_bad_payloads_fail() {
    use crate::messages::tests::run_command;
    let environment = [("CLAUDE_CODE_ENTRYPOINT", "cli")];
    let (output, _) = run_command(
        &["agent", "mail", "claude"],
        br#"{"hook_event_name":"Stop"}"#,
        &environment,
        &[],
    );
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("PROBE_RESULT:Ok(())")
    );
    let (output, requests) = run_command(
        &["agent", "mail", "claude", "--as", "S:P"],
        br#"{"hook_event_name":"Stop"}"#,
        &environment,
        &[r#"{"ok":true,"result":{"messages":[],"acked":[]}}"#],
    );
    assert_eq!(
        requests,
        [json!({"cmd":"msg.read","args":{"identity":"S:P","ack":true}})]
    );
    // the probe's stdout also carries libtest's own lines: the hook's answer would be a JSON object
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(!stdout.contains("{\""), "{stdout}");
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("PROBE_RESULT:Ok(())")
    );
    let (output, _) = run_command(&["agent", "mail", "claude"], b"not json", &environment, &[]);
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("the hook payload is not JSON:")
    );
    let (output, _) = run_command(&["agent", "mail", "gemini"], b"", &environment, &[]);
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("unknown agent: gemini (claude|codex)")
    );
}

#[cfg(unix)]
#[test]
fn a_waiting_hook_wakes_with_mail_even_if_posting_active_status_is_refused() {
    let (output, requests) = crate::messages::tests::run_command(
        &["agent", "mail", "claude", "--wait", "--as", "S:P"],
        b"{}",
        &[("CLAUDE_CODE_ENTRYPOINT", "cli"), ("NGTERM_SESSION_ID", "S")],
        &[
            r#"{"ok":true,"result":{"messages":[{"id":"m","to":"S:P","ts":1,"body":"wake up"}],"acked":["m"]}}"#,
            r#"{"ok":false,"error":"status unavailable"}"#,
        ],
    );
    assert_eq!(
        requests[1],
        json!({"cmd":"session.status","target":"S","args":{"status":"active","blink":true}})
    );
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(
        error.contains("Message 1 of 1, from outside ngterm:\nwake up\n"),
        "{error}"
    );
    assert!(error.contains("PROBE_RESULT:Err(Exit("), "{error}");
    let expected = format!("PROBE_RESULT:{:?}", Err::<(), _>(Failure::Exit(ExitCode::from(2))));
    assert!(error.contains(&expected), "{error}");
}
