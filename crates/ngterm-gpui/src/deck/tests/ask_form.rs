//! A terminal ask's form on a real page (Task 17, #778 #779), upstream's `AskDialogViewTests` and
//! `ControlAskUITests.testCLIChoiceKeysCheckItemsAndAnswerTheSelection` in GPUI's test window: the editor taking the
//! keyboard at open, typing reaching the session's draft, the keys in the field and on the list, the cap and the
//! control rule, a composition keeping every key and the close chord, the draft surviving an unmount and a layout swap,
//! and the free-form row's hand-over of the keyboard. The panes run `cat`.

use gpui_kit::{Bounds, ClipboardItem, EntityInputHandler as _, InputHandler as _, Window, point};
use ngterm_core::model::{AskDraft, AskInputForm, AskInputMode};
use ngterm_proto::command::AskCmd;
use ngterm_proto::{AskButton, AskChoice, ControlAskOutcome, ControlAskResult};

use super::*;
use crate::ask_dialog::AskDialog;
use crate::ask_dialog::form::{Editor, GuardedInput};

/// A deck with gpui-kit's widgets set up and its first session's pane started; that session.
fn form_harness(cx: &mut TestAppContext) -> (Harness<'_>, Uuid) {
    cx.update(gpui_kit::init);
    let (library, window, ids) = three_sessions();
    let mut h = harness(cx, core_of(library), window);
    h.until("the pane to start", |h| h.realized(ids[0], StatusPane::Left));
    (h, ids[0])
}

/// Lets the frames the dialog asks for come (its panel draws once its anchor is measured).
fn frames(h: &mut Harness<'_>) {
    for _ in 0..4 {
        h.cx.run_until_parked();
        h.cx.update(Window::simulate_next_frame);
    }
    h.cx.run_until_parked();
}

/// Opens an ask titled "Pick" over `session`, shaped by `edit`, and draws it; its id.
fn open(h: &mut Harness<'_>, session: Uuid, edit: impl FnOnce(&mut Args)) -> String {
    h.dispatch(Command::Ask(AskCmd::Open), session, |args| {
        args.title = Some("Pick".to_owned());
        edit(args);
    });
    h.turn();
    frames(h);
    h.session(session).ask().expect("the ask waits").ask.id.clone()
}

fn input(mode: &str, value: Option<&str>) -> impl FnOnce(&mut Args) {
    let (mode, value) = (mode.to_owned(), value.map(str::to_owned));
    move |args| {
        args.input = Some(mode);
        args.value = value;
    }
}

fn choice(id: &str, subtitle: Option<&str>) -> AskChoice {
    AskChoice {
        id: id.to_owned(),
        label: id.to_uppercase(),
        subtitle: subtitle.map(str::to_owned),
    }
}

/// `ids` as a list, checkboxes when `multiple`, with a free-form row when `other`.
fn list(ids: &[&str], multiple: bool, other: bool) -> impl FnOnce(&mut Args) {
    let choices: Vec<AskChoice> = ids.iter().map(|id| choice(id, None)).collect();
    move |args| {
        args.choices = Some(choices);
        args.multiple = Some(multiple);
        args.other = other.then(|| "Something else".to_owned());
    }
}

fn dialog(h: &mut Harness<'_>, session: Uuid) -> Entity<AskDialog> {
    h.deck
        .read_with(h.cx, |deck, _| {
            deck.page(session).and_then(Page::ask).map(|ask| ask.dialog.clone())
        })
        .expect("the dialog is up")
}

fn editor(h: &mut Harness<'_>, session: Uuid) -> Editor {
    let dialog = dialog(h, session);
    dialog.read_with(h.cx, |dialog, _| dialog.editor().cloned().expect("an editor"))
}

/// Whether the form's editor holds the keyboard.
fn editor_holds(h: &mut Harness<'_>, session: Uuid) -> bool {
    let dialog = dialog(h, session);
    h.cx.update(|window, cx| dialog.read(cx).editor_focused(window, cx))
}

/// Whether the dialog itself (its key catcher) holds the keyboard.
fn catcher_holds(h: &mut Harness<'_>, session: Uuid) -> bool {
    let dialog = dialog(h, session);
    h.cx.update(|window, cx| dialog.read(cx).focus_handle().is_focused(window))
}

