//! What an agent's TUI shows in a pane, read from the rows of its terminal: the per-agent screen matchers the chat's
//! actions check before they send a key ([`crate::chat::input`]). Each reads the bottom of the screen, where both TUIs
//! keep their prompt and their dialogs, against the layout Claude Code 2.1.317 and Codex 0.166 (its question picker:
//! 0.167.2) draw (the PTY captures in `tests/fixtures/agent_screens`, described in
//! `crates/ngterm-core/tests/fixtures/agent_chat/README.md`):
//!
//! - **Claude's prompt**: a rule the screen's whole width, the input rows (the first one opening with `❯`), another
//!   rule, then the footer; working while the footer says `esc to interrupt` or a spinner line (`✶ Pontificating…`)
//!   sits right above the box.
//! - **Claude's permission dialog**: under a full-width rule, what it asks about (the tool, the call's description,
//!   and its command between dashed rules), a question ending with `?`, the numbered options (the picked one marked
//!   `❯`, a long label wrapped onto indented rows) and, as the screen's last line, `Esc to cancel · Tab to amend`. A
//!   digit picks an option.
//! - **Claude's `AskUserQuestion` picker**: a full-width rule over the tab row (`☐ Color`, or `←  ☐ Style  ☐ Languages
//!   ✔ Submit  →`), the question (one that wraps drawn as a quote by 2.1.318, each row opening with `│`), its
//!   numbered options (each with its description on the rows under it; a multi-select one boxed `[ ]` or `[✔]`),
//!   `Type something.`, a rule, `Chat about this` and the footer `Enter to select · …`. A picker taller than the
//!   screen (long descriptions on a phone's grid) shows its bottom only: its frame, tab row and some or all of the
//!   question scrolled off the top, which leaves no rule over its options ([`Picker::cut`]). Its review tab asks
//!   `Ready to submit your answers?` over `1. Submit answers` and `2. Cancel`.
//! - **Codex's composer**: a row opening with `›` under a blank row, the rows its text wraps onto (indented, or blank
//!   for an empty line of a pasted text), then a blank row over the footer; read from the footer up, so a blank line
//!   or a `1. …` line in the text is still the composer's. Working while a line above it says `esc to interrupt`
//!   (`• Working (0s • esc to interrupt)`).
//! - **Claude's input rows** are the rows between the box's rules, whatever they hold (blank lines, `1. …`).
//! - **What a prompt holds**: the text of its input rows after the mark, faint cells left out. Both TUIs draw the
//!   placeholder of an empty prompt faint (Claude's `Try "…"`, Codex's `Ask Codex to do anything`) and typed or
//!   pasted text in a plain style, so an empty prompt holds nothing and a draft left at the terminal shows.
//! - **Codex's approval**: what it asks about (`Would you like to run the following command?`, the reason, then the
//!   command as `$ curl …`), the numbered options (`›` marks the picked one), each ending with its key in parentheses
//!   (`(y)`, `(p)`, `(esc)`), and as the last line `Press enter to confirm or esc to cancel`.
//! - **Codex's `request_user_input` picker**: in place of the composer, a faint `Question 2/3 (1 unanswered)`, the
//!   question, its numbered options (label, then the description in a column of its own, wrapped there; `›` marks
//!   the highlighted one) ending with `None of the above`, the notes row when Tab opened it (`› Add notes`, faint, or
//!   the note typed), then the hints, one saying `enter to submit answer` (`… all` on the last question). Those hints
//!   are a dialog's words: the user's own `›` message above the picker is never read as a composer holding it.
//!
//! Anything else is [`Screen::Unknown`]: a dialog half drawn, scrolled away or quoted in a shell's output, another
//! agent's screen, a follower's screen laid out for another grid. A prompt is only read as one when no dialog's marks
//! are left near it, so a dialog the matcher can't read is never taken for a prompt to type into. Its own input rows
//! (found by their frame first: Claude's between its rules, Codex's from the footer up) are left out of that search, so
//! a prompt that quotes a dialog's words (`Explain Esc to cancel.`) is still read, and sent, as one; the rows around
//! them are searched as before. One shape stays out: a Codex composer whose `›` row reads as an option while a row of
//! it names a dialog's key (`(y)`, `(esc)`) or a dialog's words is taken for an approval drawn in its place.

use std::ops::Range;
use std::sync::Arc;

use ngterm_term::{CellFlags, CellWidth, Frame, Row};

/// An agent the matchers know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Agent {
    Claude,
    Codex,
}

