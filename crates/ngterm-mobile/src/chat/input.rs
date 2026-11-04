//! The chat's actions on a pane's agent, as keys into its terminal: a prompt, Stop, an approval's answer and the
//! answers to Claude's `AskUserQuestion`. Each runs as one sequence on the core's actor (`crate::actor::inputs`):
//!
//! 1. **Lead.** A pane that is not its session's leader claims the lead first (as Take control does) and waits for
//!    the attach to report [`LeadRole::Leader`](ngterm_core::live::lead::LeadRole); a claim that reports another role,
//!    is replaced or lost, or has no answer within [`InputTiming::lead_timeout`] sends nothing
//!    ([`AgentInputOutcome::LeadRefused`]). Only the leader's input reaches the program, and its grid lays the TUI
//!    out, so the screen read next is laid out for this pane.
//! 2. **Settle.** No new output, no new frame and no new grid for [`InputTiming::quiet`] (the redraw after the claim's
//!    resize has landed), or [`InputTiming::settle_max`] at most: a working agent's spinner never goes quiet.
//! 3. **Match.** The agent's screen matcher ([`crate::chat::matchers`]) reads the terminal's current frame, and the
//!    action's [`Plan`] decides from it: the keys to send, or the outcome.
//! 4. **Send.** The keys go into the same terminal the frame was read from, on the actor, with nothing awaited between
//!    the read and the send: the role and the attach are checked again right before (the terminal's pending role
//!    reports taken first), so a lead lost or an attach replaced meanwhile sends nothing.
//!
//! A step that needs to see what its keys did (a paste before its Enter, each question of the picker, a toggle)
//! settles and matches again before its next key, and a dialog's answer is checked once more after it: the dialog
//! still up is [`AgentInputOutcome::Unconfirmed`]. Every outcome but [`AgentInputOutcome::Sent`] and
//! [`AgentInputOutcome::Unconfirmed`] means nothing was sent.
//!
//! The keys (Task 1's recordings, `crates/ngterm-core/tests/fixtures/agent_chat/README.md`):
//!
//! - **Prompt**: a bracketed paste of the whole text, then Enter; `/name args` and Codex's `$skill` go the same way,
//!   since both TUIs run a pasted command. It needs the prompt on screen, empty (a draft left in it at the terminal
//!   would be sent along: [`AgentInputOutcome::NotOnScreen`]), and no dialog ([`AgentInputOutcome::DialogOpen`]).
//!   The Enter goes only once the prompt holds the text pasted, or the stand-in a TUI shows for a long paste
//!   (Claude's `[Pasted text #1 +12 lines]`, Codex's `[Pasted Content 1200 chars]`), and nothing else; otherwise the
//!   paste is left unsent ([`AgentInputOutcome::Unconfirmed`]).
//! - **Stop**: Escape, while a turn is running.
//! - **Approval**: the option whose label is the card's, by its key: Claude's digit, Codex's letter or Escape.
//! - **Question**: per question, its tab confirmed by its text and options; a digit picks a single-select option (the
//!   picker moves on), digits toggle a multi-select question's boxes to the answer and Tab moves on; free text goes
//!   through `Type something.` (its digit, then a paste, then Enter or Tab: the row must hold the text, after the
//!   placeholder or in its place, or a paste's stand-in); the review tab's `1. Submit answers` takes Enter. A
//!   `Type something.` row that holds text before the answer starts is someone's draft: nothing is sent.

use std::time::Duration;

use ngterm_core::ghostty_config::binds::{Mods, PhysicalKey};
use ngterm_term::{ClipboardLocation, KeyInput, Paste, PasteSource};

use super::ChatQuestionItem;
use super::matchers::{Agent, OptionKey, Picker, Screen, fits, squeeze};
use crate::api::PaneInputError;

/// Why an action was refused: the text is empty.
pub const NOTHING_TO_SEND: &str = "there is nothing to send";

/// Why an action was refused: the pane runs no agent this core can drive.
pub const NO_AGENT_TO_DRIVE: &str = "the pane runs no agent the chat can answer";

/// Why an action was refused: another is still on its way to the pane.
pub const ACTION_IN_FLIGHT: &str = "another action is on its way to the pane";

/// Why a question's answer was refused: it does not fit the question.
pub const BAD_ANSWER: &str = "the answer does not fit the question";

/// Why a paste was refused: the text holds the end of a bracketed paste, or the program takes no bracketed paste.
pub const UNSAFE_PASTE: &str = "the text cannot be pasted safely";

