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
//!   approval's: an `--before` page has none. A fresh stream says nothing while nothing is known; a reconnect
//!   (`--after`) sends its first reading even then, as the caller may still show the account an earlier stream sent
//!   (signed out meanwhile, it is cleared).
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
            // a reconnecting caller may hold an account from the stream before: its first reading is sent, even empty
            announced: self.after.is_some(),
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
    /// Whether the last `account` event said anything; before the first, whether the caller may hold one (a
    /// reconnect).
    announced: bool,
}

impl Stream<'_> {
    /// Prints `first` and the record's prompt, then follows until stdin ends, the record clears or the reader goes.
    /// The record is read before the transcript it is weighed against, so a prompt's call is in what was read when
    /// the record shows the prompt ([`Approvals`]). The prompt goes before the catch-up's `live`: a reader resuming
    /// takes the prompts its catch-up names as the ones still open, and ends any other it showed.
    fn run(&mut self, first: Vec<ChatEvent>, ended: &Receiver<()>) -> Result<(), Failure> {
        let mut events = self.interleave(first);
        let live = events
            .iter()
            .rposition(|event| matches!(event.body, EventBody::Live))
            .unwrap_or(events.len());
        let prompt = self.approvals.observe(self.record.approval.as_ref());
        events.splice(live..live, prompt);
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
        // the phone's form: each value one word with its option, so one that starts with `-` stays a value
        let parsed = chat(&["--target=-s", "--after", "7", "--conversation=-c"]).unwrap();
        assert_eq!(
            (parsed.target.target.as_str(), parsed.conversation.as_deref()),
            ("-s", Some("-c"))
        );
        let parsed = chat(&["--after", "7", "--conversation="]).unwrap();
        assert_eq!(parsed.conversation.as_deref(), Some(""));
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

    fn record(path: &Path) -> ControlAgent {
        ControlAgent {
            agent: "claude".to_owned(),
            pane: "left".to_owned(),
            transcript_path: path.to_string_lossy().into_owned(),
            conversation_id: "c1".to_owned(),
            updated_at: 1.0,
            approval: None,
            pane_id: None,
        }
    }

    fn stream(chat: &Chat, record: ControlAgent) -> Stream<'_> {
        Stream {
            chat,
            socket: PathBuf::new(),
            limit: 10,
            follower: follower(&record).unwrap(),
            record,
            approvals: Approvals::new(),
            account: None,
            position: 0,
            announced: false,
        }
    }

    fn transcript(directory: &Path, name: &str) -> PathBuf {
        let path = directory.join("projects/work").join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            b"{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"content\":\"hello\"}}\n",
        )
        .unwrap();
        path
    }

    #[test]
    fn followers_refuse_unknown_agents_and_paths_outside_the_agents_tree() {
        let mut record = record(Path::new("/tmp/outside.jsonl"));
        let Err(Failure::Local(error)) = follower(&record) else {
            panic!("accepted outside path")
        };
        assert_eq!(
            error,
            "transcript path is outside a Claude projects/ tree: /tmp/outside.jsonl"
        );
        record.agent = "other".to_owned();
        let Err(Failure::Local(error)) = follower(&record) else {
            panic!("accepted unknown agent")
        };
        assert_eq!(error, "unknown agent: other");
        assert!(source(&record).is_none());
        let Failure::Local(error) = read_failure(&io::Error::other("reader refused")) else {
            unreachable!()
        };
        assert_eq!(error, "could not read the transcript: reader refused");
    }

    #[test]
    fn a_recheck_resets_only_when_the_transcript_moves_and_keeps_the_new_record() {
        let directory = tempfile::tempdir().unwrap();
        let old = record(&transcript(directory.path(), "old.jsonl"));
        let chat = chat(&[]).unwrap();
        let mut stream = stream(&chat, old.clone());
        assert!(stream.recheck(old).unwrap().is_empty());
        let new = record(&transcript(directory.path(), "new.jsonl"));
        let events = stream.recheck(new.clone()).unwrap();
        assert_eq!(events.first(), Some(&reset()));
        assert!(
            events.iter().any(|event| event.to_ndjson().contains("hello")),
            "{events:?}"
        );
        assert_eq!(stream.record, new);
        let mut invalid = new;
        invalid.agent = "unknown".to_owned();
        assert!(matches!(stream.recheck(invalid), Err(Failure::Local(error)) if error == "unknown agent: unknown"));
    }

    #[test]
    fn stream_positions_ignore_subagents_and_reset_before_a_new_conversation() {
        let directory = tempfile::tempdir().unwrap();
        let chat = chat(&[]).unwrap();
        let mut stream = stream(&chat, record(&transcript(directory.path(), "c.jsonl")));
        let mut main = reset();
        main.body = EventBody::Live;
        main.cursor = 20;
        let mut subagent = main.clone();
        subagent.parent = Some("task".to_owned());
        subagent.cursor = 999;
        stream.stand(&mut vec![main.clone(), subagent]);
        assert_eq!(stream.position, 20);
        main.cursor = 3;
        stream.stand(&mut vec![reset(), main]);
        assert_eq!(stream.position, 3);
    }

    #[test]
    fn a_changed_account_is_announced_at_the_streams_current_position() {
        let directory = tempfile::tempdir().unwrap();
        let record = record(&transcript(directory.path(), "c.jsonl"));
        std::fs::write(
            directory.path().join(".claude.json"),
            br#"{"oauthAccount":{"emailAddress":"a@example.com"}}"#,
        )
        .unwrap();
        let chat = chat(&[]).unwrap();
        let mut stream = stream(&chat, record.clone());
        stream.account = source(&record).and_then(Watch::start);
        stream.position = 42;
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut events = Vec::new();
        while events.is_empty() {
            assert!(Instant::now() < deadline, "account was not announced");
            stream.stand(&mut events);
            thread::yield_now();
        }
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].cursor, 42);
        assert!(events[0].to_ndjson().contains("a@example.com"));
        assert!(stream.announced);
        let next = record.clone();
        assert!(stream.recheck(next).unwrap().is_empty());
        let moved = self::record(&transcript(directory.path(), "next.jsonl"));
        assert_eq!(stream.recheck(moved).unwrap().first(), Some(&reset()));
    }

    /// The NDJSON events in a probe's stdout, which also carries libtest's own lines (the first event shares the
    /// probe's `test ... ` line).
    #[cfg(unix)]
    fn probe_events(stdout: &str) -> Vec<&str> {
        stdout
            .lines()
            .filter_map(|line| line.find("{\"").map(|at| &line[at..]))
            .collect()
    }

    #[cfg(unix)]
    #[test]
    fn an_empty_directory_is_not_a_transcript() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("projects/work/empty.jsonl");
        std::fs::create_dir_all(&path).unwrap();
        let agent = record(&path);
        let reply = serde_json::json!({"ok": true, "result": {"agent": agent}}).to_string();
        let (output, _) =
            crate::messages::tests::run_command(&["agent", "chat", "--before", "10"], b"", &[], &[&reply]);
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(
            error.contains(
                r#"PROBE_RESULT:Err(Local("could not read the transcript: transcript is not a regular file"))"#
            ),
            "{error}"
        );
        assert!(probe_events(&String::from_utf8(output.stdout).unwrap()).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn chat_pages_reset_mismatched_conversations_and_following_ends_with_stdin() {
        use crate::messages::tests::run_command;
        let directory = tempfile::tempdir().unwrap();
        let record = record(&transcript(directory.path(), "c.jsonl"));
        let reply = serde_json::json!({"ok":true,"result":{"agent":record}}).to_string();
        for arguments in [
            vec!["agent", "chat", "--before", "10000"],
            vec!["agent", "chat", "--before", "10000", "--conversation", "old"],
            vec!["agent", "chat"],
            vec!["agent", "chat", "--after", "0"],
            vec!["agent", "chat", "--after", "0", "--conversation", "old"],
        ] {
            let (output, requests) = run_command(&arguments, b"", &[], &[&reply]);
            assert_eq!(requests[0]["cmd"], "agent.get");
            assert!(
                String::from_utf8(output.stderr)
                    .unwrap()
                    .contains("PROBE_RESULT:Ok(())")
            );
            let stdout = String::from_utf8(output.stdout).unwrap();
            let events = probe_events(&stdout);
            let reset = reset().to_ndjson();
            match (arguments.contains(&"old"), arguments.contains(&"--before")) {
                (true, true) => assert_eq!(events, [reset.as_str()], "{stdout}"),
                (true, false) => {
                    assert_eq!(events.first(), Some(&reset.as_str()), "{stdout}");
                    assert!(events[1..].iter().any(|event| event.contains("hello")), "{stdout}");
                }
                (false, _) => {
                    assert!(!events.contains(&reset.as_str()), "{stdout}");
                    assert!(events.iter().any(|event| event.contains("hello")), "{stdout}");
                }
            }
        }
        let (output, _) = run_command(
            &["agent", "chat"],
            b"",
            &[],
            &[r#"{"ok":false,"error":"no agent in this pane"}"#],
        );
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(
            stderr.contains(r#"PROBE_RESULT:Err(Local("no agent in this pane"))"#),
            "{stderr}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_stream_rechecks_the_pane_and_ends_when_its_agent_is_gone() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "agent::chat::tests::stream_probe",
                "--exact",
                "--nocapture",
                "--test-threads=1",
            ])
            .env_clear()
            .envs(std::env::vars_os().filter(|(key, _)| !key.to_string_lossy().starts_with("NGTERM_")))
            .env("CTL_STREAM_PROBE", "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(
            output.status.success(),
            "{stdout}{}",
            String::from_utf8_lossy(&output.stderr)
        );
        // the filter matched the probe: a mistyped name would pass with no test run
        assert!(stdout.contains("1 passed"), "{stdout}");
        let events = probe_events(&stdout);
        let reset = reset().to_ndjson();
        let at = events
            .iter()
            .position(|event| *event == reset)
            .unwrap_or_else(|| panic!("no reset after the transcript moved: {stdout}"));
        assert!(events[..at].iter().any(|event| event.contains("hello")), "{stdout}");
        assert!(!events[..at].iter().any(|event| event.contains("moved on")), "{stdout}");
        assert!(events[at..].iter().any(|event| event.contains("moved on")), "{stdout}");
    }

    /// The stream of [`a_stream_rechecks_the_pane_and_ends_when_its_agent_is_gone`], run in a child process of it:
    /// the events go to the process's real stdout, which the harness does not capture.
    #[cfg(unix)]
    #[test]
    fn stream_probe() {
        use std::io::{BufRead as _, Write as _};
        use std::os::unix::net::UnixListener;
        if std::env::var_os("CTL_STREAM_PROBE").is_none() {
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("ctl.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let next = transcript(directory.path(), "next.jsonl");
        std::fs::write(
            &next,
            b"{\"type\":\"user\",\"uuid\":\"u2\",\"message\":{\"content\":\"moved on\"}}\n",
        )
        .unwrap();
        let moved = record(&next);
        let responses = [
            serde_json::json!({"ok":true,"result":{"agent":moved}}).to_string(),
            serde_json::json!({"ok":false,"error":NO_AGENT}).to_string(),
        ];
        let server = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            for response in responses {
                let mut socket = accept_before(&listener, deadline);
                socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut request = String::new();
                std::io::BufReader::new(&socket).read_line(&mut request).unwrap();
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(&request).unwrap()["cmd"],
                    "agent.get"
                );
                writeln!(socket, "{response}").unwrap();
            }
        });
        let chat = chat(&[]).unwrap();
        let mut stream = stream(&chat, record(&transcript(directory.path(), "c.jsonl")));
        stream.socket = socket;
        let first = stream.follower.backlog(10).unwrap();
        let (_sender, ended) = mpsc::channel();
        let result = stream.run(first, &ended);
        // the stream's own error first: a stream that failed leaves the server waiting for a recheck that never comes
        assert!(result.is_ok(), "{result:?}");
        server.join().unwrap();
    }

    #[cfg(unix)]
    fn accept_before(listener: &std::os::unix::net::UnixListener, deadline: Instant) -> std::os::unix::net::UnixStream {
        loop {
            match listener.accept() {
                Ok((socket, _)) => {
                    // On macOS an accepted socket inherits the listener's non-blocking mode; the caller reads blocking.
                    socket.set_nonblocking(false).unwrap();
                    return socket;
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "stream never rechecked its pane");
                    thread::yield_now();
                }
                Err(error) => panic!("accept: {error}"),
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn cyclic_transcript_paths_fail_pages_and_backlogs_with_the_local_read_error() {
        let directory = tempfile::tempdir().unwrap();
        let unreadable = directory.path().join("projects/work/cycle.jsonl");
        std::fs::create_dir_all(unreadable.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink("cycle.jsonl", &unreadable).unwrap();
        let record = record(&unreadable);
        let reply = serde_json::json!({"ok":true,"result":{"agent":record}}).to_string();
        for arguments in [
            vec!["agent", "chat", "--before", "10"],
            vec!["agent", "chat"],
            vec!["agent", "chat", "--after", "0"],
        ] {
            let (output, _) = crate::messages::tests::run_command(&arguments, b"", &[], &[&reply]);
            let error = String::from_utf8(output.stderr).unwrap();
            assert!(
                error.contains("could not read the transcript: Too many levels of symbolic links"),
                "{error}"
            );
        }
        let chat = chat(&[]).unwrap();
        let mut stream = stream(&chat, self::record(&transcript(directory.path(), "valid.jsonl")));
        let Err(Failure::Local(error)) = stream.recheck(record) else {
            panic!("accepted an unreadable transcript")
        };
        assert_eq!(
            error,
            "could not read the transcript: Too many levels of symbolic links"
        );
    }
}
