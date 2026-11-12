//! The chat's actions on a pane's agent, as keys into its terminal: a prompt, Stop, an approval's answer and the
//! answers to Claude's `AskUserQuestion` or Codex's `request_user_input`. Each runs as one sequence on the core's actor
//! (`crate::actor::inputs`):
//!
//! 1. **Lead.** A pane that is not its session's leader claims the lead first (as Take control does) and waits for
//!    the attach to report [`LeadRole::Leader`](ngterm_core::live::lead::LeadRole); a claim that reports another role,
//!    is replaced or lost, or has no answer within `InputTiming::lead_timeout` sends nothing
//!    ([`AgentInputOutcome::LeadRefused`]). Only the leader's input reaches the program, and its grid lays the TUI
//!    out, so the screen read next is laid out for this pane.
//! 2. **Settle.** No new output, no new frame and no new grid for `InputTiming::quiet` (the redraw after the claim's
//!    resize has landed), or `InputTiming::settle_max` at most: a working agent's spinner never goes quiet.
//! 3. **Match.** The agent's screen matcher (`crate::chat::matchers`) reads the program's screen itself
//!    ([`ngterm_term::Pane::active_screen`]: the terminal's active area with its screen generation), never what the
//!    terminal view shows: a view scrolled back (onto an old copy of a dialog, say), a selection, a search's
//!    highlights or anything the app draws over the view play no part. A screen the program holds (synchronized
//!    output, until the hold's timeout ends it) or the VT's reflow after a new column or row count the program has
//!    not redrawn for yet is not read: the action settles and looks again. The same grid again or a new cell size
//!    alone reflows nothing, so an idle screen stays readable. The action's `Plan` decides from the screen read:
//!    the keys to send, or the outcome.
//! 4. **Send.** The keys go into the same terminal the screen was read from, on the actor, with nothing awaited
//!    between the read and the send: the role and the attach are checked again right before (the terminal's pending
//!    role reports taken first), so a lead lost or an attach replaced meanwhile sends nothing. Each key goes only
//!    while the program's screen is still the one read, checked and sent under the terminal's lock
//!    ([`ngterm_term::Pane::key_if`]): output or a resize that landed after the read sends nothing, and the screen is
//!    settled and read again.
//! 5. **Confirm.** A step that needs to see what its keys did (a paste before its Enter, each question of the picker, a
//!    toggle, a dialog's answer) settles and matches again, until the screen shows what the keys were to do or
//!    `InputTiming::confirm` has gone by since they went. Until then any other screen is pending (`Next::Wait`),
//!    the screen they were sent to above all (no output since: its reply is still on the network); no key is sent
//!    again. At the deadline the action is [`AgentInputOutcome::Unconfirmed`].
//!
//! Every outcome but [`AgentInputOutcome::Sent`] and [`AgentInputOutcome::Unconfirmed`] means nothing was sent.
//!
//! The keys (Task 1's recordings, `crates/ngterm-core/tests/fixtures/agent_chat/README.md`):
//!
//! - **Prompt**: a bracketed paste of the whole text, then Enter; `/name args` and Codex's `$skill` go the same way,
//!   since both TUIs run a pasted command. It needs the prompt on screen, empty (a draft left in it at the terminal
//!   would be sent along: [`AgentInputOutcome::NotOnScreen`]), and no dialog ([`AgentInputOutcome::DialogOpen`]).
//!   The Enter goes only once the prompt holds the text pasted, or the stand-in a TUI shows for a long paste
//!   (Claude's `[Pasted text #1 +12 lines]`, Codex's `[Pasted Content 1200 chars]`), and nothing else; if it does not
//!   by the confirm deadline, the paste is left unsent ([`AgentInputOutcome::Unconfirmed`]).
//! - **Stop**: Escape, while a turn is running.
//! - **Approval**: the option whose label is the card's, by its key: Claude's digit, Codex's letter or Escape; only to
//!   the dialog that asks about the card's call (`Dialog::asks` of what
//!   `shown_subjects` says the dialog shows: the command or URL the hook reported, or a file tool's path relative to
//!   the agent's working directory, read in the rows above its options). A dialog about another call (the card's
//!   answered elsewhere and the agent asks the next) or a card that names nothing a dialog shows (an MCP tool's) is
//!   [`AgentInputOutcome::NotOnScreen`]: a label alone (Claude's `Yes`) would answer any dialog. The card stays up:
//!   the origin's `approval_done`, or its next prompt, ends it. Two calls a dialog shows alike (the same command, two
//!   edits of one file) can't be told apart on screen.
//! - **Question**: per question, its tab confirmed by its text and options (a picker taller than the screen by what
//!   is left of its text, and only while no other question of the call lists the same options); a digit picks a
//!   single-select option (the picker moves on), digits toggle a multi-select question's boxes to the answer and Tab moves on; free text goes
//!   through `Type something.` (its digit, then a paste, then Enter or Tab: the row must hold the text, after the
//!   placeholder or in its place, or a paste's stand-in); the review tab's `1. Submit answers` takes Enter. A
//!   `Type something.` row that holds text before the answer starts is someone's draft: nothing is sent.
//! - **Codex's question** (`request_user_input`, single-select only): per question, the picker confirmed at it by
//!   `Question N/M`, its text and options, notes closed; an option's digit picks it and moves on (submits all on the
//!   last). Free text is `None of the above` with a note: Down a row at a time (each move seen before the next) to that
//!   option, Tab opens its notes row, the paste goes once the row shows empty, and Enter once it shows the text (moves
//!   on, or submits all on the last). Either way the end is the picker gone.

