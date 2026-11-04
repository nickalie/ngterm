//! The packed row buffer: what [`crate::api::Pane::frame`] hands the app to draw a pane, the rows of the pane's
//! `ngterm-term` frame that changed since the frame the view last drew, packed into one byte array so a frame crosses
//! `UniFFI` as a single copy. Each app's reader (`RowBufferReader`: the Kotlin one on Android, the Swift one on iOS)
//! follows the layout below; the golden fixture `tests/fixtures/rows-golden.bin` keeps the writer and the readers in
//! step.
//!
//! # Layout, version 1
//!
//! Every number is little-endian and unaligned. A color is a `u32` read as `0xAARRGGBB` (Android's color `Int`). In
//! the header it is always set (alpha `0xFF`). In a run, alpha `0xFF` marks a color set on the cell, its RGB resolved
//! from the palette, and `0` (the whole `u32`) the terminal's default: draw the header's default foreground for a
//! foreground slot and its default background for a background slot, read from the frame being drawn, and swap the
//! two for inverse. A row the reader already holds is not sent again when only the defaults change (OSC 10/11), so
//! the reader resolves a `0` at draw time, never caches it. A color that can be unset (the cursor's, an underline's)
//! is `0` when unset.
//!
//! **Header** (40 bytes):
//!
//! | offset | type | field |
//! |---|---|---|
//! | 0 | `[u8; 4]` | magic, `NGRB` ([`MAGIC`]) |
//! | 4 | `u16` | version, 1 ([`VERSION`]) |
//! | 6 | `u16` | frame flags (below) |
//! | 8 | `u64` | the frame's `seq`: pass it to the next `frame` call |
//! | 16 | `u16` | columns |
//! | 18 | `u16` | rows |
//! | 20 | color | default foreground |
//! | 24 | color | default background |
//! | 28 | color | cursor color (`0`: none set, follow the cell) |
//! | 32 | `u16` | cursor column |
//! | 34 | `u16` | cursor row |
//! | 36 | `u8` | cursor style: 0 block, 1 bar, 2 underline, 3 hollow block |
//! | 37 | `u8` | cursor flags (below) |
//! | 38 | `u16` | the number of row records that follow |
//!
//! Frame flags: [`FRAME_FULL`] 1, every row of the grid follows (draw from scratch); [`FRAME_STALE`] 2, the terminal
//! was busy and this is the frame made last (ask again soon). Cursor flags: [`CURSOR_IN_VIEW`] 1, the position is on
//! screen (when clear it is 0, 0 and no cursor is drawn); [`CURSOR_VISIBLE`] 2; [`CURSOR_BLINKING`] 4;
//! [`CURSOR_PASSWORD`] 8, at a password prompt; [`CURSOR_WIDE_TAIL`] 16, on a wide character's second column.
//!
//! **Row record**, one per changed row, top to bottom: `u16` row index, `u8` row flags ([`ROW_WRAPPED`] 1: the text
//! continues on the next row), `u16` the number of runs, then the runs. A row is cleared to the default background
//! before its runs are drawn: blank cells past the last run hold nothing to draw.
//!
//! **Run**, cells of one style left to right:
//!
//! | offset | type | field |
//! |---|---|---|
//! | 0 | `u16` | first column |
//! | 2 | `u16` | columns covered |
//! | 4 | `u8` | run kind flags (below) |
//! | 5 | `u8` | underline: 0 none, 1 single, 2 double, 3 curly, 4 dotted, 5 dashed |
//! | 6 | `u16` | attribute bits (`ngterm-term`'s `CellFlags`) |
//! | 8 | color | foreground |
//! | 12 | color | background |
//! | 16 | color | underline color (`0`: the foreground) |
//! | 20 | `u32` | text length in bytes |
//! | 24 | UTF-8 | the text |
//!
//! Attribute bits: bold 1, italic 2, faint 4, blink 8, inverse 16, invisible 32, strikethrough 64, overline 128,
//! hyperlink 256. Run kind flags, which combine: [`RUN_WIDE`] 1, each glyph covers two columns; [`RUN_CLUSTER`] 2,
//! the text is one grapheme cluster, one glyph at the run's first column. Both (3) is one grapheme cluster drawn as a
//! single two-column glyph (an emoji with a skin-tone modifier or VS16, a ZWJ sequence, a flag); 0 is narrow single
//! code points.
//!
//! Outside a cluster run, each code point of the text is one glyph, one column wide (two in a wide run) after the one
//! before; a blank cell is a space. Inverse is left to the reader (swap foreground and background), as are bold and
//! faint.
//!
//! **Selection**, after the rows: `u16` the number of selected rows, then for each `u16` row, `u16` first column and
//! `u16` last column (inclusive). It lists every row the selection crosses, changed or not.
//!
//! # Changed rows
//!
//! A row is sent when it changed after the frame `since` names (`ngterm-term`'s `Frame::row_changed_since`). `since`
//! 0, or past the frame's own `seq` (the view drew a terminal since replaced), sends every row with [`FRAME_FULL`].
//! After [`crate::MobileEvent::PaneAttached`] the reader passes 0: a fresh terminal counts from the start again.

