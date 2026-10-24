//! `ask.open` forms (upstream's `ControlDispatcherAskTests` form cases and `AskTests.slotOwnersKeepTheDraft…`): the
//! text field and the choice list validated in upstream's order with its full strings, the generated buttons, and the
//! draft each slot owner keeps until the ask resolves.

use ngterm_proto::command::AskCmd;
use ngterm_proto::{Args, AskButton, AskChoice, Command, Response};
use serde_json::json;
use uuid::Uuid;

use super::{Fixture, args, wire};
use crate::model::{AskChoiceForm, AskForm, AskInputForm, AskInputMode, PendingAsk};

fn button(id: &str, label: &str, hotkey: Option<&str>) -> AskButton {
    AskButton {
        id: id.to_owned(),
        label: label.to_owned(),
        hotkey: hotkey.map(str::to_owned),
    }
}

fn choice(id: &str, label: &str, subtitle: Option<&str>) -> AskChoice {
    AskChoice {
        id: id.to_owned(),
        label: label.to_owned(),
        subtitle: subtitle.map(str::to_owned),
    }
}

/// A titled ask with no buttons, `edit` applied.
fn titled(edit: impl FnOnce(&mut Args)) -> Args {
    args(|a| {
        a.title = Some("T".to_owned());
        edit(a);
    })
}

fn send_open(fixture: &mut Fixture, session: Uuid, args: Args) -> Response {
    fixture.send(Command::Ask(AskCmd::Open), Some(&wire(session)), args)
}

/// The terminal ask `args` opens on the fixture's session.
fn opened(args: Args) -> PendingAsk {
    let mut fixture = Fixture::new(0);
    let session = fixture.sessions[0];
    let response = send_open(&mut fixture, session, args);
    assert!(response.ok, "{response:?}");
    fixture.store().session(session).unwrap().ask().unwrap().ask.clone()
}

/// The refusal `args` meets, after checking nothing reached the session's slot.
fn rejection(args: Args) -> String {
    let mut fixture = Fixture::new(0);
    let session = fixture.sessions[0];
    let response = send_open(&mut fixture, session, args);
    assert!(
        fixture.store().session(session).unwrap().ask().is_none(),
        "{response:?}"
    );
    assert!(!response.ok, "{response:?}");
    response.error.unwrap_or_default()
}

fn ok_cancel() -> Vec<AskButton> {
    vec![button("ok", "OK", None), button("cancel", "Cancel", None)]
}

/// Upstream's `formWithoutButtonsGetsOKAndCancelWithOKDefault`.
#[test]
fn a_form_without_buttons_gets_ok_and_cancel_with_ok_default() {
    let ask = opened(titled(|a| a.input = Some("line".into())));
    assert_eq!(ask.buttons, ok_cancel());
    assert_eq!(ask.default_id.as_deref(), Some("ok"));
    assert_eq!(ask.cancel_id.as_deref(), Some("cancel"));
    assert_eq!(ask.form, Some(AskForm::Input(AskInputForm::new(AskInputMode::Line))));
}

/// Upstream's `callerButtonsReplaceTheDefaultsAndCancelMayBeTheDefault`.
#[test]
fn caller_buttons_replace_the_defaults_and_cancel_may_be_the_default() {
    let ask = opened(titled(|a| {
        a.buttons = Some(vec![button("save", "Save", None), button("skip", "Skip", None)]);
        a.default_button = Some("skip".into());
        a.value = Some("x".into());
        a.cancel_button = Some("skip".into());
        a.input = Some("multiline".into());
    }));
    let ids: Vec<&str> = ask.buttons.iter().map(|button| button.id.as_str()).collect();
    assert_eq!(ids, ["save", "skip"]);
    assert_eq!(ask.default_id.as_deref(), Some("skip"));
    assert_eq!(ask.cancel_id.as_deref(), Some("skip"));
    assert_eq!(
        ask.form,
        Some(AskForm::Input(AskInputForm {
            value: "x".into(),
            ..AskInputForm::new(AskInputMode::Multiline)
        }))
    );
}

/// Upstream's `plainAskStillRequiresButtonsAndHasNoCancelRole`.
#[test]
fn a_plain_ask_still_requires_buttons_and_has_no_cancel_role() {
    assert_eq!(rejection(titled(|_| {})), "ask.open requires buttons");
    let ask = opened(titled(|a| a.buttons = Some(vec![button("ok", "OK", None)])));
    assert_eq!(ask.form, None);
    assert_eq!(ask.cancel_id, None);
    assert_eq!(ask.default_id, None);
}