fn editor_text(h: &mut Harness<'_>, session: Uuid) -> String {
    let editor = editor(h, session);
    h.cx.update(|_, cx| editor.text(cx))
}

/// The session's draft in the core.
fn draft(h: &Harness<'_>, session: Uuid) -> AskDraft {
    h.session(session).ask().expect("the ask waits").draft.clone()
}

fn pending(h: &Harness<'_>, session: Uuid, id: &str) -> bool {
    h.session(session).ask().is_some_and(|ask| ask.ask.id == id)
}

/// Ask `id`'s outcome, as `ask.result` reports it.
fn result(h: &mut Harness<'_>, id: &str) -> ControlAskResult {
    let response = h.state.core.dispatch(&Request {
        cmd: Command::Ask(AskCmd::Result),
        target: Some(id.to_owned()),
        args: None,
    });
    assert!(response.ok, "{response:?}");
    serde_json::from_value(response.result.expect("a result")["ask"].clone()).expect("an ask result")
}

fn answered(id: &str, label: &str, index: i64) -> ControlAskResult {
    ControlAskResult {
        id: Some(id.to_owned()),
        label: Some(label.to_owned()),
        index: Some(index),
        ..ControlAskResult::outcome(ControlAskOutcome::Answered)
    }
}

/// Selects `range` of the editor's text.
fn select(h: &mut Harness<'_>, session: Uuid, range: std::ops::Range<usize>) {
    match editor(h, session) {
        Editor::Line(state) => state.update(h.cx, |state, cx| state.set_selected_range(range, cx)),
        Editor::Lines(state) => state.update(h.cx, |state, cx| state.set_selected_range(range, cx)),
    }
}

/// The window's input handler over the editor, as the form registers it.
fn input_handler(h: &mut Harness<'_>, session: Uuid) -> GuardedInput {
    let editor = editor(h, session);
    let mode = match editor {
        Editor::Line(_) => AskInputMode::Line,
        Editor::Lines(_) => AskInputMode::Multiline,
    };
    GuardedInput::new(&editor, mode, Bounds::default())
}

/// Puts `text` into the editor as an input method's marked text, as a composition does through the window.
fn compose(h: &mut Harness<'_>, session: Uuid, text: &str) {
    let mut handler = input_handler(h, session);
    h.cx.update(|window, cx| handler.replace_and_mark_text_in_range(None, text, None, window, cx));
}

/// Commits the composition.
fn commit(h: &mut Harness<'_>, session: Uuid) {
    match editor(h, session) {
        Editor::Line(state) => state.update_in(h.cx, gpui_kit::EntityInputHandler::unmark_text),
        Editor::Lines(state) => state.update_in(h.cx, gpui_kit::EntityInputHandler::unmark_text),
    }
}

/// Whether element `id` lies inside what scrolls the form: its region, else the whole content.
fn in_view(h: &mut Harness<'_>, id: &'static str) -> bool {
    let viewport =
        h.cx.debug_bounds("ask-form")
            .or_else(|| h.cx.debug_bounds("ask-content"))
            .expect("the content is laid out");
    let item = h.cx.debug_bounds(id).expect("the item is laid out");
    viewport.top() <= item.top() && item.bottom() <= viewport.bottom()
}

// MARK: - an input ask

/// Upstream's `testInputAskFocusesItsEditorAtOpenAndKeepsInputOwnership` and
/// `testTypingUpdatesTheDraftAndReturnAnswersWithTheText`.
#[gpui_kit::test]
fn an_input_ask_focuses_its_editor_and_return_answers_with_the_typed_text(cx: &mut TestAppContext) {
    let (mut h, session) = form_harness(cx);
    let token = h.token(session, StatusPane::Left);
    open(&mut h, session, input("line", Some("draft")));
    assert!(editor_holds(&mut h, session), "the editor takes the keyboard at open");
    assert!(!h.focused(session, &token));
    assert_eq!(editor_text(&mut h, session), "draft");
    h.turn();
    assert!(editor_holds(&mut h, session), "a later sync leaves it there");

    let id = open_fresh(&mut h, session, input("line", None));
    h.cx.simulate_input("hello world");
    h.turn();
    assert_eq!(
        draft(&h, session).text(),
        "hello world",
        "typing reaches the session's draft"
    );
    h.cx.simulate_keystrokes("enter");
    h.turn();
    assert_eq!(
        result(&mut h, &id),
        ControlAskResult {
            text: Some("hello world".to_owned()),
            ..answered("ok", "OK", 0)
        }
    );
    assert!(h.focused(session, &token), "the terminal takes the keyboard back");
}

