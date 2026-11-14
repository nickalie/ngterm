//! `ngtermctl agent rerun <agent> -- <hook> [args…]`: what an agent-status hook runs itself through when its agent is
//! a background job of Claude Code's daemon (`CLAUDE_JOB_DIR` set). Such a process has the daemon's environment: its
//! `NGTERM_*` name the pane the daemon was started in, a session long gone or another one's, not the pane the
//! conversation came from. This reads the hook's payload on standard input, asks the instance which pane holds the
//! payload's conversation (`agent.locate`, which follows a conversation Claude Code moved to the background) and runs
//! the hook again with that pane's `NGTERM_SESSION_ID`, `NGTERM_PANE`, `NGTERM_PANE_ID` and `NGTERM_SOCKET`, the
//! payload on its standard input and [`LOCATED`] set, so it does not come back here.
//!
//! The instance asked is `--socket`'s (the daemon's `NGTERM_SOCKET`, which may be a dead instance's or another
//! one's) and then the default one. With no pane to be found (no instance, a payload with no conversation, a job
//! no pane started) the hook does not run and nothing is printed: the exit is 0, as a hook's outside ngterm.
//!
//! On Unix the hook replaces this process, so it stays the agent's own child (`agent mail --wait` ends when its
//! parent goes); elsewhere it runs as a child and its exit status is passed on.

use std::ffi::OsString;
use std::fs::File;
use std::io::{Seek as _, SeekFrom, Write as _};
use std::path::PathBuf;
use std::process::Stdio;

use ngterm_core::agent_chat::AgentKind;
use ngterm_core::paths::Env;
use ngterm_proto::command::AgentCmd;
use ngterm_proto::{AgentReport, Args, Command, ControlAgent, Request, Response};
use serde_json::Value;

use super::{REPORT_DEADLINE, parse_agent, read_payload};
use crate::{BasicOptions, Failure, client, socket};

/// The variable a hook run through here carries: its `NGTERM_*` are its conversation's pane's.
pub const LOCATED: &str = "NGTERM_AGENT_LOCATED";

#[derive(Debug, clap::Args)]
#[command(
    about = "Run an agent hook (payload on stdin) as the pane that holds its conversation.",
    long_about = "Run by the agent-status hooks of a Claude Code background job, whose environment is its daemon's: \
                  reads the hook's JSON payload on standard input, finds the pane whose agent record holds the \
                  payload's conversation (or the one it moved from) and runs HOOK with that pane's NGTERM_SESSION_ID, \
                  NGTERM_PANE, NGTERM_PANE_ID and NGTERM_SOCKET and the payload on its standard input. With no such \
                  pane it runs nothing, prints nothing and exits 0."
)]
pub struct Rerun {
    #[arg(help = "The agent: claude or codex.")]
    agent: String,

    #[arg(last = true, required = true, value_name = "HOOK", help = "The hook's command line.")]
    hook: Vec<OsString>,

    #[command(flatten)]
    options: BasicOptions,
}

/// The pane a conversation was found in.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Located {
    socket: PathBuf,
    session: String,
    pane: String,
    pane_id: String,
}

impl Rerun {
    pub(super) fn validate(&self) -> Result<(), String> {
        parse_agent(&self.agent).map(|_| ())
    }

    pub(super) fn run(&self, env: &dyn Env) -> Result<(), Failure> {
        let agent = parse_agent(&self.agent).map_err(Failure::Usage)?;
        let Ok(payload) = read_payload("agent rerun") else {
            return Ok(());
        };
        let Some(report) = conversation(agent, &payload) else {
            return Ok(());
        };
        let found = self.sockets(env).into_iter().find_map(|socket| {
            let response = client::send_within(&socket, &request(&report), REPORT_DEADLINE)
                .ok()?
                .response;
            located(response, socket)
        });
        match found.and_then(|pane| self.hook_as(&pane, &payload)) {
            Some(hook) => finish(hook),
            None => Ok(()),
        }
    }

    /// The instances asked, in order: `--socket`'s, then the default one.
    fn sockets(&self, env: &dyn Env) -> Vec<PathBuf> {
        let default = socket::resolve(None, env);
        let mut sockets = vec![self.options.socket_path(env)];
        if sockets[0] != default {
            sockets.push(default);
        }
        sockets
    }