impl Agent {
    /// The agent the presentation stream's `agent` frames name `name`; `None` for one this core can't drive.
    pub(crate) fn of(name: &str) -> Option<Self> {
        match name {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            _ => None,
        }
    }

    /// What `frame` shows of this agent.
    pub(crate) fn read(self, frame: &Frame) -> Screen {
        let rows = Rows::of(frame);
        match self {
            Self::Claude => claude(&rows),
            Self::Codex => codex(&rows),
        }
    }

    /// The labels an approval card offers when the screen gives none: the answers every dialog of the agent has, as
    /// its dialog words them, so a tap still finds its option once the screen is read.
    pub(crate) fn fallback_labels(self) -> Vec<String> {
        let labels: &[&str] = match self {
            Self::Claude => &["Yes", "No"],
            Self::Codex => &["Yes, proceed", "No, and tell Codex what to do differently"],
        };
        labels.iter().map(|label| (*label).to_owned()).collect()
    }
}

/// A key a dialog names for one of its options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OptionKey {
    /// The option's number, typed as a digit (Claude).
    Digit(u8),
    /// A letter (Codex's `(y)`, `(p)`).
    Letter(char),
    /// Escape (Codex's `(esc)`).
    Escape,
}

/// One option of a permission dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DialogOption {
    /// The option's words, wrapped rows joined, without its number or its key hint.
    pub(crate) label: String,
    pub(crate) key: OptionKey,
}

/// A permission dialog on screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Dialog {
    /// The rows that say what the dialog asks about, each trimmed: Claude's from under its top rule to its question
    /// (the tool, the call's description, its command), Codex's from its `Would you like to …` to its options (the
    /// reason, the command as `$ …`). Empty when the dialog shows no such rows.
    pub(crate) subject: Vec<SubjectRow>,
    pub(crate) options: Vec<DialogOption>,
}

/// One row of what a dialog asks about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SubjectRow {
    /// The row's text, trimmed.
    pub(crate) text: String,
    /// The row reaches the screen's edge: the screen may have cut a word there.
    pub(crate) full: bool,
}

impl SubjectRow {
    /// Whether the text that goes on in the next row may go on with no whitespace between: the row was cut at the
    /// screen's edge (Claude wraps a word too long for a row there), or it ends where a line may break inside a word
    /// (Codex breaks `https://` | `example.net`). After a letter or a digit short of the edge, the screen broke at
    /// whitespace.
    fn breaks_inside_a_word(&self) -> bool {
        self.full || self.text.chars().last().is_some_and(|c| !c.is_alphanumeric())
    }
}

impl Dialog {
    /// Whether the dialog asks about the call `detail` names (its command, path or URL, as the dialog shows it): some
    /// run of the subject's rows reads as `detail` (its ends trimmed; a command's leading `$ ` not counted). Within a
    /// row whitespace counts as it is, so `touch 'a b'` is not `touch 'ab'`; between two rows `detail` holds the
    /// whitespace the screen's wrap took (a space, a line break, the next line's indent), or none where the row may
    /// break inside a word ([`SubjectRow::breaks_inside_a_word`]). A run is whole rows, so a row that says more than
    /// `detail` (another command that starts the same, a sentence that quotes it) is not a match, and a row cut short
    /// with `…` is not either: another call may start the same.
    pub(crate) fn asks(&self, detail: &str) -> bool {
        let wanted = detail.trim();
        !wanted.is_empty() && (0..self.subject.len()).any(|from| self.reads_from(from, wanted))
    }

    /// Whether the subject's rows from `from` on read as `wanted`.
    fn reads_from(&self, from: usize, wanted: &str) -> bool {
        let mut rest = wanted;
        let mut previous: Option<&SubjectRow> = None;
        for (at, row) in self.subject.iter().enumerate().skip(from) {
            let text = if at == from {
                row.text.strip_prefix("$ ").unwrap_or(&row.text)
            } else {
                &row.text
            };
            if text.is_empty() {
                return false;
            }
            if let Some(previous) = previous {
                let after = rest.trim_start();
                if after.len() == rest.len() && !previous.breaks_inside_a_word() {
                    return false;
                }
                rest = after;
            }
            let Some(after) = rest.strip_prefix(text) else {
                return false;
            };
            if after.is_empty() {
                return true;
            }
            rest = after;
            previous = Some(row);
        }
        false
    }

