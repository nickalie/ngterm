//! The actions' plans, one look at a recorded screen at a time, and the bytes their keys put on the wire: the prompt's
//! bracketed paste and Enter (a slash command the same way), Stop's Escape, each dialog's option keys and the picker's
//! digits, Tab and Enter.

use super::matchers::{
    CODEX_QUESTIONS, Capture, LIMIT_OPTIONS, LIMIT_QUESTION, PROJECT_DIR, Term, approval_of, asked, claude_file_dialog,
    file_approval, limit_picker, marked_drafts, tricky_drafts, with_draft,
};
use crate::api::PaneInputError;
use crate::chat::input::{Action, AgentInputOutcome, Key, Next, Plan, QuestionAnswer, UNSAFE_PASTE, shown_subjects};
use crate::chat::matchers::{Agent, Picker, Screen};
use crate::chat::{ChatApproval, ChatQuestionItem, ChatQuestionOption};

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
        let mut plan = Plan::new(Action::Prompt(text.to_owned()), Agent::Claude);
        assert_eq!(step(&mut plan, &IDLE), send(Key::Paste(text.to_owned()), true));
        assert_eq!(
            step(&mut plan, &holding(shown, true)),
            send(Key::Enter, false),
            "{text}"
        );
    }
    // a working agent takes a prompt too (queued by the TUI)
    let mut plan = Plan::new(Action::Prompt("next".to_owned()), Agent::Claude);
    assert_eq!(step(&mut plan, &WORKING), send(Key::Paste("next".to_owned()), true));

    let mut plan = Plan::new(Action::Prompt("hi".to_owned()), Agent::Claude);
    assert_eq!(step(&mut plan, &dialog), Next::Done(AgentInputOutcome::DialogOpen));
    let mut plan = Plan::new(Action::Prompt("hi".to_owned()), Agent::Claude);
    assert_eq!(
        step(&mut plan, &Screen::Unknown),
        Next::Done(AgentInputOutcome::NotOnScreen)
    );

    // a dialog that came up over the pasted text: no Enter, which would answer it (the actor ends the action
    // Unconfirmed once the confirm deadline passes)
    let mut plan = Plan::new(Action::Prompt("hi".to_owned()), Agent::Claude);
    step(&mut plan, &IDLE);
    assert_eq!(step(&mut plan, &dialog), Next::Wait);
    assert_eq!(
        plan.stopped(AgentInputOutcome::NotOnScreen),
        AgentInputOutcome::Unconfirmed
    );
}

#[test]
fn a_paste_with_blank_lines_option_shaped_lines_or_long_lines_is_entered_once_shown() {
    for (agent, name) in [
        (Agent::Claude, "claude-44x36"),
        (Agent::Claude, "claude-160x48"),
        (Agent::Codex, "codex-44x36"),
        (Agent::Codex, "codex-160x48"),
    ] {
        let capture = Capture::load(name);
        // a paste quoting a dialog (its words, key hints, option rows) is the prompt's text, and is entered too
        for draft in tricky_drafts().into_iter().chain(marked_drafts()) {
            let mut plan = Plan::new(Action::Prompt(draft.clone()), agent);
            assert_eq!(step(&mut plan, &IDLE), send(Key::Paste(draft.clone()), true));
            let (term, _) = with_draft(&capture, agent, "idle", &draft);
            assert_eq!(
                step(&mut plan, &agent.read(&term.frame())),
                send(Key::Enter, false),
                "{name} {draft:?}"
            );
        }
    }
}

#[test]
fn a_draft_in_the_prompt_gets_nothing_and_a_paste_that_did_not_land_gets_no_enter() {
    // a draft left in the prompt at the terminal would go out with the prompt
    for draft in ["draft ", "/comp"] {
        for working in [false, true] {
            let mut plan = Plan::new(Action::Prompt("new prompt".to_owned()), Agent::Claude);
            assert_eq!(
                step(&mut plan, &holding(draft, working)),
                Next::Done(AgentInputOutcome::NotOnScreen),
                "{draft:?}"
            );
            assert!(!plan.sent());
        }
    }
    // after the paste: not shown yet (or dropped), landed after something else, cut short (or half drawn), or a
    // stand-in next to other text; pending, never an Enter
    for shown in [
        "",
        "draft new prompt",
        "new",
        "[Pasted text #1 +40 lines] more",
        "draft [Pasted text #1 +40 lines]",
    ] {
        let mut plan = Plan::new(Action::Prompt("new prompt".to_owned()), Agent::Claude);
        step(&mut plan, &IDLE);
        assert_eq!(step(&mut plan, &holding(shown, false)), Next::Wait, "{shown:?}");
        // the paste shows at last: the Enter goes, once
        assert_eq!(
            step(&mut plan, &holding("new prompt", false)),
            send(Key::Enter, false),
            "{shown:?}"
        );
    }
}

