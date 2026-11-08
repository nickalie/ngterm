//! Upstream's `LogFollowerTests.swift`, over a temp dir and a recording sink: a frontend's tick is [`tick`] (the read
//! run in place), a read held in flight is one asked for and finished later.

use std::cell::{Cell, RefCell};
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

/// What the follower told the page and the model.
#[derive(Default)]
struct Sink {
    gone: Cell<bool>,
    report: Cell<Option<TextLogReport>>,
    following: Cell<Option<bool>>,
    loads: RefCell<Vec<String>>,
    updates: RefCell<Vec<Update>>,
    bar: RefCell<Vec<bool>>,
}

impl LogSink for Sink {
    fn in_slot(&self) -> bool {
        !self.gone.get()
    }
    fn report(&self, report: TextLogReport) {
        self.report.set(Some(report));
    }
    fn following(&self, following: bool) {
        self.following.set(Some(following));
    }
    fn load(&self, html: String) {
        self.loads.borrow_mut().push(html);
    }
    fn update(&self, update: &Update) {
        self.updates.borrow_mut().push(update.clone());
    }
    fn set_following(&self, following: bool) {
        self.bar.borrow_mut().push(following);
    }
}

struct Log {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

impl Log {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.log");
        let log = Self { _dir: dir, path };
        log.write("a\n");
        log
    }

    fn write(&self, text: &str) {
        fs::write(&self.path, text).unwrap();
    }

    fn append(&self, text: &str) {
        let mut file = OpenOptions::new().append(true).open(&self.path).unwrap();
        file.write_all(text.as_bytes()).unwrap();
    }

    fn follower(&self, following: bool) -> LogFollower {
        LogFollower::new(
            self.path.to_string_lossy().into_owned(),
            TextLog {
                tail: 50,
                follow: following,
            },
            following,
            String::new(),
        )
    }

    /// A follower whose reader counts the reads that found the file unchanged.
    fn counted(&self, skipped: &Arc<AtomicUsize>) -> LogFollower {
        let skipped = Arc::clone(skipped);
        self.follower(false).with_reader(Arc::new(move |path, tail, stamp| {
            let result = documents::load_log(path, tail, stamp);
            if matches!(result, Ok(None)) {
                skipped.fetch_add(1, Ordering::SeqCst);
            }
            result
        }))
    }
}

/// One tick, its read run at once.
fn tick(follower: &mut LogFollower, sink: &Sink) {
    if let Some(read) = follower.tick(sink) {
        let result = read.run();
        follower.finish(&read, result, sink);
    }
}

/// The build generation `html`, a built document, carries for its renderer's ready report.
fn load_of(html: &str) -> u64 {
    let (_, rest) = html
        .split_once(" data-load=\"")
        .expect("a built document carries its load");
    rest.split_once('"').unwrap().0.parse().unwrap()
}

/// The renderer of the last document `sink` loaded reports ready.
fn ready(follower: &mut LogFollower, sink: &Sink) {
    let load = load_of(sink.loads.borrow().last().expect("a loaded document"));
    follower.renderer_ready(load, sink);
}

fn text(follower: &LogFollower) -> Option<&str> {
    follower.snapshot().map(|shown| shown.text.as_str())
}

fn report(lines: usize, waiting: bool, unreadable: bool) -> TextLogReport {
    TextLogReport {
        lines,
        clipped: false,
        waiting,
        unreadable,
    }
}

#[test]
fn an_append_is_picked_up_and_reported() {
    let (log, sink) = (Log::new(), Sink::default());
    let mut follower = log.follower(false);
    tick(&mut follower, &sink);
    log.append("b\n");

    tick(&mut follower, &sink);

    assert_eq!(text(&follower), Some("a\nb\n"));
    assert_eq!(sink.report.get(), Some(report(2, false, false)));
    let updates = sink.updates.borrow();
    assert_eq!(updates.last().map(|update| update.text.as_str()), Some("a\nb\n"));
    assert_eq!(updates.last().map(|update| update.shift), Some(Shift::default()));
}

