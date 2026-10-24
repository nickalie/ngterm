//! An ask's form in the dialog, after upstream's `Views/AskFormViews.swift` and the form half of
//! `AskDialogView.swift` (#778, #779): the text field or multiline editor of an input ask, the radio or checkbox rows
//! of a choice list with its free-form row, the keys the editor hands the dialog, the guard holding the editor to the
//! draft's rules, and how the panel lays out around a form.
//!
//! - The editor is gpui-kit's one-line [`Input`] or multiline [`Textarea`] (one per dialog, kept across every layout
//!   change, so focus and composition survive a swap); the free-form row is a one-line editor. Its text is the draft's
//!   ([`ngterm_core::model::AskDraft`], owned by the ask's slot owner in the core, mirrored by the dialog): every
//!   accepted change goes to the core as a [`DraftEdit`].
//! - The guard (upstream's `askEditAllowed`): an edit, paste or composition is held to
//!   [`AskInputForm::MAX_TEXT_BYTES`] and the mode's control rule before it applies, refused whole otherwise, the text
//!   keeping its end, its selection and its editing history. Typed text, an input method's marked text and its commit
//!   are checked by the form's input handler ([`GuardedInput`]) before they reach the editor, a newline the multiline
//!   editor's Return would insert by the key monitor ([`intercept_key`]), and a paste on its raw clipboard text (the
//!   one-line field would strip a newline before validating); the one-line field also checks the pending text through
//!   gpui-kit's validator. A change that still breaks a rule through a path with no hook (an editor command such as
//!   indenting) is undone at once: the previous text comes back with the selection it had, though gpui-kit then starts
//!   the editing history over.
//! - Keys reach the editor's dialog before the editor's own bindings ([`intercept_key`], from the app's key monitor):
//!   Tab and Shift-Tab move the button highlight, Esc escapes, Return answers a one-line field, the submit chord
//!   (Cmd-Return on macOS, Ctrl-Return on Linux and Windows) answers the multiline editor, Up and Down leave the
//!   free-form row for the list, and the close chord escapes the ask; while the editor composes (the input state's
//!   marked range, which every platform's input handler feeds: text-input-v3 and XIM on Linux, `NSTextInputClient` on
//!   macOS, IMM32/TSF on Windows) every key stays the editor's and the close chord is kept without closing. Letters
//!   typed in an editor never fire a hotkey.
//! - Element ids are upstream's accessibility ids: `ask-input` (the editor), `ask-choice-<id>` (a caller row) and
//!   `ask-other` (the free-form row; its identity carries no caller prefix, so a caller id cannot collide with it).

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Bounds, Context, Entity, EntityInputHandler as _, FocusHandle, Focusable as _,
    Hsla, InteractiveElement as _, IntoElement, Keystroke, Modifiers, ParentElement as _, Pixels, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, WeakEntity, Window, canvas, div, px,
};
use ngterm_core::chrome::{ChromeColors, Rgb};
use ngterm_core::keymap::BuiltinAction;
use ngterm_core::model::{AskChoiceForm, AskDraft, AskForm, AskInputForm, AskInputMode};
use ngterm_proto::AskChoice;

use super::{AskDialog, AskKey};
use crate::modal;
use crate::terminal::input::KeyStroke;

mod guard;

pub use guard::GuardedInput;

/// The free-form row's element id (upstream's `AskChoiceList.otherRowID`).
pub const OTHER_ROW_ID: &str = "ask-other";
/// The fewest lines the multiline editor shows while it has the room (upstream's `sizeThatFits`).
pub const MIN_LINES: usize = 5;
/// The most lines the multiline editor grows to before it scrolls.
pub const MAX_LINES: usize = 15;
/// The lines the multiline editor gives way to in a short anchor ([`SHORT_LINES`] as a row count).
pub const SHORT_ROWS: usize = 2;
/// The lines the multiline editor gives way to in a short anchor, as the form's least height counts them.
pub const SHORT_LINES: f64 = 2.0;
/// The lines a choice list's region keeps in a short anchor (upstream's `minHeight: lineHeight * 3`).
pub const LIST_LINES: f64 = 3.0;
/// A text line's height per pixel of font size, as the dialog counts the form's least heights.
pub const LINE_FACTOR: f64 = 1.4;
/// The free-form row's field is at least this many cells wide.
pub const OTHER_FIELD_CELLS: f64 = 24.0;