/// Upstream's `choicesReachHostWithSubtitlesAndCheckedIDs`.
#[test]
fn choices_reach_the_slot_with_subtitles_and_checked_ids() {
    let choices = vec![
        choice("398", "issue #398", Some("waits on you\nhttps://example.com/398")),
        choice("395", "PR #395", None),
    ];
    let ask = opened(titled(|a| {
        a.choices = Some(choices.clone());
        a.multiple = Some(true);
        a.checked = Some(vec!["395".into()]);
    }));
    assert_eq!(
        ask.form,
        Some(AskForm::Choices(AskChoiceForm {
            multiple: true,
            checked: vec!["395".into()],
            ..AskChoiceForm::new(choices)
        }))
    );
}

/// Upstream's `otherPlaceholderReachesHostWithTheChoices`.
#[test]
fn the_other_placeholder_reaches_the_slot_with_the_choices() {
    let choices = vec![choice("a", "A", None)];
    let ask = opened(titled(|a| {
        a.choices = Some(choices.clone());
        a.other = Some("Something else".into());
    }));
    assert_eq!(
        ask.form,
        Some(AskForm::Choices(AskChoiceForm {
            other: Some("Something else".into()),
            ..AskChoiceForm::new(choices)
        }))
    );
}

/// Upstream's `invalidOtherNeverReachesHost`.
#[test]
fn an_invalid_other_never_reaches_the_slot() {
    let one = || Some(vec![choice("a", "A", None)]);
    let cases: Vec<(Args, &str)> = vec![
        (
            titled(|a| {
                a.input = Some("line".into());
                a.other = Some("x".into());
            }),
            "other requires choices",
        ),
        (
            titled(|a| {
                a.buttons = Some(vec![button("ok", "OK", None)]);
                a.other = Some("x".into());
            }),
            "other requires choices",
        ),
        (
            titled(|a| {
                a.choices = one();
                a.other = Some("  ".into());
            }),
            "ask other placeholder must not be blank",
        ),
        (
            titled(|a| {
                a.choices = one();
                a.other = Some("a\tb".into());
            }),
            "ask text must not contain control characters",
        ),
    ];
    for (args, error) in cases {
        assert_eq!(rejection(args.clone()), error, "{args:?}");
    }
}

/// Upstream's `invalidFormNeverReachesHost`, the form's arguments and the text field.
#[test]
fn an_invalid_form_never_reaches_the_slot() {
    let a = || choice("a", "A", None);
    let ok = || Some(vec![button("ok", "OK", None)]);
    let cases: Vec<(Args, &str)> = vec![
        (
            titled(|x| {
                x.input = Some("line".into());
                x.choices = Some(vec![a()]);
            }),
            "input and choices are exclusive",
        ),
        (titled(|x| x.input = Some("wide".into())), "unknown input"),
        (
            titled(|x| {
                x.buttons = ok();
                x.value = Some("x".into());
            }),
            "value requires input",
        ),
        (
            titled(|x| {
                x.buttons = ok();
                x.placeholder = Some("x".into());
            }),
            "placeholder requires input",
        ),
        (
            titled(|x| {
                x.input = Some("line".into());
                x.checked = Some(vec!["a".into()]);
            }),
            "checked requires choices",
        ),
        (
            titled(|x| {
                x.input = Some("line".into());
                x.multiple = Some(true);
            }),
            "multiple requires choices",
        ),
    ];
    for (args, error) in cases {
        assert_eq!(rejection(args.clone()), error, "{args:?}");
    }
}

/// Upstream's `invalidFormNeverReachesHost`, the choice list and the button roles.
#[test]
fn an_invalid_choice_list_or_role_never_reaches_the_slot() {
    let a = || choice("a", "A", None);
    let cases: Vec<(Args, &str)> = vec![
        (titled(|x| x.choices = Some(Vec::new())), "ask choices must be 1 to 50"),
        (
            titled(|x| x.choices = Some(vec![choice("a", "", None)])),
            "ask choice label must not be empty",
        ),
        (
            titled(|x| x.choices = Some(vec![a(), choice("a", "B", None)])),
            "ask choice ids must be unique",
        ),
        (
            titled(|x| {
                x.choices = Some(vec![a()]);
                x.checked = Some(vec!["b".into()]);
            }),
            "unknown checked choice: b",
        ),
        (
            titled(|x| {
                x.choices = Some(vec![a()]);
                x.multiple = Some(true);
                x.checked = Some(vec!["a".into(), "a".into()]);
            }),
            "ask checked ids must be unique",
        ),
        (
            titled(|x| {
                x.choices = Some(vec![a(), choice("b", "B", None)]);
                x.checked = Some(vec!["a".into(), "b".into()]);
            }),
            "radio choices accept one checked id",
        ),
        (
            titled(|x| {
                x.cancel_button = Some("nope".into());
                x.input = Some("line".into());
            }),
            "unknown cancel button: nope",
        ),
        (
            titled(|x| {
                x.buttons = Some(vec![button("ok", "OK", Some("o"))]);
                x.input = Some("line".into());
            }),
            "ask input does not take hotkeys",
        ),
    ];
    for (args, error) in cases {
        assert_eq!(rejection(args.clone()), error, "{args:?}");
    }
}