#[test]
fn a_copy_truncated_log_shows_what_follows_the_truncation() {
    let (log, sink) = (Log::new(), Sink::default());
    let mut follower = log.follower(false);
    log.write("a\nb\nc\n");
    tick(&mut follower, &sink);
    let file = OpenOptions::new().write(true).open(&log.path).unwrap();
    file.set_len(0).unwrap();
    drop(file);
    log.append("d\n");

    tick(&mut follower, &sink);

    assert_eq!(text(&follower), Some("d\n"));
    assert!(
        sink.updates.borrow().last().unwrap().shift.replaced,
        "a shrunk file is another text"
    );
}

#[test]
fn a_rotated_log_is_followed_by_its_new_file() {
    let (log, sink) = (Log::new(), Sink::default());
    let mut follower = log.follower(false);
    tick(&mut follower, &sink);
    fs::rename(&log.path, log.path.with_extension("log.1")).unwrap();
    log.write("fresh\n");

    tick(&mut follower, &sink);

    assert_eq!(text(&follower), Some("fresh\n"));
}

#[test]
fn a_missing_file_waits_keeping_the_last_text_until_it_reappears() {
    let (log, sink) = (Log::new(), Sink::default());
    let mut follower = log.follower(false);
    tick(&mut follower, &sink);
    fs::remove_file(&log.path).unwrap();

    tick(&mut follower, &sink);
    assert_eq!(text(&follower), Some("a\n"));
    assert_eq!(sink.report.get(), Some(report(1, true, false)));

    log.write("back\n");
    tick(&mut follower, &sink);
    assert_eq!(text(&follower), Some("back\n"));
    assert_eq!(sink.report.get(), Some(report(1, false, false)));
}

#[test]
fn binary_data_keeps_the_last_text_and_marks_it_unreadable() {
    let (log, sink) = (Log::new(), Sink::default());
    let mut follower = log.follower(false);
    tick(&mut follower, &sink);
    log.append("b\0\n");

    tick(&mut follower, &sink);

    assert_eq!(text(&follower), Some("a\n"));
    assert_eq!(sink.report.get(), Some(report(1, false, true)));
}

#[test]
fn a_pause_drops_a_read_in_flight() {
    let (log, sink) = (Log::new(), Sink::default());
    let mut follower = log.follower(true);
    log.append("b\n");
    let read = follower.tick(&sink).unwrap();

    follower.set_following(false, &sink);
    let result = read.run();
    follower.finish(&read, result, &sink);

    assert_eq!(text(&follower), None);
    assert_eq!(*sink.bar.borrow(), vec![false]);
}

#[test]
fn a_user_scroll_pause_drops_a_read_in_flight_before_the_model_comes_back() {
    let (log, sink) = (Log::new(), Sink::default());
    let mut follower = log.follower(true);
    log.append("b\n");
    let read = follower.tick(&sink).unwrap();

    follower.scrolled(false, Some(80.0), &sink);
    let result = read.run();
    follower.finish(&read, result, &sink);

    assert_eq!(text(&follower), None);
    assert!(!follower.following());
    assert_eq!(sink.following.get(), Some(false));
}

#[test]
fn an_unchanged_file_is_checked_without_reading_it() {
    let (log, sink) = (Log::new(), Sink::default());
    let skipped = Arc::new(AtomicUsize::new(0));
    let mut follower = log.counted(&skipped);
    tick(&mut follower, &sink);

    tick(&mut follower, &sink);
    tick(&mut follower, &sink);

    assert_eq!(skipped.load(Ordering::SeqCst), 2);
    assert_eq!(text(&follower), Some("a\n"));
    assert_eq!(sink.updates.borrow().len(), 1, "an unchanged file pushes nothing");
}

/// Upstream's regression: a log whose end turned binary lost its stamp and was read in full on every tick.
#[test]
fn an_unchanged_binary_end_is_checked_without_reading_it() {
    let (log, sink) = (Log::new(), Sink::default());
    let skipped = Arc::new(AtomicUsize::new(0));
    let mut follower = log.counted(&skipped);
    tick(&mut follower, &sink);
    log.append("b\0\n");
    tick(&mut follower, &sink);

    tick(&mut follower, &sink);

    assert_eq!(skipped.load(Ordering::SeqCst), 1);
    assert_eq!(text(&follower), Some("a\n"));
    assert_eq!(sink.report.get(), Some(report(1, false, true)));
}

