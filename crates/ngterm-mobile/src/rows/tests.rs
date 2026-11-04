//! The packed row buffer, written from real stream terminals fed known VT bytes and read back with a decoder that
//! follows the module docs' layout, as the apps' readers (Kotlin, Swift) do; and the golden fixture all are held to.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ngterm_term::{GridPoint, GridSize, Outgoing, StreamFeed, StreamOut};

use super::*;
use crate::api::{Pane, PaneInputError, PaneSlot};

const DEADLINE: Duration = Duration::from_secs(10);

/// A stream terminal at `cols` × `rows`.
struct Term {
    pane: ngterm_term::Pane,
    feed: StreamFeed,
    out: StreamOut,
}

fn size(cols: u16, rows: u16) -> GridSize {
    GridSize {
        cols,
        rows,
        cell_width: 8,
        cell_height: 16,
    }
}

impl Term {
    fn new(cols: u16, rows: u16) -> Self {
        let (pane, feed, out) =
            ngterm_term::Pane::stream(&crate::pane::config(size(cols, rows), Arc::new(|| {}))).unwrap();
        Self { pane, feed, out }
    }

    /// Feeds `bytes` and waits until the terminal has taken all of them: a cursor report asked for after them comes
    /// back.
    fn feed(&self, bytes: &[u8]) {
        assert!(self.feed.deliver(bytes));
        assert!(self.feed.deliver(b"\x1b[6n"));
        let deadline = Instant::now() + DEADLINE;
        let mut sent = Vec::new();
        while !sent.ends_with(b"R") {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "no cursor report: {sent:?}");
            if let Ok(Some(Outgoing::Bytes(bytes))) = self.out.next(Some(left)) {
                sent.extend(bytes);
            }
        }
    }

    /// A fresh frame, not a stale one.
    fn frame(&self) -> Arc<Frame> {
        let deadline = Instant::now() + DEADLINE;
        loop {
            let snapshot = self.pane.frame();
            if !snapshot.stale {
                return snapshot.frame;
            }
            assert!(Instant::now() < deadline, "the terminal stayed busy");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// A fresh frame packed since `since`, decoded.
    fn pack(&self, since: u64) -> Decoded {
        decode(&pack(&self.frame(), since, false))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DecodedRun {
    column: u16,
    span: u16,
    kind: u8,
    underline: u8,
    attrs: u16,
    foreground: u32,
    background: u32,
    underline_color: u32,
    text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DecodedRow {
    index: u16,
    flags: u8,
    runs: Vec<DecodedRun>,
}

impl DecodedRow {
    /// The run holding `text`.
    fn run(&self, text: &str) -> &DecodedRun {
        self.runs
            .iter()
            .find(|run| run.text == text)
            .unwrap_or_else(|| panic!("no run {text:?} in {:?}", self.runs))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Decoded {
    version: u16,
    flags: u16,
    seq: u64,
    cols: u16,
    rows: u16,
    foreground: u32,
    background: u32,
    cursor_color: u32,
    cursor_column: u16,
    cursor_row: u16,
    cursor_style: u8,
    cursor_flags: u8,
    changed: Vec<DecodedRow>,
    selection: Vec<(u16, u16, u16)>,
}

impl Decoded {
    fn row(&self, index: u16) -> &DecodedRow {
        self.changed
            .iter()
            .find(|row| row.index == index)
            .unwrap_or_else(|| panic!("row {index} not sent"))
    }
}

/// Reads the buffer as the module docs lay it out.
struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take(&mut self, len: usize) -> &[u8] {
        let (head, rest) = self.0.split_at(len);
        self.0 = rest;
        head
    }

    fn u8(&mut self) -> u8 {
        self.take(1)[0]
    }

    fn u16(&mut self) -> u16 {
        u16::from_le_bytes(self.take(2).try_into().unwrap())
    }

    fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.take(4).try_into().unwrap())
    }

    fn u64(&mut self) -> u64 {
        u64::from_le_bytes(self.take(8).try_into().unwrap())
    }

    fn run(&mut self) -> DecodedRun {
        let (column, span, kind, underline, attrs) = (self.u16(), self.u16(), self.u8(), self.u8(), self.u16());
        let (foreground, background, underline_color) = (self.u32(), self.u32(), self.u32());
        let len = self.u32() as usize;
        let text = String::from_utf8(self.take(len).to_vec()).unwrap();
        DecodedRun {
            column,
            span,
            kind,
            underline,
            attrs,
            foreground,
            background,
            underline_color,
            text,
        }
    }
}

fn decode(buffer: &[u8]) -> Decoded {
    let mut reader = Reader(buffer);
    assert_eq!(reader.take(4), MAGIC);
    let (version, flags, seq) = (reader.u16(), reader.u16(), reader.u64());
    let (cols, rows) = (reader.u16(), reader.u16());
    let (foreground, background, cursor_color) = (reader.u32(), reader.u32(), reader.u32());
    let (cursor_column, cursor_row, cursor_style, cursor_flags) =
        (reader.u16(), reader.u16(), reader.u8(), reader.u8());
    let count = reader.u16();
    assert_eq!(buffer.len() - reader.0.len(), HEADER_LEN);
    let changed = (0..count)
        .map(|_| {
            let (index, flags) = (reader.u16(), reader.u8());
            let runs = (0..reader.u16()).map(|_| reader.run()).collect();
            DecodedRow { index, flags, runs }
        })
        .collect();
    let selection = (0..reader.u16())
        .map(|_| (reader.u16(), reader.u16(), reader.u16()))
        .collect();
    assert!(reader.0.is_empty(), "bytes past the selection");
    Decoded {
        version,
        flags,
        seq,
        cols,
        rows,
        foreground,
        background,
        cursor_color,
        cursor_column,
        cursor_row,
        cursor_style,
        cursor_flags,
        changed,
        selection,
    }
}

fn set(color: Rgb) -> u32 {
    argb(color, 0xff)
}

#[test]
fn a_full_frame_carries_the_grid_and_the_default_colors() {
    let term = Term::new(30, 3);
    term.feed(b"text");
    let frame = term.frame();
    let decoded = decode(&pack(&frame, 0, false));
    assert_eq!(decoded.version, VERSION);
    assert_eq!((decoded.cols, decoded.rows), (30, 3));
    assert_eq!(decoded.foreground, set(frame.colors.foreground));
    assert_eq!(decoded.background, set(frame.colors.background));
    assert_eq!(decoded.flags, FRAME_FULL);
    assert_eq!(decoded.seq, frame.seq);
    assert_eq!(decoded.changed.len(), 3, "a full frame sends every row");
    assert!(decoded.row(1).runs.is_empty(), "a blank row has no runs");
}

#[test]
fn text_colors_and_attributes_come_through_in_runs() {
    let term = Term::new(30, 3);
    term.feed(b"\x1b[1;31mred\x1b[0m plain \x1b[3;4:3;38;2;10;20;30;48;2;1;2;3;58;2;4;5;6mx\x1b[0m\x1b[9;7m!\x1b[m");
    let frame = term.frame();
    let run = |column, text: &str, attrs: CellFlags, foreground, background| DecodedRun {
        column,
        span: u16::try_from(text.chars().count()).unwrap(),
        kind: 0,
        underline: 0,
        attrs: attrs.bits(),
        foreground,
        background,
        underline_color: 0,
        text: text.to_owned(),
    };
    let expected = vec![
        // a palette color, resolved; a default color is 0, taken from the header when drawn
        run(0, "red", CellFlags::BOLD, set(frame.colors.palette[1]), 0),
        run(3, " plain ", CellFlags::empty(), 0, 0),
        DecodedRun {
            underline: 3,
            underline_color: 0xff04_0506,
            ..run(10, "x", CellFlags::ITALIC, 0xff0a_141e, 0xff01_0203)
        },
        run(11, "!", CellFlags::STRIKETHROUGH | CellFlags::INVERSE, 0, 0),
    ];
    // the blank cells after the text are left out
    assert_eq!(decode(&pack(&frame, 0, false)).row(0).runs, expected);
}

#[test]
fn wide_characters_and_clusters_have_runs_of_their_own() {
    let term = Term::new(20, 2);
    term.feed("a中文b e\u{301}x".as_bytes());
    let decoded = term.pack(0);
    let row = decoded.row(0);
    assert_eq!(row.run("a").column, 0);
    let wide = row.run("中文");
    assert_eq!(
        (wide.column, wide.span, wide.kind),
        (1, 4, RUN_WIDE),
        "each covers two columns"
    );
    let narrow = row.run("b ");
    assert_eq!((narrow.column, narrow.span, narrow.kind), (5, 2, 0));
    let cluster = row.run("e\u{301}");
    assert_eq!((cluster.column, cluster.span, cluster.kind), (7, 1, RUN_CLUSTER));
    assert_eq!(row.run("x").column, 8);
}

#[test]
fn a_wide_cluster_is_one_run_of_both_kinds() {
    let term = Term::new(20, 2);
    // grapheme clustering (mode 2027) joins the skin-tone modifier to the emoji
    term.feed("\x1b[?2027ha\u{1f44d}\u{1f3fd}b".as_bytes());
    let decoded = term.pack(0);
    let row = decoded.row(0);
    let cluster = row.run("\u{1f44d}\u{1f3fd}");
    assert_eq!(
        (cluster.column, cluster.span, cluster.kind),
        (1, 2, RUN_WIDE | RUN_CLUSTER),
        "one glyph two columns wide"
    );
    assert_eq!(row.run("b").column, 3);
}

/// A row sent before OSC 10/11 changed the defaults and not sent again still draws in the new defaults: its default
/// colors are 0, resolved from the header of the frame being drawn.
#[test]
fn an_unsent_row_follows_a_change_of_the_default_colors() {
    let term = Term::new(10, 3);
    term.feed(b"kept\x1b[7mi\x1b[m\r\n\r\n");
    let first = term.pack(0);
    let kept = first.row(0).clone();
    assert_eq!((kept.run("kept").foreground, kept.run("kept").background), (0, 0));
    assert_eq!((kept.run("i").foreground, kept.run("i").background), (0, 0));

    term.feed(b"\x1b]10;#000000\x07\x1b]11;#ffffff\x07");
    let next = term.pack(first.seq);
    assert!(next.seq > first.seq, "a new frame");
    assert!(
        next.changed.iter().all(|row| row.index != 0),
        "row 0 did not change and is not sent"
    );
    assert_eq!((next.foreground, next.background), (0xff00_0000, 0xffff_ffff));
    // drawn as the docs say, the row kept from the first frame takes the new defaults
    let draw = |color: u32, default: u32| if color == 0 { default } else { color };
    let plain = kept.run("kept");
    assert_eq!(draw(plain.foreground, next.foreground), 0xff00_0000);
    assert_eq!(draw(plain.background, next.background), 0xffff_ffff);
    let inverse = kept.run("i");
    assert_eq!(
        draw(inverse.background, next.background),
        0xffff_ffff,
        "drawn as the foreground"
    );
}

#[test]
fn a_wrapped_row_says_so() {
    let term = Term::new(5, 3);
    term.feed(b"abcdefg");
    let decoded = term.pack(0);
    assert_eq!(decoded.row(0).flags, ROW_WRAPPED);
    assert_eq!(decoded.row(1).flags, 0);
    assert_eq!(decoded.row(1).run("fg").column, 0);
}

#[test]
fn the_cursor_carries_its_position_shape_and_visibility() {
    let term = Term::new(20, 4);
    term.feed(b"\x1b[3;5H");
    let decoded = term.pack(0);
    assert_eq!((decoded.cursor_column, decoded.cursor_row), (4, 2));
    assert_eq!(decoded.cursor_style, 0, "a block by default");
    assert_eq!(decoded.cursor_flags, CURSOR_IN_VIEW | CURSOR_VISIBLE | CURSOR_BLINKING);
    assert_eq!(decoded.cursor_color, 0, "no cursor color set");

    for (sequence, style, blinking) in [
        (&b"\x1b[2 q"[..], 0, false),
        (b"\x1b[3 q", 2, true),
        (b"\x1b[4 q", 2, false),
        (b"\x1b[5 q", 1, true),
        (b"\x1b[6 q", 1, false),
    ] {
        term.feed(sequence);
        let decoded = term.pack(0);
        assert_eq!(decoded.cursor_style, style, "{sequence:?}");
        assert_eq!(decoded.cursor_flags & CURSOR_BLINKING != 0, blinking, "{sequence:?}");
    }

    term.feed(b"\x1b[?25l\x1b]12;#102030\x07");
    let decoded = term.pack(0);
    assert_eq!(decoded.cursor_flags & CURSOR_VISIBLE, 0, "hidden");
    assert_eq!(decoded.cursor_color, 0xff10_2030);

    term.feed("\x1b[?25h\x1b[H中\x1b[D".as_bytes());
    let decoded = term.pack(0);
    assert_eq!(decoded.cursor_flags & CURSOR_WIDE_TAIL, CURSOR_WIDE_TAIL);
}

#[test]
fn the_selection_lists_each_row_it_crosses() {
    let term = Term::new(10, 4);
    term.feed(b"one\r\ntwo\r\nthree");
    term.pane
        .select(GridPoint::viewport(1, 0), GridPoint::viewport(2, 1), false)
        .unwrap();
    let decoded = term.pack(0);
    assert_eq!(decoded.selection, vec![(0, 1, 9), (1, 0, 2)]);

    term.pane.clear_selection().unwrap();
    assert!(term.pack(0).selection.is_empty());
}

#[test]
fn only_rows_changed_since_a_seq_are_sent() {
    let term = Term::new(10, 4);
    // the cursor is left on row 2, where the next text goes: a cursor that moves dirties the row it leaves
    term.feed(b"first\r\n\r\n");
    let first = term.pack(0);
    assert_eq!(first.changed.len(), 4);
    assert!(first.seq > 0);

    term.feed(b"third");
    let next = term.pack(first.seq);
    assert!(next.seq > first.seq);
    assert_eq!(next.flags & FRAME_FULL, 0);
    let indices: Vec<u16> = next.changed.iter().map(|row| row.index).collect();
    assert_eq!(indices, vec![2], "row 0 is unchanged");
    assert_eq!(next.row(2).run("third").column, 0);

    let unchanged = term.pack(next.seq);
    assert!(unchanged.changed.is_empty(), "nothing since the last frame drawn");

    let again = term.pack(0);
    assert_eq!(again.flags & FRAME_FULL, FRAME_FULL);
    assert_eq!(again.changed.len(), 4, "0 sends every row");
    let replaced = term.pack(next.seq + 100);
    assert_eq!(replaced.flags & FRAME_FULL, FRAME_FULL, "a seq of a replaced terminal");
    assert_eq!(replaced.changed.len(), 4);
}

#[test]
fn a_stale_frame_says_so() {
    let frame = Frame::default();
    let decoded = decode(&pack(&frame, 0, true));
    assert_eq!(decoded.flags, FRAME_FULL | FRAME_STALE);
    assert_eq!((decoded.seq, decoded.rows), (0, 0));
}

#[test]
fn the_handle_packs_its_current_terminal_and_refuses_with_none() {
    let grid = size(10, 2);
    let slot = Arc::new(PaneSlot::new(grid, Arc::new(|| {})));
    let pane = Pane::new("pane".to_owned(), Arc::clone(&slot));
    assert_eq!(pane.frame(0), Err(PaneInputError::Closed));

    let (terminal, _feed, _out) = ngterm_term::Pane::stream(&crate::pane::config(grid, Arc::new(|| {}))).unwrap();
    slot.install(Arc::new(terminal), grid);
    let decoded = decode(&pane.frame(0).unwrap());
    assert_eq!((decoded.cols, decoded.rows), (10, 2));
}

/// The golden fixture's screen: colors, attributes, a wide character, a cluster, a wide cluster, a wrapped row, a bar cursor and a
/// selection, on a 12 × 4 grid.
const GOLDEN_BYTES: &[u8] =
    "\x1b[1;31mbold\x1b[0m \x1b[4;48;2;1;2;3mul\x1b[0m\r\n中e\u{301}\x1b[7mi\x1b[0m\u{1f44d}\u{1f3fd}\r\nwrapping past the edge\x1b[5 q"
        .as_bytes();

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rows-golden.bin")
}

fn golden_buffer() -> Vec<u8> {
    let term = Term::new(12, 4);
    term.feed(GOLDEN_BYTES);
    term.pane
        .select(GridPoint::viewport(0, 0), GridPoint::viewport(3, 0), false)
        .unwrap();
    pack(&term.frame(), 0, false)
}

/// The writer still writes the fixture byte for byte. `UPDATE_GOLDEN=1` rewrites it (after a deliberate layout
/// change, with [`VERSION`] bumped and the apps' readers following).
#[test]
fn the_writer_matches_the_golden_fixture() {
    let buffer = golden_buffer();
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(golden_path(), &buffer).unwrap();
    }
    let golden = std::fs::read(golden_path()).unwrap();
    assert!(buffer == golden, "the writer's buffer differs from the golden fixture");
}

/// The fixture's header and selection decode as the module docs lay them out: the values the apps' reader tests
/// check too.
#[test]
fn the_golden_fixtures_header() {
    let golden = std::fs::read(golden_path()).unwrap();
    assert_eq!(&golden[..4], b"NGRB");
    assert_eq!(&golden[4..6], &[1, 0], "version 1, little-endian");
    let decoded = decode(&golden);
    assert_eq!((decoded.flags, decoded.seq), (FRAME_FULL, 1));
    assert_eq!((decoded.cols, decoded.rows), (12, 4));
    assert_eq!((decoded.cursor_column, decoded.cursor_row), (10, 3));
    assert_eq!(decoded.cursor_style, 1, "a bar");
    assert_eq!(decoded.cursor_flags, CURSOR_IN_VIEW | CURSOR_VISIBLE | CURSOR_BLINKING);
    assert_eq!(decoded.selection, vec![(0, 0, 3)]);
    assert_eq!(decoded.changed.len(), 4);
}

/// The fixture's rows decode to the screen [`GOLDEN_BYTES`] draws.
#[test]
fn the_golden_fixtures_rows() {
    let decoded = decode(&std::fs::read(golden_path()).unwrap());
    let summary = |index| -> Vec<(u16, u16, u8, u8, u16, u8, String)> {
        let row: &DecodedRow = decoded.row(index);
        row.runs
            .iter()
            .map(|run| {
                let alpha = u8::try_from(run.foreground >> 24).unwrap();
                (
                    run.column,
                    run.span,
                    run.kind,
                    run.underline,
                    run.attrs,
                    alpha,
                    run.text.clone(),
                )
            })
            .collect()
    };
    let bold = CellFlags::BOLD.bits();
    let inverse = CellFlags::INVERSE.bits();
    let text = str::to_owned;
    assert_eq!(
        summary(0),
        vec![
            (0, 4, 0, 0, bold, 0xff, text("bold")),
            (4, 1, 0, 0, 0, 0, text(" ")),
            (5, 2, 0, 1, 0, 0, text("ul"))
        ]
    );
    assert_eq!(decoded.row(0).run("ul").background, 0xff01_0203);
    assert_eq!(
        summary(1),
        vec![
            (0, 2, RUN_WIDE, 0, 0, 0, text("中")),
            (2, 1, RUN_CLUSTER, 0, 0, 0, text("e\u{301}")),
            (3, 1, 0, 0, inverse, 0, text("i")),
            (4, 2, RUN_WIDE | RUN_CLUSTER, 0, 0, 0, text("\u{1f44d}\u{1f3fd}")),
        ]
    );
    assert_eq!(decoded.row(2).flags, ROW_WRAPPED);
    assert_eq!(summary(2), vec![(0, 12, 0, 0, 0, 0, text("wrapping pas"))]);
    assert_eq!(summary(3), vec![(0, 10, 0, 0, 0, 0, text("t the edge"))]);
}

/// What a full-screen `cat` of a large file costs the core per frame the view draws (plan > Task 15's frame budget):
/// 2 MiB of 40–120-column lines with a color every few lines, fed in 64 KiB reads into a phone's 52 × 50 grid, a frame
/// packed after each read since the one before (every row changes). Prints the frame and pack times; run with
/// `cargo test --release -p ngterm-mobile --lib -- --ignored --nocapture packing_a_full_screen_cat`.
#[test]
#[ignore = "a profile, not a check: run it by hand with --release"]
fn packing_a_full_screen_cat() {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789 .,;:(){}";
    let term = Term::new(52, 50);
    let mut file = Vec::new();
    let mut line = 0usize;
    while file.len() < 2 << 20 {
        let width = 40 + (line * 37) % 81;
        if line.is_multiple_of(5) {
            file.extend_from_slice(format!("\x1b[3{}m{line:>6}\x1b[0m ", 1 + line % 6).as_bytes());
        }
        file.extend((0..width).map(|i| ALPHABET[(line + i * 7) % ALPHABET.len()]));
        file.extend_from_slice(b"\r\n");
        line += 1;
    }
    let (mut frame_times, mut pack_times, mut bytes) = (Vec::new(), Vec::new(), 0usize);
    let mut since = 0;
    for chunk in file.chunks(64 << 10) {
        term.feed(chunk);
        let started = Instant::now();
        let frame = term.frame();
        frame_times.push(started.elapsed());
        let packing = Instant::now();
        bytes += pack(&frame, since, false).len();
        pack_times.push(packing.elapsed());
        since = frame.seq;
    }
    let percentiles = |times: &mut Vec<Duration>| {
        times.sort();
        let at = |p: usize| times[(times.len() - 1) * p / 100];
        format!(
            "p50 {:?}, p90 {:?}, p99 {:?}, max {:?}",
            at(50),
            at(90),
            at(99),
            at(100)
        )
    };
    println!(
        "{} frames of 52 x 50, {} bytes each on average\n  frame(): {}\n  pack():  {}",
        frame_times.len(),
        bytes / frame_times.len(),
        percentiles(&mut frame_times),
        percentiles(&mut pack_times),
    );
}
