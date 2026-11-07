//! The agent chat view's core: what each pane of an attached session runs, the chat channels of the panes the app
//! shows as a chat, their older pages and the agents' slash-command catalogs. Everything runs on the origin as
//! `ngtermctl agent` (Rust-only, `crates/ngterm-ctl/src/agent.rs`), on exec channels of the machine's connection.
//!
//! - **Discovery.** A session's presentation stream lists the opt-in `agent` kind in its hello
//!   ([`crate::presentation`]), so the origin announces each pane's agent (or none) when the stream connects and on
//!   every change, as [`MobileEvent::PaneAgent`]; an origin that predates the kind never does, and the app keeps the
//!   terminal. The frames name a pane by its identity; `ngtermctl agent` names it `left` or `right`, which the session's
//!   layout maps (its primary pane is the left one), so the chats keep each attached session's last layout.
//! - **The channel.** `Chats::open` runs `ngtermctl agent chat --target=SESSION --pane SIDE` for a shown pane, and
//!   only for one: the app opens it while the pane shows as a chat and closes it on hide, so a session costs at most
//!   one chat channel per shown pane besides the pane's own attach (the ssh server's `MaxSessions` counts them). Its
//!   NDJSON lines arrive as [`MobileEvent::Chat`], a batch per read, in order. The chat keeps a cursor of the main
//!   transcript (a subagent's cursor is in its own file) to resume from, and the oldest to page from.
//! - **Resuming.** A lost channel (a lost connection, an origin command that failed after the backlog arrived) is
//!   [`ChatState::Reconnecting`] and resumes with `--after` the resume cursor: at once when the machine is back, or on
//!   the presentation stream's backoff ([`retry_delay`]) for a failed command, up to [`MAX_FAILURES`] in a row. The
//!   origin sends the cursor's line again, and an event identical to one already delivered (its id, parent and body:
//!   not its cursor, which a carried `conversation` changes) is dropped here, so the app sees no repeats and no gaps.
//!   A cursor is an offset into one transcript, so it always goes with the conversation it was read in
//!   (`--conversation`, the id the chat's last main `conversation` event named, empty when none came): the origin
//!   weighs it against the pane's record when the command starts, and a pane that moved to another conversation
//!   meanwhile (a `/clear` or a resume while the phone was away, the new file longer or shorter than the cursor)
//!   answers a `reset` and the new backlog, which start the chat over as a `reset` on a live channel does. The
//!   presentation stream's [`MobileEvent::PaneAgent`] plays no part in it: it may come before the channel is back,
//!   after it or not at all.
//! - **The resume cursor.** The origin ends each batch it writes with a `live`: a launch's catch-up (the main
//!   transcript's lines, then the subagents': the backlogs of those among them, and what every subagent wrote since
//!   the main line before the cursor, by its stamp to the minute), a `reset`'s backlog, and each poll's new lines
//!   while following (the main transcript's, then the subagents'). The `live`'s cursor is where the origin follows
//!   from, and only a `live` moves the resume cursor: a channel lost within a batch resumes where the last one ended,
//!   and the origin sends that batch's subagent lines again. Moved to a main event of the lost batch, the resume would
//!   read the subagents only from that event's stamp on and skip what they wrote before it and had not sent yet. A
//!   catch-up that began with no cursor (the first launch, a `reset`) takes its first main event's: resuming from what
//!   the app holds first sends all of it again, never less, where a plain backlog may start later and leave a gap.
//!   The app sees the `live` that ends a catch-up; the ones that end a poll's batch are not passed on.
//! - **The end.** The command's exit 0 (the record cleared: the agent ended), its "no agent in this pane", a failure
//!   before the first `live` (an origin without `ngtermctl agent chat`) or too many failures in a row end the chat:
//!   [`ChatState::Ended`], with no error, and the app shows the terminal.
//! - **Older pages** (`Chats::older`): `--before` the oldest cursor, once, with its conversation; a `reset` meanwhile
//!   (on the channel, or the page's own when the pane moved to another conversation) makes the page stale. Each chat
//!   opened and each conversation it shows takes a new generation of one counter, so a page begun before a close and
//!   answered after the pane's chat was opened again is stale too.
//! - **The catalog** (`Chats::commands`): `ngtermctl agent commands` once per conversation, kept per pane until the
//!   pane's announced conversation changes or its chat reads a `reset`. A failed fetch is not kept.
//!
//! The composer's dropdown narrows the catalog with [`filter_commands`], the native picker's ranking.
//!
//! **Answering the agent** happens in its pane, not over the chat channel: [`input`] sends a prompt, Stop, an
//! approval's or a question's answer as keys into the pane's terminal ([`crate::api::Pane::send_prompt`] and the
//! others), after taking the lead and checking the screen with the agent's matcher (`matchers`); the pane's attach
//! stays open behind the chat for it, at the grid the terminal view gave, so opening, reading or closing a chat never
//! resizes or claims anything.

