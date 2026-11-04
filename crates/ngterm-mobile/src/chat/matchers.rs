//! What an agent's TUI shows in a pane, read from the rows of its terminal: the per-agent screen matchers the chat's
//! actions check before they send a key ([`crate::chat::input`]). Each reads the bottom of the screen, where both TUIs
//! keep their prompt and their dialogs, against the layout Claude Code 2.1.317 and Codex 0.166 draw (the PTY captures
//! in `tests/fixtures/agent_screens`, described in `crates/ngterm-core/tests/fixtures/agent_chat/README.md`):
//!
//! - **Claude's prompt**: a rule the screen's whole width, the input rows (the first one opening with `❯`), another
//!   rule, then the footer; working while the footer says `esc to interrupt` or a spinner line (`✶ Pontificating…`)
//!   sits right above the box.
//! - **Claude's permission dialog**: under a full-width rule, a question ending with `?`, the numbered options (the
//!   picked one marked `❯`, a long label wrapped onto indented rows) and, as the screen's last line,
//!   `Esc to cancel · Tab to amend`. A digit picks an option.
//! - **Claude's `AskUserQuestion` picker**: a full-width rule over the tab row (`☐ Color`, or `←  ☐ Style  ☐ Languages
//!   ✔ Submit  →`), the question, its numbered options (each with its description on the rows under it; a
//!   multi-select one boxed `[ ]` or `[✔]`), `Type something.`, a rule, `Chat about this` and the footer
//!   `Enter to select · …`. Its review tab asks `Ready to submit your answers?` over `1. Submit answers` and
//!   `2. Cancel`.
//! - **Codex's composer**: a row opening with `›` between blank rows, its footer under it; working while a line above
//!   it says `esc to interrupt` (`• Working (0s • esc to interrupt)`).
//! - **Codex's approval**: the numbered options (`›` marks the picked one), each ending with its key in parentheses
//!   (`(y)`, `(p)`, `(esc)`), and as the last line `Press enter to confirm or esc to cancel`.
//!
//! Anything else is [`Screen::Unknown`]: a dialog half drawn, scrolled away or quoted in a shell's output, another
//! agent's screen, a follower's screen laid out for another grid. A prompt is only read as one when no dialog's marks
//! are left near it, so a dialog the matcher can't read is never taken for a prompt to type into.

use ngterm_term::Frame;

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
    pub(crate) options: Vec<DialogOption>,
}

impl Dialog {
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

/// Claude's `AskUserQuestion` picker on screen, at one question's tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Picker {
    /// The question's rows joined.
    pub(crate) question: String,
    /// Its options as numbered, `Type something.` included, `Chat about this` not.
    pub(crate) options: Vec<PickerOption>,
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
    /// The prompt, taking input: idle, or `working` on a turn.
    Prompt { working: bool },
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
const DIALOG_MARKS: [&str; 6] = [
    "Esc to cancel",
    "esc to cancel",
    "Enter to select",
    "Press enter to confirm",
    "Do you want to proceed?",
    "Ready to submit your answers?",
];

/// The most rows under a prompt: its footer, or a popup the TUI shows there (a slash command's list).
const FOOTER_ROWS: usize = 12;

/// The rows above a prompt searched for a turn's spinner.
const NEAR: usize = 6;

/// The rows above a prompt searched for what is left of a dialog: as tall as a dialog gets on a phone.
const ABOVE: usize = 24;

/// A frame's rows as text, with its width.
struct Rows {
    lines: Vec<String>,
    cols: usize,
}

impl Rows {
    fn of(frame: &Frame) -> Self {
        Self {
            lines: frame.lines(),
            cols: usize::from(frame.cols),
        }
    }

    fn get(&self, index: usize) -> &str {
        self.lines.get(index).map_or("", String::as_str)
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
        (from..to.min(self.lines.len())).any(|index| {
            let line = self.get(index);
            DIALOG_MARKS.iter().any(|mark| line.contains(mark))
        })
    }

