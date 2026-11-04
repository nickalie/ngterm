//! Following an agent's transcript for `ngtermctl agent chat`: the backlog read backward from the end, an older page,
//! everything after a cursor, then new lines as the agent writes them.
//!
//! [`Follower`] reads through [`Files`] (a file opened as `Read + Seek`, and a size probe) and waits through
//! [`Clock`], so the same code runs on every OS, and the tests count the bytes it reads and drive its time.
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
//! - **Following** ([`Follower::poll`]): each poll probes the size; growth is read and split into lines, a partial last
//!   line buffered until its newline. A file that shrinks, or whose first bytes change (it was replaced), starts over
//!   with a `reset` and a new backlog. A line over [`LINE_CAP`] becomes `unknown{oversized}` and is never held whole.
//!   A file that does not exist yet (Claude writes its transcript at the first prompt) is read once it appears.
//! - **Subagents** (`subagents`): their transcripts are followed as extra sources, their events under the task tool.
//!   A subagent whose task is among the lines a window parsed is shown with a backlog of its own, one found while
//!   following likewise; the others are followed from their end. A subagent event's `cursor` is an offset in its own
//!   file: paging goes by the events without `parent`.
//!
//! A backlog and the events after a cursor end with a `live` event whose cursor is where following resumes. Following
//! polls every [`POLL_INTERVAL`] by default, and looks for new subagents at most every [`DISCOVERY_INTERVAL`].

mod lines;
mod subagents;

use std::collections::HashSet;
use std::fs::File;
use std::io::{self, ErrorKind, Read, Seek, SeekFrom};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use lines::{BackScan, RawLine, Splitter};

#[cfg(doc)]
use super::event::LINE_CAP;
use super::event::OVERSIZED;
use super::{AgentKind, ChatEvent, ClaudeParser, CodexParser, EventBody, TranscriptParser, unknown};

/// The events a backlog holds by default: the phone's first screenful and some.
pub const BACKLOG: usize = 200;

/// How often following probes the transcript by default.
pub const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// How often following looks for new subagents at most.
pub const DISCOVERY_INTERVAL: Duration = Duration::from_secs(1);

/// The bytes of older lines a window parses for pairing and does not show.
pub const CONTEXT: u64 = 64 * 1024;

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

/// The time a follower polls by.
pub trait Clock {
    fn now(&self) -> Instant;
    fn sleep(&self, duration: Duration);
}

/// The system's clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

/// Reads one conversation's transcript, and its subagents', as chat events.
#[derive(Debug)]
pub struct Follower<F: Files = Disk, C: Clock = SystemClock> {
    files: F,
    clock: C,
    interval: Duration,
    agent: AgentKind,
    path: PathBuf,
    /// The backlog size, kept for the backlog after a `reset`.
    limit: usize,
    main: Stream,
    subagents: Vec<Stream>,
    /// The subagents followed, by [`subagents::Found::key`].
    known: HashSet<String>,
    polled_at: Option<Instant>,
    discovered_at: Option<Instant>,
}

impl Follower {
    /// A follower of `agent`'s transcript at `path` on disk.
    pub fn new(agent: AgentKind, path: impl Into<PathBuf>) -> Self {
        Self::with(Disk, SystemClock, agent, path)
    }
}

impl<F: Files, C: Clock> Follower<F, C> {
    /// A follower reading through `files` and waiting by `clock`. Nothing is read until the first call.
    pub fn with(files: F, clock: C, agent: AgentKind, path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        Self {
            files,
            clock,
            interval: POLL_INTERVAL,
            agent,
            limit: BACKLOG,
            main: Stream::absent(&path, Spec::main(agent)),
            path,
            subagents: Vec::new(),
            known: HashSet::new(),
            polled_at: None,
            discovered_at: None,
        }
    }

    /// Polls every `interval` instead of [`POLL_INTERVAL`].
    #[must_use]
    pub fn interval(mut self, interval: Duration) -> Self {
        self.interval = interval;
        self
    }

