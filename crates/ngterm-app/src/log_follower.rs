//! A log page's view of its file (upstream's `LogFollower.swift`), frontend-neutral: while the page follows and is on
//! screen its frontend ticks it once per [`LogFollower::interval`], and each tick checks the file by path, reading its
//! end only when its stamp changed. Polling by path works on network filesystems, and picks up a rotated, truncated or
//! recreated log at the next check on every OS. The follower keeps the last snapshot and the paused scroll offset for a
//! rebuilt document, and reports to the model.
//!
//! The follower does no I/O and keeps no timer itself: a tick or a document build hands back a [`LogRead`] the frontend
//! runs off the main thread ([`LogRead::run`], the injected [`Reader`]) and gives back to [`LogFollower::finish`],
//! which lands only the newest read and none across a pause. What the follower tells the page and the model goes
//! through a [`LogSink`]; whether ticks should run is [`LogFollower::wants_ticks`], which the frontend checks after
//! each call (upstream's `updateLoop`), ticking at once whenever they start.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use ngterm_core::html::log_tail::Snapshot;
use ngterm_core::html::text_page::{self, PageState, SearchState, TextLog, TextLogReport, TextLogState};

use crate::documents::{self, FileStamp};

/// Reads `path`'s last `tail` lines, `None` when the file still has the given stamp (upstream's `LogFollower.Reader`).
pub type Reader =
    Arc<dyn Fn(&Path, usize, Option<&FileStamp>) -> Result<Option<documents::LogRead>, String> + Send + Sync>;

/// What a read answered: the file's end, `None` for an unchanged file, or why it could not be read.
pub type ReadResult = Result<Option<documents::LogRead>, String>;

/// How the text a push carries relates to what the page shows (upstream's `LogFollower.Shift`): `dropped` lines left
/// its front, `cut` when the new start falls inside a line, `replaced` when the file is not the one the page's text
/// came from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Shift {
    pub dropped: usize,
    pub cut: bool,
    pub replaced: bool,
}

/// What a push hands the page's renderer (upstream's `agtermTextUpdate` arguments): the text, the report's flags, the
/// language to highlight in (`None` past [`text_page::MAX_HIGHLIGHT_BYTES`]) and the shift since the last push.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    pub text: String,
    pub report: TextLogReport,
    pub language: Option<&'static str>,
    pub shift: Shift,
}

/// Where the follower's results go: the page's renderer and the model.
pub trait LogSink {
    /// Whether the page still holds a slot: a page that left it reads nothing.
    fn in_slot(&self) -> bool;
    /// The model records what the app last read, emitting nothing (upstream's `setTextLogReport`).
    fn report(&self, report: TextLogReport);
    /// The model records a follow change the user's scroll made (upstream's `setTextLogFollowing(_:following:)`).
    fn following(&self, following: bool);
    /// The page loads `html` as its document.
    fn load(&self, html: String);
    /// The renderer shows `update` (upstream's `agtermTextUpdate`).
    fn update(&self, update: &Update);
    /// The renderer's bar follows `following` (upstream's `agtermTextSetFollowing`).
    fn set_following(&self, following: bool);
}

/// Why a read was asked for, and what it must still match when it lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Purpose {
    /// A tick: lands only while it is the newest read and no pause came between.
    Tick { sequence: u64, epoch: u64 },
    /// A document build's forced read: lands while it is the newest read; the build goes on while it is the newest.
    Build { sequence: u64, load: u64 },
}

/// A read the follower asked for: run it off the main thread, then hand it and its result to
/// [`LogFollower::finish`].
#[derive(Clone)]
pub struct LogRead {
    purpose: Purpose,
    path: String,
    tail: usize,
    unchanged_from: Option<FileStamp>,
    reader: Reader,
}

impl LogRead {
    /// Reads the file: the blocking part, for a worker thread.
    ///
    /// # Errors
    /// The reader's refusal (`unreadable text file: P`), which the follower shows as waiting for the file.
    pub fn run(&self) -> ReadResult {
        (self.reader)(Path::new(&self.path), self.tail, self.unchanged_from.as_ref())
    }
}

impl std::fmt::Debug for LogRead {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LogRead")
            .field("purpose", &self.purpose)
            .field("path", &self.path)
            .field("tail", &self.tail)
            .field("unchanged_from", &self.unchanged_from)
            .finish_non_exhaustive()
    }
}

