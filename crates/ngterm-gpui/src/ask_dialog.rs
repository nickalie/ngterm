//! The question dialog (`ask.open`), after upstream's `Views/AskDialogView.swift` and `ControlServer+Ask.swift` (port
//! of the GTK build's `ask_dialog.rs`).
//!
//! One dialog view ([`AskDialog`]) serves both owners:
//! - a terminal ask waits on its session and is drawn inside the session's page above every pane and the HUD (the
//!   deck lays it out over the whole page or the pane it was opened over, [`shown_scope`]), in the terminal's font and
//!   colors, square, with plain block buttons;
//! - a GUI ask holds its window's modal slot and is drawn over the window ([`crate::window::Layer::Ask`]), covering the
//!   terminal area or the session or pane it is anchored to, as a native rounded panel with native buttons, while the
//!   rest of the window takes no clicks.
//!
//! Either way a dim layer covers the anchor and the panel is centered in it ([`crate::modal`]), except that a pane ask
//! over a shown session-wide program or page leaves it unshaded, as a pane-sized wash would split that one cover into
//! two tones at the divider (upstream's `askShadesFrame`, [`AskDialog::set_shaded`]); its pane-sized click layer stays.
//! Inside it: the title, the message, the form if the ask has one ([`form`]: a text field or editing area, or a radio
//! or checkbox list with an optional free-form row) and a row of equal-width buttons that falls back to a column when
//! the row does not fit, aligned as asked. The keyboard follows [`AskNavigation`]: Tab, Right and Down move forward,
//! Shift+Tab, Left and Up back, Return answers with the highlighted button, a button's hotkey letter answers with it,
//! Escape dismisses (`escaped`); a key with Ctrl, Alt or Super goes on to the window's key monitor. A choice list's Up
//! and Down move its cursor instead and Space picks or flips the cursor's row. Content taller than its anchor scrolls,
//! and the highlighted button is kept in view ([`reveal_offset`]); around a form the title and buttons stay put while
//! the form's region scrolls, unless even they do not fit, when the whole panel scrolls. Answers go to the core as
//! main-loop tasks, a button answering with the owner's draft ([`ngterm_core::model::AskDraft`], which the dialog
//! mirrors and edits through the core, so hiding and remounting keeps it); a stale id is ignored there. Element ids are
//! upstream's accessibility ids: `ask-dialog`, `ask-title`, `ask-message`, `ask-button-<id>` and the form's.
//!
//! The dialog takes the keyboard when it is allowed to (upstream's `wantsFocus`, [`wants_focus`]): a GUI ask as it
//! opens, a terminal ask while its session is shown and its window holds no picker or GUI ask, the quick terminal is
//! hidden and, for a pane ask, its pane is the focused one. The deck gives it the keyboard in place of the terminal it
//! covers and gives the terminal the keyboard back once the dialog ends. What holds the keyboard inside it
//! ([`AskDialog::focus_target`]) is its editor for an input ask, and for a choice list the free-form row's editor while
//! the cursor is on that row, else the dialog itself.

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::component::input::InputEvent;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Context, FocusHandle, FontWeight, HighlightStyle, Hsla, InteractiveElement as _, IntoElement,
    KeyDownEvent, Keystroke, MouseButton, ParentElement as _, Pixels, Render, ScrollHandle, SharedString, Size,
    StatefulInteractiveElement as _, Styled as _, StyledText, Subscription, UnderlineStyle, Window, canvas, div, point,
    px,
};
use ngterm_core::Core;
use ngterm_core::chrome::{ChromeColors, Rgb};
use ngterm_core::model::{
    AskAlignment, AskDraft, AskForm, AskInputMode, AskNavigation, AskStyle, OverlayPane, PendingAsk, Session,
};
use ngterm_proto::{AskButton, ControlAskOutcome, ControlAskResult};
use uuid::Uuid;

use crate::modal::{self, ButtonLook, NavKey};
use crate::split::{AskScope, Slot};
use crate::state::{AppState, Poster, post};
use crate::terminal::TerminalConfig;
use crate::terminal::input::{self, KeyStroke, Source, chord_key};

pub mod form;

pub use form::intercept_key;
use form::{DraftEdit, Editor, FieldKey, FieldLook, FitInputs, FormFit, RowLook};

/// The GUI dialog's spacing unit (upstream's `cell` for `.gui`).
const GUI_CELL: f64 = 8.0;
/// The GUI dialog's text size.
const GUI_TEXT: f32 = 13.;
/// The font size when neither the settings nor the terminal config name one.
const DEFAULT_FONT_SIZE: f64 = 13.0;

// MARK: - the look

/// The terminal's font and colors the terminal style draws in (upstream's `askFont`, `chromeText` and
/// `terminalColor`): the settings' font family and size, else the terminal config's, and the terminal config's
/// foreground and background.
#[derive(Debug, Clone, PartialEq)]
pub struct Look {
    pub family: Option<String>,
    /// Points.
    pub size: f64,
    pub foreground: Rgb,
    pub background: Rgb,
}

impl Look {
    /// The look the settings and the terminals' `config` give now.
    #[must_use]
    pub fn current(core: &Core, config: &TerminalConfig) -> Self {
        let settings = core.settings();
        let rgb = |color: ngterm_term::Rgb| Rgb::new(color.r, color.g, color.b);
        Self {
            family: settings
                .font_family
                .clone()
                .filter(|family| !family.trim().is_empty())
                .or_else(|| config.font.family.first().cloned()),
            size: settings
                .font_size
                .filter(|size| *size > 0.0)
                .or_else(|| Some(f64::from(config.font.size)).filter(|size| *size > 0.0))
                .unwrap_or(DEFAULT_FONT_SIZE),
            foreground: rgb(config.theme.foreground),
            background: rgb(config.theme.background),
        }
    }