pub mod input;
pub(crate) mod matchers;
mod records;
#[cfg(test)]
pub(crate) mod tests;
pub(crate) mod upload;

pub use self::upload::UploadCancel;

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;
use std::time::{Duration, Instant};

use ngterm_core::agent_chat::catalog::SlashCommand;
use ngterm_core::agent_chat::{self as wire, EventBody, NO_AGENT};
use ngterm_core::command_restore::shell_quoted_line;
use ngterm_core::fuzzy::fuzzy_rank;
use ngterm_core::palette::search::palette_search_keys;
use ngterm_core::pane_env::uuid_string;
use ngterm_core::remote::reconnect::retry_delay;
use ngterm_core::remote::tree::{REMOTE_CLI, remote_cli_chain};
use tokio::sync::mpsc;
use uuid::Uuid;

pub use self::records::{
    AgentCommand, AgentCommandKind, ChatApproval, ChatBody, ChatEvent, ChatNoticeKind, ChatQuestion, ChatQuestionItem,
    ChatQuestionOption, ChatState, ChatTool, ChatToolKind, ChatToolStatus, ChatUsage,
};
use crate::api::{MobileError, MobileEvent};
use crate::pane::{PaneRef, UNKNOWN_PANE};
use crate::presentation::RemoteLayout;
use crate::transport::{ChannelEnd, ChannelOutput, Connection};

/// The events an older page holds, about.
pub const OLDER_PAGE: usize = 100;

/// The most an older page or the catalog takes, the command's start included.
pub const FETCH_DEADLINE: Duration = Duration::from_secs(15);

/// The longest chat line read: the longest event the origin sends. It caps a tool's output (4 KiB) and a diff (64 KiB),
/// but sends a message's text whole (a prompt, a reply, a compaction's summary) from a transcript line of up to
/// [`LINE_CAP`](ngterm_core::agent_chat::event::LINE_CAP), escaped no longer than that line had it (both are JSON
/// and escape the same characters); the rest is the event's id, cursor and fields. A line past this is not one: it is skipped to its newline.
pub const EVENT_LINE_CAP: usize = ngterm_core::agent_chat::event::LINE_CAP + 64 * 1024;

/// The failures in a row (each one retried on its backoff) after which a chat ends.
pub const MAX_FAILURES: u32 = 5;

/// Why a chat was not opened: the app shows no such pane.
pub const NOT_SHOWN: &str = "the pane is not shown";

/// Why a page was not read: the pane's chat is not open.
pub const NOT_OPEN: &str = "the pane's chat is not open";

/// The most standard error kept of a chat channel, to read its end by.
const STDERR_CAP: usize = 4096;

/// Which pane `ngtermctl agent` addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Side {
    /// The session's primary pane.
    Left,
    /// Its split.
    Right,
}

impl Side {
    fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}

/// The words `ngtermctl agent SUBCOMMAND` takes for `side`'s pane of `session_id`. A value the origin or an agent made
/// goes as `--option=VALUE`, one word, so one that starts with `-` stays a value.
fn agent_words(subcommand: &str, session_id: &str, side: Side) -> Vec<String> {
    vec![
        "agent".to_owned(),
        subcommand.to_owned(),
        format!("--target={session_id}"),
        "--pane".to_owned(),
        side.name().to_owned(),
    ]
}

/// A cursor in the main transcript of the conversation it was read in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Cursor<'a> {
    pub(crate) at: u64,
    /// The conversation's id; empty when none was announced, which no conversation matches.
    pub(crate) conversation: &'a str,
}

/// The chat channel's remote command: the backlog, or everything from the line at `after` on, then following (or a
/// `reset` and the backlog when the pane is in another conversation now).
#[must_use]
pub(crate) fn chat_tail(session_id: &str, side: Side, after: Option<Cursor<'_>>) -> String {
    let mut words = agent_words("chat", session_id, side);
    if let Some(after) = after {
        words.extend([
            "--after".to_owned(),
            after.at.to_string(),
            format!("--conversation={}", after.conversation),
        ]);
    }
    remote_cli_chain(&format!("exec {REMOTE_CLI} {}", shell_quoted_line(&words)))
}