    /// The option whose label is `label`, whitespace aside (a label read at another width wraps elsewhere).
    pub(crate) fn option(&self, label: &str) -> Option<&DialogOption> {
        let wanted = squeeze(label);
        let mut found = self.options.iter().filter(|option| squeeze(&option.label) == wanted);
        let option = found.next()?;
        found.next().is_none().then_some(option)
    }
}

/// One option of the question picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PickerOption {
    pub(crate) number: u8,
    /// The option's rows joined: its label, then its description; a multi-select box not included.
    pub(crate) text: String,
    /// A multi-select option's box: whether it is ticked.
    pub(crate) checked: Option<bool>,
    /// The picker's highlight is on it.
    pub(crate) cursor: bool,
}

/// A question picker on screen at one of its questions: Claude's `AskUserQuestion` at a question's tab, or Codex's
/// `request_user_input`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Picker {
    /// The question's rows joined, a quote's bars left out.
    pub(crate) question: String,
    /// Its options as numbered: Claude's with `Type something.`, `Chat about this` left out; Codex's with
    /// `None of the above`.
    pub(crate) options: Vec<PickerOption>,
    /// The picker is taller than the screen: its top scrolled off, and `question` is what is left of it.
    pub(crate) cut: bool,
    /// Codex's: the question shown and how many the call asks (`Question 2/3`).
    pub(crate) position: Option<(u8, u8)>,
    /// Codex's notes row under the options, while open (Tab): what it holds, empty while it shows its placeholder.
    pub(crate) notes: Option<String>,
}

impl Picker {
    /// Whether the question's options are boxed (several can be picked).
    pub(crate) fn multi(&self) -> bool {
        self.options.first().is_some_and(|option| option.checked.is_some())
    }
}

/// What a pane's screen shows of its agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Screen {
    /// The prompt, taking input: idle, or `working` on a turn; `composer` is what its input rows hold (rows joined with
    /// `\n`, the placeholder left out), empty when nothing was typed.
    Prompt { working: bool, composer: String },
    /// A permission dialog.
    Permission(Dialog),
    /// The question picker at a question's tab.
    Question(Picker),
    /// The picker's review tab; `on_submit` while `1. Submit answers` is highlighted.
    Review { on_submit: bool },
    /// Nothing the matchers can read.
    Unknown,
}

impl Screen {
    /// Whether a dialog (permission, picker or review) is up.
    pub(crate) fn is_dialog(&self) -> bool {
        matches!(self, Self::Permission(_) | Self::Question(_) | Self::Review { .. })
    }
}