/// The form is checked before the buttons, and an input's hotkey refusal comes before the hotkey's own checks.
#[test]
fn the_form_is_checked_before_the_buttons() {
    assert_eq!(
        rejection(titled(|a| {
            a.buttons = Some(Vec::new());
            a.input = Some("wide".into());
        })),
        "unknown input"
    );
    assert_eq!(
        rejection(titled(|a| {
            a.buttons = Some(vec![button("ok", "OK", Some("1"))]);
            a.input = Some("line".into());
        })),
        "ask input does not take hotkeys"
    );
    // a choice list keeps its hotkeys
    let ask = opened(titled(|a| {
        a.buttons = Some(vec![button("ok", "OK", Some("O"))]);
        a.choices = Some(vec![choice("a", "A", None)]);
    }));
    assert_eq!(ask.buttons[0].hotkey.as_deref(), Some("o"));
    assert_eq!(ask.cancel_id, None, "only the generated pair cancels by default");
}

/// Upstream's `controlCharactersOutsideTheirAllowedFieldsAreRejected`.
#[test]
fn control_characters_outside_their_allowed_fields_are_rejected() {
    let cases = [
        titled(|a| {
            a.value = Some("a\tb".into());
            a.input = Some("line".into());
        }),
        titled(|a| {
            a.value = Some("a\nb".into());
            a.input = Some("line".into());
        }),
        titled(|a| {
            a.value = Some("a\u{7}b".into());
            a.input = Some("multiline".into());
        }),
        titled(|a| {
            a.input = Some("multiline".into());
            a.placeholder = Some("a\nb".into());
        }),
        titled(|a| a.choices = Some(vec![choice("a", "A\nB", None)])),
        titled(|a| a.choices = Some(vec![choice("a", "A", Some("x\ty"))])),
    ];
    for args in cases {
        assert_eq!(
            rejection(args.clone()),
            "ask text must not contain control characters",
            "{args:?}"
        );
    }
}

/// Upstream's `multilineValueAndSubtitleKeepTheirAllowedControls`.
#[test]
fn a_multiline_value_and_a_subtitle_keep_their_allowed_controls() {
    let input = opened(titled(|a| {
        a.value = Some("a\nb\tc".into());
        a.input = Some("multiline".into());
    }));
    assert_eq!(
        input.form,
        Some(AskForm::Input(AskInputForm {
            value: "a\nb\tc".into(),
            ..AskInputForm::new(AskInputMode::Multiline)
        }))
    );
    let list = opened(titled(|a| a.choices = Some(vec![choice("a", "A", Some("x\ny"))])));
    assert!(list.form.is_some());
}

/// Upstream's `valueIsCappedInUTF8Bytes`.
#[test]
fn the_value_is_capped_in_utf8_bytes() {
    let fits = "é".repeat(AskInputForm::MAX_TEXT_BYTES / 2);
    let input = |value: String| {
        titled(|a| {
            a.value = Some(value);
            a.input = Some("line".into());
        })
    };
    opened(input(fits.clone()));
    assert_eq!(rejection(input(fits + "x")), "ask text too long (max 65536 bytes)");
}

/// Upstream's `fiftyChoicesAreAcceptedAndFiftyOneRejected`.
#[test]
fn fifty_choices_are_accepted_and_fifty_one_rejected() {
    let fifty: Vec<AskChoice> = (0..50)
        .map(|n| choice(&n.to_string(), &format!("item {n}"), None))
        .collect();
    opened(titled(|a| a.choices = Some(fifty.clone())));
    let mut fifty_one = fifty;
    fifty_one.push(choice("x", "x", None));
    assert_eq!(
        rejection(titled(|a| a.choices = Some(fifty_one))),
        "ask choices must be 1 to 50"
    );
}