#[test]
fn stop_is_escape_and_only_while_a_turn_runs() {
    let mut plan = Plan::new(Action::Interrupt, Agent::Claude);
    assert_eq!(step(&mut plan, &WORKING), send(Key::Escape, false));
    assert_eq!(
        Plan::new(Action::Interrupt, Agent::Claude).next(&IDLE),
        Next::Done(AgentInputOutcome::NotOnScreen)
    );
    let dialog = screen("codex-44x36", Agent::Codex, "permission");
    assert_eq!(
        Plan::new(Action::Interrupt, Agent::Codex).next(&dialog),
        Next::Done(AgentInputOutcome::DialogOpen)
    );
}

#[test]
fn an_approval_sends_its_options_key_and_looks_that_the_dialog_went() {
    let claude = screen("claude-100x40", Agent::Claude, "permission");
    let card = asked("claude-100x40");
    for (label, key) in [("Yes", Key::Digit(1)), ("No", Key::Digit(4))] {
        let mut plan = Plan::new(Action::Approval(card.clone(), label.to_owned()), Agent::Claude);
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
        let mut plan = Plan::new(Action::Approval(asked("codex-160x48"), label.to_owned()), Agent::Codex);
        assert_eq!(step(&mut plan, &codex), send(key, true), "{label}");
    }

    // the same dialog still up after the key: pending (the key is not sent again), then gone
    let mut plan = Plan::new(Action::Approval(card.clone(), "Yes".to_owned()), Agent::Claude);
    step(&mut plan, &claude);
    assert_eq!(step(&mut plan, &claude), Next::Wait);
    assert_eq!(step(&mut plan, &claude), Next::Wait);
    assert_eq!(step(&mut plan, &IDLE), Next::Done(AgentInputOutcome::Sent));
    // no such option, or no dialog: nothing goes
    for screen in [&claude, &IDLE, &Screen::Unknown] {
        let mut plan = Plan::new(Action::Approval(card.clone(), "Maybe".to_owned()), Agent::Claude);
        assert_eq!(step(&mut plan, screen), Next::Done(AgentInputOutcome::NotOnScreen));
    }
}

#[test]
fn an_approval_goes_only_to_the_dialog_that_asks_about_the_cards_call() {
    // the card answered elsewhere and the agent asking about another call: no label of the card answers the dialog
    // up now, its own or the agent's usual ones (the card's buttons when the screen could not be read)
    for (name, agent) in [
        ("claude-44x36", Agent::Claude),
        ("claude-100x40", Agent::Claude),
        ("claude-160x48", Agent::Claude),
        ("codex-44x36", Agent::Codex),
        ("codex-100x40", Agent::Codex),
        ("codex-160x48", Agent::Codex),
    ] {
        let dialog = screen(name, agent, "permission");
        let Screen::Permission(shown) = &dialog else {
            panic!("{name}: not a dialog");
        };
        let mut labels: Vec<String> = shown.options.iter().map(|option| option.label.clone()).collect();
        labels.extend(agent.fallback_labels());
        let own = asked(name);
        let others = [
            approval_of("rm -rf build"),
            approval_of(if name.ends_with("160x48") {
                "touch picked.txt"
            } else {
                "touch allowed.txt"
            }),
            approval_of(if name.ends_with("160x48") {
                "curl -sI https://example.net"
            } else {
                "curl -sI https://example.com"
            }),
            // a card whose call can't be confirmed on screen: no command, path or URL to look for
            ChatApproval {
                detail: None,
                ..own.clone()
            },
        ];
        for label in &labels {
            for card in &others {
                let mut plan = Plan::new(Action::Approval(card.clone(), label.clone()), agent);
                assert_eq!(
                    step(&mut plan, &dialog),
                    Next::Done(AgentInputOutcome::NotOnScreen),
                    "{name}: {card:?} {label}"
                );
                assert!(!plan.sent(), "{name}: nothing went");
            }
        }
        // the card's own call: its option's key goes
        let mut plan = Plan::new(Action::Approval(own, labels[0].clone()), agent);
        assert!(matches!(step(&mut plan, &dialog), Next::Send { .. }), "{name}");
    }
}