/// A caller row's element id (upstream's `AskChoiceList.rowID`).
#[must_use]
pub fn row_id(id: &str) -> String {
    format!("ask-choice-{id}")
}

// MARK: - the draft

/// One change the dialog makes to its draft, applied to its own mirror and posted to the slot owner's in the core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftEdit {
    /// The editor's text.
    Text(String),
    /// The list cursor moved by this many rows.
    MoveCursor(isize),
    /// The row at this index was picked or flipped.
    Toggle(usize),
}

impl DraftEdit {
    /// Applies the edit to `draft`.
    pub fn apply(&self, draft: &mut AskDraft) {
        match self {
            Self::Text(text) => draft.set_text(text),
            Self::MoveCursor(offset) => draft.move_cursor(*offset),
            Self::Toggle(index) => draft.toggle(*index),
        }
    }
}

// MARK: - the guard

/// Whether the editor may hold `text`: within the cap and free of the controls `mode` forbids.
#[must_use]
pub fn edit_allowed(text: &str, mode: AskInputMode) -> bool {
    text.len() <= AskInputForm::MAX_TEXT_BYTES && !AskInputForm::has_forbidden_controls(text, mode)
}

/// Whether replacing `range` of `current` with `replacement` keeps the editor within the rules (upstream's
/// `askEditAllowed`): the result within the cap and `replacement` free of the controls `mode` forbids, so a paste
/// carrying one is refused whole, as one that would pass the cap.
#[must_use]
pub fn change_allowed(current: &str, range: Range<usize>, replacement: &str, mode: AskInputMode) -> bool {
    let replaced = range.end.saturating_sub(range.start);
    !AskInputForm::has_forbidden_controls(replacement, mode)
        && current.len().saturating_sub(replaced) + replacement.len() <= AskInputForm::MAX_TEXT_BYTES
}

/// Whether `keystroke` is the multiline editor's Return, which inserts a newline (with or without Shift; the submit
/// chord answers instead).
#[must_use]
pub fn inserts_newline(keystroke: &Keystroke, mode: AskInputMode) -> bool {
    let modifiers = &keystroke.modifiers;
    mode == AskInputMode::Multiline
        && matches!(keystroke.key.as_str(), "enter" | "kp_enter")
        && !(modifiers.control || modifiers.alt || modifiers.platform)
}

/// The most lines the multiline editor may show: all [`MAX_LINES`] when its room is not bounded (`None`: the whole
/// panel scrolls), else as many as fit `room` with the editor's frame (`frame`, its padding and border) around them,
/// down to [`SHORT_ROWS`] (upstream's `sizeThatFits` against the offered height); `line` is one line's height.
#[must_use]
pub fn editor_rows(room: Option<f64>, frame: f64, line: f64) -> usize {
    let Some(room) = room.filter(|_| line > 0.0) else {
        return MAX_LINES;
    };
    #[expect(clippy::cast_precision_loss, reason = "at most MAX_LINES rows")]
    let fits = |rows: usize| rows as f64 * line + frame <= room;
    (SHORT_ROWS..=MAX_LINES)
        .rev()
        .find(|&rows| fits(rows))
        .unwrap_or(SHORT_ROWS)
}

// MARK: - the editor's keys

/// What a key pressed in the editor does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldKey {
    /// The dialog's action.
    Ask(AskKey),
    /// Kept: nothing happens, and the key goes nowhere else.
    Swallow,
}

/// What a key pressed in the editor does (upstream's `AskTextEditor.Coordinator.handle` and the submit chord of
/// `AskTextView`); `None` leaves it to the editor. `in_list`: the editor is a choice list's free-form row, whose Up and
/// Down go to the list. `close`: the key is the keymap's close chord. `composing`: the editor has marked text.
#[must_use]
pub fn field_key(
    keystroke: &Keystroke,
    mode: AskInputMode,
    in_list: bool,
    close: bool,
    composing: bool,
) -> Option<FieldKey> {
    if composing {
        // a composition keeps every key, and the close chord closes nothing behind it
        return close.then_some(FieldKey::Swallow);
    }
    if close {
        return Some(FieldKey::Ask(AskKey::Cancel));
    }
    let modifiers = &keystroke.modifiers;
    let plain = !(modifiers.control || modifiers.alt || modifiers.platform);
    let unshifted = plain && !modifiers.shift;
    let key = match keystroke.key.as_str() {
        "tab" if plain && modifiers.shift => AskKey::Backward,
        "tab" if plain => AskKey::Forward,
        "escape" if plain => AskKey::Cancel,
        "enter" | "kp_enter" if mode == AskInputMode::Line && plain => AskKey::Activate,
        "enter" | "kp_enter" if mode == AskInputMode::Multiline && submit_chord(*modifiers) => AskKey::Activate,
        "up" if in_list && unshifted => AskKey::Up,
        "down" if in_list && unshifted => AskKey::Down,
        _ => return None,
    };
    Some(FieldKey::Ask(key))
}

