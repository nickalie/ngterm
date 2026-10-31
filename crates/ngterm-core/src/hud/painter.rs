//! The HUD panel's painter: what upstream's bundled `hud.sh` (`agterm/Resources/hud/hud.sh`) draws from the body file
//! [`super::rendered_body`] writes, as the bytes a terminal of the panel's grid is fed. The frontend runs it in process
//! on every OS (`ngterm-term`'s painted pane), re-reading the file each tick, so no shell paints the panel.
//!
//! The body (upstream's `HudLayout.renderedBody`): a `<columns> <rows> <spinner> <pid> <interval> <textcolor>
//! <blockwidth> [frame...]` header, then the lines. The grid is the panel's own, which the text is centered in; it is
//! never measured from the terminal, because the app derives it from the font and the size it gave the panel. A
//! spinner of `1` draws a frame glyph before the first line, the frame advancing every tick; `textcolor` is SGR
//! parameters (`38;2;r;g;b`) or `-`; `blockwidth` 0 is plain mode (message lines, an empty separator line, detail lines
//! dimmed), any other a markdown body of finished rows that wide, printed verbatim at one shared offset.
//!
//! Each tick builds the whole frame — reset, home, erase down, the color, the vertical pad, the block — and an
//! unchanged frame is not written again, so a spinner-less panel is finished after its first paint. Widths count code
//! points, as the app counted them (`HudLayout.cellCount`). A field the header gets wrong keeps the built-in default,
//! as the script's `case` filters keep it; the owner pid is the script's second stop for a hard-killed app, which an
//! in-process painter has no need of.

use std::fmt::Write as _;
use std::time::Duration;

/// Hides the cursor while the panel paints.
pub const START: &str = "\x1b[?25l";
/// Shows it again and resets the attributes when the painter stops (the script's exit trap).
pub const END: &str = "\x1b[?25h\x1b[0m";

const CSI: &str = "\x1b[";
/// Rows advance with CNL rather than a newline: it stops at the last line instead of scrolling, and lands on column 1
/// without depending on the terminal's newline mode.
const DOWN: &str = "\x1b[E";
const DIM: &str = "\x1b[2m";
/// The built-in box a header without a field falls back to.
const DEFAULT_COLUMNS: i64 = 40;
const DEFAULT_ROWS: i64 = 5;
const DEFAULT_INTERVAL: Duration = Duration::from_millis(500);
/// The shortest tick, so a zero interval cannot spin.
const MIN_INTERVAL: Duration = Duration::from_millis(10);

/// One tick's result: the bytes to write (`None`: the frame did not change), and how long to wait for the next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tick {
    pub output: Option<String>,
    pub interval: Duration,
}

/// The painter's state between ticks: the spinner's frame counter and the frame last written.
#[derive(Debug, Default)]
pub struct Painter {
    frame: usize,
    painted: Option<String>,
}

/// The header's fields, each the script's default when absent or malformed.
#[derive(Debug)]
struct Header<'a> {
    columns: i64,
    rows: i64,
    interval: Duration,
    foreground: Option<&'a str>,
    block_width: i64,
    glyph: Option<&'a str>,
}

impl<'a> Header<'a> {
    /// The header line, `frame` picking the spinner's glyph. The fields are split on spaces and tabs only (a frame may
    /// be a no-break space).
    fn parse(line: &'a str, frame: usize) -> Self {
        let fields: Vec<&str> = line.split([' ', '\t']).filter(|field| !field.is_empty()).collect();
        let field = |index: usize| fields.get(index).copied().unwrap_or("");
        let frames = fields.get(7..).unwrap_or_default();
        let spinning = field(2) == "1" && !frames.is_empty();
        Self {
            columns: number(field(0)).unwrap_or(DEFAULT_COLUMNS),
            rows: number(field(1)).unwrap_or(DEFAULT_ROWS),
            interval: interval(field(4)).unwrap_or(DEFAULT_INTERVAL),
            foreground: Some(field(5))
                .filter(|sgr| *sgr != "-" && !sgr.is_empty() && sgr.chars().all(|c| c.is_ascii_digit() || c == ';')),
            block_width: number(field(6)).unwrap_or(0),
            glyph: spinning.then(|| frames[frame % frames.len()]),
        }
    }