/// Upstream's regression: a pause from control kept the offset of an earlier user scroll, so a rebuild jumped there.
#[test]
fn a_pause_from_the_model_drops_an_earlier_scroll_offset() {
    let (log, sink) = (Log::new(), Sink::default());
    let mut follower = log.follower(true);
    follower.scrolled(false, Some(120.0), &sink);
    follower.scrolled(true, Some(900.0), &sink);

    follower.set_following(false, &sink);

    assert_eq!(follower.anchor(), None);
}

#[test]
fn a_pause_by_scroll_keeps_its_offset_when_the_model_echoes_it() {
    let (log, sink) = (Log::new(), Sink::default());
    let mut follower = log.follower(true);

    follower.scrolled(false, Some(320.0), &sink);
    follower.set_following(false, &sink);

    assert_eq!(follower.anchor(), Some(320.0));
    assert!(
        sink.bar.borrow().is_empty(),
        "the page made the change, so nothing goes back to it"
    );
}

#[test]
fn a_rebuild_holds_ticks_until_the_renderer_is_ready() {
    let (log, sink) = (Log::new(), Sink::default());
    let mut follower = log.follower(false);
    tick(&mut follower, &sink);
    assert!(
        follower.load_document(false, &sink).is_none(),
        "a theme rebuild reuses the snapshot"
    );
    assert_eq!(sink.loads.borrow().len(), 1);
    log.append("b\n");

    tick(&mut follower, &sink);
    assert_eq!(text(&follower), Some("a\n"));

    ready(&mut follower, &sink);
    tick(&mut follower, &sink);
    assert_eq!(text(&follower), Some("a\nb\n"));
}

/// A rebuild that starts while the previous document still loads waits for its own document's ready report: the older
/// one's late report releases nothing, so no push goes into the loading document (a fix over upstream).
#[test]
fn only_the_newest_document_ready_releases_a_rebuild() {
    let (log, sink) = (Log::new(), Sink::default());
    let mut follower = log.follower(true);
    let read = follower.load_document(true, &sink).unwrap();
    let result = read.run();
    follower.finish(&read, result, &sink);
    let first = load_of(&sink.loads.borrow()[0]);

    assert!(follower.load_document(false, &sink).is_none());
    let second = load_of(&sink.loads.borrow()[1]);
    assert_ne!(first, second);
    log.append("b\n");

    follower.renderer_ready(first, &sink);
    assert!(
        follower.tick(&sink).is_none(),
        "the rebuild still waits for its renderer"
    );
    assert!(sink.updates.borrow().is_empty() && sink.bar.borrow().is_empty());

    follower.renderer_ready(second, &sink);
    assert_eq!(sink.updates.borrow().len(), 1);
    assert_eq!(*sink.bar.borrow(), [true]);
    tick(&mut follower, &sink);
    assert_eq!(text(&follower), Some("a\nb\n"));
}

#[test]
fn follow_off_before_the_first_document_still_builds_it() {
    let (log, sink) = (Log::new(), Sink::default());
    let mut follower = log.follower(true);

    let read = follower.load_document(true, &sink).unwrap();
    follower.set_following(false, &sink);
    let result = read.run();
    follower.finish(&read, result, &sink);

    assert_eq!(text(&follower), Some("a\n"));
    let loads = sink.loads.borrow();
    assert_eq!(loads.len(), 1);
    assert!(loads[0].contains("data-log data-following=\"false\""), "{}", loads[0]);
    assert!(
        sink.bar.borrow().is_empty(),
        "a pending document gets the state when it is ready"
    );
}

/// A theme rebuild reuses the cached snapshot and the paused anchor; an explicit reload reads afresh.
#[test]
fn a_rebuild_reuses_the_snapshot_and_anchor_and_a_reload_reads_afresh() {
    let (log, sink) = (Log::new(), Sink::default());
    let mut follower = log.follower(true);
    let read = follower.load_document(true, &sink).unwrap();
    let result = read.run();
    follower.finish(&read, result, &sink);
    ready(&mut follower, &sink);
    follower.scrolled(false, Some(42.0), &sink);
    log.append("b\n");

    assert!(follower.load_document(false, &sink).is_none());
    let rebuilt = sink.loads.borrow().last().unwrap().clone();
    assert!(
        rebuilt.contains(">a\n</div>") && rebuilt.contains("data-anchor=\"42.0\""),
        "{rebuilt}"
    );

    let read = follower.load_document(true, &sink).unwrap();
    let result = read.run();
    follower.finish(&read, result, &sink);
    assert!(sink.loads.borrow().last().unwrap().contains(">a\nb\n</div>"));
}

