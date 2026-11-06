//! `ngtermctl agent mail <claude|codex> [--wait]` (an ngterm extension): carries the pane's mailbox (`msg`) into the
//! conversation of the coding agent running in it, so another agent's `ngtermctl msg send` reaches it without anyone
//! typing into its composer. The agent-status hooks run it with the hook's payload on standard input: Claude Code's
//! through `ngterm-agent-mail.sh` (or its PowerShell form), Codex's through its lifecycle adapter.
//!
//! Without `--wait` it is a synchronous hook. It takes every message waiting for the pane in one read+ack and answers
//! in the hook protocol both agents share: `PostToolUse` and `UserPromptSubmit` add the mail to the agent's context
//! (`hookSpecificOutput.additionalContext`), and `Stop` keeps the turn going with the mail as its reason
//! (`decision: block`). With no mail, or for any other event, it prints nothing and reads nothing.
//!
//! With `--wait` it is Claude Code's `asyncRewake` hook, started on `SessionStart` and `Stop`: it waits in the
//! background until mail arrives, prints it on standard error and exits 2, which wakes an idle Claude Code with the
//! mail as a system reminder (a busy one gets it when its turn ends). Claude Code starts one per `Stop` without ending
//! the last, so each waiter claims the pane in a token file under the control socket's directory and an older one
//! that finds its claim gone exits 0 without reading. A waiter also exits 0 when its agent is gone (its parent changed:
//! Unix only), and when the pane or the app is (the read keeps failing).
//!
//! Mail reaches only the pane's own interactive agent: a subagent's event (a payload with an `agent_id`) and a Claude
//! Code other than the interactive CLI (`CLAUDE_CODE_ENTRYPOINT` not `cli`: a `claude -p` worker inherits the pane's
//! `NGTERM_*`) take nothing, and the Codex adapter leaves a `codex exec` run out. The read+ack is at most once, as
//! `msg read --ack` is: a message an agent then drops (its hook timed out, it quit) is gone.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ngterm_core::agent_chat::AgentKind;
use ngterm_core::paths::Env;
use ngterm_proto::command::MsgCmd;
use ngterm_proto::{Args, BusMessage, Command, ControlResult, Request};
use serde_json::{Value, json};

use super::{parse_agent, read_payload};
use crate::client::{self, ClientError, Reply};
use crate::messages::{Identity, ack_confirmed};
use crate::{BasicOptions, Failure};

/// The most one read+ack waits, its reply included: the agent waits on a synchronous hook.
const READ_DEADLINE: Duration = Duration::from_secs(5);

/// How often a waiter reads the mailbox, as `msg read --wait` does at a quarter of it.
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// How long a waiter holds its first read: the `Stop` that starts it also posts `completed` through the status
/// adapter, in parallel, and the `active` the wake posts must land after it.
const FIRST_READ_DELAY: Duration = Duration::from_secs(1);

/// Failed reads in a row after which a waiter takes the pane or the app for gone.
const FAILED_READS: u32 = 3;

/// `ngtermctl agent mail <agent>`: the pane's mail into the agent's conversation, as a hook.
#[derive(Debug, clap::Args)]
#[command(
    about = "Hand the pane's mail to its coding agent (run by the agent-status hooks).",
    long_about = "Run by the agent-status hooks with the hook's JSON payload on standard input. Takes the messages \
                  waiting in this pane's mailbox (msg send) and hands them to the agent: as added context on \
                  PostToolUse and UserPromptSubmit, as the reason to keep going on Stop. With --wait (Claude Code's \
                  asyncRewake hook) it waits until mail arrives, prints it on standard error and exits 2, which \
                  wakes an idle Claude Code. Only the pane's interactive agent takes mail: a subagent's event or a \
                  `claude -p` worker takes nothing. Prints nothing when there is no mail."
)]
pub struct Mail {
    #[arg(help = "The agent: claude or codex.")]
    agent: String,

    #[arg(
        long,
        help = "Wait for mail and exit 2 with it on stderr (Claude Code's asyncRewake hook)."
    )]
    wait: bool,

    #[command(flatten)]
    identity: Identity,

    #[command(flatten)]
    options: BasicOptions,
}

impl Mail {
    pub(super) fn validate(&self) -> Result<(), String> {
        let agent = parse_agent(&self.agent)?;
        if self.wait && agent != AgentKind::Claude {
            return Err("--wait is Claude Code's asyncRewake hook: codex has no way to be woken".to_owned());
        }
        Ok(())
    }