/// `text` without whitespace: how labels and questions are compared, since a screen wraps them where it likes.
pub(crate) fn squeeze(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Whether `shown` (as the screen has it) is `expected`: equal whitespace aside, or cut short with `…`.
pub(crate) fn fits(shown: &str, expected: &str) -> bool {
    let (shown, expected) = (squeeze(shown), squeeze(expected));
    if shown == expected {
        return true;
    }
    shown
        .strip_suffix('…')
        .is_some_and(|cut| !cut.is_empty() && expected.starts_with(cut))
}

/// Words that only a dialog draws: a prompt is not read as one with any of them near it.
const DIALOG_MARKS: [&str; 8] = [
    "Esc to cancel",
    "esc to cancel",
    "Enter to select",
    "Press enter to confirm",
    "Do you want to proceed?",
    "Ready to submit your answers?",
    "enter to submit answer",
    "enter to submit all",
];

/// The most rows under Codex's composer: its footer, or a popup it shows there.
const FOOTER_ROWS: usize = 12;

/// The most rows under Claude's prompt box: its footer, or a popup it shows there (a slash command's list, whose
/// entries take two rows each on a phone's narrow grid: Claude Code 2.1.318 lists seven, fourteen rows at 52 columns).
/// Each of them is indented ([`Rows::indented`]): what a shell prints under a box Claude left on screen is not.
const CLAUDE_FOOTER_ROWS: usize = 24;

/// The rows above a prompt searched for a turn's spinner.
const NEAR: usize = 6;

/// The rows above a prompt searched for what is left of a dialog: as tall as a dialog gets on a phone.
const ABOVE: usize = 24;

/// A frame's rows as text, with its width and the rows' cells.
struct Rows {
    lines: Vec<String>,
    cols: usize,
    cells: Vec<Arc<Row>>,
}

impl Rows {
    fn of(frame: &Frame) -> Self {
        Self {
            lines: frame.lines(),
            cols: usize::from(frame.cols),
            cells: frame.rows.clone(),
        }
    }

    /// What the input rows `from..to` hold after the prompt's mark: their text from the second column on, a faint
    /// (placeholder) or hidden cell taken as blank, each row trimmed, blank rows dropped.
    fn typed(&self, from: usize, to: usize) -> String {
        let lines: Vec<String> = (from..to.min(self.cells.len()))
            .map(|index| plain(&self.cells[index]))
            .filter(|line| !line.is_empty())
            .collect();
        lines.join("\n")
    }

    fn get(&self, index: usize) -> &str {
        self.lines.get(index).map_or("", String::as_str)
    }

    /// Row `index` as a dialog's subject row: trimmed, and whether it reaches the screen's edge (as [`options`] tells
    /// a cut row).
    fn subject_row(&self, index: usize) -> SubjectRow {
        let line = self.get(index);
        SubjectRow {
            text: line.trim().to_owned(),
            full: line.trim_end().chars().count() + 1 >= self.cols,
        }
    }

    /// Whether every row in `range` is blank or indented: the rows Claude draws under its prompt box (its footer, a
    /// popup's entries and their wrapped descriptions) all are.
    fn indented(&self, range: Range<usize>) -> bool {
        range
            .into_iter()
            .all(|index| self.blank(index) || self.get(index).starts_with("  "))
    }

    fn blank(&self, index: usize) -> bool {
        self.get(index).trim().is_empty()
    }

    /// A rule the screen's whole width.
    fn rule(&self, index: usize) -> bool {
        let line = self.get(index);
        line.chars().count() == self.cols && line.chars().all(|c| c == '─')
    }

    /// The last row with anything on it.
    fn last(&self) -> Option<usize> {
        (0..self.lines.len()).rev().find(|index| !self.blank(*index))
    }

    /// Whether a dialog's marks show on the rows `from..to`.
    fn marked(&self, from: usize, to: usize) -> bool {
        (from..to.min(self.lines.len())).any(|index| marks(self.get(index)))
    }

    /// Whether a dialog's marks show from row `from` to the screen's bottom, a prompt's own input rows `input` left
    /// out: what was typed there may quote a dialog's words.
    fn marked_around(&self, from: usize, input: Range<usize>) -> bool {
        self.marked(from, input.start) || self.marked(input.end, self.lines.len())
    }

    /// The first row at or before `index` that is not blank, looking up at most `limit` rows.
    fn up_to_text(&self, index: usize, limit: usize) -> Option<usize> {
        (index.saturating_sub(limit)..=index).rev().find(|at| !self.blank(*at))
    }
}

/// Whether `line` holds a dialog's words.
fn marks(line: &str) -> bool {
    DIALOG_MARKS.iter().any(|mark| line.contains(mark))
}

/// `row`'s text from its second column on, trimmed: a faint (placeholder) or hidden cell is blank, a spacer cell (the
/// rest of a wide glyph) nothing.
fn plain(row: &Row) -> String {
    let mut text = String::new();
    for (column, cell) in row.cells().iter().enumerate().skip(1) {
        if matches!(cell.width, CellWidth::SpacerTail | CellWidth::SpacerHead) {
            continue;
        }
        let glyph = row.text(column);
        let hidden = cell.flags.intersects(CellFlags::FAINT | CellFlags::INVISIBLE);
        text.push_str(if hidden || glyph.is_empty() { " " } else { glyph });
    }
    text.trim().to_owned()
}

/// An option row: `[cursor] N. text`, with the column its number starts at.
struct OptionRow {
    number: u8,
    cursor: bool,
    column: usize,
    text: String,
}

fn option_row(line: &str, cursor: char) -> Option<OptionRow> {
    let trimmed = line.trim_start();
    let mut column = line.chars().count() - trimmed.chars().count();
    let (marked, rest) = match trimmed.strip_prefix(cursor) {
        Some(rest) => {
            let after = rest.trim_start();
            column += 1 + rest.chars().count() - after.chars().count();
            (true, after)
        }
        None => (false, trimmed),
    };
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let text = rest[digits.len()..].strip_prefix(". ")?;
    let number = digits.parse().ok().filter(|number| *number > 0)?;
    Some(OptionRow {
        number,
        cursor: marked,
        column,
        text: text.trim_end().to_owned(),
    })
}

/// An option as its rows give it: the option row's text and the rows wrapped under it.
struct Block {
    number: u8,
    cursor: bool,
    text: String,
}

/// The numbered options from row `start` (option 1) down to `end` (exclusive): each option row followed by the rows
/// indented past its number. `None` unless the numbers run 1, 2, … with exactly one cursor.
fn options(rows: &Rows, start: usize, end: usize, cursor: char) -> Option<Vec<Block>> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut column = 0;
    let mut previous_full = false;
    for index in start..end {
        let line = rows.get(index);
        let width = line.chars().count();
        if let Some(row) = option_row(line, cursor) {
            if usize::from(row.number) != blocks.len() + 1 {
                return None;
            }
            column = row.column;
            blocks.push(Block {
                number: row.number,
                cursor: row.cursor,
                text: row.text,
            });
        } else {
            let indent = width - line.trim_start().chars().count();
            let block = blocks
                .last_mut()
                .filter(|_| !line.trim().is_empty() && indent > column)?;
            // a row cut at the screen's edge breaks a word, one that ends short breaks between words
            if !previous_full {
                block.text.push(' ');
            }
            block.text.push_str(line.trim());
        }
        previous_full = width + 1 >= rows.cols;
    }
    let cursors = blocks.iter().filter(|block| block.cursor).count();
    (!blocks.is_empty() && cursors == 1).then_some(blocks)
}