    /// The hook as `pane`'s, `payload` on its standard input.
    fn hook_as(&self, pane: &Located, payload: &[u8]) -> Option<std::process::Command> {
        let (program, args) = self.hook.split_first()?;
        let mut hook = std::process::Command::new(program);
        hook.args(args)
            .env("NGTERM_SESSION_ID", &pane.session)
            .env("NGTERM_PANE", &pane.pane)
            .env("NGTERM_PANE_ID", &pane.pane_id)
            .env("NGTERM_SOCKET", &pane.socket)
            .env(LOCATED, "1")
            .stdin(Stdio::from(replayed(payload)?));
        Some(hook)
    }
}

/// `hook` in this process's place: only a hook that could not start comes back, which is no failure of the agent's.
#[cfg(unix)]
#[expect(
    clippy::unnecessary_wraps,
    reason = "the other platforms' form fails with the hook's exit status"
)]
fn finish(mut hook: std::process::Command) -> Result<(), Failure> {
    use std::os::unix::process::CommandExt as _;
    let _ = hook.exec();
    Ok(())
}

/// `hook` to its end, its exit status passed on.
#[cfg(not(unix))]
fn finish(mut hook: std::process::Command) -> Result<(), Failure> {
    use std::process::ExitCode;
    match hook.status().ok().and_then(|status| status.code()) {
        None | Some(0) => Ok(()),
        Some(code) => Err(Failure::Exit(ExitCode::from(u8::try_from(code).unwrap_or(1)))),
    }
}

/// `payload` in a file of its own, to be read from its start: a payload can be larger than a pipe holds.
fn replayed(payload: &[u8]) -> Option<File> {
    let mut file = tempfile::tempfile().ok()?;
    file.write_all(payload).ok()?;
    file.seek(SeekFrom::Start(0)).ok()?;
    Some(file)
}

