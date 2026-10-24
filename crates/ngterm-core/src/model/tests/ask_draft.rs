//! The ask draft, its answers and the validated remote answer (upstream's `AskTests` form cases).

use ngterm_proto::{AskButton, AskChoice, ControlAskOutcome, ControlAskResult};
use uuid::Uuid;

use super::support::add;
use crate::model::{
    AppStore, AskAlignment, AskAnchor, AskChoiceForm, AskDraft, AskForm, AskInputForm, AskInputMode, AskStyle,
    MAX_ASK_CHOICES, PendingAsk,
};

fn form_buttons() -> Vec<AskButton> {
    [("ok", "OK"), ("cancel", "Cancel")]
        .into_iter()
        .map(|(id, label)| AskButton {
            id: id.to_owned(),
            label: label.to_owned(),
            hotkey: None,
        })
        .collect()
}

/// Items labelled with their upper-cased ids.
fn choices(ids: &[&str]) -> Vec<AskChoice> {
    ids.iter()
        .map(|id| AskChoice {
            id: (*id).to_owned(),
            label: id.to_uppercase(),
            subtitle: None,
        })
        .collect()
}

fn list(ids: &[&str], multiple: bool, checked: &[&str]) -> AskForm {
    AskForm::Choices(AskChoiceForm {
        multiple,
        checked: checked.iter().map(|id| (*id).to_owned()).collect(),
        ..AskChoiceForm::new(choices(ids))
    })
}

fn other_form(multiple: bool, checked: &[&str]) -> AskForm {
    let AskForm::Choices(form) = list(&["a", "b", "c"], multiple, checked) else {
        unreachable!()
    };
    AskForm::Choices(AskChoiceForm {
        other: Some("Something else".to_owned()),
        ..form
    })
}

fn ask(form: Option<AskForm>, cancel: Option<&str>) -> PendingAsk {
    PendingAsk {
        id: "a".to_owned(),
        title: "Pick".to_owned(),
        message: None,
        buttons: form_buttons(),
        default_id: None,
        destructive_id: None,
        style: AskStyle::Terminal,
        align: AskAlignment::Right,
        width: None,
        anchor: None,
        form,
        cancel_id: cancel.map(str::to_owned),
    }
}

fn answered(index: i64, text: Option<&str>, selected: Option<&[&str]>) -> ControlAskResult {
    let (id, label) = [("ok", "OK"), ("cancel", "Cancel")][usize::try_from(index).unwrap()];
    ControlAskResult {
        id: Some(id.to_owned()),
        label: Some(label.to_owned()),
        index: Some(index),
        text: text.map(str::to_owned),
        selected: selected.map(|ids| ids.iter().map(|id| (*id).to_owned()).collect()),
        ..ControlAskResult::outcome(ControlAskOutcome::Answered)
    }
}

fn escaped() -> ControlAskResult {
    ControlAskResult::outcome(ControlAskOutcome::Escaped)
}

