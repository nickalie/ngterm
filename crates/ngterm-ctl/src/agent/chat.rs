//! `ngtermctl agent chat`: a pane's agent conversation as NDJSON chat events, for the Android chat view, which runs
//! it on the origin under its ssh. It reads the pane's record (`agent.get`) and follows the transcript the record
//! names itself ([`Follower`]), so the app does no file IO for it.
//!
//! - **Modes:** the default prints the backlog, then `live`, and follows; `--before CURSOR` prints one older page and
//!   exits; `--after CURSOR` prints everything from the cursor's line on, then `live`, and follows (a reconnect).
//!   Following, each poll that read anything prints its events (the main transcript's, then the subagents') and a
//!   `live` after them ([`Follower::poll`]): a caller resumes `--after` the last `live` it got, never a main event's
//!   cursor, so a channel lost within a poll's batch leaves no subagent line behind.
//! - **The cursor's conversation** (`--conversation ID`): a cursor is an offset into one transcript, and the pane may
//!   have moved to another conversation since the caller read it (a `/clear` or a resume while the phone was away).
//!   Given the id the caller's `conversation` events named, a record naming another conversation answers `reset`:
//!   `--after` then prints the new conversation's backlog and `live`, and follows; `--before` prints the `reset` alone.
//!   An empty id (the caller saw none announced) never matches. The record's id is the transcript's own (both are
//!   the agent's session id, and a resume appends to the same file), so a match means the cursor's file; an agent
//!   whose ids differed would only start over at each reconnect, never read a cursor in the wrong file.
//! - **The record** is read again about once a second ([`RECHECK`]): a new transcript (a `/clear`, a resume) is a
//!   `reset` and its backlog; a cleared record (the agent ended) ends the stream with a zero exit; its live permission
//!   prompt becomes `approval` and `approval_done` events ([`Approvals`]).
//! - **The account** the agent is signed in with and its weekly limit are read on a thread of their own
//!   ([`Watch`]) and sent as an `account` event when they change, with the cursor of where the stream stands, as an
//!   approval's: an `--before` page has none.
//! - **Stdin's end** ends the process: the phone's ssh channel went away. A thread reads it to its end, as
//!   `stream_bridge` watches its input, and the follower's wait between polls is the wait on that thread, so the end
//!   is seen within a poll on every OS.

use std::io::{self, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use ngterm_core::agent_chat::approvals::Approvals;
use ngterm_core::agent_chat::follow::{BACKLOG, POLL_INTERVAL};
use ngterm_core::agent_chat::hook::check_transcript;
use ngterm_core::agent_chat::{AgentKind, ChatEvent, EventBody, Follower, NO_AGENT};
use ngterm_core::paths::Env;
use ngterm_proto::ControlAgent;

use super::account::{Source, Watch};
use crate::{Failure, TargetOptions, client, socket};

/// How often the record is read again.
const RECHECK: Duration = Duration::from_secs(1);

/// `ngtermctl agent chat`: the pane's agent conversation, as NDJSON chat events.
#[derive(Debug, clap::Args)]
#[command(
    about = "Print a pane's agent conversation as NDJSON chat events, following it.",
    long_about = "Reads the pane's agent record and prints the conversation its transcript holds as NDJSON chat \
                  events: about the last --limit events, then `live`, then each poll's new events as the agent \
                  writes them, each poll's ended by a `live` whose cursor is where --after resumes. \
                  A permission prompt the agent waits on (the pane's status `blocked`) is an `approval`, and an \
                  `approval_done` once it is answered anywhere. The account the agent is signed in with and its \
                  weekly limit, as far as they can be read, are an `account` event, sent again when they change. A \
                  new conversation in the pane is a `reset` and its backlog; the record clearing ends the stream. \
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

    #[arg(
        long,
        value_name = "ID",
        help = "The conversation the --after or --before cursor belongs to. When the pane's agent is in another one, \
                the answer starts with `reset`: --after then prints the new backlog, --before nothing else."
    )]
    conversation: Option<String>,

    #[command(flatten)]
    target: TargetOptions,

    #[arg(long, help = "Override the control socket path.")]
    socket: Option<PathBuf>,
}