/// The conversation `payload` names, as `agent.locate` takes it; `None` for a payload that names none.
fn conversation(agent: AgentKind, payload: &[u8]) -> Option<AgentReport> {
    let payload: Value = serde_json::from_slice(payload).ok()?;
    let field = |name: &str| {
        payload
            .get(name)
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    Some(AgentReport {
        agent: agent.name().to_owned(),
        event: "SessionStart".to_owned(),
        transcript_path: Some(field("transcript_path")?),
        conversation_id: Some(field("session_id")?),
        approval: None,
    })
}

/// The `agent.locate` request for `report`'s conversation.
fn request(report: &AgentReport) -> Request {
    Request {
        cmd: Command::Agent(AgentCmd::Locate),
        target: None,
        args: Some(Args {
            report: Some(Box::new(report.clone())),
            ..Args::default()
        }),
    }
}

/// The pane `response` names, at the instance behind `socket`; `None` for a refusal (no pane holds the
/// conversation) or a reply that names no pane.
fn located(response: Response, socket: PathBuf) -> Option<Located> {
    let mut result = response.result.filter(|_| response.ok)?;
    let session = result.get("id")?.as_str()?.to_owned();
    let agent: ControlAgent = serde_json::from_value(result.get_mut("agent")?.take()).ok()?;
    Some(Located {
        socket,
        session,
        pane: agent.pane,
        pane_id: agent.pane_id?,
    })
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::io::Read as _;

    use clap::Parser as _;
    use ngterm_core::paths::MapEnv;
    use serde_json::json;

    use super::*;
    use crate::{Command as Cli, Ngtermctl};

    fn rerun(argv: &[&str]) -> Rerun {
        let argv = ["ngtermctl", "agent", "rerun"].iter().chain(argv).copied();
        let Cli::Agent(super::super::Agent {
            command: super::super::AgentCommand::Rerun(rerun),
        }) = Ngtermctl::try_parse_from(argv).unwrap().command
        else {
            panic!("not agent rerun")
        };
        rerun
    }

    fn pane() -> Located {
        Located {
            socket: PathBuf::from("/run/ngterm.sock"),
            session: "S1".to_owned(),
            pane: "right".to_owned(),
            pane_id: "P1".to_owned(),
        }
    }

    #[test]
    fn a_payload_names_its_conversation_or_none() {
        let named = conversation(
            AgentKind::Claude,
            br#"{"session_id":"aaaa","transcript_path":"/h/.claude/projects/p/aaaa.jsonl","hook_event_name":"Stop"}"#,
        );
        let named = named.expect("a conversation");
        assert_eq!(
            (&named.agent, &named.conversation_id, &named.transcript_path),
            (
                &"claude".to_owned(),
                &Some("aaaa".to_owned()),
                &Some("/h/.claude/projects/p/aaaa.jsonl".to_owned())
            )
        );
        let sent = serde_json::to_value(request(&named)).unwrap();
        assert_eq!(
            sent,
            json!({"cmd": "agent.locate", "args": {"report": {
                "agent": "claude", "event": "SessionStart", "conversationId": "aaaa",
                "transcriptPath": "/h/.claude/projects/p/aaaa.jsonl",
            }}})
        );
        for payload in [
            &br#"{"session_id":"aaaa"}"#[..],
            br#"{"session_id":"","transcript_path":"/t.jsonl"}"#,
            br#"{"session_id":"aaaa","transcript_path":null}"#,
            b"[]",
            b"not json",
        ] {
            assert_eq!(conversation(AgentKind::Claude, payload), None);
        }
    }

    #[test]
    fn a_reply_names_the_pane_and_a_refusal_or_a_reply_without_one_names_none() {
        let socket = || PathBuf::from("/run/ngterm.sock");
        let agent = json!({"agent": "claude", "pane": "right", "transcriptPath": "/t.jsonl", "conversationId": "aaaa",
            "updatedAt": 1.0, "paneID": "P1"});
        let found = Response::with_result(json!({"id": "S1", "agent": agent}));
        assert_eq!(located(found, socket()), Some(pane()));
        assert_eq!(
            located(Response::failure("no pane holds this conversation"), socket()),
            None
        );
        let mut no_identity = agent.clone();
        no_identity.as_object_mut().unwrap().remove("paneID");
        for reply in [
            json!({"agent": agent}),
            json!({"id": "S1"}),
            json!({"id": "S1", "agent": "claude"}),
            json!({"id": "S1", "agent": no_identity}),
        ] {
            assert_eq!(located(Response::with_result(reply), socket()), None);
        }
    }

    #[test]
    fn the_sockets_asked_are_the_given_one_then_the_default() {
        let env = MapEnv::new().with("NGTERM_STATE_DIR", "/state");
        let default = socket::resolve(None, &env);
        assert_eq!(rerun(&["claude", "--", "hook"]).sockets(&env), vec![default.clone()]);
        assert_eq!(
            rerun(&["claude", "--socket", "/daemons.sock", "--", "hook"]).sockets(&env),
            vec![PathBuf::from("/daemons.sock"), default.clone()]
        );
        let same = default.to_str().unwrap();
        assert_eq!(
            rerun(&["claude", "--socket", same, "--", "hook"]).sockets(&env),
            vec![default]
        );
    }

    #[test]
    fn the_hook_runs_with_the_panes_variables_its_own_arguments_and_the_payload_to_read() {
        let command = rerun(&["claude", "--", "bash", "hook.sh", "active", "--blink"]);
        assert_eq!(command.validate(), Ok(()));
        let hook = command.hook_as(&pane(), b"{}").expect("a hook");
        assert_eq!(hook.get_program(), "bash");
        assert_eq!(hook.get_args().collect::<Vec<_>>(), ["hook.sh", "active", "--blink"]);
        let set = |name: &str| {
            hook.get_envs()
                .find(|(key, _)| *key == name)
                .and_then(|(_, value)| value)
                .map(OsStr::to_owned)
        };
        for (name, value) in [
            ("NGTERM_SESSION_ID", "S1"),
            ("NGTERM_PANE", "right"),
            ("NGTERM_PANE_ID", "P1"),
            ("NGTERM_SOCKET", "/run/ngterm.sock"),
            (LOCATED, "1"),
        ] {
            assert_eq!(set(name).as_deref(), Some(OsStr::new(value)), "{name}");
        }
        let mut read = String::new();
        replayed(b"{\"session_id\":\"aaaa\"}")
            .expect("a file")
            .read_to_string(&mut read)
            .unwrap();
        assert_eq!(read, "{\"session_id\":\"aaaa\"}");
        assert_eq!(
            rerun(&["gemini", "--", "hook"]).validate(),
            Err("unknown agent: gemini (claude|codex)".to_owned())
        );
    }
}