/// How often a following page checks its file (upstream's one second).
pub const INTERVAL: Duration = Duration::from_secs(1);

/// One log page's follower (upstream's `LogFollower`).
pub struct LogFollower {
    path: String,
    tail: usize,
    /// The text page's stylesheet, which a built document wears.
    style: String,
    snapshot: Option<Snapshot>,
    report: TextLogReport,
    following: bool,
    /// The scroll offset the page last reported while paused.
    anchor: Option<f64>,
    /// Gates the ticks with `following`.
    on_screen: bool,
    reader: Reader,
    interval: Duration,
    stamp: Option<FileStamp>,
    /// What the page was last given, which the next push's shift is measured against.
    page_snapshot: Option<Snapshot>,
    page_stamp: Option<FileStamp>,
    /// Bumped by every read, so only the newest read lands.
    read_sequence: u64,
    /// Bumped by a pause, so no tick lands on the view a user scrolled into; a document build is never dropped by it.
    pause_epoch: u64,
    /// Bumped by every document build and by [`Self::stop`], so a slow build cannot replace a newer one.
    document_load: u64,
    /// Set from a document build until the renderer of that build (its `document_load`) reports ready: ticks wait and
    /// nothing is pushed, since the old document or a renderer not yet running would drop it, and the ready report
    /// delivers the latest state instead.
    document_pending: bool,
    /// The search a built document starts with (upstream's `loadDocument(search:)`).
    search: SearchState,
}

impl LogFollower {
    /// The follower of the log page at `path` asking for `log`, following as `following` says (the model's value),
    /// building its documents in `style`, reading with [`documents::load_log`] every [`INTERVAL`].
    #[must_use]
    pub fn new(path: String, log: TextLog, following: bool, style: String) -> Self {
        Self {
            path,
            tail: log.tail,
            style,
            snapshot: None,
            report: TextLogReport::default(),
            following,
            anchor: None,
            on_screen: true,
            reader: Arc::new(documents::load_log),
            interval: INTERVAL,
            stamp: None,
            page_snapshot: None,
            page_stamp: None,
            read_sequence: 0,
            pause_epoch: 0,
            document_load: 0,
            document_pending: false,
            search: SearchState::default(),
        }
    }

    /// This follower reading with `reader` instead.
    #[must_use]
    pub fn with_reader(mut self, reader: Reader) -> Self {
        self.reader = reader;
        self
    }

    /// This follower ticking every `interval` instead.
    #[must_use]
    pub fn with_interval(mut self, interval: Duration) -> Self {
        self.interval = interval;
        self
    }

    #[must_use]
    pub fn snapshot(&self) -> Option<&Snapshot> {
        self.snapshot.as_ref()
    }

    #[must_use]
    pub fn report(&self) -> TextLogReport {
        self.report
    }

    #[must_use]
    pub fn following(&self) -> bool {
        self.following
    }

    #[must_use]
    pub fn anchor(&self) -> Option<f64> {
        self.anchor
    }

    #[must_use]
    pub fn interval(&self) -> Duration {
        self.interval
    }

    /// Whether a document build waits for its renderer's ready report.
    #[must_use]
    pub fn document_pending(&self) -> bool {
        self.document_pending
    }

    /// The search the next built document starts with, so a rebuild keeps the query and the current match.
    pub fn set_search(&mut self, search: SearchState) {
        self.search = search;
    }

    /// Whether the frontend should tick the follower now: it follows and is on screen. When this turns true the first
    /// tick runs at once (coming back on screen, a resume).
    #[must_use]
    pub fn wants_ticks(&self) -> bool {
        self.following && self.on_screen
    }

    /// Whether the page is shown: off screen it reads nothing.
    pub fn set_on_screen(&mut self, on_screen: bool) {
        self.on_screen = on_screen;
    }

    /// Builds the page from a fresh read, or from the cached snapshot and anchor when `fresh` is false (upstream's
    /// `loadDocument`): an explicit reload reads the file again, a theme change redraws what is shown. A read that
    /// failed after the open was accepted shows an empty page marked waiting or unreadable, which the next good read
    /// fills. The read it needs first, if any, comes back to run; else the document goes to `sink` now.
    pub fn load_document(&mut self, fresh: bool, sink: &dyn LogSink) -> Option<LogRead> {
        self.document_load += 1;
        self.document_pending = true;
        if fresh || self.snapshot.is_none() {
            self.read_sequence += 1;
            return Some(self.read(
                Purpose::Build {
                    sequence: self.read_sequence,
                    load: self.document_load,
                },
                None,
            ));
        }
        self.build(sink);
        None
    }