/// Escapes the pending ask over `session`, then opens a new one shaped by `edit`; its id.
fn open_fresh(h: &mut Harness<'_>, session: Uuid, edit: impl FnOnce(&mut Args)) -> String {
    h.cx.simulate_keystrokes("escape");
    h.turn();
    assert!(h.session(session).ask().is_none());
    open(h, session, edit)
}

/// Upstream's `testTabMovesTheHighlightToCancelWhichEscapesWithoutText` and `testEscInTheEditorEscapes`.
#[gpui_kit::test]
fn tab_moves_to_cancel_which_escapes_without_text_and_esc_escapes(cx: &mut TestAppContext) {
    let (mut h, session) = form_harness(cx);
    let id = open(&mut h, session, input("line", Some("x")));
    h.cx.simulate_keystrokes("tab");
    assert!(
        editor_holds(&mut h, session),
        "Tab moves the highlight, not the keyboard"
    );
    assert_eq!(
        dialog(&mut h, session).read_with(h.cx, |dialog, _| dialog.highlighted()),
        Some(1)
    );
    h.cx.simulate_keystrokes("enter");
    h.turn();
    assert_eq!(
        result(&mut h, &id),
        ControlAskResult::outcome(ControlAskOutcome::Escaped)
    );

    let id = open(&mut h, session, input("line", None));
    h.cx.simulate_keystrokes("escape");
    h.turn();
    assert_eq!(
        result(&mut h, &id),
        ControlAskResult::outcome(ControlAskOutcome::Escaped)
    );
}

/// Upstream's `testMultilineReturnAddsALineAndCommandReturnAnswers`, with Linux's Ctrl-Return.
#[gpui_kit::test]
fn multiline_return_adds_a_line_and_the_submit_chord_answers(cx: &mut TestAppContext) {
    let (mut h, session) = form_harness(cx);
    let id = open(&mut h, session, input("multiline", None));
    assert!(editor_holds(&mut h, session));
    h.cx.simulate_input("one");
    h.cx.simulate_keystrokes("enter");
    h.cx.simulate_input("two");
    h.turn();
    assert!(pending(&h, session, &id), "Return is a newline here");
    assert_eq!(draft(&h, session).text(), "one\ntwo");
    let submit = if cfg!(target_os = "macos") {
        "cmd-enter"
    } else {
        "ctrl-enter"
    };
    h.cx.simulate_keystrokes(submit);
    h.turn();
    assert_eq!(result(&mut h, &id).text.as_deref(), Some("one\ntwo"));
}

/// Upstream's `testAnEditAtTheCapIsRefusedWithoutLosingTheEnd`, in both modes.
#[gpui_kit::test]
fn an_edit_at_the_cap_is_refused_without_losing_the_end(cx: &mut TestAppContext) {
    let (mut h, session) = form_harness(cx);
    let full = format!("{}Z", "x".repeat(AskInputForm::MAX_TEXT_BYTES - 1));
    for mode in ["line", "multiline"] {
        open(&mut h, session, input(mode, Some(&full)));
        select(&mut h, session, 0..0);
        h.cx.simulate_input("A");
        h.turn();
        assert_eq!(editor_text(&mut h, session), full, "{mode}: refused whole");
        assert_eq!(draft(&h, session).text(), full, "{mode}");
        select(&mut h, session, 0..1);
        h.cx.simulate_input("A");
        h.turn();
        assert_eq!(
            draft(&h, session).text(),
            format!("A{}", &full[1..]),
            "{mode}: a replacement fits"
        );
        h.cx.simulate_keystrokes("escape");
        h.turn();
    }
}