/// What became of a chat action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AgentInputOutcome {
    /// The keys went to the program, and the screen showed what the action expected.
    Sent,
    /// The screen does not show what the action answers (no such dialog, option or question, a prompt that holds a
    /// draft typed at the terminal, or a screen the core can't read): nothing was sent. Offer "Answer in terminal".
    NotOnScreen,
    /// A dialog is open, so a prompt or Stop would answer it: nothing was sent. Answer the dialog first.
    DialogOpen,
    /// The lead could not be taken (another client claimed it, the attach was lost, or no answer in time): nothing
    /// was sent.
    LeadRefused,
    /// Keys were sent, but then the screen was not what the action expected (the dialog stayed up, a step's result
    /// did not show, the lead went): look at the terminal.
    Unconfirmed,
}

/// One question's answer: the option labels picked (one for a single-select question), and the text typed into its
/// "Type something." option.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct QuestionAnswer {
    pub picked: Vec<String>,
    pub other: Option<String>,
}

/// How long an action waits, settles and looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InputTiming {
    /// The quiet that makes a screen settled.
    pub(crate) quiet: Duration,
    /// The longest a settle waits for quiet before reading what is there.
    pub(crate) settle_max: Duration,
    /// The longest a claim waits for the attach's role report.
    pub(crate) lead_timeout: Duration,
    /// How often the waits look.
    pub(crate) poll: Duration,
}

impl Default for InputTiming {
    fn default() -> Self {
        Self {
            quiet: Duration::from_millis(150),
            settle_max: Duration::from_secs(1),
            lead_timeout: Duration::from_secs(10),
            poll: Duration::from_millis(20),
        }
    }
}

/// A chat action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Action {
    Prompt(String),
    Interrupt,
    Approval(String),
    Question(Vec<ChatQuestionItem>, Vec<QuestionAnswer>),
}

impl Action {
    /// Refuses an action that can't be sent whatever the screen shows.
    ///
    /// # Errors
    ///
    /// [`PaneInputError::Refused`] for an empty prompt, or answers that don't fit their questions.
    pub(crate) fn check(&self) -> Result<(), PaneInputError> {
        let refused = |reason: &str| PaneInputError::Refused {
            reason: reason.to_owned(),
        };
        match self {
            Self::Prompt(text) if text.trim().is_empty() => Err(refused(NOTHING_TO_SEND)),
            Self::Question(items, answers) if !answers_fit(items, answers) => Err(refused(BAD_ANSWER)),
            _ => Ok(()),
        }
    }
}

/// Whether each answer fits its question: labels it offers, one pick (or the text) for a single-select one, and every
/// option within a digit.
fn answers_fit(items: &[ChatQuestionItem], answers: &[QuestionAnswer]) -> bool {
    !items.is_empty()
        && items.len() == answers.len()
        && items.iter().zip(answers).all(|(item, answer)| {
            let known = answer
                .picked
                .iter()
                .all(|label| item.options.iter().any(|option| option.label == *label));
            let other = answer.other.as_deref().map(str::trim);
            let count = answer.picked.len() + usize::from(other.is_some());
            let single = item.multi_select || count == 1;
            known && single && other != Some("") && item.options.len() < 9
        })
}

/// A key the plan sends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Key {
    Digit(u8),
    Letter(char),
    Enter,
    Escape,
    Tab,
    /// A paste of the text, bracketed as the program asked.
    Paste(String),
}

impl From<OptionKey> for Key {
    fn from(key: OptionKey) -> Self {
        match key {
            OptionKey::Digit(digit) => Self::Digit(digit),
            OptionKey::Letter(letter) => Self::Letter(letter),
            OptionKey::Escape => Self::Escape,
        }
    }
}

