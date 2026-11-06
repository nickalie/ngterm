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
    assert!(takes_mail(AgentKind::Codex, &main, &MapEnv::default()));
    assert!(!takes_mail(AgentKind::Codex, &subagent, &MapEnv::default()));
    let empty_agent = json!({"hook_event_name": "Stop", "agent_id": ""});
    assert!(takes_mail(AgentKind::Codex, &empty_agent, &MapEnv::default()));
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