use ngterm_term::{Cell, CellFlags, CellWidth, Cursor, CursorStyle, Frame, Rgb, Row, Underline};

/// The buffer's first four bytes.
pub const MAGIC: [u8; 4] = *b"NGRB";
/// The layout's version, after [`MAGIC`].
pub const VERSION: u16 = 1;
/// The header's length in bytes.
pub const HEADER_LEN: usize = 40;

/// Frame flag: every row follows.
pub const FRAME_FULL: u16 = 1;
/// Frame flag: the terminal was busy and this is the frame made last.
pub const FRAME_STALE: u16 = 1 << 1;

/// Cursor flag: the cursor is in the viewport.
pub const CURSOR_IN_VIEW: u8 = 1;
/// Cursor flag: shown (DECTCEM).
pub const CURSOR_VISIBLE: u8 = 1 << 1;
/// Cursor flag: blinks.
pub const CURSOR_BLINKING: u8 = 1 << 2;
/// Cursor flag: at a password prompt.
pub const CURSOR_PASSWORD: u8 = 1 << 3;
/// Cursor flag: on the second column of a wide character.
pub const CURSOR_WIDE_TAIL: u8 = 1 << 4;

/// Row flag: soft-wraps into the next row.
pub const ROW_WRAPPED: u8 = 1;

/// Run kind flag: each glyph covers two columns.
pub const RUN_WIDE: u8 = 1;
/// Run kind flag: the text is one grapheme cluster (with [`RUN_WIDE`], drawn two columns wide).
pub const RUN_CLUSTER: u8 = 1 << 1;

/// Alpha of a color set on the cell.
const SET: u8 = 0xff;
/// A run's color that follows the terminal's default: the reader takes it from the header of the frame it draws.
const DEFAULT: u32 = 0;

/// Packs the rows of `frame` that changed after frame `since`, with its cursor, selection and grid; `stale` when the
/// frame is older than the terminal ([`FRAME_STALE`]).
#[must_use]
pub fn pack(frame: &Frame, since: u64, stale: bool) -> Vec<u8> {
    let full = since == 0 || since > frame.seq;
    let changed: Vec<(usize, &Row)> = frame
        .rows
        .iter()
        .enumerate()
        .filter(|&(index, _)| full || frame.row_changed_since(index, since))
        .map(|(index, row)| (index, &**row))
        .collect();
    let mut out = Writer(Vec::with_capacity(HEADER_LEN + changed.len() * 64));
    let mut flags = 0;
    if full {
        flags |= FRAME_FULL;
    }
    if stale {
        flags |= FRAME_STALE;
    }
    out.bytes(&MAGIC);
    out.u16(VERSION);
    out.u16(flags);
    out.u64(frame.seq);
    out.u16(frame.cols);
    out.u16(narrow(frame.rows.len()));
    out.u32(argb(frame.colors.foreground, SET));
    out.u32(argb(frame.colors.background, SET));
    out.u32(frame.colors.cursor.map_or(0, |color| argb(color, SET)));
    cursor(&mut out, &frame.cursor);
    out.u16(narrow(changed.len()));
    for (index, row) in changed {
        self::row(&mut out, index, row);
    }
    selection(&mut out, frame);
    out.0
}

fn cursor(out: &mut Writer, cursor: &Cursor) {
    let mut flags = 0;
    let (column, row) = cursor.position.map_or((0, 0), |position| {
        flags |= CURSOR_IN_VIEW;
        if position.wide_tail {
            flags |= CURSOR_WIDE_TAIL;
        }
        (position.column, position.row)
    });
    for (on, flag) in [
        (cursor.visible, CURSOR_VISIBLE),
        (cursor.blinking, CURSOR_BLINKING),
        (cursor.password_input, CURSOR_PASSWORD),
    ] {
        if on {
            flags |= flag;
        }
    }
    out.u16(column);
    out.u16(row);
    out.u8(match cursor.style {
        CursorStyle::Block => 0,
        CursorStyle::Bar => 1,
        CursorStyle::Underline => 2,
        CursorStyle::BlockHollow => 3,
    });
    out.u8(flags);
}

fn selection(out: &mut Writer, frame: &Frame) {
    let selected: Vec<(usize, (u16, u16))> = frame
        .rows
        .iter()
        .enumerate()
        .filter_map(|(index, row)| Some((index, row.selection()?)))
        .collect();
    out.u16(narrow(selected.len()));
    for (index, (first, last)) in selected {
        out.u16(narrow(index));
        out.u16(first);
        out.u16(last);
    }
}