/// Upstream's `AskTests.slotOwnersKeepTheDraftUntilResolutionAndIgnoreStaleIDs`, terminal style: the session keeps
/// the draft, a stale id edits nothing, and the answer carries the draft.
#[test]
fn the_session_keeps_a_terminal_asks_draft_until_it_resolves() {
    let mut fixture = Fixture::new(0);
    let session = fixture.sessions[0];
    let response = send_open(
        &mut fixture,
        session,
        titled(|a| {
            a.input = Some("line".into());
            a.value = Some("x".into());
        }),
    );
    let id = response.result.as_ref().unwrap()["id"].as_str().unwrap().to_owned();
    let draft_text = |fixture: &Fixture| {
        fixture
            .store()
            .session(session)
            .unwrap()
            .ask()
            .map(|pending| pending.draft.text().to_owned())
    };
    assert_eq!(draft_text(&fixture).as_deref(), Some("x"));
    fixture
        .core
        .update_session_ask_draft(session, &id, |draft| draft.set_text("typed"));
    fixture
        .core
        .update_session_ask_draft(session, "stale", |draft| draft.set_text("lost"));
    assert_eq!(draft_text(&fixture).as_deref(), Some("typed"));
    fixture.core.answer_session_ask_button(session, "stale", 0);
    fixture.core.answer_session_ask_button(session, &id, 9);
    assert_eq!(draft_text(&fixture).as_deref(), Some("typed"), "nothing answered");
    fixture.core.answer_session_ask_button(session, &id, 0);
    assert_eq!(draft_text(&fixture), None);
    let result = fixture.send(Command::Ask(AskCmd::Result), Some(&id), Args::default());
    assert_eq!(
        result.result.unwrap()["ask"],
        json!({"result": "answered", "id": "ok", "label": "OK", "index": 0, "text": "typed"})
    );
}

/// Opens a GUI checkbox ask over `a` and `b` and checks `b` in its window slot's draft; returns the ask's id and the
/// window.
fn open_gui_ask_and_check_b(fixture: &mut Fixture) -> (String, Uuid) {
    let window = fixture.window;
    let response = fixture.send(
        Command::Ask(AskCmd::Open),
        None,
        titled(|a| {
            a.style = Some("gui".into());
            a.choices = Some(vec![choice("a", "A", None), choice("b", "B", None)]);
            a.multiple = Some(true);
        }),
    );
    let id = response.result.as_ref().unwrap()["id"].as_str().unwrap().to_owned();
    assert_eq!(window_selected(fixture, window), Some(Vec::new()));
    fixture
        .core
        .update_window_ask_draft(window, &id, |draft| draft.toggle(1));
    fixture
        .core
        .update_window_ask_draft(window, "stale", |draft| draft.toggle(0));
    assert_eq!(window_selected(fixture, window), Some(vec!["b".to_owned()]));
    (id, window)
}

/// The ids checked in window `window`'s ask draft, `None` without a pending ask there.
fn window_selected(fixture: &Fixture, window: Uuid) -> Option<Vec<String>> {
    fixture
        .core
        .modal_slot(window)
        .and_then(|slot| slot.ask_draft())
        .map(crate::model::AskDraft::selected)
}

/// The same for a GUI ask, whose window slot keeps the draft; its OK button answers with the draft's checks.
#[test]
fn the_window_slot_keeps_a_gui_asks_draft_until_it_resolves() {
    let mut fixture = Fixture::new(0);
    let (id, window) = open_gui_ask_and_check_b(&mut fixture);
    fixture.core.answer_window_ask_button(window, "stale", 0);
    assert!(
        window_selected(&fixture, window).is_some(),
        "a stale id answers nothing"
    );
    fixture.core.answer_window_ask_button(window, &id, 9);
    assert!(
        window_selected(&fixture, window).is_some(),
        "an index past the buttons answers nothing"
    );
    fixture.core.answer_window_ask_button(window, &id, 0);
    assert_eq!(window_selected(&fixture, window), None);
    let result = fixture.send(Command::Ask(AskCmd::Result), Some(&id), Args::default());
    assert_eq!(
        result.result.unwrap()["ask"],
        json!({"result": "answered", "id": "ok", "label": "OK", "index": 0, "selected": ["b"]})
    );
}

/// A GUI ask's cancel button escapes without the draft's checks.
#[test]
fn a_gui_asks_cancel_button_escapes_without_the_draft() {
    let mut fixture = Fixture::new(0);
    let (id, window) = open_gui_ask_and_check_b(&mut fixture);
    fixture.core.answer_window_ask_button(window, &id, 1);
    assert_eq!(window_selected(&fixture, window), None);
    let result = fixture.send(Command::Ask(AskCmd::Result), Some(&id), Args::default());
    assert_eq!(result.result.unwrap()["ask"], json!({"result": "escaped"}));
}