/// The row where the options that end at `end` (inclusive) start: the last option 1 of the rows with text above it.
fn options_start(rows: &Rows, end: usize, cursor: char) -> Option<usize> {
    let mut top = end;
    while top > 0 && !rows.blank(top - 1) && !rows.rule(top - 1) {
        top -= 1;
    }
    (top..=end)
        .rev()
        .find(|index| option_row(rows.get(*index), cursor).is_some_and(|row| row.number == 1))
}

fn claude(rows: &Rows) -> Screen {
    if let Some(dialog) = claude_permission(rows) {
        return Screen::Permission(dialog);
    }
    if let Some(on_submit) = claude_review(rows) {
        return Screen::Review { on_submit };
    }
    if let Some(picker) = claude_picker(rows) {
        return Screen::Question(picker);
    }
    claude_prompt(rows).map_or(Screen::Unknown, |(working, composer)| Screen::Prompt {
        working,
        composer,
    })
}

const CLAUDE_CURSOR: char = '❯';

fn claude_permission(rows: &Rows) -> Option<Dialog> {
    let footer = rows.last()?;
    if !rows.get(footer).trim_start().starts_with("Esc to cancel") {
        return None;
    }
    let end = rows.up_to_text(footer.checked_sub(1)?, 2)?;
    let start = options_start(rows, end, CLAUDE_CURSOR)?;
    let blocks = options(rows, start, end + 1, CLAUDE_CURSOR)?;
    let question = start.checked_sub(1)?;
    if !rows.get(question).trim_end().ends_with('?') {
        return None;
    }
    // the dialog's top: a full-width rule over its title
    let top = (0..question).rev().find(|index| rows.rule(*index))?;
    let options = blocks
        .into_iter()
        .map(|block| {
            (block.number <= 9).then_some(DialogOption {
                label: block.text,
                key: OptionKey::Digit(block.number),
            })
        })
        .collect::<Option<Vec<_>>>()?;
    let subject = (top + 1..question).map(|index| rows.subject_row(index)).collect();
    (options.len() >= 2).then_some(Dialog { subject, options })
}

/// The picker's tab row: the question tabs, or `☐ Color` for a lone question.
fn tab_row(line: &str) -> bool {
    let trimmed = line.trim_start();
    ["←", "☐", "☒", "✔"].iter().any(|start| trimmed.starts_with(start))
}

/// The tab row of the picker's frame over `row`: under the full-width rule, or at the screen's top (a picker a row
/// taller than the screen left its rule above it).
fn picker_top(rows: &Rows, row: usize) -> Option<usize> {
    let tabs = (0..row).rev().find(|index| tab_row(rows.get(*index)))?;
    (tabs == 0 || rows.rule(tabs - 1)).then_some(tabs)
}

/// The question over the picker's options (which start at row `start`), its rows joined, and whether the picker's top
/// scrolled off the screen. Under the tab row the question is whole. A picker taller than the screen leaves no tab row
/// and no rule over its options: the rows over them are what is left of the question (none, maybe). A question that
/// wraps is drawn as a quote (Claude Code 2.1.318), each of its rows opening with `│`: the bars are left out.
fn picker_question(rows: &Rows, start: usize) -> Option<(String, bool)> {
    let (from, cut) = match picker_top(rows, start) {
        Some(tabs) => (tabs + 1, false),
        None if !(0..start).any(|index| rows.rule(index) || tab_row(rows.get(index))) => (0, true),
        None => return None,
    };
    let lines: Vec<&str> = (from..start)
        .map(|index| rows.get(index).trim())
        .filter(|line| !line.is_empty())
        .collect();
    if lines.is_empty() && !cut {
        return None;
    }
    let quoted = !lines.is_empty() && lines.iter().all(|line| line.starts_with(QUOTE_BAR));
    let lines: Vec<&str> = if quoted {
        lines
            .iter()
            .map(|line| line[QUOTE_BAR.len_utf8()..].trim_start())
            .collect()
    } else {
        lines
    };
    Some((lines.join(" "), cut))
}