#[test]
fn a_file_tools_approval_goes_to_the_dialog_naming_its_path_relative_to_the_working_directory() {
    let file = format!("{PROJECT_DIR}/src/hello.py");
    // the dialog names `src/hello.py`, as Claude does from the project; or the whole path (its verbose mode)
    for (shown, card) in [
        ("src/hello.py", file_approval("Edit", &file, Some(PROJECT_DIR))),
        (
            "src/hello.py",
            file_approval("Write", &file, Some(&format!("{PROJECT_DIR}/"))),
        ),
        ("src/hello.py", file_approval("MultiEdit", &file, Some(PROJECT_DIR))),
        ("src/hello.py", file_approval("NotebookEdit", &file, Some(PROJECT_DIR))),
        (
            "../src/hello.py",
            file_approval("Edit", &file, Some(&format!("{PROJECT_DIR}/app"))),
        ),
        (file.as_str(), file_approval("Edit", &file, Some(PROJECT_DIR))),
        (file.as_str(), file_approval("Edit", &file, None)),
    ] {
        let dialog = claude_file_dialog("Edit file", shown);
        let mut plan = Plan::new(Action::Approval(card.clone(), "Yes".to_owned()), Agent::Claude);
        assert_eq!(step(&mut plan, &dialog), send(Key::Digit(1), true), "{shown}: {card:?}");
    }
    // a dialog about another file, or one the card can't be told from: nothing goes, and the card is not answered
    let dialog = claude_file_dialog("Edit file", "src/hello.py");
    for card in [
        // the file of the same name a directory up, which a suffix of the path would take for this one
        file_approval("Edit", &format!("{PROJECT_DIR}/hello.py"), Some(PROJECT_DIR)),
        file_approval("Edit", &format!("{PROJECT_DIR}/lib/src/hello.py"), Some(PROJECT_DIR)),
        // the same file asked from another directory: the dialog would name it otherwise
        file_approval("Edit", &file, Some(&format!("{PROJECT_DIR}/src"))),
        file_approval("Edit", &file, Some("/home/user/other")),
        // no working directory: only the whole path confirms it
        file_approval("Edit", &file, None),
        // not a file tool: its detail is matched as it is
        ChatApproval {
            tool: "Read".to_owned(),
            ..file_approval("Edit", &file, Some(PROJECT_DIR))
        },
        // an MCP tool's card names nothing a dialog shows
        ChatApproval {
            tool: "mcp__github__create_issue".to_owned(),
            title: "mcp__github__create_issue".to_owned(),
            detail: None,
            tool_use_id: None,
            cwd: Some(PROJECT_DIR.to_owned()),
            questions: Vec::new(),
        },
    ] {
        let mut plan = Plan::new(Action::Approval(card.clone(), "Yes".to_owned()), Agent::Claude);
        assert_eq!(
            step(&mut plan, &dialog),
            Next::Done(AgentInputOutcome::NotOnScreen),
            "{card:?}"
        );
        assert!(!plan.sent());
    }
}