fn row(out: &mut Writer, index: usize, row: &Row) {
    let runs = runs(row);
    out.u16(narrow(index));
    out.u8(if row.wrapped() { ROW_WRAPPED } else { 0 });
    out.u16(narrow(runs.len()));
    for run in runs {
        out.u16(run.column);
        out.u16(run.span);
        out.u8(run.kind);
        out.u8(run.style.underline);
        out.u16(run.style.attrs);
        out.u32(run.style.foreground);
        out.u32(run.style.background);
        out.u32(run.style.underline_color);
        out.u32(u32::try_from(run.text.len()).unwrap_or(u32::MAX));
        out.bytes(run.text.as_bytes());
    }
}

/// What a run's cells share.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Style {
    attrs: u16,
    underline: u8,
    foreground: u32,
    background: u32,
    underline_color: u32,
}

impl Style {
    /// The cell's style; a default color is [`DEFAULT`], never the default's RGB, which a later frame may change
    /// without resending the row.
    fn of(cell: &Cell) -> Self {
        let color = |set: Option<Rgb>| set.map_or(DEFAULT, |color| argb(color, SET));
        Self {
            attrs: cell.flags.bits(),
            underline: match cell.underline {
                Underline::None => 0,
                Underline::Single => 1,
                Underline::Double => 2,
                Underline::Curly => 3,
                Underline::Dotted => 4,
                Underline::Dashed => 5,
            },
            foreground: color(cell.foreground),
            background: color(cell.background),
            underline_color: cell.underline_color.map_or(0, |color| argb(color, SET)),
        }
    }
}

/// One drawn cell: a character at a column, one or two columns wide.
struct Glyph<'a> {
    column: u16,
    wide: bool,
    text: &'a str,
    style: Style,
    /// Nothing to draw: no text, the default background, no line through or under it, not inverse.
    blank: bool,
}

/// Whether a cell with no text still draws something.
fn draws_when_empty(cell: &Cell) -> bool {
    cell.background.is_some()
        || cell.underline != Underline::None
        || cell
            .flags
            .intersects(CellFlags::INVERSE | CellFlags::STRIKETHROUGH | CellFlags::OVERLINE)
}

/// The row's glyphs, left to right, with the blank ones at its end left out (the row is cleared first).
fn glyphs(row: &Row) -> Vec<Glyph<'_>> {
    let mut glyphs = Vec::with_capacity(row.cells().len());
    for (column, cell) in row.cells().iter().enumerate() {
        let wide = match cell.width {
            // covered by the wide character before it
            CellWidth::SpacerTail => continue,
            CellWidth::Wide => true,
            CellWidth::Narrow | CellWidth::SpacerHead => false,
        };
        let text = row.text(column);
        glyphs.push(Glyph {
            column: narrow(column),
            wide,
            text: if text.is_empty() { " " } else { text },
            style: Style::of(cell),
            blank: text.is_empty() && !draws_when_empty(cell),
        });
    }
    while glyphs.last().is_some_and(|glyph| glyph.blank) {
        glyphs.pop();
    }
    glyphs
}

/// A run being packed.
struct Run {
    column: u16,
    span: u16,
    kind: u8,
    style: Style,
    text: String,
}

/// The row's glyphs joined into runs: adjacent glyphs of one style and width share one, and a grapheme of more than
/// one code point is a run of its own.
fn runs(row: &Row) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    for glyph in glyphs(row) {
        let span = if glyph.wide { 2 } else { 1 };
        let cluster = glyph.text.chars().nth(1).is_some();
        let mut kind = if glyph.wide { RUN_WIDE } else { 0 };
        if let Some(run) = runs.last_mut()
            && !cluster
            && run.kind == kind
            && run.style == glyph.style
            && run.column.saturating_add(run.span) == glyph.column
        {
            run.span = run.span.saturating_add(span);
            run.text.push_str(glyph.text);
            continue;
        }
        if cluster {
            kind |= RUN_CLUSTER;
        }
        runs.push(Run {
            column: glyph.column,
            span,
            kind,
            style: glyph.style,
            text: glyph.text.to_owned(),
        });
    }
    runs
}

/// `0xAARRGGBB`.
fn argb(color: Rgb, alpha: u8) -> u32 {
    u32::from_be_bytes([alpha, color.r, color.g, color.b])
}

/// A count or index as the layout's `u16`; a grid never holds more.
fn narrow(value: usize) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

/// The buffer, little-endian.
struct Writer(Vec<u8>);

impl Writer {
    fn bytes(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
    }

    fn u8(&mut self, value: u8) {
        self.0.push(value);
    }

    fn u16(&mut self, value: u16) {
        self.bytes(&value.to_le_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.bytes(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.bytes(&value.to_le_bytes());
    }
}

#[cfg(test)]
mod tests;
