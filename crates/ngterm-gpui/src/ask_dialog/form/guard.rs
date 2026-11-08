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
    use gpui_kit::{
        App, Bounds, ClipboardEntry, ClipboardItem, ElementInputHandler, Image, ImageFormat, InputHandler,
        TestAppContext, VisualTestContext, Window, point, px, size,
    };
    use ngterm_core::chrome::Rgb;
    use ngterm_core::model::{AskAlignment, AskInputForm, AskInputMode, AskStyle, PendingAsk};
    use uuid::Uuid;

    use super::{Editor, GuardedInput, byte_offset};
    use crate::ask_dialog::{AskDialog, AskOpening, AskOwner, Look};

    /// An editor of `mode` holding `text` in a test window, made by a dialog that does not listen to it, so nothing
    /// but the guard stands between an edit and the editor.
    fn editor<'a>(cx: &'a mut TestAppContext, mode: AskInputMode, text: &str) -> (Editor, &'a mut VisualTestContext) {
        cx.update(gpui_kit::init);
        let pending = PendingAsk {
            id: "A1".to_owned(),
            title: "Name?".to_owned(),
            message: None,
            buttons: Vec::new(),
            default_id: None,
            destructive_id: None,
            style: AskStyle::Gui,
            align: AskAlignment::Right,
            width: None,
            anchor: None,
            form: None,
            cancel_id: None,
        };
        let opening = AskOpening {
            ask: &pending,
            draft: None,
            owner: AskOwner::Session(Uuid::new_v4()),
            look: Look {
                family: None,
                size: 13.0,
                foreground: Rgb::WHITE,
                background: Rgb::BLACK,
            },
            poster: crate::state::detached_poster(),
        };
        let (dialog, cx) = cx.add_window_view(|window, cx| AskDialog::new(opening, window, cx));
        let editor = dialog.update_in(cx, |_, window, cx| Editor::new(mode, text, None, window, cx));
        (editor, cx)
    }

    fn bounds() -> Bounds<gpui_kit::Pixels> {
        Bounds::new(point(px(0.), px(0.)), size(px(400.), px(40.)))
    }

    /// The editor's own handler, unguarded.
    fn own(editor: &Editor) -> Box<dyn InputHandler> {
        match editor {
            Editor::Line(state) => Box::new(ElementInputHandler::new(bounds(), state.clone())),
            Editor::Lines(state) => Box::new(ElementInputHandler::new(bounds(), state.clone())),
        }
    }

    #[gpui_kit::test]
    fn typed_text_within_the_rules_reaches_the_editor_and_a_forbidden_control_is_dropped(cx: &mut TestAppContext) {
        for mode in [AskInputMode::Line, AskInputMode::Multiline] {
            let (editor, cx) = editor(cx, mode, "ab");
            cx.update(|window, cx| {
                let mut guard = GuardedInput::new(&editor, mode, bounds());
                guard.replace_text_in_range(Some(2..2), "\u{1}", window, cx);
                assert_eq!(editor.text(cx), "ab", "{mode:?}: no control character in any mode");
                guard.replace_text_in_range(Some(2..2), "c", window, cx);
                assert_eq!(editor.text(cx), "abc", "{mode:?}");
                guard.replace_text_in_range(Some(0..0), "\t", window, cx);
                let tab = if mode == AskInputMode::Line { "abc" } else { "\tabc" };
                assert_eq!(editor.text(cx), tab, "{mode:?}: a tab only where the mode takes one");
            });
        }
    }

    #[gpui_kit::test]
    fn the_cap_counts_the_replaced_utf16_range_and_the_selection_in_bytes(cx: &mut TestAppContext) {
        let cap = AskInputForm::MAX_TEXT_BYTES;
        // two bytes but one UTF-16 unit each, so the cap is full
        let full = "\u{e9}".repeat(cap / 2);
        let (editor, cx) = editor(cx, AskInputMode::Multiline, &full);
        cx.update(|window, cx| {
            let mut guard = GuardedInput::new(&editor, AskInputMode::Multiline, bounds());
            guard.replace_text_in_range(Some(0..0), "x", window, cx);
            assert!(editor.holds(&full, cx), "an insertion past the cap is dropped");
            // one UTF-16 unit is the first two bytes: two bytes in their place stay within the cap
            guard.replace_text_in_range(Some(0..1), "xy", window, cx);
            let rest = "\u{e9}".repeat(cap / 2 - 1);
            assert!(
                editor.holds(&format!("xy{rest}"), cx),
                "a replacement within the cap goes in"
            );
            // with no range the selection (in bytes) is what is replaced: two characters, four bytes, two UTF-16 units
            editor.restore(&full, 0..4, window, cx);
            guard.replace_text_in_range(None, "abcde", window, cx);
            assert!(
                editor.holds(&full, cx),
                "five bytes over a four-byte selection pass the cap"
            );
            guard.replace_text_in_range(None, "abcd", window, cx);
            let rest = "\u{e9}".repeat(cap / 2 - 2);
            assert!(
                editor.holds(&format!("abcd{rest}"), cx),
                "four bytes over it fill the cap"
            );
        });
    }

    #[gpui_kit::test]
    fn a_composition_is_checked_as_it_is_marked_and_as_it_is_committed(cx: &mut TestAppContext) {
        let cap = AskInputForm::MAX_TEXT_BYTES;
        let start = "a".repeat(cap - 3);
        let (editor, cx) = editor(cx, AskInputMode::Line, &start);
        cx.update(|window, cx| {
            let mut guard = GuardedInput::new(&editor, AskInputMode::Line, bounds());
            let end = cap - 3;
            editor.restore(&start, end..end, window, cx);
            guard.replace_and_mark_text_in_range(None, "\u{304b}", None, window, cx);
            assert!(
                editor.holds(&format!("{start}\u{304b}"), cx),
                "the composition fills the cap"
            );
            assert_eq!(guard.marked_text_range(window, cx), Some(end..end + 1));
            // the marked text is what a further composition or the commit replaces
            guard.replace_and_mark_text_in_range(None, "\u{304b}\u{306a}", None, window, cx);
            assert!(
                editor.holds(&format!("{start}\u{304b}"), cx),
                "a longer composition past the cap is dropped whole"
            );
            assert_eq!(
                guard.marked_text_range(window, cx),
                Some(end..end + 1),
                "still composing"
            );
            guard.replace_and_mark_text_in_range(None, "\n", None, window, cx);
            assert!(
                editor.holds(&format!("{start}\u{304b}"), cx),
                "a line composes no newline"
            );
            guard.replace_text_in_range(None, "\u{706b}", window, cx);
            assert!(
                editor.holds(&format!("{start}\u{706b}"), cx),
                "the commit replaces the marked text"
            );
            assert_eq!(guard.marked_text_range(window, cx), None, "committed");
            // a composition over a range, ended by the input method without a commit
            guard.replace_and_mark_text_in_range(Some(end..end + 1), "\u{304b}", None, window, cx);
            assert_eq!(guard.marked_text_range(window, cx), Some(end..end + 1));
            guard.unmark_text(window, cx);
            assert_eq!(guard.marked_text_range(window, cx), None, "no longer composing");
            assert!(editor.holds(&format!("{start}\u{304b}"), cx), "the composed text stays");
        });
    }

    #[gpui_kit::test]
    fn a_paste_is_checked_on_its_text_and_one_without_text_goes_to_the_editor(cx: &mut TestAppContext) {
        for mode in [AskInputMode::Line, AskInputMode::Multiline] {
            let (editor, cx) = editor(cx, mode, "ab");
            cx.update(|window, cx| {
                let mut guard = GuardedInput::new(&editor, mode, bounds());
                editor.restore("ab", 2..2, window, cx);
                guard.paste(ClipboardItem::new_string("c\nd".to_owned()), window, cx);
                let newline = if mode == AskInputMode::Line { "ab" } else { "abc\nd" };
                assert_eq!(
                    editor.text(cx),
                    newline,
                    "{mode:?}: a newline only where the mode takes one"
                );
                guard.paste(ClipboardItem::new_string("\u{7f}".to_owned()), window, cx);
                assert_eq!(editor.text(cx), newline, "{mode:?}: no control character");
                let picture = ClipboardItem {
                    entries: vec![ClipboardEntry::Image(Image::from_bytes(ImageFormat::Png, Vec::new()))],
                };
                guard.paste(picture, window, cx);
                assert_eq!(editor.text(cx), newline, "{mode:?}: a picture brings no text");
            });
        }
        let (editor, cx) = editor(cx, AskInputMode::Line, "ab");
        cx.update(|window, cx| {
            let mut guard = GuardedInput::new(&editor, AskInputMode::Line, bounds());
            editor.restore("ab", 2..2, window, cx);
            guard.paste(ClipboardItem::new_string("cd".to_owned()), window, cx);
            assert_eq!(editor.text(cx), "abcd");
        });
    }

    /// Asserts that `guard` answers every platform query (no edit) as `own` does.
    fn answers_alike(guard: &mut dyn InputHandler, own: &mut dyn InputHandler, window: &mut Window, cx: &mut App) {
        let selection = |handler: &mut dyn InputHandler, window: &mut Window, cx: &mut App| {
            handler
                .selected_text_range(false, window, cx)
                .map(|selection| (selection.range, selection.reversed))
        };
        assert_eq!(selection(guard, window, cx), selection(own, window, cx));
        let (mut guarded, mut owned) = (None, None);
        assert_eq!(
            guard.text_for_range(1..4, &mut guarded, window, cx),
            own.text_for_range(1..4, &mut owned, window, cx)
        );
        assert_eq!(guarded, owned);
        assert_eq!(guard.marked_text_range(window, cx), own.marked_text_range(window, cx));
        assert_eq!(
            guard.bounds_for_range(0..1, window, cx),
            own.bounds_for_range(0..1, window, cx)
        );
        let inside = point(px(4.), px(4.));
        assert_eq!(
            guard.character_index_for_point(inside, window, cx),
            own.character_index_for_point(inside, window, cx)
        );
        assert_eq!(guard.element_bounds(window, cx), own.element_bounds(window, cx));
        assert_eq!(guard.text_length_utf16(window, cx), own.text_length_utf16(window, cx));
        assert_eq!(guard.apple_press_and_hold_enabled(), own.apple_press_and_hold_enabled());
        assert_eq!(guard.accepts_text_input(window, cx), own.accepts_text_input(window, cx));
        assert_eq!(
            guard.text_input_editable_range(window, cx),
            own.text_input_editable_range(window, cx)
        );
        assert_eq!(
            guard.prefers_ime_for_printable_keys(window, cx),
            own.prefers_ime_for_printable_keys(window, cx)
        );
        assert_eq!(
            guard.text_input_configuration(window, cx),
            own.text_input_configuration(window, cx)
        );
    }

    #[gpui_kit::test]
    fn every_query_is_answered_as_the_editors_own_handler_answers_it(cx: &mut TestAppContext) {
        for mode in [AskInputMode::Line, AskInputMode::Multiline] {
            let (editor, cx) = editor(cx, mode, "hello");
            cx.update(|window, cx| {
                let mut guard = GuardedInput::new(&editor, mode, bounds());
                let mut own = own(&editor);
                editor.restore("hello", 1..3, window, cx);
                let selected = guard
                    .selected_text_range(false, window, cx)
                    .map(|selection| selection.range);
                assert_eq!(selected, Some(1..3), "{mode:?}");
                let mut adjusted = None;
                assert_eq!(
                    guard.text_for_range(1..4, &mut adjusted, window, cx).as_deref(),
                    Some("ell")
                );
                answers_alike(&mut guard, own.as_mut(), window, cx);
                // the platform's selection goes to the editor, which sets it as its own handler would
                guard.set_selected_text_range(0..5, window, cx);
                let guarded = editor.selection(cx);
                own.set_selected_text_range(0..5, window, cx);
                assert_eq!(guarded, editor.selection(cx), "{mode:?}");
            });
        }
    }

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