#[test]
fn a_file_tools_path_is_shown_relative_as_node_makes_it() {
    let subjects = |tool: &str, path: &str, cwd: Option<&str>| shown_subjects(&file_approval(tool, path, cwd));
    let pairs = [
        ("/a/b", "/a/b/c.rs", "c.rs"),
        ("/a/b/", "/a/b/./x/../c.rs", "c.rs"),
        ("/a/b", "/a/c/d.rs", "../c/d.rs"),
        ("/a/b/c", "/x.rs", "../../../x.rs"),
        ("/", "/x/y.rs", "x/y.rs"),
    ];
    for (cwd, path, relative) in pairs {
        assert_eq!(
            subjects("Edit", path, Some(cwd)),
            [relative.to_owned(), path.to_owned()],
            "{cwd} {path}"
        );
    }
    // the directory itself, a path that is not absolute (Windows), no working directory: only the whole path
    for (cwd, path) in [
        (Some("/a/b"), "/a/b"),
        (Some("C:\\work"), "C:\\work\\x.rs"),
        (Some("/a/b"), "C:\\work\\x.rs"),
        (None, "/a/b/c.rs"),
    ] {
        assert_eq!(subjects("Write", path, cwd), [path.to_owned()], "{cwd:?} {path}");
    }
    // a shell call's command is matched as it is, wherever it ran; a card with nothing to match, nothing
    assert_eq!(
        subjects("Bash", "/a/b/run.sh", Some("/a/b")),
        ["/a/b/run.sh".to_owned()]
    );
    assert!(
        shown_subjects(&ChatApproval {
            detail: Some("  ".to_owned()),
            ..approval_of("x")
        })
        .is_empty()
    );
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
    let mut plan = Plan::new(
        Action::Question(vec![color()], vec![answer(&["Green"], None)]),
        Agent::Claude,
    );
    assert_eq!(step(&mut plan, &picker), send(Key::Digit(2), true));
    assert_eq!(step(&mut plan, &WORKING), Next::Done(AgentInputOutcome::Sent));

    let mut plan = Plan::new(
        Action::Question(vec![color()], vec![answer(&[], Some("Blue"))]),
        Agent::Claude,
    );
    assert_eq!(step(&mut plan, &picker), send(Key::Digit(3), true), "Type something.");
    let focused = varied(&picker, Some(3), &[], None);
    assert_eq!(step(&mut plan, &focused), send(Key::Paste("Blue".to_owned()), true));
    let typed = varied(&picker, Some(3), &[], Some(Typed::After("Blue")));
    assert_eq!(step(&mut plan, &typed), send(Key::Enter, true));
    assert_eq!(step(&mut plan, &IDLE), Next::Done(AgentInputOutcome::Sent));
    // the text not shown in the row yet: no Enter until it is
    let mut plan = Plan::new(
        Action::Question(vec![color()], vec![answer(&[], Some("Blue"))]),
        Agent::Claude,
    );
    step(&mut plan, &picker);
    step(&mut plan, &focused);
    assert_eq!(step(&mut plan, &focused), Next::Wait);
    assert_eq!(step(&mut plan, &typed), send(Key::Enter, true));
    // the row not focused yet: no paste until it is
    let mut plan = Plan::new(
        Action::Question(vec![color()], vec![answer(&[], Some("Blue"))]),
        Agent::Claude,
    );
    step(&mut plan, &picker);
    assert_eq!(step(&mut plan, &picker), Next::Wait);
    assert_eq!(step(&mut plan, &focused), send(Key::Paste("Blue".to_owned()), true));
    // the picked option's digit went, the picker still up: pending, the digit not sent again
    let mut plan = Plan::new(
        Action::Question(vec![color()], vec![answer(&["Green"], None)]),
        Agent::Claude,
    );
    step(&mut plan, &picker);
    assert_eq!(step(&mut plan, &picker), Next::Wait);
    assert_eq!(step(&mut plan, &Screen::Unknown), Next::Wait);
    assert_eq!(step(&mut plan, &WORKING), Next::Done(AgentInputOutcome::Sent));
}

#[test]
fn two_questions_go_tab_by_tab_toggling_boxes_then_submit_on_the_review() {
    let first = screen("claude-160x48", Agent::Claude, "question");
    let multi = screen("claude-160x48", Agent::Claude, "question_multi");
    let review = screen("claude-160x48", Agent::Claude, "question_submit");
    let mut plan = Plan::new(
        Action::Question(
            vec![style(), languages()],
            vec![answer(&["Casual"], None), answer(&["Spanish", "German"], None)],
        ),
        Agent::Claude,
    );
    assert_eq!(step(&mut plan, &first), send(Key::Digit(2), true));
    assert_eq!(step(&mut plan, &multi), send(Key::Digit(1), true));
    let spanish = varied(&multi, None, &[true, false, false], None);
    assert_eq!(step(&mut plan, &spanish), send(Key::Digit(3), true));
    let both = varied(&multi, None, &[true, false, true], None);
    assert_eq!(step(&mut plan, &both), send(Key::Tab, true));
    assert_eq!(step(&mut plan, &review), send(Key::Enter, true));
    assert_eq!(step(&mut plan, &WORKING), Next::Done(AgentInputOutcome::Sent));
}

#[test]
fn each_question_step_waits_for_its_screen_and_never_sends_a_key_twice() {
    let first = screen("claude-160x48", Agent::Claude, "question");
    let multi = screen("claude-160x48", Agent::Claude, "question_multi");
    let review = screen("claude-160x48", Agent::Claude, "question_submit");
    // a box that has not ticked yet is not toggled again: pending until it shows
    let mut plan = Plan::new(
        Action::Question(vec![languages()], vec![answer(&["French"], None)]),
        Agent::Claude,
    );
    assert_eq!(step(&mut plan, &multi), send(Key::Digit(2), true));
    assert_eq!(step(&mut plan, &multi), Next::Wait);
    assert_eq!(step(&mut plan, &multi), Next::Wait);
    let french = varied(&multi, None, &[false, true, false], None);
    assert_eq!(step(&mut plan, &french), send(Key::Tab, true));
    // the first tab still up after its digit: the next tab is waited for, not answered on the old one
    let mut plan = Plan::new(
        Action::Question(
            vec![style(), languages()],
            vec![answer(&["Casual"], None), answer(&["Spanish"], None)],
        ),
        Agent::Claude,
    );
    assert_eq!(step(&mut plan, &first), send(Key::Digit(2), true));
    assert_eq!(step(&mut plan, &first), Next::Wait);
    assert_eq!(step(&mut plan, &multi), send(Key::Digit(1), true));
    // the review not up yet after the last Tab, then the review still up after its Enter: pending
    let mut plan = Plan::new(
        Action::Question(vec![languages()], vec![answer(&[], None)]),
        Agent::Claude,
    );
    assert_eq!(step(&mut plan, &multi), send(Key::Tab, true));
    assert_eq!(step(&mut plan, &multi), Next::Wait);
    assert_eq!(step(&mut plan, &review), send(Key::Enter, true));
    assert_eq!(step(&mut plan, &review), Next::Wait);
    assert_eq!(step(&mut plan, &WORKING), Next::Done(AgentInputOutcome::Sent));
}