/// An older page's remote command: about `limit` events before `before` (or a `reset` alone when the pane is in
/// another conversation now).
#[must_use]
pub(crate) fn page_tail(session_id: &str, side: Side, before: Cursor<'_>, limit: usize) -> String {
    let mut words = agent_words("chat", session_id, side);
    words.extend([
        "--before".to_owned(),
        before.at.to_string(),
        format!("--conversation={}", before.conversation),
        "--limit".to_owned(),
        limit.to_string(),
    ]);
    remote_cli_chain(&format!("{REMOTE_CLI} {}", shell_quoted_line(&words)))
}

/// The catalog's remote command.
#[must_use]
pub(crate) fn commands_tail(session_id: &str, side: Side) -> String {
    let words = agent_words("commands", session_id, side);
    remote_cli_chain(&format!("{REMOTE_CLI} {}", shell_quoted_line(&words)))
}

/// The dropdown's rows for `query` (what follows the `/`): the commands whose name matches, best first, ties A→Z, as
/// the native picker ranks a caller's rows (`fuzzy_rank` over the label alone: a description's words would match
/// nearly any short query); a blank query lists them all A→Z.
#[uniffi::export]
#[must_use]
#[expect(
    clippy::needless_pass_by_value,
    reason = "UniFFI passes the foreign side's arguments owned"
)]
pub fn filter_commands(query: String, items: Vec<AgentCommand>) -> Vec<AgentCommand> {
    fuzzy_rank(query.trim(), items, |item| {
        palette_search_keys(&item.name, Some(&item.description), true)
    })
}

/// The chat events of an origin's output, one per line; a line that is not one (a newer origin's event) is skipped.
pub(crate) fn parse_events(output: &str) -> Vec<wire::ChatEvent> {
    output
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// The catalog an origin printed; an entry that is not one (a newer origin's kind) is skipped.
///
/// # Errors
///
/// What was printed is not a JSON array.
pub(crate) fn parse_catalog(output: &str) -> Result<Vec<AgentCommand>, String> {
    let entries: Vec<serde_json::Value> =
        serde_json::from_str(output.trim()).map_err(|_| "the origin's catalog is not a list".to_owned())?;
    Ok(entries
        .into_iter()
        .filter_map(|entry| serde_json::from_value::<SlashCommand>(entry).ok())
        .map(AgentCommand::from)
        .collect())
}

/// Splits a chat channel's output into events.
#[derive(Default)]
struct Lines {
    partial: Vec<u8>,
    /// A line past [`EVENT_LINE_CAP`] is being skipped to its newline.
    skipping: bool,
}

impl Lines {
    fn take(&mut self, bytes: &[u8]) -> Vec<wire::ChatEvent> {
        let mut events = Vec::new();
        let mut rest = bytes;
        while let Some(at) = rest.iter().position(|&byte| byte == b'\n') {
            let (line, tail) = rest.split_at(at);
            rest = &tail[1..];
            if std::mem::take(&mut self.skipping) {
                continue;
            }
            self.partial.extend_from_slice(line);
            if let Ok(event) = serde_json::from_slice(&self.partial) {
                events.push(event);
            }
            self.partial.clear();
        }
        if !self.skipping {
            self.partial.extend_from_slice(rest);
            if self.partial.len() > EVENT_LINE_CAP {
                self.partial.clear();
                self.skipping = true;
            }
        }
        events
    }
}

/// What a chat channel reports to its owner.
#[derive(Debug)]
pub(crate) enum LinkReport {
    /// The events of one read, in order.
    Events(Vec<wire::ChatEvent>),
    /// The channel ended, with what it wrote on standard error (its first [`STDERR_CAP`] bytes).
    Ended { end: ChannelEnd, stderr: String },
}

/// Where the chat channels report, under their launch number.
pub(crate) type Reporter = Rc<dyn Fn(u64, LinkReport)>;

/// One launch of a chat channel: opens it and reports what it reads, until its stop sender is dropped, which closes
/// the channel (the origin's `ngtermctl` sees its standard input end and exits).
async fn run_link<C: Connection>(
    connection: Rc<C>,
    command: String,
    launch: u64,
    mut stop: mpsc::UnboundedReceiver<()>,
    report: Reporter,
) {
    let opened = tokio::select! {
        opened = connection.exec(&command, None) => opened,
        _ = stop.recv() => return,
    };
    let mut channel = match opened {
        Ok(channel) => channel,
        Err(error) => {
            let end = ChannelEnd::Lost(error.to_string());
            return report(
                launch,
                LinkReport::Ended {
                    end,
                    stderr: String::new(),
                },
            );
        }
    };
    let mut lines = Lines::default();
    let mut stderr = Vec::new();
    loop {
        tokio::select! {
            biased;
            _ = stop.recv() => {
                // A channel already gone has nothing to close.
                let _ = channel.writer.close();
                return;
            }
            output = channel.reader.read() => match output {
                ChannelOutput::Data(bytes) => {
                    let events = lines.take(&bytes);
                    if !events.is_empty() {
                        report(launch, LinkReport::Events(events));
                    }
                }
                ChannelOutput::Stderr(bytes) => {
                    let room = STDERR_CAP.saturating_sub(stderr.len());
                    stderr.extend_from_slice(&bytes[..bytes.len().min(room)]);
                }
                ChannelOutput::End(end) => {
                    let stderr = String::from_utf8_lossy(&stderr).into_owned();
                    return report(launch, LinkReport::Ended { end, stderr });
                }
            },
        }
    }
}

/// A pane, as the chats name it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ChatKey {
    pub(crate) machine_id: String,
    pub(crate) session_id: String,
    pub(crate) pane: Uuid,
}

