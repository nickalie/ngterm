//! The actions' plans, one look at a recorded screen at a time, and the bytes their keys put on the wire: the prompt's
//! bracketed paste and Enter (a slash command the same way), Stop's Escape, each dialog's option keys and the picker's
//! digits, Tab and Enter.

use super::matchers::{Capture, Term};
use crate::api::PaneInputError;
use crate::chat::input::{Action, AgentInputOutcome, Key, Next, Plan, QuestionAnswer, UNSAFE_PASTE};
use crate::chat::matchers::{Agent, Picker, Screen};
use crate::chat::{ChatQuestionItem, ChatQuestionOption};

fn screen(name: &str, agent: Agent, state: &str) -> Screen {
    let capture = Capture::load(name);
    let term = capture.term();
    term.feed(capture.at(state));
    agent.read(&term.frame())
}

/// The plan's move on `screen`, its keys counted as sent as the actor does once they went.
fn step(plan: &mut Plan, screen: &Screen) -> Next {
    let next = plan.next(screen);
    if matches!(next, Next::Send { .. }) {
        plan.mark_sent();
    }
    next
}

fn send(key: Key, look: bool) -> Next {
    Next::Send { keys: vec![key], look }
}

const IDLE: Screen = Screen::Prompt {
    working: false,
    composer: String::new(),
};
const WORKING: Screen = Screen::Prompt {
    working: true,
    composer: String::new(),
};

/// The prompt holding `composer`.
fn holding(composer: &str, working: bool) -> Screen {
    Screen::Prompt {
        working,
        composer: composer.to_owned(),
    }
}

#[test]
fn a_prompt_is_pasted_whole_then_entered_once_the_prompt_shows_it() {
    let dialog = screen("claude-44x36", Agent::Claude, "permission");
    let (lines, chars) = ("x\n".repeat(40), "y".repeat(1200));
    for (text, shown) in [
        ("hello there", "hello there"),
        ("/compact keep it short", "/compact keep it short"),
        ("$greet-check please", "$greet-check please"),
        ("two\nlines", "two\nlines"),
        // wrapped where the screen likes
        ("a long line of text", "a long line\nof text"),
        // a long paste's stand-in, Claude's and Codex's
        (lines.as_str(), "[Pasted text #1 +40 lines]"),
        (chars.as_str(), "[Pasted Content 1200 chars]"),
    ] {
        let mut plan = Plan::new(Action::Prompt(text.to_owned()));
        assert_eq!(step(&mut plan, &IDLE), send(Key::Paste(text.to_owned()), true));
        assert_eq!(
            step(&mut plan, &holding(shown, true)),
            send(Key::Enter, false),
            "{text}"
        );
    }
    // a working agent takes a prompt too (queued by the TUI)
    let mut plan = Plan::new(Action::Prompt("next".to_owned()));
    assert_eq!(step(&mut plan, &WORKING), send(Key::Paste("next".to_owned()), true));

    let mut plan = Plan::new(Action::Prompt("hi".to_owned()));
    assert_eq!(step(&mut plan, &dialog), Next::Done(AgentInputOutcome::DialogOpen));
    let mut plan = Plan::new(Action::Prompt("hi".to_owned()));
    assert_eq!(
        step(&mut plan, &Screen::Unknown),
        Next::Done(AgentInputOutcome::NotOnScreen)
    );

    // a dialog that came up over the pasted text: no Enter, which would answer it
    let mut plan = Plan::new(Action::Prompt("hi".to_owned()));
    step(&mut plan, &IDLE);
    assert_eq!(step(&mut plan, &dialog), Next::Done(AgentInputOutcome::Unconfirmed));
}

