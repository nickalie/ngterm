//! `ngtermctl msg`: pane mailbox request shapes (`agtermctlKit/MessageCommands.swift`).

use std::fmt::Write as _;
use std::io::{self, Read as _};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use ngterm_core::paths::Env;
use ngterm_proto::command::MsgCmd;
use ngterm_proto::{Args, BusMessage, Command, ControlResult, Request};

use crate::client::{ClientError, Reply};
use crate::session_wait::Watchdog;
use crate::{BasicOptions, Failure, client};

#[derive(Debug, clap::Args)]
#[command(
    about = "Send, read and acknowledge messages between panes.",
    subcommand_required = true
)]
pub struct Msg {
    #[command(subcommand)]
    command: MsgCommand,
}

#[derive(Debug, clap::Subcommand)]
enum MsgCommand {
    Send(Send),
    Read(Read),
    Ack(Ack),
}

#[derive(Debug, clap::Args)]
pub(crate) struct Identity {
    #[arg(
        long = "as",
        help = "Act as this pane, SESSION:PANE-ID (defaults to NGTERM_SESSION_ID:NGTERM_PANE_ID)."
    )]
    identity: Option<String>,
}

impl Identity {
    pub(crate) fn resolve(&self, env: &dyn Env) -> Result<Option<String>, Failure> {
        if let Some(identity) = &self.identity {
            return Ok(Some(identity.clone()));
        }
        let session = env.var("NGTERM_SESSION_ID");
        let pane = env.var("NGTERM_PANE_ID");
        match (session, pane) {
            (None, None) => Ok(None),
            (Some(session), Some(pane)) if !session.is_empty() && !pane.is_empty() => {
                Ok(Some(format!("{}:{}", session.to_string_lossy(), pane.to_string_lossy())))
            }
            _ => Err(Failure::Local("incomplete pane identity: NGTERM_SESSION_ID and NGTERM_PANE_ID must both be set; pass --as SESSION:PANE-ID".to_owned())),
        }
    }
}

#[derive(Debug, clap::Args)]
#[command(
    about = "Send a message to one or more panes.",
    long_about = "msg send \"build done\" --to SESSION:right\n\
                  msg send --stdin --all --workspace W < notes.txt\n\
                  \n\
                  A recipient is SESSION:PANE, where PANE is a pane id or left, right or scratch. --all reaches \
                  every live pane in every open window except the sender. Prints the message id. Exit 1 when any \
                  recipient was rejected; the copies that were delivered stay queued."
)]
struct Send {
    #[arg(help = "Message body; use --stdin to read it instead.")]
    text: Option<String>,
    #[arg(long, help = "Read the body from stdin.")]
    stdin: bool,
    #[arg(long = "to", action = clap::ArgAction::Append, help = "Recipient SESSION:PANE; repeat for several.")]
    recipients: Vec<String>,
    #[arg(long, help = "Send to every live pane except the sender.")]
    all: bool,
    #[arg(long, help = "With --all, only the panes of this workspace.")]
    workspace: Option<String>,
    #[command(flatten)]
    identity: Identity,
    #[command(flatten)]
    options: BasicOptions,
}

#[derive(Debug, clap::Args)]
#[command(
    about = "Read the oldest messages in this pane's mailbox.",
    long_about = "msg read [--ack] [--wait [--timeout N]]\n\
                  \n\
                  Prints each message as a line \"ID from SENDER\" followed by its body. Reading does not remove \
                  anything; acknowledge with msg ack, or pass --ack to remove the printed batch in the same step. \
                  --wait repeats the read until a message arrives; --timeout ends it with exit 124."
)]
struct Read {
    #[arg(long, help = "Remove the messages this read returns.")]
    ack: bool,
    #[arg(long, help = "Wait until a message arrives.")]
    wait: bool,
    #[arg(long, allow_hyphen_values = true, help = "With --wait, seconds before exiting 124.")]
    timeout: Option<f64>,
    #[command(flatten)]
    identity: Identity,
    #[command(flatten)]
    options: BasicOptions,
}

#[derive(Debug, clap::Args)]
#[command(
    about = "Remove messages from this pane's mailbox by id.",
    long_about = "Unknown or already removed ids are not an error. Prints how many were removed."
)]
struct Ack {
    #[arg(required = true, help = "Message ids from msg read.")]
    ids: Vec<String>,
    #[command(flatten)]
    identity: Identity,
    #[command(flatten)]
    options: BasicOptions,
}