impl ChatKey {
    /// The pane `reference` names.
    ///
    /// # Errors
    ///
    /// [`MobileError::Remote`] with [`UNKNOWN_PANE`] for a pane id that is not an identity.
    pub(crate) fn of(reference: &PaneRef) -> Result<Self, MobileError> {
        let pane = Uuid::parse_str(&reference.pane_id).map_err(|_| MobileError::Remote {
            reason: UNKNOWN_PANE.to_owned(),
        })?;
        Ok(Self {
            machine_id: reference.machine_id.clone(),
            session_id: reference.session_id.clone(),
            pane,
        })
    }

    fn reference(&self) -> PaneRef {
        PaneRef {
            machine_id: self.machine_id.clone(),
            session_id: self.session_id.clone(),
            pane_id: uuid_string(self.pane),
        }
    }

    fn of_session(&self, machine_id: &str, session_id: &str) -> bool {
        self.machine_id == machine_id && self.session_id == session_id
    }
}

/// A one-shot command for a pane's chat or catalog, to run on `connection`; its answer is weighed against
/// `generation`.
pub(crate) struct Job<C> {
    pub(crate) connection: Rc<C>,
    pub(crate) command: String,
    pub(crate) generation: u64,
}

/// What [`Chats::commands`] found.
pub(crate) enum Lookup<C> {
    /// The catalog kept for the pane's conversation.
    Kept(Vec<AgentCommand>),
    /// None is kept: fetch it.
    Fetch(Job<C>),
}

/// A pane's catalog, kept for its conversation.
#[derive(Default)]
struct Catalog {
    /// Counts the pane's conversations: a fetch begun under an earlier one is not kept.
    generation: u64,
    commands: Option<Vec<AgentCommand>>,
}

/// A running chat channel; dropping it stops the channel.
struct Link {
    launch: u64,
    _stop: mpsc::UnboundedSender<()>,
}

/// One open chat.
struct Chat<C> {
    key: ChatKey,
    side: Side,
    report: Reporter,
    /// `None` while the machine's connection is down.
    connection: Option<Rc<C>>,
    link: Option<Link>,
    /// Where a reconnect resumes (the module docs): the last `live`'s, or the first main event's of a catch-up that
    /// began with none.
    resume: Option<u64>,
    /// Between a launch (or a `reset`) and its `live`, which the app is shown; the later ones end a poll's batch.
    catching_up: bool,
    /// The oldest: an older page ends there.
    oldest: Option<u64>,
    /// The conversation the cursors are in: what the last main `conversation` event since the last `reset` named.
    conversation: Option<String>,
    /// A `live` arrived: the origin runs the chat.
    started: bool,
    /// [`ChatState::Reconnecting`] was reported and no `live` has arrived since.
    reconnecting: bool,
    /// Failures since the last `live`.
    failures: u32,
    retry_at: Option<Instant>,
    /// What was delivered of each id, hashed: an identical repeat is dropped.
    delivered: HashMap<String, u64>,
    /// This chat's conversation shown now (a new one at each `reset`), from [`Chats`]' one counter: an older page
    /// begun under another generation, of this chat or of an earlier chat of the pane, is stale.
    generation: u64,
}

