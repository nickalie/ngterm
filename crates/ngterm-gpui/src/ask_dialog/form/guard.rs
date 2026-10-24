//! The editor's input handler: the pre-change hook of upstream's `askEditAllowed` (`shouldChangeText`), which
//! gpui-kit's editors lack (only the one-line field has a validator, and it never sees marked text).
//!
//! The editor registers its own handler with the window as it paints; the form paints this one after it, so the
//! window hands every platform text edit to this one (GPUI takes the last handler a frame registered). Each typed
//! character, input-method composition and commit, and platform paste is checked against what the editor holds before
//! it reaches the editor: one that would pass the cap or bring in a control the mode forbids is dropped whole, so the
//! text, the selection and the editing history stay as they were. Everything else goes to the editor's own handler.

use std::ops::Range;

use gpui_kit::{
    App, Bounds, ClipboardItem, ElementInputHandler, InputHandler, Pixels, Point, TextInputConfiguration,
    UTF16Selection, Window,
};
use ngterm_core::model::AskInputMode;

use super::{Editor, change_allowed};

/// The editor's own input handler behind the guard.
pub struct GuardedInput {
    inner: Box<dyn InputHandler>,
    editor: Editor,
    mode: AskInputMode,
}

impl GuardedInput {
    /// The guard over `editor`'s handler, the editor's text laid out in `bounds`.
    #[must_use]
    pub fn new(editor: &Editor, mode: AskInputMode, bounds: Bounds<Pixels>) -> Self {
        let inner: Box<dyn InputHandler> = match editor {
            Editor::Line(state) => Box::new(ElementInputHandler::new(bounds, state.clone())),
            Editor::Lines(state) => Box::new(ElementInputHandler::new(bounds, state.clone())),
        };
        Self {
            inner,
            editor: editor.clone(),
            mode,
        }
    }

    /// Whether replacing `range_utf16` (the marked text, else the selection, when `None`, as the editor reads it) with
    /// `text` keeps the editor within the rules.
    fn allows(&mut self, range_utf16: Option<Range<usize>>, text: &str, window: &mut Window, cx: &mut App) -> bool {
        let current = self.editor.text(cx);
        let range = match range_utf16.or_else(|| self.inner.marked_text_range(window, cx)) {
            Some(range) => byte_offset(&current, range.start)..byte_offset(&current, range.end),
            None => self.editor.selection(cx),
        };
        change_allowed(&current, range, text, self.mode)
    }
}

/// The byte offset of the UTF-16 offset `utf16` in `text`, clamped to its end.
fn byte_offset(text: &str, utf16: usize) -> usize {
    let mut units = 0;
    for (index, ch) in text.char_indices() {
        if units >= utf16 {
            return index;
        }
        units += ch.len_utf16();
    }
    text.len()
}

impl InputHandler for GuardedInput {
    fn selected_text_range(
        &mut self,
        ignore_disabled_input: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<UTF16Selection> {
        self.inner.selected_text_range(ignore_disabled_input, window, cx)
    }

    fn marked_text_range(&mut self, window: &mut Window, cx: &mut App) -> Option<Range<usize>> {
        self.inner.marked_text_range(window, cx)
    }

    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<String> {
        self.inner.text_for_range(range_utf16, adjusted_range, window, cx)
    }

    fn replace_text_in_range(
        &mut self,
        replacement_range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut App,
    ) {
        if self.allows(replacement_range.clone(), text, window, cx) {
            self.inner.replace_text_in_range(replacement_range, text, window, cx);
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut App,
    ) {
        if self.allows(range_utf16.clone(), new_text, window, cx) {
            self.inner
                .replace_and_mark_text_in_range(range_utf16, new_text, new_selected_range, window, cx);
        }
    }

    fn unmark_text(&mut self, window: &mut Window, cx: &mut App) {
        self.inner.unmark_text(window, cx);
    }

    fn paste(&mut self, item: ClipboardItem, window: &mut Window, cx: &mut App) {
        if item.text().is_none_or(|text| self.allows(None, &text, window, cx)) {
            self.inner.paste(item, window, cx);
        }
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Bounds<Pixels>> {
        self.inner.bounds_for_range(range_utf16, window, cx)
    }

    fn character_index_for_point(&mut self, point: Point<Pixels>, window: &mut Window, cx: &mut App) -> Option<usize> {
        self.inner.character_index_for_point(point, window, cx)
    }

    fn set_selected_text_range(&mut self, range_utf16: Range<usize>, window: &mut Window, cx: &mut App) {
        self.inner.set_selected_text_range(range_utf16, window, cx);
    }

    fn element_bounds(&mut self, window: &mut Window, cx: &mut App) -> Option<Bounds<Pixels>> {
        self.inner.element_bounds(window, cx)
    }

    fn text_length_utf16(&mut self, window: &mut Window, cx: &mut App) -> Option<usize> {
        self.inner.text_length_utf16(window, cx)
    }

    fn apple_press_and_hold_enabled(&mut self) -> bool {
        self.inner.apple_press_and_hold_enabled()
    }

    fn accepts_text_input(&mut self, window: &mut Window, cx: &mut App) -> bool {
        self.inner.accepts_text_input(window, cx)
    }

    fn text_input_editable_range(&mut self, window: &mut Window, cx: &mut App) -> Option<Range<usize>> {
        self.inner.text_input_editable_range(window, cx)
    }

    fn prefers_ime_for_printable_keys(&mut self, window: &mut Window, cx: &mut App) -> bool {
        self.inner.prefers_ime_for_printable_keys(window, cx)
    }

    fn text_input_configuration(&mut self, window: &mut Window, cx: &mut App) -> TextInputConfiguration {
        self.inner.text_input_configuration(window, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::byte_offset;

    #[test]
    fn utf16_offsets_map_to_bytes() {
        let text = "a\u{f1}\u{1f600}b";
        assert_eq!(byte_offset(text, 0), 0);
        assert_eq!(byte_offset(text, 1), 1);
        assert_eq!(byte_offset(text, 2), 3, "after the two-byte char");
        assert_eq!(byte_offset(text, 4), 7, "after the surrogate pair");
        assert_eq!(byte_offset(text, 5), 8);
        assert_eq!(byte_offset(text, 99), 8, "clamped to the end");
    }
}