/// Upstream's `testACompositionThatWouldPassTheCapIsRefusedAndOneThatFitsCommits`, in both modes, through the
/// window's input handler.
#[gpui_kit::test]
fn a_composition_past_the_cap_is_refused_and_one_that_fits_commits(cx: &mut TestAppContext) {
    let (mut h, session) = form_harness(cx);
    let nearly = "x".repeat(AskInputForm::MAX_TEXT_BYTES - 2);
    for mode in ["line", "multiline"] {
        open(&mut h, session, input(mode, Some(&nearly)));
        let end = nearly.len();
        select(&mut h, session, end..end);
        let mut handler = input_handler(&mut h, session);
        h.cx.update(|window, cx| handler.replace_and_mark_text_in_range(Some(end..end), "abc", Some(3..3), window, cx));
        assert_eq!(
            editor_text(&mut h, session),
            nearly,
            "{mode}: the composition never shows"
        );
        h.cx.update(|window, cx| handler.replace_and_mark_text_in_range(Some(end..end), "ab", Some(2..2), window, cx));
        let marked = h.cx.update(|window, cx| handler.marked_text_range(window, cx));
        h.cx.update(|window, cx| handler.replace_text_in_range(marked, "ab", window, cx));
        h.turn();
        assert!(
            h.cx.update(|window, cx| handler.marked_text_range(window, cx))
                .is_none(),
            "{mode}: committed"
        );
        assert_eq!(editor_text(&mut h, session), format!("{nearly}ab"), "{mode}");
        assert_eq!(draft(&h, session).text(), format!("{nearly}ab"), "{mode}");
        h.cx.simulate_keystrokes("escape");
        h.turn();
    }
}

/// A refused edit leaves the editor as it was: Return at the cap inserts no newline, and the editing history still
/// undoes what came before.
#[gpui_kit::test]
fn a_refused_edit_keeps_the_selection_and_the_editing_history(cx: &mut TestAppContext) {
    let (mut h, session) = form_harness(cx);
    let nearly = "x".repeat(AskInputForm::MAX_TEXT_BYTES - 2);
    open(&mut h, session, input("multiline", Some(&nearly)));
    let end = nearly.len();
    select(&mut h, session, end..end);
    h.cx.simulate_input("ab");
    h.turn();
    assert_eq!(draft(&h, session).text(), format!("{nearly}ab"), "fits the cap");
    select(&mut h, session, 3..3);
    for refused in ["enter", "c"] {
        if refused == "enter" {
            h.cx.simulate_keystrokes(refused);
        } else {
            h.cx.simulate_input(refused);
        }
        h.turn();
        assert_eq!(draft(&h, session).text(), format!("{nearly}ab"), "{refused} refused");
        let editor = editor(&mut h, session);
        assert_eq!(
            h.cx.update(|_, cx| editor.selection(cx)),
            3..3,
            "{refused}: the caret stays"
        );
    }
    select(&mut h, session, end + 2..end + 2);
    h.cx.simulate_keystrokes("ctrl-z");
    h.turn();
    assert_eq!(draft(&h, session).text(), nearly, "the typing before is still undone");
}

/// A change through a path with no hook (here the editor's own handler, past the guard) that breaks the cap is undone
/// with the selection it had, not one guessed from the two texts: in a run of one letter, inserting that letter
/// matches the text after the caret wherever it goes.
#[gpui_kit::test]
fn a_change_past_the_guard_is_undone_with_the_selection_it_had(cx: &mut TestAppContext) {
    let (mut h, session) = form_harness(cx);
    let full = "x".repeat(AskInputForm::MAX_TEXT_BYTES);
    for mode in ["line", "multiline"] {
        open(&mut h, session, input(mode, Some(&full)));
        select(&mut h, session, 0..0);
        frames(&mut h);
        match editor(&mut h, session) {
            Editor::Line(state) => state.update_in(h.cx, |state, window, cx| {
                state.replace_text_in_range(Some(0..0), "x", window, cx);
            }),
            Editor::Lines(state) => state.update_in(h.cx, |state, window, cx| {
                state.replace_text_in_range(Some(0..0), "x", window, cx);
            }),
        }
        h.turn();
        assert_eq!(editor_text(&mut h, session), full, "{mode}: undone");
        let editor = editor(&mut h, session);
        assert_eq!(
            h.cx.update(|_, cx| editor.selection(cx)),
            0..0,
            "{mode}: the caret where it was"
        );
        h.cx.simulate_keystrokes("escape");
        h.turn();
    }
}