impl<C: Connection> Chat<C> {
    /// (Re)opens the channel, from where the chat stands: after its newest cursor, in that cursor's conversation.
    fn launch(&mut self, last_launch: &mut u64) {
        self.retry_at = None;
        let Some(connection) = self.connection.clone() else {
            self.link = None;
            return;
        };
        self.catching_up = true;
        *last_launch += 1;
        let launch = *last_launch;
        let (stop, stopped) = mpsc::unbounded_channel();
        let after = self.resume.map(|at| Cursor {
            at,
            conversation: self.conversation.as_deref().unwrap_or_default(),
        });
        let command = chat_tail(&self.key.session_id, self.side, after);
        tokio::task::spawn_local(run_link(connection, command, launch, stopped, Rc::clone(&self.report)));
        self.link = Some(Link { launch, _stop: stop });
    }

    /// The events of one read the app has not seen, and whether a `reset` was among them; a `reset` takes the next
    /// of `generations`.
    fn take(&mut self, events: Vec<wire::ChatEvent>, generations: &mut u64) -> (Vec<ChatEvent>, bool) {
        let mut shown = Vec::with_capacity(events.len());
        let mut reset = false;
        for event in events {
            match &event.body {
                EventBody::Reset => {
                    reset = true;
                    *generations += 1;
                    self.generation = *generations;
                    self.resume = None;
                    self.oldest = None;
                    self.conversation = None;
                    self.delivered.clear();
                    self.catching_up = true;
                }
                EventBody::Live => {
                    self.started = true;
                    self.reconnecting = false;
                    self.failures = 0;
                    self.track(&event);
                    if !std::mem::replace(&mut self.catching_up, false) {
                        continue;
                    }
                }
                body => {
                    if let (EventBody::Conversation { conversation_id, .. }, None) = (body, &event.parent) {
                        self.conversation = Some(conversation_id.clone());
                    }
                    self.track(&event);
                    let print = fingerprint(&event);
                    if self.delivered.insert(event.id.clone(), print) == Some(print) {
                        continue;
                    }
                }
            }
            shown.push(event.into());
        }
        (shown, reset)
    }

    /// Moves the cursors by a main-transcript event (the module docs). The `account` event is the origin's own, not a
    /// transcript line's, and moves neither.
    fn track(&mut self, event: &wire::ChatEvent) {
        if event.parent.is_some() || matches!(event.body, EventBody::Account { .. }) {
            return;
        }
        let at = event.cursor;
        self.oldest = Some(self.oldest.map_or(at, |oldest| oldest.min(at)));
        self.resume = Some(match (&event.body, self.resume) {
            (EventBody::Live, _) | (_, None) => at,
            (_, Some(resume)) => resume,
        });
    }

    /// The channel ended: whether that ends the chat; otherwise it is retried, and the app told once.
    fn ended(&mut self, end: &ChannelEnd, stderr: &str, now: Instant, events: &mut Vec<MobileEvent>) -> bool {
        self.link = None;
        let for_good = match end {
            ChannelEnd::Exit { code: Some(0), .. } => true,
            _ if stderr.contains(NO_AGENT) => true,
            ChannelEnd::Exit { .. } => !self.started,
            ChannelEnd::Lost(_) => false,
        };
        self.failures += 1;
        if for_good || self.failures > MAX_FAILURES {
            return true;
        }
        if self.connection.is_some() {
            self.retry_at = Some(now + retry_delay(self.failures));
        }
        self.reconnect(events);
        false
    }

    /// The machine's connection went: the channel goes with it, and waits for the next one.
    fn unlink(&mut self) {
        self.connection = None;
        self.link = None;
        self.retry_at = None;
    }

    /// Reports [`ChatState::Reconnecting`], once until the next `live`.
    fn reconnect(&mut self, events: &mut Vec<MobileEvent>) {
        if !std::mem::replace(&mut self.reconnecting, true) {
            events.push(MobileEvent::ChatState {
                pane: self.key.reference(),
                state: ChatState::Reconnecting,
            });
        }
    }
}