    /// The built-in box, for a body with no header line.
    fn fallback() -> Self {
        Self::parse("", 0)
    }
}

/// A field of digits only.
fn number(field: &str) -> Option<i64> {
    if field.is_empty() || !field.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    field.parse().ok()
}

/// Seconds of digits and dots.
fn interval(field: &str) -> Option<Duration> {
    if field.is_empty() || !field.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    let seconds: f64 = field.parse().ok()?;
    Some(Duration::try_from_secs_f64(seconds).ok()?.max(MIN_INTERVAL))
}

/// Code points, the unit the app measured the lines in.
fn width(text: &str) -> i64 {
    i64::try_from(text.chars().count()).unwrap_or(i64::MAX)
}

/// The rows of a block and how many lines they took.
#[derive(Debug, Default)]
struct Block {
    text: String,
    count: i64,
}

impl Block {
    /// Starts the next line: the separator before every line but the first.
    fn next_line(&mut self) {
        if self.count > 0 {
            self.text.push_str(DOWN);
        }
        self.count += 1;
    }

    /// Moves `left` columns right, when that is a move.
    fn indent(&mut self, left: i64) {
        if left > 0 {
            let _ = write!(self.text, "{CSI}{left}C");
        }
    }
}

/// The lines below the header, as the script's `read` loop sees them: split on newlines, a last line without one kept
/// when it is not empty.
fn body_lines(text: &str) -> impl Iterator<Item = &str> {
    let text = text.strip_suffix('\n').unwrap_or(text);
    text.split('\n')
}

/// The block of a markdown body: every row at the shared offset, the glyph before the first.
fn markdown_block<'a>(header: &Header<'_>, lines: impl Iterator<Item = &'a str>) -> Block {
    let mut block = Block::default();
    let mut first = true;
    for line in lines {
        block.next_line();
        let pre = match header.glyph {
            Some(glyph) if first => format!("{glyph} "),
            _ => String::new(),
        };
        first = false;
        if pre.is_empty() && line.is_empty() {
            continue;
        }
        block.indent((header.columns - header.block_width) / 2);
        block.text.push_str(&pre);
        block.text.push_str(line);
    }
    block
}

/// The block of a plain body: each line centered, the glyph before the first, every line after the empty separator
/// dimmed.
fn plain_block<'a>(header: &Header<'_>, lines: impl Iterator<Item = &'a str>) -> Block {
    let mut block = Block::default();
    let mut attribute = "";
    let mut first = true;
    for line in lines {
        block.next_line();
        if line.is_empty() {
            attribute = DIM;
            continue;
        }
        let pre = match header.glyph {
            Some(glyph) if first => format!("{glyph} "),
            _ => String::new(),
        };
        first = false;
        block.indent((header.columns - width(line) - width(&pre)) / 2);
        block.text.push_str(attribute);
        block.text.push_str(&pre);
        block.text.push_str(line);
    }
    block
}

impl Painter {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The terminal was resized, which reflows what is on screen: the next tick paints again even when its frame did
    /// not change (the script's `WINCH` trap).
    pub fn forget(&mut self) {
        self.painted = None;
    }

    /// One tick over the body file's current `body`.
    pub fn tick(&mut self, body: &str) -> Tick {
        let (header, rest) = match body.split_once('\n') {
            Some((header, rest)) => (Header::parse(header, self.frame), rest),
            None if !body.is_empty() => (Header::parse(body, self.frame), ""),
            None => (Header::fallback(), ""),
        };
        let block = match rest {
            "" => Block::default(),
            rest if header.block_width > 0 => markdown_block(&header, body_lines(rest)),
            rest => plain_block(&header, body_lines(rest)),
        };
        self.frame = self.frame.wrapping_add(1);
        let top = usize::try_from((header.rows - block.count) / 2).unwrap_or(0);
        let foreground = header.foreground.map(|sgr| format!("{CSI}{sgr}m")).unwrap_or_default();
        let out = format!(
            "{CSI}0m{CSI}H{CSI}J{foreground}{}{}{CSI}0m",
            DOWN.repeat(top),
            block.text
        );
        let output = (self.painted.as_ref() != Some(&out)).then(|| {
            self.painted = Some(out.clone());
            out
        });
        Tick {
            output,
            interval: header.interval,
        }
    }
}

#[cfg(test)]
mod tests;