/// The multiline editor's submit chord: Cmd-Return on macOS, Ctrl-Return elsewhere (the Linux table's ⌘ → Ctrl rule),
/// with no other modifier.
fn submit_chord(modifiers: Modifiers) -> bool {
    let primary = if cfg!(target_os = "macos") {
        modifiers.platform && !modifiers.control
    } else {
        modifiers.control && !modifiers.platform
    };
    primary && !modifiers.alt && !modifiers.shift
}

thread_local! {
    /// Every dialog alive, so the key monitor finds the one whose editor holds the keyboard.
    static DIALOGS: RefCell<Vec<WeakEntity<AskDialog>>> = const { RefCell::new(Vec::new()) };
}

/// Puts `dialog` where [`intercept_key`] looks.
pub(super) fn register(dialog: WeakEntity<AskDialog>) {
    DIALOGS.with_borrow_mut(|dialogs| {
        dialogs.retain(|known| known.upgrade().is_some());
        dialogs.push(dialog);
    });
}

/// The app's key monitor saw `keystroke` (`stroke` as the keymap reads it) in `window`: while an ask's editor holds
/// the keyboard, the dialog's keys go to the dialog before the editor's own bindings. Whether the key was taken.
pub fn intercept_key(keystroke: &Keystroke, stroke: &KeyStroke, window: &mut Window, cx: &mut App) -> bool {
    let dialogs: Vec<Entity<AskDialog>> =
        DIALOGS.with_borrow(|dialogs| dialogs.iter().filter_map(WeakEntity::upgrade).collect());
    let Some(dialog) = dialogs
        .into_iter()
        .find(|dialog| dialog.read(cx).editor_focused(window, cx))
    else {
        return false;
    };
    let close = crate::keymap_dispatch::menu_action(stroke) == Some(BuiltinAction::CloseSession);
    dialog.update(cx, |dialog, cx| dialog.editor_key(keystroke, close, window, cx))
}

// MARK: - the editor

/// How the editor draws: the dialog's style, font and ink.
#[derive(Debug, Clone)]
pub struct FieldLook {
    pub gui: bool,
    pub size: Pixels,
    pub ink: Hsla,
    pub family: Option<String>,
    /// The terminal style's frame: its padding and its border's color.
    pub inset: Pixels,
    pub border: Hsla,
}

/// The form's text editor: an input ask's field or editing area, or a choice list's free-form row.
#[derive(Clone)]
pub enum Editor {
    Line(Entity<InputState>),
    Lines(Entity<TextareaState>),
}