impl Msg {
    pub fn check(&self) -> Result<(), String> {
        match &self.command {
            MsgCommand::Send(command) if command.text.is_some() == command.stdin => {
                Err("provide the body as TEXT or --stdin, exactly one".to_owned())
            }
            MsgCommand::Read(command) => match command.timeout {
                Some(_) if !command.wait => Err("--timeout needs --wait".to_owned()),
                Some(seconds) if !seconds.is_finite() || seconds <= 0.0 => {
                    Err("--timeout must be a positive number of seconds".to_owned())
                }
                _ => Ok(()),
            },
            _ => Ok(()),
        }
    }

    fn request(&self, env: &dyn Env, input: Option<Vec<u8>>) -> Result<Request, Failure> {
        let (cmd, args) = match &self.command {
            MsgCommand::Send(command) => {
                let body = if command.stdin {
                    String::from_utf8(input.unwrap_or_default())
                        .map_err(|_| Failure::Usage("stdin must be valid UTF-8".to_owned()))?
                } else {
                    command.text.clone().unwrap_or_default()
                };
                (
                    MsgCmd::Send,
                    Args {
                        body: Some(body),
                        recipients: (!command.recipients.is_empty()).then(|| command.recipients.clone()),
                        all: command.all.then_some(true),
                        workspace: command.workspace.clone(),
                        identity: command.identity.resolve(env)?,
                        ..Args::default()
                    },
                )
            }
            MsgCommand::Read(command) => (
                MsgCmd::Read,
                Args {
                    identity: command.identity.resolve(env)?,
                    ack: command.ack.then_some(true),
                    ..Args::default()
                },
            ),
            MsgCommand::Ack(command) => (
                MsgCmd::Ack,
                Args {
                    identity: command.identity.resolve(env)?,
                    ids: Some(command.ids.clone()),
                    ..Args::default()
                },
            ),
        };
        Ok(Request {
            cmd: Command::Msg(cmd),
            target: None,
            args: Some(args),
        })
    }

    pub fn run(&self, env: &dyn Env) -> Result<(), Failure> {
        let options = match &self.command {
            MsgCommand::Send(command) => &command.options,
            MsgCommand::Read(command) => &command.options,
            MsgCommand::Ack(command) => &command.options,
        };
        let input = if matches!(&self.command, MsgCommand::Send(command) if command.stdin) {
            let mut bytes = Vec::new();
            io::stdin()
                .read_to_end(&mut bytes)
                .map_err(|error| Failure::Local(format!("could not read stdin: {}", client::os_message(&error))))?;
            Some(bytes)
        } else {
            None
        };
        let request = self.request(env, input)?;
        let reply = match &self.command {
            MsgCommand::Read(command) => command.read_live(&request, options.socket_path(env))?,
            _ => client::send(&options.socket_path(env), &request)?,
        };
        self.output(&reply)?.emit()
    }

    fn output(&self, reply: &Reply) -> Result<MessageOutput, Failure> {
        let (json, ack) = match &self.command {
            MsgCommand::Send(command) => (command.options.json, false),
            MsgCommand::Read(command) => (command.options.json, command.ack),
            MsgCommand::Ack(command) => (command.options.json, false),
        };
        let response = crate::events::typed(reply.response.clone())?;
        if !response.ok {
            return Ok(if json {
                MessageOutput {
                    stdout: reply.line() + "\n",
                    failed: true,
                    ..MessageOutput::default()
                }
            } else {
                MessageOutput {
                    stderr: format!("error: {}\n", response.error.as_deref().unwrap_or("unknown error")),
                    failed: true,
                    ..MessageOutput::default()
                }
            });
        }
        let empty = ControlResult::default();
        let result = response.result.as_ref().unwrap_or(&empty);
        if ack && !ack_confirmed(result) {
            return Ok(MessageOutput {
                stderr: "error: the app did not confirm which messages it removed; nothing was printed\n".to_owned(),
                failed: true,
                ..MessageOutput::default()
            });
        }
        let rejected = result.rejected.as_deref().unwrap_or_default();
        let failed = matches!(self.command, MsgCommand::Send(_)) && !rejected.is_empty();
        if json {
            return Ok(MessageOutput {
                stdout: reply.line() + "\n",
                failed,
                ..MessageOutput::default()
            });
        }
        Ok(match &self.command {
            MsgCommand::Send(_) => MessageOutput {
                stdout: result.message_id.as_deref().unwrap_or_default().to_owned() + "\n",
                stderr: rejected.iter().fold(String::new(), |mut output, item| {
                    let reason = match item.reason {
                        ngterm_proto::MessageRejectionReason::MailboxFull => "mailbox full",
                        ngterm_proto::MessageRejectionReason::UnknownPane => "unknown pane",
                    };
                    let _ = writeln!(output, "rejected {}: {reason}", item.address);
                    output
                }),
                failed,
            },
            MsgCommand::Read(_) => MessageOutput {
                stdout: result
                    .messages
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .map(render)
                    .collect(),
                stderr: result
                    .remaining
                    .filter(|left| *left > 0)
                    .map(|left| format!("{left} more pending\n"))
                    .unwrap_or_default(),
                failed: false,
            },
            MsgCommand::Ack(_) => MessageOutput {
                stdout: format!("acked {}\n", result.acked.as_deref().unwrap_or_default().len()),
                ..MessageOutput::default()
            },
        })
    }
}