fn strings(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

#[test]
fn an_input_answer_carries_the_draft_text_and_cancel_escapes_without_it() {
    let form = AskForm::Input(AskInputForm {
        value: "draft".into(),
        ..AskInputForm::new(AskInputMode::Multiline)
    });
    let ask = ask(Some(form.clone()), Some("cancel"));
    let mut draft = AskDraft::new(Some(&form));
    assert_eq!(draft.text(), "draft");
    draft.set_text("  line one\nline two  ");
    assert_eq!(
        ask.answer(0, Some(&draft)),
        Some(answered(0, Some("  line one\nline two  "), None))
    );
    assert_eq!(ask.answer(1, Some(&draft)), Some(escaped()));
    assert_eq!(ask.answer(2, Some(&draft)), None);
}

#[test]
fn a_choice_answer_lists_checked_ids_in_caller_order() {
    let form = list(&["a", "b", "c"], true, &["c"]);
    let ask = ask(Some(form.clone()), Some("cancel"));
    let mut draft = AskDraft::new(Some(&form));
    draft.toggle(0);
    assert_eq!(draft.selected(), strings(&["a", "c"]));
    let answer = ask.answer(0, Some(&draft)).unwrap();
    assert_eq!(answer.selected, Some(strings(&["a", "c"])));
    assert_eq!(answer.text, None);
}

#[test]
fn an_answer_without_a_draft_uses_the_forms_initial_values() {
    let ask = ask(Some(list(&["a", "b"], false, &[])), None);
    assert_eq!(ask.answer(0, None).unwrap().selected, Some(strings(&["a"])));
}

#[test]
fn the_cancel_role_applies_without_a_form() {
    let ask = ask(None, Some("cancel"));
    assert_eq!(ask.answer(1, None), Some(escaped()));
    assert_eq!(ask.answer(0, None), Some(answered(0, None, None)));
}

#[test]
fn a_radio_draft_starts_on_the_first_item_and_picks_exactly_one() {
    let mut draft = AskDraft::new(Some(&list(&["a", "b", "c"], false, &[])));
    assert_eq!(draft.selected(), strings(&["a"]));
    assert_eq!(draft.cursor(), 0);
    draft.move_cursor(2);
    draft.toggle_cursor();
    assert_eq!(draft.selected(), strings(&["c"]));
    draft.toggle_cursor();
    assert_eq!(draft.selected(), strings(&["c"]));
}

#[test]
fn the_draft_cursor_starts_on_the_first_checked_item_and_stops_at_the_ends() {
    let mut draft = AskDraft::new(Some(&list(&["a", "b", "c"], true, &["c", "b"])));
    assert_eq!(draft.cursor(), 1);
    draft.move_cursor(-5);
    assert_eq!(draft.cursor(), 0);
    draft.move_cursor(9);
    assert_eq!(draft.cursor(), 2);
}

#[test]
fn a_checkbox_draft_toggles_and_may_end_empty() {
    let mut draft = AskDraft::new(Some(&list(&["a", "b"], true, &["b"])));
    draft.toggle(1);
    assert!(draft.selected().is_empty());
    draft.toggle(7);
    assert!(draft.selected().is_empty());
    assert_eq!(draft.cursor(), 1);
}

#[test]
fn typing_selects_the_other_row_and_blank_text_disables_it() {
    for multiple in [false, true] {
        let mut draft = AskDraft::new(Some(&other_form(multiple, &[])));
        assert!(!draft.other_enabled());
        draft.set_text(" ");
        assert!(!draft.other_enabled() && !draft.other_selected() && draft.other_text().is_none());
        draft.set_text(" x");
        assert!(draft.other_enabled() && draft.other_selected());
        assert_eq!(draft.other_text(), Some(" x"));
        draft.set_text("  ");
        assert!(!draft.other_enabled() && !draft.other_selected() && draft.other_text().is_none());
        assert_eq!(draft.text(), "  ", "{multiple}");
    }
}

#[test]
fn a_radio_other_restores_the_pick_it_took_over_from() {
    let mut draft = AskDraft::new(Some(&other_form(false, &["b"])));
    draft.set_text("x");
    assert!(draft.selected().is_empty() && draft.other_selected());
    draft.set_text("xyz");
    draft.set_text("");
    assert!(draft.selected() == strings(&["b"]) && !draft.other_selected());
}

#[test]
fn a_radio_pick_elsewhere_keeps_the_other_text_and_clearing_it_keeps_that_pick() {
    let mut draft = AskDraft::new(Some(&other_form(false, &["b"])));
    draft.set_text("x");
    draft.toggle(2);
    assert_eq!(draft.selected(), strings(&["c"]));
    assert!(!draft.other_selected() && draft.text() == "x" && draft.other_enabled());
    draft.set_text("");
    assert_eq!(draft.selected(), strings(&["c"]));
    draft.set_text("y");
    assert!(draft.selected().is_empty() && draft.other_selected());
    draft.set_text("");
    assert_eq!(draft.selected(), strings(&["c"]));
}

#[test]
fn a_radio_other_without_an_initial_pick_falls_back_to_the_first_item() {
    let mut draft = AskDraft::new(Some(&other_form(false, &[])));
    draft.toggle(3);
    assert!(!draft.other_selected());
    draft.set_text("x");
    draft.set_text("");
    assert_eq!(draft.selected(), strings(&["a"]));
}

#[test]
fn a_radio_other_marker_picks_an_enabled_row_and_is_a_no_op_once_picked() {
    let mut draft = AskDraft::new(Some(&other_form(false, &["b"])));
    draft.set_text("x");
    draft.toggle(0);
    draft.toggle(3);
    assert!(draft.other_selected() && draft.selected().is_empty());
    draft.toggle(3);
    assert!(draft.other_selected() && draft.selected().is_empty());
    draft.set_text("");
    assert_eq!(draft.selected(), strings(&["a"]));
}

#[test]
fn a_checkbox_other_marker_toggles_while_enabled_and_keeps_the_text() {
    let mut draft = AskDraft::new(Some(&other_form(true, &["a"])));
    draft.toggle(3);
    assert!(!draft.other_selected());
    draft.set_text("x");
    draft.toggle(3);
    assert!(!draft.other_selected() && draft.other_enabled() && draft.text() == "x");
    assert_eq!(draft.other_text(), None);
    assert_eq!(draft.selected(), strings(&["a"]));
    draft.set_text("xy");
    assert!(draft.other_selected());
    assert_eq!(draft.other_text(), Some("xy"));
}

#[test]
fn the_cursor_reaches_the_other_row_after_fifty_items_and_clamps_there() {
    let items: Vec<AskChoice> = (1..=MAX_ASK_CHOICES)
        .map(|n| AskChoice {
            id: format!("i{n}"),
            label: format!("I{n}"),
            subtitle: None,
        })
        .collect();
    let form = AskForm::Choices(AskChoiceForm {
        multiple: true,
        other: Some("Other".into()),
        ..AskChoiceForm::new(items)
    });
    let mut draft = AskDraft::new(Some(&form));
    draft.move_cursor(100);
    assert!(draft.cursor() == MAX_ASK_CHOICES && draft.cursor_on_other());
    draft.move_cursor(1);
    assert_eq!(draft.cursor(), MAX_ASK_CHOICES);
    draft.move_cursor(-1);
    assert!(!draft.cursor_on_other());
    let mut plain = AskDraft::new(Some(&list(&["a", "b"], false, &[])));
    plain.move_cursor(5);
    assert!(plain.cursor() == 1 && !plain.cursor_on_other());
}

#[test]
fn an_other_answer_returns_raw_text_only_while_selected() {
    for multiple in [false, true] {
        let AskForm::Choices(base) = list(&["a", "b", "other"], multiple, &["other"]) else {
            unreachable!()
        };
        let form = AskForm::Choices(AskChoiceForm {
            other: Some("Something else".into()),
            ..base
        });
        let ask = ask(Some(form.clone()), Some("cancel"));
        let mut draft = AskDraft::new(Some(&form));
        draft.set_text("  mine  ");
        let answer = ask.answer(0, Some(&draft)).unwrap();
        assert_eq!(answer.text.as_deref(), Some("  mine  "));
        let expected: &[&str] = if multiple { &["other"] } else { &[] };
        assert_eq!(answer.selected, Some(strings(expected)));
        assert_eq!(ask.answer(1, Some(&draft)), Some(escaped()));
        draft.toggle(0);
        let text = ask.answer(0, Some(&draft)).unwrap().text;
        assert_eq!(text.as_deref(), multiple.then_some("  mine  "));
        if multiple {
            draft.toggle(3);
        }
        assert_eq!(ask.answer(0, Some(&draft)).unwrap().text, None, "{multiple}");
    }
}

#[test]
fn a_validated_other_answer_follows_the_forms_shape() {
    let long = "x".repeat(AskInputForm::MAX_TEXT_BYTES + 1);
    let cases: [(bool, &[&str], Option<&str>, bool); 10] = [
        (false, &[], Some(" mine "), true),
        (false, &["a"], None, true),
        (true, &["a", "c"], Some(" mine "), true),
        (true, &[], None, true),
        (false, &["a"], Some("mine"), false),
        (false, &[], None, false),
        (true, &["a"], Some("  "), false),
        (true, &[], Some("a\u{1}b"), false),
        (true, &[], Some("a\tb"), false),
        (true, &[], Some(long.as_str()), false),
    ];
    for (multiple, selected, text, accepted) in cases {
        let ask = ask(Some(other_form(multiple, &[])), Some("cancel"));
        let picked = strings(selected);
        let result = ask.validated_answer("ok", text, Some(&picked));
        if accepted {
            assert_eq!(
                result,
                Some(answered(0, text, Some(selected))),
                "{multiple} {selected:?} {text:?}"
            );
        } else {
            assert_eq!(result, None, "{multiple} {selected:?} {text:?}");
        }
    }
}

#[test]
fn a_validated_answer_keeps_the_caller_order_and_known_buttons() {
    let ask = ask(Some(list(&["a", "b", "c"], true, &[])), Some("cancel"));
    assert_eq!(ask.validated_answer("ok", None, Some(&strings(&["c", "a"]))), None);
    assert_eq!(ask.validated_answer("ok", None, Some(&strings(&["a", "a"]))), None);
    assert_eq!(ask.validated_answer("ok", None, Some(&strings(&["z"]))), None);
    assert_eq!(ask.validated_answer("ok", None, None), None);
    assert_eq!(ask.validated_answer("nope", None, Some(&[])), None);
    let input = AskForm::Input(AskInputForm::new(AskInputMode::Line));
    let line = self::ask(Some(input), None);
    assert_eq!(
        line.validated_answer("ok", Some("x"), None),
        Some(answered(0, Some("x"), None))
    );
    assert_eq!(line.validated_answer("ok", None, None), None);
    assert_eq!(line.validated_answer("ok", Some("a\nb"), None), None);
    assert_eq!(line.validated_answer("ok", Some("x"), Some(&[])), None);
    let plain = self::ask(None, None);
    assert_eq!(plain.validated_answer("ok", None, None), Some(answered(0, None, None)));
    assert_eq!(plain.validated_answer("ok", Some("x"), None), None);
    assert_eq!(plain.validated_answer("ok", None, Some(&[])), None);
}

#[test]
fn retyping_the_same_text_reselects_the_row() {
    for multiple in [false, true] {
        let mut draft = AskDraft::new(Some(&other_form(multiple, &[])));
        draft.set_text("x");
        draft.toggle(if multiple { 3 } else { 0 });
        assert!(!draft.other_selected());
        draft.set_text("x");
        assert!(draft.other_selected(), "{multiple}");
        assert_eq!(draft.other_text(), Some("x"));
    }
}

#[test]
fn a_validated_choice_answer_refuses_text_without_an_other_row() {
    let ask = ask(Some(list(&["a", "b"], true, &[])), Some("cancel"));
    assert_eq!(ask.validated_answer("ok", Some("x"), Some(&strings(&["a"]))), None);
    assert_eq!(ask.validated_answer("cancel", None, None), Some(escaped()));
    assert_eq!(ask.validated_answer("cancel", Some("x"), None), None);
}

#[test]
fn draft_text_is_clamped_to_the_byte_cap() {
    let cap = AskInputForm::MAX_TEXT_BYTES;
    for (length, expected) in [(cap, cap), (cap + 1, cap)] {
        let mut draft = AskDraft::new(Some(&AskForm::Input(AskInputForm::new(AskInputMode::Line))));
        draft.set_text(&"x".repeat(length));
        assert_eq!(draft.text().len(), expected);
    }
}

#[test]
fn clamping_never_splits_a_multibyte_character() {
    let text = "é".repeat(AskInputForm::MAX_TEXT_BYTES / 2) + "é";
    let clamped = AskInputForm::clamped(&text);
    assert_eq!(clamped.len(), AskInputForm::MAX_TEXT_BYTES);
    assert!(clamped.chars().all(|ch| ch == 'é'));
    // an odd cap position backs off to the boundary before
    let shifted = format!("x{text}");
    assert_eq!(AskInputForm::clamped(&shifted).len(), AskInputForm::MAX_TEXT_BYTES - 1);
}

#[test]
fn clamping_drops_a_grapheme_cluster_that_straddles_the_cap_whole() {
    let cap = AskInputForm::MAX_TEXT_BYTES;
    // "e" plus a combining acute is one user-perceived character; its base fits, its mark does not
    let text = "x".repeat(cap - 1) + "e\u{301}";
    assert_eq!(AskInputForm::clamped(&text), "x".repeat(cap - 1));
    // a cluster ending exactly at the cap stays
    let fits = "x".repeat(cap - 3) + "e\u{301}" + "y";
    assert_eq!(AskInputForm::clamped(&fits), &fits[..cap]);
}

#[test]
fn control_characters_are_forbidden_except_newline_and_tab_in_multiline() {
    let cases = [
        ("plain é \"q\"", AskInputMode::Line, false),
        ("a\nb\tc", AskInputMode::Multiline, false),
        ("a\nb", AskInputMode::Line, true),
        ("a\tb", AskInputMode::Line, true),
        ("a\u{1}b", AskInputMode::Multiline, true),
        ("a\rb", AskInputMode::Multiline, true),
        ("a\u{7f}", AskInputMode::Multiline, true),
        ("a\u{1b}[31m", AskInputMode::Line, true),
    ];
    for (text, mode, forbidden) in cases {
        assert_eq!(
            AskInputForm::has_forbidden_controls(text, mode),
            forbidden,
            "{text:?} {mode:?}"
        );
    }
}

/// Upstream's `releasingASessionAskDropsItsDraft`, with the hiding and the resolution around it: the session keeps
/// the draft while the ask waits, and both a release and a resolution drop it.
#[test]
fn a_session_keeps_the_draft_until_release_or_resolution() {
    let mut store = AppStore::new();
    let workspace = store.add_workspace("work");
    let session = add(&mut store, workspace, "/repo");
    let form = AskForm::Input(AskInputForm::new(AskInputMode::Line));
    assert!(store.open_session_ask(session, ask(Some(form.clone()), None), None));
    store.update_session_ask_draft(session, "a", |draft| draft.set_text("typed"));
    store.update_session_ask_draft(session, "stale", |draft| draft.set_text("lost"));
    let draft = |store: &AppStore| {
        store
            .session(session)
            .unwrap()
            .ask()
            .map(|ask| ask.draft.text().to_owned())
    };
    assert_eq!(draft(&store).as_deref(), Some("typed"));
    store.release_session_ask(session);
    assert_eq!(draft(&store), None);
    assert!(store.open_session_ask(session, ask(Some(form), None), None));
    assert_eq!(draft(&store).as_deref(), Some(""), "a new ask starts from its form");
    assert!(store.resolve_session_ask(session, "a", escaped()));
    assert_eq!(draft(&store), None);
}

#[test]
fn anchoring_keeps_the_form_and_cancel_role() {
    let form = list(&["a"], false, &[]);
    let ask = ask(Some(form.clone()), Some("cancel"));
    let anchored = ask.anchored(Some(AskAnchor {
        session: Uuid::new_v4(),
        pane: None,
        pane_identity: None,
    }));
    assert_eq!(anchored.form, Some(form));
    assert_eq!(anchored.cancel_id.as_deref(), Some("cancel"));
    assert!(anchored.anchor.is_some());
}
