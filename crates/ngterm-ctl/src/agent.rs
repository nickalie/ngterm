//! `ngtermctl agent` (Rust-only, `conformance/docs/rust-build.md` > Rust-only extensions): the coding agent a pane
//! runs, which the origin keeps a record of for the Android chat view (`ngterm-core::commands::agent`).
//!
//! `hook` is what the agent-status hooks run (`resources/agent-status`): it reads the payload the agent writes on a
//! hook's standard input, parses it in Rust ([`ngterm_core::agent_chat::hook::report`]) and sends what it carries as
//! `agent.report`. Nothing large travels through the command line, which matters for Windows' length limit. As a hook
//! it prints nothing on success (an agent adds a hook's output to its context) and fails with exit 1, which no agent
//! reads as a block (exit 2 is Claude Code's and Codex's).
//!
//! `chat` ([`chat`]) is what the Android app runs on the origin under its ssh: the pane's conversation as NDJSON chat
//! events, read from the transcript the record names. `commands` ([`commands`]) is what it runs for the composer's `/`
//! dropdown: the agent's slash commands and skills, once, as JSON.

mod account;
mod chat;
mod commands;

use std::io::{self, IsTerminal, Read};
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use ngterm_core::agent_chat::hook::{HookEvent, report};
use ngterm_core::agent_chat::{AgentKind, NO_AGENT};
use ngterm_core::pane_env::StatusPane;
use ngterm_core::paths::Env;
use ngterm_proto::command::AgentCmd;
use ngterm_proto::{AgentReport, Args, Command, ControlAgent, Request};

use crate::session_io::validate_pane;
use crate::{BasicOptions, Failure, TargetOptions, client, output};

/// The most a hook payload may hold. A permission prompt for a file write carries the file, so this is generous; a
/// payload past it is refused rather than read to the end.
const PAYLOAD_CAP: u64 = 64 * 1024 * 1024;

/// The most a report waits on the control socket, its reply included. The agent waits on its hook (a permission
/// dialog is not shown until it returns), so an instance whose main loop has stalled loses the report instead of
/// holding the agent up; one that is responsive answers in milliseconds. The server's own per-request deadlines are
/// as long.
const REPORT_DEADLINE: Duration = Duration::from_secs(5);

/// The most one `agent.get` waits, its reply included: a stalled app fails the command rather than holding it.
const GET_DEADLINE: Duration = Duration::from_secs(5);

/// `ngtermctl agent <subcommand>`.
#[derive(Debug, clap::Args)]
#[command(
    about = "The coding agent a pane runs, for the Android chat view.",
    long_about = "The instance keeps, in memory, which agent (Claude Code or Codex) each pane runs and where its \
                  conversation's transcript is: the agent-status hooks fill it through `hook` (reinstall them to get \
                  the call, then start a new agent process), and the Android app runs `chat` and `commands` on this \
                  machine over ssh. An ngterm extension: upstream's protocol has no agent group.",
    subcommand_required = true,
    arg_required_else_help = true
)]
pub struct Agent {
    #[command(subcommand)]
    command: AgentCommand,
}

#[derive(Debug, clap::Subcommand)]
enum AgentCommand {
    Hook(Hook),
    Chat(chat::Chat),
    Commands(commands::Commands),
}

impl Agent {
    /// What the parser cannot check, as a usage error's message.
    pub fn check(&self) -> Result<(), String> {
        match &self.command {
            AgentCommand::Hook(command) => command.validate(),
            AgentCommand::Chat(command) => command.validate(),
            AgentCommand::Commands(command) => command.validate(),
        }
    }

    /// Runs the command.
    pub fn execute(&self, env: &dyn Env) -> Result<(), Failure> {
        match &self.command {
            AgentCommand::Hook(command) => command.run(env),
            AgentCommand::Chat(command) => command.run(env),
            AgentCommand::Commands(command) => command.run(env),
        }
    }
}

/// `ngtermctl agent hook <agent> <event>`: one hook's payload, from standard input, to the pane's agent record.
#[derive(Debug, clap::Args)]
#[command(
    about = "Report an agent hook's payload (on stdin) to the pane's agent record.",
    long_about = "Run by the agent-status hooks: reads the JSON payload Claude Code or Codex writes on a hook's \
                  standard input and sends what the pane's agent record keeps (the conversation, its transcript, a \
                  permission prompt). A payload with nothing for the record (a subagent's event, a thread with no \
                  transcript) sends nothing. Prints nothing on success."
)]
pub struct Hook {
    #[arg(help = "The agent: claude or codex.")]
    agent: String,

    #[arg(help = "The hook event: SessionStart, UserPromptSubmit, PermissionRequest or SessionEnd.")]
    event: String,

    #[arg(
        long,
        help = "Which pane the agent runs in: primary/left/top, split/right/bottom, or scratch. Defaults to primary."
    )]
    pane: Option<String>,

    #[arg(
        long = "pane-id",
        help = "The pane's stable token (the shell's $NGTERM_PANE_ID). Overrides --pane when it resolves; falls back \
                to --pane otherwise."
    )]
    pane_id: Option<String>,

    #[command(flatten)]
    target: TargetOptions,

    #[command(flatten)]
    options: BasicOptions,
}

impl Hook {
    fn validate(&self) -> Result<(), String> {
        parse_agent(&self.agent)?;
        HookEvent::parse(&self.event)?;
        validate_pane(self.pane.as_deref())
    }