#[test]
fn typed_text_may_follow_the_placeholder_or_take_its_place_and_a_draft_there_gets_nothing() {
    let picker = screen("claude-44x36", Agent::Claude, "question");
    let focused = varied(&picker, Some(3), &[], None);
    // the text in the placeholder's place, or a long paste's stand-in there
    for typed in [Typed::Instead("Blue"), Typed::Instead("[Pasted text #1 +3 lines]")] {
        let mut plan = Plan::new(
            Action::Question(vec![color()], vec![answer(&[], Some("Blue"))]),
            Agent::Claude,
        );
        step(&mut plan, &picker);
        step(&mut plan, &focused);
        let typed = varied(&picker, Some(3), &[], Some(typed));
        assert_eq!(step(&mut plan, &typed), send(Key::Enter, true));
        assert_eq!(step(&mut plan, &IDLE), Next::Done(AgentInputOutcome::Sent));
    }
    // something else in the row after the paste (or the paste half shown): no Enter
    let mut plan = Plan::new(
        Action::Question(vec![color()], vec![answer(&[], Some("Blue"))]),
        Agent::Claude,
    );
    step(&mut plan, &picker);
    step(&mut plan, &focused);
    assert_eq!(
        step(&mut plan, &varied(&picker, Some(3), &[], Some(Typed::Instead("Bl")))),
        Next::Wait
    );
    // a draft typed into the row at the terminal, after the placeholder or in its place: nothing goes
    for draft in [Typed::After("Gre"), Typed::Instead("Gre")] {
        for answered in [answer(&["Red"], None), answer(&[], Some("Blue"))] {
            let mut plan = Plan::new(Action::Question(vec![color()], vec![answered]), Agent::Claude);
            assert_eq!(
                step(&mut plan, &varied(&picker, None, &[], Some(draft))),
                Next::Done(AgentInputOutcome::NotOnScreen)
            );
        }
    }
}

fn limit() -> ChatQuestionItem {
    item(LIMIT_QUESTION, false, &LIMIT_OPTIONS)
}

#[test]
fn a_wrapped_question_with_long_options_is_answered_at_any_height_that_shows_its_options() {
    // the question Claude Code 2.1.318 asked, its rows quoted with `│`; on a short screen its top scrolled off
    for rows in [40, 34, 30, 28] {
        let screen = Agent::Claude.read(&limit_picker(rows).frame());
        let mut plan = Plan::new(
            Action::Question(vec![limit()], vec![answer(&["Statusline bridge"], None)]),
            Agent::Claude,
        );
        assert_eq!(step(&mut plan, &screen), send(Key::Digit(2), true), "{rows} rows");
        assert_eq!(
            step(&mut plan, &IDLE),
            Next::Done(AgentInputOutcome::Sent),
            "{rows} rows"
        );
    }
}

#[test]
fn a_picker_whose_question_scrolled_off_is_answered_only_when_its_options_tell_it_apart() {
    let short = Agent::Claude.read(&limit_picker(28).frame());
    let usage = || answer(&["Usage API (Recommended)"], None);
    // another question of the call offers the same options: which one is on screen can't be told
    let twin = item("Where should Codex's limit come from?", false, &LIMIT_OPTIONS);
    let mut plan = Plan::new(
        Action::Question(vec![limit(), twin], vec![usage(), usage()]),
        Agent::Claude,
    );
    assert_eq!(step(&mut plan, &short), Next::Done(AgentInputOutcome::NotOnScreen));
    // what is left of the question on screen must be its end
    let Screen::Question(mut picker) = Agent::Claude.read(&limit_picker(30).frame()) else {
        panic!("not the picker")
    };
    assert!(picker.cut);
    picker.question = "Where should the chat get Codex's weekly limit?".to_owned();
    let mut plan = Plan::new(Action::Question(vec![limit()], vec![usage()]), Agent::Claude);
    assert_eq!(
        step(&mut plan, &Screen::Question(picker)),
        Next::Done(AgentInputOutcome::NotOnScreen)
    );
}

