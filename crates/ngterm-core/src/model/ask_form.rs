//! The text field or choice list an ask shows between its message and its buttons, and the user's unsubmitted input
//! (upstream's `AskInputForm`, `AskChoiceForm`, `AskForm` and `AskDraft` in `Ask.swift`).
//!
//! The draft lives on the ask's slot owner, the session for a terminal ask and the window's modal slot for a GUI one,
//! so hiding or remounting the dialog keeps what the user typed or checked.

use std::collections::HashSet;

use ngterm_proto::AskChoice;
use unicode_segmentation::UnicodeSegmentation;

/// The most items one choice list takes (upstream's `ControlAskChoice.maxChoices`).
pub const MAX_ASK_CHOICES: usize = 50;

/// A one-line or multiline text field (upstream's `ControlAskInputMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskInputMode {
    /// One line, where Return answers.
    Line,
    /// An editing area, where Return inserts a newline and the submit chord answers.
    Multiline,
}

impl AskInputMode {
    /// The mode a wire name names.
    #[must_use]
    pub fn from_wire(raw: &str) -> Option<Self> {
        match raw {
            "line" => Some(Self::Line),
            "multiline" => Some(Self::Multiline),
            _ => None,
        }
    }
}

/// An ask's text field (upstream's `AskInputForm`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskInputForm {
    pub mode: AskInputMode,
    /// The field's initial text.
    pub value: String,
    /// The hint shown while the field is empty.
    pub placeholder: Option<String>,
}

impl AskInputForm {
    /// The cap on the prefill and the field's content, in UTF-8 bytes (upstream's `maxTextBytes`).
    pub const MAX_TEXT_BYTES: usize = 65_536;

    /// A field of `mode` with no prefill and no placeholder.
    #[must_use]
    pub fn new(mode: AskInputMode) -> Self {
        Self {
            mode,
            value: String::new(),
            placeholder: None,
        }
    }

    /// Whether `text` holds a C0 control or DEL, newline and tab excepted in a multiline field (upstream's
    /// `hasForbiddenControls`).
    #[must_use]
    pub fn has_forbidden_controls(text: &str, mode: AskInputMode) -> bool {
        text.chars().any(|ch| {
            let control = u32::from(ch) < 0x20 || ch == '\u{7f}';
            control && (mode == AskInputMode::Line || (ch != '\n' && ch != '\t'))
        })
    }

    /// `text` cut so it fits [`Self::MAX_TEXT_BYTES`] (upstream's `clamped`): at the end of the last extended
    /// grapheme cluster that fits, as upstream walks Swift `Character`s, so a cluster straddling the cap goes whole.
    #[must_use]
    pub fn clamped(text: &str) -> &str {
        if text.len() <= Self::MAX_TEXT_BYTES {
            return text;
        }
        let end = text
            .grapheme_indices(true)
            .map(|(start, cluster)| start + cluster.len())
            .take_while(|&end| end <= Self::MAX_TEXT_BYTES)
            .last()
            .unwrap_or(0);
        &text[..end]
    }
}

/// An ask's radio or checkbox list (upstream's `AskChoiceForm`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskChoiceForm {
    pub choices: Vec<AskChoice>,
    /// Checkboxes; radio buttons otherwise.
    pub multiple: bool,
    /// The ids selected at open; a radio list holds at most one.
    pub checked: Vec<String>,
    /// The placeholder of a free-form row after the choices; `None` means no such row.
    pub other: Option<String>,
}

impl AskChoiceForm {
    /// A radio list over `choices` with nothing checked and no free-form row.
    #[must_use]
    pub fn new(choices: Vec<AskChoice>) -> Self {
        Self {
            choices,
            multiple: false,
            checked: Vec::new(),
            other: None,
        }
    }

    /// Whether `text` has no non-whitespace character (upstream's `isBlank`): the free-form row is disabled and
    /// unselected while its text is blank, and an answer never carries blank text.
    #[must_use]
    pub fn is_blank(text: &str) -> bool {
        text.chars().all(char::is_whitespace)
    }
}

/// What an ask shows between its message and its buttons (upstream's `AskForm`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AskForm {
    Input(AskInputForm),
    Choices(AskChoiceForm),
}

/// The user's unsubmitted input (upstream's `AskDraft`): an input field's text, or a choice list's checks, cursor and
/// free-form row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskDraft {
    /// The field text, or the free-form row's text in a choice list, held within [`AskInputForm::MAX_TEXT_BYTES`].
    text: String,
    /// The list item keyboard actions apply to; the free-form row follows the choices.
    cursor: usize,
    checked: HashSet<String>,
    /// The free-form row is part of the answer.
    other_selected: bool,
    choice_ids: Vec<String>,
    multiple: bool,
    has_other: bool,
    /// The radio item the free-form row took over from, restored when its text goes blank.
    previous_pick: Option<String>,
}

