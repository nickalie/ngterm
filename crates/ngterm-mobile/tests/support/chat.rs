//! The e2e test's agent chat steps: a fake agent ([`FakeAgent`]) in the pane, the phone's chat of it read, followed,
//! paged and resumed, a prompt sent while another client leads, a prompt sent while the agent works (absorbed into its
//! turn), an approval, an approval answered while the phone is away, a question Claude Code has not written yet shown
//! and answered, and the agent's end. The Left arrow steps between them are in `left_arrow.rs`.

use std::collections::HashSet;

use ngterm_mobile::{
    AgentInputOutcome, ChatApproval, ChatBody, ChatEvent, ChatQuestion, ChatQuestionItem, ChatQuestionOption,
    ChatState, ChatToolStatus, MachineState, PaneRole, PaneSize, QuestionAnswer,
};
use serde_json::Value;

use crate::agent::{
    APPROVAL_COMMAND, AWAY_COMMAND, CODEX_COMMAND, CODEX_PICK, CODEX_QUESTION, CODEX_THREAD, CONVERSATION,
    FIRST_PROMPT, FakeAgent, FakeCodex, QUESTION_CALL, State, question_input,
};
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
/// The prompt the phone sends while the agent works.
const ABSORBED: &str = "and add a test for it";
/// What the fake agent's footer says, as the capture drew it.
pub(crate) const FOOTER: &str = "for shortcuts";
/// What the working screen's footer says.
pub(crate) const WORKING: &str = "esc to interrupt";
/// The option the phone picks in the question.
const PICKED: &str = "Green";

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
        self.left_arrow(&agent);
        self.absorbed(&agent);
        self.approval(&agent);
        self.approval_away(&agent);
        self.question(&agent);
        self.end();
        self.left_arrow_elsewhere();
        self.codex();
    }

    fn act(&mut self, text: &str) -> AgentInputOutcome {
        let pane = self.first.terminal.clone().expect("the phone's pane");
        block_on(pane.send_prompt(text.to_owned())).unwrap_or_else(|error| panic!("send {text:?}: {error:?}"))
    }

    /// `second` leads at the tablet's grid and starts the fake agent in the pane's shell; `first` follows at the
    /// phone's. The origin announces the agent to the phone.
    fn start(&mut self, agent: &FakeAgent) {
        // the second client leads since the leadership step; the takeover attaches it again all the same, and what is
        // typed waits for that attach to lead
        self.second.take_lead(self.session, self.pane);
        let leader = self.second.terminal.clone().expect("the second client's pane");
        leader.resize(TABLET).expect("the tablet's grid");
        self.first
            .terminal
            .as_ref()
            .expect("the phone's pane")
            .resize(PHONE)
            .expect("the phone's grid");
        self.first.drain();
        let agents = self.first.seen.agents.len();
        assert!(leader.type_text(agent.command_line()).expect("type"));
        assert!(leader.key(enter()).expect("press enter"));
        self.second.wait_screen(FOOTER);
        let announced = (
            self.pane.to_owned(),
            Some("claude".to_owned()),
            Some(CONVERSATION.to_owned()),
        );
        self.first.wait("the agent announced", |seen| {
            seen.agents[agents..].last() == Some(&announced)
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
        self.first.drain();
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
        self.first.drain();
        self.second.drain();
        let (first, second) = (self.first.seen.roles.len(), self.second.seen.roles.len());
        assert_eq!(self.act(PROMPT), AgentInputOutcome::Sent);
        self.first.wait("the phone leading", |seen| {
            seen.roles[first..].contains(&Some(PaneRole::Leader))
        });
        self.second.wait("the second client following", |seen| {
            seen.roles[second..].contains(&Some(PaneRole::Follower))
        });
        self.first.wait_screen(&format!("> {PROMPT}"));
        self.first.wait("the prompt in the chat", |seen| {
            seen.chat
                .iter()
                .any(|event| matches!(&event.body, ChatBody::User { text, .. } if text == PROMPT))
        });
    }

    /// The pane's arguments for `ngtermctl agent hook` and `session status`.
    fn pane_arguments(&self) -> [&str; 6] {
        ["--target", self.session, "--pane", "primary", "--pane-id", self.pane]
    }

    /// Reports `payload` through the agent's `PermissionRequest` hook and sets the pane `blocked`, as Claude Code's
    /// hooks do when its dialog opens.
    fn ask(&self, payload: &str) {
        self.ask_as("claude", payload);
    }

    fn ask_as(&self, agent: &str, payload: &str) {
        let hook = [
            &["agent", "hook", agent, "PermissionRequest"][..],
            &self.pane_arguments(),
        ]
        .concat();
        self.world
            .headless
            .ctl_fed(&hook, Some(payload.as_bytes()))
            .expect("report the permission prompt");
        self.status("blocked");
    }

    fn status(&self, state: &str) {
        let status = [&["session", "status", state][..], &self.pane_arguments()].concat();
        self.world.headless.ctl(&status).expect("set the status");
    }

    /// The approval for `command` the phone was shown, once it is.
    fn approval_card(&mut self, command: &str) -> (String, ChatApproval) {
        let card = |event: &ChatEvent| match &event.body {
            ChatBody::Approval { approval }
                if approval.tool == "Bash" && approval.detail.as_deref() == Some(command) =>
            {
                Some((event.id.clone(), approval.clone()))
            }
            _ => None,
        };
        self.first.wait("the approval", |seen| {
            seen.chat.iter().any(|event| card(event).is_some())
        });
        self.first.seen.chat.iter().find_map(card).expect("the approval")
    }

    fn wait_done(&mut self, id: &str) {
        self.first.wait("the approval done", |seen| {
            seen.chat
                .iter()
                .any(|event| event.id == id && matches!(event.body, ChatBody::ApprovalDone { .. }))
        });
    }

    /// A blocked status with a permission candidate is an approval in the chat; the status leaving `blocked` ends it.
    fn approval(&mut self, agent: &FakeAgent) {
        self.ask(&agent.permission_payload(APPROVAL_COMMAND));
        let (id, _) = self.approval_card(APPROVAL_COMMAND);
        self.status("active");
        self.wait_done(&id);
    }

    /// An approval the phone was shown and that was answered while its link was down is over once it is back: its
    /// card must not stay up, holding the composer's Send, with no dialog left to answer.
    fn approval_away(&mut self, agent: &FakeAgent) {
        self.ask(&agent.permission_payload(AWAY_COMMAND));
        let (id, _) = self.approval_card(AWAY_COMMAND);
        self.first.drain();
        let states = self.first.seen.chat_states.len();
        self.first.seen.states.clear();
        self.world.proxy.sever();
        self.first.wait("the chat reconnecting", |seen| {
            seen.chat_states[states..].contains(&ChatState::Reconnecting)
        });
        self.status("active");
        self.first.wait("the machine connected again", |seen| {
            seen.states
                .iter()
                .position(|state| matches!(state, MachineState::Reconnecting { .. }))
                .is_some_and(|lost| seen.states[lost..].contains(&MachineState::Connected))
        });
        self.wait_done(&id);
    }

    /// While the agent works, the phone's prompt goes into the running turn: the agent records it only as part of that
    /// turn, and the chat still shows it as the user's message.
    fn absorbed(&mut self, agent: &FakeAgent) {
        self.first.take_lead(self.session, self.pane);
        agent.show(State::Working);
        self.first.wait_screen(WORKING);
        assert_eq!(self.act(ABSORBED), AgentInputOutcome::Sent);
        self.first.wait("the absorbed prompt in the chat", |seen| {
            seen.chat
                .iter()
                .any(|event| matches!(&event.body, ChatBody::User { text, .. } if text == ABSORBED))
        });
        agent.show(State::Idle);
        self.first.wait_screen(FOOTER);
    }

    /// Claude Code's `AskUserQuestion` picker waits with its call not yet in the transcript: the question comes from its
    /// `PermissionRequest` as a card the phone answers on the picker; the call written with its answer is a question
    /// row, and the status leaving `blocked` ends the card.
    fn question(&mut self, agent: &FakeAgent) {
        self.first.take_lead(self.session, self.pane);
        agent.show(State::Question);
        let asked = asked(&question_input());
        self.first.wait_screen(&asked[0].question);
        self.ask(&agent.question_payload());
        let card = |event: &ChatEvent| match &event.body {
            ChatBody::Approval { approval } if approval.questions == asked => Some(event.id.clone()),
            _ => None,
        };
        self.first.wait("the question card", |seen| {
            seen.chat.iter().any(|event| card(event).is_some())
        });
        let id = self.first.seen.chat.iter().find_map(card).expect("the question card");
        let question = ChatQuestion {
            questions: asked,
            status: ChatToolStatus::Running,
            output: None,
        };
        let answers = vec![QuestionAnswer {
            picked: vec![PICKED.to_owned()],
            other: None,
        }];
        let pane = self.first.terminal.clone().expect("the phone's pane");
        let outcome = block_on(pane.answer_question(question, answers)).expect("answer the question");
        assert_eq!(outcome, AgentInputOutcome::Sent);
        self.first.wait("the answered question", |seen| {
            seen.chat.iter().any(|event| {
                event.id == QUESTION_CALL
                    && matches!(&event.body, ChatBody::Question { question }
                        if question.status == ChatToolStatus::Ok
                            && question.output.as_deref().is_some_and(|output| output.contains(PICKED)))
            })
        });
        self.status("active");
        self.wait_done(&id);
    }

    /// Codex in the pane: its approval answered from the phone, then its plan-mode question, shown from the rollout and
    /// answered on its picker; then it exits.
    fn codex(&mut self) {
        let codex = FakeCodex::prepare(self.world, self.session, self.pane);
        self.first.drain();
        let agents = self.first.seen.agents.len();
        let pane = self.first.terminal.clone().expect("the phone's pane");
        assert!(pane.type_text(codex.command_line()).expect("type"));
        assert!(pane.key(enter()).expect("press enter"));
        let announced = (
            self.pane.to_owned(),
            Some("codex".to_owned()),
            Some(CODEX_THREAD.to_owned()),
        );
        self.first.wait("Codex announced", |seen| {
            seen.agents[agents..].last() == Some(&announced)
        });
        self.first.drain();
        let lives = count_lives(&self.first.seen.chat);
        block_on(
            self.first
                .core
                .open_chat(MACHINE_ID.to_owned(), self.session.to_owned(), self.pane.to_owned()),
        )
        .expect("open Codex's chat");
        self.first
            .wait("Codex's backlog", |seen| count_lives(&seen.chat) > lives);

        codex.show(State::Permission);
        self.first.wait_screen("Would you like to run");
        self.ask_as("codex", &codex.permission_payload());
        let (id, card) = self.approval_card(CODEX_COMMAND);
        let labels = block_on(pane.approval_labels()).expect("the dialog's labels");
        assert!(labels.iter().any(|label| label == "Yes, proceed"), "{labels:?}");
        let outcome = block_on(pane.answer_approval(card, "Yes, proceed".to_owned())).expect("answer the approval");
        assert_eq!(outcome, AgentInputOutcome::Sent);
        self.first.wait("the approved command's result", |seen| {
            seen.chat.iter().any(|event| {
                matches!(&event.body, ChatBody::Tool { tool }
                    if tool.title == CODEX_COMMAND && tool.status == ChatToolStatus::Ok)
            })
        });
        self.status("active");
        self.wait_done(&id);

        codex.show(State::Question);
        let open = |event: &ChatEvent| match &event.body {
            ChatBody::Question { question }
                if question.status == ChatToolStatus::Running
                    && question
                        .questions
                        .first()
                        .is_some_and(|item| item.question == CODEX_QUESTION) =>
            {
                Some((event.id.clone(), question.clone()))
            }
            _ => None,
        };
        self.first.wait("Codex's question", |seen| {
            seen.chat.iter().any(|event| open(event).is_some())
        });
        let (id, question) = self.first.seen.chat.iter().find_map(open).expect("the question");
        let answers = vec![QuestionAnswer {
            picked: vec![CODEX_PICK.to_owned()],
            other: None,
        }];
        let outcome = block_on(pane.answer_question(question, answers)).expect("answer the question");
        assert_eq!(outcome, AgentInputOutcome::Sent);
        self.first.wait("Codex's answered question", |seen| {
            seen.chat.iter().any(|event| {
                event.id == id
                    && matches!(&event.body, ChatBody::Question { question }
                        if question.status == ChatToolStatus::Ok
                            && question.output.as_deref().is_some_and(|output| output.contains(CODEX_PICK)))
            })
        });

        self.first.drain();
        let (agents, states) = (self.first.seen.agents.len(), self.first.seen.chat_states.len());
        codex.show(State::Exit);
        let cleared = (self.pane.to_owned(), None, None);
        self.first
            .wait("Codex cleared", |seen| seen.agents[agents..].last() == Some(&cleared));
        self.first.wait("Codex's chat ended", |seen| {
            seen.chat_states[states..].contains(&ChatState::Ended)
        });
    }

    /// The agent exits, leaving the shell in the foreground: the origin clears the record, the phone hears the pane
    /// runs no agent and the chat ends.
    fn end(&mut self) {
        self.first.drain();
        let (agents, states) = (self.first.seen.agents.len(), self.first.seen.chat_states.len());
        assert_eq!(self.act("/exit"), AgentInputOutcome::Sent);
        let cleared = (self.pane.to_owned(), None, None);
        self.first.wait("the agent cleared", |seen| {
            seen.agents[agents..].last() == Some(&cleared)
        });
        self.first.wait("the chat ended", |seen| {
            seen.chat_states[states..].contains(&ChatState::Ended)
        });
    }
}

fn count_lives(events: &[ChatEvent]) -> usize {
    events.iter().filter(|event| event.body == ChatBody::Live).count()
}

/// The questions of an `AskUserQuestion` input, as the chat names them.
fn asked(input: &Value) -> Vec<ChatQuestionItem> {
    let text = |value: &Value| value.as_str().map(str::to_owned);
    input["questions"]
        .as_array()
        .expect("questions")
        .iter()
        .map(|item| ChatQuestionItem {
            question: text(&item["question"]).expect("a question"),
            header: text(&item["header"]),
            multi_select: item["multiSelect"].as_bool().unwrap_or_default(),
            options: item["options"]
                .as_array()
                .expect("options")
                .iter()
                .map(|option| ChatQuestionOption {
                    label: text(&option["label"]).expect("a label"),
                    description: text(&option["description"]),
                })
                .collect(),
        })
        .collect()
}

/// The oldest cursor of `events` in the main transcript (a subagent's are in its own file), as the core pages by.
fn oldest_cursor(events: &[ChatEvent]) -> Option<u64> {
    events
        .iter()
        .filter(|event| event.parent.is_none())
        .map(|event| event.cursor)
        .min()
}