/// The bar each row of a quoted question opens with.
const QUOTE_BAR: char = '│';

/// Whether only blank rows, or at most `wrapped` rows of a footer wrapped onto them, follow `row`.
fn ends_after(rows: &Rows, row: usize, wrapped: usize) -> bool {
    let after = (row + 1..rows.lines.len()).filter(|index| !rows.blank(*index)).count();
    after <= wrapped
}

fn claude_picker(rows: &Rows) -> Option<Picker> {
    let footer = (0..rows.lines.len())
        .rev()
        .find(|index| rows.get(*index).trim_start().starts_with("Enter to select"))?;
    if !ends_after(rows, footer, 1) {
        return None;
    }
    let chat = rows.up_to_text(footer.checked_sub(1)?, 2)?;
    if !option_row(rows.get(chat), CLAUDE_CURSOR).is_some_and(|row| row.text.starts_with("Chat about this")) {
        return None;
    }
    let rule = chat.checked_sub(1).filter(|rule| rows.rule(*rule))?;
    let end = rule.checked_sub(1)?;
    let start = options_start(rows, end, CLAUDE_CURSOR)?;
    let blocks = options(rows, start, end + 1, CLAUDE_CURSOR)?;
    let (question, cut) = picker_question(rows, start)?;
    let options = blocks
        .into_iter()
        .map(|block| {
            let (checked, text) = match boxed(&block.text) {
                Some((checked, text)) => (Some(checked), text.to_owned()),
                None => (None, block.text),
            };
            PickerOption {
                number: block.number,
                text,
                checked,
                cursor: block.cursor,
            }
        })
        .collect();
    Some(Picker {
        question,
        options,
        cut,
        position: None,
        notes: None,
    })
}

/// A multi-select option's box and the text after it: `[ ] Spanish` is unticked.
fn boxed(text: &str) -> Option<(bool, &str)> {
    let mut chars = text.chars();
    if chars.next() != Some('[') {
        return None;
    }
    let mark = chars.next()?;
    if chars.next() != Some(']') {
        return None;
    }
    Some((mark != ' ', chars.as_str().trim_start()))
}

fn claude_review(rows: &Rows) -> Option<bool> {
    let ready = (0..rows.lines.len())
        .rev()
        .find(|index| rows.get(*index).trim() == "Ready to submit your answers?")?;
    let start = (ready + 1..rows.lines.len()).find(|index| !rows.blank(*index))?;
    let blocks = options(rows, start, start + 2, CLAUDE_CURSOR)?;
    let [submit, cancel] = blocks.as_slice() else {
        return None;
    };
    if submit.text != "Submit answers" || cancel.text != "Cancel" {
        return None;
    }
    let tail_ok = (start + 2..rows.lines.len()).all(|index| {
        let line = rows.get(index).trim_start();
        line.is_empty() || line.starts_with("Enter to select") || line.starts_with("Esc to cancel")
    });
    let tabs = picker_top(rows, ready)?;
    (tail_ok && rows.get(tabs).contains("Submit")).then_some(submit.cursor)
}

/// Claude's prompt box: whether a turn is running and what the box holds, or `None` when the box is not the screen's
/// bottom.
fn claude_prompt(rows: &Rows) -> Option<(bool, String)> {
    let last = rows.last()?;
    let lower = (0..=last).rev().find(|index| rows.rule(*index))?;
    if last - lower > CLAUDE_FOOTER_ROWS || !rows.indented(lower + 1..last + 1) {
        return None;
    }
    let upper = (0..lower).rev().find(|index| rows.rule(*index))?;
    if upper + 1 == lower || !rows.get(upper + 1).starts_with(CLAUDE_CURSOR) {
        return None;
    }
    // a dialog drawn over the box from the top leaves the box's rows under it until it reaches them: its own rule
    // or its words above or under the box mean the box is what is left of the screen before. The input rows between
    // the rules are left out of the words' search: they hold what was typed, and no dialog reaches them while the
    // box's upper rule (no other above it) and its `❯` row are still there
    let input = upper + 1..lower;
    if (0..upper).any(|index| rows.rule(index)) || rows.marked_around(upper.saturating_sub(ABOVE), input.clone()) {
        return None;
    }
    let footer = (lower + 1..=last).any(|index| rows.get(index).contains("esc to interrupt"));
    let spinner = (upper.saturating_sub(NEAR)..upper).any(|index| spinning(rows.get(index)));
    Some((footer || spinner, rows.typed(input.start, input.end)))
}