    pub(super) fn run(&self, env: &dyn Env) -> Result<(), Failure> {
        let agent = parse_agent(&self.agent).map_err(Failure::Usage)?;
        let payload: Value = serde_json::from_slice(&read_payload()?)
            .map_err(|error| Failure::Local(format!("the hook payload is not JSON: {error}")))?;
        if !takes_mail(agent, &payload, env) {
            return Ok(());
        }
        let Some(identity) = self.identity.resolve(env)? else {
            return Ok(()); // outside a pane: no mailbox to read
        };
        let socket = self.options.socket_path(env);
        let request = read_request(identity.clone());
        if self.wait {
            return wait_live(&socket, request, &identity);
        }
        let Some(event) = payload.get("hook_event_name").and_then(Value::as_str) else {
            return Ok(());
        };
        if Delivery::for_event(event).is_none() {
            return Ok(());
        }
        let messages = taken(&client::send_within(&socket, &request, READ_DEADLINE)?)?;
        if let Some(output) = hook_output(event, &messages) {
            println!("{output}");
        }
        Ok(())
    }
}

/// Whether the agent that fired this hook takes the pane's mail: the interactive agent itself, not a subagent of it
/// nor a headless Claude Code started inside the pane.
fn takes_mail(agent: AgentKind, payload: &Value, env: &dyn Env) -> bool {
    let subagent = payload
        .get("agent_id")
        .and_then(Value::as_str)
        .is_some_and(|id| !id.is_empty());
    let interactive = match agent {
        AgentKind::Claude => env.var("CLAUDE_CODE_ENTRYPOINT").is_some_and(|entry| entry == "cli"),
        AgentKind::Codex => true, // the adapter leaves out `codex exec`, which only its process tells apart
    };
    interactive && !subagent
}

/// How a synchronous hook hands mail over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Delivery {
    /// Added to the agent's context: `PostToolUse`, `UserPromptSubmit`.
    Context,
    /// The reason the turn goes on: `Stop`.
    Block,
}

impl Delivery {
    fn for_event(event: &str) -> Option<Self> {
        match event {
            "PostToolUse" | "UserPromptSubmit" => Some(Self::Context),
            "Stop" => Some(Self::Block),
            _ => None,
        }
    }
}

/// The hook's answer handing `messages` over on `event`, or `None` with nothing to hand over.
fn hook_output(event: &str, messages: &[BusMessage]) -> Option<Value> {
    if messages.is_empty() {
        return None;
    }
    let text = render(messages);
    Some(match Delivery::for_event(event)? {
        Delivery::Context => json!({
            "hookSpecificOutput": {"hookEventName": event, "additionalContext": text},
        }),
        Delivery::Block => json!({"decision": "block", "reason": text}),
    })
}

/// The `msg.read` taking every waiting message off the pane's mailbox (up to the app's batch) in one step.
fn read_request(identity: String) -> Request {
    Request {
        cmd: Command::Msg(MsgCmd::Read),
        target: None,
        args: Some(Args {
            identity: Some(identity),
            ack: Some(true),
            ..Args::default()
        }),
    }
}

/// The messages a read+ack took, or why it took none: a refusal, or a reply that does not confirm what it removed.
fn taken(reply: &Reply) -> Result<Vec<BusMessage>, Failure> {
    let response = crate::events::typed(reply.response.clone())?;
    if !response.ok {
        return Err(Failure::Local(
            response.error.unwrap_or_else(|| "unknown error".to_owned()),
        ));
    }
    let result = response.result.unwrap_or_else(ControlResult::default);
    if !ack_confirmed(&result) {
        return Err(Failure::Local(
            "the app did not confirm which messages it removed".to_owned(),
        ));
    }
    Ok(result.messages.unwrap_or_default())
}

/// The mail as the agent reads it: who sent each message and how to answer, then its body.
fn render(messages: &[BusMessage]) -> String {
    let count = messages.len();
    let mut text = if count == 1 {
        "ngterm mail: a message arrived in this pane's mailbox (sent with `ngtermctl msg send`).".to_owned()
    } else {
        format!("ngterm mail: {count} messages arrived in this pane's mailbox (sent with `ngtermctl msg send`).")
    };
    for (index, message) in messages.iter().enumerate() {
        let sender = message.from.as_deref().map_or_else(
            || "from outside ngterm".to_owned(),
            |from| format!("from {from} (answer with `ngtermctl msg send --to {from} --stdin`)"),
        );
        let _ = write!(
            text,
            "\n\nMessage {} of {count}, {sender}:\n{}",
            index + 1,
            message.body.trim_end()
        );
    }
    text
}