#[test]
fn a_draft_in_the_prompt_gets_nothing_and_a_paste_that_did_not_land_gets_no_enter() {
    // a draft left in the prompt at the terminal would go out with the prompt
    for draft in ["draft ", "/comp"] {
        for working in [false, true] {
            let mut plan = Plan::new(Action::Prompt("new prompt".to_owned()));
            assert_eq!(
                step(&mut plan, &holding(draft, working)),
                Next::Done(AgentInputOutcome::NotOnScreen),
                "{draft:?}"
            );
            assert!(!plan.sent());
        }
    }
    // after the paste: dropped, landed after something else, cut short, or a stand-in next to other text
    for shown in [
        "",
        "draft new prompt",
        "new",
        "[Pasted text #1 +40 lines] more",
        "draft [Pasted text #1 +40 lines]",
    ] {
        let mut plan = Plan::new(Action::Prompt("new prompt".to_owned()));
        step(&mut plan, &IDLE);
        assert_eq!(
            step(&mut plan, &holding(shown, false)),
            Next::Done(AgentInputOutcome::Unconfirmed),
            "{shown:?}"
        );
    }
}

#[test]
fn stop_is_escape_and_only_while_a_turn_runs() {
    let mut plan = Plan::new(Action::Interrupt);
    assert_eq!(step(&mut plan, &WORKING), send(Key::Escape, false));
    assert_eq!(
        Plan::new(Action::Interrupt).next(&IDLE),
        Next::Done(AgentInputOutcome::NotOnScreen)
    );
    let dialog = screen("codex-44x36", Agent::Codex, "permission");
    assert_eq!(
        Plan::new(Action::Interrupt).next(&dialog),
        Next::Done(AgentInputOutcome::DialogOpen)
    );
}

#[test]
fn an_approval_sends_its_options_key_and_looks_that_the_dialog_went() {
    let claude = screen("claude-100x40", Agent::Claude, "permission");
    for (label, key) in [("Yes", Key::Digit(1)), ("No", Key::Digit(4))] {
        let mut plan = Plan::new(Action::Approval(label.to_owned()));
        assert_eq!(step(&mut plan, &claude), send(key, true), "{label}");
        assert_eq!(step(&mut plan, &WORKING), Next::Done(AgentInputOutcome::Sent));
    }
    let codex = screen("codex-160x48", Agent::Codex, "permission");
    for (label, key) in [
        ("Yes, proceed", Key::Letter('y')),
        (
            "Yes, and don't ask again for commands that start with `curl -sI https://example.com`",
            Key::Letter('p'),
        ),
        ("No, and tell Codex what to do differently", Key::Escape),
    ] {
        let mut plan = Plan::new(Action::Approval(label.to_owned()));
        assert_eq!(step(&mut plan, &codex), send(key, true), "{label}");
    }

    // the same dialog still up after the key
    let mut plan = Plan::new(Action::Approval("Yes".to_owned()));
    step(&mut plan, &claude);
    assert_eq!(step(&mut plan, &claude), Next::Done(AgentInputOutcome::Unconfirmed));
    // no such option, or no dialog: nothing goes
    for screen in [&claude, &IDLE, &Screen::Unknown] {
        let mut plan = Plan::new(Action::Approval("Maybe".to_owned()));
        assert_eq!(step(&mut plan, screen), Next::Done(AgentInputOutcome::NotOnScreen));
    }
}

fn item(question: &str, multi: bool, options: &[(&str, &str)]) -> ChatQuestionItem {
    ChatQuestionItem {
        question: question.to_owned(),
        header: None,
        multi_select: multi,
        options: options
            .iter()
            .map(|(label, description)| ChatQuestionOption {
                label: (*label).to_owned(),
                description: Some((*description).to_owned()),
            })
            .collect(),
    }
}

fn answer(picked: &[&str], other: Option<&str>) -> QuestionAnswer {
    QuestionAnswer {
        picked: picked.iter().map(|label| (*label).to_owned()).collect(),
        other: other.map(str::to_owned),
    }
}

fn color() -> ChatQuestionItem {
    item("Pick a color", false, &[("Red", "Red"), ("Green", "Green")])
}

fn style() -> ChatQuestionItem {
    item(
        "Which greeting style?",
        false,
        &[
            ("Formal", "Polite, professional greeting"),
            ("Casual", "Relaxed, friendly greeting"),
            ("Pirate", "Arr, matey-style greeting"),
        ],
    )
}