/// A newer build overtakes a slow one: only the newest document loads.
#[test]
fn a_slow_build_never_replaces_a_newer_one() {
    let (log, sink) = (Log::new(), Sink::default());
    let mut follower = log.follower(false);
    let slow = follower.load_document(true, &sink).unwrap();
    let newer = follower.load_document(true, &sink).unwrap();

    let result = slow.run();
    follower.finish(&slow, result, &sink);
    assert!(sink.loads.borrow().is_empty());
    let result = newer.run();
    follower.finish(&newer, result, &sink);
    assert_eq!(sink.loads.borrow().len(), 1);
}

/// A read that failed after the open was accepted shows an empty page marked waiting.
#[test]
fn a_failed_first_read_builds_an_empty_waiting_page() {
    let (log, sink) = (Log::new(), Sink::default());
    let mut follower = log.follower(false);
    fs::remove_file(&log.path).unwrap();

    let read = follower.load_document(true, &sink).unwrap();
    let result = read.run();
    follower.finish(&read, result, &sink);

    let loads = sink.loads.borrow();
    assert!(
        loads[0].contains("data-waiting") && loads[0].contains("></div>"),
        "{}",
        loads[0]
    );
}

#[test]
fn a_page_that_left_its_slot_reads_nothing() {
    let (log, sink) = (Log::new(), Sink::default());
    let mut follower = log.follower(false);
    tick(&mut follower, &sink);
    sink.gone.set(true);
    log.append("b\n");

    assert!(follower.tick(&sink).is_none());
    assert_eq!(text(&follower), Some("a\n"));
}

#[test]
fn stop_drops_every_read_and_build_in_flight() {
    let (log, sink) = (Log::new(), Sink::default());
    let mut follower = log.follower(true);
    let read = follower.tick(&sink).unwrap();
    let build = follower.load_document(true, &sink).unwrap();

    follower.stop();
    for read in [read, build] {
        let result = read.run();
        follower.finish(&read, result, &sink);
    }

    assert_eq!(text(&follower), None);
    assert!(sink.loads.borrow().is_empty());
}

/// Ticks run while the page follows and is on screen; leaving the screen or pausing stops them, and coming back
/// starts them (the frontend ticks at once then).
#[test]
fn ticks_run_only_while_following_on_screen() {
    let (log, sink) = (Log::new(), Sink::default());
    let mut follower = log.follower(true);
    assert!(follower.wants_ticks());
    follower.set_on_screen(false);
    assert!(!follower.wants_ticks());
    follower.set_on_screen(true);
    assert!(follower.wants_ticks());
    follower.set_following(false, &sink);
    assert!(!follower.wants_ticks());
    follower.scrolled(true, None, &sink);
    assert!(follower.wants_ticks());
    assert_eq!(follower.interval(), INTERVAL);
    assert_eq!(
        follower.with_interval(Duration::from_millis(20)).interval(),
        Duration::from_millis(20)
    );
}

#[test]
fn a_scroll_that_changes_following_reaches_the_model() {
    let (log, sink) = (Log::new(), Sink::default());
    let mut follower = log.follower(true);

    follower.scrolled(false, Some(120.0), &sink);

    assert_eq!(follower.anchor(), Some(120.0));
    assert_eq!(sink.following.get(), Some(false));
}

/// A push past the highlight cap drops the language, as the first document drops it.
#[test]
fn highlighting_drops_past_the_cap() {
    let sink = Sink::default();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trace.json");
    let line = format!("{}\n", "x".repeat(99));
    fs::write(&path, line.repeat(text_page::MAX_HIGHLIGHT_BYTES / 100 + 1)).unwrap();
    let mut follower = LogFollower::new(
        path.to_string_lossy().into_owned(),
        TextLog {
            tail: text_page::MAX_TAIL,
            follow: false,
        },
        false,
        String::new(),
    );
    tick(&mut follower, &sink);
    assert_eq!(sink.updates.borrow().last().unwrap().language, None);
    fs::write(&path, "x\n").unwrap();
    tick(&mut follower, &sink);
    assert_eq!(sink.updates.borrow().last().unwrap().language, Some("json"));
}