#[test]
fn a_multi_select_question_before_the_last_one_shows_next_under_its_free_text_row() {
    // Claude Code draws `Submit` under the free-text row of the last question's boxes and `Next` under one with
    // another question after it (seen on 2.1.318): both rows are the untouched placeholder
    let multi = screen("claude-160x48", Agent::Claude, "question_multi");
    let Screen::Question(mut picker) = multi.clone() else {
        panic!("not the picker")
    };
    picker.options.last_mut().expect("its free-text row").text = "Type something Next".to_owned();
    let before_last = Screen::Question(picker);
    let mut plan = Plan::new(
        Action::Question(vec![languages()], vec![answer(&["Spanish"], None)]),
        Agent::Claude,
    );
    assert_eq!(step(&mut plan, &before_last), send(Key::Digit(1), true));
    let spanish = varied(&before_last, None, &[true, false, false], None);
    assert_eq!(step(&mut plan, &spanish), send(Key::Tab, true));
}

#[test]
fn free_text_in_a_multi_select_question_goes_through_its_row_then_tab() {
    let multi = screen("claude-160x48", Agent::Claude, "question_multi");
    let review = screen("claude-160x48", Agent::Claude, "question_submit");
    // its digit, the paste, then Tab, whether the text followed the placeholder or took its place
    for typed in [Typed::After("Latin"), Typed::Instead("Latin")] {
        let mut plan = Plan::new(
            Action::Question(vec![languages()], vec![answer(&[], Some("Latin"))]),
            Agent::Claude,
        );
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
        let mut plan = Plan::new(
            Action::Question(vec![asked], vec![answer(&[], Some("x"))]),
            Agent::Claude,
        );
        assert_eq!(
            step(&mut plan, &picker),
            Next::Done(AgentInputOutcome::NotOnScreen),
            "{why}"
        );
    }
    let permission = screen("claude-160x48", Agent::Claude, "permission");
    let mut plan = Plan::new(
        Action::Question(vec![style()], vec![answer(&["Formal"], None)]),
        Agent::Claude,
    );
    assert_eq!(step(&mut plan, &permission), Next::Done(AgentInputOutcome::NotOnScreen));
}

fn codex(state: &str) -> Screen {
    screen(CODEX_QUESTIONS, Agent::Codex, state)
}

/// Codex's single question, and the two it asked in one call, as the transcript gives them.
fn colors() -> ChatQuestionItem {
    item(
        "Which color should the button be?",
        false,
        &[
            ("Red", "Use red for the button."),
            ("Green", "Use green for the button."),
            ("Blue", "Use blue for the button."),
        ],
    )
}

const SIZES: [(&str, &str); 3] = [
    ("Small", "Use a small button."),
    ("Medium", "Use a medium button."),
    ("Large", "Use a large button."),
];

fn sizes() -> ChatQuestionItem {
    item("Which size?", false, &SIZES)
}

fn shapes() -> ChatQuestionItem {
    item(
        "Which shape?",
        false,
        &[("Round", "Use a round button."), ("Square", "Use a square button.")],
    )
}

/// Codex's `picker` with its cursor on option `cursor` (0-based) and its notes row as `notes`.
fn codex_varied(screen: &Screen, cursor: usize, notes: Option<&str>) -> Screen {
    let Screen::Question(picker) = screen else {
        panic!("not the picker")
    };
    let mut picker: Picker = picker.clone();
    for (at, option) in picker.options.iter_mut().enumerate() {
        option.cursor = at == cursor;
    }
    picker.notes = notes.map(str::to_owned);
    Screen::Question(picker)
}

#[test]
fn a_codex_question_is_answered_by_its_digit_and_done_once_the_picker_goes() {
    let picker = codex("question");
    let mut plan = Plan::new(
        Action::Question(vec![colors()], vec![answer(&["Green"], None)]),
        Agent::Codex,
    );
    assert_eq!(step(&mut plan, &picker), send(Key::Digit(2), true));
    assert_eq!(step(&mut plan, &picker), Next::Wait, "the picker still up");
    assert_eq!(
        step(&mut plan, &codex("question_picked")),
        Next::Done(AgentInputOutcome::Sent)
    );
}