    /// Takes the model's value (upstream's `setFollowing`): the page's bar follows it, a pause drops any read in
    /// flight, and a resume ticks at once (the frontend's, once [`Self::wants_ticks`] turned true).
    pub fn set_following(&mut self, following: bool, sink: &dyn LogSink) {
        if following == self.following {
            return;
        }
        // a pause from control or the model freezes the view where it is, not where an earlier scroll left it
        if !following {
            self.anchor = None;
        }
        self.apply_following(following);
        if !self.document_pending {
            sink.set_following(following);
        }
    }

    /// Takes the page's report of a user scroll (upstream's `scrolled`): its offset, and whether it now sits at the
    /// bottom. The follower takes the change before the model does, so a read already in flight cannot replace a
    /// frozen view.
    pub fn scrolled(&mut self, following: bool, top: Option<f64>, sink: &dyn LogSink) {
        self.anchor = top;
        if following == self.following {
            return;
        }
        self.apply_following(following);
        sink.following(following);
    }

    /// Takes the report of the document built as `load` that it can take updates, and hands it the latest state, which
    /// a read during the build may have changed (upstream's `rendererReady`). An older document's late report is
    /// ignored, so it cannot release ticks and pushes into a newer document still loading (a fix over upstream, whose
    /// report names no document).
    pub fn renderer_ready(&mut self, load: u64, sink: &dyn LogSink) {
        if !self.document_pending || load != self.document_load {
            return;
        }
        self.document_pending = false;
        self.push(sink);
        sink.set_following(self.following);
    }

    /// One check of the file (upstream's `tick`); the read to run, `None` while the page left its slot or its document
    /// is being rebuilt.
    pub fn tick(&mut self, sink: &dyn LogSink) -> Option<LogRead> {
        if self.document_pending || !sink.in_slot() {
            return None;
        }
        self.read_sequence += 1;
        let purpose = Purpose::Tick {
            sequence: self.read_sequence,
            epoch: self.pause_epoch,
        };
        Some(self.read(purpose, self.stamp))
    }

    /// Lands `read`'s `result` unless a newer read or a pause overtook it, then goes on with the build it was for.
    pub fn finish(&mut self, read: &LogRead, result: ReadResult, sink: &dyn LogSink) {
        match read.purpose {
            Purpose::Tick { sequence, epoch } => {
                if sequence == self.read_sequence && epoch == self.pause_epoch {
                    self.apply(result, sink);
                }
            }
            Purpose::Build { sequence, load } => {
                if sequence == self.read_sequence {
                    self.apply(result, sink);
                }
                if load == self.document_load {
                    self.build(sink);
                }
            }
        }
    }

    /// Drops every read and build in flight (upstream's `stop`); the frontend ends its ticks.
    pub fn stop(&mut self) {
        self.read_sequence += 1;
        self.pause_epoch += 1;
        self.document_load += 1;
    }

    fn read(&self, purpose: Purpose, unchanged_from: Option<FileStamp>) -> LogRead {
        LogRead {
            purpose,
            path: self.path.clone(),
            tail: self.tail,
            unchanged_from,
            reader: Arc::clone(&self.reader),
        }
    }

    fn apply_following(&mut self, following: bool) {
        self.following = following;
        if !following {
            self.pause_epoch += 1;
        }
    }

    fn apply(&mut self, result: ReadResult, sink: &dyn LogSink) {
        match result {
            Ok(None) => {}
            Ok(Some(read)) => {
                // a binary end keeps its stamp, so the next tick skips an unchanged file instead of reading it again
                self.stamp = Some(read.stamp);
                let Some(shown) = read.snapshot else {
                    let next = TextLogReport {
                        unreadable: true,
                        waiting: false,
                        ..self.report
                    };
                    let changed = next != self.report;
                    return self.publish(next, changed, sink);
                };
                let next = TextLogReport {
                    lines: shown.lines,
                    clipped: shown.clipped,
                    ..TextLogReport::default()
                };
                self.snapshot = Some(shown);
                self.publish(next, true, sink);
            }
            Err(_) => {
                self.stamp = None;
                let next = TextLogReport {
                    waiting: true,
                    unreadable: false,
                    ..self.report
                };
                let changed = next != self.report;
                self.publish(next, changed, sink);
            }
        }
    }