fn stamp(identity: u64, size: u64) -> FileStamp {
    FileStamp {
        identity: (1, identity),
        size,
        modified: (i64::try_from(size).unwrap(), 0),
    }
}

fn shot(text: &str, start_offset: u64) -> Snapshot {
    Snapshot {
        text: text.to_owned(),
        lines: text.lines().count(),
        clipped: false,
        start_offset,
    }
}

fn shift(old: &Snapshot, old_size: u64, new: &Snapshot, new_size: u64) -> Shift {
    LogFollower::shift(
        Some(old),
        Some(&stamp(7, old_size)),
        Some(new),
        Some(&stamp(7, new_size)),
    )
}

fn dropped(dropped: usize, cut: bool) -> Shift {
    Shift {
        dropped,
        cut,
        replaced: false,
    }
}

#[test]
fn a_shift_counts_the_lines_that_left_the_front_of_one_file() {
    let old = shot("a\nb\nc\n", 10);
    let appended = shot("a\nb\nc\nd\n", 10);
    let evicted = shot("c\nd\ne\n", 14);
    let inside = shot("\nd\n", 15);
    let past = shot("x\n", 40);

    assert_eq!(shift(&old, 16, &old, 16), Shift::default());
    assert_eq!(shift(&old, 16, &appended, 18), Shift::default());
    assert_eq!(shift(&old, 16, &evicted, 20), dropped(2, false));
    assert_eq!(shift(&old, 16, &inside, 20), dropped(2, true));
    assert_eq!(shift(&old, 16, &past, 42), dropped(3, false));
}

#[test]
fn a_shift_counts_crlf_and_a_lone_cr_as_one_break_each() {
    let lone = shot("a\rb\nc\n", 0);
    let crlf = shot("a\r\nb\n", 0);
    let progress = shot("a\rb\rc", 0);

    assert_eq!(shift(&lone, 6, &shot("z\n", 2), 8), dropped(1, false));
    assert_eq!(shift(&crlf, 5, &shot("z\n", 3), 7), dropped(1, false));
    assert_eq!(shift(&crlf, 5, &shot("z\n", 2), 7), dropped(0, true));
    assert_eq!(shift(&progress, 5, &shot("z\n", 9), 11), dropped(3, false));
}

#[test]
fn a_shift_calls_another_file_or_a_shrunk_one_a_replacement() {
    let old = shot("a\nb\n", 10);
    let new = shot("c\n", 0);
    let later = shot("b\nc\n", 12);
    let replaced = Shift {
        replaced: true,
        ..Shift::default()
    };

    let other = LogFollower::shift(Some(&old), Some(&stamp(7, 14)), Some(&later), Some(&stamp(8, 16)));
    assert_eq!(other, replaced);
    assert_eq!(shift(&old, 14, &new, 2), replaced);
    assert_eq!(
        LogFollower::shift(Some(&old), None, Some(&later), Some(&stamp(7, 16))),
        replaced
    );
}

/// Upstream's `loadDocument(search:)`: a built log document starts with the search it was given, so a rebuild keeps the
/// query and the current match; the build waits for its renderer until that one reports ready.
#[test]
fn a_built_document_carries_its_search() {
    let (log, sink) = (Log::new(), Sink::default());
    let mut follower = log.follower(false);
    follower.set_search(SearchState {
        query: "a<".to_owned(),
        revision: 3,
        current: 1,
    });
    let read = follower.load_document(true, &sink).unwrap();
    let result = read.run();
    follower.finish(&read, result, &sink);
    assert!(follower.document_pending());
    assert!(sink.loads.borrow()[0].contains(" data-revision=\"3\" data-query=\"a&lt;\" data-current=\"1\""));
    let load = load_of(&sink.loads.borrow()[0]);
    follower.renderer_ready(load, &sink);
    assert!(!follower.document_pending());
}