    /// The font size in logical pixels (a point is 4/3 of a pixel, as the terminal counts it).
    #[must_use]
    pub fn font_pixels(&self) -> f64 {
        self.size * 4.0 / 3.0
    }

    /// One terminal cell across (upstream's `terminalCell`, `max(8, pointSize * 0.6)`).
    #[must_use]
    pub fn terminal_cell(&self) -> f64 {
        (self.font_pixels() * 0.6).max(8.0)
    }

    /// The spacing unit of a dialog in `style`.
    #[must_use]
    pub fn cell(&self, style: AskStyle) -> f64 {
        match style {
            AskStyle::Gui => GUI_CELL,
            AskStyle::Terminal => self.terminal_cell(),
        }
    }
}

// MARK: - keys and labels

/// What a key does to a dialog (upstream's `AskKey`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AskKey {
    Forward,
    Backward,
    /// A choice list's cursor up, else [`Self::Backward`].
    Up,
    /// A choice list's cursor down, else [`Self::Forward`].
    Down,
    /// Picks or flips a choice list's cursor row; nothing without a list.
    Toggle,
    Activate,
    Cancel,
    /// A letter, lower-cased.
    Hotkey(String),
}

/// The dialog's action for a key press (upstream's `AskKeyCatcher.key(for:)`); `None` for a key it ignores, and for
/// any key with Ctrl, Alt or Super held, which goes on to the window. `source` is what the platform said about the
/// press ([`input::current_source`]): a hotkey letter resolves from the physical key through [`resolved_hotkey`], so
/// a non-Latin layout answers by key position; text an input method committed as a key resolves as GPUI's keystroke
/// names it (an ASCII letter counts, as before the layout rule). The dialog's own keys only: a text field of the
/// dialog takes its keys before this runs, so a letter typed into one is never a hotkey.
#[must_use]
pub fn ask_key(keystroke: &Keystroke, source: &Source) -> Option<AskKey> {
    let modifiers = &keystroke.modifiers;
    if modifiers.control || modifiers.alt || modifiers.platform {
        return None;
    }
    // Up and Down stand apart from Left and Right here: a choice list moves its cursor with them
    match keystroke.key.as_str() {
        "up" => return Some(AskKey::Up),
        "down" => return Some(AskKey::Down),
        _ => {}
    }
    Some(match modal::nav_key(keystroke) {
        Some(NavKey::Forward) => AskKey::Forward,
        Some(NavKey::Backward) => AskKey::Backward,
        Some(NavKey::Activate) => AskKey::Activate,
        Some(NavKey::Cancel) => AskKey::Cancel,
        Some(NavKey::Space) => AskKey::Toggle,
        Some(NavKey::Letter(_)) | None => {
            let source = match source {
                Source::Text => &Source::Keystroke,
                source => source,
            };
            let stroke = KeyStroke::new(keystroke, source, false);
            AskKey::Hotkey(resolved_hotkey(stroke.keycode, stroke.unshifted, stroke.ascii_layout)?)
        }
    })
}

/// The lowercase ASCII letter a key press spells for a hotkey, `None` for any other key (upstream's
/// `AskNavigation.resolvedHotkey`): the key resolves as keymap chords do ([`chord_key`]), so a layout that types ASCII
/// answers by the letter it types (AZERTY's A is `a`) and any other by the US layout's letter at that position.
#[must_use]
pub fn resolved_hotkey(keycode: u32, produced: Option<char>, ascii_layout: bool) -> Option<String> {
    let key = chord_key(keycode, produced, ascii_layout)?;
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(letter), None) if letter.is_ascii_lowercase() => Some(key),
        _ => None,
    }
}

/// A button's label (upstream's `buttonLabel`): the text, the hotkey's first case-insensitive occurrence underlined
/// (else ` (X)` appended with the letter underlined), and whether it is bold; a terminal-style destructive button is
/// prefixed with `! ` and bold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ButtonLabel {
    pub text: String,
    /// The byte range underlined.
    pub underline: Option<Range<usize>>,
    pub bold: bool,
}

/// The label of `button` (module docs of [`ButtonLabel`]).
#[must_use]
pub fn button_label(button: &AskButton, terminal_destructive: bool) -> ButtonLabel {
    let prefix = if terminal_destructive { "! " } else { "" };
    let label = &button.label;
    let (text, underline) = match button.hotkey.as_deref() {
        Some(hotkey) => {
            let wanted = hotkey.to_lowercase();
            let found = label
                .char_indices()
                .find(|(_, ch)| ch.to_lowercase().to_string() == wanted);
            if let Some((at, ch)) = found {
                let start = prefix.len() + at;
                (format!("{prefix}{label}"), Some(start..start + ch.len_utf8()))
            } else {
                let upper = hotkey.to_uppercase();
                let start = prefix.len() + label.len() + 2;
                (format!("{prefix}{label} ({upper})"), Some(start..start + upper.len()))
            }
        }
        None => (format!("{prefix}{label}"), None),
    };
    ButtonLabel {
        text,
        underline,
        bold: terminal_destructive,
    }
}