impl Editor {
    /// An editor of `mode` holding `text`, showing `placeholder` while empty.
    pub fn new(
        mode: AskInputMode,
        text: &str,
        placeholder: Option<&str>,
        window: &mut Window,
        cx: &mut Context<AskDialog>,
    ) -> Self {
        let (text, placeholder) = (text.to_owned(), placeholder.unwrap_or_default().to_owned());
        match mode {
            AskInputMode::Line => Self::Line(cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .validate(|text, _| edit_allowed(text, AskInputMode::Line))
                    .default_value(text)
            })),
            AskInputMode::Multiline => Self::Lines(cx.new(|cx| {
                TextareaState::new(window, cx)
                    .auto_grow(MIN_LINES, MAX_LINES)
                    .placeholder(placeholder)
                    .default_value(text)
            })),
        }
    }

    /// The dialog hears the editor's changes and its focus, and sees every update of its state (a caret move too).
    pub(super) fn subscribe(&self, window: &mut Window, cx: &mut Context<AskDialog>) -> [Subscription; 2] {
        match self {
            Self::Line(state) => [
                cx.subscribe_in(state, window, |dialog, _, event: &InputEvent, window, cx| {
                    dialog.editor_event(event, window, cx);
                }),
                cx.observe(state, |dialog, _, cx| dialog.editor_updated(cx)),
            ],
            Self::Lines(state) => [
                cx.subscribe_in(state, window, |dialog, _, event: &InputEvent, window, cx| {
                    dialog.editor_event(event, window, cx);
                }),
                cx.observe(state, |dialog, _, cx| dialog.editor_updated(cx)),
            ],
        }
    }

    #[must_use]
    pub fn focus_handle(&self, cx: &App) -> FocusHandle {
        match self {
            Self::Line(state) => state.read(cx).focus_handle(cx),
            Self::Lines(state) => state.read(cx).focus_handle(cx),
        }
    }

    /// The text the editor holds, marked text included.
    #[must_use]
    pub fn text(&self, cx: &App) -> String {
        match self {
            Self::Line(state) => state.read(cx).value().to_string(),
            Self::Lines(state) => state.read(cx).value().to_string(),
        }
    }

    /// Whether the editor holds exactly `text`.
    #[must_use]
    pub fn holds(&self, text: &str, cx: &App) -> bool {
        let len = match self {
            Self::Line(state) => state.read(cx).text().len(),
            Self::Lines(state) => state.read(cx).text().len(),
        };
        len == text.len() && self.text(cx) == text
    }

    /// The selection, in bytes.
    #[must_use]
    pub fn selection(&self, cx: &App) -> Range<usize> {
        match self {
            Self::Line(state) => state.read(cx).selected_range(),
            Self::Lines(state) => state.read(cx).selected_range(),
        }
    }

    /// Whether an input method is composing in the editor (its marked range).
    pub fn composing(&self, window: &mut Window, cx: &mut App) -> bool {
        match self {
            Self::Line(state) => state.update(cx, |state, cx| state.marked_text_range(window, cx).is_some()),
            Self::Lines(state) => state.update(cx, |state, cx| state.marked_text_range(window, cx).is_some()),
        }
    }

    /// Puts `text` back with `selection` selected (a refused edit).
    pub fn restore(&self, text: &str, selection: Range<usize>, window: &mut Window, cx: &mut App) {
        let text = SharedString::from(text.to_owned());
        match self {
            Self::Line(state) => state.update(cx, |state, cx| {
                state.set_value(text, window, cx);
                state.set_selected_range(selection, cx);
            }),
            Self::Lines(state) => state.update(cx, |state, cx| {
                state.set_value(text, window, cx);
                state.set_selected_range(selection, cx);
            }),
        }
    }

    /// The editor drawn in `look`, its input handler guarded ([`GuardedInput`]); a paste that breaks `mode`'s rules is
    /// refused whole. `fit`: the multiline editor of an input ask, which gives way to the room its form's region has
    /// ([`FormFit::fit_rows`]).
    #[must_use]
    pub(super) fn element(&self, mode: AskInputMode, look: &FieldLook, fit: Option<Rc<FormFit>>) -> AnyElement {
        let editor = self.clone();
        let guard = move |item: &gpui_kit::ClipboardItem, _: &mut Window, cx: &mut App| {
            item.text()
                .is_some_and(|paste| !change_allowed(&editor.text(cx), editor.selection(cx), &paste, mode))
        };
        let field = match self {
            Self::Line(state) => Input::new(state)
                .id("ask-input")
                .accessibility_id("ask-input")
                .appearance(look.gui)
                .on_paste(guard)
                .text_size(look.size)
                .text_color(look.ink)
                .when_some(look.family.clone().filter(|_| !look.gui), gpui_kit::Styled::font_family)
                .into_any_element(),
            Self::Lines(state) => Textarea::new(state)
                .accessibility_id("ask-input")
                .appearance(look.gui)
                .on_paste(guard)
                .text_size(look.size)
                .text_color(look.ink)
                .when_some(look.family.clone().filter(|_| !look.gui), gpui_kit::Styled::font_family)
                .into_any_element(),
        };
        let frame = div()
            .w_full()
            .min_w(px(0.))
            .when(!look.gui, |frame| {
                frame.p(look.inset).border_1().border_color(look.border)
            })
            .child(field);
        // painted after the editor, so the window takes its input handler over the editor's own
        let editor = self.clone();
        let hook = canvas(
            |_, _, _| {},
            move |bounds, (), window, cx| {
                let focus = editor.focus_handle(cx);
                let text_bounds = editor.text_bounds(cx).unwrap_or(bounds);
                window.handle_input(&focus, GuardedInput::new(&editor, mode, text_bounds), cx);
                if let (Some(fit), Self::Lines(state)) = (fit, &editor) {
                    fit.fit_rows(state, bounds, window, cx);
                }
            },
        )
        .absolute()
        .inset_0();
        div()
            .debug_selector(|| "ask-input".to_owned())
            .relative()
            .w_full()
            .min_w(px(0.))
            .child(frame)
            .child(hook)
            .into_any_element()
    }

    /// Where the editor laid its text out on the last frame.
    fn text_bounds(&self, cx: &App) -> Option<Bounds<Pixels>> {
        match self {
            Self::Line(state) => state.read(cx).text_bounds(),
            Self::Lines(state) => state.read(cx).text_bounds(),
        }
    }
}

