//! The chat's actions through the whole core, over the fake transport's channels: a pane's attach fed a recorded
//! agent screen, its role reports, and the keys that come back down the channel. Leadership first (a follower claims,
//! a leader doesn't, a claim refused or unanswered sends nothing, a dialog gone by the time the claim lands sends
//! nothing), then each action's keys, the lead lost between a paste and its Enter, a handle whose pane was hidden,
//! and reading the chat or switching it with the terminal never claiming or resizing. Last, every action with the
//! terminal view scrolled back onto an old, complete copy of another state in the history: the program's live screen
//! is what is read, and no key goes to the state the view shows.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use futures_lite::future::block_on;
use ngterm_core::remote::frames::{
    self, Body, PresentationAgent, PresentationFrame, PresentationHello, PresentationLayout, PresentationMode,
    PresentationPane, PresentationSnapshot,
};
use ngterm_core::remote::ssh::present_tail;

use super::pane_tests::{
    Fixture, MACHINE, PANE, SESSION, contains, inputs_until, lead_of, listing, pane_uuid, role_title, send,
};
use crate::api::{MobileEvent, Pane, PaneInputError, PaneSize, PaneWheel, PaneWheelOutcome};
use crate::chat::input::{ACTION_IN_FLIGHT, AgentInputOutcome, InputTiming, NO_AGENT_TO_DRIVE, QuestionAnswer};
use crate::chat::matchers::{Agent, Screen};
use crate::chat::tests::matchers::{Capture, approval_of, asked, draft_drawn, into_prompt};
use crate::chat::{ChatQuestion, ChatQuestionItem, ChatQuestionOption, ChatToolStatus};
use crate::chat::{Side, commands_tail};
use crate::pane::{PaneRole, without_escapes};
use crate::transport::fake::{Opened, Script};
use crate::transport::{ChannelInput, ChannelPeer};

/// Short settles, and deadlines no stall of a test's own thread can cross: the test decides when the screen answers,
/// so a loaded machine must not make the action give up on it first. A test that waits for a deadline to run out
/// uses [`expiring`].
fn timing() -> InputTiming {
    InputTiming {
        quiet: Duration::from_millis(150),
        settle_max: Duration::from_secs(1),
        confirm: Duration::from_secs(60),
        lead_timeout: Duration::from_secs(60),
        poll: Duration::from_millis(10),
    }
}

/// Short deadlines, for a test that waits for one to run out: a claim that is never answered, a step that never shows.
fn expiring() -> InputTiming {
    InputTiming {
        confirm: Duration::from_millis(1500),
        lead_timeout: Duration::from_millis(800),
        ..timing()
    }
}

/// A grid the size of a capture's.
fn grid(capture: &Capture) -> PaneSize {
    PaneSize {
        cols: capture.cols,
        rows: capture.rows,
        cell_width: 9,
        cell_height: 18,
    }
}

/// An attached session whose origin announces `agent` in its one pane, that pane shown at `capture`'s grid.
struct Agents {
    fixture: Fixture,
    stream: Opened,
    capture: Capture,
    pane: Arc<Pane>,
    /// The pane's first attach.
    first: Opened,
}

impl Agents {
    fn start(agent: &str, capture: &str) -> Self {
        Self::start_timed(agent, capture, timing())
    }

    fn start_timed(agent: &str, capture: &str, timing: InputTiming) -> Self {
        let mut fixture = Fixture::connected_timed(vec![listing(true)], timing);
        block_on(fixture.core.attach_session(MACHINE.to_owned(), SESSION.to_owned())).unwrap();
        let stream = fixture.opened(|opened| opened.command == present_tail(SESSION).unwrap());
        let frame = |rev: i64, body: Body| {
            frames::encode(&PresentationFrame {
                generation: 1,
                rev,
                body,
            })
            .unwrap()
        };
        let hello = Body::Hello(PresentationHello {
            version: frames::VERSION,
            kinds: Vec::new(),
            mode: PresentationMode::Mirror,
        });
        let snapshot = Body::Snapshot(PresentationSnapshot {
            layout: Some(PresentationLayout {
                panes: vec![pane_uuid()],
                primary: pane_uuid(),
                axis: None,
                shown: false,
            }),
            ..PresentationSnapshot::default()
        });
        let announced = Body::Agent(PresentationAgent {
            pane: PresentationPane::Identity(pane_uuid()),
            agent: Some(agent.to_owned()),
            conversation_id: Some("c1".to_owned()),
        });
        send(&stream.peer, &frame(1, hello));
        send(&stream.peer, &frame(2, snapshot));
        send(&stream.peer, &frame(3, announced));
        fixture.next(|event| matches!(event, MobileEvent::PaneAgent { .. }).then_some(()));
        let capture = Capture::load(capture);
        let pane = block_on(fixture.core.show_pane(
            MACHINE.to_owned(),
            SESSION.to_owned(),
            PANE.to_owned(),
            grid(&capture),
        ))
        .unwrap();
        let first = fixture.attach();
        fixture.attached();
        Self {
            fixture,
            stream,
            capture,
            pane,
            first,
        }
    }

    /// The pane's first attach reports `role`.
    fn first_is(&mut self, role: &str) {
        let nonce = lead_of(&self.first.command).nonce;
        send(&self.first.peer, &role_title(&nonce, role, 1));
        let expected = match role {
            "leader" => PaneRole::Leader,
            "follower" => PaneRole::Follower,
            _ => PaneRole::Unowned,
        };
        loop {
            let (role, _) = self.fixture.lead();
            if role == Some(expected) {
                return;
            }
        }
    }