    /// The events of about the last `limit` events' lines, the backlogs of the subagents among them, then `live`;
    /// following resumes after them.
    ///
    /// # Errors
    /// The transcript cannot be read (a missing one is not an error: it is read once it appears).
    pub fn backlog(&mut self, limit: usize) -> io::Result<Vec<ChatEvent>> {
        self.limit = limit;
        self.forget_subagents();
        let (main, window) = Stream::start(
            &self.files,
            &self.path,
            Spec::main(self.agent),
            None,
            Want::backlog(limit),
        )?;
        self.main = main;
        let shown = window.ids();
        let mut events = window.events;
        self.add_subagents(&|parent| shown.contains(parent), &mut events);
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
        let (main, window) = Stream::start(&self.files, &self.path, spec, Some(cursor), Want::backlog(limit))?;
        let shown = window.ids();
        let mut events = window.events;
        let found = subagents::find(self.agent, &self.path, &main.parser.spawned(), &HashSet::new());
        for found in found.into_iter().filter(|found| shown.contains(&found.parent)) {
            let spec = Spec::subagent(self.agent, found.parent);
            if let Ok((_, window)) = Stream::start(&self.files, &found.path, spec, None, Want::backlog(limit)) {
                events.extend(window.events);
            }
        }
        Ok(events)
    }