// MARK: - the layout around a form

/// The least height the form's region keeps when the anchor is short (upstream's `content(fill: true)`): a one-line
/// field's own height, two lines of the multiline editor in its frame (`frame`, its padding and border), three of a
/// list; `line` is one text line's height.
#[must_use]
pub fn form_min_height(form: &AskForm, line: f64, frame: f64) -> f64 {
    match form {
        AskForm::Input(input) if input.mode == AskInputMode::Line => line + 8.0,
        AskForm::Input(_) => SHORT_LINES * line + frame,
        AskForm::Choices(_) => LIST_LINES * line,
    }
}

/// Whether the whole panel scrolls (upstream's last `ViewThatFits` fallback): the title, message and buttons with the
/// gaps and padding around them (`pinned`) plus the form's least height do not fit `available`.
#[must_use]
pub fn scrolls_whole(pinned: f64, form_min: f64, available: f64) -> bool {
    pinned + form_min > available
}

/// What the panel around a form measured on the last frame, which picks its layout on the next.
pub(super) struct FormFit {
    /// The title's, the message's and the button strip's heights.
    heights: [Cell<f64>; 3],
    /// The whole panel scrolls: the pinned parts and the form's least height do not fit.
    whole: Cell<bool>,
    /// The most height the form's region has between the pinned parts; `None` while the whole panel scrolls.
    room: Cell<Option<f64>>,
    /// The multiline editor's frame around its lines: its padding and border.
    frame: Cell<f64>,
    /// The multiline editor's line height, once laid out.
    line: Cell<Option<f64>>,
    /// The most lines the multiline editor shows now.
    rows: Cell<usize>,
}

impl Default for FormFit {
    fn default() -> Self {
        Self {
            heights: Default::default(),
            whole: Cell::new(false),
            room: Cell::new(None),
            // upstream's inset until the editor is measured
            frame: Cell::new(4.0),
            line: Cell::new(None),
            rows: Cell::new(MAX_LINES),
        }
    }
}

/// The measures [`FormFit::probe`] decides the layout from.
#[derive(Debug, Clone, Copy)]
pub(super) struct FitInputs {
    /// The space between the panel's parts, and its padding.
    pub gap: f64,
    pub padding: f64,
    /// How many of the title, message and strip there are.
    pub pinned: usize,
    pub form_min: f64,
    /// The content's tallest height.
    pub available: f64,
}

impl FormFit {
    /// Whether the whole panel scrolls, as the last frame measured.
    pub fn scrolls_whole(&self) -> bool {
        self.whole.get()
    }

    /// The multiline editor's frame around its lines, as the last frame measured.
    pub fn editor_frame(&self) -> f64 {
        self.frame.get()
    }

    /// The multiline editor's line height, as the last frame measured.
    pub fn editor_line(&self) -> Option<f64> {
        self.line.get()
    }

    /// Over the multiline editor laid out in `bounds` (its frame included), once painted: fits its most lines to the
    /// room its region has ([`editor_rows`]), so in a short anchor the editor itself gives way and scrolls to its caret
    /// instead of the region clipping it; a changed count takes effect on the next frame.
    pub fn fit_rows(&self, state: &Entity<TextareaState>, bounds: Bounds<Pixels>, window: &mut Window, cx: &App) {
        let editor = state.read(cx);
        let Some(line) = editor.line_height() else {
            return;
        };
        let (line, frame) = (
            f64::from(f32::from(line)),
            f64::from(f32::from(bounds.size.height - editor.input_bounds().size.height)).max(0.0),
        );
        self.line.set(Some(line));
        self.frame.set(frame);
        let rows = editor_rows(self.room.get(), frame, line);
        if rows != self.rows.replace(rows) {
            let state = state.clone();
            window.on_next_frame(move |window, cx| {
                state.update(cx, |state, cx| state.set_auto_grow(MIN_LINES.min(rows), rows, cx));
                window.refresh();
            });
        }
    }

