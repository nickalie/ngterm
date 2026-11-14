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
use std::io::{Seek as _, SeekFrom, Write as _};
use std::path::{Path, PathBuf};
use std::process::Stdio;

use ngterm_core::agent_chat::AgentKind;
use ngterm_core::paths::Env;
use ngterm_proto::command::AgentCmd;
use ngterm_proto::{AgentReport, Args, Command, ControlAgent, Request};
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
        let default = socket::resolve(None, env);
        let mut sockets = vec![self.options.socket_path(env)];
        if sockets[0] != default {
            sockets.push(default);
        }
        let Some(located) = sockets.iter().find_map(|socket| locate(socket, &report)) else {
            return Ok(());
        };
        self.hook_as(&located, &payload)
    }

    /// Runs the hook as `located`'s pane, `payload` on its standard input.
    fn hook_as(&self, located: &Located, payload: &[u8]) -> Result<(), Failure> {
        let Some((program, args)) = self.hook.split_first() else {
            return Ok(());
        };
        let Some(input) = replayed(payload) else {
            return Ok(());
        };
        let mut hook = std::process::Command::new(program);
        hook.args(args)
            .env("NGTERM_SESSION_ID", &located.session)
            .env("NGTERM_PANE", &located.pane)
            .env("NGTERM_PANE_ID", &located.pane_id)
            .env("NGTERM_SOCKET", &located.socket)
            .env(LOCATED, "1")
            .stdin(input);
        finish(hook)
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

/// `payload` in a file of its own, read from its start: a payload can be larger than a pipe holds.
fn replayed(payload: &[u8]) -> Option<Stdio> {
    let mut file = tempfile::tempfile().ok()?;
    file.write_all(payload).ok()?;
    file.seek(SeekFrom::Start(0)).ok()?;
    Some(Stdio::from(file))
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

/// The pane the instance at `socket` holds `report`'s conversation in; `None` when it holds none or does not answer.
fn locate(socket: &Path, report: &AgentReport) -> Option<Located> {
    let request = Request {
        cmd: Command::Agent(AgentCmd::Locate),
        target: None,
        args: Some(Args {
            report: Some(Box::new(report.clone())),
            ..Args::default()
        }),
    };
    let response = client::send_within(socket, &request, REPORT_DEADLINE).ok()?.response;
    let mut result = response.result.filter(|_| response.ok)?;
    let session = result.get("id")?.as_str()?.to_owned();
    let agent: ControlAgent = serde_json::from_value(result.get_mut("agent")?.take()).ok()?;
    Some(Located {
        socket: socket.to_path_buf(),
        session,
        pane: agent.pane,
        pane_id: agent.pane_id?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_payload_names_its_conversation_or_none() {
        let named = conversation(
            AgentKind::Claude,
            br#"{"session_id":"aaaa","transcript_path":"/h/.claude/projects/p/aaaa.jsonl","hook_event_name":"Stop"}"#,
        );
        assert_eq!(
            named.map(|report| (report.agent, report.conversation_id, report.transcript_path)),
            Some((
                "claude".to_owned(),
                Some("aaaa".to_owned()),
                Some("/h/.claude/projects/p/aaaa.jsonl".to_owned())
            ))
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
}