/// Upstream's `AskTextEditor.sizeThatFits` in a short anchor: the multiline editor gives way to the form's region,
/// down to two lines, instead of the region clipping it, and its own scroll keeps the caret in view as lines are typed.
#[gpui_kit::test]
fn the_multiline_editor_gives_way_in_a_short_anchor_and_keeps_its_caret_in_view(cx: &mut TestAppContext) {
    let (mut h, session) = form_harness(cx);
    open(&mut h, session, input("multiline", None));
    let grown =
        h.cx.debug_bounds("ask-input")
            .expect("the editor is laid out")
            .size
            .height;
    h.cx.simulate_resize(size(px(803.), px(250.)));
    frames(&mut h);
    let region =
        h.cx.debug_bounds("ask-form")
            .expect("the region is pinned between title and buttons");
    let field = h.cx.debug_bounds("ask-input").expect("the editor is laid out");
    assert!(
        field.size.height < grown,
        "the editor gave way: {field:?}, {grown:?} grown"
    );
    assert!(
        field.bottom() <= region.bottom() + px(0.5),
        "the editor fits its region: {field:?} in {region:?}"
    );
    for _ in 0..8 {
        h.cx.simulate_input("line");
        h.cx.simulate_keystrokes("enter");
    }
    h.cx.simulate_input("last");
    h.turn();
    frames(&mut h);
    let region = h.cx.debug_bounds("ask-form").expect("the region");
    let Editor::Lines(state) = editor(&mut h, session) else {
        panic!("a multiline editor");
    };
    // the caret as laid out in the editor's text, moved by the editor's own scroll
    let (caret, scroll) = state.read_with(h.cx, |state, _| {
        let (caret, _) = state.cursor_layout().expect("the caret is laid out");
        (caret, state.scroll_offset().y)
    });
    assert!(scroll < px(0.), "the editor scrolled itself, not the region");
    let caret = Bounds::new(point(caret.origin.x, caret.origin.y + scroll), caret.size);
    assert!(
        region.top() <= caret.top() && caret.bottom() <= region.bottom() + px(0.5),
        "the caret stays in view: {caret:?} in {region:?}"
    );
    assert!(draft(&h, session).text().ends_with("line\nlast"), "typed in full");
}

/// Upstream's `testAPasteWithAForbiddenControlIsRefusedWhole`.
#[gpui_kit::test]
fn a_paste_with_a_forbidden_control_is_refused_whole(cx: &mut TestAppContext) {
    let (mut h, session) = form_harness(cx);
    let cases = [
        ("line", "a\u{1}b", "kept"),
        ("line", "a\tb", "kept"),
        ("line", "a\nb", "kept"),
        ("multiline", "a\u{1}b", "kept"),
        ("multiline", "a\r\nb", "kept"),
        ("multiline", "a\u{7f}", "kept"),
        ("multiline", "a\tb\nc", "kepta\tb\nc"),
    ];
    for (mode, paste, expected) in cases {
        open(&mut h, session, input(mode, Some("kept")));
        select(&mut h, session, 4..4);
        h.cx.write_to_clipboard(ClipboardItem::new_string(paste.to_owned()));
        h.cx.simulate_keystrokes("ctrl-v");
        h.turn();
        assert_eq!(editor_text(&mut h, session), expected, "{mode} {paste:?}");
        assert_eq!(draft(&h, session).text(), expected, "{mode} {paste:?}");
        h.cx.simulate_keystrokes("escape");
        h.turn();
    }
}