    fn run(&self, env: &dyn Env) -> Result<(), Failure> {
        let agent = parse_agent(&self.agent).map_err(Failure::Usage)?;
        let event = HookEvent::parse(&self.event).map_err(Failure::Usage)?;
        let payload = read_payload()?;
        let Some(report) = report(agent, event, &payload).map_err(Failure::Local)? else {
            return Ok(());
        };
        let reply = client::send_within(&self.options.socket_path(env), &self.request(report), REPORT_DEADLINE)?;
        if self.options.json || !reply.response.ok {
            output::print_response(&reply, self.options.json, false);
        }
        if reply.response.ok {
            Ok(())
        } else {
            Err(Failure::Exit(ExitCode::FAILURE))
        }
    }

    /// The `agent.report` request carrying `report`, addressed as the status hooks address theirs.
    fn request(&self, report: AgentReport) -> Request {
        Request {
            cmd: Command::Agent(AgentCmd::Report),
            target: Some(self.target.target.clone()),
            args: Some(Args {
                pane: self.pane.clone(),
                pane_id: self.pane_id.clone(),
                report: Some(Box::new(report)),
                ..Args::default()
            }),
        }
    }
}

/// The pane's agent record (`agent.get`), or `None` when it has none. Any other refusal is printed and fails.
fn look(socket: &Path, target: &str, pane: Option<&str>) -> Result<Option<ControlAgent>, Failure> {
    let request = Request {
        cmd: Command::Agent(AgentCmd::Get),
        target: Some(target.to_owned()),
        args: Some(Args {
            pane: pane.map(str::to_owned),
            ..Args::default()
        }),
    };
    let reply = client::send_within(socket, &request, GET_DEADLINE)?;
    if !reply.response.ok {
        if reply.response.error.as_deref() == Some(NO_AGENT) {
            return Ok(None);
        }
        output::print_response(&reply, false, false);
        return Err(Failure::Exit(ExitCode::FAILURE));
    }
    let agent = reply
        .response
        .result
        .and_then(|mut result| result.get_mut("agent").map(serde_json::Value::take))
        .and_then(|agent| serde_json::from_value::<ControlAgent>(agent).ok());
    agent
        .map(Some)
        .ok_or_else(|| Failure::Local("the app's reply has no agent record".to_owned()))
}

/// A `--pane` of the commands that read a record: an agent runs in the left or the right pane.
fn validate_pane_side(pane: Option<&str>) -> Result<(), String> {
    let pane = pane.map(StatusPane::from_control_name);
    if matches!(pane, Some(None | Some(StatusPane::Scratch))) {
        return Err("--pane must be left or right".to_owned());
    }
    Ok(())
}

fn parse_agent(name: &str) -> Result<AgentKind, String> {
    AgentKind::from_name(name).ok_or_else(|| format!("unknown agent: {name} (claude|codex)"))
}

/// The hook payload on standard input, read to its end. A terminal is refused, since nothing would end the read; so is
/// an empty payload, which no agent writes.
fn read_payload() -> Result<Vec<u8>, Failure> {
    let stdin = io::stdin();
    if stdin.is_terminal() {
        return Err(Failure::Usage(
            "agent hook reads the hook payload on standard input".to_owned(),
        ));
    }
    let mut payload = Vec::new();
    stdin
        .lock()
        .take(PAYLOAD_CAP + 1)
        .read_to_end(&mut payload)
        .map_err(|error| Failure::Local(format!("could not read standard input: {}", client::os_message(&error))))?;
    if payload.len() as u64 > PAYLOAD_CAP {
        return Err(Failure::Local(format!(
            "hook payload is over {} MiB",
            PAYLOAD_CAP / 1024 / 1024
        )));
    }
    if payload.iter().all(u8::is_ascii_whitespace) {
        return Err(Failure::Local("no hook payload on standard input".to_owned()));
    }
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use super::*;
    use crate::{Command as Cli, Ngtermctl};

    fn hook(argv: &[&str]) -> Hook {
        let argv = ["ngtermctl", "agent", "hook"].iter().chain(argv).copied();
        let Cli::Agent(Agent {
            command: AgentCommand::Hook(hook),
        }) = Ngtermctl::try_parse_from(argv).unwrap().command
        else {
            panic!("not agent hook")
        };
        hook
    }

    #[test]
    fn the_request_addresses_the_pane_as_the_status_hooks_do() {
        let hook = hook(&[
            "claude",
            "SessionStart",
            "--target",
            "S",
            "--pane",
            "right",
            "--pane-id",
            "P",
            "--socket",
            "/s",
        ]);
        assert_eq!(hook.validate(), Ok(()));
        let report = AgentReport {
            agent: "claude".to_owned(),
            event: "SessionStart".to_owned(),
            ..AgentReport::default()
        };
        assert_eq!(
            serde_json::to_value(hook.request(report)).unwrap(),
            serde_json::json!({
                "cmd": "agent.report",
                "target": "S",
                "args": {"pane": "right", "paneID": "P", "report": {"agent": "claude", "event": "SessionStart"}},
            })
        );
    }

    #[test]
    fn the_target_defaults_to_the_active_session() {
        let hook = hook(&["codex", "UserPromptSubmit"]);
        let request = hook.request(AgentReport::default());
        assert_eq!(request.target.as_deref(), Some("active"));
        let args = request.args.unwrap();
        assert_eq!((args.pane, args.pane_id), (None, None));
    }

    #[test]
    fn an_unknown_agent_event_or_pane_is_a_usage_error() {
        assert_eq!(
            hook(&["gemini", "SessionStart"]).validate(),
            Err("unknown agent: gemini (claude|codex)".to_owned())
        );
        assert_eq!(
            hook(&["claude", "Stop"]).validate(),
            Err("unknown agent event: Stop (SessionStart|UserPromptSubmit|PermissionRequest|SessionEnd)".to_owned())
        );
        assert_eq!(
            hook(&["claude", "SessionStart", "--pane", "middle"]).validate(),
            Err("--pane must be left, right, or scratch".to_owned())
        );
    }
}
