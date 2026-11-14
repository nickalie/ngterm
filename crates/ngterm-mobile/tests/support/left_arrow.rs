//! The e2e test's Left arrow steps. Claude Code takes a lone Left arrow at its empty prompt for "open agents" and
//! moves the conversation to a background process, so the phone's core holds that key back there and nowhere else:
//! with a draft in the prompt, a modifier held, a dialog up, in the shell and in another program it is sent as typed.
//!
//! What reached the fake agent is read from the cursor keys it logs ([`FakeAgent::keys`]); a Right arrow sent after
//! each Left marks the place a held-back Left would have had in that log.

use std::time::Duration;

use ngterm_mobile::{Pane, PaneKey, PaneKeyAction};

use crate::agent::{FakeAgent, State};
use crate::chat::{FOOTER, Scene, WORKING};
use crate::phone::{enter, poll, screen_text};

const SHIFT: u8 = 1;
const CTRL: u8 = 1 << 1;
const ALT: u8 = 1 << 2;
const SUPER: u8 = 1 << 3;
const CAPS_LOCK: u8 = 1 << 4;
const NUM_LOCK: u8 = 1 << 5;
/// The placeholder Claude Code draws in its empty prompt, as the capture holds it.
const PLACEHOLDER: &str = "how does";
/// What the phone types as a draft.
const DRAFT: &str = "e2e draft";
/// The picker's question, as the capture draws it.
const QUESTION: &str = "Pick a color";

fn key(name: &str, action: PaneKeyAction, mods: u8) -> PaneKey {
    PaneKey {
        action,
        key: name.to_owned(),
        mods,
        consumed: 0,
        composing: false,
        text: String::new(),
        unshifted: None,
    }
}

fn left(mods: u8) -> PaneKey {
    key("arrow_left", PaneKeyAction::Press, mods)
}

/// Whether `sequence` is the arrow whose final byte is `direction`, as the fake agent logs it.
fn arrow(sequence: &str, direction: char) -> bool {
    sequence.starts_with("ESC") && sequence.ends_with(direction)
}