/// Upstream's `testACompositionInTheOtherFieldKeepsUpDownReturnAndEscNative` for the input field, and
/// `testCommandWWithTheEditorFocusedDismissesTheAsk` with the composition rule of `escapePendingSessionAsk`.
#[gpui_kit::test]
fn a_composition_keeps_every_key_and_the_close_chord_keeps_the_ask(cx: &mut TestAppContext) {
    let (mut h, session) = form_harness(cx);
    let id = open(&mut h, session, input("line", None));
    for key in ["enter", "escape", "tab", "ctrl-shift-q"] {
        compose(&mut h, session, "\u{304b}");
        h.cx.simulate_keystrokes(key);
        h.turn();
        assert!(pending(&h, session, &id), "{key} keeps the ask while composing");
        assert!(h.session(session).ask().is_some(), "{key}");
    }
    assert_eq!(
        dialog(&mut h, session).read_with(h.cx, |dialog, _| dialog.highlighted()),
        Some(0),
        "Tab moved nothing"
    );
    commit(&mut h, session);
    h.cx.simulate_keystrokes("ctrl-shift-q");
    h.turn();
    assert_eq!(
        result(&mut h, &id),
        ControlAskResult::outcome(ControlAskOutcome::Escaped),
        "the close chord escapes once the composition is committed"
    );
    assert_eq!(h.store().sessions().count(), 3, "and closes nothing behind it");
}

/// Upstream's `testTheDraftSurvivesTheDialogUnmountingAndComingBack` and a layout swap: the session's draft comes
/// back in a new dialog, and a panel too short for its form keeps its editor, the keyboard and the text.
#[gpui_kit::test]
fn the_draft_survives_an_unmount_and_a_layout_swap(cx: &mut TestAppContext) {
    let (mut h, session) = form_harness(cx);
    let id = open(&mut h, session, input("multiline", None));
    h.cx.simulate_input("kept");
    h.turn();
    let other = h.store().sessions().map(Session::id).find(|id| *id != session).unwrap();
    h.store().select_session(Some(other));
    h.state.settle();
    h.turn();
    assert!(
        h.deck
            .read_with(h.cx, |deck, _| deck.page(session).and_then(Page::ask).is_none()),
        "hidden, its dialog goes"
    );
    h.store().select_session(Some(session));
    h.state.settle();
    h.turn();
    frames(&mut h);
    assert!(pending(&h, session, &id));
    assert_eq!(
        editor_text(&mut h, session),
        "kept",
        "the remounted editor starts from the draft"
    );
    assert!(editor_holds(&mut h, session));

    let focus = |h: &mut Harness<'_>| {
        let editor = editor(h, session);
        h.cx.update(|_, cx| editor.focus_handle(cx))
    };
    let before = focus(&mut h);
    h.cx.simulate_resize(size(px(803.), px(120.)));
    frames(&mut h);
    assert!(
        editor_holds(&mut h, session),
        "the swap keeps the keyboard on the editor"
    );
    assert!(focus(&mut h) == before, "the same editor");
    assert_eq!(editor_text(&mut h, session), "kept");
    assert_eq!(draft(&h, session).text(), "kept");
}

// MARK: - a choice list

/// Upstream's `testCLIChoiceKeysCheckItemsAndAnswerTheSelection` (`--multiple a,b,c`: Down Space Down Space Return)
/// and `testChoiceKeysMoveTheCursorToggleAndAnswerWithTheSelection`.
#[gpui_kit::test]
fn list_keys_move_the_cursor_toggle_and_answer_with_the_selection(cx: &mut TestAppContext) {
    let (mut h, session) = form_harness(cx);
    let id = open(&mut h, session, list(&["a", "b", "c"], true, false));
    assert!(
        catcher_holds(&mut h, session),
        "a list without a free-form row keeps the keyboard on the dialog"
    );
    h.cx.simulate_keystrokes("down space down space");
    h.turn();
    assert_eq!(draft(&h, session).selected(), ["b", "c"]);
    h.cx.simulate_keystrokes("enter");
    h.turn();
    assert_eq!(
        result(&mut h, &id),
        ControlAskResult {
            selected: Some(vec!["b".to_owned(), "c".to_owned()]),
            ..answered("ok", "OK", 0)
        }
    );

    let id = open(&mut h, session, |args| {
        list(&["a", "b", "c"], true, false)(args);
        args.checked = Some(vec!["c".to_owned()]);
        args.choices.as_mut().unwrap()[1].subtitle = Some("two\nlines".to_owned());
    });
    h.cx.simulate_keystrokes("up space up space");
    h.turn();
    assert_eq!(draft(&h, session).selected(), ["a", "b", "c"]);
    h.cx.simulate_keystrokes("enter");
    h.turn();
    assert_eq!(
        result(&mut h, &id).selected,
        Some(vec!["a".to_owned(), "b".to_owned(), "c".to_owned()])
    );
}