/// What a waiter needs from the world, faked in the tests.
trait Waiting {
    /// One read+ack of the pane's mailbox.
    fn read(&mut self) -> Result<Reply, ClientError>;
    fn sleep(&mut self, delay: Duration);
    /// Whether this waiter still holds the pane and its agent still runs.
    fn current(&mut self) -> bool;
}

/// Waits until a read takes mail, returning it; `None` once the waiter is superseded or its agent, pane or app gone.
fn wait_for_mail(waiting: &mut dyn Waiting) -> Option<Vec<BusMessage>> {
    waiting.sleep(FIRST_READ_DELAY);
    let mut failures = 0;
    loop {
        if !waiting.current() {
            return None;
        }
        match waiting.read().map_err(Failure::from).and_then(|reply| taken(&reply)) {
            Ok(messages) if !messages.is_empty() => return Some(messages),
            Ok(_) => failures = 0,
            Err(_) => {
                failures += 1;
                if failures >= FAILED_READS {
                    return None;
                }
            }
        }
        waiting.sleep(POLL_INTERVAL);
    }
}

/// The live waiter: reads over the control socket, holding the pane's claim while its agent runs.
fn wait_live(socket: &Path, request: Request, identity: &str) -> Result<(), Failure> {
    let claim = Claim::take(socket, identity);
    let mut waiting = LiveWaiting {
        socket: socket.to_owned(),
        request,
        claim: claim.as_ref(),
        parent: parent_pid(),
    };
    let mail = wait_for_mail(&mut waiting);
    if let Some(claim) = &claim {
        claim.release();
    }
    let Some(messages) = mail else {
        return Ok(());
    };
    eprintln!("{}", render(&messages));
    Err(Failure::Exit(ExitCode::from(2)))
}

struct LiveWaiting<'a> {
    socket: PathBuf,
    request: Request,
    claim: Option<&'a Claim>,
    parent: Option<u32>,
}

impl Waiting for LiveWaiting<'_> {
    fn read(&mut self) -> Result<Reply, ClientError> {
        client::send_within(&self.socket, &self.request, READ_DEADLINE)
    }

    fn sleep(&mut self, delay: Duration) {
        std::thread::sleep(delay);
    }

    fn current(&mut self) -> bool {
        self.claim.is_none_or(Claim::held) && parent_pid() == self.parent
    }
}

/// A waiter's claim on its pane: a file under the control socket's directory naming the newest waiter. Without a
/// writable directory a waiter runs unclaimed, and an older one is then superseded only by its agent's exit.
struct Claim {
    path: PathBuf,
    token: String,
}

impl Claim {
    fn take(socket: &Path, identity: &str) -> Option<Self> {
        let directory = socket.parent()?.join("agent-mail");
        fs::create_dir_all(&directory).ok()?;
        let path = directory.join(format!("{}.waiter", file_stem(identity)));
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let token = format!("{}-{nanos}", std::process::id());
        fs::write(&path, &token).ok()?;
        Some(Self { path, token })
    }

    fn held(&self) -> bool {
        fs::read_to_string(&self.path).is_ok_and(|token| token == self.token)
    }

    /// Removes the claim if it is still this waiter's, so the next one starts clean.
    fn release(&self) {
        if self.held() {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// `SESSION:PANE-ID` as a file name: anything but ASCII letters, digits, `-` and `_` becomes `_`.
fn file_stem(identity: &str) -> String {
    identity
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' })
        .collect()
}

/// This process's parent, which changes when the agent that started the hook exits (the hook is re-parented).
#[cfg(unix)]
fn parent_pid() -> Option<u32> {
    // SAFETY: getppid has no preconditions and cannot fail.
    u32::try_from(unsafe { libc::getppid() }).ok()
}

/// Windows has no re-parenting to watch: a waiter there ends with its claim, its pane or its app.
#[cfg(not(unix))]
fn parent_pid() -> Option<u32> {
    None
}

#[cfg(test)]
mod tests;