#[test]
fn codex_questions_go_one_at_a_time_and_free_text_is_a_note_on_none_of_the_above() {
    let mut plan = Plan::new(
        Action::Question(
            vec![sizes(), shapes()],
            vec![answer(&["Large"], None), answer(&[], Some("hexagon please"))],
        ),
        Agent::Codex,
    );
    let first = codex("question2");
    assert_eq!(step(&mut plan, &first), send(Key::Digit(3), true));
    assert_eq!(step(&mut plan, &first), Next::Wait, "the first question still up");
    // down to `None of the above` a row at a time, each move seen before the next
    let second = codex("question2_second");
    assert_eq!(step(&mut plan, &second), send(Key::Down, true));
    assert_eq!(step(&mut plan, &second), Next::Wait, "the Down not seen yet");
    let square = codex_varied(&second, 1, None);
    assert_eq!(step(&mut plan, &square), send(Key::Down, true));
    let none = codex_varied(&second, 2, None);
    assert_eq!(step(&mut plan, &none), send(Key::Tab, true));
    assert_eq!(step(&mut plan, &none), Next::Wait, "the notes row not open yet");
    let notes = codex("question2_notes");
    assert_eq!(
        step(&mut plan, &notes),
        send(Key::Paste("hexagon please".to_owned()), true)
    );
    assert_eq!(step(&mut plan, &notes), Next::Wait, "the note not shown yet");
    let typed = codex("question2_typed");
    assert_eq!(step(&mut plan, &typed), send(Key::Enter, true));
    assert_eq!(step(&mut plan, &typed), Next::Wait);
    assert_eq!(
        step(&mut plan, &codex("question2_done")),
        Next::Done(AgentInputOutcome::Sent)
    );
}

#[test]
fn free_text_on_an_earlier_codex_question_moves_on_with_its_enter() {
    let (first, second) = (codex("question2"), codex("question2_second"));
    let mut plan = Plan::new(
        Action::Question(
            vec![sizes(), shapes()],
            vec![answer(&[], Some("huge")), answer(&["Round"], None)],
        ),
        Agent::Codex,
    );
    for (at, key) in [(0, Key::Down), (1, Key::Down), (2, Key::Down), (3, Key::Tab)] {
        assert_eq!(step(&mut plan, &codex_varied(&first, at, None)), send(key, true));
    }
    assert_eq!(
        step(&mut plan, &codex_varied(&first, 3, Some(""))),
        send(Key::Paste("huge".to_owned()), true)
    );
    assert_eq!(
        step(&mut plan, &codex_varied(&first, 3, Some("huge"))),
        send(Key::Enter, true)
    );
    assert_eq!(step(&mut plan, &second), send(Key::Digit(1), true));
}

#[test]
fn a_codex_move_or_note_that_does_not_land_as_sent_sends_nothing_more() {
    let second = codex("question2_second");
    let other = |plan: &mut Plan| {
        step(plan, &codex("question2"));
        step(plan, &second)
    };
    let new_plan = || {
        Plan::new(
            Action::Question(
                vec![sizes(), shapes()],
                vec![answer(&["Small"], None), answer(&[], Some("hexagon please"))],
            ),
            Agent::Codex,
        )
    };
    // the cursor somewhere else than a row down: no second Down
    let mut plan = new_plan();
    assert_eq!(other(&mut plan), send(Key::Down, true));
    assert_eq!(step(&mut plan, &codex_varied(&second, 2, None)), Next::Wait);
    // the notes row opened holding someone's draft: no paste
    let mut plan = new_plan();
    other(&mut plan);
    step(&mut plan, &codex_varied(&second, 1, None));
    assert_eq!(step(&mut plan, &codex_varied(&second, 2, None)), send(Key::Tab, true));
    assert_eq!(step(&mut plan, &codex_varied(&second, 2, Some("draft"))), Next::Wait);
    // the note shown with more than was pasted: no Enter
    let mut plan = new_plan();
    other(&mut plan);
    step(&mut plan, &codex_varied(&second, 1, None));
    step(&mut plan, &codex_varied(&second, 2, None));
    step(&mut plan, &codex("question2_notes"));
    assert_eq!(
        step(&mut plan, &codex_varied(&second, 2, Some("draft hexagon please"))),
        Next::Wait
    );
}