/// Upstream's `testUpAndDownStillMoveButtonsOfAPlainAsk`; a list's hotkeys still answer.
#[gpui_kit::test]
fn a_plain_asks_up_and_down_move_buttons_and_a_lists_hotkey_answers(cx: &mut TestAppContext) {
    let (mut h, session) = form_harness(cx);
    let buttons = || {
        vec![
            AskButton {
                id: "a".to_owned(),
                label: "A".to_owned(),
                hotkey: None,
            },
            AskButton {
                id: "b".to_owned(),
                label: "B".to_owned(),
                hotkey: Some("b".to_owned()),
            },
        ]
    };
    let id = open(&mut h, session, |args| args.buttons = Some(buttons()));
    h.cx.simulate_keystrokes("down enter");
    h.turn();
    assert_eq!(result(&mut h, &id).id.as_deref(), Some("b"));

    let id = open(&mut h, session, |args| {
        list(&["x", "y"], false, false)(args);
        args.buttons = Some(buttons());
    });
    h.cx.simulate_keystrokes("b");
    h.turn();
    assert_eq!(
        result(&mut h, &id),
        ControlAskResult {
            selected: Some(vec!["x".to_owned()]),
            ..answered("b", "B", 1)
        }
    );
}

/// Upstream's `testALongListOpensScrolledToTheCheckedItem`.
#[gpui_kit::test]
fn a_long_list_opens_scrolled_to_its_checked_item(cx: &mut TestAppContext) {
    let (mut h, session) = form_harness(cx);
    open(&mut h, session, |args| {
        args.choices = Some(
            (1..=50)
                .map(|index| choice(&format!("i{index}"), Some("one\ntwo\nthree")))
                .collect(),
        );
        args.checked = Some(vec!["i50".to_owned()]);
    });
    assert!(
        h.cx.debug_bounds("ask-form").is_some(),
        "the list scrolls between the title and the buttons"
    );
    assert!(in_view(&mut h, "ask-choice-i50"), "opened on the checked item");
    assert!(!in_view(&mut h, "ask-choice-i1"));
    assert!(h.cx.debug_bounds("ask-button-ok").is_some(), "the buttons stay");
}

// MARK: - the free-form row

/// Upstream's `testTheCatcherKeepsFocusOnAnOrdinaryRowAndTheCursorHandsItToTheOtherFieldAndBack`.
#[gpui_kit::test]
fn the_cursor_hands_the_keyboard_to_the_free_form_row_and_back(cx: &mut TestAppContext) {
    let (mut h, session) = form_harness(cx);
    open(&mut h, session, list(&["a", "b"], false, true));
    assert!(
        catcher_holds(&mut h, session),
        "an ordinary row leaves the keyboard on the dialog"
    );
    h.cx.simulate_keystrokes("down down");
    h.turn();
    assert!(editor_holds(&mut h, session), "the free-form row's field takes it");
    assert!(draft(&h, session).cursor_on_other());
    h.cx.simulate_keystrokes("down");
    h.turn();
    assert!(editor_holds(&mut h, session), "Down clamps on the last row");
    h.cx.simulate_keystrokes("up");
    h.turn();
    assert!(catcher_holds(&mut h, session), "Up hands the keyboard back");
    assert_eq!(draft(&h, session).cursor(), 1);
}