fn languages() -> ChatQuestionItem {
    item(
        "Which languages should greet() support?",
        true,
        &[
            ("Spanish", "Support Spanish greetings"),
            ("French", "Support French greetings"),
            ("German", "Support German greetings"),
        ],
    )
}

/// How text typed into the picker's "Type something." row shows.
#[derive(Clone, Copy)]
enum Typed<'a> {
    /// After the placeholder.
    After(&'a str),
    /// In its place.
    Instead(&'a str),
}

/// `screen`'s picker with its cursor on option `cursor` and the boxes `checked`, and `typed` in its last option.
fn varied(screen: &Screen, cursor: Option<u8>, checked: &[bool], typed: Option<Typed<'_>>) -> Screen {
    let Screen::Question(picker) = screen else {
        panic!("not the picker")
    };
    let mut picker: Picker = picker.clone();
    let last = picker.options.len() - 1;
    for (at, option) in picker.options.iter_mut().enumerate() {
        if let Some(cursor) = cursor {
            option.cursor = option.number == cursor;
        }
        if let Some(checked) = checked.get(at) {
            option.checked = Some(*checked);
        }
        match (typed, at == last) {
            (Some(Typed::After(text)), true) => option.text = format!("{} {text}", option.text),
            (Some(Typed::Instead(text)), true) => option.text = text.to_owned(),
            _ => {}
        }
    }
    Screen::Question(picker)
}

#[test]
fn a_lone_question_is_answered_by_its_digit_or_by_typed_text() {
    let picker = screen("claude-44x36", Agent::Claude, "question");
    let mut plan = Plan::new(Action::Question(vec![color()], vec![answer(&["Green"], None)]));
    assert_eq!(step(&mut plan, &picker), send(Key::Digit(2), true));
    assert_eq!(step(&mut plan, &WORKING), Next::Done(AgentInputOutcome::Sent));

    let mut plan = Plan::new(Action::Question(vec![color()], vec![answer(&[], Some("Blue"))]));
    assert_eq!(step(&mut plan, &picker), send(Key::Digit(3), true), "Type something.");
    let focused = varied(&picker, Some(3), &[], None);
    assert_eq!(step(&mut plan, &focused), send(Key::Paste("Blue".to_owned()), true));
    let typed = varied(&picker, Some(3), &[], Some(Typed::After("Blue")));
    assert_eq!(step(&mut plan, &typed), send(Key::Enter, true));
    assert_eq!(step(&mut plan, &IDLE), Next::Done(AgentInputOutcome::Sent));
    // the text did not show in the row: no Enter
    let mut plan = Plan::new(Action::Question(vec![color()], vec![answer(&[], Some("Blue"))]));
    step(&mut plan, &picker);
    step(&mut plan, &focused);
    assert_eq!(step(&mut plan, &focused), Next::Done(AgentInputOutcome::Unconfirmed));
    // the row was not focused: no paste
    let mut plan = Plan::new(Action::Question(vec![color()], vec![answer(&[], Some("Blue"))]));
    step(&mut plan, &picker);
    assert_eq!(step(&mut plan, &picker), Next::Done(AgentInputOutcome::Unconfirmed));
}

#[test]
fn two_questions_go_tab_by_tab_toggling_boxes_then_submit_on_the_review() {
    let first = screen("claude-160x48", Agent::Claude, "question");
    let multi = screen("claude-160x48", Agent::Claude, "question_multi");
    let review = screen("claude-160x48", Agent::Claude, "question_submit");
    let mut plan = Plan::new(Action::Question(
        vec![style(), languages()],
        vec![answer(&["Casual"], None), answer(&["Spanish", "German"], None)],
    ));
    assert_eq!(step(&mut plan, &first), send(Key::Digit(2), true));
    assert_eq!(step(&mut plan, &multi), send(Key::Digit(1), true));
    let spanish = varied(&multi, None, &[true, false, false], None);
    assert_eq!(step(&mut plan, &spanish), send(Key::Digit(3), true));
    let both = varied(&multi, None, &[true, false, true], None);
    assert_eq!(step(&mut plan, &both), send(Key::Tab, true));
    assert_eq!(step(&mut plan, &review), send(Key::Enter, true));
    assert_eq!(step(&mut plan, &WORKING), Next::Done(AgentInputOutcome::Sent));

    // a box that does not tick is not toggled again
    let mut plan = Plan::new(Action::Question(vec![languages()], vec![answer(&["French"], None)]));
    assert_eq!(step(&mut plan, &multi), send(Key::Digit(2), true));
    assert_eq!(step(&mut plan, &multi), Next::Done(AgentInputOutcome::Unconfirmed));
    // the review still up after its Enter
    let mut plan = Plan::new(Action::Question(vec![languages()], vec![answer(&[], None)]));
    assert_eq!(step(&mut plan, &multi), send(Key::Tab, true));
    step(&mut plan, &review);
    assert_eq!(step(&mut plan, &review), Next::Done(AgentInputOutcome::Unconfirmed));
}

#[test]
fn typed_text_may_follow_the_placeholder_or_take_its_place_and_a_draft_there_gets_nothing() {
    let picker = screen("claude-44x36", Agent::Claude, "question");
    let focused = varied(&picker, Some(3), &[], None);
    // the text in the placeholder's place, or a long paste's stand-in there
    for typed in [Typed::Instead("Blue"), Typed::Instead("[Pasted text #1 +3 lines]")] {
        let mut plan = Plan::new(Action::Question(vec![color()], vec![answer(&[], Some("Blue"))]));
        step(&mut plan, &picker);
        step(&mut plan, &focused);
        let typed = varied(&picker, Some(3), &[], Some(typed));
        assert_eq!(step(&mut plan, &typed), send(Key::Enter, true));
        assert_eq!(step(&mut plan, &IDLE), Next::Done(AgentInputOutcome::Sent));
    }
    // something else in the row after the paste: no Enter
    let mut plan = Plan::new(Action::Question(vec![color()], vec![answer(&[], Some("Blue"))]));
    step(&mut plan, &picker);
    step(&mut plan, &focused);
    assert_eq!(
        step(&mut plan, &varied(&picker, Some(3), &[], Some(Typed::Instead("Bl")))),
        Next::Done(AgentInputOutcome::Unconfirmed)
    );
    // a draft typed into the row at the terminal, after the placeholder or in its place: nothing goes
    for draft in [Typed::After("Gre"), Typed::Instead("Gre")] {
        for answered in [answer(&["Red"], None), answer(&[], Some("Blue"))] {
            let mut plan = Plan::new(Action::Question(vec![color()], vec![answered]));
            assert_eq!(
                step(&mut plan, &varied(&picker, None, &[], Some(draft))),
                Next::Done(AgentInputOutcome::NotOnScreen)
            );
        }
    }
}

#[test]
fn free_text_in_a_multi_select_question_goes_through_its_row_then_tab() {
    let multi = screen("claude-160x48", Agent::Claude, "question_multi");
    let review = screen("claude-160x48", Agent::Claude, "question_submit");
    // its digit, the paste, then Tab, whether the text followed the placeholder or took its place
    for typed in [Typed::After("Latin"), Typed::Instead("Latin")] {
        let mut plan = Plan::new(Action::Question(vec![languages()], vec![answer(&[], Some("Latin"))]));
        assert_eq!(step(&mut plan, &multi), send(Key::Digit(4), true));
        let focused = varied(&multi, Some(4), &[], None);
        assert_eq!(step(&mut plan, &focused), send(Key::Paste("Latin".to_owned()), true));
        let shown = varied(&multi, Some(4), &[], Some(typed));
        assert_eq!(step(&mut plan, &shown), send(Key::Tab, true));
        assert_eq!(step(&mut plan, &review), send(Key::Enter, true));
    }
}

#[test]
fn a_picker_at_another_question_gets_nothing() {
    let picker = screen("claude-160x48", Agent::Claude, "question");
    let other = item(
        "Which greeting style?",
        false,
        &[("Formal", "Polite"), ("Casual", "Relaxed")],
    );
    let cases = [
        (color(), "another question"),
        (languages(), "another tab"),
        (other, "other options"),
    ];
    for (asked, why) in cases {
        let mut plan = Plan::new(Action::Question(vec![asked], vec![answer(&[], Some("x"))]));
        assert_eq!(
            step(&mut plan, &picker),
            Next::Done(AgentInputOutcome::NotOnScreen),
            "{why}"
        );
    }
    let permission = screen("claude-160x48", Agent::Claude, "permission");
    let mut plan = Plan::new(Action::Question(vec![style()], vec![answer(&["Formal"], None)]));
    assert_eq!(step(&mut plan, &permission), Next::Done(AgentInputOutcome::NotOnScreen));
}

#[test]
fn answers_that_do_not_fit_their_questions_are_refused_before_anything_runs() {
    let refused = |items: Vec<ChatQuestionItem>, answers: Vec<QuestionAnswer>| {
        matches!(
            Action::Question(items, answers).check(),
            Err(PaneInputError::Refused { .. })
        )
    };
    assert!(refused(vec![color()], vec![]), "an answer per question");
    assert!(
        refused(vec![color()], vec![answer(&["Blue"], None)]),
        "a label it offers"
    );
    assert!(
        refused(vec![color()], vec![answer(&["Red", "Green"], None)]),
        "one pick"
    );
    assert!(
        refused(vec![color()], vec![answer(&["Red"], Some("x"))]),
        "a pick or text"
    );
    assert!(refused(vec![color()], vec![answer(&[], Some("  "))]), "text with words");
    assert!(!refused(
        vec![languages()],
        vec![answer(&["Spanish", "German"], Some("Latin"))]
    ));
    assert!(matches!(
        Action::Prompt(" \n".to_owned()).check(),
        Err(PaneInputError::Refused { .. })
    ));
}

/// What `keys` put on the wire from a terminal that took `setup`.
fn wire(setup: &[u8], keys: &[Key]) -> Vec<u8> {
    let term = Term::new(80, 24);
    term.feed(setup);
    term.sent();
    for key in keys {
        key.send(&term.pane).unwrap();
    }
    term.sent()
}

#[test]
fn the_keys_go_out_encoded_for_the_programs_modes() {
    const BRACKETED: &[u8] = b"\x1b[?2004h";
    assert_eq!(
        wire(
            BRACKETED,
            &[Key::Paste("/compact keep it short".to_owned()), Key::Enter]
        ),
        b"\x1b[200~/compact keep it short\x1b[201~\r",
        "a slash command is pasted whole, then Enter"
    );
    assert_eq!(
        wire(BRACKETED, &[Key::Paste("two\nlines".to_owned())]),
        b"\x1b[200~two\nlines\x1b[201~"
    );
    assert_eq!(wire(b"", &[Key::Escape]), b"\x1b");
    assert_eq!(wire(b"", &[Key::Digit(3), Key::Letter('y'), Key::Tab]), b"3y\t");
    // under the Kitty keyboard protocol Codex pushes, Escape is unambiguous
    assert_eq!(wire(b"\x1b[>7u", &[Key::Escape]), b"\x1b[27u");

    // a paste the terminal can't frame safely is refused whole
    let term = Term::new(80, 24);
    assert_eq!(
        Key::Paste("rm -rf x\n".to_owned()).send(&term.pane),
        Err(PaneInputError::Refused {
            reason: UNSAFE_PASTE.to_owned()
        })
    );
    assert!(term.sent().is_empty());
}

#[test]
fn both_agents_screens_take_a_bracketed_paste() {
    for (name, state) in [("claude-44x36", "idle"), ("codex-44x36", "idle")] {
        let capture = Capture::load(name);
        let term = capture.term();
        term.feed(capture.at(state));
        term.sent();
        Key::Paste("hi\nthere".to_owned()).send(&term.pane).unwrap();
        assert_eq!(term.sent(), b"\x1b[200~hi\nthere\x1b[201~", "{name}");
    }
}
