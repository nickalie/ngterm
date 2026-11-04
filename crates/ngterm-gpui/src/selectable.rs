//! Text the user can select and copy but not edit (upstream's `.textSelection(.enabled)`): the Settings popup's paths
//! and diagnostics, a page's load error. GPUI text has no selection of its own, so each is a read-only field of
//! gpui-kit's, drawn without a field's frame so it reads as text: one line ([`Selectable::Line`], scrolled sideways
//! past its width, as upstream truncates a path) or several ([`Selectable::Lines`], growing with them). A click puts the
//! caret in it, a drag or Ctrl+A selects, Ctrl+C copies; typing changes nothing.

use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::{ActiveTheme as _, Size};
use gpui_kit::{
    AnyElement, App, AppContext as _, Entity, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, Pixels,
    Role, SharedString, Styled as _, Window, div, px,
};

/// What assistive technology reads it as: text, as upstream's selectable `Text` reads (static text), not the editable
/// field gpui-kit's input publishes (GPUI sets no read-only state on an AccessKit node, so AT-SPI would call the field
/// editable). Its value is its text, which AccessKit names a label after.
const TEXT_ROLE: Role = Role::Label;

/// What gpui-kit's field keeps free at the right of its wrap width (gpui-base's input `RIGHT_MARGIN`); its padding is
/// its size's `input_px`, the default size here.
const FIELD_RIGHT_MARGIN: Pixels = px(10.);

/// The most lines a multi-line text grows to before it scrolls.
const MAX_LINES: usize = 12;

/// A read-only selectable text's state.
#[derive(Clone)]
pub enum Selectable {
    Line(Entity<InputState>),
    Lines(Entity<TextareaState>),
}

impl Selectable {
    /// A one-line text holding `text`.
    pub fn line(text: &str, window: &mut Window, cx: &mut App) -> Self {
        let text = text.to_owned();
        Self::Line(cx.new(|cx| InputState::new(window, cx).default_value(text)))
    }

    /// A text of `text`'s lines.
    pub fn lines(text: &str, window: &mut Window, cx: &mut App) -> Self {
        let text = text.to_owned();
        Self::Lines(cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, MAX_LINES)
                .default_value(text)
        }))
    }

    /// The text it holds.
    #[must_use]
    pub fn text(&self, cx: &App) -> SharedString {
        match self {
            Self::Line(state) => state.read(cx).value(),
            Self::Lines(state) => state.read(cx).value(),
        }
    }

    /// Holds `text` from now on; nothing when it already does (so a selection survives an unchanged sync).
    pub fn set(&self, text: &str, window: &mut Window, cx: &mut App) {
        if self.text(cx).as_ref() == text {
            return;
        }
        let text = text.to_owned();
        match self {
            Self::Line(state) => state.update(cx, |state, cx| state.set_value(text, window, cx)),
            Self::Lines(state) => state.update(cx, |state, cx| state.set_value(text, window, cx)),
        }
    }

    /// [`Selectable::element`] in a box as wide as its widest line, capped at its parent's width, so a parent that
    /// centers its children centers the text: gpui-kit's multi-line field ignores `text_align`, and with no width of
    /// its own it would wrap after every character in a shrink-wrapped column. Lines past the cap wrap inside it.
    /// The width is measured as the field lays it out: in the theme's font (gpui-kit's root sets it), plus the
    /// padding a multi-line field keeps on both sides whatever its style says and the margin it keeps at the right of
    /// its wrap width; the box adds that margin on the left too, so the text itself sits in the middle.
    #[must_use]
    pub fn centered_element(&self, id: &str, color: Hsla, size: Pixels, window: &mut Window, cx: &App) -> AnyElement {
        let text = self.text(cx);
        let mut style = window.text_style();
        style.font_family = cx.theme().font_family.clone();
        let widest = text
            .lines()
            .map(|line| {
                let run = style.to_run(line.len());
                window
                    .text_system()
                    .shape_line(SharedString::from(line.to_owned()), size, &[run], None)
                    .width
            })
            .fold(px(0.), Pixels::max);
        let inset = Size::default().input_px() + FIELD_RIGHT_MARGIN;
        let selector = format!("{id}-box");
        div()
            .debug_selector(move || selector)
            // a pixel past the measure, so rounding never wraps the last word
            .w(widest + inset * 2. + px(1.))
            .max_w_full()
            .pl(FIELD_RIGHT_MARGIN)
            .child(self.element(id, color, size, false))
            .into_any_element()
    }

    /// The text drawn in `color` at `size`, monospaced when `mono`, as element `id` (and its accessibility id).
    #[must_use]
    pub fn element(&self, id: &str, color: Hsla, size: Pixels, mono: bool) -> AnyElement {
        let id = SharedString::from(id.to_owned());
        match self {
            Self::Line(state) => {
                let field = Input::new(state)
                    .readonly(true)
                    .appearance(false)
                    .role(TEXT_ROLE)
                    .accessibility_id(id)
                    .p_0()
                    .text_size(size)
                    .text_color(color);
                if mono { field.font_family("monospace") } else { field }.into_any_element()
            }
            Self::Lines(state) => {
                let field = Textarea::new(state)
                    .readonly(true)
                    .appearance(false)
                    .role(TEXT_ROLE)
                    .accessibility_id(id)
                    .p_0()
                    .text_size(size)
                    .text_color(color);
                if mono { field.font_family("monospace") } else { field }.into_any_element()
            }
        }
    }
}

#[cfg(test)]
mod tests;