/// An event's id, parent and body, hashed: what the app would show of it.
fn fingerprint(event: &wire::ChatEvent) -> u64 {
    let mut hasher = DefaultHasher::new();
    let mut shown = event.clone();
    shown.cursor = 0;
    shown.to_ndjson().hash(&mut hasher);
    hasher.finish()
}

/// A session's panes as its layout names them: the primary one is `left`.
struct Layout {
    primary: Uuid,
    panes: Vec<Uuid>,
}

impl Layout {
    fn of(layout: &RemoteLayout) -> Option<Self> {
        let parse = |id: &str| Uuid::parse_str(id).ok();
        Some(Self {
            primary: parse(&layout.primary)?,
            panes: layout.panes.iter().filter_map(|id| parse(id)).collect(),
        })
    }

    fn side(&self, pane: Uuid) -> Option<Side> {
        if pane == self.primary {
            Some(Side::Left)
        } else {
            self.panes.contains(&pane).then_some(Side::Right)
        }
    }
}

/// Every open chat, and what the chats need to know of the attached sessions.
pub(crate) struct Chats<C> {
    open: Vec<Chat<C>>,
    /// Each attached session's last layout, by machine and session id.
    layouts: HashMap<(String, String), Layout>,
    /// What each pane was last announced to run: its agent and conversation.
    announced: HashMap<ChatKey, (Option<String>, Option<String>)>,
    catalogs: HashMap<ChatKey, Catalog>,
    /// The last launch number handed out: unique among every chat's channels.
    last_launch: u64,
    /// The last chat generation handed out: unique among every chat opened and every conversation each one showed.
    last_generation: u64,
}

impl<C> Default for Chats<C> {
    fn default() -> Self {
        Self {
            open: Vec::new(),
            layouts: HashMap::new(),
            announced: HashMap::new(),
            catalogs: HashMap::new(),
            last_launch: 0,
            last_generation: 0,
        }
    }
}

impl<C: Connection> Chats<C> {
    /// Learns from an event of the presentation streams: a session's layout (an open chat whose pane changed sides
    /// resumes on its new side) and a pane's announced conversation (a new one drops the pane's catalog).
    pub(crate) fn observe(&mut self, event: &MobileEvent) {
        match event {
            MobileEvent::SessionLayout {
                machine_id,
                session_id,
                layout,
            } => {
                if let Some(layout) = Layout::of(layout) {
                    self.laid_out(machine_id, session_id, layout);
                }
            }
            MobileEvent::PaneAgent {
                machine_id,
                session_id,
                pane_id,
                agent,
                conversation_id,
            } => {
                let Ok(pane) = Uuid::parse_str(pane_id) else { return };
                let key = ChatKey {
                    machine_id: machine_id.clone(),
                    session_id: session_id.clone(),
                    pane,
                };
                let now = (agent.clone(), conversation_id.clone());
                if self.announced.insert(key.clone(), now.clone()).as_ref() != Some(&now) {
                    self.forget_catalog(&key);
                }
            }
            _ => {}
        }
    }

    /// `session_id`'s new layout: each of its open chats whose pane changed sides resumes on its new side.
    fn laid_out(&mut self, machine_id: &str, session_id: &str, layout: Layout) {
        let moved = self.open.iter_mut().filter_map(|chat| {
            let side = layout.side(chat.key.pane)?;
            (chat.key.of_session(machine_id, session_id) && side != chat.side).then_some((chat, side))
        });
        for (chat, side) in moved {
            chat.side = side;
            if chat.link.is_some() {
                chat.launch(&mut self.last_launch);
            }
        }
        self.layouts
            .insert((machine_id.to_owned(), session_id.to_owned()), layout);
    }

    /// The agent `key`'s pane was last announced to run, if any.
    pub(crate) fn agent(&self, key: &ChatKey) -> Option<&str> {
        self.announced.get(key).and_then(|(agent, _)| agent.as_deref())
    }

    /// The side `key`'s pane is on, as its session's last layout has it.
    fn side(&self, key: &ChatKey) -> Option<Side> {
        self.layouts
            .get(&(key.machine_id.clone(), key.session_id.clone()))
            .and_then(|layout| layout.side(key.pane))
    }

    fn find(&self, key: &ChatKey) -> Option<usize> {
        self.open.iter().position(|chat| chat.key == *key)
    }

    fn forget_catalog(&mut self, key: &ChatKey) {
        let catalog = self.catalogs.entry(key.clone()).or_default();
        catalog.generation += 1;
        catalog.commands = None;
    }