    /// The first row at or before `index` that is not blank, looking up at most `limit` rows.
    fn up_to_text(&self, index: usize, limit: usize) -> Option<usize> {
        (index.saturating_sub(limit)..=index).rev().find(|at| !self.blank(*at))
    }
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
    claude_prompt(rows).map_or(Screen::Unknown, |working| Screen::Prompt { working })
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
    (0..question).rev().find(|index| rows.rule(*index))?;
    let options = blocks
        .into_iter()
        .map(|block| {
            (block.number <= 9).then_some(DialogOption {
                label: block.text,
                key: OptionKey::Digit(block.number),
            })
        })
        .collect::<Option<Vec<_>>>()?;
    (options.len() >= 2).then_some(Dialog { options })
}

/// The picker's tab row: the question tabs, or `☐ Color` for a lone question.
fn tab_row(line: &str) -> bool {
    let trimmed = line.trim_start();
    ["←", "☐", "☒", "✔"].iter().any(|start| trimmed.starts_with(start))
}

/// The row of the picker's frame under `row`: the full-width rule right above its tab row.
fn picker_top(rows: &Rows, row: usize) -> Option<usize> {
    let tabs = (0..row).rev().find(|index| tab_row(rows.get(*index)))?;
    rows.rule(tabs.checked_sub(1)?).then_some(tabs)
}

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
    let tabs = picker_top(rows, start)?;
    let question: Vec<&str> = (tabs + 1..start)
        .map(|index| rows.get(index).trim())
        .filter(|line| !line.is_empty())
        .collect();
    if question.is_empty() {
        return None;
    }
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
        question: question.join(" "),
        options,
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

/// Claude's prompt box: whether a turn is running, or `None` when the box is not the screen's bottom.
fn claude_prompt(rows: &Rows) -> Option<bool> {
    let last = rows.last()?;
    let lower = (0..=last).rev().find(|index| rows.rule(*index))?;
    if last - lower > FOOTER_ROWS {
        return None;
    }
    let upper = (0..lower).rev().find(|index| rows.rule(*index))?;
    if upper + 1 == lower || !rows.get(upper + 1).starts_with(CLAUDE_CURSOR) {
        return None;
    }
    // a dialog drawn over the box from the top leaves the box's rows under it until it reaches them: its own rule
    // or its words above the box mean the box is what is left of the screen before
    if (0..upper).any(|index| rows.rule(index)) || rows.marked(upper.saturating_sub(ABOVE), rows.lines.len()) {
        return None;
    }
    let footer = (lower + 1..=last).any(|index| rows.get(index).contains("esc to interrupt"));
    let spinner = (upper.saturating_sub(NEAR)..upper).any(|index| spinning(rows.get(index)));
    Some(footer || spinner)
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
    codex_prompt(rows).map_or(Screen::Unknown, |working| Screen::Prompt { working })
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
    (options.len() >= 2).then_some(Dialog { options })
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

/// Codex's composer: whether a turn is running, or `None` when it is not on screen.
fn codex_prompt(rows: &Rows) -> Option<bool> {
    let last = rows.last()?;
    // the composer: a `›` row (not an option), the rows its text wraps onto, a blank row, then the footer
    let composer = (0..=last).rev().find(|index| {
        let line = rows.get(*index);
        line.starts_with(CODEX_CURSOR) && option_row(line, CODEX_CURSOR).is_none()
    })?;
    if composer > 0 && !rows.blank(composer - 1) {
        return None;
    }
    let mut below = composer + 1;
    while below <= last && rows.get(below).starts_with("  ") && !rows.blank(below) {
        below += 1;
    }
    if below > last || !rows.blank(below) || last - below > FOOTER_ROWS {
        return None;
    }
    if rows.marked(composer.saturating_sub(ABOVE), rows.lines.len()) {
        return None;
    }
    let options_near = (composer.saturating_sub(ABOVE)..rows.lines.len())
        .any(|index| option_row(rows.get(index), CODEX_CURSOR).is_some_and(|row| row.cursor));
    if options_near {
        return None;
    }
    let working = (composer.saturating_sub(NEAR)..composer).any(|index| rows.get(index).contains("esc to interrupt"));
    Some(working)
}
