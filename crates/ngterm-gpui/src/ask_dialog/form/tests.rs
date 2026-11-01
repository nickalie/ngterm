//! The form's pure parts: the guard, the editor's keys, the draft edits and the layout's choice (upstream's
//! `AskDialogViewTests` cases that need no window).

use gpui_kit::Keystroke;
use ngterm_core::model::{AskChoiceForm, AskDraft, AskForm, AskInputForm, AskInputMode};
use ngterm_proto::AskChoice;

use super::*;

fn key(source: &str) -> Keystroke {
    Keystroke::parse(source).unwrap()
}

#[test]
fn an_edit_is_held_to_the_cap_and_the_modes_control_rule() {
    let cap = AskInputForm::MAX_TEXT_BYTES;
    assert!(edit_allowed(&"x".repeat(cap), AskInputMode::Line));
    assert!(!edit_allowed(&"x".repeat(cap + 1), AskInputMode::Line));
    assert!(!edit_allowed("a\u{1}b", AskInputMode::Multiline));
    assert!(!edit_allowed("a\tb", AskInputMode::Line), "a line takes no tab");
    assert!(!edit_allowed("a\nb", AskInputMode::Line), "nor a newline");
    assert!(edit_allowed("a\tb\nc", AskInputMode::Multiline));
    assert!(
        !edit_allowed("a\r\nb", AskInputMode::Multiline),
        "a carriage return is no newline"
    );
    assert!(!edit_allowed("a\u{7f}", AskInputMode::Multiline));
}

#[test]
fn a_paste_is_refused_whole() {
    // upstream's testAPasteWithAForbiddenControlIsRefusedWhole cases, at the end of "kept"
    let cases = [
        (AskInputMode::Line, "a\u{1}b", false),
        (AskInputMode::Line, "a\tb", false),
        (AskInputMode::Line, "a\nb", false),
        (AskInputMode::Multiline, "a\u{1}b", false),
        (AskInputMode::Multiline, "a\r\nb", false),
        (AskInputMode::Multiline, "a\u{7f}", false),
        (AskInputMode::Multiline, "a\tb\nc", true),
    ];
    for (mode, paste, allowed) in cases {
        assert_eq!(change_allowed("kept", 4..4, paste, mode), allowed, "{mode:?} {paste:?}");
    }
    let full = "x".repeat(AskInputForm::MAX_TEXT_BYTES);
    assert!(!change_allowed(&full, 0..0, "A", AskInputMode::Line), "past the cap");
    assert!(
        change_allowed(&full, 0..1, "A", AskInputMode::Line),
        "over a selection it replaces"
    );
}

#[test]
fn the_multiline_editors_return_inserts_a_newline() {
    assert!(inserts_newline(&key("enter"), AskInputMode::Multiline));
    assert!(inserts_newline(&key("shift-enter"), AskInputMode::Multiline));
    assert!(!inserts_newline(&key("enter"), AskInputMode::Line), "a line answers");
    assert!(
        !inserts_newline(&key("ctrl-enter"), AskInputMode::Multiline),
        "the submit chord"
    );
    assert!(!inserts_newline(&key("a"), AskInputMode::Multiline));
}

#[test]
fn the_editors_keys_drive_the_dialog() {
    use AskInputMode::{Line, Multiline};
    let ask = |key: AskKey| Some(FieldKey::Ask(key));
    let field = |source: &str, mode, in_list| field_key(&key(source), mode, in_list, false, false);
    assert_eq!(field("tab", Line, false), ask(AskKey::Forward));
    assert_eq!(field("shift-tab", Multiline, false), ask(AskKey::Backward));
    assert_eq!(field("escape", Multiline, false), ask(AskKey::Cancel));
    assert_eq!(field("enter", Line, false), ask(AskKey::Activate), "one line answers");
    assert_eq!(
        field("enter", Multiline, false),
        None,
        "the editing area takes a newline"
    );
    let submit = if cfg!(target_os = "macos") {
        "cmd-enter"
    } else {
        "ctrl-enter"
    };
    assert_eq!(
        field(submit, Multiline, false),
        ask(AskKey::Activate),
        "the submit chord"
    );
    let shifted = if cfg!(target_os = "macos") {
        "cmd-shift-enter"
    } else {
        "ctrl-shift-enter"
    };
    assert_eq!(field(shifted, Multiline, false), None, "only the bare chord submits");
}

#[test]
fn the_free_form_rows_editor_hands_up_and_down_to_the_list() {
    use AskInputMode::Line;
    let ask = |key: AskKey| Some(FieldKey::Ask(key));
    let field = |source: &str, mode, in_list| field_key(&key(source), mode, in_list, false, false);
    assert_eq!(
        field("up", Line, false),
        None,
        "a standalone field keeps its caret keys"
    );
    assert_eq!(
        field("up", Line, true),
        ask(AskKey::Up),
        "the free-form row hands them to the list"
    );
    assert_eq!(field("down", Line, true), ask(AskKey::Down));
    assert_eq!(field("g", Line, true), None, "letters are text, never hotkeys");
    assert_eq!(field("space", Line, true), None, "Space is text in the row");
    assert_eq!(field("left", Line, false), None);
}