    /// Runs `action` on the pane on a thread of its own: its outcome arrives on the receiver.
    fn act<F>(&self, action: F) -> Receiver<Result<AgentInputOutcome, PaneInputError>>
    where
        F: FnOnce(Arc<Pane>) -> Result<AgentInputOutcome, PaneInputError> + Send + 'static,
    {
        let (done, outcome) = mpsc::channel();
        let pane = Arc::clone(&self.pane);
        thread::spawn(move || {
            let _ = done.send(action(pane));
        });
        outcome
    }

    /// Feeds `peer` the capture up to `state`, and waits until the terminal took it.
    fn show(&self, peer: &ChannelPeer, state: &str) {
        self.feed(peer, self.capture.at(state));
    }

    /// Sends `bytes` down `peer` and waits until the pane's terminal took them.
    fn feed(&self, peer: &ChannelPeer, bytes: &[u8]) {
        feed_into(&self.pane, peer, bytes);
    }

    /// Feeds `peer` `text` in the prompt of the capture's `state`, as the TUI draws what was pasted.
    fn show_typed(&self, peer: &ChannelPeer, agent: Agent, state: &str, text: &str) {
        self.show_in_prompt(peer, agent, state, "0", text);
    }

    /// Feeds `peer` `text` in `style` (an SGR) in the prompt of the capture's `state`.
    fn show_in_prompt(&self, peer: &ChannelPeer, agent: Agent, state: &str, style: &str, text: &str) {
        let term = self.capture.term();
        term.feed(self.capture.at(state));
        self.feed(peer, &into_prompt(&term, agent, style, text));
    }
}

