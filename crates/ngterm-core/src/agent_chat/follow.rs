//! Following an agent's transcript for `ngtermctl agent chat`: the backlog read backward from the end, an older page,
//! everything after a cursor, then new lines as the agent writes them.
//!
//! [`Follower`] reads through [`Files`] (a file opened as `Read + Seek`, and a size probe) and tells the time by
//! [`Clock`], so the same code runs on every OS, and the tests count the bytes it reads and drive its time. It does
//! not wait: its caller polls it (`ngtermctl agent chat` every [`POLL_INTERVAL`], until its reader goes).
//!
//! - **Backlog** ([`Follower::backlog`]): the file is read backward from its end, each step doubling what was read,
//!   until its newest complete lines make the asked number of events, with [`CONTEXT`] bytes of older lines before
//!   them parsed but not shown, so a tool result in the window still pairs with a call made just before it. A file of
//!   any size costs that window and context, and at most as much again (the last step overshoots), never the whole
//!   file. Windows are cut at line boundaries, so a page never splits the events of one line.
//! - **Page** ([`Follower::page`]): the same, ending before a cursor; following is untouched.
//! - **After** ([`Follower::after`]): everything from the line at a cursor on, with [`CONTEXT`] bytes before it parsed
//!   for pairing. That line is sent again: the phone may have got its events in part, and an event's id makes a
//!   repeated one update the same item. A cursor past the file's end means the file shrank: a `reset` and a backlog.
//! - A backlog and the events after a cursor start with the `conversation` the lines before them announced when their
//!   own lines announce none (the parsers announce it once, and again only when the model changes); its cursor is the
//!   first shown line's, so paging from the oldest cursor skips nothing.
//! - **Following** ([`Follower::poll`]): each poll probes the size; growth is read and split into lines, a partial last
//!   line buffered until its newline. A file that shrinks, or whose first bytes change (it was replaced), starts over
//!   with a `reset` and a new backlog. The first bytes are compared even when the size did not change, on every poll
//!   for the main transcript and at most every [`DISCOVERY_INTERVAL`] for a subagent's. A line over [`LINE_CAP`] becomes `unknown{oversized}` and is never held whole.
//!   A file that does not exist yet (Claude writes its transcript at the first prompt) is read once it appears.
//! - **Subagents** (`subagents`): their transcripts are followed as extra sources, their events under the task tool.
//!   A subagent whose task is among the lines a window parsed is shown with a backlog of its own, one found while
//!   following likewise; the others are followed from their end. After a cursor, each subagent also sends every line
//!   stamped (`timestamp`, compared to the minute) no earlier than the last stamped main line before the cursor: the
//!   lines it wrote while the phone was away. Every subagent the conversation has is followed, Codex's too when its
//!   spawn is older than the lines read (found by its rollout's `session_meta`). A subagent event's `cursor` is an
//!   offset in its own file: paging goes by the events without `parent`, and the ids of its lines that name none are
//!   prefixed with the file's name.
//! - **Unshown** ([`Follower::take_unshown`]): the events a window parsed and did not show (its context's, for the main
//!   transcript and each subagent's) are kept for the caller, which weighs a permission prompt's call against them
//!   ([`Approvals::context`](super::approvals::Approvals::context)): a reconnect's cursor may be past the call's
//!   result. While the record holds a prompt ([`Follower::expect`]), the backlog's subagents followed from their end
//!   get that context too, and the main transcript's window reads further back until its lines hold the prompt's
//!   call or a prompt of the user's (or about [`REACH`] bytes were read); those older lines are only kept unshown, so
//!   the lines shown, and which subagents get a backlog, stay the same.
//!
//! A backlog and the events after a cursor end with a `live` event whose cursor is where following resumes, and so
//! does each poll that read anything, after its main transcript's events and its subagents': a reader that got the
//! `live` has every line of the transcripts up to it, so `--after` its cursor is where a reconnect resumes (the main
//! line before it was read no later than any subagent line not sent yet was written, which the minute stamps then
//! send again). A poll looks for new subagents at most every [`DISCOVERY_INTERVAL`].

mod lines;
mod subagents;