impl Key {
    /// Sends the key into `terminal`, encoded for the program's modes.
    ///
    /// # Errors
    ///
    /// [`PaneInputError`] from the terminal; [`PaneInputError::Refused`] with [`UNSAFE_PASTE`] for a paste the
    /// terminal would not frame safely.
    pub(crate) fn send(&self, terminal: &ngterm_term::Pane) -> Result<(), PaneInputError> {
        let press = |name: &str, text: &str| -> Result<(), PaneInputError> {
            let key = PhysicalKey::new(name).unwrap_or(PhysicalKey::UNIDENTIFIED);
            terminal.key(&KeyInput::press(key, Mods::empty(), text))?;
            Ok(())
        };
        match self {
            Self::Digit(digit) => press(&format!("digit_{digit}"), &digit.to_string()),
            Self::Letter(letter) => press(&format!("key_{letter}"), &letter.to_string()),
            Self::Enter => press("enter", ""),
            Self::Escape => press("escape", ""),
            Self::Tab => press("tab", ""),
            Self::Paste(text) => {
                match terminal.paste(text, PasteSource::Clipboard, ClipboardLocation::Standard, false)? {
                    Paste::Written | Paste::Empty => Ok(()),
                    Paste::Unsafe => Err(PaneInputError::Refused {
                        reason: UNSAFE_PASTE.to_owned(),
                    }),
                }
            }
        }
    }
}

/// What the plan wants after a look.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Next {
    /// Send these keys; then, when `look`, settle and look again, otherwise the action is [`AgentInputOutcome::Sent`].
    Send { keys: Vec<Key>, look: bool },
    /// The action is over.
    Done(AgentInputOutcome),
}

/// Where a question's answer stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Asking {
    /// Its tab is to be confirmed, then answered (a multi-select one, a box at a time).
    Picking,
    /// The digit of "Type something." went: its row should be the picked one.
    Focusing,
    /// The text went: the row should hold it.
    Typed,
}

/// The question being answered: which, where its answer stands, and which of its boxes were toggled.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Asked {
    index: usize,
    asking: Asking,
    toggled: Vec<bool>,
}

impl Asked {
    fn new(index: usize, item: &ChatQuestionItem) -> Self {
        Self {
            index,
            asking: Asking::Picking,
            toggled: vec![false; item.options.len()],
        }
    }

    fn at(self, asking: Asking) -> Self {
        Self { asking, ..self }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    Start,
    /// A prompt's text was pasted.
    Pasted,
    /// An approval's key went to this dialog.
    Answered(super::matchers::Dialog),
    /// A question is being answered.
    Asking(Asked),
    /// Every question is answered: the review, or the picker gone.
    Reviewing,
    /// The review's Enter went.
    Submitted,
}

/// An action's sequence, decided one look at the screen at a time.
#[derive(Debug, Clone)]
pub(crate) struct Plan {
    action: Action,
    step: Step,
    sent: bool,
}

impl Plan {
    pub(crate) fn new(action: Action) -> Self {
        Self {
            action,
            step: Step::Start,
            sent: false,
        }
    }

    /// Whether keys went (as [`Plan::mark_sent`] records).
    pub(crate) fn sent(&self) -> bool {
        self.sent
    }

    /// The outcome when the sequence can't go on (the lead lost, the screen not as expected): nothing sent, or
    /// something.
    pub(crate) fn stopped(&self, before: AgentInputOutcome) -> AgentInputOutcome {
        if self.sent {
            AgentInputOutcome::Unconfirmed
        } else {
            before
        }
    }

    /// The next move, given the screen now.
    pub(crate) fn next(&mut self, screen: &Screen) -> Next {
        match &self.action {
            Action::Prompt(text) => self.prompt(text.clone(), screen),
            Action::Interrupt => interrupt(screen),
            Action::Approval(label) => self.approval(&label.clone(), screen),
            Action::Question(items, answers) => {
                let (items, answers) = (items.clone(), answers.clone());
                self.question(&items, &answers, screen)
            }
        }
    }

    /// The keys [`Plan::next`] gave went into the terminal.
    pub(crate) fn mark_sent(&mut self) {
        self.sent = true;
    }

    fn not_on_screen(&self) -> Next {
        Next::Done(self.stopped(AgentInputOutcome::NotOnScreen))
    }

    fn prompt(&mut self, text: String, screen: &Screen) -> Next {
        match (&self.step, screen) {
            (Step::Start, Screen::Prompt { composer, .. }) if composer.is_empty() => {
                self.step = Step::Pasted;
                Next::Send {
                    keys: vec![Key::Paste(text)],
                    look: true,
                }
            }
            (Step::Start, screen) if screen.is_dialog() => Next::Done(AgentInputOutcome::DialogOpen),
            (Step::Pasted, Screen::Prompt { composer, .. }) if holds_paste(composer, &text) => Next::Send {
                keys: vec![Key::Enter],
                look: false,
            },
            _ => self.not_on_screen(),
        }
    }

