//! The chat's actions through the whole core, over the fake transport's channels: a pane's attach fed a recorded
//! agent screen, its role reports, and the keys that come back down the channel. Leadership first (a follower claims,
//! a leader doesn't, a claim refused or unanswered sends nothing, a dialog gone by the time the claim lands sends
//! nothing), then each action's keys, the lead lost between a paste and its Enter, and reading the chat or switching
//! it with the terminal never claiming or resizing.

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
use crate::api::{MobileEvent, Pane, PaneInputError, PaneSize};
use crate::chat::input::{ACTION_IN_FLIGHT, AgentInputOutcome, InputTiming, NO_AGENT_TO_DRIVE};
use crate::chat::tests::matchers::Capture;
use crate::chat::{Side, commands_tail};
use crate::pane::{PaneRole, without_escapes};
use crate::transport::fake::{Opened, Script};
use crate::transport::{ChannelInput, ChannelPeer};

/// Short waits, so a refused claim times out quickly.
fn timing() -> InputTiming {
    InputTiming {
        quiet: Duration::from_millis(150),
        settle_max: Duration::from_secs(1),
        lead_timeout: Duration::from_millis(800),
        poll: Duration::from_millis(10),
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
        let mut fixture = Fixture::connected_timed(vec![listing(true)], timing());
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

    /// Feeds `peer` the capture up to `state`.
    fn show(&self, peer: &ChannelPeer, state: &str) {
        send(peer, self.capture.at(state));
    }
}

fn outcome(
    receiver: &Receiver<Result<AgentInputOutcome, PaneInputError>>,
) -> Result<AgentInputOutcome, PaneInputError> {
    receiver
        .recv_timeout(Duration::from_secs(20))
        .expect("the action answers")
}

/// The keys typed down `peer` within a moment: what the core sent, the terminal's replies to the program's queries
/// taken out.
fn typed(peer: &mut ChannelPeer) -> Vec<u8> {
    thread::sleep(Duration::from_millis(300));
    let mut bytes = Vec::new();
    while let Some(input) = peer.try_input() {
        if let ChannelInput::Data(data) = input {
            bytes.extend(data);
        }
    }
    without_escapes(&bytes)
}

/// The claiming attach the action opened, reported leader, showing the capture's `state`.
fn claimed(agents: &mut Agents, state: &str) -> Opened {
    let claim = agents.fixture.attach();
    let lead = lead_of(&claim.command);
    assert!(lead.claim, "the action claims the lead");
    send(&claim.peer, &role_title(&lead.nonce, "leader", 2));
    agents.show(&claim.peer, state);
    claim
}

#[test]
fn a_follower_claims_the_lead_waits_for_the_redraw_then_answers() {
    let mut agents = Agents::start("claude", "claude-44x36");
    agents.first_is("follower");
    agents.show(&agents.first.peer, "permission");
    let answer = agents.act(|pane| block_on(pane.answer_approval("No".to_owned())));

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
    let answer = agents.act(|pane| block_on(pane.answer_approval("Yes, proceed".to_owned())));
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
    let mut agents = Agents::start("claude", "claude-44x36");
    agents.first_is("follower");
    let answer = agents.act(|pane| block_on(pane.answer_approval("Yes".to_owned())));
    let mut claim = agents.fixture.attach();
    let lead = lead_of(&claim.command);
    agents.show(&claim.peer, "permission");
    // another client claimed meanwhile: this attach follows
    send(&claim.peer, &role_title(&lead.nonce, "follower", 2));
    assert_eq!(outcome(&answer), Ok(AgentInputOutcome::LeadRefused));
    assert!(typed(&mut claim.peer).is_empty());

    // a claim whose attach never reports
    let answer = agents.act(|pane| block_on(pane.answer_approval("Yes".to_owned())));
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
    let answer = agents.act(|pane| block_on(pane.answer_approval("Yes".to_owned())));
    // answered at the desktop while the claim was on its way: the leader's grid shows the prompt again
    let mut claim = claimed(&mut agents, "idle");
    assert_eq!(outcome(&answer), Ok(AgentInputOutcome::NotOnScreen));
    assert!(typed(&mut claim.peer).is_empty());
}

#[test]
fn a_prompt_is_pasted_then_entered_and_a_second_action_waits_its_turn() {
    let mut agents = Agents::start("claude", "claude-44x36");
    agents.first_is("leader");
    agents.show(&agents.first.peer, "idle");
    let prompt = agents.act(|pane| block_on(pane.send_prompt("/compact keep it short".to_owned())));
    thread::sleep(Duration::from_millis(50));
    assert_eq!(
        block_on(agents.pane.interrupt()),
        Err(PaneInputError::Refused {
            reason: ACTION_IN_FLIGHT.to_owned()
        })
    );
    let (bytes, _) = inputs_until(&mut agents.first.peer, |bytes, _| bytes.ends_with(b"\r"));
    assert!(
        bytes.ends_with(b"\x1b[200~/compact keep it short\x1b[201~\r"),
        "a bracketed paste of the whole command, then Enter: {:?}",
        String::from_utf8_lossy(&bytes)
    );
    assert_eq!(outcome(&prompt), Ok(AgentInputOutcome::Sent));
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
    let (_, others) = inputs_until(&mut agents.first.peer, |_, _| true);
    assert!(others.is_empty());
    assert!(!typed(&mut agents.first.peer).contains(&b'\r'), "no Enter");
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
