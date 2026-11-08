use super::*;

#[gpui_kit::test]
fn stale_answers_do_not_release_waiters_or_remember_a_choice(cx: &mut gpui_kit::TestAppContext) {
    let mut prompts = ClipboardPrompts::default();
    let key = PromptKey {
        access: ClipboardAccess::Read,
        terminal: 7,
    };
    let answers = Rc::new(std::cell::RefCell::new(Vec::new()));
    let recorded = Rc::clone(&answers);
    let Admission::Prompt(id) = prompts.admit(key, Box::new(move |allowed, _| recorded.borrow_mut().push(allowed)))
    else {
        panic!("a fresh request prompts")
    };
    assert!(prompts.answer(key, id + 1, true, true).is_empty());
    assert_eq!(prompts.decision(ClipboardAccess::Read), ClipboardDecision::Prompt);
    for waiter in prompts.answer(key, id, false, true) {
        cx.update(|cx| waiter(false, cx));
    }
    assert_eq!(*answers.borrow(), [false]);
    assert!(prompts.answer(key, id, true, true).is_empty());
    assert_eq!(prompts.decision(ClipboardAccess::Read), ClipboardDecision::Deny);
    let recorded = Rc::clone(&answers);
    let Admission::Decided(waiter, allowed) =
        prompts.admit(key, Box::new(move |allowed, _| recorded.borrow_mut().push(allowed)))
    else {
        panic!("the denial is remembered")
    };
    cx.update(|cx| waiter(allowed, cx));
    assert_eq!(*answers.borrow(), [false, false]);
    assert_eq!(prompts.decision(ClipboardAccess::Write), ClipboardDecision::Prompt);
}

use super::super::{TerminalConfig, TerminalSpec};
use gpui_kit::AppContext as _;

#[gpui_kit::test]
fn writes_keep_the_clipboard_on_refusal_or_nontext_and_accept_text_parameters(cx: &mut gpui_kit::TestAppContext) {
    let terminal = cx.new(|cx| {
        Terminal::new(
            TerminalSpec::default(),
            TerminalConfig::default(),
            Box::new(|_| panic!("no view")),
            cx,
        )
    });
    let shared = terminal.read_with(cx, |terminal, _| Rc::clone(terminal.shared()));
    let write = |mime: &str| ClipboardWrite {
        location: ClipboardLocation::Standard,
        contents: vec![ngterm_term::ClipboardContent {
            mime: mime.into(),
            data: b"new text".to_vec(),
        }],
    };
    for policy in [ConfigAccess::Deny, ConfigAccess::Ask, ConfigAccess::Allow] {
        let mut config = TerminalConfig::default();
        config.clipboard.write = policy;
        terminal.update(cx, |terminal, cx| terminal.set_config(config, cx));
        cx.update(|cx| {
            cx.write_to_clipboard(ClipboardItem::new_string("before".into()));
            on_write(&shared, &write("image/png"), cx);
            assert_eq!(
                cx.read_from_clipboard().and_then(|item| item.text()).as_deref(),
                Some("before")
            );
            on_write(&shared, &write("text/plain;charset=utf-8"), cx);
            let expected = if policy == ConfigAccess::Allow {
                "new text"
            } else {
                "before"
            };
            assert_eq!(
                cx.read_from_clipboard().and_then(|item| item.text()).as_deref(),
                Some(expected)
            );
        });
    }
}

#[gpui_kit::test]
#[cfg_attr(windows, ignore = "drives its pane with a POSIX sh script")]
fn allowed_reads_answer_the_program_without_prompting(cx: &mut gpui_kit::TestAppContext) {
    use super::super::surface::tests::Fixture;
    let script = "stty raw -echo; printf 'ready'; dd bs=1 count=1 >/dev/null 2>&1; printf '\\033]52;c;?\\007'; dd bs=1 count=12 2>/dev/null | od -An -tx1; printf done; exec cat";
    let mut f = Fixture::new(cx, script, "clipboard-read = allow\n");
    f.cx.write_to_clipboard(ClipboardItem::new_string("hi".into()));
    f.pane().write(b"x").unwrap();
    f.wait("done");
    let text = f.text();
    let bytes: Vec<_> = text.split_whitespace().collect();
    assert!(
        bytes.windows(4).any(|bytes| bytes == ["61", "47", "6b", "3d"]),
        "base64 hi in the reply: {text}"
    );
}

#[gpui_kit::test]
fn one_prompt_answers_queued_requests_and_a_session_allow_applies_to_other_terminals(
    cx: &mut gpui_kit::TestAppContext,
) {
    let mut prompts = ClipboardPrompts::default();
    let key = PromptKey {
        access: ClipboardAccess::Write,
        terminal: 1,
    };
    let replies = Rc::new(std::cell::RefCell::new(Vec::new()));
    let waiter = || {
        let replies = Rc::clone(&replies);
        Box::new(move |allowed, _: &mut App| replies.borrow_mut().push(allowed)) as Waiter
    };
    let Admission::Prompt(id) = prompts.admit(key, waiter()) else {
        panic!("the first request prompts")
    };
    assert!(matches!(prompts.admit(key, waiter()), Admission::Queued));
    for waiter in prompts.answer(key, id, true, true) {
        cx.update(|cx| waiter(true, cx));
    }
    assert_eq!(*replies.borrow(), [true, true]);
    let key = PromptKey { terminal: 2, ..key };
    let Admission::Decided(waiter, allowed) = prompts.admit(key, waiter()) else {
        panic!("the session choice applies to every terminal")
    };
    cx.update(|cx| waiter(allowed, cx));
    assert_eq!(*replies.borrow(), [true, true, true]);
    assert_eq!(prompts.decision(ClipboardAccess::Read), ClipboardDecision::Prompt);
}

#[gpui_kit::test]
#[cfg_attr(windows, ignore = "drives its pane with a POSIX sh script")]
fn denied_reads_release_the_pane_without_pasting_clipboard_data(cx: &mut gpui_kit::TestAppContext) {
    use super::super::surface::tests::Fixture;
    let script =
        "stty raw -echo; printf 'ready'; dd bs=1 count=1 >/dev/null 2>&1; printf '\\033]52;c;?\\007done'; exec cat -v";
    let mut f = Fixture::new(cx, script, "clipboard-read = deny\n");
    f.cx.write_to_clipboard(ClipboardItem::new_string("secret".into()));
    f.pane().write(b"x").unwrap();
    f.wait("done");
    // a reply goes to the pane's input before anything written after the read was answered, so `cat` echoes it
    // ahead of the sentinel
    f.pane().write(b"END").unwrap();
    f.wait("END");
    assert!(
        !f.text().contains("c2VjcmV0"),
        "no clipboard data in the pane: {}",
        f.text()
    );
    assert!(
        !f.cx
            .update(|window, cx| crate::modal::alert_open(window.window_handle(), cx))
    );
}