    fn publish(&mut self, next: TextLogReport, push: bool, sink: &dyn LogSink) {
        self.report = next;
        sink.report(next);
        if push && !self.document_pending {
            self.push(sink);
        }
    }

    /// Hands the renderer the latest text; highlighting is dropped past [`text_page::MAX_HIGHLIGHT_BYTES`], as the
    /// first document drops it.
    fn push(&mut self, sink: &dyn LogSink) {
        let text = self.snapshot.as_ref().map_or("", |shown| shown.text.as_str());
        let language = text_page::language(&self.path).filter(|_| text.len() <= text_page::MAX_HIGHLIGHT_BYTES);
        let shift = Self::shift(
            self.page_snapshot.as_ref(),
            self.page_stamp.as_ref(),
            self.snapshot.as_ref(),
            self.stamp.as_ref(),
        );
        let update = Update {
            text: text.to_owned(),
            report: self.report,
            language,
            shift,
        };
        self.page_snapshot.clone_from(&self.snapshot);
        self.page_stamp = self.stamp;
        sink.update(&update);
    }

    /// The document of what is shown, or an empty page before a good read.
    fn build(&mut self, sink: &dyn LogSink) {
        let empty = Snapshot {
            text: String::new(),
            lines: 0,
            clipped: false,
            start_offset: 0,
        };
        let shown = self.snapshot.as_ref().unwrap_or(&empty);
        let log = TextLogState {
            following: self.following,
            report: self.report,
        };
        let state = PageState {
            log: Some(&log),
            anchor: if self.following { None } else { self.anchor },
            load: Some(self.document_load),
            search: Some(&self.search),
            // a log page steps no changes
            change: None,
        };
        let html = text_page::html(&shown.text, text_page::language(&self.path), None, &self.style, &state);
        self.page_snapshot.clone_from(&self.snapshot);
        self.page_stamp = self.stamp;
        sink.load(html);
    }

    /// Compares two snapshots of one file by their start offsets, counting the line breaks in the bytes the newer one
    /// no longer shows; another file, a shrunk one or a start that moved back is a replacement (upstream's `shift`).
    #[must_use]
    pub fn shift(
        old: Option<&Snapshot>,
        old_stamp: Option<&FileStamp>,
        new: Option<&Snapshot>,
        new_stamp: Option<&FileStamp>,
    ) -> Shift {
        let (Some(old), Some(new)) = (old, new) else {
            return Shift::default();
        };
        if old == new && old_stamp == new_stamp {
            return Shift::default();
        }
        let same_file = matches!((old_stamp, new_stamp), (Some(before), Some(after))
            if before.identity == after.identity && after.size >= before.size);
        if !same_file || new.start_offset < old.start_offset {
            return Shift {
                replaced: true,
                ..Shift::default()
            };
        }
        let bytes = old.text.as_bytes();
        let gone = usize::try_from(new.start_offset - old.start_offset)
            .unwrap_or(usize::MAX)
            .min(bytes.len());
        if gone >= bytes.len() {
            let unterminated = bytes.last().is_some_and(|last| *last != b'\n' && *last != b'\r');
            return Shift {
                dropped: line_breaks(bytes, bytes.len()) + usize::from(unterminated),
                ..Shift::default()
            };
        }
        Shift {
            dropped: line_breaks(bytes, gone),
            cut: gone > 0 && !break_ends(bytes, gone),
            replaced: false,
        }
    }
}

/// The line breaks in `bytes[..end]`: LF, CRLF and a lone CR are each one, as the page counts lines once it normalizes
/// them.
fn line_breaks(bytes: &[u8], end: usize) -> usize {
    (0..end)
        .filter(|&index| bytes[index] == b'\n' || lone_cr(bytes, index))
        .count()
}

fn break_ends(bytes: &[u8], index: usize) -> bool {
    bytes[index - 1] == b'\n' || lone_cr(bytes, index - 1)
}

fn lone_cr(bytes: &[u8], index: usize) -> bool {
    bytes[index] == b'\r' && bytes.get(index + 1) != Some(&b'\n')
}

#[cfg(test)]
#[path = "log_follower/tests.rs"]
mod tests;