use std::time::Duration;

use ngterm_core::ghostty_config::binds::{Mods, PhysicalKey};
use ngterm_term::{ClipboardLocation, KeyInput, Paste, PasteSource};

use super::matchers::{Agent, OptionKey, Picker, Screen, fits, squeeze};
use super::{ChatApproval, ChatQuestionItem, ChatQuestionOption};
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
    /// did not show in time, the lead went): look at the terminal.
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
    /// The longest an action waits, after its keys went, for the screen to show what they did.
    pub(crate) confirm: Duration,
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
            confirm: Duration::from_secs(5),
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
    /// The card's prompt and the label of the option picked.
    Approval(ChatApproval, String),
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
    Down,
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
    /// Sends the key into `terminal`, encoded for the program's modes, while the program's screen is still the one
    /// read at `generation` ([`ngterm_term::ActiveScreen::generation`]); whether it went (`false`: the screen changed
    /// since, nothing went).
    ///
    /// # Errors
    ///
    /// [`PaneInputError`] from the terminal; [`PaneInputError::Refused`] with [`UNSAFE_PASTE`] for a paste the
    /// terminal would not frame safely.
    pub(crate) fn send_if(&self, terminal: &ngterm_term::Pane, generation: u64) -> Result<bool, PaneInputError> {
        let press = |name: &str, text: &str| -> Result<bool, PaneInputError> {
            let key = PhysicalKey::new(name).unwrap_or(PhysicalKey::UNIDENTIFIED);
            Ok(terminal
                .key_if(generation, &KeyInput::press(key, Mods::empty(), text))?
                .is_some())
        };
        match self {
            Self::Digit(digit) => press(&format!("digit_{digit}"), &digit.to_string()),
            Self::Letter(letter) => press(&format!("key_{letter}"), &letter.to_string()),
            Self::Enter => press("enter", ""),
            Self::Escape => press("escape", ""),
            Self::Tab => press("tab", ""),
            Self::Down => press("arrow_down", ""),
            Self::Paste(text) => {
                match terminal.paste_if(
                    generation,
                    text,
                    PasteSource::Clipboard,
                    ClipboardLocation::Standard,
                    false,
                )? {
                    Some(Paste::Written | Paste::Empty) => Ok(true),
                    None => Ok(false),
                    Some(Paste::Unsafe) => Err(PaneInputError::Refused {
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
    /// The screen does not show yet what the keys sent did: settle and look again, sending nothing, until
    /// [`InputTiming::confirm`] has gone by since they went ([`AgentInputOutcome::Unconfirmed`] then).
    Wait,
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
    /// Codex: a Down went from the option at this index (0-based) toward `None of the above`.
    Moving(usize),
    /// Codex: Tab went on `None of the above`: its notes row should be open, empty.
    Noting,
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
    /// Whose screens the plan reads: a question is answered the way that agent's picker takes it.
    agent: Agent,
    step: Step,
    sent: bool,
}

impl Plan {
    pub(crate) fn new(action: Action, agent: Agent) -> Self {
        Self {
            action,
            agent,
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
            Action::Approval(card, label) => {
                let (card, label) = (card.clone(), label.clone());
                self.approval(&card, &label, screen)
            }
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

    /// A screen that is not what the step needs: before any key went, nothing is sent; after, it is pending (what the
    /// keys did may not have shown yet).
    fn unmet(&self) -> Next {
        if self.sent { Next::Wait } else { self.not_on_screen() }
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
            // before the paste: a draft in the prompt, or no prompt; after it: the paste not (all) shown yet
            _ => self.unmet(),
        }
    }

    fn approval(&mut self, card: &ChatApproval, label: &str, screen: &Screen) -> Next {
        match (&self.step, screen) {
            (Step::Start, Screen::Permission(dialog)) => match dialog.option(label) {
                // the dialog asks about the card's call, not the next one the agent asked after it
                Some(option) if shown_subjects(card).iter().any(|subject| dialog.asks(subject)) => {
                    let key = option.key.into();
                    self.step = Step::Answered(dialog.clone());
                    Next::Send {
                        keys: vec![key],
                        look: true,
                    }
                }
                _ => self.not_on_screen(),
            },
            // the dialog the key went to, still up
            (Step::Answered(answered), Screen::Permission(dialog)) if dialog == answered => Next::Wait,
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
                    return self.unmet();
                };
                if self.agent == Agent::Codex {
                    if !codex_shows(picker, items, asked.index) {
                        return self.unmet();
                    }
                    return self.ask_codex(items, answers, picker, asked);
                }
                let other = answers[asked.index].other.as_deref().map(str::trim);
                if !shows(picker, &items[asked.index], asked.asking, other)
                    || (picker.cut && !told_apart(items, asked.index))
                {
                    return self.unmet();
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
                Screen::Review { .. } | Screen::Question(_) | Screen::Unknown => self.unmet(),
                _ => Next::Done(AgentInputOutcome::Sent),
            },
            Step::Submitted => match screen {
                Screen::Review { .. } | Screen::Question(_) | Screen::Unknown => Next::Wait,
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
                _ => self.unmet(),
            },
            // [`shows`] found the text in its row
            Asking::Typed => {
                let key = if item.multi_select { Key::Tab } else { Key::Enter };
                self.move_on(items, asked.index, key)
            }
            Asking::Moving(_) | Asking::Noting => self.unmet(),
        }
    }

    /// The next key of Codex's question `asked`, which `picker` shows ([`codex_shows`]).
    fn ask_codex(
        &mut self,
        items: &[ChatQuestionItem],
        answers: &[QuestionAnswer],
        picker: &Picker,
        asked: Asked,
    ) -> Next {
        let (item, answer) = (&items[asked.index], &answers[asked.index]);
        // `None of the above`, after the question's own options
        let none = item.options.len();
        let cursor = picker.options.iter().position(|option| option.cursor);
        let notes = picker.notes.as_deref();
        let other = answer.other.as_deref().map(str::trim);
        match (asked.asking, notes, other) {
            // with notes open before any key of this question went (someone typing one at the terminal), nothing
            (Asking::Picking, None, _) => match answer.picked.first() {
                Some(label) => self.move_on(items, asked.index, Key::Digit(digit_of(item, label))),
                None => self.toward_notes(asked, cursor, none),
            },
            // each Down seen landing a row lower before the next goes
            (Asking::Moving(from), None, _) if cursor == Some(from + 1) => self.toward_notes(asked, cursor, none),
            (Asking::Noting, Some(""), Some(text)) if cursor == Some(none) => {
                self.go(asked.at(Asking::Typed), Key::Paste(text.to_owned()))
            }
            (Asking::Typed, Some(shown), Some(text)) if cursor == Some(none) && holds_note(shown, text) => {
                self.move_on(items, asked.index, Key::Enter)
            }
            _ => self.unmet(),
        }
    }

    /// Toward Codex's `None of the above` (option `none`) from the highlighted option `cursor`: Down a row, or Tab to
    /// open its notes once there.
    fn toward_notes(&mut self, asked: Asked, cursor: Option<usize>, none: usize) -> Next {
        match cursor {
            Some(at) if at == none => self.go(asked.at(Asking::Noting), Key::Tab),
            Some(at) if at < none => self.go(asked.at(Asking::Moving(at)), Key::Down),
            _ => self.unmet(),
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
            if asked.toggled[at] {
                // toggled once already and not shown yet: never toggled again
                return Next::Wait;
            }
            asked.toggled[at] = true;
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
    let asked = if picker.cut {
        squeeze(&item.question).ends_with(&squeeze(&picker.question))
    } else {
        fits(&picker.question, &item.question)
    };
    if !asked || picker.multi() != item.multi_select {
        return false;
    }
    let listed = item
        .options
        .iter()
        .zip(&picker.options)
        .all(|(option, shown)| fits(&shown.text, &listing(option)));
    let typing = picker.options.get(item.options.len()).is_some_and(|shown| {
        let row = squeeze(&shown.text);
        match (asking, other) {
            (Asking::Picking, _) => untouched(&row),
            (Asking::Focusing, _) => row.starts_with(TYPING),
            (Asking::Typed, Some(text)) => row.contains(&squeeze(text)) || stand_in(&row).is_some(),
            (Asking::Typed, None) | (Asking::Moving(_) | Asking::Noting, _) => false,
        }
    });
    listed && picker.options.len() > item.options.len() && typing
}

/// `None of the above`, whitespace aside.
const NONE_OF_THE_ABOVE: &str = "Noneoftheabove";

/// Whether Codex's `picker` is at question `index` of `items`: `Question N/M` says so, the question reads as asked,
/// and the options are the question's (label and description as the screen wraps them) and then `None of the above`.
fn codex_shows(picker: &Picker, items: &[ChatQuestionItem], index: usize) -> bool {
    let item = &items[index];
    let position = (u8::try_from(index + 1).ok(), u8::try_from(items.len()).ok());
    let at = picker
        .position
        .is_some_and(|(shown, count)| position == (Some(shown), Some(count)));
    let listed = picker.options.len() == item.options.len() + 1
        && item
            .options
            .iter()
            .zip(&picker.options)
            .all(|(option, shown)| fits(&shown.text, &listing(option)));
    let none = picker
        .options
        .last()
        .is_some_and(|shown| squeeze(&shown.text).starts_with(NONE_OF_THE_ABOVE));
    let single = !item.multi_select && !picker.multi();
    at && single && !picker.cut && fits(&picker.question, &item.question) && listed && none
}

/// Whether Codex's notes row, opened empty, holds `text` now: the text (wrapped where the screen likes), or a paste's
/// stand-in for it.
fn holds_note(shown: &str, text: &str) -> bool {
    let shown = squeeze(shown);
    shown == squeeze(text) || (stand_in(&shown) == Some(0) && shown.ends_with(']'))
}

/// An option as the picker lists it: its label, then its description.
fn listing(option: &ChatQuestionOption) -> String {
    format!("{} {}", option.label, option.description.as_deref().unwrap_or_default())
}

/// Whether question `index`'s options, as the picker lists them, are no other question's of the call: with the tab
/// row and the question's start scrolled off ([`Picker::cut`]), only they tell which question the picker is at.
fn told_apart(items: &[ChatQuestionItem], index: usize) -> bool {
    let listed = |item: &ChatQuestionItem| {
        item.options
            .iter()
            .map(|option| squeeze(&listing(option)))
            .collect::<Vec<_>>()
    };
    let own = listed(&items[index]);
    items
        .iter()
        .enumerate()
        .all(|(at, item)| at == index || listed(item) != own)
}

/// Whether the free-text row (squeezed) shows its placeholder and nothing typed: `Type something.`, and in a
/// multi-select question the button drawn under it (`Submit` on the last question, `Next` on one before it).
fn untouched(row: &str) -> bool {
    row.strip_prefix(TYPING).is_some_and(|rest| {
        let rest = rest.strip_prefix('.').unwrap_or(rest);
        rest.is_empty() || rest == "Submit" || rest == "Next"
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

/// Claude's tools whose dialog names the file they write, by its path relative to the agent's working directory.
const FILE_TOOLS: [&str; 4] = ["Edit", "MultiEdit", "Write", "NotebookEdit"];

/// What a dialog that asks about `card`'s call shows of it, any one of which confirms the dialog: the command or URL
/// as the hook reported it; for a file tool, its path as Claude's dialog names it (relative to the working directory
/// the agent asked from, `../` included, as Node's `path.relative` makes it) or whole (Claude's verbose mode). Empty
/// for a card that names nothing (an MCP tool's): no dialog can be confirmed as its own.
pub(crate) fn shown_subjects(card: &ChatApproval) -> Vec<String> {
    let Some(detail) = card.detail.as_deref().filter(|detail| !detail.trim().is_empty()) else {
        return Vec::new();
    };
    let relative = FILE_TOOLS
        .contains(&card.tool.as_str())
        .then(|| card.cwd.as_deref().and_then(|cwd| relative_path(cwd, detail)))
        .flatten();
    relative.into_iter().chain([detail.to_owned()]).collect()
}

/// `path` relative to the directory `from`, both absolute POSIX paths, as Node's `path.relative` gives it: the
/// components past their common start, after a `..` for each of `from`'s left. `None` for a path that is not absolute
/// (a Windows one: only the whole path is matched) or that is `from` itself.
fn relative_path(from: &str, path: &str) -> Option<String> {
    let (from, path) = (components(from)?, components(path)?);
    let common = from.iter().zip(&path).take_while(|(a, b)| a == b).count();
    let parts: Vec<&str> = std::iter::repeat_n("..", from.len() - common)
        .chain(path[common..].iter().copied())
        .collect();
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// An absolute POSIX path's components, `.` and `..` resolved as Node's `path.resolve` does; `None` when it is not
/// absolute.
fn components(path: &str) -> Option<Vec<&str>> {
    let rest = path.strip_prefix('/')?;
    let mut parts = Vec::new();
    for part in rest.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    Some(parts)
}

/// The labels the card offers for `agent`'s approval: the dialog's on `screen`, or the agent's usual ones.
pub(crate) fn approval_labels(agent: Agent, screen: &Screen) -> Vec<String> {
    match screen {
        Screen::Permission(dialog) => dialog.options.iter().map(|option| option.label.clone()).collect(),
        _ => agent.fallback_labels(),
    }
}