    /// Over part `slot` (0 the title, 1 the message, 2 the strip): records its height once laid out; the strip's
    /// probe, the last, decides the layout, and a changed decision draws again on the next frame.
    pub fn probe(fit: &Rc<Self>, slot: usize, inputs: Option<FitInputs>) -> AnyElement {
        let fit = Rc::clone(fit);
        canvas(
            move |bounds, window, _| {
                fit.heights[slot].set(f64::from(f32::from(bounds.size.height)));
                let Some(inputs) = inputs else {
                    return;
                };
                #[expect(clippy::cast_precision_loss, reason = "at most three parts")]
                let gaps = inputs.gap * inputs.pinned as f64;
                let pinned = fit.heights.iter().map(Cell::get).sum::<f64>() + gaps + 2.0 * inputs.padding;
                let whole = scrolls_whole(pinned, inputs.form_min, inputs.available);
                fit.room.set((!whole).then(|| (inputs.available - pinned).max(0.0)));
                if whole != fit.whole.replace(whole) {
                    window.on_next_frame(|window, _| window.refresh());
                }
            },
            |_, (), _, _| {},
        )
        .absolute()
        .inset_0()
        .into_any_element()
    }
}

// MARK: - the choice list

/// How a choice row draws.
#[derive(Debug, Clone, Copy)]
pub(super) struct RowLook {
    pub gui: bool,
    pub cell: f64,
    pub colors: ChromeColors,
    /// The terminal style's ink and paper.
    pub foreground: Rgb,
    pub background: Rgb,
    /// One terminal character across.
    pub char_width: f64,
}

impl RowLook {
    /// The marker's width, which a subtitle is indented past.
    fn marker_width(&self) -> f64 {
        if self.gui { 16.0 } else { 3.0 * self.char_width }
    }

    /// The row's ink, inverted on the terminal style's cursor row.
    fn ink(&self, current: bool) -> Hsla {
        match (self.gui, current) {
            (true, _) => crate::chrome::color(self.colors.foreground, 1.0),
            (false, true) => crate::chrome::color(self.background, 1.0),
            (false, false) => crate::chrome::color(self.foreground, 1.0),
        }
    }

    /// The cursor row's mark: an accent wash, or the terminal style's inverted block.
    fn cursor_fill(&self) -> Hsla {
        if self.gui {
            crate::chrome::color(modal::ACCENT, 0.25)
        } else {
            crate::chrome::color(self.foreground, 1.0)
        }
    }

    /// The radio or checkbox marker: a drawn box or circle, or the terminal style's `[x]`/`(•)`.
    fn marker(&self, checked: bool, multiple: bool, current: bool) -> AnyElement {
        if !self.gui {
            let text = match (multiple, checked) {
                (true, true) => "[x]",
                (true, false) => "[ ]",
                (false, true) => "(\u{2022})",
                (false, false) => "( )",
            };
            return div()
                .flex_shrink_0()
                .whitespace_nowrap()
                .text_color(self.ink(current))
                .child(text)
                .into_any_element();
        }
        let white = crate::chrome::color(Rgb::WHITE, 1.0);
        let fill = if checked {
            crate::chrome::color(modal::ACCENT, 1.0)
        } else {
            crate::chrome::color(self.colors.foreground, 0.1)
        };
        div()
            .flex_shrink_0()
            .size(px(16.))
            .when(multiple, |mark| mark.rounded(px(4.)))
            .when(!multiple, gpui_kit::Styled::rounded_full)
            .bg(fill)
            .border_1()
            .border_color(crate::chrome::color(self.colors.foreground, 0.3))
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(11.))
            .text_color(white)
            .when(checked && multiple, |mark| mark.child("\u{2713}"))
            .when(checked && !multiple, |mark| {
                mark.child(div().size(px(6.)).rounded_full().bg(white))
            })
            .into_any_element()
    }

    /// A row's first line: its marker and, for a caller row, its label on one line, the cursor row marked.
    fn header(&self, marker: AnyElement, label: Option<&str>, current: bool) -> gpui_kit::Div {
        let cell = self.cell;
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(modal::logical(cell))
            .px(modal::logical(cell * 0.5))
            .py(modal::logical(cell * 0.15))
            .min_w(px(0.))
            .text_color(self.ink(current))
            .when(current, |header| {
                header
                    .bg(self.cursor_fill())
                    .when(self.gui, |header| header.rounded(px(5.)))
            })
            .child(marker)
            .when_some(label, |header, label| {
                header.child(
                    div()
                        .min_w(px(0.))
                        .truncate()
                        .child(SharedString::from(label.to_owned())),
                )
            })
    }

    /// A subtitle under its row: dimmed, wrapping, indented past the marker.
    fn subtitle(&self, text: &str) -> AnyElement {
        let ink = if self.gui {
            crate::chrome::color(self.colors.foreground, crate::chrome::DIM_ALPHA)
        } else {
            crate::chrome::color(self.foreground, 0.7)
        };
        div()
            .pl(modal::logical(self.cell * 1.5 + self.marker_width()))
            .text_color(ink)
            .child(SharedString::from(text.to_owned()))
            .into_any_element()
    }
}