    fn approval(&mut self, label: &str, screen: &Screen) -> Next {
        match (&self.step, screen) {
            (Step::Start, Screen::Permission(dialog)) => match dialog.option(label) {
                Some(option) => {
                    let key = option.key.into();
                    self.step = Step::Answered(dialog.clone());
                    Next::Send {
                        keys: vec![key],
                        look: true,
                    }
                }
                None => self.not_on_screen(),
            },
            (Step::Answered(answered), Screen::Permission(dialog)) if dialog == answered => {
                Next::Done(AgentInputOutcome::Unconfirmed)
            }
            (Step::Answered(_), _) => Next::Done(AgentInputOutcome::Sent),
            _ => self.not_on_screen(),
        }
    }

    fn question(&mut self, items: &[ChatQuestionItem], answers: &[QuestionAnswer], screen: &Screen) -> Next {
        match self.step.clone() {
            Step::Start => {
                self.step = Step::Asking(Asked::new(0, &items[0]));
                self.question(items, answers, screen)
            }
            Step::Asking(asked) => {
                let Screen::Question(picker) = screen else {
                    return self.not_on_screen();
                };
                let other = answers[asked.index].other.as_deref().map(str::trim);
                if !shows(picker, &items[asked.index], asked.asking, other) {
                    return self.not_on_screen();
                }
                self.ask(items, answers, picker, asked)
            }
            Step::Reviewing => match screen {
                Screen::Review { on_submit: true } => {
                    self.step = Step::Submitted;
                    Next::Send {
                        keys: vec![Key::Enter],
                        look: true,
                    }
                }
                Screen::Review { .. } | Screen::Question(_) | Screen::Unknown => self.not_on_screen(),
                _ => Next::Done(AgentInputOutcome::Sent),
            },
            Step::Submitted => match screen {
                Screen::Review { .. } | Screen::Question(_) | Screen::Unknown => {
                    Next::Done(AgentInputOutcome::Unconfirmed)
                }
                _ => Next::Done(AgentInputOutcome::Sent),
            },
            Step::Pasted | Step::Answered(_) => self.not_on_screen(),
        }
    }

    /// The next key of the question `asked`, whose tab `picker` is on screen.
    fn ask(&mut self, items: &[ChatQuestionItem], answers: &[QuestionAnswer], picker: &Picker, asked: Asked) -> Next {
        let (item, answer) = (&items[asked.index], &answers[asked.index]);
        let other = answer.other.as_deref().map(str::trim);
        // "Type something.", after the question's own options
        let typing = picker.options.get(item.options.len());
        match asked.asking {
            Asking::Picking if !item.multi_select => match answer.picked.first() {
                Some(label) => self.move_on(items, asked.index, Key::Digit(digit_of(item, label))),
                None => self.go(asked.at(Asking::Focusing), Key::Digit(typing_digit(item))),
            },
            Asking::Picking => self.toggle(items, answer, picker, asked),
            Asking::Focusing => match other {
                Some(text) if typing.is_some_and(|option| option.cursor) => {
                    self.go(asked.at(Asking::Typed), Key::Paste(text.to_owned()))
                }
                _ => self.not_on_screen(),
            },
            // [`shows`] found the text in its row
            Asking::Typed => {
                let key = if item.multi_select { Key::Tab } else { Key::Enter };
                self.move_on(items, asked.index, key)
            }
        }
    }

    /// A multi-select question's next box to toggle to the answer (each at most once), then its typed text, then Tab.
    fn toggle(
        &mut self,
        items: &[ChatQuestionItem],
        answer: &QuestionAnswer,
        picker: &Picker,
        mut asked: Asked,
    ) -> Next {
        let item = &items[asked.index];
        for (at, option) in item.options.iter().enumerate() {
            let wanted = answer.picked.contains(&option.label);
            if picker.options[at].checked == Some(wanted) {
                continue;
            }
            if std::mem::replace(&mut asked.toggled[at], true) {
                // toggled once already and still not as wanted
                return Next::Done(AgentInputOutcome::Unconfirmed);
            }
            return self.go(asked, Key::Digit(digit_of(item, &option.label)));
        }
        if answer.other.is_some() {
            return self.go(asked.at(Asking::Focusing), Key::Digit(typing_digit(item)));
        }
        self.move_on(items, asked.index, Key::Tab)
    }

    /// Sends `key`, the question then standing as `asked`.
    fn go(&mut self, asked: Asked, key: Key) -> Next {
        self.step = Step::Asking(asked);
        look_after(key)
    }