/// How a button draws (upstream's `guiButton`/`terminalButton`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskButtonLook {
    /// A GUI button.
    Gui(ButtonLook),
    /// A terminal block at 12% foreground.
    Block,
    /// The highlighted terminal block: inverted.
    Inverted,
}

/// In the GUI style the highlighted button is the prominent one (destructive red when it is the destructive button)
/// and a destructive one is otherwise red text; in the terminal style the highlighted block is inverted.
#[must_use]
pub fn button_look(style: AskStyle, highlighted: bool, destructive: bool) -> AskButtonLook {
    match style {
        AskStyle::Gui => AskButtonLook::Gui(match (highlighted, destructive) {
            (true, true) => ButtonLook::Destructive,
            (true, false) => ButtonLook::Suggested,
            (false, true) => ButtonLook::DestructiveText,
            (false, false) => ButtonLook::Plain,
        }),
        AskStyle::Terminal if highlighted => AskButtonLook::Inverted,
        AskStyle::Terminal => AskButtonLook::Block,
    }
}

// MARK: - the button strip

/// Whether the buttons go in a row or a column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StripAxis {
    Row,
    Column,
}

/// The row's width: every button as wide as the widest (upstream's `AskButtonLayout.sharedWidth`).
#[must_use]
pub fn row_width(widest: f64, count: usize, spacing: f64) -> f64 {
    #[expect(clippy::cast_precision_loss, reason = "at most six buttons")]
    let count = count as f64;
    widest * count + spacing * (count - 1.0).max(0.0)
}

/// A row while it fits `available`, else a column (upstream's `ViewThatFits(in: .horizontal)`).
#[must_use]
pub fn strip_axis(widest: f64, count: usize, spacing: f64, available: f64) -> StripAxis {
    if row_width(widest, count, spacing) <= available {
        StripAxis::Row
    } else {
        StripAxis::Column
    }
}

// MARK: - answers and placement

/// How the user ended a dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// The button at this index, in the caller's order: it answers with the owner's draft, or escapes when it is the
    /// cancel button.
    Button(usize),
    /// Escape.
    Escaped,
}

/// What a session's terminal ask covers while it shows (upstream's `SessionAskInput.visible` and `coveredFrame`): the
/// whole page, or its pane while that pane is laid out and no scratch covers it; nothing while the page is not shown.
#[must_use]
pub fn shown_scope(session: &Session, shown: bool) -> Option<AskScope> {
    // one a viewer presents keeps the slot here but is drawn and answered there
    let ask = session.ask().filter(|pending| pending.remote_owner.is_none())?;
    if !shown {
        return None;
    }
    if ask.pane_identity.is_none() {
        return Some(AskScope::Session);
    }
    let pane = session.ask_target_pane()?;
    (session.renders_overlay_pane(pane) && !session.scratch_active).then_some(AskScope::Pane(Slot::of_overlay(pane)))
}

/// Whether a shown terminal ask takes the keyboard (upstream's `wantsFocus`): nothing else holds it and, for a pane
/// ask, its pane is the focused one.
#[must_use]
pub fn wants_focus(session: &Session, focus_blocked: bool) -> bool {
    if focus_blocked {
        return false;
    }
    match session.ask_target_pane() {
        None => true,
        Some(pane) => {
            pane == if session.split.focused {
                OverlayPane::Right
            } else {
                OverlayPane::Left
            }
        }
    }
}

// MARK: - the dialog

/// The scroll offset (GPUI's: negative going down) that brings a button spanning `item` (as drawn at `offset`) into
/// the content's `viewport`, moving it the least (the GTK build's `clamp_page`, upstream's `scrollTo`): its top first
/// when it is taller than the viewport; `offset` when it is in view already.
#[must_use]
pub fn reveal_offset(viewport: Range<Pixels>, item: Range<Pixels>, offset: Pixels) -> Pixels {
    if item.start < viewport.start || item.end - item.start > viewport.end - viewport.start {
        offset + (viewport.start - item.start)
    } else if item.end > viewport.end {
        offset - (item.end - viewport.end)
    } else {
        offset
    }
}

/// Whose ask a dialog shows: a window's GUI ask, or a session's terminal ask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskOwner {
    Window(Uuid),
    Session(Uuid),
}

/// What opens a dialog: the ask, its owner's draft (the form's initial values without one), whose ask it is, the look
/// it draws in and where its answers go.
pub struct AskOpening<'a> {
    pub ask: &'a PendingAsk,
    pub draft: Option<&'a AskDraft>,
    pub owner: AskOwner,
    pub look: Look,
    pub poster: Poster,
}

/// The panel's room in its anchor, and the window's chrome colors.
#[derive(Debug, Clone, Copy)]
struct Frame {
    /// The panel's widest width.
    cap: f64,
    /// The content's tallest height: the anchor's less a cell each side.
    available: f64,
    colors: ChromeColors,
}