impl Chat {
    pub(super) fn validate(&self) -> Result<(), String> {
        super::validate_pane_side(self.pane.as_deref())?;
        if self.limit == Some(0) {
            return Err("--limit must be at least 1".to_owned());
        }
        if self.conversation.is_some() && self.after.is_none() && self.before.is_none() {
            return Err("--conversation needs --after or --before".to_owned());
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
        // A cursor of another conversation is an offset into another file.
        let moved = self
            .conversation
            .as_ref()
            .is_some_and(|id| *id != record.conversation_id);
        if let Some(before) = self.before {
            let page = if moved {
                vec![reset()]
            } else {
                follower.page(before, limit).map_err(|error| read_failure(&error))?
            };
            let _ = write_events(&page);
            return Ok(());
        }
        let ended = watch_stdin()?;
        follower.expect(record.approval.as_ref());
        let first = match self.after {
            Some(_) if moved => follower.backlog(limit).map(|backlog| [vec![reset()], backlog].concat()),
            Some(after) => follower.after(after),
            None => follower.backlog(limit),
        }
        .map_err(|error| read_failure(&error))?;
        let account = source(&record).and_then(Watch::start);
        let mut stream = Stream {
            chat: self,
            socket,
            limit,
            follower,
            record,
            approvals: Approvals::new(),
            account,
            position: 0,
            announced: false,
        };
        stream.run(first, &ended)
    }

    /// The pane's record, or `None` when it has none.
    fn look(&self, socket: &Path) -> Result<Option<ControlAgent>, Failure> {
        super::look(socket, &self.target.target, self.pane.as_deref())
    }
}

/// A follower of the transcript `record` names, once the path is checked to lie in the agent's tree.
fn follower(record: &ControlAgent) -> Result<Follower, Failure> {
    let agent = AgentKind::from_name(&record.agent)
        .ok_or_else(|| Failure::Local(format!("unknown agent: {}", record.agent)))?;
    check_transcript(agent, &record.transcript_path).map_err(Failure::Local)?;
    Ok(Follower::new(agent, &record.transcript_path))
}

/// What the account watcher reads for `record`: its agent and transcript.
fn source(record: &ControlAgent) -> Option<Source> {
    Some(Source {
        agent: AgentKind::from_name(&record.agent)?,
        transcript: PathBuf::from(&record.transcript_path),
    })
}

/// The event that tells the reader to drop what it shows: another conversation follows.
fn reset() -> ChatEvent {
    ChatEvent {
        id: "reset".to_owned(),
        cursor: 0,
        parent: None,
        body: EventBody::Reset,
    }
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
    /// The account watcher; `None` when its thread couldn't start.
    account: Option<Watch>,
    /// Where the stream stands: the newest cursor of the main transcript's events written, the `account` event's.
    position: u64,
    /// Whether the last `account` event said anything.
    announced: bool,
}

impl Stream<'_> {
    /// Prints `first` and the record's prompt, then follows until stdin ends, the record clears or the reader goes.
    /// The record is read before the transcript it is weighed against, so a prompt's call is in what was read when
    /// the record shows the prompt ([`Approvals`]).
    fn run(&mut self, first: Vec<ChatEvent>, ended: &Receiver<()>) -> Result<(), Failure> {
        let mut events = self.interleave(first);
        events.extend(self.approvals.observe(self.record.approval.as_ref()));
        let mut checked = Instant::now();
        loop {
            self.stand(&mut events);
            if write_events(&events).is_err() {
                return Ok(());
            }
            match ended.recv_timeout(POLL_INTERVAL) {
                Err(RecvTimeoutError::Timeout) => {}
                Ok(()) | Err(RecvTimeoutError::Disconnected) => return Ok(()),
            }
            let looked = if checked.elapsed() >= RECHECK {
                checked = Instant::now();
                Some(self.chat.look(&self.socket)?)
            } else {
                None
            };
            let polled = self.follower.poll().map_err(|error| read_failure(&error))?;
            events = self.interleave(polled);
            match looked {
                Some(Some(record)) => events.extend(self.recheck(record)?),
                Some(None) => {
                    let _ = write_events(&events);
                    return Ok(());
                }
                None => {}
            }
        }
    }

    /// The events the record read again makes: a new transcript's `reset` and backlog, and its prompt's changes.
    fn recheck(&mut self, record: ControlAgent) -> Result<Vec<ChatEvent>, Failure> {
        let mut events = Vec::new();
        let moved = record.agent != self.record.agent || record.transcript_path != self.record.transcript_path;
        if moved {
            self.follower = follower(&record)?;
            if let (Some(watch), Some(source)) = (&self.account, source(&record)) {
                watch.switch(source);
            }
        }
        self.follower.expect(record.approval.as_ref());
        if moved {
            let mut fresh = vec![reset()];
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

    /// Moves [`Stream::position`] past `events`, then adds the `account` event when the watcher read a changed one.
    fn stand(&mut self, events: &mut Vec<ChatEvent>) {
        for event in events.iter().filter(|event| event.parent.is_none()) {
            if matches!(event.body, EventBody::Reset) {
                self.position = 0;
            } else {
                self.position = self.position.max(event.cursor);
            }
        }
        let Some(account) = self.account.as_ref().and_then(Watch::changed) else {
            return;
        };
        // nothing known says nothing, unless it clears what the last event said
        if account.is_empty() && !self.announced {
            return;
        }
        self.announced = !account.is_empty();
        events.push(account.event(self.position));
    }

    /// `events` with the `approval_done` each one makes right after it. The events the follower parsed and did not
    /// show are weighed first, after the `reset` that leads `events` when the read started over.
    fn interleave(&mut self, events: Vec<ChatEvent>) -> Vec<ChatEvent> {
        let mut unshown = Some(self.follower.take_unshown());
        let mut out = Vec::with_capacity(events.len());
        for event in events {
            if !matches!(event.body, EventBody::Reset)
                && let Some(unshown) = unshown.take()
            {
                self.approvals.context(&unshown);
            }
            let done = self.approvals.read(&event);
            out.push(event);
            out.extend(done);
        }
        if let Some(unshown) = unshown {
            self.approvals.context(&unshown);
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
    fn conversation_goes_with_a_cursor() {
        for cursor in ["--after", "--before"] {
            let parsed = chat(&[cursor, "7", "--conversation", "c1"]).unwrap();
            assert_eq!(
                (parsed.conversation.as_deref(), parsed.validate()),
                (Some("c1"), Ok(()))
            );
        }
        let parsed = chat(&["--after", "7", "--conversation", ""]).unwrap();
        assert_eq!(
            parsed.conversation.as_deref(),
            Some(""),
            "an empty id is one that never matches"
        );
        assert_eq!(
            chat(&["--conversation", "c1"]).unwrap().validate(),
            Err("--conversation needs --after or --before".to_owned())
        );
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
