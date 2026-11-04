//! The e2e test's agent chat steps: a fake agent ([`FakeAgent`]) in the pane, the phone's chat of it read, followed,
//! paged and resumed, a prompt sent while another client leads, an approval, and the agent's end.

use std::collections::HashSet;

use ngterm_mobile::{AgentInputOutcome, ChatBody, ChatEvent, ChatState, MachineState, PaneRole, PaneSize};

use crate::agent::{APPROVAL_COMMAND, CONVERSATION, FIRST_PROMPT, FakeAgent};
use crate::machine::World;
use crate::phone::{Client, MACHINE_ID, block_on, enter};

/// The grids of Task 1's captures: the phone's portrait and a tablet's. The fake agent draws Claude's recorded idle
/// screen at either.
const PHONE: PaneSize = PaneSize {
    cols: 44,
    rows: 36,
    cell_width: 10,
    cell_height: 20,
};
const TABLET: PaneSize = PaneSize {
    cols: 100,
    rows: 40,
    cell_width: 10,
    cell_height: 20,
};
/// The batches the test appends to the transcript: while the chat follows, and while its connection is down.
const FOLLOWED: u64 = 2;
const RESUMED: u64 = 3;
/// The prompt the phone sends.
const PROMPT: &str = "hello from the phone";
/// What the fake agent's footer says, as the capture drew it.
const FOOTER: &str = "for shortcuts";

/// Where the chat steps run: the session's pane and the two clients of it.
pub(crate) struct Scene<'a> {
    pub(crate) world: &'a World,
    pub(crate) first: &'a mut Client,
    pub(crate) second: &'a mut Client,
    pub(crate) session: &'a str,
    pub(crate) pane: &'a str,
}