/// One open dialog (module docs).
pub struct AskDialog {
    ask: PendingAsk,
    owner: AskOwner,
    look: Look,
    navigation: AskNavigation,
    destructive: Option<usize>,
    focus: FocusHandle,
    /// The owner's draft as this dialog left it: every edit applies here and goes on to the core's.
    draft: AskDraft,
    /// The form's editor: an input ask's field or editing area, or a choice list's free-form row.
    editor: Option<Editor>,
    /// The control rule the editor holds to: the input's mode, one line for the free-form row.
    editor_mode: AskInputMode,
    /// The editor's selection while it last held the draft's text, which a refused change restores.
    selection: Range<usize>,
    /// The anchor's size at the last layout, which the panel's cap is counted from.
    anchor: Rc<Cell<Option<Size<Pixels>>>>,
    /// The content's scroll, which keeps the highlighted button in view.
    scroll: ScrollHandle,
    /// The form region's own scroll, between the pinned title and buttons.
    form_scroll: ScrollHandle,
    /// The highlighted button is to be brought into view at the next layout (the dialog opened, the keyboard moved).
    reveal: Rc<Cell<bool>>,
    /// The list's cursor row is to be brought into view at the next layout (the dialog opened, the cursor moved).
    reveal_row: Rc<Cell<bool>>,
    /// What the panel around a form measured, which picks its layout.
    fit: Rc<FormFit>,
    /// An answer went out: later keys, edits and clicks do nothing.
    done: bool,
    /// The layer dims the anchor (upstream's `AskDialogView.shaded`); only a terminal ask ever drops it.
    shaded: bool,
    poster: Poster,
    _editor_events: Option<[Subscription; 2]>,
}

