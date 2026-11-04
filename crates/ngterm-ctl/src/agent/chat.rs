//! `ngtermctl agent chat`: a pane's agent conversation as NDJSON chat events, for the Android chat view, which runs
//! it on the origin under its ssh. It reads the pane's record (`agent.get`) and follows the transcript the record
//! names itself ([`Follower`]), so the app does no file IO for it.
//!
//! - **Modes:** the default prints the backlog, then `live`, and follows; `--before CURSOR` prints one older page and
//!   exits; `--after CURSOR` prints everything from the cursor's line on, then `live`, and follows (a reconnect).
//! - **The record** is read again about once a second ([`RECHECK`]): a new transcript (a `/clear`, a resume) is a
//!   `reset` and its backlog; a cleared record (the agent ended) ends the stream with a zero exit; its live permission
//!   prompt becomes `approval` and `approval_done` events ([`Approvals`]).
//! - **Stdin's end** ends the process: the phone's ssh channel went away. A thread reads it to its end, as
//!   `stream_bridge` watches its input, and the follower's wait between polls is the wait on that thread, so the end
//!   is seen within a poll on every OS.

use std::io::{self, ErrorKind, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use ngterm_core::agent_chat::approvals::Approvals;
use ngterm_core::agent_chat::follow::{BACKLOG, POLL_INTERVAL};
use ngterm_core::agent_chat::hook::check_transcript;
use ngterm_core::agent_chat::{AgentKind, ChatEvent, EventBody, Follower, NO_AGENT};
use ngterm_core::pane_env::StatusPane;
use ngterm_core::paths::Env;
use ngterm_proto::command::AgentCmd;
use ngterm_proto::{Args, Command, ControlAgent, Request};

use crate::{Failure, TargetOptions, client, output, socket};

/// How often the record is read again.
const RECHECK: Duration = Duration::from_secs(1);

/// The most one `agent.get` waits, its reply included: a stalled app ends the stream rather than holding it.
const GET_DEADLINE: Duration = Duration::from_secs(5);

/// `ngtermctl agent chat`: the pane's agent conversation, as NDJSON chat events.
#[derive(Debug, clap::Args)]
#[command(
    about = "Print a pane's agent conversation as NDJSON chat events, following it.",
    long_about = "Reads the pane's agent record and prints the conversation its transcript holds as NDJSON chat \
                  events: about the last --limit events, then `live`, then each new event as the agent writes it. \
                  A new conversation in the pane is a `reset` and its backlog; the record clearing ends the stream. \
                  Ends when standard input reaches its end."
)]
pub struct Chat {
    #[arg(long, help = "Which pane the agent runs in: left or right. Defaults to left.")]
    pane: Option<String>,

    #[arg(
        long,
        value_name = "CURSOR",
        conflicts_with = "after",
        help = "Print the page of events before this cursor and exit; nothing when the start was reached."
    )]
    before: Option<u64>,

    #[arg(
        long,
        value_name = "CURSOR",
        help = "Print the events from the line at this cursor on, then follow (a reconnect)."
    )]
    after: Option<u64>,

    #[arg(
        long,
        conflicts_with = "after",
        help = "About how many events the backlog or the page holds. Defaults to 200."
    )]
    limit: Option<usize>,

    #[command(flatten)]
    target: TargetOptions,

    #[arg(long, help = "Override the control socket path.")]
    socket: Option<PathBuf>,
}

impl Chat {
    pub(super) fn validate(&self) -> Result<(), String> {
        let pane = self.pane.as_deref().map(StatusPane::from_control_name);
        if matches!(pane, Some(None | Some(StatusPane::Scratch))) {
            return Err("--pane must be left or right".to_owned());
        }
        if self.limit == Some(0) {
            return Err("--limit must be at least 1".to_owned());
        }
        Ok(())
    }

    pub(super) fn run(&self, env: &dyn Env) -> Result<(), Failure> {
        let socket = socket::resolve(self.socket.as_deref(), env);
        let Some(record) = self.look(&socket)? else {
            return Err(Failure::Local(NO_AGENT.to_owned()));
        };
        let limit = self.limit.unwrap_or(BACKLOG);
        let mut follower = follower(&record)?;
        if let Some(before) = self.before {
            let page = follower.page(before, limit).map_err(|error| read_failure(&error))?;
            let _ = write_events(&page);
            return Ok(());
        }
        let ended = watch_stdin()?;
        let first = match self.after {
            Some(after) => follower.after(after),
            None => follower.backlog(limit),
        }
        .map_err(|error| read_failure(&error))?;
        let mut stream = Stream {
            chat: self,
            socket,
            limit,
            follower,
            record,
            approvals: Approvals::new(),
        };
        stream.run(first, &ended)
    }