    /// Sends `key`, which leaves question `index` for the next one, or for the review after the last.
    fn move_on(&mut self, items: &[ChatQuestionItem], index: usize, key: Key) -> Next {
        self.step = items
            .get(index + 1)
            .map_or(Step::Reviewing, |next| Step::Asking(Asked::new(index + 1, next)));
        look_after(key)
    }
}

/// Sends `key`, then looks again.
fn look_after(key: Key) -> Next {
    Next::Send {
        keys: vec![key],
        look: true,
    }
}

fn interrupt(screen: &Screen) -> Next {
    match screen {
        Screen::Prompt { working: true, .. } => Next::Send {
            keys: vec![Key::Escape],
            look: false,
        },
        screen if screen.is_dialog() => Next::Done(AgentInputOutcome::DialogOpen),
        _ => Next::Done(AgentInputOutcome::NotOnScreen),
    }
}

/// The digit of `item`'s "Type something.", after its options.
fn typing_digit(item: &ChatQuestionItem) -> u8 {
    u8::try_from(item.options.len() + 1).unwrap_or(u8::MAX)
}

/// The digit of `label` in `item` (checked to exist by [`Action::check`]).
fn digit_of(item: &ChatQuestionItem, label: &str) -> u8 {
    let at = item
        .options
        .iter()
        .position(|option| option.label == label)
        .unwrap_or_default();
    u8::try_from(at + 1).unwrap_or(u8::MAX)
}

/// "Type something.", whitespace aside.
const TYPING: &str = "Typesomething";

/// Whether `picker` is at `item`'s tab: its question, its options in order (each option's label and description as
/// the screen wraps them), then the free-text row, boxed the way the question picks. That row shows the placeholder
/// alone while the answer is picked (any more is a draft typed at the terminal) and once focused, and the text
/// `other` once typed: after the placeholder or in its place, or as a paste's stand-in.
fn shows(picker: &Picker, item: &ChatQuestionItem, asking: Asking, other: Option<&str>) -> bool {
    if !fits(&picker.question, &item.question) || picker.multi() != item.multi_select {
        return false;
    }
    let listed = item.options.iter().zip(&picker.options).all(|(option, shown)| {
        let expected = format!("{} {}", option.label, option.description.as_deref().unwrap_or_default());
        fits(&shown.text, &expected)
    });
    let typing = picker.options.get(item.options.len()).is_some_and(|shown| {
        let row = squeeze(&shown.text);
        match (asking, other) {
            (Asking::Picking, _) => untouched(&row),
            (Asking::Focusing, _) => row.starts_with(TYPING),
            (Asking::Typed, Some(text)) => row.contains(&squeeze(text)) || stand_in(&row).is_some(),
            (Asking::Typed, None) => false,
        }
    });
    listed && picker.options.len() > item.options.len() && typing
}

/// Whether the free-text row (squeezed) shows its placeholder and nothing typed: `Type something.`, and in a
/// multi-select question the `Submit` drawn under it.
fn untouched(row: &str) -> bool {
    row.strip_prefix(TYPING).is_some_and(|rest| {
        let rest = rest.strip_prefix('.').unwrap_or(rest);
        rest.is_empty() || rest == "Submit"
    })
}

/// Where a TUI's stand-in for a long paste starts in `squeezed` (Claude's `[Pasted text #1 +12 lines]`, Codex's
/// `[Pasted Content 1200 chars]`, whitespace aside).
fn stand_in(squeezed: &str) -> Option<usize> {
    ["[Pastedtext#", "[PastedContent"]
        .iter()
        .find_map(|start| squeezed.find(start))
}

/// Whether a prompt that was empty before the paste holds `text` now: the text itself (the screen wraps it where it
/// likes), or nothing but a stand-in for it.
fn holds_paste(composer: &str, text: &str) -> bool {
    let shown = squeeze(composer);
    if shown.is_empty() {
        return false;
    }
    shown == squeeze(text) || (stand_in(&shown) == Some(0) && shown.ends_with(']') && shown.matches('[').count() == 1)
}

/// The labels the card offers for `agent`'s approval: the dialog's on `screen`, or the agent's usual ones.
pub(crate) fn approval_labels(agent: Agent, screen: &Screen) -> Vec<String> {
    match screen {
        Screen::Permission(dialog) => dialog.options.iter().map(|option| option.label.clone()).collect(),
        _ => agent.fallback_labels(),
    }
}