impl Scene<'_> {
    /// The fake agent's whole run: `first` is the phone with the chat, `second` leads the pane until the phone acts.
    pub(crate) fn agent_chat(&mut self) {
        let agent = FakeAgent::prepare(self.world, self.session, self.pane);
        self.start(&agent);
        self.open();
        let followed = agent.append(FOLLOWED, 2);
        self.first.wait("the appended turns", |seen| seen.chat_holds(&followed));
        self.page_back(&agent);
        self.resume(&agent);
        self.prompt();
        self.approval(&agent);
        self.end();
    }

    fn act(&mut self, text: &str) -> AgentInputOutcome {
        let pane = self.first.terminal.clone().expect("the phone's pane");
        block_on(pane.send_prompt(text.to_owned())).unwrap_or_else(|error| panic!("send {text:?}: {error:?}"))
    }

    /// `second` leads at the tablet's grid and starts the fake agent in the pane's shell; `first` follows at the
    /// phone's. The origin announces the agent to the phone.
    fn start(&mut self, agent: &FakeAgent) {
        self.second
            .core
            .take_lead(MACHINE_ID.to_owned(), self.session.to_owned(), self.pane.to_owned())
            .expect("take the lead");
        self.second
            .wait("the second client leading", |seen| seen.role == Some(PaneRole::Leader));
        let leader = self.second.terminal.clone().expect("the second client's pane");
        leader.resize(TABLET).expect("the tablet's grid");
        self.first
            .terminal
            .as_ref()
            .expect("the phone's pane")
            .resize(PHONE)
            .expect("the phone's grid");
        assert!(leader.type_text(agent.command_line()).expect("type"));
        assert!(leader.key(enter()).expect("press enter"));
        self.second.wait_screen(FOOTER);
        let pane = self.pane.to_owned();
        self.first.wait("the agent announced", |seen| {
            seen.agents.last() == Some(&(pane.clone(), Some("claude".to_owned()), Some(CONVERSATION.to_owned())))
        });
    }

    /// The chat opens on the transcript's last events, then `live`.
    fn open(&mut self) {
        block_on(
            self.first
                .core
                .open_chat(MACHINE_ID.to_owned(), self.session.to_owned(), self.pane.to_owned()),
        )
        .expect("open the chat");
        self.first.wait("the backlog", |seen| {
            seen.chat.iter().any(|event| event.body == ChatBody::Live)
        });
        let chat = &self.first.seen.chat;
        let backlog = &chat[..chat
            .iter()
            .position(|event| event.body == ChatBody::Live)
            .unwrap_or_default()];
        assert!(
            backlog.iter().any(|event| matches!(&event.body,
                ChatBody::Conversation { agent, conversation_id, .. } if agent == "claude" && conversation_id == CONVERSATION)),
            "the backlog names no conversation"
        );
        let ids: HashSet<&str> = backlog.iter().map(|event| event.id.as_str()).collect();
        assert!(
            !ids.contains(FIRST_PROMPT),
            "the backlog reached the transcript's start"
        );
        assert!(backlog.len() > 100, "a backlog of {} events", backlog.len());
    }

    /// Older pages, until the transcript's start: each before the last, every generated turn read once.
    fn page_back(&mut self, agent: &FakeAgent) {
        let mut oldest = oldest_cursor(&self.first.seen.chat).expect("events");
        let mut read: Vec<ChatEvent> = Vec::new();
        for _ in 0..10 {
            let page = block_on(self.first.core.load_older(
                MACHINE_ID.to_owned(),
                self.session.to_owned(),
                self.pane.to_owned(),
            ))
            .expect("an older page");
            if page.is_empty() {
                break;
            }
            let page_oldest = oldest_cursor(&page).expect("a page of the main transcript");
            assert!(
                page.iter().all(|event| event.cursor < oldest),
                "a page reaches past the oldest event held ({oldest})"
            );
            oldest = page_oldest;
            read.extend(page);
        }
        let paged: HashSet<&str> = read.iter().map(|event| event.id.as_str()).collect();
        assert!(paged.contains(FIRST_PROMPT), "paging never reached the start");
        let streamed: HashSet<&str> = self.first.seen.chat.iter().map(|event| event.id.as_str()).collect();
        for id in &agent.filler {
            assert!(
                paged.contains(id.as_str()) != streamed.contains(id.as_str()),
                "the generated event {id} was read {} times",
                usize::from(paged.contains(id.as_str())) + usize::from(streamed.contains(id.as_str()))
            );
        }
    }

    /// The phone's link drops; what the agent writes meanwhile arrives once it is back, and nothing twice.
    fn resume(&mut self, agent: &FakeAgent) {
        let (attached, states) = (self.first.seen.attached, self.first.seen.chat_states.len());
        self.first.seen.states.clear();
        self.world.proxy.sever();
        self.first.wait("the chat reconnecting", |seen| {
            seen.chat_states[states..].contains(&ChatState::Reconnecting)
        });
        let resumed = agent.append(RESUMED, 2);
        // connected after a reconnect: a `Connected` read before the link went says nothing
        self.first.wait("the machine connected again", |seen| {
            seen.states
                .iter()
                .position(|state| matches!(state, MachineState::Reconnecting { .. }))
                .is_some_and(|lost| seen.states[lost..].contains(&MachineState::Connected))
        });
        self.first
            .wait("the pane attached again", |seen| seen.attached > attached);
        self.first.wait("the resumed chat", |seen| seen.chat_holds(&resumed));
        let mut delivered = HashSet::new();
        for event in self.first.seen.chat.iter().filter(|event| event.body != ChatBody::Live) {
            assert!(
                delivered.insert((event.id.as_str(), event.parent.as_deref(), format!("{:?}", event.body))),
                "delivered twice: {event:?}"
            );
        }
    }

    /// While `second` leads, the phone's prompt takes the lead, lays the agent out for the phone, pastes and sends:
    /// the agent shows it and writes it to the transcript, which the chat reads.
    fn prompt(&mut self) {
        assert_eq!(self.act(PROMPT), AgentInputOutcome::Sent);
        self.first
            .wait("the phone leading", |seen| seen.role == Some(PaneRole::Leader));
        self.second.wait("the second client following", |seen| {
            seen.role == Some(PaneRole::Follower)
        });
        self.first.wait_screen(&format!("> {PROMPT}"));
        self.first.wait("the prompt in the chat", |seen| {
            seen.chat
                .iter()
                .any(|event| matches!(&event.body, ChatBody::User { text, .. } if text == PROMPT))
        });
    }

    /// A blocked status with a permission candidate is an approval in the chat; the status leaving `blocked` ends it.
    fn approval(&mut self, agent: &FakeAgent) {
        let pane_arguments = ["--target", self.session, "--pane", "primary", "--pane-id", self.pane];
        let hook = [&["agent", "hook", "claude", "PermissionRequest"][..], &pane_arguments].concat();
        let payload = agent.permission_payload();
        self.world
            .headless
            .ctl_fed(&hook, Some(payload.as_bytes()))
            .expect("report the permission prompt");
        let status = |state: &'static str| [&["session", "status", state][..], &pane_arguments].concat();
        self.world.headless.ctl(&status("blocked")).expect("set blocked");
        let shown = |event: &ChatEvent| {
            matches!(&event.body, ChatBody::Approval { approval }
                if approval.tool == "Bash" && approval.detail.as_deref() == Some(APPROVAL_COMMAND))
        };
        self.first.wait("the approval", |seen| seen.chat.iter().any(shown));
        let id = self
            .first
            .seen
            .chat
            .iter()
            .find(|event| shown(event))
            .expect("the approval")
            .id
            .clone();
        self.world.headless.ctl(&status("active")).expect("set active");
        self.first.wait("the approval done", |seen| {
            seen.chat
                .iter()
                .any(|event| event.id == id && matches!(event.body, ChatBody::ApprovalDone { .. }))
        });
    }

    /// The agent exits, leaving the shell in the foreground: the origin clears the record, the phone hears the pane
    /// runs no agent and the chat ends.
    fn end(&mut self) {
        assert_eq!(self.act("/exit"), AgentInputOutcome::Sent);
        let pane = self.pane.to_owned();
        self.first.wait("the agent cleared", |seen| {
            seen.agents.last() == Some(&(pane.clone(), None, None))
        });
        self.first
            .wait("the chat ended", |seen| seen.chat_states.contains(&ChatState::Ended));
    }
}

/// The oldest cursor of `events` in the main transcript (a subagent's are in its own file), as the core pages by.
fn oldest_cursor(events: &[ChatEvent]) -> Option<u64> {
    events
        .iter()
        .filter(|event| event.parent.is_none())
        .map(|event| event.cursor)
        .min()
}