    /// The pane's record, or `None` when it has none. Any other refusal is printed and fails.
    fn look(&self, socket: &std::path::Path) -> Result<Option<ControlAgent>, Failure> {
        let request = Request {
            cmd: Command::Agent(AgentCmd::Get),
            target: Some(self.target.target.clone()),
            args: Some(Args {
                pane: self.pane.clone(),
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
}

/// A follower of the transcript `record` names, once the path is checked to lie in the agent's tree.
fn follower(record: &ControlAgent) -> Result<Follower, Failure> {
    let agent = AgentKind::from_name(&record.agent)
        .ok_or_else(|| Failure::Local(format!("unknown agent: {}", record.agent)))?;
    check_transcript(agent, &record.transcript_path).map_err(Failure::Local)?;
    Ok(Follower::new(agent, &record.transcript_path))
}

fn read_failure(error: &io::Error) -> Failure {
    Failure::Local(format!("could not read the transcript: {}", client::os_message(error)))
}

/// A thread reading stdin to its end; the receiver hears of the end (or of the thread's).
fn watch_stdin() -> Result<Receiver<()>, Failure> {
    let (ended, receiver) = mpsc::channel();
    thread::Builder::new()
        .name("ngtermctl.agent.stdin".to_owned())
        .spawn(move || {
            let mut stdin = io::stdin();
            let mut chunk = [0_u8; 4096];
            loop {
                match stdin.read(&mut chunk) {
                    Ok(0) => break,
                    Err(error) if error.kind() != ErrorKind::Interrupted => break,
                    _ => {}
                }
            }
            let _ = ended.send(());
        })
        .map_err(|error| Failure::Local(format!("cannot watch standard input: {}", client::os_message(&error))))?;
    Ok(receiver)
}

/// Writes `events` to standard output, one per line, and flushes them. An error means the reader went away.
fn write_events(events: &[ChatEvent]) -> io::Result<()> {
    if events.is_empty() {
        return Ok(());
    }
    let mut stdout = io::stdout().lock();
    for event in events {
        stdout.write_all(event.to_ndjson().as_bytes())?;
        stdout.write_all(b"\n")?;
    }
    stdout.flush()
}

/// A followed conversation and the record it came from.
struct Stream<'a> {
    chat: &'a Chat,
    socket: PathBuf,
    limit: usize,
    follower: Follower,
    record: ControlAgent,
    approvals: Approvals,
}

impl Stream<'_> {
    /// Prints `first` and the record's prompt, then follows until stdin ends, the record clears or the reader goes.
    fn run(&mut self, first: Vec<ChatEvent>, ended: &Receiver<()>) -> Result<(), Failure> {
        let mut events = self.interleave(first);
        events.extend(self.approvals.observe(self.record.approval.as_ref()));
        let mut checked = Instant::now();
        loop {
            if write_events(&events).is_err() {
                return Ok(());
            }
            match ended.recv_timeout(POLL_INTERVAL) {
                Err(RecvTimeoutError::Timeout) => {}
                Ok(()) | Err(RecvTimeoutError::Disconnected) => return Ok(()),
            }
            let polled = self.follower.poll().map_err(|error| read_failure(&error))?;
            events = self.interleave(polled);
            if checked.elapsed() >= RECHECK {
                checked = Instant::now();
                let Some(record) = self.chat.look(&self.socket)? else {
                    let _ = write_events(&events);
                    return Ok(());
                };
                events.extend(self.recheck(record)?);
            }
        }
    }

    /// The events the record read again makes: a new transcript's `reset` and backlog, and its prompt's changes.
    fn recheck(&mut self, record: ControlAgent) -> Result<Vec<ChatEvent>, Failure> {
        let mut events = Vec::new();
        if record.agent != self.record.agent || record.transcript_path != self.record.transcript_path {
            self.follower = follower(&record)?;
            let mut fresh = vec![ChatEvent {
                id: "reset".to_owned(),
                cursor: 0,
                parent: None,
                body: EventBody::Reset,
            }];
            fresh.extend(
                self.follower
                    .backlog(self.limit)
                    .map_err(|error| read_failure(&error))?,
            );
            events = self.interleave(fresh);
        }
        events.extend(self.approvals.observe(record.approval.as_ref()));
        self.record = record;
        Ok(events)
    }

    /// `events` with the `approval_done` each one makes right after it.
    fn interleave(&mut self, events: Vec<ChatEvent>) -> Vec<ChatEvent> {
        let mut out = Vec::with_capacity(events.len());
        for event in events {
            let done = self.approvals.read(&event);
            out.push(event);
            out.extend(done);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use super::*;
    use crate::agent::{Agent, AgentCommand};
    use crate::{Command as Cli, Ngtermctl};

    fn chat(argv: &[&str]) -> Result<Chat, clap::Error> {
        let argv = ["ngtermctl", "agent", "chat"].iter().chain(argv).copied();
        match Ngtermctl::try_parse_from(argv)?.command {
            Cli::Agent(Agent {
                command: AgentCommand::Chat(chat),
            }) => Ok(chat),
            _ => panic!("not agent chat"),
        }
    }

    #[test]
    fn the_modes_and_their_options_parse() {
        let parsed = chat(&["--target", "S", "--pane", "right", "--before", "120", "--limit", "5"]).unwrap();
        assert_eq!(
            (
                parsed.target.target.as_str(),
                parsed.pane.as_deref(),
                parsed.before,
                parsed.after,
                parsed.limit
            ),
            ("S", Some("right"), Some(120), None, Some(5))
        );
        assert_eq!(parsed.validate(), Ok(()));
        let parsed = chat(&["--after", "7"]).unwrap();
        assert_eq!((parsed.target.target.as_str(), parsed.after), ("active", Some(7)));
    }

    #[test]
    fn before_and_after_or_after_and_limit_conflict() {
        assert!(chat(&["--before", "1", "--after", "2"]).is_err());
        assert!(chat(&["--after", "2", "--limit", "5"]).is_err());
        assert!(chat(&["--before", "-1"]).is_err());
    }

    #[test]
    fn a_pane_other_than_left_or_right_or_a_zero_limit_is_a_usage_error() {
        let refused = |argv: &[&str]| chat(argv).unwrap().validate();
        assert_eq!(
            refused(&["--pane", "scratch"]),
            Err("--pane must be left or right".to_owned())
        );
        assert_eq!(
            refused(&["--pane", "middle"]),
            Err("--pane must be left or right".to_owned())
        );
        assert_eq!(refused(&["--limit", "0"]), Err("--limit must be at least 1".to_owned()));
        assert_eq!(refused(&["--pane", "split"]), Ok(()));
    }
}