/// A turn's spinner line: a glyph, a word ending with `…` (`✶ Pontificating…`, `✢ Cogitating… (running …)`).
fn spinning(line: &str) -> bool {
    let mut words = line.split_whitespace();
    let glyph = words.next().unwrap_or_default();
    let verb = words.next().unwrap_or_default();
    glyph.chars().count() == 1
        && !glyph.chars().all(char::is_alphanumeric)
        && verb.ends_with('…')
        && verb.trim_end_matches('…').chars().all(char::is_alphabetic)
}

fn codex(rows: &Rows) -> Screen {
    if let Some(dialog) = codex_approval(rows) {
        return Screen::Permission(dialog);
    }
    if let Some(picker) = codex_picker(rows) {
        return Screen::Question(picker);
    }
    codex_prompt(rows).map_or(Screen::Unknown, |(working, composer)| Screen::Prompt {
        working,
        composer,
    })
}

const CODEX_CURSOR: char = '›';

fn codex_approval(rows: &Rows) -> Option<Dialog> {
    let footer = rows.last()?;
    if !rows
        .get(footer)
        .trim_start()
        .starts_with("Press enter to confirm or esc to cancel")
    {
        return None;
    }
    let end = rows.up_to_text(footer.checked_sub(1)?, 2)?;
    let start = options_start(rows, end, CODEX_CURSOR)?;
    if start > 0 && !rows.blank(start - 1) {
        return None;
    }
    let options = options(rows, start, end + 1, CODEX_CURSOR)?
        .into_iter()
        .map(|block| {
            let (label, key) = key_hint(&block.text)?;
            Some(DialogOption { label, key })
        })
        .collect::<Option<Vec<_>>>()?;
    // what it asks about: from its `Would you like to …` down to its options, none when that row is not near
    let subject = (start.saturating_sub(ABOVE)..start)
        .rev()
        .find(|index| rows.get(*index).trim_start().starts_with("Would you like to"))
        .map_or_else(Vec::new, |from| {
            (from..start).map(|index| rows.subject_row(index)).collect()
        });
    (options.len() >= 2).then_some(Dialog { subject, options })
}

/// Codex's question picker: its `Question N/M` header (the last on screen), the question's rows, the numbered options,
/// the notes row when open, then the hints (one saying `enter to submit`) as the screen's last rows.
fn codex_picker(rows: &Rows) -> Option<Picker> {
    let header = (0..rows.lines.len())
        .rev()
        .find(|index| question_position(rows.get(*index)).is_some())?;
    let position = question_position(rows.get(header))?;
    let question_end = run_end(rows, header + 1);
    let question: Vec<&str> = (header + 1..question_end).map(|index| rows.get(index).trim()).collect();
    if question.is_empty() {
        return None;
    }
    let start = next_text(rows, question_end)?;
    let end = run_end(rows, start);
    let blocks = options(rows, start, end, CODEX_CURSOR)?;
    let mut hints = next_text(rows, end)?;
    let notes = if rows.get(hints).trim_start().starts_with(CODEX_CURSOR) {
        let notes_end = run_end(rows, hints);
        let typed = rows.typed(hints, notes_end);
        hints = next_text(rows, notes_end)?;
        Some(typed.strip_prefix(CODEX_CURSOR).unwrap_or(&typed).trim().to_owned())
    } else {
        None
    };
    let hints_end = run_end(rows, hints);
    let submits = (hints..hints_end).any(|index| rows.get(index).contains(CODEX_SUBMIT));
    if !submits || next_text(rows, hints_end).is_some() {
        return None;
    }
    let options = blocks
        .into_iter()
        .map(|block| PickerOption {
            number: block.number,
            text: block.text,
            checked: None,
            cursor: block.cursor,
        })
        .collect();
    Some(Picker {
        question: question.join(" "),
        options,
        cut: false,
        position: Some(position),
        notes,
    })
}

/// What one of the Codex picker's hints says: `enter to submit answer`, or `… all` on the last question.
const CODEX_SUBMIT: &str = "enter to submit";

