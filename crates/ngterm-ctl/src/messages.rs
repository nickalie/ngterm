//! `ngtermctl msg`: pane mailbox request shapes (`agtermctlKit/MessageCommands.swift`).

use std::io::{self, Read as _};

use ngterm_core::paths::Env;
use ngterm_proto::command::MsgCmd;
use ngterm_proto::{Args, Command, Request};

use crate::{BasicOptions, Failure, client, output};

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
struct Identity {
    #[arg(
        long = "as",
        help = "Act as this pane, SESSION:PANE-ID (defaults to NGTERM_SESSION_ID:NGTERM_PANE_ID)."
    )]
    identity: Option<String>,
}

impl Identity {
    fn resolve(&self, env: &dyn Env) -> Result<Option<String>, Failure> {
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
        let reply = client::send(&options.socket_path(env), &self.request(env, input)?)?;
        output::print_response(&reply, options.json, false);
        if reply.response.ok {
            Ok(())
        } else {
            Err(Failure::Exit(std::process::ExitCode::FAILURE))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use ngterm_core::paths::MapEnv;
    use serde_json::json;

    fn parse(args: &[&str]) -> Msg {
        let line = [vec!["ngtermctl", "msg"], args.to_vec()].concat();
        let crate::Command::Msg(command) = crate::Ngtermctl::try_parse_from(line).unwrap().command else {
            unreachable!()
        };
        command
    }

    #[test]
    fn requests_keep_recipient_lists_separate_from_navigation_and_include_identity() {
        let command = parse(&["send", "hello", "--to", "s:left", "--to", "s:right", "--as", "s:p"]);
        assert_eq!(
            serde_json::to_value(command.request(&MapEnv::default(), None).unwrap()).unwrap(),
            json!({"cmd":"msg.send", "args":{"body":"hello","recipients":["s:left","s:right"],"identity":"s:p"}})
        );
        let command = parse(&["read", "--ack", "--as", "s:p"]);
        assert_eq!(
            serde_json::to_value(command.request(&MapEnv::default(), None).unwrap()).unwrap(),
            json!({"cmd":"msg.read", "args":{"ack":true,"identity":"s:p"}})
        );
        let command = parse(&["ack", "m1", "m2"]);
        assert_eq!(
            serde_json::to_value(command.request(&MapEnv::default(), None).unwrap()).unwrap(),
            json!({"cmd":"msg.ack", "args":{"ids":["m1","m2"]}})
        );
    }

    #[test]
    fn checks_body_and_wait_forms_before_connecting() {
        assert_eq!(
            parse(&["send"]).check().unwrap_err(),
            "provide the body as TEXT or --stdin, exactly one"
        );
        assert!(parse(&["send", "hi", "--stdin"]).check().is_err());
        assert_eq!(
            parse(&["read", "--timeout", "1"]).check().unwrap_err(),
            "--timeout needs --wait"
        );
        for timeout in ["0", "-1", "NaN", "inf"] {
            assert_eq!(
                parse(&["read", "--wait", "--timeout", timeout]).check().unwrap_err(),
                "--timeout must be a positive number of seconds"
            );
        }
    }
}