impl Scene<'_> {
    /// The phone leads Claude's pane, which is idle with nothing typed.
    pub(crate) fn left_arrow(&mut self, agent: &FakeAgent) {
        let pane = self.first.terminal.clone().expect("the phone's pane");
        self.first.wait_screen(PLACEHOLDER);

        // the empty prompt: a press, a held key's repeats and its release all stay on the phone
        for action in [PaneKeyAction::Press, PaneKeyAction::Repeat, PaneKeyAction::Release] {
            held_back(agent, &pane, key("arrow_left", action, 0), "the empty prompt");
        }
        // the locks are no modifiers
        held_back(
            agent,
            &pane,
            left(CAPS_LOCK | NUM_LOCK),
            "the empty prompt, the locks on",
        );

        // a modifier makes it a cursor movement, which Claude Code never takes for "open agents"
        for (mods, name) in [(SHIFT, "Shift"), (CTRL, "Ctrl"), (ALT, "Alt"), (SUPER, "Super")] {
            let sent = reaches(agent, &pane, left(mods), name);
            assert_ne!(sent, "ESC[D", "{name}+Left reached the agent as a lone Left");
        }

        // a turn in progress: Claude Code moves the conversation once the turn ends
        agent.show(State::Working);
        self.first.wait_screen(WORKING);
        held_back(agent, &pane, left(0), "the empty prompt of a working agent");
        agent.show(State::Idle);
        self.first.wait_screen(FOOTER);

        // a draft: the key moves the cursor in it
        assert!(pane.type_text(DRAFT.to_owned()).expect("type a draft"));
        self.first.wait_screen(DRAFT);
        reaches(agent, &pane, left(0), "a draft");
        self.erase(&pane, DRAFT.chars().count());
        held_back(agent, &pane, left(0), "the prompt emptied again");

        // a draft of blanks reads as an empty prompt, but for the cursor standing after them
        assert!(pane.type_text("  ".to_owned()).expect("type blanks"));
        self.wait_gone(PLACEHOLDER);
        reaches(agent, &pane, left(0), "a draft of blanks");
        self.erase(&pane, 2);
        held_back(agent, &pane, left(0), "the blanks erased");

        // a dialog: the key moves between its choices
        agent.show(State::Question);
        self.first.wait_screen(QUESTION);
        reaches(agent, &pane, left(0), "the question picker");
        agent.show(State::Idle);
        self.first.wait_screen(PLACEHOLDER);
    }

    /// The agent is gone and the pane is back at its shell: the key edits the line there, and a program that reads
    /// the terminal itself gets it.
    pub(crate) fn left_arrow_elsewhere(&mut self) {
        let pane = self.first.terminal.clone().expect("the phone's pane");
        // the shell's line editor: `b` lands before the `c` only when the Left arrived
        assert!(pane.type_text("echo lef-ac".to_owned()).expect("type"));
        self.first.wait_screen("echo lef-ac");
        assert!(
            pane.key(left(0)).expect("Left in the shell"),
            "Left was not sent to the shell"
        );
        assert!(pane.type_text("b".to_owned()).expect("type"));
        assert!(pane.key(enter()).expect("press enter"));
        self.first.wait_screen("\nlef-abc");

        // another program with nothing typed: it reads three bytes and prints them quoted
        let reader = r#"bash -c 'IFS= read -rsn3 k; printf "got-%q-end\n" "$k"; echo rea"d"y'"#;
        assert!(pane.type_text(reader.to_owned()).expect("type"));
        assert!(pane.key(enter()).expect("press enter"));
        // the program is in the foreground once the shell's echo of the line is past: nothing shows it reading, so
        // the key is pressed until it answers
        poll("the program's answer", || {
            assert!(
                pane.key(left(0)).expect("Left in a program"),
                "Left was not sent to the program"
            );
            self.first.screen_shows("ready", Duration::from_secs(2)).ok()
        });
        self.first.wait_screen("got-$'\\E[D'-end");
    }

    /// Erases `count` characters of the draft and waits for the empty prompt's placeholder.
    fn erase(&mut self, pane: &Pane, count: usize) {
        for _ in 0..count {
            assert!(
                pane.key(key("backspace", PaneKeyAction::Press, 0))
                    .expect("press Backspace")
            );
        }
        self.first.wait_screen(PLACEHOLDER);
    }

    /// Waits for the phone's screen to hold `needle` no more.
    fn wait_gone(&mut self, needle: &str) {
        poll("the screen without it", || {
            self.first.drain();
            let pane = self.first.terminal.as_ref()?;
            let screen = screen_text(&pane.frame(0).ok()?);
            (!screen.contains(needle)).then_some(())
        });
    }
}

/// `key` pressed on `screen` is not sent: the core says so, and of it and the Right arrow pressed after it only
/// the Right arrow reaches the agent.
fn held_back(agent: &FakeAgent, pane: &Pane, key: PaneKey, screen: &str) {
    let before = agent.keys().len();
    let sent = pane.key(key).unwrap_or_else(|error| panic!("{screen}: {error:?}"));
    assert!(!sent, "{screen}: the core says it sent the Left arrow");
    let after = marked(agent, pane, before);
    assert_eq!(
        after.len(),
        1,
        "{screen}: the agent read {after:?}, not just the Right arrow"
    );
}

/// `key` pressed on `screen` is sent and reaches the agent as a Left arrow; answers the sequence the agent read.
fn reaches(agent: &FakeAgent, pane: &Pane, key: PaneKey, screen: &str) -> String {
    let before = agent.keys().len();
    let sent = pane.key(key).unwrap_or_else(|error| panic!("{screen}: {error:?}"));
    assert!(sent, "{screen}: the core held the Left arrow back");
    let after = marked(agent, pane, before);
    assert!(
        after.len() == 2 && arrow(&after[0], 'D'),
        "{screen}: the agent read {after:?}, not a Left arrow and then the Right arrow"
    );
    after[0].clone()
}

/// Presses a Right arrow and waits for the agent to read it; answers what the agent read since `before`.
fn marked(agent: &FakeAgent, pane: &Pane, before: usize) -> Vec<String> {
    let right = key("arrow_right", PaneKeyAction::Press, 0);
    assert!(pane.key(right).expect("press Right"), "the Right arrow was not sent");
    poll("the Right arrow at the agent", || {
        let read = agent.keys().split_off(before);
        read.last().is_some_and(|last| arrow(last, 'C')).then_some(read)
    })
}