/// `Question 2/3 (1 unanswered)`: the question shown and how many the call asks.
fn question_position(line: &str) -> Option<(u8, u8)> {
    let rest = line.trim_start().strip_prefix("Question ")?;
    let (shown, count) = rest.split_whitespace().next()?.split_once('/')?;
    let (shown, count): (u8, u8) = (shown.parse().ok()?, count.parse().ok()?);
    (1..=count).contains(&shown).then_some((shown, count))
}

/// The first row from `from` on that is not blank.
fn next_text(rows: &Rows, from: usize) -> Option<usize> {
    (from..rows.lines.len()).find(|index| !rows.blank(*index))
}

/// Where the run of rows with text from `from` ends: the first blank row, or the screen's end.
fn run_end(rows: &Rows, from: usize) -> usize {
    (from..rows.lines.len())
        .find(|index| rows.blank(*index))
        .unwrap_or(rows.lines.len())
}

/// A Codex option's label and the key its `(…)` names: a letter, or `esc`.
fn key_hint(text: &str) -> Option<(String, OptionKey)> {
    let open = text.rfind(" (")?;
    let hint = text[open + 2..].strip_suffix(')')?;
    let key = match hint {
        "esc" => OptionKey::Escape,
        letter if letter.len() == 1 && letter.chars().all(|c| c.is_ascii_lowercase()) => {
            OptionKey::Letter(letter.chars().next()?)
        }
        _ => return None,
    };
    Some((text[..open].trim_end().to_owned(), key))
}

/// Codex's composer: whether a turn is running and what it holds, or `None` when it is not on screen.
fn codex_prompt(rows: &Rows) -> Option<(bool, String)> {
    let (composer, below) = codex_composer(rows)?;
    // the composer's own rows hold what was typed: a dialog's words there are text, unless the rows are shaped like
    // an approval (below)
    if rows.marked_around(composer.saturating_sub(ABOVE), composer..below) {
        return None;
    }
    // an option row with the cursor on it is a dialog's, unless it is the composer's own text (`› 1. Fix the bug`,
    // or `  › 2. …` further down a paste)
    let options_near = (composer.saturating_sub(ABOVE)..rows.lines.len())
        .filter(|index| !(composer..below).contains(index))
        .any(|index| option_row(rows.get(index), CODEX_CURSOR).is_some_and(|row| row.cursor));
    if options_near {
        return None;
    }
    // a composer row shaped like an option is only text while nothing in its rows names a dialog's key (`(y)`,
    // `(esc)`) or holds a dialog's words: an approval drawn where the composer was (its footer not the screen's last
    // run yet, a layout the approval matcher can't read) takes this shape, and Stop's Escape would answer it. Kept in
    // either state: idle it costs only a prompt whose first line is numbered and that names a key or a dialog's words
    // (pasted, not entered: Unconfirmed), working it keeps Escape off such an approval
    let keyed = option_row(rows.get(composer), CODEX_CURSOR).is_some()
        && (composer..below).any(|index| {
            let line = rows.get(index);
            key_hint(line.trim()).is_some() || marks(line)
        });
    if keyed {
        return None;
    }
    let working = (composer.saturating_sub(NEAR)..composer).any(|index| rows.get(index).contains("esc to interrupt"));
    Some((working, rows.typed(composer, below)))
}

/// Where Codex's composer is: its `›` row and the blank row under its text, above the footer. Read from the bottom:
/// the footer (the rows with text at the screen's bottom, no `›` among them), the blank row over it, then up through
/// the composer's rows (wrapped text indented, or a blank line inside a pasted text) to the `›` row, which has a blank
/// row above it. The `›` row may read as an option (`› 1. Fix the bug`): it is still the composer's.
fn codex_composer(rows: &Rows) -> Option<(usize, usize)> {
    let last = rows.last()?;
    let mut top = last;
    while top > 0 && !rows.blank(top - 1) {
        top -= 1;
    }
    let below = top.checked_sub(1)?;
    if last - below > FOOTER_ROWS || (top..=last).any(|index| rows.get(index).starts_with(CODEX_CURSOR)) {
        return None;
    }
    let composer = (0..below).rev().find(|index| {
        let line = rows.get(*index);
        line.starts_with(CODEX_CURSOR) || !(rows.blank(*index) || line.starts_with("  "))
    })?;
    if !rows.get(composer).starts_with(CODEX_CURSOR) || (composer > 0 && !rows.blank(composer - 1)) {
        return None;
    }
    Some((composer, below))
}