use std::collections::HashSet;
use std::fs::File;
use std::io::{self, ErrorKind, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use lines::{BackScan, RawLine, Splitter};
use ngterm_proto::ControlAgentApproval;
use serde::Deserialize;
use subagents::Look;

use super::approvals::is_call;
#[cfg(doc)]
use super::event::LINE_CAP;
use super::event::OVERSIZED;
use super::{AgentKind, Approval, ChatEvent, ClaudeParser, CodexParser, EventBody, TranscriptParser, line_id, unknown};

/// The events a backlog holds by default: the phone's first screenful and some.
pub const BACKLOG: usize = 200;

/// How often `ngtermctl agent chat` polls its follower.
pub const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// How often following looks for new subagents at most.
pub const DISCOVERY_INTERVAL: Duration = Duration::from_secs(1);

/// The bytes of older lines a window parses for pairing and does not show.
pub const CONTEXT: u64 = 64 * 1024;

/// How far before its end the main transcript's window reads back for a live prompt's call: past this, it stops at the
/// next step (each step doubles what was read, as for the backlog).
pub const REACH: u64 = 4 * 1024 * 1024;

/// A backward scan's first step beyond the context it wants; each later step doubles what was read, so the lines are
/// parsed at most about twice over in all.
const STEP: u64 = 64 * 1024;

/// A forward read's size.
const CHUNK: usize = 64 * 1024;

/// The first bytes of a file, compared to tell a replaced file from a grown one.
const FINGERPRINT: u64 = 1024;

/// The most bytes read for a Codex rollout's first line.
const HEAD_CAP: u64 = 256 * 1024;

/// The transcripts a follower reads.
pub trait Files {
    type Reader: Read + Seek;

    /// Opens the file at `path` for reading.
    ///
    /// # Errors
    /// The file cannot be opened; `NotFound` when it does not exist.
    fn open(&self, path: &Path) -> io::Result<Self::Reader>;

    /// The size of the file at `path` now.
    ///
    /// # Errors
    /// The file cannot be probed; `NotFound` when it does not exist.
    fn size(&self, path: &Path) -> io::Result<u64>;
}

/// The files on disk.
#[derive(Debug, Default, Clone, Copy)]
pub struct Disk;

impl Files for Disk {
    type Reader = File;

    fn open(&self, path: &Path) -> io::Result<File> {
        File::open(path)
    }

    fn size(&self, path: &Path) -> io::Result<u64> {
        Ok(std::fs::metadata(path)?.len())
    }
}

/// The time a follower's polls are told by.
pub trait Clock {
    fn now(&self) -> Instant;
}

/// The system's clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// Reads one conversation's transcript, and its subagents', as chat events.
#[derive(Debug)]
pub struct Follower<F: Files = Disk, C: Clock = SystemClock> {
    files: F,
    clock: C,
    agent: AgentKind,
    path: PathBuf,
    /// The backlog size, kept for the backlog after a `reset`.
    limit: usize,
    main: Stream,
    subagents: Vec<Stream>,
    /// The subagents followed, by [`subagents::Found::key`].
    known: HashSet<String>,
    discovered_at: Option<Instant>,
    /// The permission prompt the record holds, which a window reads back for.
    prompt: Option<Approval>,
    /// The events parsed and not shown since [`Follower::take_unshown`].
    unshown: Vec<ChatEvent>,
}

impl Follower {
    /// A follower of `agent`'s transcript at `path` on disk.
    pub fn new(agent: AgentKind, path: impl Into<PathBuf>) -> Self {
        Self::with(Disk, SystemClock, agent, path)
    }
}

impl<F: Files, C: Clock> Follower<F, C> {
    /// A follower reading through `files` and telling the time by `clock`. Nothing is read until the first call.
    pub fn with(files: F, clock: C, agent: AgentKind, path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        Self {
            files,
            clock,
            agent,
            limit: BACKLOG,
            main: Stream::absent(&path, Spec::main(agent)),
            path,
            subagents: Vec::new(),
            known: HashSet::new(),
            discovered_at: None,
            prompt: None,
            unshown: Vec::new(),
        }
    }

    /// The permission prompt the pane's record holds now, if any: the windows read from here on read back for its
    /// call (the module docs).
    pub fn expect(&mut self, live: Option<&ControlAgentApproval>) {
        self.prompt = live.map(Approval::from);
    }

    /// The events parsed and not shown since the last call, oldest first per transcript: the context of every window
    /// read since. A `reset` comes before all of them.
    pub fn take_unshown(&mut self) -> Vec<ChatEvent> {
        std::mem::take(&mut self.unshown)
    }

    /// The events of about the last `limit` events' lines, the backlogs of the subagents among them, then `live`;
    /// following resumes after them.
    ///
    /// # Errors
    /// The transcript cannot be read (a missing one is not an error: it is read once it appears).
    pub fn backlog(&mut self, limit: usize) -> io::Result<Vec<ChatEvent>> {
        self.limit = limit;
        self.forget_subagents();
        self.unshown.clear();
        let want = Want {
            reach: self.prompt.clone(),
            ..Want::backlog(limit)
        };
        let (main, window) = Stream::start(&self.files, &self.path, Spec::main(self.agent), None, &want)?;
        self.main = main;
        let shown = window.ids();
        let (mut events, unshown) = window.shown();
        self.unshown.extend(unshown);
        let limit = self.limit;
        // While a prompt is live, a subagent followed from its end still parses its context for the prompt's call.
        let end = if self.prompt.is_some() {
            Want::backlog(0)
        } else {
            Want::END
        };
        let want = |parent: &str| {
            if shown.contains(parent) {
                Want::backlog(limit)
            } else {
                end.clone()
            }
        };
        self.add_subagents(&want, Look::Scan, &mut events);
        events.push(marker(EventBody::Live, self.main.lines.start()));
        Ok(events)
    }

    /// The events of about `limit` events' lines before `cursor`, with the backlogs of the subagents among them; no
    /// `live`. An empty page means the transcript's start was reached. Following is untouched.
    ///
    /// # Errors
    /// The transcript cannot be read.
    pub fn page(&self, cursor: u64, limit: usize) -> io::Result<Vec<ChatEvent>> {
        let spec = Spec::main(self.agent);
        let (main, window) = Stream::start(&self.files, &self.path, spec, Some(cursor), &Want::backlog(limit))?;
        let shown = window.ids();
        let mut events = window.events;
        let spawned = main.parser.spawned();
        let found = subagents::find(self.agent, &self.path, &spawned, &HashSet::new(), Look::Spawned);
        for found in found.into_iter().filter(|found| shown.contains(&found.parent)) {
            let spec = Spec::subagent(self.agent, found.parent, found.source);
            if let Ok((_, window)) = Stream::start(&self.files, &found.path, spec, None, &Want::backlog(limit)) {
                events.extend(window.events);
            }
        }
        Ok(events)
    }

    /// Everything from the line at `cursor` on, the backlogs of the subagents among it and what every subagent wrote
    /// since the line before `cursor`, then `live`; following resumes after it. A cursor past the end is a `reset`
    /// and a backlog.
    ///
    /// # Errors
    /// The transcript cannot be read.
    pub fn after(&mut self, cursor: u64) -> io::Result<Vec<ChatEvent>> {
        let size = match self.files.size(&self.path) {
            Ok(size) => size,
            Err(error) if error.kind() == ErrorKind::NotFound => 0,
            Err(error) => return Err(error),
        };
        if cursor > size {
            return self.restart();
        }
        self.forget_subagents();
        self.unshown.clear();
        let want = Want {
            reach: self.prompt.clone(),
            ..Want::backlog(0)
        };
        let (main, window) = Stream::start(&self.files, &self.path, Spec::main(self.agent), Some(cursor), &want)?;
        self.main = main;
        let Some(new) = self.main.poll(&self.files, true)? else {
            return self.restart();
        };
        let mut shown = window.ids();
        shown.extend(new.iter().map(|event| event.id.clone()));
        // The phone has every line before the cursor's: the subagents' lines since the last of them are new to it.
        let since = window.stamp.clone().unwrap_or_default();
        let (mut events, unshown) = window.shown();
        self.unshown.extend(unshown);
        events.extend(new);
        let limit = self.limit;
        let want = |parent: &str| Want {
            limit: if shown.contains(parent) { limit } else { 0 },
            since: Some(since.clone()),
            ..Want::backlog(0)
        };
        self.add_subagents(&want, Look::Scan, &mut events);
        events.push(marker(EventBody::Live, self.main.lines.start()));
        Ok(events)
    }

    /// The events the transcripts' new lines make since the last read, then `live` where following resumes; nothing
    /// when there were none. A shrunk or replaced transcript makes a `reset`, a new backlog and `live`.
    ///
    /// # Errors
    /// The transcript cannot be read. A subagent's transcript that cannot be read is skipped.
    pub fn poll(&mut self) -> io::Result<Vec<ChatEvent>> {
        let now = self.clock.now();
        let Some(mut events) = self.main.poll(&self.files, true)? else {
            return self.restart();
        };
        let discover = self
            .discovered_at
            .is_none_or(|at| now.saturating_duration_since(at) >= DISCOVERY_INTERVAL);
        for stream in &mut self.subagents {
            match stream.poll(&self.files, discover) {
                Ok(Some(new)) => events.extend(new),
                Ok(None) => {
                    let spec = stream.spec.clone();
                    let want = Want::backlog(self.limit);
                    if let Ok((fresh, window)) = Stream::start(&self.files, &stream.path, spec, None, &want) {
                        *stream = fresh;
                        let (shown, unshown) = window.parts();
                        events.extend(shown);
                        self.unshown.extend(unshown);
                    }
                }
                Err(_) => {}
            }
        }
        if discover {
            let limit = self.limit;
            self.add_subagents(&|_| Want::backlog(limit), Look::Spawned, &mut events);
        }
        if !events.is_empty() {
            events.push(marker(EventBody::Live, self.main.lines.start()));
        }
        Ok(events)
    }

    /// The bytes held for the main transcript's partial last line.
    #[cfg(test)]
    pub(crate) fn buffered(&self) -> usize {
        self.main.lines.buffered()
    }

    /// A `reset`, then the backlog.
    fn restart(&mut self) -> io::Result<Vec<ChatEvent>> {
        let mut events = vec![marker(EventBody::Reset, 0)];
        events.extend(self.backlog(self.limit)?);
        Ok(events)
    }

    fn forget_subagents(&mut self) {
        self.subagents.clear();
        self.known.clear();
    }

    /// Follows the subagents not followed yet, looking for them as `look` says: each with the window `want` asks for
    /// by its task tool's id, added to `events`.
    fn add_subagents(&mut self, want: &dyn Fn(&str) -> Want, look: Look, events: &mut Vec<ChatEvent>) {
        self.discovered_at = Some(self.clock.now());
        let spawned = self.main.parser.spawned();
        for found in subagents::find(self.agent, &self.path, &spawned, &self.known, look) {
            let want = want(&found.parent);
            let spec = Spec::subagent(self.agent, found.parent, found.source);
            let Ok((stream, window)) = Stream::start(&self.files, &found.path, spec, None, &want) else {
                continue;
            };
            let (shown, unshown) = window.parts();
            events.extend(shown);
            self.unshown.extend(unshown);
            self.known.insert(found.key);
            self.subagents.push(stream);
        }
    }
}

/// A `reset` or `live` event.
fn marker(body: EventBody, cursor: u64) -> ChatEvent {
    let id = if matches!(body, EventBody::Reset) {
        "reset"
    } else {
        "live"
    };
    ChatEvent {
        id: id.to_owned(),
        cursor,
        parent: None,
        body,
    }
}

/// What a window asks for: about `limit` events, every line stamped no earlier than `since` (to the minute; the
/// unstamped lines among them too), and `context` bytes of older lines parsed before them; for `reach`, older lines
/// still, until the lines parsed hold its call or a prompt of the user's (about [`REACH`] bytes before the end at most).
#[derive(Debug, Clone)]
struct Want {
    limit: usize,
    context: u64,
    since: Option<String>,
    reach: Option<Approval>,
}

impl Want {
    /// No events and no context: only where following resumes.
    const END: Self = Self {
        limit: 0,
        context: 0,
        since: None,
        reach: None,
    };

    fn backlog(limit: usize) -> Self {
        Self {
            limit,
            context: CONTEXT,
            since: None,
            reach: None,
        }
    }
}

/// Which parser a transcript takes.
#[derive(Debug, Clone)]
struct Spec {
    agent: AgentKind,
    /// The task tool's id, for a subagent's transcript.
    parent: Option<String>,
    /// What names a subagent's transcript, for the ids of its lines that name none.
    source: Option<String>,
}

impl Spec {
    fn main(agent: AgentKind) -> Self {
        Self {
            agent,
            parent: None,
            source: None,
        }
    }

    fn subagent(agent: AgentKind, parent: String, source: String) -> Self {
        Self {
            agent,
            parent: Some(parent),
            source: Some(source),
        }
    }

    fn parser(&self) -> Parser {
        let kind = match (self.agent, &self.parent) {
            (AgentKind::Claude, None) => Kind::Claude(ClaudeParser::new()),
            (AgentKind::Claude, Some(parent)) => Kind::Claude(ClaudeParser::subagent(parent.clone())),
            (AgentKind::Codex, None) => Kind::Codex(CodexParser::new()),
            (AgentKind::Codex, Some(parent)) => Kind::Codex(CodexParser::subagent(parent.clone())),
        };
        let kind = match (kind, &self.source) {
            (Kind::Claude(parser), Some(source)) => Kind::Claude(parser.source(source.clone())),
            (Kind::Codex(parser), Some(source)) => Kind::Codex(parser.source(source.clone())),
            (kind, None) => kind,
        };
        Parser {
            kind,
            parent: self.parent.clone(),
            source: self.source.clone(),
        }
    }

    /// A Codex rollout's first line names its thread and, for a subagent, where the parent's history it starts with
    /// ends: a window that starts later reads that line first.
    fn primed(&self) -> bool {
        self.agent == AgentKind::Codex
    }
}

/// A transcript's parser.
#[derive(Debug)]
struct Parser {
    kind: Kind,
    parent: Option<String>,
    source: Option<String>,
}

#[derive(Debug)]
enum Kind {
    Claude(ClaudeParser),
    Codex(CodexParser),
}

impl Parser {
    fn parse(&mut self, line: &RawLine) -> Vec<ChatEvent> {
        let Some(bytes) = &line.bytes else {
            let mut event = unknown(line.cursor, OVERSIZED);
            event.id = line_id(self.source.as_deref(), line.cursor);
            event.parent.clone_from(&self.parent);
            return vec![event];
        };
        match &mut self.kind {
            Kind::Claude(parser) => parser.parse_line(bytes, line.cursor),
            Kind::Codex(parser) => parser.parse_line(bytes, line.cursor),
        }
    }

    /// The subagents a Codex conversation spawned: each thread and its task tool's id.
    fn spawned(&self) -> Vec<(String, String)> {
        match &self.kind {
            Kind::Claude(_) => Vec::new(),
            Kind::Codex(parser) => parser
                .spawned_threads()
                .map(|(thread, task)| (thread.to_owned(), task.to_owned()))
                .collect(),
        }
    }
}

/// What a window read: the events of its newest lines, those of the older lines it parsed for context (the Codex
/// rollout's first line's first) and of the lines older still it read for a prompt's call ([`Want::reach`]), where
/// the lines parsed start, where following resumes, and the newest `timestamp` among the context lines.
#[derive(Debug, Default)]
struct Window {
    reach: Vec<ChatEvent>,
    context: Vec<ChatEvent>,
    events: Vec<ChatEvent>,
    oldest: u64,
    next: u64,
    stamp: Option<String>,
}

impl Window {
    /// The ids of every event parsed for the window and its context.
    fn ids(&self) -> HashSet<String> {
        self.context
            .iter()
            .chain(&self.events)
            .map(|event| event.id.clone())
            .collect()
    }

    /// Whether the lines parsed hold `approval`'s call, or a prompt of the user's: a prompt still open was asked
    /// after the user's last one.
    fn holds(&self, approval: &Approval) -> bool {
        self.reach
            .iter()
            .chain(&self.context)
            .chain(&self.events)
            .any(|event| matches!(event.body, EventBody::User { .. }) || is_call(approval, event))
    }

    /// The window's events, and the older ones it parsed (oldest first).
    fn parts(self) -> (Vec<ChatEvent>, Vec<ChatEvent>) {
        let mut unshown = self.reach;
        unshown.extend(self.context);
        (self.events, unshown)
    }

    /// The window's events, led by the `conversation` the older lines announced last when they announce none: a
    /// parser announces it once, so it is usually on a line before the window. It takes the first event's cursor
    /// (where following resumes when there is none), so paging from the oldest cursor shown skips no line. Then the
    /// older events, as [`Window::parts`].
    fn shown(self) -> (Vec<ChatEvent>, Vec<ChatEvent>) {
        let announces = |event: &&ChatEvent| matches!(event.body, EventBody::Conversation { .. });
        let next = self.next;
        let (events, unshown) = self.parts();
        if events.iter().any(|event| announces(&event)) {
            return (events, unshown);
        }
        let Some(mut conversation) = unshown.iter().rev().find(announces).cloned() else {
            return (events, unshown);
        };
        conversation.cursor = events.first().map_or(next, |event| event.cursor);
        let mut shown = vec![conversation];
        shown.extend(events);
        (shown, unshown)
    }
}

/// The window of complete lines ending at or before `end`, and the parser after them.
fn read_window<R: Read + Seek>(reader: &mut R, spec: &Spec, end: u64, want: &Want) -> io::Result<(Window, Parser)> {
    let head = if spec.primed() { first_line(reader)? } else { None };
    let mut scan = BackScan::new(end);
    let mut step = want.context + STEP;
    // Where the context starts once it is read: the lines before it are read only for `want.reach`.
    let mut context_from = None;
    loop {
        scan.read(reader, step)?;
        let ready = scan.next().is_some() && scan.read_bytes() >= want.context;
        if ready || scan.done() {
            let mut parser = spec.parser();
            let head = match head.as_ref().filter(|_| !scan.done()) {
                Some(head) => parser.parse(head),
                None => Vec::new(),
            };
            let parsed: Vec<Vec<ChatEvent>> = scan.lines().map(|line| parser.parse(line)).collect();
            if let Some(mut window) = cut(parsed, &scan, want, context_from) {
                let reaching = want.reach.as_ref().is_some_and(|approval| !window.holds(approval));
                if !reaching || scan.done() || scan.read_bytes() >= REACH {
                    window.context.splice(0..0, head);
                    return Ok((window, parser));
                }
                context_from.get_or_insert(window.oldest);
            }
        }
        step = scan.read_bytes().max(STEP);
    }
}

/// Splits the lines' events (oldest first) into context and window: the window is the fewest newest lines making
/// `want.limit` events and holding every line since `want.since`, with `want.context` bytes of lines before it. The
/// lines before `context_from` are the reach's. `None` while the scan has not read enough.
fn cut(mut parsed: Vec<Vec<ChatEvent>>, scan: &BackScan, want: &Want, context_from: Option<u64>) -> Option<Window> {
    let lines: Vec<&RawLine> = scan.lines().collect();
    let mut start = parsed.len();
    let mut count = 0;
    while start > 0 && count < want.limit {
        start -= 1;
        count += parsed[start].len();
    }
    let (since_start, since_found) = want
        .since
        .as_deref()
        .map_or((start, true), |since| since_start(&lines, since));
    let start = start.min(since_start);
    let next = scan.next().unwrap_or(0);
    let window_start = lines.get(start).map_or(next, |line| line.cursor);
    let oldest = lines.first().map_or(window_start, |line| line.cursor);
    if !scan.done() && (count < want.limit || !since_found || window_start - oldest < want.context) {
        return None;
    }
    let reach = context_from.map_or(0, |from| lines[..start].partition_point(|line| line.cursor < from));
    let stamp = lines[reach..start].iter().rev().find_map(|line| stamp(line));
    let events = parsed.split_off(start).into_iter().flatten().collect();
    let context = parsed.split_off(reach).into_iter().flatten().collect();
    Some(Window {
        reach: parsed.into_iter().flatten().collect(),
        context,
        events,
        oldest,
        next,
        stamp,
    })
}

/// Where the lines (oldest first) since `since` start, and whether a line stamped before it was found: the newest
/// lines up to it, unstamped ones included.
fn since_start(lines: &[&RawLine], since: &str) -> (usize, bool) {
    let older = lines
        .iter()
        .rposition(|line| stamp(line).is_some_and(|stamp| minute(&stamp) < minute(since)));
    older.map_or((0, false), |older| (older + 1, true))
}

/// A line's `timestamp`, when it is a JSON object with one.
fn stamp(line: &RawLine) -> Option<String> {
    #[derive(Deserialize)]
    struct Stamped {
        timestamp: Option<String>,
    }
    let bytes = line.bytes.as_deref()?;
    serde_json::from_slice::<Stamped>(bytes).ok()?.timestamp
}

/// A timestamp (`2026-11-03T20:01:46.725Z`, as both agents write it) cut to its minute: lines are compared to the
/// minute, so a line written a little after its stamp is still sent again rather than lost.
fn minute(stamp: &str) -> &str {
    const MINUTE: usize = "2026-01-01T00:00".len();
    stamp.get(..MINUTE).unwrap_or(stamp)
}

/// The file's first line, when it ends within [`HEAD_CAP`] bytes.
fn first_line<R: Read + Seek>(reader: &mut R) -> io::Result<Option<RawLine>> {
    reader.seek(SeekFrom::Start(0))?;
    let mut lines = Splitter::at(0);
    let mut buf = vec![0; 16 * 1024];
    while lines.offset() < HEAD_CAP {
        let read = reader.read(&mut buf)?;
        if read == 0 {
            break;
        }
        if let Some(line) = lines.feed(&buf[..read]).into_iter().next() {
            return Ok(Some(line));
        }
    }
    Ok(None)
}

/// The first [`FINGERPRINT`] bytes of a file of `size` bytes.
fn fingerprint<R: Read + Seek>(reader: &mut R, size: u64) -> io::Result<Vec<u8>> {
    let mut head = vec![0; usize::try_from(size.min(FINGERPRINT)).unwrap_or(0)];
    reader.seek(SeekFrom::Start(0))?;
    reader.read_exact(&mut head)?;
    Ok(head)
}

/// One transcript being followed.
#[derive(Debug)]
struct Stream {
    path: PathBuf,
    spec: Spec,
    parser: Parser,
    lines: Splitter,
    /// The size read up to.
    size: u64,
    /// The file's first bytes, as last read.
    head: Vec<u8>,
}

impl Stream {
    /// A transcript not there yet: it is read from its start once it appears.
    fn absent(path: &Path, spec: Spec) -> Self {
        Self {
            path: path.to_owned(),
            parser: spec.parser(),
            spec,
            lines: Splitter::at(0),
            size: 0,
            head: Vec::new(),
        }
    }

    /// Reads the window ending at `end` (the file's end when `None`, and never past it), following after it.
    fn start<F: Files>(
        files: &F,
        path: &Path,
        spec: Spec,
        end: Option<u64>,
        want: &Want,
    ) -> io::Result<(Self, Window)> {
        let opened = files.size(path).and_then(|size| Ok((size, files.open(path)?)));
        let (size, mut reader) = match opened {
            Ok(opened) => opened,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Ok((Self::absent(path, spec), Window::default()));
            }
            Err(error) => return Err(error),
        };
        let head = fingerprint(&mut reader, size)?;
        let end = end.map_or(size, |end| end.min(size));
        let (window, parser) = read_window(&mut reader, &spec, end, want)?;
        let stream = Self {
            path: path.to_owned(),
            spec,
            parser,
            lines: Splitter::at(window.next),
            size: end,
            head,
        };
        Ok((stream, window))
    }

    /// The events of the lines written since the last read; `None` when the file shrank or was replaced.
    /// A file whose size did not change is opened only when `recheck` asks for its first bytes to be compared: a file
    /// replaced by one of the same size is caught only then.
    fn poll<F: Files>(&mut self, files: &F, recheck: bool) -> io::Result<Option<Vec<ChatEvent>>> {
        let size = match files.size(&self.path) {
            Ok(size) => size,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Some(Vec::new())),
            Err(error) => return Err(error),
        };
        if size < self.size || size < self.lines.offset() {
            return Ok(None);
        }
        if size == self.size && !recheck {
            return Ok(Some(Vec::new()));
        }
        let mut reader = match files.open(&self.path) {
            Ok(reader) => reader,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Some(Vec::new())),
            Err(error) => return Err(error),
        };
        if self.replaced(&mut reader, size)? {
            return Ok(None);
        }
        if size == self.size {
            return Ok(Some(Vec::new()));
        }
        self.size = size;
        self.read_to(&mut reader, size).map(Some)
    }

    /// Whether the file's first bytes differ from those read before; keeps the longer of the two.
    fn replaced<R: Read + Seek>(&mut self, reader: &mut R, size: u64) -> io::Result<bool> {
        let head = fingerprint(reader, size)?;
        let common = head.len().min(self.head.len());
        if head[..common] != self.head[..common] {
            return Ok(true);
        }
        if head.len() > self.head.len() {
            self.head = head;
        }
        Ok(false)
    }

    /// The events of the lines from the splitter's offset up to `end`.
    fn read_to<R: Read + Seek>(&mut self, reader: &mut R, end: u64) -> io::Result<Vec<ChatEvent>> {
        reader.seek(SeekFrom::Start(self.lines.offset()))?;
        let mut events = Vec::new();
        let mut buf = vec![0; CHUNK];
        while self.lines.offset() < end {
            let want = usize::try_from(end - self.lines.offset()).map_or(CHUNK, |left| left.min(CHUNK));
            let read = reader.read(&mut buf[..want])?;
            if read == 0 {
                break;
            }
            for line in self.lines.feed(&buf[..read]) {
                events.extend(self.parser.parse(&line));
            }
        }
        Ok(events)
    }
}