impl AskDialog {
    /// The list's rows, the free-form row after them (upstream's `AskChoiceList`); `field` is that row's editor drawn.
    pub(super) fn render_list(
        &self,
        list: &AskChoiceForm,
        look: RowLook,
        field: Option<AnyElement>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let rows: Vec<AnyElement> = list
            .choices
            .iter()
            .enumerate()
            .map(|(index, choice)| self.render_row(index, choice, list.multiple, look, cx))
            .collect();
        let other = field.map(|field| self.render_other_row(list, look, field, cx));
        div()
            .flex()
            .flex_col()
            .w_full()
            .gap(modal::logical(look.cell * 0.6))
            .children(rows)
            .children(other)
            .into_any_element()
    }

    /// A caller row: a click picks or flips it and puts the cursor there.
    fn render_row(
        &self,
        index: usize,
        choice: &AskChoice,
        multiple: bool,
        look: RowLook,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let draft = self.draft();
        let (checked, current) = (draft.is_checked(&choice.id), draft.cursor() == index);
        let id = row_id(&choice.id);
        let header = look.header(look.marker(checked, multiple, current), Some(&choice.label), current);
        let subtitle = choice
            .subtitle
            .as_deref()
            .filter(|subtitle| !subtitle.is_empty())
            .map(|subtitle| look.subtitle(subtitle));
        div()
            .id(SharedString::from(id.clone()))
            .accessibility_id(SharedString::from(id.clone()))
            .role(if multiple {
                gpui_kit::Role::CheckBox
            } else {
                gpui_kit::Role::RadioButton
            })
            .debug_selector(move || id)
            .relative()
            .flex()
            .flex_col()
            .gap(modal::logical(look.cell * 0.2))
            .cursor_pointer()
            .child(header)
            .children(subtitle)
            .children(current.then(|| self.row_reveal()))
            .on_click(cx.listener(move |dialog, _, window, cx| dialog.toggle(index, window, cx)))
            .into_any_element()
    }

    /// The free-form row: its marker, dimmed until the text is not blank, toggles like a row's; the field edits.
    fn render_other_row(
        &self,
        list: &AskChoiceForm,
        look: RowLook,
        field: AnyElement,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let index = list.choices.len();
        let draft = self.draft();
        let current = draft.cursor() == index;
        let marker = look
            .header(
                look.marker(draft.other_selected(), list.multiple, current),
                None,
                current,
            )
            .flex_shrink_0()
            .opacity(if draft.other_enabled() { 1.0 } else { 0.4 });
        div()
            .id(OTHER_ROW_ID)
            .accessibility_id(OTHER_ROW_ID)
            .debug_selector(|| OTHER_ROW_ID.to_owned())
            .relative()
            .flex()
            .flex_row()
            .items_center()
            .gap(modal::logical(look.cell))
            .child(
                div()
                    .id("ask-other-marker")
                    .cursor_pointer()
                    .child(marker)
                    .on_click(cx.listener(move |dialog, _, window, cx| dialog.toggle(index, window, cx))),
            )
            // a content-sized list still gives the field room to type in (an `NSTextField`'s intrinsic width)
            .child(
                div()
                    .flex_1()
                    .min_w(modal::logical(look.cell * OTHER_FIELD_CELLS))
                    .child(field),
            )
            .children(current.then(|| self.row_reveal()))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests;