impl AskDialog {
    #[must_use]
    pub fn new(opening: AskOpening<'_>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let AskOpening {
            ask,
            draft,
            owner,
            look,
            poster,
        } = opening;
        let draft = draft.cloned().unwrap_or_else(|| AskDraft::new(ask.form.as_ref()));
        let (editor, editor_mode) = match &ask.form {
            Some(AskForm::Input(input)) => (
                Some(Editor::new(
                    input.mode,
                    draft.text(),
                    input.placeholder.as_deref(),
                    window,
                    cx,
                )),
                input.mode,
            ),
            Some(AskForm::Choices(list)) => (
                list.other
                    .as_deref()
                    .map(|placeholder| Editor::new(AskInputMode::Line, draft.text(), Some(placeholder), window, cx)),
                AskInputMode::Line,
            ),
            None => (None, AskInputMode::Line),
        };
        let editor_events = editor.as_ref().map(|editor| editor.subscribe(window, cx));
        let selection = editor.as_ref().map(|editor| editor.selection(cx)).unwrap_or_default();
        form::register(cx.weak_entity());
        Self {
            navigation: AskNavigation::new(ask),
            destructive: ask
                .destructive_id
                .as_ref()
                .and_then(|id| ask.buttons.iter().position(|button| &button.id == id)),
            ask: ask.clone(),
            owner,
            look,
            focus: cx.focus_handle(),
            draft,
            editor,
            editor_mode,
            selection,
            anchor: Rc::default(),
            scroll: ScrollHandle::new(),
            form_scroll: ScrollHandle::new(),
            reveal: Rc::new(Cell::new(true)),
            reveal_row: Rc::new(Cell::new(true)),
            fit: Rc::default(),
            done: false,
            shaded: true,
            poster,
            _editor_events: editor_events,
        }
    }

    /// The ask shown.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.ask.id
    }

    /// The dialog's own focus, which holds the keyboard while no editor does.
    #[must_use]
    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    /// What takes the keyboard when the dialog is given it: the editor of an input ask, the free-form row's editor
    /// while a list's cursor is on that row, else the dialog itself (upstream's `editorActive`).
    #[must_use]
    pub fn focus_target(&self, cx: &App) -> FocusHandle {
        match &self.editor {
            Some(editor) if self.editor_active() => editor.focus_handle(cx),
            _ => self.focus.clone(),
        }
    }

    /// Whether the form's editor holds `window`'s keyboard.
    #[must_use]
    pub fn editor_focused(&self, window: &Window, cx: &App) -> bool {
        self.editor
            .as_ref()
            .is_some_and(|editor| editor.focus_handle(cx).is_focused(window))
    }

    /// Whether the dialog or its editor holds `window`'s keyboard.
    #[must_use]
    pub fn holds_keyboard(&self, window: &Window, cx: &App) -> bool {
        self.focus.is_focused(window) || self.editor_focused(window, cx)
    }

    /// The draft as the dialog left it.
    #[must_use]
    pub fn draft(&self) -> &AskDraft {
        &self.draft
    }

    /// The form's editor.
    #[must_use]
    pub fn editor(&self) -> Option<&Editor> {
        self.editor.as_ref()
    }

    /// The highlighted button.
    #[must_use]
    pub fn highlighted(&self) -> Option<usize> {
        self.navigation.highlighted()
    }

    /// Whether the layer dims the anchor: a terminal ask follows [`ngterm_core::model::Session::ask_shades_frame`].
    pub fn set_shaded(&mut self, shaded: bool, cx: &mut Context<Self>) {
        if self.shaded != shaded {
            self.shaded = shaded;
            cx.notify();
        }
    }

    /// Whether the layer dims the anchor.
    #[must_use]
    pub fn shaded(&self) -> bool {
        self.shaded
    }

    /// Takes a new look (the terminal's font or colors changed).
    pub fn set_look(&mut self, look: Look, cx: &mut Context<Self>) {
        if self.look != look {
            self.look = look;
            cx.notify();
        }
    }

    /// The ask has a choice list.
    fn in_list(&self) -> bool {
        matches!(self.ask.form, Some(AskForm::Choices(_)))
    }

    /// Whether the editor rather than the dialog should hold the keyboard (module docs of [`Self::focus_target`]).
    fn editor_active(&self) -> bool {
        match &self.ask.form {
            Some(AskForm::Input(_)) => true,
            Some(AskForm::Choices(_)) => self.draft.cursor_on_other(),
            None => false,
        }
    }

    /// Ends the dialog with `answer`, once: a button answers through the core with the owner's draft, which first takes
    /// what the editor holds (an input method's marked text reaches the editor without a change event).
    pub fn finish(&mut self, answer: Answer, window: &mut Window, cx: &mut Context<Self>) {
        if self.done {
            return;
        }
        if matches!(answer, Answer::Button(_)) {
            self.text_changed(window, cx);
        }
        self.done = true;
        if matches!(answer, Answer::Button(index) if index >= self.ask.buttons.len()) {
            return;
        }
        let id = self.ask.id.clone();
        let escaped = ControlAskResult::outcome(ControlAskOutcome::Escaped);
        match (self.owner, answer) {
            (AskOwner::Window(window), Answer::Button(index)) => post(&self.poster, move |state: &mut AppState| {
                state.answer_window_ask_button(window, &id, index);
            }),
            (AskOwner::Window(window), Answer::Escaped) => post(&self.poster, move |state: &mut AppState| {
                state.answer_window_ask(window, &id, escaped);
            }),
            (AskOwner::Session(session), Answer::Button(index)) => post(&self.poster, move |state: &mut AppState| {
                state.answer_session_ask_button(session, &id, index);
            }),
            (AskOwner::Session(session), Answer::Escaped) => post(&self.poster, move |state: &mut AppState| {
                state.answer_session_ask(session, &id, escaped);
            }),
        }
    }

    /// Applies `edit` to the draft here and posts it to the owner's in the core.
    fn edit(&mut self, edit: DraftEdit) {
        if self.done {
            return;
        }
        edit.apply(&mut self.draft);
        let id = self.ask.id.clone();
        match self.owner {
            AskOwner::Window(window) => post(&self.poster, move |state: &mut AppState| {
                state.edit_window_ask_draft(window, &id, &edit);
            }),
            AskOwner::Session(session) => post(&self.poster, move |state: &mut AppState| {
                state.edit_session_ask_draft(session, &id, &edit);
            }),
        }
    }

    /// One key's effect (upstream's `AskDialogView.handle`).
    pub fn handle(&mut self, key: AskKey, window: &mut Window, cx: &mut Context<Self>) {
        let was_active = self.editor_active();
        let list = self.in_list();
        // without a list, Up and Down move the button highlight as Left and Right do
        let key = match key {
            AskKey::Down if !list => AskKey::Forward,
            AskKey::Up if !list => AskKey::Backward,
            key => key,
        };
        match key {
            AskKey::Forward => self.move_highlight(true),
            AskKey::Backward => self.move_highlight(false),
            AskKey::Down => self.move_cursor(1),
            AskKey::Up => self.move_cursor(-1),
            AskKey::Toggle if list => self.edit(DraftEdit::Toggle(self.draft.cursor())),
            AskKey::Toggle => {}
            AskKey::Activate => {
                if let Some(index) = self.navigation.activate() {
                    self.finish(Answer::Button(index), window, cx);
                }
            }
            AskKey::Cancel => self.finish(Answer::Escaped, window, cx),
            AskKey::Hotkey(letter) => {
                if let Some(index) = self.navigation.hotkey(&letter) {
                    self.finish(Answer::Button(index), window, cx);
                }
            }
        }
        self.follow_cursor(was_active, window, cx);
        cx.notify();
    }

    fn move_highlight(&mut self, forward: bool) {
        if forward {
            self.navigation.move_forward();
        } else {
            self.navigation.move_backward();
        }
        self.reveal.set(true);
    }

    fn move_cursor(&mut self, offset: isize) {
        self.edit(DraftEdit::MoveCursor(offset));
        self.reveal_row.set(true);
    }

    /// A click on the list row at `index`: it is picked or flipped and takes the cursor.
    fn toggle(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let was_active = self.editor_active();
        self.edit(DraftEdit::Toggle(index));
        self.reveal_row.set(true);
        self.follow_cursor(was_active, window, cx);
        cx.notify();
    }

    /// The cursor crossed onto or off the free-form row while the dialog held the keyboard: the keyboard follows,
    /// between the row's editor and the dialog (upstream's `editorActive` hand-over).
    fn follow_cursor(&self, was_active: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor_active() != was_active && self.holds_keyboard(window, cx) {
            window.focus(&self.focus_target(cx), cx);
        }
    }

    /// What the editor reports: a change of its text, or its focus.
    fn editor_event(&mut self, event: &InputEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            InputEvent::Change => self.text_changed(window, cx),
            // a click into the free-form row's field puts the list cursor there (upstream's `onFocus`)
            InputEvent::Focus if self.in_list() && !self.draft.cursor_on_other() => {
                self.move_cursor(isize::MAX);
                cx.notify();
            }
            _ => {}
        }
    }

    /// The editor's state was updated: while it holds the draft's text, its selection is the one a refused change
    /// restores (a caret move emits no event of its own).
    fn editor_updated(&mut self, cx: &mut Context<Self>) {
        if let Some(editor) = &self.editor
            && editor.holds(self.draft.text(), cx)
        {
            self.selection = editor.selection(cx);
        }
    }

    /// The editor's text changed: within the rules it becomes the draft's, else the change is undone, the previous
    /// text coming back with the selection it had (module docs of [`form`]).
    fn text_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.clone() else {
            return;
        };
        let text = editor.text(cx);
        if self.done || text == self.draft.text() {
            return;
        }
        if !form::edit_allowed(&text, self.editor_mode) {
            editor.restore(self.draft.text(), self.selection.clone(), window, cx);
            return;
        }
        self.edit(DraftEdit::Text(text));
        self.selection = editor.selection(cx);
        cx.notify();
    }

    /// A key pressed in the editor, before its own bindings ([`form::intercept_key`]); `close`: the keymap's close
    /// chord. Whether the dialog took it.
    fn editor_key(&mut self, keystroke: &Keystroke, close: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(editor) = self.editor.clone() else {
            return false;
        };
        let composing = editor.composing(window, cx);
        match form::field_key(keystroke, self.editor_mode, self.in_list(), close, composing) {
            // the newline Return would insert is held to the rules as typed text is ([`form::GuardedInput`])
            None if !composing && form::inserts_newline(keystroke, self.editor_mode) => {
                !form::change_allowed(&editor.text(cx), editor.selection(cx), "\n", self.editor_mode)
            }
            None => false,
            Some(FieldKey::Swallow) => true,
            Some(FieldKey::Ask(key)) => {
                self.handle(key, window, cx);
                true
            }
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let modifiers = &event.keystroke.modifiers;
        if modifiers.control || modifiers.alt || modifiers.platform || !self.focus.is_focused(window) {
            // the window's key monitor had a modified key first, and what reaches here from the editor is the
            // editor's (its dialog keys were taken on the way in)
            return;
        }
        cx.stop_propagation();
        if let Some(key) = ask_key(&event.keystroke, &input::current_source(&event.keystroke)) {
            self.handle(key, window, cx);
        }
    }

    /// A press on the dialog outside its buttons: it takes the keyboard back (its editor keeps it), and a terminal
    /// ask's pane becomes the focused one (upstream's `SessionAskInput.selectPane`).
    fn pressed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.editor_focused(window, cx) {
            window.focus(&self.focus_target(cx), cx);
        }
        if let AskOwner::Session(session) = self.owner {
            post(&self.poster, move |state: &mut AppState| state.select_ask_pane(session));
        }
    }

    fn gui(&self) -> bool {
        self.ask.style == AskStyle::Gui
    }

    fn ink(&self, colors: ChromeColors) -> Rgb {
        if self.gui() {
            colors.foreground
        } else {
            self.look.foreground
        }
    }

    fn text_size(&self) -> Pixels {
        if self.gui() {
            px(GUI_TEXT)
        } else {
            modal::logical(self.look.font_pixels())
        }
    }

    /// One text line's height, as the form's least heights count it.
    fn line_height(&self) -> f64 {
        f64::from(f32::from(self.text_size())) * form::LINE_FACTOR
    }

    /// The widest button's natural width, its label shaped in the dialog's font.
    fn widest_button(&self, cell: f64, window: &Window) -> f64 {
        let mut style = window.text_style();
        if !self.gui()
            && let Some(family) = &self.look.family
        {
            style.font_family = family.clone().into();
        }
        let size = self.text_size();
        let padding = if self.gui() { 2.0 * 12.0 } else { cell };
        self.ask
            .buttons
            .iter()
            .enumerate()
            .map(|(index, button)| {
                let label = button_label(button, !self.gui() && self.destructive == Some(index));
                let run = style.to_run(label.text.len());
                let shaped = window
                    .text_system()
                    .shape_line(label.text.replace('\n', " ").into(), size, &[run], None);
                f64::from(f32::from(shaped.width)) + padding
            })
            .fold(0.0, f64::max)
    }

    fn render_button(
        &self,
        index: usize,
        width: Option<f64>,
        colors: ChromeColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let button = &self.ask.buttons[index];
        let destructive = self.destructive == Some(index);
        let highlighted = self.navigation.highlighted() == Some(index);
        let label = button_label(button, !self.gui() && destructive);
        let mut highlights = Vec::new();
        if let Some(range) = label.underline.clone() {
            let underline = UnderlineStyle {
                thickness: px(1.),
                ..UnderlineStyle::default()
            };
            highlights.push((
                range,
                HighlightStyle {
                    underline: Some(underline),
                    ..HighlightStyle::default()
                },
            ));
        }
        let text = StyledText::new(label.text).with_highlights(highlights);
        let (fill, ink) = self.button_colors(button_look(self.ask.style, highlighted, destructive), colors);
        let cell = self.look.cell(self.ask.style);
        let selector = format!("ask-button-{}", button.id);
        let reveal = highlighted.then(|| Self::reveal_canvas(self.scroll.clone(), Rc::clone(&self.reveal)));
        div()
            .id(SharedString::from(selector.clone()))
            .accessibility_id(SharedString::from(selector.clone()))
            .role(gpui_kit::Role::Button)
            .debug_selector(move || selector)
            .relative()
            .when_some(width, |button, width| button.w(modal::logical(width)))
            .flex()
            .justify_center()
            .text_center()
            .bg(fill)
            .text_color(ink)
            .when(label.bold, |button| button.font_weight(FontWeight::BOLD))
            .when(self.gui(), |button| button.px_3().py(px(5.)).rounded(px(6.)))
            .when(!self.gui(), |button| {
                button.px(modal::logical(cell * 0.5)).py(modal::logical(cell * 0.4))
            })
            .cursor_pointer()
            .child(text)
            .children(reveal)
            // a press on a button is the button's: it does not focus the dialog or select its pane
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |dialog, _, window, cx| {
                dialog.finish(Answer::Button(index), window, cx);
                cx.notify();
            }))
            .into_any_element()
    }

    /// Over an item `scroll` holds: once it is laid out, brings it into the view if `reveal` asks to, on the next
    /// frame (a frame being drawn takes no refresh).
    fn reveal_canvas(scroll: ScrollHandle, reveal: Rc<Cell<bool>>) -> AnyElement {
        canvas(
            move |bounds, window, _| {
                let viewport = scroll.bounds();
                if !reveal.get() || viewport.size.height <= px(0.) {
                    return;
                }
                reveal.set(false);
                let offset = scroll.offset();
                let y = reveal_offset(
                    viewport.top()..viewport.bottom(),
                    bounds.top()..bounds.bottom(),
                    offset.y,
                );
                if y != offset.y {
                    window.on_next_frame(move |window, _| {
                        scroll.set_offset(point(offset.x, y));
                        window.refresh();
                    });
                }
            },
            |_, (), _, _| {},
        )
        .absolute()
        .inset_0()
        .into_any_element()
    }

    /// Over the list's cursor row: keeps it in the view of whatever scrolls the list.
    fn row_reveal(&self) -> AnyElement {
        let scroll = if self.fit.scrolls_whole() {
            &self.scroll
        } else {
            &self.form_scroll
        };
        Self::reveal_canvas(scroll.clone(), Rc::clone(&self.reveal_row))
    }

    fn button_colors(&self, look: AskButtonLook, colors: ChromeColors) -> (Hsla, Hsla) {
        let (fg, bg) = (self.look.foreground, self.look.background);
        match look {
            AskButtonLook::Gui(look) => modal::button_colors(look, colors),
            AskButtonLook::Block => (crate::chrome::color(fg, 0.12), crate::chrome::color(fg, 1.0)),
            AskButtonLook::Inverted => (crate::chrome::color(fg, 1.0), crate::chrome::color(bg, 1.0)),
        }
    }

    fn render_strip(&self, cap: f64, window: &Window, colors: ChromeColors, cx: &mut Context<Self>) -> AnyElement {
        let cell = self.look.cell(self.ask.style);
        let count = self.ask.buttons.len();
        let widest = self.widest_button(cell, window);
        let available = (cap - 4.0 * cell).max(0.0);
        let axis = strip_axis(widest, count, cell, available);
        let width = match axis {
            StripAxis::Row => widest,
            StripAxis::Column => widest.min(available),
        };
        let buttons: Vec<AnyElement> = (0..count)
            .map(|index| self.render_button(index, Some(width), colors, cx))
            .collect();
        let strip = div()
            .flex()
            .w_full()
            .gap(modal::logical(cell))
            .when(axis == StripAxis::Column, gpui_kit::Styled::flex_col)
            .when(axis == StripAxis::Row, gpui_kit::Styled::flex_row);
        let strip = match (axis, self.ask.align) {
            (StripAxis::Row, AskAlignment::Left) => strip.justify_start(),
            (StripAxis::Row, AskAlignment::Center) => strip.justify_center(),
            (StripAxis::Row, AskAlignment::Right) => strip.justify_end(),
            (StripAxis::Column, AskAlignment::Left) => strip.items_start(),
            (StripAxis::Column, AskAlignment::Center) => strip.items_center(),
            (StripAxis::Column, AskAlignment::Right) => strip.items_end(),
        };
        strip.children(buttons).into_any_element()
    }

    fn render_title(&self) -> gpui_kit::Stateful<gpui_kit::Div> {
        div()
            .id("ask-title")
            .accessibility_id("ask-title")
            .role(gpui_kit::Role::Heading)
            .font_weight(FontWeight::BOLD)
            .child(SharedString::from(self.ask.title.clone()))
    }

    fn render_message(&self, colors: ChromeColors) -> Option<gpui_kit::Stateful<gpui_kit::Div>> {
        let message = self.ask.message.clone().filter(|message| !message.is_empty())?;
        let ink = self.ink(colors);
        let ink = if self.gui() {
            crate::chrome::color(ink, crate::chrome::DIM_ALPHA)
        } else {
            crate::chrome::color(ink, 0.7)
        };
        Some(
            div()
                .id("ask-message")
                .accessibility_id("ask-message")
                .role(gpui_kit::Role::Label)
                .text_color(ink)
                .child(SharedString::from(message)),
        )
    }

    /// The content column with the spacing every layout shares.
    fn content_column(&self) -> gpui_kit::Stateful<gpui_kit::Div> {
        let cell = self.look.cell(self.ask.style);
        div()
            .id("ask-content")
            .debug_selector(|| "ask-content".to_owned())
            .flex()
            .flex_col()
            .gap(modal::logical(cell * 1.5))
            .p(modal::logical(cell * 2.0))
    }

    /// A plain ask's content: title, message and buttons, all scrolling when taller than the anchor.
    fn plain_content(&self, frame: Frame, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let strip = self.render_strip(frame.cap, window, frame.colors, cx);
        self.content_column()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .max_h(modal::logical(frame.available))
            .child(self.render_title())
            .children(self.render_message(frame.colors))
            .child(strip)
            .into_any_element()
    }

    /// A form ask's content (upstream's `panel`): all of it when it fits; else the title, message and buttons pinned
    /// around the form's region, which scrolls in what is left down to its least height; else, when even that does
    /// not fit, all of it scrolling.
    fn form_content(&self, form: &AskForm, frame: Frame, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let cell = self.look.cell(self.ask.style);
        let whole = self.fit.scrolls_whole();
        let message = self.render_message(frame.colors);
        let inputs = FitInputs {
            gap: cell * 1.5,
            padding: cell * 2.0,
            pinned: 2 + usize::from(message.is_some()),
            form_min: form::form_min_height(
                form,
                self.fit.editor_line().unwrap_or_else(|| self.line_height()),
                self.fit.editor_frame(),
            ),
            available: frame.available,
        };
        let title = self
            .render_title()
            .relative()
            .flex_shrink_0()
            .child(FormFit::probe(&self.fit, 0, None));
        let message = message.map(|message| {
            message
                .relative()
                .flex_shrink_0()
                .child(FormFit::probe(&self.fit, 1, None))
        });
        let strip = div()
            .relative()
            .flex_shrink_0()
            .child(self.render_strip(frame.cap, window, frame.colors, cx))
            .child(FormFit::probe(&self.fit, 2, Some(inputs)));
        let body = self.render_form(form, frame.colors, cx);
        let region = if whole {
            body
        } else {
            div()
                .id("ask-form")
                .debug_selector(|| "ask-form".to_owned())
                .flex_shrink(1.)
                .min_h(modal::logical(inputs.form_min))
                .overflow_y_scroll()
                .track_scroll(&self.form_scroll)
                .child(body)
                .into_any_element()
        };
        let content = self
            .content_column()
            .max_h(modal::logical(frame.available))
            .child(title)
            .children(message)
            .child(region)
            .child(strip);
        if whole {
            content
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
                .into_any_element()
        } else {
            content.into_any_element()
        }
    }

    /// The form: the editor of an input ask, or the choice list with its free-form row.
    fn render_form(&self, form: &AskForm, colors: ChromeColors, cx: &mut Context<Self>) -> AnyElement {
        let cell = self.look.cell(self.ask.style);
        let look = FieldLook {
            gui: self.gui(),
            size: self.text_size(),
            ink: crate::chrome::color(self.ink(colors), 1.0),
            family: self.look.family.clone(),
            inset: modal::logical(cell * 0.5),
            border: crate::chrome::color(self.look.foreground, 0.2),
        };
        // the multiline editor of an input ask gives way to its region's room; the free-form row is one line
        let fit = matches!(form, AskForm::Input(_)).then(|| Rc::clone(&self.fit));
        let field = self
            .editor
            .as_ref()
            .map(|editor| editor.element(self.editor_mode, &look, fit));
        match form {
            AskForm::Input(_) => field.unwrap_or_else(|| div().into_any_element()),
            AskForm::Choices(list) => {
                let rows = RowLook {
                    gui: self.gui(),
                    cell,
                    colors,
                    foreground: self.look.foreground,
                    background: self.look.background,
                    char_width: self.look.font_pixels() * 0.6,
                };
                self.render_list(list, rows, field, cx)
            }
        }
    }

    fn render_panel(&self, anchor: Size<Pixels>, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let colors = crate::chrome::current(cx);
        let cell = self.look.cell(self.ask.style);
        let anchor_width = f64::from(f32::from(anchor.width));
        let anchor_height = f64::from(f32::from(anchor.height));
        let frame = Frame {
            cap: modal::ask_panel_cap(self.ask.width, anchor_width, self.look.terminal_cell()),
            available: (anchor_height - 2.0 * cell).max(0.0),
            colors,
        };
        let ink = self.ink(colors);
        let panel = if self.gui() {
            modal::panel(colors)
        } else {
            div()
                .bg(crate::chrome::color(self.look.background, 1.0))
                .text_color(crate::chrome::color(ink, 1.0))
                .border_1()
                .border_color(crate::chrome::color(self.look.foreground, 0.3))
                .when_some(self.look.family.clone(), gpui_kit::Styled::font_family)
        };
        let content = match &self.ask.form {
            None => self.plain_content(frame, window, cx),
            Some(form) => self.form_content(form, frame, window, cx),
        };
        // an input ask, as one of a set width, takes the whole width it may (upstream's `isInput` panel)
        let fills = self.ask.width.is_some() || matches!(self.ask.form, Some(AskForm::Input(_)));
        panel
            .id("ask-dialog")
            .accessibility_id("ask-dialog")
            .role(gpui_kit::Role::Dialog)
            .text_size(self.text_size())
            .max_w(modal::logical(frame.cap))
            .when(fills, |panel| panel.w(modal::logical(frame.cap)))
            .child(content)
            .into_any_element()
    }
}

impl Render for AskDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let anchor = Rc::clone(&self.anchor);
        // the panel is counted from the anchor measured here, so a new size is drawn on the next frame (a frame being
        // drawn takes no refresh)
        let measure = canvas(
            move |bounds, window, _| {
                if anchor.get() != Some(bounds.size) {
                    anchor.set(Some(bounds.size));
                    window.on_next_frame(|window, _| window.refresh());
                }
            },
            |_, (), _, _| {},
        )
        .absolute()
        .size_full();
        let panel = self.anchor.get().map(|size| self.render_panel(size, window, cx));
        div()
            .id("ask-layer")
            .key_context("NgtermAsk")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .size_full()
            .relative()
            .occlude()
            .when(self.shaded, |layer| layer.bg(modal::dim()))
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|dialog, _, window, cx| {
                    dialog.pressed(window, cx);
                    cx.stop_propagation();
                }),
            )
            .child(measure)
            .children(panel)
    }
}

#[cfg(test)]
mod tests;