/// Sends `bytes` down `peer` and waits until `pane`'s terminal took them: an action settles on what the terminal
/// holds, so one started sooner would read the screen before them and give up on it.
fn feed_into(pane: &Pane, peer: &ChannelPeer, bytes: &[u8]) {
    let terminal = pane.terminal().expect("the pane's terminal");
    let taken = terminal.stats().bytes + bytes.len() as u64;
    send(peer, bytes);
    for _ in 0..1000 {
        if terminal.stats().bytes >= taken {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("the terminal never took what was sent");
}

fn outcome(
    receiver: &Receiver<Result<AgentInputOutcome, PaneInputError>>,
) -> Result<AgentInputOutcome, PaneInputError> {
    receiver
        .recv_timeout(Duration::from_secs(20))
        .expect("the action answers")
}

/// What the core sent down `peer` within a moment: the bytes, and the other inputs (a resize, a close) in order.
fn drained(peer: &mut ChannelPeer) -> (Vec<u8>, Vec<ChannelInput>) {
    thread::sleep(Duration::from_millis(300));
    let (mut bytes, mut others) = (Vec::new(), Vec::new());
    while let Some(input) = peer.try_input() {
        match input {
            ChannelInput::Data(data) => bytes.extend(data),
            other => others.push(other),
        }
    }
    (bytes, others)
}

/// The keys typed down `peer` within a moment: what the core sent, the terminal's replies to the program's queries
/// taken out.
fn typed(peer: &mut ChannelPeer) -> Vec<u8> {
    without_escapes(&drained(peer).0)
}

/// The claiming attach the action opened, reported leader, showing the capture's `state`.
fn claimed(agents: &mut Agents, state: &str) -> Opened {
    let claim = agents.fixture.attach();
    let lead = lead_of(&claim.command);
    assert!(lead.claim, "the action claims the lead");
    agents.feed(&claim.peer, &role_title(&lead.nonce, "leader", 2));
    agents.show(&claim.peer, state);
    claim
}

#[test]
fn a_follower_claims_the_lead_waits_for_the_redraw_then_answers() {
    let mut agents = Agents::start("claude", "claude-44x36");
    agents.first_is("follower");
    agents.show(&agents.first.peer, "permission");
    let answer = agents.act(|pane| block_on(pane.answer_approval(asked("claude-44x36"), "No".to_owned())));

    let mut claim = claimed(&mut agents, "permission");
    let (bytes, _) = inputs_until(&mut claim.peer, |bytes, _| without_escapes(bytes).contains(&b'4'));
    assert_eq!(
        without_escapes(&bytes),
        b"4",
        "option 4, No, once the claim's screen settled"
    );
    // the dialog goes: the answer is confirmed
    send(&claim.peer, b"\x1b[2J\x1b[H");
    assert_eq!(outcome(&answer), Ok(AgentInputOutcome::Sent));
    assert!(
        typed(&mut agents.first.peer).is_empty(),
        "nothing into the follower's attach"
    );
}

#[test]
fn a_leader_skips_the_claim() {
    let mut agents = Agents::start("codex", "codex-44x36");
    agents.first_is("leader");
    agents.show(&agents.first.peer, "permission");
    let answer = agents.act(|pane| block_on(pane.answer_approval(asked("codex-44x36"), "Yes, proceed".to_owned())));
    let (bytes, _) = inputs_until(&mut agents.first.peer, |bytes, _| {
        without_escapes(bytes).contains(&b'y')
    });
    assert_eq!(without_escapes(&bytes), b"y");
    send(&agents.first.peer, b"\x1b[2J\x1b[H");
    assert_eq!(outcome(&answer), Ok(AgentInputOutcome::Sent));
    assert!(!agents.fixture.attaches_soon(), "no claim");
}

#[test]
fn a_claim_refused_or_unanswered_sends_nothing() {
    let mut agents = Agents::start_timed("claude", "claude-44x36", expiring());
    agents.first_is("follower");
    let answer = agents.act(|pane| block_on(pane.answer_approval(asked("claude-44x36"), "Yes".to_owned())));
    let mut claim = agents.fixture.attach();
    let lead = lead_of(&claim.command);
    agents.show(&claim.peer, "permission");
    // another client claimed meanwhile: this attach follows
    send(&claim.peer, &role_title(&lead.nonce, "follower", 2));
    assert_eq!(outcome(&answer), Ok(AgentInputOutcome::LeadRefused));
    assert!(typed(&mut claim.peer).is_empty());

    // a claim whose attach never reports
    let answer = agents.act(|pane| block_on(pane.answer_approval(asked("claude-44x36"), "Yes".to_owned())));
    let mut silent = agents.fixture.attach();
    agents.show(&silent.peer, "permission");
    assert_eq!(outcome(&answer), Ok(AgentInputOutcome::LeadRefused));
    assert!(typed(&mut silent.peer).is_empty());
}

#[test]
fn a_dialog_gone_by_the_time_the_claim_lands_gets_nothing() {
    let mut agents = Agents::start("claude", "claude-44x36");
    agents.first_is("follower");
    agents.show(&agents.first.peer, "permission");
    let answer = agents.act(|pane| block_on(pane.answer_approval(asked("claude-44x36"), "Yes".to_owned())));
    // answered at the desktop while the claim was on its way: the leader's grid shows the prompt again
    let mut claim = claimed(&mut agents, "idle");
    assert_eq!(outcome(&answer), Ok(AgentInputOutcome::NotOnScreen));
    assert!(typed(&mut claim.peer).is_empty());
}

#[test]
fn a_card_answered_elsewhere_never_answers_the_next_dialog() {
    // the card's call was answered at the desktop and the agent asks about another: the card's `Yes` (its own
    // label, or the agent's usual one) finds that dialog's `Yes`, and still nothing goes
    for (name, capture, label) in [
        ("claude", "claude-44x36", "Yes"),
        ("codex", "codex-44x36", "Yes, proceed"),
    ] {
        let mut agents = Agents::start(name, capture);
        agents.first_is("leader");
        agents.show(&agents.first.peer, "permission");
        let card = approval_of("rm -rf build");
        let answer = agents.act(move |pane| block_on(pane.answer_approval(card, label.to_owned())));
        assert_eq!(outcome(&answer), Ok(AgentInputOutcome::NotOnScreen), "{capture}");
        assert!(typed(&mut agents.first.peer).is_empty(), "{capture}: no key");
    }
}

#[test]
fn a_prompt_is_pasted_then_entered_and_a_second_action_waits_its_turn() {
    let mut agents = Agents::start("claude", "claude-44x36");
    agents.first_is("leader");
    agents.show(&agents.first.peer, "idle");
    let prompt = agents.act(|pane| block_on(pane.send_prompt("/compact keep it short".to_owned())));
    let (pasted, _) = inputs_until(&mut agents.first.peer, |bytes, _| contains(bytes, b"\x1b[201~"));
    // the paste went and its Enter waits for the screen: the action holds the pane
    assert_eq!(
        block_on(agents.pane.interrupt()),
        Err(PaneInputError::Refused {
            reason: ACTION_IN_FLIGHT.to_owned()
        })
    );
    assert!(
        pasted.ends_with(b"\x1b[200~/compact keep it short\x1b[201~"),
        "a bracketed paste of the whole command: {:?}",
        String::from_utf8_lossy(&pasted)
    );
    // the TUI shows the pasted text in its prompt: then the Enter goes
    agents.show_typed(&agents.first.peer, Agent::Claude, "idle", "/compact keep it short");
    let (entered, _) = inputs_until(&mut agents.first.peer, |bytes, _| bytes.ends_with(b"\r"));
    assert_eq!(without_escapes(&entered), b"\r", "Enter alone");
    assert_eq!(outcome(&prompt), Ok(AgentInputOutcome::Sent));
}

/// Longer than a few quiet periods: a round trip on a slow link, the action still waiting for its screen.
const ROUND_TRIP: Duration = Duration::from_millis(700);

#[test]
fn a_slow_round_trip_holds_the_next_key_until_the_screen_answers_and_never_repeats_one() {
    // a prompt: the screen before the paste stays up a while, then the TUI shows the paste (a blank line in it)
    let mut agents = Agents::start("codex", "codex-44x36");
    agents.first_is("leader");
    agents.show(&agents.first.peer, "idle");
    let text = "Fix the parser.\n\nThen add tests.";
    let prompt = agents.act(move |pane| block_on(pane.send_prompt(text.to_owned())));
    inputs_until(&mut agents.first.peer, |bytes, _| contains(bytes, b"\x1b[201~"));
    thread::sleep(ROUND_TRIP);
    assert!(prompt.try_recv().is_err(), "still waiting for the paste to show");
    assert!(typed(&mut agents.first.peer).is_empty(), "no Enter, no second paste");
    let (drawn, _) = draft_drawn(&agents.capture, Agent::Codex, "idle", text);
    send(&agents.first.peer, &drawn);
    let (entered, _) = inputs_until(&mut agents.first.peer, |bytes, _| bytes.ends_with(b"\r"));
    assert_eq!(without_escapes(&entered), b"\r", "Enter alone");
    assert_eq!(outcome(&prompt), Ok(AgentInputOutcome::Sent));

    // an approval: the dialog stays up a while after its key, then goes
    let mut agents = Agents::start("claude", "claude-44x36");
    agents.first_is("leader");
    agents.show(&agents.first.peer, "permission");
    let answer = agents.act(|pane| block_on(pane.answer_approval(asked("claude-44x36"), "No".to_owned())));
    inputs_until(&mut agents.first.peer, |bytes, _| {
        without_escapes(bytes).contains(&b'4')
    });
    thread::sleep(ROUND_TRIP);
    assert!(answer.try_recv().is_err(), "still waiting for the dialog to go");
    assert!(typed(&mut agents.first.peer).is_empty(), "the key is not sent again");
    send(&agents.first.peer, b"\x1b[2J\x1b[H");
    assert_eq!(outcome(&answer), Ok(AgentInputOutcome::Sent));
}

#[test]
fn a_prompt_holding_a_draft_or_a_paste_that_never_shows_gets_no_enter() {
    let mut agents = Agents::start_timed("codex", "codex-44x36", expiring());
    agents.first_is("leader");
    // a draft typed at the terminal: nothing goes
    agents.show(&agents.first.peer, "idle");
    agents.show_typed(&agents.first.peer, Agent::Codex, "idle", "draft ");
    assert_eq!(
        block_on(agents.pane.send_prompt("new prompt".to_owned())),
        Ok(AgentInputOutcome::NotOnScreen)
    );
    assert!(typed(&mut agents.first.peer).is_empty());

    // the draft cleared (the placeholder back, faint), the paste goes, but the prompt never shows it: no Enter
    agents.show_in_prompt(
        &agents.first.peer,
        Agent::Codex,
        "idle",
        "2",
        "Ask Codex to do anything",
    );
    let prompt = agents.act(|pane| block_on(pane.send_prompt("new prompt".to_owned())));
    inputs_until(&mut agents.first.peer, |bytes, _| contains(bytes, b"\x1b[201~"));
    assert_eq!(outcome(&prompt), Ok(AgentInputOutcome::Unconfirmed));
    assert!(!typed(&mut agents.first.peer).contains(&b'\r'), "no Enter");
}

#[test]
fn stop_sends_escape_to_a_working_turn_and_nothing_to_an_idle_one() {
    let mut agents = Agents::start("codex", "codex-44x36");
    agents.first_is("leader");
    agents.show(&agents.first.peer, "idle");
    assert_eq!(block_on(agents.pane.interrupt()), Ok(AgentInputOutcome::NotOnScreen));
    assert!(typed(&mut agents.first.peer).is_empty());

    agents.show(&agents.first.peer, "working");
    let stop = agents.act(|pane| block_on(pane.interrupt()));
    // Codex runs under the Kitty keyboard protocol: an unambiguous Escape
    inputs_until(&mut agents.first.peer, |bytes, _| contains(bytes, b"\x1b[27u"));
    assert_eq!(outcome(&stop), Ok(AgentInputOutcome::Sent));
}

#[test]
fn a_prompt_with_a_dialog_open_sends_nothing() {
    let mut agents = Agents::start("claude", "claude-44x36");
    agents.first_is("leader");
    agents.show(&agents.first.peer, "question");
    assert_eq!(
        block_on(agents.pane.send_prompt("hello".to_owned())),
        Ok(AgentInputOutcome::DialogOpen)
    );
    assert!(typed(&mut agents.first.peer).is_empty());
}

#[test]
fn the_lead_lost_between_the_paste_and_its_enter_sends_no_enter() {
    let mut agents = Agents::start("claude", "claude-44x36");
    agents.first_is("leader");
    agents.show(&agents.first.peer, "idle");
    let prompt = agents.act(|pane| block_on(pane.send_prompt("hello".to_owned())));
    inputs_until(&mut agents.first.peer, |bytes, _| contains(bytes, b"\x1b[201~"));
    // another client claims as the paste lands
    let nonce = lead_of(&agents.first.command).nonce;
    send(&agents.first.peer, &role_title(&nonce, "follower", 2));
    assert_eq!(outcome(&prompt), Ok(AgentInputOutcome::Unconfirmed));
    let (bytes, others) = drained(&mut agents.first.peer);
    assert!(!without_escapes(&bytes).contains(&b'\r'), "no Enter");
    assert!(
        others.is_empty(),
        "the attach is neither resized nor closed: {others:?}"
    );
}

#[test]
fn a_handle_whose_pane_was_hidden_is_closed_even_once_the_pane_is_shown_again() {
    let mut agents = Agents::start("claude", "claude-44x36");
    agents.first_is("leader");
    agents.show(&agents.first.peer, "permission");
    agents
        .fixture
        .core
        .hide_pane(MACHINE.to_owned(), SESSION.to_owned(), PANE.to_owned())
        .unwrap();
    let shown_again = block_on(agents.fixture.core.show_pane(
        MACHINE.to_owned(),
        SESSION.to_owned(),
        PANE.to_owned(),
        grid(&agents.capture),
    ))
    .unwrap();
    let mut second = agents.fixture.attach();
    let nonce = lead_of(&second.command).nonce;
    feed_into(&shown_again, &second.peer, &role_title(&nonce, "leader", 1));
    feed_into(&shown_again, &second.peer, agents.capture.at("permission"));

    // the retired handle reaches nothing, though the pane it named is shown again with the dialog up
    assert_eq!(
        block_on(agents.pane.answer_approval(asked("claude-44x36"), "Yes".to_owned())),
        Err(PaneInputError::Closed)
    );
    assert_eq!(block_on(agents.pane.approval_labels()), Err(PaneInputError::Closed));
    assert!(typed(&mut second.peer).is_empty());

    // the handle the pane was shown again with answers it
    let answer = {
        let (done, outcome) = mpsc::channel();
        thread::spawn(move || {
            let _ = done.send(block_on(
                shown_again.answer_approval(asked("claude-44x36"), "Yes".to_owned()),
            ));
        });
        outcome
    };
    let (bytes, _) = inputs_until(&mut second.peer, |bytes, _| without_escapes(bytes).contains(&b'1'));
    assert_eq!(without_escapes(&bytes), b"1");
    send(&second.peer, b"\x1b[2J\x1b[H");
    assert_eq!(outcome(&answer), Ok(AgentInputOutcome::Sent));
}

#[test]
fn a_pane_with_no_agent_the_chat_can_answer_is_refused() {
    let mut agents = Agents::start("aider", "claude-44x36");
    agents.first_is("leader");
    assert_eq!(
        block_on(agents.pane.send_prompt("hi".to_owned())),
        Err(PaneInputError::Refused {
            reason: NO_AGENT_TO_DRIVE.to_owned()
        })
    );
    assert_eq!(block_on(agents.pane.approval_labels()), Ok(Vec::new()));
    assert!(!agents.fixture.attaches_soon(), "no claim");
}

#[test]
fn reading_the_chat_and_switching_it_with_the_terminal_never_claim_or_resize() {
    let mut agents = Agents::start("claude", "claude-44x36");
    agents.first_is("follower");
    agents.show(&agents.first.peer, "permission");
    let open = || {
        block_on(
            agents
                .fixture
                .core
                .open_chat(MACHINE.to_owned(), SESSION.to_owned(), PANE.to_owned()),
        )
    };
    open().unwrap();
    // the dialog as the follower's screen shows it, once the terminal took it
    let mut labels = Vec::new();
    for _ in 0..200 {
        labels = block_on(agents.pane.approval_labels()).unwrap();
        if labels.len() == 4 {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(labels.len(), 4);
    assert_eq!((labels[0].as_str(), labels[3].as_str()), ("Yes", "No"));
    agents
        .fixture
        .transport
        .script(&commands_tail(SESSION, Side::Left), Script::ok("[]"));
    let commands = block_on(agents.fixture.core.agent_commands(
        MACHINE.to_owned(),
        SESSION.to_owned(),
        PANE.to_owned(),
    ));
    assert_eq!(commands, Ok(Vec::new()));
    agents
        .fixture
        .core
        .close_chat(MACHINE.to_owned(), SESSION.to_owned(), PANE.to_owned())
        .unwrap();
    open().unwrap();
    assert!(!agents.fixture.attaches_soon(), "no claim, no attach again");
    let mut others = Vec::new();
    while let Some(input) = agents.first.peer.try_input() {
        if !matches!(input, ChannelInput::Data(_)) {
            others.push(input);
        }
    }
    assert!(
        others.is_empty(),
        "the pane's attach is neither resized nor closed: {others:?}"
    );
    assert_eq!(
        agents.pane.size(),
        grid(&agents.capture),
        "the grid the terminal view gave"
    );
    drop(agents.stream);
}

#[test]
fn approval_labels_fall_back_to_the_agents_own_when_the_screen_has_none() {
    let mut agents = Agents::start("codex", "codex-44x36");
    agents.first_is("follower");
    agents.show(&agents.first.peer, "idle");
    assert_eq!(
        block_on(agents.pane.approval_labels()),
        Ok(vec![
            "Yes, proceed".to_owned(),
            "No, and tell Codex what to do differently".to_owned()
        ])
    );
}

/// The program's screen and what the terminal view shows apart: an old, complete copy of one state in the history,
/// another drawn live below it, and the view scrolled back the way a swipe in the terminal view scrolls it.
impl Agents {
    /// Feeds `peer` the capture's `old` state, pushes its screen whole into the history, then draws the capture's
    /// `live` state from a clean screen below it; waits until the program's screen reads as `is_live`. The recorded
    /// TUIs draw on the alternate screen with mouse reporting on, where the view neither keeps a history nor scrolls
    /// on a swipe; both run inline as well, which this replays: the same draws on the primary screen, the mouse left
    /// to the view.
    fn over_history(&self, peer: &ChannelPeer, agent: Agent, (old, live): (&str, &str), is_live: fn(&Screen) -> bool) {
        let mut bytes = inline(self.capture.at(old));
        bytes.extend(format!("\x1b[0m\x1b[{};1H", self.capture.rows).as_bytes());
        bytes.extend(std::iter::repeat_n(b'\n', usize::from(self.capture.rows)));
        bytes.extend(b"\x1b[H");
        bytes.extend(inline(self.capture.at(live)));
        send(peer, &bytes);
        self.reads(agent, is_live, live);
    }

    /// Waits until the program's screen reads as `is_state`.
    fn reads(&self, agent: Agent, is_state: fn(&Screen) -> bool, state: &str) {
        let terminal = self.pane.terminal().expect("the pane's terminal");
        for _ in 0..1000 {
            if let Some(screen) = terminal.active_screen().unwrap()
                && is_state(&agent.read(&screen.frame))
            {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("the program's screen never showed {state}");
    }

    /// Scrolls the terminal view back a row at a time through the phone's wheel until it shows a screen `agent` reads
    /// as `is_old`, then checks the program's own screen still reads as `is_live`.
    fn scroll_back_to(&self, agent: Agent, is_old: fn(&Screen) -> bool, is_live: fn(&Screen) -> bool) {
        let terminal = self.pane.terminal().expect("the pane's terminal");
        let up = PaneWheel {
            rows: 1,
            columns: 0,
            mods: 0,
            x: 0.0,
            y: 0.0,
        };
        let mut found = false;
        for _ in 0..usize::from(self.capture.rows) * 4 {
            assert_eq!(self.pane.wheel(up), Ok(PaneWheelOutcome::Scrolled), "the view scrolls");
            if is_old(&agent.read(&terminal.frame().frame)) {
                found = true;
                break;
            }
        }
        assert!(found, "the view never showed the old copy");
        let live = terminal.active_screen().unwrap().expect("not held");
        assert!(
            is_live(&agent.read(&live.frame)),
            "the program's screen is the live one"
        );
    }
}

/// `bytes` with the alternate screen and mouse reporting never turned on.
fn inline(bytes: &[u8]) -> Vec<u8> {
    const LEFT_OUT: [&[u8]; 5] = [
        b"\x1b[?1049h",
        b"\x1b[?1000h",
        b"\x1b[?1002h",
        b"\x1b[?1003h",
        b"\x1b[?1006h",
    ];
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if let Some(mode) = LEFT_OUT.iter().find(|mode| bytes[at..].starts_with(mode)) {
            at += mode.len();
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    out
}

fn is_dialog(screen: &Screen) -> bool {
    matches!(screen, Screen::Permission(_))
}

fn is_idle(screen: &Screen) -> bool {
    matches!(screen, Screen::Prompt { working: false, composer } if composer.is_empty())
}

fn is_working(screen: &Screen) -> bool {
    matches!(screen, Screen::Prompt { working: true, .. })
}

fn is_question(screen: &Screen) -> bool {
    matches!(screen, Screen::Question(_))
}

/// One state of a capture, and how its screen reads.
type State = (&'static str, fn(&Screen) -> bool);

/// A leader showing the capture's `live` state under an old copy of its `old` state, the view scrolled back onto it.
fn scrolled(agent: Agent, capture: &str, old: State, live: State) -> Agents {
    let name = match agent {
        Agent::Claude => "claude",
        Agent::Codex => "codex",
    };
    let mut agents = Agents::start(name, capture);
    agents.first_is("leader");
    agents.over_history(&agents.first.peer, agent, (old.0, live.0), live.1);
    agents.scroll_back_to(agent, old.1, live.1);
    agents
}

#[test]
fn an_approval_answers_the_live_dialog_never_one_scrolled_back_to() {
    // an old dialog in view, the prompt live: nothing goes, and the card's labels are the agent's own
    for (agent, capture, answer) in [
        (Agent::Claude, "claude-44x36", "No"),
        (Agent::Codex, "codex-44x36", "Yes, proceed"),
    ] {
        let mut agents = scrolled(agent, capture, ("permission", is_dialog), ("idle", is_idle));
        assert_eq!(block_on(agents.pane.approval_labels()), Ok(agent.fallback_labels()));
        assert_eq!(
            block_on(agents.pane.answer_approval(asked(capture), answer.to_owned())),
            Ok(AgentInputOutcome::NotOnScreen),
            "{capture}"
        );
        assert!(
            typed(&mut agents.first.peer).is_empty(),
            "no key into the prompt: {capture}"
        );
    }

    // the dialog live, an old prompt in view: the dialog's own labels, and its key goes
    let mut agents = scrolled(
        Agent::Codex,
        "codex-44x36",
        ("idle", is_idle),
        ("permission", is_dialog),
    );
    let labels = block_on(agents.pane.approval_labels()).unwrap();
    assert_ne!(labels, Agent::Codex.fallback_labels(), "read from the live dialog");
    assert_eq!(labels[0], "Yes, proceed");
    let answer = agents.act(|pane| block_on(pane.answer_approval(asked("codex-44x36"), "Yes, proceed".to_owned())));
    let (bytes, _) = inputs_until(&mut agents.first.peer, |bytes, _| {
        without_escapes(bytes).contains(&b'y')
    });
    assert_eq!(without_escapes(&bytes), b"y");
    send(&agents.first.peer, b"\x1b[2J\x1b[H");
    assert_eq!(outcome(&answer), Ok(AgentInputOutcome::Sent));

    let mut agents = scrolled(
        Agent::Claude,
        "claude-44x36",
        ("idle", is_idle),
        ("permission", is_dialog),
    );
    let answer = agents.act(|pane| block_on(pane.answer_approval(asked("claude-44x36"), "No".to_owned())));
    let (bytes, _) = inputs_until(&mut agents.first.peer, |bytes, _| {
        without_escapes(bytes).contains(&b'4')
    });
    assert_eq!(without_escapes(&bytes), b"4");
    send(&agents.first.peer, b"\x1b[2J\x1b[H");
    assert_eq!(outcome(&answer), Ok(AgentInputOutcome::Sent));
}

#[test]
fn a_prompt_goes_to_the_live_prompt_never_one_scrolled_back_to() {
    // an old prompt in view, a dialog live: nothing is pasted into the dialog
    let mut agents = scrolled(
        Agent::Claude,
        "claude-44x36",
        ("idle", is_idle),
        ("permission", is_dialog),
    );
    assert_eq!(
        block_on(agents.pane.send_prompt("hello".to_owned())),
        Ok(AgentInputOutcome::DialogOpen)
    );
    assert!(typed(&mut agents.first.peer).is_empty());

    // an old dialog in view, the prompt live: the paste goes, then its Enter once the prompt shows it
    let mut agents = scrolled(
        Agent::Codex,
        "codex-44x36",
        ("permission", is_dialog),
        ("idle", is_idle),
    );
    let prompt = agents.act(|pane| block_on(pane.send_prompt("hello".to_owned())));
    let (pasted, _) = inputs_until(&mut agents.first.peer, |bytes, _| contains(bytes, b"\x1b[201~"));
    assert!(pasted.ends_with(b"\x1b[200~hello\x1b[201~"));
    agents.show_typed(&agents.first.peer, Agent::Codex, "idle", "hello");
    let (entered, _) = inputs_until(&mut agents.first.peer, |bytes, _| bytes.ends_with(b"\r"));
    assert_eq!(without_escapes(&entered), b"\r");
    assert_eq!(outcome(&prompt), Ok(AgentInputOutcome::Sent));
}

#[test]
fn stop_goes_to_the_live_turn_never_one_scrolled_back_to() {
    // an old working turn in view, the prompt idle: no Escape
    let mut agents = scrolled(Agent::Codex, "codex-44x36", ("working", is_working), ("idle", is_idle));
    assert_eq!(block_on(agents.pane.interrupt()), Ok(AgentInputOutcome::NotOnScreen));
    assert!(typed(&mut agents.first.peer).is_empty());

    // an old idle prompt in view, a turn working: the Escape goes
    let mut agents = scrolled(Agent::Codex, "codex-44x36", ("idle", is_idle), ("working", is_working));
    let stop = agents.act(|pane| block_on(pane.interrupt()));
    inputs_until(&mut agents.first.peer, |bytes, _| contains(bytes, b"\x1b[27u"));
    assert_eq!(outcome(&stop), Ok(AgentInputOutcome::Sent));
}

#[test]
fn a_question_is_answered_on_the_live_picker_never_one_scrolled_back_to() {
    let question = ChatQuestion {
        questions: vec![ChatQuestionItem {
            question: "Pick a color".to_owned(),
            header: None,
            multi_select: false,
            options: ["Red", "Green"]
                .into_iter()
                .map(|label| ChatQuestionOption {
                    label: label.to_owned(),
                    description: Some(label.to_owned()),
                })
                .collect(),
        }],
        status: ChatToolStatus::Running,
        output: None,
    };
    let green = || {
        vec![QuestionAnswer {
            picked: vec!["Green".to_owned()],
            other: None,
        }]
    };
    // an old picker in view, the prompt live: no digit
    let mut agents = scrolled(
        Agent::Claude,
        "claude-44x36",
        ("question", is_question),
        ("idle", is_idle),
    );
    assert_eq!(
        block_on(agents.pane.answer_question(question.clone(), green())),
        Ok(AgentInputOutcome::NotOnScreen)
    );
    assert!(typed(&mut agents.first.peer).is_empty());

    // an old prompt in view, the picker live: Green's digit goes, once
    let mut agents = scrolled(
        Agent::Claude,
        "claude-44x36",
        ("idle", is_idle),
        ("question", is_question),
    );
    let answer = agents.act(move |pane| block_on(pane.answer_question(question, green())));
    let (bytes, _) = inputs_until(&mut agents.first.peer, |bytes, _| {
        without_escapes(bytes).contains(&b'2')
    });
    assert_eq!(without_escapes(&bytes), b"2");
    // the picker goes, the turn goes on
    let mut working = b"\x1b[0m\x1b[H\x1b[2J".to_vec();
    working.extend(agents.capture.at("working"));
    send(&agents.first.peer, &working);
    assert_eq!(outcome(&answer), Ok(AgentInputOutcome::Sent));
    assert!(typed(&mut agents.first.peer).is_empty(), "nothing more");
}

#[test]
fn a_screen_resized_and_not_redrawn_yet_gets_no_key() {
    let mut agents = Agents::start("claude", "claude-44x36");
    agents.first_is("leader");
    agents.show(&agents.first.peer, "permission");
    agents.reads(Agent::Claude, is_dialog, "permission");
    // the view gives a new grid: the terminal reflows the dialog, the program has not drawn for it yet
    let tablet = Capture::load("claude-100x40");
    agents.pane.resize(grid(&tablet)).unwrap();
    assert_eq!(
        block_on(agents.pane.answer_approval(asked("claude-44x36"), "No".to_owned())),
        Ok(AgentInputOutcome::NotOnScreen)
    );
    assert!(typed(&mut agents.first.peer).is_empty());

    // the program draws the dialog for the new grid: its key goes
    let mut redrawn = b"\x1b[0m\x1b[H\x1b[2J".to_vec();
    redrawn.extend(tablet.at("permission"));
    send(&agents.first.peer, &redrawn);
    let answer = agents.act(|pane| block_on(pane.answer_approval(asked("claude-44x36"), "No".to_owned())));
    let (bytes, _) = inputs_until(&mut agents.first.peer, |bytes, _| {
        without_escapes(bytes).contains(&b'4')
    });
    assert_eq!(without_escapes(&bytes), b"4");
    send(&agents.first.peer, b"\x1b[2J\x1b[H");
    assert_eq!(outcome(&answer), Ok(AgentInputOutcome::Sent));

    // inline, a grid two rows shorter: the reflow still shows the whole dialog, and still gets no key
    let mut agents = Agents::start("claude", "claude-44x36");
    agents.first_is("leader");
    send(&agents.first.peer, &inline(agents.capture.at("permission")));
    agents.reads(Agent::Claude, is_dialog, "permission");
    let shorter = PaneSize {
        rows: agents.capture.rows - 2,
        ..grid(&agents.capture)
    };
    agents.pane.resize(shorter).unwrap();
    let terminal = agents.pane.terminal().unwrap();
    let reflowed = terminal.active_screen().unwrap().unwrap();
    assert!(reflowed.reflowed);
    assert!(
        is_dialog(&Agent::Claude.read(&reflowed.frame)),
        "the reflow reads as the dialog"
    );
    assert_eq!(
        block_on(agents.pane.answer_approval(asked("claude-44x36"), "No".to_owned())),
        Ok(AgentInputOutcome::NotOnScreen)
    );
    assert!(typed(&mut agents.first.peer).is_empty());
    // the program writes again: its key goes
    send(&agents.first.peer, b"\x1b[0m");
    let answer = agents.act(|pane| block_on(pane.answer_approval(asked("claude-44x36"), "No".to_owned())));
    let (bytes, _) = inputs_until(&mut agents.first.peer, |bytes, _| {
        without_escapes(bytes).contains(&b'4')
    });
    assert_eq!(without_escapes(&bytes), b"4");
    send(&agents.first.peer, b"\x1b[2J\x1b[H");
    assert_eq!(outcome(&answer), Ok(AgentInputOutcome::Sent));
}

/// The grid the view measured again, or with only its cells' pixel size new: the pane's columns and rows as they are.
fn unchanged_grids(capture: &Capture) -> [PaneSize; 2] {
    let same = grid(capture);
    [
        same,
        PaneSize {
            cell_width: same.cell_width + 3,
            cell_height: same.cell_height + 5,
            ..same
        },
    ]
}

#[test]
fn a_grid_measured_again_or_new_cells_alone_leave_an_idle_screen_readable() {
    for (at, size) in unchanged_grids(&Capture::load("claude-44x36")).into_iter().enumerate() {
        // the dialog is drawn, then the view measures its grid (a new terminal view does, once it lays out); the
        // program, its pty unchanged, prints nothing more
        let mut agents = Agents::start("claude", "claude-44x36");
        agents.first_is("leader");
        agents.show(&agents.first.peer, "permission");
        agents.reads(Agent::Claude, is_dialog, "permission");
        agents.pane.resize(size).unwrap();
        agents.pane.resize(size).unwrap();
        let answer = agents.act(|pane| block_on(pane.answer_approval(asked("claude-44x36"), "No".to_owned())));
        let (bytes, _) = inputs_until(&mut agents.first.peer, |bytes, _| {
            without_escapes(bytes).contains(&b'4')
        });
        assert_eq!(without_escapes(&bytes), b"4", "grid {at}: the dialog's key goes");
        send(&agents.first.peer, b"\x1b[2J\x1b[H");
        assert_eq!(outcome(&answer), Ok(AgentInputOutcome::Sent));

        // the idle prompt likewise: pasted, then entered once shown
        let mut agents = Agents::start("codex", "codex-44x36");
        agents.first_is("leader");
        agents.show(&agents.first.peer, "idle");
        agents.reads(Agent::Codex, is_idle, "idle");
        agents.pane.resize(size).unwrap();
        let prompt = agents.act(|pane| block_on(pane.send_prompt("hello".to_owned())));
        let (pasted, _) = inputs_until(&mut agents.first.peer, |bytes, _| contains(bytes, b"\x1b[201~"));
        assert!(
            pasted.ends_with(b"\x1b[200~hello\x1b[201~"),
            "grid {at}: the prompt is pasted: {:?}",
            String::from_utf8_lossy(&pasted)
        );
        agents.show_typed(&agents.first.peer, Agent::Codex, "idle", "hello");
        inputs_until(&mut agents.first.peer, |bytes, _| bytes.ends_with(b"\r"));
        assert_eq!(outcome(&prompt), Ok(AgentInputOutcome::Sent));
    }

    // a new column count alone still waits for the program to draw for it
    let mut agents = Agents::start("claude", "claude-44x36");
    agents.first_is("leader");
    agents.show(&agents.first.peer, "permission");
    agents.reads(Agent::Claude, is_dialog, "permission");
    let wider = PaneSize {
        cols: agents.capture.cols + 4,
        ..grid(&agents.capture)
    };
    agents.pane.resize(wider).unwrap();
    assert_eq!(
        block_on(agents.pane.answer_approval(asked("claude-44x36"), "No".to_owned())),
        Ok(AgentInputOutcome::NotOnScreen)
    );
    assert!(typed(&mut agents.first.peer).is_empty());
}