impl AskDraft {
    /// Starts from the form's initial values: the prefill, the checked ids, and a radio list's first item when
    /// nothing is checked. The cursor starts on the first checked item in the caller's order.
    #[must_use]
    pub fn new(form: Option<&AskForm>) -> Self {
        let mut draft = Self {
            text: String::new(),
            cursor: 0,
            checked: HashSet::new(),
            other_selected: false,
            choice_ids: Vec::new(),
            multiple: false,
            has_other: false,
            previous_pick: None,
        };
        match form {
            Some(AskForm::Input(input)) => AskInputForm::clamped(&input.value).clone_into(&mut draft.text),
            Some(AskForm::Choices(list)) => {
                draft.choice_ids = list.choices.iter().map(|choice| choice.id.clone()).collect();
                draft.multiple = list.multiple;
                draft.has_other = list.other.is_some();
                draft.checked = if list.multiple {
                    list.checked.iter().cloned().collect()
                } else {
                    list.checked
                        .first()
                        .or_else(|| draft.choice_ids.first())
                        .cloned()
                        .into_iter()
                        .collect()
                };
                draft.cursor = draft
                    .choice_ids
                    .iter()
                    .position(|id| draft.checked.contains(id))
                    .unwrap_or(0);
            }
            None => {}
        }
        draft
    }

    /// The field text, or the free-form row's text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Replaces the text, clamped to the cap. In a choice list a non-blank text selects the free-form row and a
    /// blank one unselects it, even when the text did not change.
    pub fn set_text(&mut self, text: &str) {
        AskInputForm::clamped(text).clone_into(&mut self.text);
        if !self.has_other {
            return;
        }
        if AskChoiceForm::is_blank(&self.text) {
            self.unselect_other();
        } else {
            self.select_other();
        }
    }

    /// The row keyboard actions apply to.
    #[must_use]
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Whether the caller's item `id` is checked.
    #[must_use]
    pub fn is_checked(&self, id: &str) -> bool {
        self.checked.contains(id)
    }

    /// Whether the free-form row is part of the answer.
    #[must_use]
    pub fn other_selected(&self) -> bool {
        self.other_selected
    }

    /// The checked caller ids in the caller's order; the free-form row answers through [`Self::other_text`].
    #[must_use]
    pub fn selected(&self) -> Vec<String> {
        self.choice_ids
            .iter()
            .filter(|id| self.checked.contains(*id))
            .cloned()
            .collect()
    }

    /// A free-form row whose text is not blank, so it can be selected.
    #[must_use]
    pub fn other_enabled(&self) -> bool {
        self.has_other && !AskChoiceForm::is_blank(&self.text)
    }

    /// The free-form row's text as typed while the row is selected.
    #[must_use]
    pub fn other_text(&self) -> Option<&str> {
        (self.other_selected && self.other_enabled()).then_some(self.text.as_str())
    }

    /// The cursor is on the free-form row, where the row's own editor takes the keys.
    #[must_use]
    pub fn cursor_on_other(&self) -> bool {
        self.has_other && self.cursor == self.choice_ids.len()
    }

    /// Steps the cursor by `offset`, stopping at the list ends; the free-form row is the last stop.
    pub fn move_cursor(&mut self, offset: isize) {
        let rows = self.choice_ids.len() + usize::from(self.has_other);
        if rows == 0 {
            return;
        }
        self.cursor = self.cursor.saturating_add_signed(offset).min(rows - 1);
    }

    /// Moves the cursor to `index`, then picks a radio item or flips a checkbox. The free-form row toggles only while
    /// enabled, and a picked radio row stays picked; an index past the rows changes nothing.
    pub fn toggle(&mut self, index: usize) {
        if self.has_other && index == self.choice_ids.len() {
            self.cursor = index;
            if !self.other_enabled() {
                return;
            }
            if self.multiple && self.other_selected {
                self.other_selected = false;
            } else {
                self.select_other();
            }
            return;
        }
        let Some(id) = self.choice_ids.get(index).cloned() else {
            return;
        };
        self.cursor = index;
        if !self.multiple {
            self.checked = HashSet::from([id]);
            self.other_selected = false;
            self.previous_pick = None;
            return;
        }
        if !self.checked.remove(&id) {
            self.checked.insert(id);
        }
    }

    /// [`Self::toggle`] on the cursor's row.
    pub fn toggle_cursor(&mut self) {
        self.toggle(self.cursor);
    }

    fn select_other(&mut self) {
        if self.other_selected {
            return;
        }
        self.other_selected = true;
        if self.multiple {
            return;
        }
        self.previous_pick = self.checked.drain().next();
    }

    fn unselect_other(&mut self) {
        if !self.other_selected {
            return;
        }
        self.other_selected = false;
        if self.multiple {
            return;
        }
        self.checked = self
            .previous_pick
            .take()
            .or_else(|| self.choice_ids.first().cloned())
            .into_iter()
            .collect();
    }
}