#[test]
fn a_codex_question_not_on_screen_gets_nothing() {
    let first = codex("question2");
    let cases = [
        // the picker at another question than the one to answer, or asking more or fewer than the call
        (vec![shapes(), sizes()], "another question first"),
        (vec![sizes()], "one question, the picker asking two"),
        (
            vec![sizes(), shapes(), colors()],
            "three questions, the picker asking two",
        ),
        (vec![item("Which size?", false, &SIZES[..2]), shapes()], "other options"),
        (vec![item("Which width?", false, &SIZES), shapes()], "another question"),
        (
            vec![item("Which size?", true, &SIZES), shapes()],
            "a multi-select question",
        ),
    ];
    for (items, why) in cases {
        let answers = vec![answer(&[], Some("x")); items.len()];
        let mut plan = Plan::new(Action::Question(items, answers), Agent::Codex);
        assert_eq!(
            step(&mut plan, &first),
            Next::Done(AgentInputOutcome::NotOnScreen),
            "{why}"
        );
    }
    // a note being typed at the terminal: neither a digit nor a Down goes
    for picked in [answer(&["Small"], None), answer(&[], Some("x"))] {
        let mut plan = Plan::new(
            Action::Question(vec![sizes(), shapes()], vec![picked, answer(&["Round"], None)]),
            Agent::Codex,
        );
        assert_eq!(
            step(&mut plan, &codex_varied(&first, 3, Some(""))),
            Next::Done(AgentInputOutcome::NotOnScreen)
        );
    }
    // Claude's way with a picker is not Codex's: a Claude plan finds no `Type something.` on it
    let mut plan = Plan::new(
        Action::Question(vec![colors()], vec![answer(&["Green"], None)]),
        Agent::Claude,
    );
    assert_eq!(
        step(&mut plan, &codex("question")),
        Next::Done(AgentInputOutcome::NotOnScreen)
    );
    for state in ["idle", "permission", "question_picked"] {
        let mut plan = Plan::new(
            Action::Question(vec![colors()], vec![answer(&["Green"], None)]),
            Agent::Codex,
        );
        assert_eq!(
            step(&mut plan, &codex(state)),
            Next::Done(AgentInputOutcome::NotOnScreen),
            "{state}"
        );
    }
}

#[test]
fn codexs_answers_fit_as_one_pick_or_free_text_alone() {
    let fits = |answer: QuestionAnswer| Action::Question(vec![colors()], vec![answer]).check().is_ok();
    assert!(fits(answer(&["Green"], None)));
    assert!(fits(answer(&[], Some("teal"))));
    assert!(!fits(answer(&["Green"], Some("teal"))), "a pick or a note");
    assert!(!fits(answer(&[], None)));
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

/// The generation of `term`'s screen now.
fn generation(term: &Term) -> u64 {
    term.frame();
    loop {
        if let Some(screen) = term.pane.active_screen().unwrap() {
            return screen.generation;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// What `keys` put on the wire from a terminal that took `setup`.
fn wire(setup: &[u8], keys: &[Key]) -> Vec<u8> {
    let term = Term::new(80, 24);
    term.feed(setup);
    term.sent();
    let shown = generation(&term);
    for key in keys {
        assert_eq!(key.send_if(&term.pane, shown), Ok(true));
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
    // Down as the cursor keys mode asks
    assert_eq!(wire(b"", &[Key::Down]), b"\x1b[B");
    assert_eq!(wire(b"\x1b[?1h", &[Key::Down]), b"\x1bOB");
    // under the Kitty keyboard protocol Codex pushes, Escape is unambiguous
    assert_eq!(wire(b"\x1b[>7u", &[Key::Escape]), b"\x1b[27u");

    // a paste the terminal can't frame safely is refused whole
    let term = Term::new(80, 24);
    assert_eq!(
        Key::Paste("rm -rf x\n".to_owned()).send_if(&term.pane, generation(&term)),
        Err(PaneInputError::Refused {
            reason: UNSAFE_PASTE.to_owned()
        })
    );
    assert!(term.sent().is_empty());
}

#[test]
fn keys_go_only_to_the_screen_they_were_chosen_from() {
    let capture = Capture::load("claude-44x36");
    let term = capture.term();
    term.feed(capture.at("idle"));
    term.sent();
    let shown = generation(&term);
    // the agent draws a dialog after the screen was read: neither the Enter nor a paste goes
    term.feed(&capture.bytes[capture.at("idle").len()..capture.at("permission").len()]);
    assert_eq!(Key::Enter.send_if(&term.pane, shown), Ok(false));
    assert_eq!(Key::Paste("hi".to_owned()).send_if(&term.pane, shown), Ok(false));
    assert!(term.sent().is_empty());
    // read again, it goes
    assert_eq!(Key::Digit(1).send_if(&term.pane, generation(&term)), Ok(true));
    assert_eq!(term.sent(), b"1");
}

#[test]
fn both_agents_screens_take_a_bracketed_paste() {
    for (name, state) in [("claude-44x36", "idle"), ("codex-44x36", "idle")] {
        let capture = Capture::load(name);
        let term = capture.term();
        term.feed(capture.at(state));
        term.sent();
        assert_eq!(
            Key::Paste("hi\nthere".to_owned()).send_if(&term.pane, generation(&term)),
            Ok(true)
        );
        assert_eq!(term.sent(), b"\x1b[200~hi\nthere\x1b[201~", "{name}");
    }
}