/// The two output streams must be assembled before printing, especially for an unconfirmed read+ack.
#[derive(Debug, Default, PartialEq, Eq)]
struct MessageOutput {
    stdout: String,
    stderr: String,
    failed: bool,
}

impl MessageOutput {
    fn emit(self) -> Result<(), Failure> {
        print!("{}", self.stdout);
        eprint!("{}", self.stderr);
        if self.failed {
            Err(Failure::Exit(ExitCode::FAILURE))
        } else {
            Ok(())
        }
    }
}

pub(crate) fn ack_confirmed(result: &ControlResult) -> bool {
    let ids: Vec<&str> = result
        .messages
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|message| message.id.as_str())
        .collect();
    result
        .acked
        .as_ref()
        .is_some_and(|acked| acked.iter().map(String::as_str).eq(ids))
}

fn render(message: &BusMessage) -> String {
    let newline = if message.body.ends_with('\n') { "" } else { "\n" };
    format!(
        "{} from {}\n{}{newline}",
        message.id,
        message.from.as_deref().unwrap_or("outside ngterm"),
        message.body
    )
}

trait ReadDependencies {
    fn send(&mut self, request: &Request) -> Result<Reply, ClientError>;
    fn sleep(&mut self, delay: Duration);
    fn now(&mut self) -> Duration;
}

struct LiveRead {
    socket: PathBuf,
    start: Instant,
}
impl ReadDependencies for LiveRead {
    fn send(&mut self, request: &Request) -> Result<Reply, ClientError> {
        client::send(&self.socket, request)
    }
    fn sleep(&mut self, delay: Duration) {
        std::thread::sleep(delay);
    }
    fn now(&mut self) -> Duration {
        self.start.elapsed()
    }
}

impl Read {
    fn timeout_duration(&self) -> Option<Duration> {
        self.timeout
            .and_then(|seconds| Duration::try_from_secs_f64(seconds).ok())
    }

    fn poll(&self, request: &Request, dependencies: &mut dyn ReadDependencies) -> Result<Option<Reply>, ClientError> {
        let deadline = self
            .timeout_duration()
            .and_then(|timeout| dependencies.now().checked_add(timeout));
        loop {
            let reply = dependencies.send(request)?;
            let response = crate::events::typed(reply.response.clone())?;
            let empty = ControlResult::default();
            let result = response.result.as_ref().unwrap_or(&empty);
            if !self.wait
                || !response.ok
                || result.messages.as_ref().is_some_and(|messages| !messages.is_empty())
                || (self.ack && !ack_confirmed(result))
            {
                return Ok(Some(reply));
            }
            let mut delay = Duration::from_millis(250);
            if let Some(deadline) = deadline {
                let now = dependencies.now();
                if now >= deadline {
                    return Ok(None);
                }
                delay = delay.min(deadline.saturating_sub(now));
            }
            dependencies.sleep(delay);
        }
    }

    fn read_live(&self, request: &Request, socket: PathBuf) -> Result<Reply, Failure> {
        let watchdog = self.timeout_duration().map(|timeout| {
            Watchdog::start(timeout, || {
                eprintln!("timed out");
                std::process::exit(124);
            })
        });
        let outcome = self.poll(
            request,
            &mut LiveRead {
                socket,
                start: Instant::now(),
            },
        );
        if let Some(watchdog) = watchdog {
            watchdog.cancel();
        }
        if let Some(reply) = outcome? {
            Ok(reply)
        } else {
            eprintln!("timed out");
            Err(Failure::Exit(ExitCode::from(124)))
        }
    }
}

#[cfg(test)]
mod tests;