#[test]
fn a_composition_keeps_every_key_and_the_close_chord() {
    for source in ["up", "down", "enter", "escape", "tab"] {
        assert_eq!(
            field_key(&key(source), AskInputMode::Line, true, false, true),
            None,
            "{source} stays the composition's"
        );
    }
    let close = key("ctrl-shift-q");
    assert_eq!(
        field_key(&close, AskInputMode::Line, false, true, true),
        Some(FieldKey::Swallow),
        "the close chord is kept, closing nothing"
    );
    assert_eq!(
        field_key(&close, AskInputMode::Multiline, false, true, false),
        Some(FieldKey::Ask(AskKey::Cancel)),
        "without a composition it escapes the ask"
    );
}

fn choices(ids: &[&str]) -> Vec<AskChoice> {
    ids.iter()
        .map(|id| AskChoice {
            id: (*id).to_owned(),
            label: id.to_uppercase(),
            subtitle: None,
        })
        .collect()
}

#[test]
fn draft_edits_apply_as_the_core_applies_them() {
    let mut list = AskChoiceForm::new(choices(&["a", "b"]));
    list.multiple = true;
    list.other = Some("Something else".to_owned());
    let form = AskForm::Choices(list);
    let mut draft = AskDraft::new(Some(&form));
    DraftEdit::MoveCursor(1).apply(&mut draft);
    DraftEdit::Toggle(draft.cursor()).apply(&mut draft);
    assert_eq!(draft.selected(), ["b"]);
    DraftEdit::MoveCursor(isize::MAX).apply(&mut draft);
    assert!(draft.cursor_on_other(), "the free-form row is the last stop");
    DraftEdit::Text("kept".to_owned()).apply(&mut draft);
    assert_eq!(draft.other_text(), Some("kept"), "non-blank text selects the row");
}

#[test]
fn the_form_keeps_its_least_height_and_the_panel_scrolls_when_even_that_does_not_fit() {
    let line = 20.0;
    let input = |mode| AskForm::Input(AskInputForm::new(mode));
    // a one-line field's own height: the GUI box, line + 12, and the terminal field, line + 4 (upstream's #842)
    assert!((form_min_height(&input(AskInputMode::Line), line, 10.0, true) - 32.0).abs() < 1e-9);
    assert!((form_min_height(&input(AskInputMode::Line), line, 10.0, false) - 24.0).abs() < 1e-9);
    assert!(
        (form_min_height(&input(AskInputMode::Multiline), line, 10.0, true) - 50.0).abs() < 1e-9,
        "two lines in the editor's frame"
    );
    let list = AskForm::Choices(AskChoiceForm::new(choices(&["a"])));
    assert!((form_min_height(&list, line, 10.0, true) - 60.0).abs() < 1e-9);
    assert!(
        !scrolls_whole(200.0, 60.0, 300.0),
        "the region shrinks between the pinned parts"
    );
    assert!(scrolls_whole(260.0, 60.0, 300.0), "the whole panel scrolls");
}

#[test]
fn the_multiline_editor_gives_way_to_its_regions_room() {
    // upstream's sizeThatFits: as many lines as the offered height takes, never fewer than two
    let (line, frame) = (20.0, 10.0);
    assert_eq!(
        editor_rows(None, frame, line),
        MAX_LINES,
        "unbounded while the whole panel scrolls"
    );
    assert_eq!(
        editor_rows(Some(1000.0), frame, line),
        MAX_LINES,
        "at most the grown height"
    );
    assert_eq!(editor_rows(Some(110.0), frame, line), 5);
    assert_eq!(
        editor_rows(Some(109.0), frame, line),
        4,
        "a line that does not fit whole goes"
    );
    assert_eq!(editor_rows(Some(50.0), frame, line), SHORT_ROWS);
    assert_eq!(editor_rows(Some(10.0), frame, line), SHORT_ROWS, "never fewer than two");
    assert_eq!(
        editor_rows(Some(100.0), frame, 0.0),
        MAX_LINES,
        "before a line is measured"
    );
}

#[test]
fn row_ids_are_tagged_and_the_free_form_row_carries_no_caller_prefix() {
    assert_eq!(row_id("a"), "ask-choice-a");
    assert_eq!(
        row_id(OTHER_ROW_ID),
        "ask-choice-ask-other",
        "a caller id named like the row stays apart"
    );
    assert!(!OTHER_ROW_ID.starts_with("ask-choice-"));
}