    /// Opens `key`'s chat on `connection`, its channel reporting to `report`. An open chat stays as it is.
    ///
    /// # Errors
    ///
    /// [`MobileError::Remote`] with [`UNKNOWN_PANE`] when the session's layout does not hold the pane.
    pub(crate) fn open(&mut self, key: ChatKey, connection: Rc<C>, report: Reporter) -> Result<(), MobileError> {
        if self.find(&key).is_some() {
            return Ok(());
        }
        let side = self.side(&key).ok_or_else(|| MobileError::Remote {
            reason: UNKNOWN_PANE.to_owned(),
        })?;
        self.last_generation += 1;
        let mut chat = Chat {
            key,
            side,
            report,
            connection: Some(connection),
            link: None,
            resume: None,
            catching_up: true,
            oldest: None,
            conversation: None,
            started: false,
            reconnecting: false,
            failures: 0,
            retry_at: None,
            delivered: HashMap::new(),
            generation: self.last_generation,
        };
        chat.launch(&mut self.last_launch);
        self.open.push(chat);
        Ok(())
    }

    /// Closes `key`'s chat, reporting nothing.
    pub(crate) fn close(&mut self, key: &ChatKey) {
        self.open.retain(|chat| chat.key != *key);
    }

    /// Closes the chats of `session_id`, whose stream was detached, and forgets what was known of it.
    pub(crate) fn close_session(&mut self, machine_id: &str, session_id: &str) {
        self.open.retain(|chat| !chat.key.of_session(machine_id, session_id));
        self.layouts.remove(&(machine_id.to_owned(), session_id.to_owned()));
        self.announced.retain(|key, _| !key.of_session(machine_id, session_id));
        self.catalogs.retain(|key, _| !key.of_session(machine_id, session_id));
    }

    /// Closes the chats of `machine_id`, whose connection was closed, and forgets what was known of its sessions.
    pub(crate) fn forget_machine(&mut self, machine_id: &str) {
        self.open.retain(|chat| chat.key.machine_id != machine_id);
        self.layouts.retain(|(machine, _), _| machine != machine_id);
        self.announced.retain(|key, _| key.machine_id != machine_id);
        self.catalogs.retain(|key, _| key.machine_id != machine_id);
    }

    /// Closes every chat. The counters go on, so nothing begun before is taken for a later chat's.
    pub(crate) fn clear(&mut self) {
        *self = Self {
            last_launch: self.last_launch,
            last_generation: self.last_generation,
            ..Self::default()
        };
    }

    /// `machine_id`'s connection was lost: its chats' channels go, each reported reconnecting, and wait for it.
    pub(crate) fn machine_down(&mut self, machine_id: &str) -> Vec<MobileEvent> {
        let mut events = Vec::new();
        for chat in self.open.iter_mut().filter(|chat| chat.key.machine_id == machine_id) {
            chat.unlink();
            chat.reconnect(&mut events);
        }
        events
    }

    /// `machine_id`'s connection was closed to suspend: as [`Chats::machine_down`], reporting nothing, so nothing
    /// counts as reported either: a resume that does not go live reports [`ChatState::Reconnecting`] then.
    pub(crate) fn suspend_machine(&mut self, machine_id: &str) {
        for chat in self.open.iter_mut().filter(|chat| chat.key.machine_id == machine_id) {
            chat.unlink();
        }
    }

    /// `machine_id` is connected on `connection`: each of its chats with no channel resumes at once.
    pub(crate) fn machine_up(&mut self, machine_id: &str, connection: &Rc<C>) {
        for chat in self.open.iter_mut().filter(|chat| chat.key.machine_id == machine_id) {
            chat.connection = Some(Rc::clone(connection));
            if chat.link.is_none() {
                chat.failures = 0;
                chat.launch(&mut self.last_launch);
            }
        }
    }

    /// Resumes each chat whose retry is due.
    pub(crate) fn tick(&mut self, now: Instant) {
        for chat in &mut self.open {
            if chat.link.is_none() && chat.retry_at.is_some_and(|at| now >= at) {
                chat.launch(&mut self.last_launch);
            }
        }
    }