    /// Everything from the line at `cursor` on, the backlogs of the subagents among it, then `live`; following resumes
    /// after it. A cursor past the end is a `reset` and a backlog.
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
        let want = Want {
            limit: 0,
            context: CONTEXT,
        };
        let (main, window) = Stream::start(&self.files, &self.path, Spec::main(self.agent), Some(cursor), want)?;
        self.main = main;
        let Some(mut events) = self.main.poll(&self.files)? else {
            return self.restart();
        };
        let mut shown = window.ids();
        shown.extend(events.iter().map(|event| event.id.clone()));
        self.add_subagents(&|parent| shown.contains(parent), &mut events);
        events.push(marker(EventBody::Live, self.main.lines.start()));
        Ok(events)
    }

    /// The events the transcripts' new lines make since the last read. A shrunk or replaced transcript makes a
    /// `reset`, a new backlog and `live`.
    ///
    /// # Errors
    /// The transcript cannot be read. A subagent's transcript that cannot be read is skipped.
    pub fn poll(&mut self) -> io::Result<Vec<ChatEvent>> {
        let now = self.clock.now();
        self.polled_at = Some(now);
        let Some(mut events) = self.main.poll(&self.files)? else {
            return self.restart();
        };
        for stream in &mut self.subagents {
            match stream.poll(&self.files) {
                Ok(Some(new)) => events.extend(new),
                Ok(None) => {
                    let spec = stream.spec.clone();
                    let want = Want::backlog(self.limit);
                    if let Ok((fresh, window)) = Stream::start(&self.files, &stream.path, spec, None, want) {
                        *stream = fresh;
                        events.extend(window.events);
                    }
                }
                Err(_) => {}
            }
        }
        if self
            .discovered_at
            .is_none_or(|at| now.saturating_duration_since(at) >= DISCOVERY_INTERVAL)
        {
            self.add_subagents(&|_| true, &mut events);
        }
        Ok(events)
    }

    /// Sleeps until the next poll is due.
    pub fn wait(&self) {
        let now = self.clock.now();
        let due = self.polled_at.map_or(now, |at| at + self.interval);
        let left = due.saturating_duration_since(now);
        if !left.is_zero() {
            self.clock.sleep(left);
        }
    }

    /// Polls and hands each poll's events (none, often) to `emit` until it breaks.
    ///
    /// # Errors
    /// A poll's, or `emit`'s.
    pub fn follow(&mut self, mut emit: impl FnMut(Vec<ChatEvent>) -> io::Result<ControlFlow<()>>) -> io::Result<()> {
        loop {
            let events = self.poll()?;
            if emit(events)?.is_break() {
                return Ok(());
            }
            self.wait();
        }
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

    /// Follows the subagents not followed yet: those whose task `show` names with a backlog, added to `events`, the
    /// others from their end.
    fn add_subagents(&mut self, show: &dyn Fn(&str) -> bool, events: &mut Vec<ChatEvent>) {
        self.discovered_at = Some(self.clock.now());
        for found in subagents::find(self.agent, &self.path, &self.main.parser.spawned(), &self.known) {
            let want = if show(&found.parent) {
                Want::backlog(self.limit)
            } else {
                Want::END
            };
            let spec = Spec::subagent(self.agent, found.parent);
            let Ok((stream, window)) = Stream::start(&self.files, &found.path, spec, None, want) else {
                continue;
            };
            events.extend(window.events);
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

/// What a window asks for: about `limit` events, and `context` bytes of older lines parsed before them.
#[derive(Debug, Clone, Copy)]
struct Want {
    limit: usize,
    context: u64,
}

impl Want {
    /// No events and no context: only where following resumes.
    const END: Self = Self { limit: 0, context: 0 };

    fn backlog(limit: usize) -> Self {
        Self {
            limit,
            context: CONTEXT,
        }
    }
}

/// Which parser a transcript takes.
#[derive(Debug, Clone)]
struct Spec {
    agent: AgentKind,
    /// The task tool's id, for a subagent's transcript.
    parent: Option<String>,
}

impl Spec {
    fn main(agent: AgentKind) -> Self {
        Self { agent, parent: None }
    }

    fn subagent(agent: AgentKind, parent: String) -> Self {
        Self {
            agent,
            parent: Some(parent),
        }
    }

    fn parser(&self) -> Parser {
        let kind = match (self.agent, &self.parent) {
            (AgentKind::Claude, None) => Kind::Claude(ClaudeParser::new()),
            (AgentKind::Claude, Some(parent)) => Kind::Claude(ClaudeParser::subagent(parent.clone())),
            (AgentKind::Codex, None) => Kind::Codex(CodexParser::new()),
            (AgentKind::Codex, Some(parent)) => Kind::Codex(CodexParser::subagent(parent.clone())),
        };
        Parser {
            kind,
            parent: self.parent.clone(),
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

/// What a window read: the events of its newest lines, those of the older lines it parsed for context, and where
/// following resumes.
#[derive(Debug, Default)]
struct Window {
    context: Vec<ChatEvent>,
    events: Vec<ChatEvent>,
    next: u64,
}

impl Window {
    /// The ids of every event parsed, context included.
    fn ids(&self) -> HashSet<String> {
        self.context
            .iter()
            .chain(&self.events)
            .map(|event| event.id.clone())
            .collect()
    }
}

/// The window of complete lines ending at or before `end`, and the parser after them.
fn read_window<R: Read + Seek>(reader: &mut R, spec: &Spec, end: u64, want: Want) -> io::Result<(Window, Parser)> {
    let head = if spec.primed() { first_line(reader)? } else { None };
    let mut scan = BackScan::new(end);
    let mut step = want.context + STEP;
    loop {
        scan.read(reader, step)?;
        let ready = scan.next().is_some() && scan.read_bytes() >= want.context;
        if ready || scan.done() {
            let mut parser = spec.parser();
            if let Some(head) = head.as_ref().filter(|_| !scan.done()) {
                parser.parse(head);
            }
            let parsed: Vec<Vec<ChatEvent>> = scan.lines().map(|line| parser.parse(line)).collect();
            if let Some(window) = cut(parsed, &scan, want) {
                return Ok((window, parser));
            }
        }
        step = scan.read_bytes().max(STEP);
    }
}

/// Splits the lines' events (oldest first) into context and window: the window is the fewest newest lines making
/// `want.limit` events, with `want.context` bytes of lines before it. `None` while the scan has not read enough.
fn cut(mut parsed: Vec<Vec<ChatEvent>>, scan: &BackScan, want: Want) -> Option<Window> {
    let cursors: Vec<u64> = scan.lines().map(|line| line.cursor).collect();
    let mut start = parsed.len();
    let mut count = 0;
    while start > 0 && count < want.limit {
        start -= 1;
        count += parsed[start].len();
    }
    let next = scan.next().unwrap_or(0);
    let window_start = cursors.get(start).copied().unwrap_or(next);
    let oldest = cursors.first().copied().unwrap_or(window_start);
    if !scan.done() && (count < want.limit || window_start - oldest < want.context) {
        return None;
    }
    let events = parsed.split_off(start).into_iter().flatten().collect();
    Some(Window {
        context: parsed.into_iter().flatten().collect(),
        events,
        next,
    })
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
    fn start<F: Files>(files: &F, path: &Path, spec: Spec, end: Option<u64>, want: Want) -> io::Result<(Self, Window)> {
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
    fn poll<F: Files>(&mut self, files: &F) -> io::Result<Option<Vec<ChatEvent>>> {
        let size = match files.size(&self.path) {
            Ok(size) => size,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Some(Vec::new())),
            Err(error) => return Err(error),
        };
        if size == self.size {
            return Ok(Some(Vec::new()));
        }
        if size < self.size || size < self.lines.offset() {
            return Ok(None);
        }
        let mut reader = match files.open(&self.path) {
            Ok(reader) => reader,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Some(Vec::new())),
            Err(error) => return Err(error),
        };
        if self.replaced(&mut reader, size)? {
            return Ok(None);
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