/// Upstream's `testTypingInTheOtherFieldSelectsItAndReturnAnswersWithTheText`.
#[gpui_kit::test]
fn typing_in_the_free_form_row_selects_it_and_return_answers_with_its_text(cx: &mut TestAppContext) {
    let (mut h, session) = form_harness(cx);
    let id = open(&mut h, session, list(&["a", "b"], false, true));
    h.cx.simulate_keystrokes("down down");
    h.cx.simulate_input(" ");
    h.turn();
    assert!(!draft(&h, session).other_selected(), "blank text selects nothing");
    assert_eq!(draft(&h, session).selected(), ["a"]);
    h.cx.simulate_input("a b");
    h.turn();
    assert_eq!(draft(&h, session).text(), " a b");
    assert!(draft(&h, session).other_selected());
    h.cx.simulate_keystrokes("enter");
    h.turn();
    assert_eq!(
        result(&mut h, &id),
        ControlAskResult {
            text: Some(" a b".to_owned()),
            selected: Some(Vec::new()),
            ..answered("ok", "OK", 0)
        }
    );
}

/// Upstream's `testLettersInTheOtherFieldNeverFireButtonHotkeys`, and a click into the field moving the cursor there.
#[gpui_kit::test]
fn letters_in_the_free_form_row_never_fire_hotkeys_and_a_click_puts_the_cursor_there(cx: &mut TestAppContext) {
    let (mut h, session) = form_harness(cx);
    let id = open(&mut h, session, |args| {
        list(&["a", "b"], true, true)(args);
        args.buttons = Some(vec![AskButton {
            id: "go".to_owned(),
            label: "Go".to_owned(),
            hotkey: Some("g".to_owned()),
        }]);
    });
    assert!(catcher_holds(&mut h, session));
    let field =
        h.cx.debug_bounds("ask-input")
            .expect("the row's field is laid out")
            .center();
    h.cx.simulate_click(field, gpui_kit::Modifiers::default());
    // the field hears of its focus once the frame after the click is drawn
    frames(&mut h);
    h.turn();
    assert!(
        draft(&h, session).cursor_on_other(),
        "a click into the field moves the cursor there"
    );
    assert!(editor_holds(&mut h, session));
    h.cx.simulate_input("xg");
    h.turn();
    assert!(pending(&h, session, &id), "g typed in the field is text");
    assert_eq!(editor_text(&mut h, session), "xg");
    assert_eq!(draft(&h, session).text(), "xg");
}

/// Upstream's `testTheOtherRowIsReachedAndScrolledIntoViewAfterFiftyItems`.
#[gpui_kit::test]
fn the_free_form_row_is_reached_and_scrolled_into_view_after_fifty_items(cx: &mut TestAppContext) {
    let (mut h, session) = form_harness(cx);
    open(&mut h, session, |args| {
        args.choices = Some(
            (1..=50)
                .map(|index| choice(&format!("i{index}"), Some("one\ntwo")))
                .collect(),
        );
        args.multiple = Some(true);
        args.other = Some("Something else".to_owned());
    });
    assert!(!in_view(&mut h, "ask-other"));
    for _ in 0..60 {
        h.cx.simulate_keystrokes("down");
    }
    h.turn();
    frames(&mut h);
    assert!(editor_holds(&mut h, session), "the row's field holds the keyboard");
    assert!(draft(&h, session).cursor_on_other());
    assert!(in_view(&mut h, "ask-other"), "and is scrolled into view");
}

/// `ask.cancel` still resolves an ask whose editor holds the keyboard, and the terminal takes it back.
#[gpui_kit::test]
fn ask_cancel_resolves_an_ask_whose_editor_holds_the_keyboard(cx: &mut TestAppContext) {
    let (mut h, session) = form_harness(cx);
    let token = h.token(session, StatusPane::Left);
    let id = open(&mut h, session, input("multiline", Some("draft")));
    assert!(editor_holds(&mut h, session));
    let response = h.state.core.dispatch(&Request {
        cmd: Command::Ask(AskCmd::Cancel),
        target: Some(id.clone()),
        args: None,
    });
    assert!(response.ok, "{response:?}");
    h.state.settle();
    h.turn();
    assert_eq!(result(&mut h, &id).result, ControlAskOutcome::Cancelled);
    assert!(
        h.deck
            .read_with(h.cx, |deck, _| deck.page(session).and_then(Page::ask).is_none())
    );
    assert!(h.focused(session, &token), "the terminal takes the keyboard back");
}