    /// What channel `launch` reported; a report of a channel since stopped is dropped.
    pub(crate) fn report(&mut self, launch: u64, report: LinkReport, now: Instant) -> Vec<MobileEvent> {
        let Some(index) = self
            .open
            .iter()
            .position(|chat| chat.link.as_ref().is_some_and(|link| link.launch == launch))
        else {
            return Vec::new();
        };
        let mut out = Vec::new();
        match report {
            LinkReport::Events(events) => {
                let chat = &mut self.open[index];
                let (events, reset) = chat.take(events, &mut self.last_generation);
                let key = chat.key.clone();
                if !events.is_empty() {
                    out.push(MobileEvent::Chat {
                        pane: key.reference(),
                        events,
                    });
                }
                if reset {
                    self.forget_catalog(&key);
                }
            }
            LinkReport::Ended { end, stderr } => {
                if self.open[index].ended(&end, &stderr, now, &mut out) {
                    let chat = self.open.remove(index);
                    out.push(MobileEvent::ChatState {
                        pane: chat.key.reference(),
                        state: ChatState::Ended,
                    });
                }
            }
        }
        out
    }

    /// The page before `key`'s oldest event to read, in its conversation, or `None` when there is nothing before it
    /// (or no conversation was announced, which leaves nothing to page).
    ///
    /// # Errors
    ///
    /// [`MobileError::Remote`] with [`NOT_OPEN`] when the pane's chat is not open, [`MobileError::NotConnected`]
    /// while its machine is down.
    pub(crate) fn older(&self, key: &ChatKey) -> Result<Option<Job<C>>, MobileError> {
        let chat = self
            .find(key)
            .map(|index| &self.open[index])
            .ok_or_else(|| MobileError::Remote {
                reason: NOT_OPEN.to_owned(),
            })?;
        let connection = chat.connection.clone().ok_or_else(|| MobileError::NotConnected {
            machine_id: key.machine_id.clone(),
        })?;
        let before = chat
            .oldest
            .filter(|oldest| *oldest > 0)
            .zip(chat.conversation.as_deref());
        Ok(before.map(|(at, conversation)| Job {
            connection,
            command: page_tail(&key.session_id, chat.side, Cursor { at, conversation }, OLDER_PAGE),
            generation: chat.generation,
        }))
    }

    /// The older page read for `key` under `generation`, which moves its oldest cursor.
    ///
    /// # Errors
    ///
    /// [`MobileError::Cancelled`] when the chat closed (even if the pane's chat was opened again) or its
    /// conversation switched meanwhile, here or on the origin (the page is a `reset`: the channel brings its own).
    pub(crate) fn paged(
        &mut self,
        key: &ChatKey,
        generation: u64,
        events: Vec<wire::ChatEvent>,
    ) -> Result<Vec<ChatEvent>, MobileError> {
        let chat = self
            .find(key)
            .map(|index| &mut self.open[index])
            .filter(|chat| chat.generation == generation)
            .ok_or(MobileError::Cancelled)?;
        if events.iter().any(|event| matches!(event.body, EventBody::Reset)) {
            return Err(MobileError::Cancelled);
        }
        for event in events.iter().filter(|event| event.parent.is_none()) {
            chat.oldest = Some(chat.oldest.map_or(event.cursor, |oldest| oldest.min(event.cursor)));
        }
        Ok(events.into_iter().map(ChatEvent::from).collect())
    }

    /// `key`'s catalog: the one kept for its conversation, or what to run to fetch it on `connection`.
    ///
    /// # Errors
    ///
    /// [`MobileError::Remote`] with [`UNKNOWN_PANE`] when the session's layout does not hold the pane.
    pub(crate) fn commands(&mut self, key: &ChatKey, connection: Rc<C>) -> Result<Lookup<C>, MobileError> {
        let side = self.side(key).ok_or_else(|| MobileError::Remote {
            reason: UNKNOWN_PANE.to_owned(),
        })?;
        let catalog = self.catalogs.entry(key.clone()).or_default();
        if let Some(commands) = &catalog.commands {
            return Ok(Lookup::Kept(commands.clone()));
        }
        Ok(Lookup::Fetch(Job {
            connection,
            command: commands_tail(&key.session_id, side),
            generation: catalog.generation,
        }))
    }

    /// `key`'s catalog, fetched under `generation`: kept unless its conversation changed meanwhile.
    pub(crate) fn fetched(&mut self, key: &ChatKey, generation: u64, commands: &[AgentCommand]) {
        if let Some(catalog) = self
            .catalogs
            .get_mut(key)
            .filter(|catalog| catalog.generation == generation)
        {
            catalog.commands = Some(commands.to_vec());
        }
    }
}
