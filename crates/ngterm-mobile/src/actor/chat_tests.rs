//! The agent chat view through the actor, over the scripted fake transport on tokio's paused clock: the presentation
//! stream's agent frames, a shown pane's chat channel and its resumes, older pages, the catalog, and how a chat ends.

use std::time::Duration;

use ngterm_core::live::daemon_name;
use ngterm_core::remote::frames::{
    self, Body, PresentationAgent, PresentationFrame, PresentationHello, PresentationLayout, PresentationMode,
    PresentationPane, PresentationSnapshot,
};
use ngterm_core::remote::hub::AGENT_KIND;
use ngterm_core::remote::ssh::tree_tail;
use tokio::sync::oneshot;
use uuid::Uuid;

use super::Command;
use super::presentation_tests::{Actor, SESSION, local, received, settle};
use crate::api::{MobileError, MobileEvent, PaneSize};
use crate::chat::{
    AgentCommand, AgentCommandKind, ChatBody, ChatEvent, ChatState, Cursor, NOT_OPEN, NOT_SHOWN, OLDER_PAGE, Side,
    chat_tail, commands_tail, page_tail,
};
use crate::pane::PaneRef;
use crate::transport::fake::{Opened, Script};
use crate::transport::{ChannelEnd, ChannelInput, ChannelPeer};

const LEFT: &str = "00000000-0000-0000-0000-0000000000B1";
const RIGHT: &str = "00000000-0000-0000-0000-0000000000B2";

fn uuid(id: &str) -> Uuid {
    Uuid::parse_str(id).unwrap()
}

/// The machine's listing: the session with both its panes.
fn listing() -> Script {
    let remote = serde_json::json!({
        "endpoint": {"executable": "/ngterm-session", "socketDirectory": "/tmp/ngterm-live-x"},
        "sessions": [{
            "id": SESSION, "name": "main", "windowID": "w1", "windowName": "main", "workspaceID": "ws1",
            "workspaceName": "work", "cwd": "/repo",
            "panes": [{"pane": "left", "daemon": daemon_name(uuid(LEFT))},
                      {"pane": "right", "daemon": daemon_name(uuid(RIGHT))}],
        }],
        "presentation": 1,
    });
    Script::ok(&serde_json::json!({"ok": true, "result": {"remote": remote}}).to_string())
}

fn layout(primary: &str, other: &str) -> PresentationLayout {
    PresentationLayout {
        panes: vec![uuid(primary), uuid(other)],
        primary: uuid(primary),
        axis: Some("vertical".to_owned()),
        shown: true,
    }
}

fn agent_frame(pane: &str, conversation: Option<&str>) -> Body {
    Body::Agent(PresentationAgent {
        pane: PresentationPane::Identity(uuid(pane)),
        agent: conversation.map(|_| "claude".to_owned()),
        conversation_id: conversation.map(str::to_owned),
    })
}

fn reference(pane: &str) -> PaneRef {
    PaneRef {
        machine_id: "m1".to_owned(),
        session_id: SESSION.to_owned(),
        pane_id: pane.to_owned(),
    }
}

/// One NDJSON line of `ngtermctl agent chat`.
fn line(id: &str, cursor: u64, body: &serde_json::Value) -> String {
    let mut event = body.clone();
    event["id"] = id.into();
    event["cursor"] = cursor.into();
    format!("{event}\n")
}

fn assistant(id: &str, cursor: u64, text: &str) -> String {
    line(id, cursor, &serde_json::json!({"type": "assistant", "text": text}))
}

/// The `conversation` event that leads a backlog or a resume.
fn conversation(id: &str, cursor: u64) -> String {
    line(
        "conversation",
        cursor,
        &serde_json::json!({"type": "conversation", "agent": "claude", "conversation_id": id}),
    )
}

/// The chat channel resuming after `at` in conversation `id`.
fn resumed(side: Side, at: u64, id: &str) -> String {
    chat_tail(SESSION, side, Some(Cursor { at, conversation: id }))
}

/// An older page before `at` in conversation `id`.
fn paging(at: u64, id: &str) -> String {
    page_tail(SESSION, Side::Left, Cursor { at, conversation: id }, OLDER_PAGE)
}

fn live(cursor: u64) -> String {
    line("live", cursor, &serde_json::json!({"type": "live"}))
}

fn reset() -> String {
    line("reset", 0, &serde_json::json!({"type": "reset"}))
}

fn texts(events: &[ChatEvent]) -> Vec<String> {
    events
        .iter()
        .map(|event| match &event.body {
            ChatBody::Assistant { text } => format!("{}:{text}", event.id),
            ChatBody::Live => "live".to_owned(),
            ChatBody::Reset => "reset".to_owned(),
            ChatBody::Conversation { conversation_id, .. } => format!("conversation:{conversation_id}"),
            other => format!("{other:?}"),
        })
        .collect()
}

/// An attached session showing its left pane, the origin announcing an agent in it.
struct Fixture {
    actor: Actor,
    stream: ChannelPeer,
    rev: i64,
    /// The left pane's attach, held open.
    attach: Vec<Opened>,
}

impl Fixture {
    async fn start() -> Self {
        let actor = Actor::start();
        actor.transport.script(&tree_tail(), listing());
        actor.connect().await;
        actor.list().await;
        actor.attach().await.unwrap();
        settle().await;
        let mut stream = actor.transport.take_opened().remove(0).peer;
        let (bodies, _) = received(&mut stream).await;
        let Body::Hello(hello) = &bodies[0] else {
            panic!("the stream opens with the hello")
        };
        assert!(
            hello.kinds.iter().any(|kind| kind == AGENT_KIND),
            "the hello lists agent"
        );
        let mut fixture = Self {
            actor,
            stream,
            rev: 0,
            attach: Vec::new(),
        };
        fixture
            .send(Body::Hello(PresentationHello {
                version: frames::VERSION,
                kinds: Vec::new(),
                mode: PresentationMode::Mirror,
            }))
            .await;
        let snapshot = PresentationSnapshot {
            layout: Some(layout(LEFT, RIGHT)),
            ..PresentationSnapshot::default()
        };
        fixture.send(Body::Snapshot(snapshot)).await;
        fixture.send(agent_frame(LEFT, Some("c1"))).await;
        fixture.send(agent_frame(RIGHT, None)).await;
        fixture.show(LEFT).await;
        fixture.actor.sink.take();
        fixture
    }

    /// One frame of the presentation stream.
    async fn send(&mut self, body: Body) {
        self.rev += 1;
        let line = frames::encode(&PresentationFrame {
            generation: 1,
            rev: self.rev,
            body,
        })
        .unwrap();
        assert!(self.stream.data(line).await);
        settle().await;
    }

    async fn show(&mut self, pane: &str) {
        let (reply, answer) = oneshot::channel();
        self.actor.post(Command::ShowPane {
            machine_id: "m1".to_owned(),
            session_id: SESSION.to_owned(),
            pane_id: pane.to_owned(),
            size: PaneSize {
                cols: 80,
                rows: 24,
                cell_width: 9,
                cell_height: 18,
            },
            reply,
        });
        assert!(answer.await.unwrap().is_ok());
        settle().await;
        self.attach.extend(self.actor.transport.take_opened());
    }

    async fn open(&self, pane: &str) -> Result<(), MobileError> {
        let (reply, answer) = oneshot::channel();
        self.actor.post(Command::OpenChat {
            pane: reference(pane),
            reply,
        });
        let opened = answer.await.unwrap();
        settle().await;
        opened
    }

    /// The chat channels opened since the last call, each with its command.
    fn chats(&mut self) -> Vec<Opened> {
        let (chats, others): (Vec<Opened>, Vec<Opened>) = self
            .actor
            .transport
            .take_opened()
            .into_iter()
            .partition(|opened| opened.command.contains("agent"));
        self.attach.extend(others);
        chats
    }

    /// The one chat channel opened since the last call, run as `command`.
    fn chat(&mut self, command: &str) -> ChannelPeer {
        let mut chats = self.chats();
        assert_eq!(chats.len(), 1, "one chat channel");
        let chat = chats.remove(0);
        assert_eq!(chat.command, command);
        chat.peer
    }

    async fn older(&self, pane: &str) -> Result<Vec<ChatEvent>, MobileError> {
        let (reply, answer) = oneshot::channel();
        self.actor.post(Command::LoadOlder {
            pane: reference(pane),
            reply,
        });
        answer.await.unwrap()
    }

    async fn commands(&self, pane: &str) -> Result<Vec<AgentCommand>, MobileError> {
        let (reply, answer) = oneshot::channel();
        self.actor.post(Command::AgentCommands {
            pane: reference(pane),
            reply,
        });
        answer.await.unwrap()
    }

    /// The chat events reported since the last take, and the chat states.
    fn reported(&self) -> (Vec<ChatEvent>, Vec<ChatState>) {
        let (mut events, mut states) = (Vec::new(), Vec::new());
        for event in self.actor.sink.take() {
            match event {
                MobileEvent::Chat { pane, events: batch } => {
                    assert_eq!(pane, reference(LEFT));
                    events.extend(batch);
                }
                MobileEvent::ChatState { pane, state } => {
                    assert_eq!(pane, reference(LEFT));
                    states.push(state);
                }
                _ => {}
            }
        }
        (events, states)
    }

    fn chat_execs(&self) -> Vec<String> {
        self.actor
            .transport
            .execs()
            .into_iter()
            .map(|exec| exec.command)
            .filter(|command| command.contains("agent"))
            .collect()
    }
}

async fn feed(peer: &ChannelPeer, lines: &[String]) {
    assert!(peer.data(lines.concat().into_bytes()).await);
    settle().await;
}

/// Whether the core closed `peer`'s channel.
async fn closed(peer: &mut ChannelPeer) -> bool {
    settle().await;
    let mut closed = false;
    while let Some(input) = peer.try_input() {
        closed |= input == ChannelInput::Close;
    }
    closed
}

#[test]
fn the_hello_lists_agent_and_each_agent_frame_becomes_a_pane_agent_event() {
    local(async {
        let mut fixture = Fixture::start().await;
        fixture.send(agent_frame(RIGHT, Some("c9"))).await;
        fixture
            .send(Body::Agent(PresentationAgent {
                pane: PresentationPane::Scratch,
                agent: Some("codex".to_owned()),
                conversation_id: None,
            }))
            .await;

        let agents: Vec<MobileEvent> = fixture
            .actor
            .sink
            .take()
            .into_iter()
            .filter(|event| matches!(event, MobileEvent::PaneAgent { .. }))
            .collect();
        assert_eq!(
            agents,
            [MobileEvent::PaneAgent {
                machine_id: "m1".to_owned(),
                session_id: SESSION.to_owned(),
                pane_id: RIGHT.to_owned(),
                agent: Some("claude".to_owned()),
                conversation_id: Some("c9".to_owned()),
            }],
            "the scratch pane keeps no agent"
        );
    });
}

#[test]
fn a_chat_delivers_its_events_in_order_and_a_later_line_updates_the_same_id() {
    local(async {
        let mut fixture = Fixture::start().await;
        assert_eq!(fixture.open(LEFT).await, Ok(()));
        let chat = fixture.chat(&chat_tail(SESSION, Side::Left, None));

        let running = line(
            "toolu_1",
            100,
            &serde_json::json!({"type": "tool", "name": "Bash", "kind": "shell", "title": "ls", "status": "running"}),
        );
        let done = line(
            "toolu_1",
            100,
            &serde_json::json!({"type": "tool", "name": "Bash", "kind": "shell", "title": "ls", "status": "ok",
                                "output": "a\nb", "truncated": true}),
        );
        feed(&chat, &[assistant("line-0", 0, "hi"), running, live(200)]).await;
        feed(&chat, &[done, assistant("line-300", 300, "bye")]).await;

        let (events, states) = fixture.reported();
        assert!(states.is_empty());
        let ids: Vec<&str> = events.iter().map(|event| event.id.as_str()).collect();
        assert_eq!(ids, ["line-0", "toolu_1", "live", "toolu_1", "line-300"]);
        let ChatBody::Tool { tool } = &events[3].body else {
            panic!("a tool event")
        };
        assert_eq!((tool.output.as_deref(), tool.truncated), (Some("a\nb"), true));
    });
}

#[test]
fn older_pages_run_before_the_oldest_main_cursor_until_the_start() {
    local(async {
        let mut fixture = Fixture::start().await;
        fixture.open(LEFT).await.unwrap();
        let chat = fixture.chat(&chat_tail(SESSION, Side::Left, None));
        assert_eq!(fixture.older(LEFT).await, Ok(Vec::new()), "nothing to page from yet");

        // a subagent's cursor is in its own file: paging goes by the main transcript's
        let sub = line(
            "agent-a:line-5",
            5,
            &serde_json::json!({"type": "assistant", "text": "sub", "parent": "toolu_9"}),
        );
        feed(
            &chat,
            &[conversation("c1", 100), assistant("line-100", 100, "b"), sub, live(200)],
        )
        .await;
        let page = format!("{}{}", assistant("line-0", 0, "a"), assistant("line-50", 50, "a2"));
        fixture.actor.transport.script(&paging(100, "c1"), Script::ok(&page));

        let older = fixture.older(LEFT).await.unwrap();
        assert_eq!(texts(&older), ["line-0:a", "line-50:a2"]);
        assert_eq!(fixture.older(LEFT).await, Ok(Vec::new()), "the start was reached");
        let pages = fixture
            .chat_execs()
            .into_iter()
            .filter(|command| command.contains("--before"))
            .count();
        assert_eq!(pages, 1, "at the start no page is read");
        let (events, _) = fixture.reported();
        assert_eq!(events.len(), 4, "a page is answered, not reported");
    });
}

#[test]
fn a_severed_connection_resumes_after_the_newest_cursor_with_no_repeats_and_no_gaps() {
    local(async {
        let mut fixture = Fixture::start().await;
        fixture.open(LEFT).await.unwrap();
        let chat = fixture.chat(&chat_tail(SESSION, Side::Left, None));
        feed(&chat, &[conversation("c1", 0), assistant("line-0", 0, "a"), live(100)]).await;
        feed(&chat, &[assistant("line-100", 100, "b")]).await;
        fixture.reported();

        fixture.actor.transport.drop_connections("m1");
        settle().await;
        assert_eq!(fixture.reported().1, [ChatState::Reconnecting]);
        tokio::time::sleep(Duration::from_secs(2)).await;
        settle().await;
        let chat = fixture.chat(&resumed(Side::Left, 100, "c1"));

        // the carried conversation and the cursor's line again, unchanged, then changed, then what came while away
        feed(
            &chat,
            &[
                conversation("c1", 100),
                assistant("line-100", 100, "b"),
                assistant("line-100", 100, "b, more"),
                assistant("line-200", 200, "c"),
                live(300),
            ],
        )
        .await;
        let (events, states) = fixture.reported();
        assert!(states.is_empty());
        assert_eq!(texts(&events), ["line-100:b, more", "line-200:c", "live"]);
    });
}

#[test]
fn a_failed_command_after_the_backlog_is_retried_on_its_backoff_from_the_newest_cursor() {
    local(async {
        let mut fixture = Fixture::start().await;
        fixture.open(LEFT).await.unwrap();
        let chat = fixture.chat(&chat_tail(SESSION, Side::Left, None));
        feed(&chat, &[conversation("c1", 0), assistant("line-0", 0, "a"), live(40)]).await;
        chat.end(ChannelEnd::Exit {
            code: Some(1),
            signal: None,
        })
        .await;
        settle().await;
        assert_eq!(fixture.reported().1, [ChatState::Reconnecting]);
        assert!(fixture.chats().is_empty(), "not before its backoff");

        tokio::time::sleep(Duration::from_secs(2)).await;
        settle().await;
        let chat = fixture.chat(&resumed(Side::Left, 40, "c1"));
        feed(&chat, &[live(40)]).await;
        assert_eq!(texts(&fixture.reported().0), ["live"]);
    });
}

#[test]
fn a_reset_starts_over_and_the_catalog_is_fetched_once_per_conversation() {
    local(async {
        let mut fixture = Fixture::start().await;
        let catalog = serde_json::json!([
            {"name": "compact", "description": "Clear history", "kind": "builtin", "argument_hint": "[instructions]"},
            {"name": "deploy", "description": "Ship it", "kind": "a-kind-from-later"},
        ]);
        fixture
            .actor
            .transport
            .script(&commands_tail(SESSION, Side::Left), Script::ok(&catalog.to_string()));
        let expected = vec![AgentCommand {
            name: "compact".to_owned(),
            description: "Clear history".to_owned(),
            kind: AgentCommandKind::Builtin,
            argument_hint: Some("[instructions]".to_owned()),
        }];
        assert_eq!(fixture.commands(LEFT).await, Ok(expected.clone()));
        assert_eq!(fixture.commands(LEFT).await, Ok(expected.clone()));
        assert_eq!(fixture.chat_execs().len(), 1, "fetched once");

        fixture.open(LEFT).await.unwrap();
        let chat = fixture.chat(&chat_tail(SESSION, Side::Left, None));
        feed(&chat, &[assistant("line-0", 0, "a"), live(10)]).await;
        feed(&chat, &[reset(), assistant("line-0", 0, "a"), live(10)]).await;
        let (events, _) = fixture.reported();
        assert_eq!(
            texts(&events),
            ["line-0:a", "live", "reset", "line-0:a", "live"],
            "after a reset nothing counts as delivered"
        );
        assert_eq!(fixture.commands(LEFT).await, Ok(expected.clone()));
        assert_eq!(fixture.chat_execs().len(), 3, "the chat, then the catalog again");

        fixture.send(agent_frame(LEFT, Some("c2"))).await;
        assert_eq!(fixture.commands(LEFT).await, Ok(expected));
        assert_eq!(fixture.chat_execs().len(), 4, "a new conversation fetches it again");
        fixture.actor.transport.script(
            &commands_tail(SESSION, Side::Left),
            Script::failing(64, "unrecognized subcommand"),
        );
        fixture.send(agent_frame(LEFT, Some("c3"))).await;
        assert!(matches!(fixture.commands(LEFT).await, Err(MobileError::Remote { .. })));
    });
}

#[test]
fn the_channel_ending_tells_the_app_to_show_the_terminal() {
    local(async {
        let mut fixture = Fixture::start().await;
        fixture.open(LEFT).await.unwrap();
        let chat = fixture.chat(&chat_tail(SESSION, Side::Left, None));
        feed(&chat, &[live(0)]).await;
        chat.end(ChannelEnd::Exit {
            code: Some(0),
            signal: None,
        })
        .await;
        settle().await;
        assert_eq!(fixture.reported().1, [ChatState::Ended]);
        assert_eq!(
            fixture.older(LEFT).await,
            Err(MobileError::Remote {
                reason: NOT_OPEN.to_owned()
            }),
            "the chat is closed"
        );

        // no agent left in the pane, said after the backlog: ended too, not retried
        fixture.open(LEFT).await.unwrap();
        let chat = fixture.chat(&chat_tail(SESSION, Side::Left, None));
        feed(&chat, &[live(0)]).await;
        assert!(chat.stderr(b"Error: no agent in this pane\n".to_vec()).await);
        chat.end(ChannelEnd::Exit {
            code: Some(1),
            signal: None,
        })
        .await;
        settle().await;
        assert_eq!(fixture.reported().1, [ChatState::Ended]);
        tokio::time::sleep(Duration::from_secs(60)).await;
        settle().await;
        assert!(fixture.chats().is_empty());
    });
}

#[test]
fn an_origin_without_the_chat_command_degrades_to_the_terminal_with_no_error() {
    local(async {
        let mut fixture = Fixture::start().await;
        fixture.open(LEFT).await.unwrap();
        let chat = fixture.chat(&chat_tail(SESSION, Side::Left, None));
        assert!(chat.stderr(b"error: unrecognized subcommand 'agent'\n".to_vec()).await);
        chat.end(ChannelEnd::Exit {
            code: Some(64),
            signal: None,
        })
        .await;
        settle().await;
        assert_eq!(fixture.reported(), (Vec::new(), vec![ChatState::Ended]));
        tokio::time::sleep(Duration::from_secs(60)).await;
        settle().await;
        assert!(fixture.chats().is_empty(), "never retried");
    });
}

#[test]
fn an_origin_that_never_announces_an_agent_reports_none() {
    local(async {
        let actor = Actor::start();
        actor.transport.script(&tree_tail(), listing());
        actor.connect().await;
        actor.list().await;
        actor.attach().await.unwrap();
        let stream = actor.stream().await;
        super::presentation_tests::deliver(&stream, vec![Body::Context(Some("x".to_owned()))]).await;
        assert!(
            !actor
                .sink
                .take()
                .iter()
                .any(|event| matches!(event, MobileEvent::PaneAgent { .. }))
        );
    });
}

#[test]
fn chat_channels_exist_only_for_shown_panes_and_close_on_hide() {
    local(async {
        let mut fixture = Fixture::start().await;
        assert_eq!(
            fixture.open(RIGHT).await,
            Err(MobileError::Remote {
                reason: NOT_SHOWN.to_owned()
            })
        );
        fixture.open(LEFT).await.unwrap();
        fixture.open(LEFT).await.unwrap();
        let mut chat = fixture.chat(&chat_tail(SESSION, Side::Left, None));
        assert!(fixture.chats().is_empty(), "opening an open chat opens nothing");

        fixture.actor.post(Command::CloseChat(reference(LEFT)));
        assert!(closed(&mut chat).await, "closing the chat closes its channel");
        fixture.open(LEFT).await.unwrap();
        let mut chat = fixture.chat(&chat_tail(SESSION, Side::Left, None));
        fixture.actor.post(Command::HidePane(reference(LEFT)));
        assert!(closed(&mut chat).await, "hiding the pane closes its chat");
        assert_eq!(
            fixture.open(LEFT).await,
            Err(MobileError::Remote {
                reason: NOT_SHOWN.to_owned()
            })
        );
        assert_eq!(fixture.reported(), (Vec::new(), Vec::new()), "closing reports nothing");
    });
}

#[test]
fn a_pane_that_changes_sides_resumes_its_chat_on_the_new_side() {
    local(async {
        let mut fixture = Fixture::start().await;
        fixture.open(LEFT).await.unwrap();
        let mut chat = fixture.chat(&chat_tail(SESSION, Side::Left, None));
        feed(&chat, &[conversation("c1", 0), assistant("line-0", 0, "a"), live(30)]).await;

        fixture.send(Body::Layout(layout(RIGHT, LEFT))).await;
        assert!(closed(&mut chat).await);
        let chat = fixture.chat(&resumed(Side::Right, 30, "c1"));
        feed(&chat, &[assistant("line-30", 30, "b"), live(60)]).await;
        assert_eq!(
            texts(&fixture.reported().0),
            ["conversation:c1", "line-0:a", "live", "line-30:b", "live"]
        );
    });
}

const FAILED: ChannelEnd = ChannelEnd::Exit {
    code: Some(1),
    signal: None,
};

/// The pane moving to another conversation while the chat's channel is down is never read at the old cursor: each
/// resume names the cursor's conversation, whether the origin's announcement of the new one came before the channel
/// is back, after it or not at all, and the origin's `reset` (whatever the new file's size) starts the chat over:
/// cursors, pages and later resumes all go by the new conversation.
#[test]
fn a_conversation_switched_while_away_resumes_by_the_cursors_conversation_and_starts_over() {
    local(async {
        let mut fixture = Fixture::start().await;
        fixture.open(LEFT).await.unwrap();
        let chat = fixture.chat(&chat_tail(SESSION, Side::Left, None));
        feed(
            &chat,
            &[
                conversation("c1", 0),
                assistant("line-0", 0, "a"),
                assistant("line-100", 100, "b"),
                live(150),
            ],
        )
        .await;
        chat.end(FAILED).await;
        settle().await;
        fixture.reported();

        // announced before the channel is back: the resume still names the cursor's conversation
        fixture.send(agent_frame(LEFT, Some("c2"))).await;
        tokio::time::sleep(Duration::from_secs(2)).await;
        settle().await;
        let chat = fixture.chat(&resumed(Side::Left, 150, "c1"));
        // the origin's answer: the new conversation, a file longer than the cursor
        feed(
            &chat,
            &[
                reset(),
                conversation("c2", 0),
                assistant("line-0", 0, "x"),
                assistant("line-500", 500, "y"),
                live(600),
            ],
        )
        .await;
        let (events, states) = fixture.reported();
        assert!(states.is_empty());
        assert_eq!(
            texts(&events),
            ["reset", "conversation:c2", "line-0:x", "line-500:y", "live"],
            "nothing of the old conversation counts as delivered"
        );

        // announced after it: nothing more
        fixture.send(agent_frame(LEFT, Some("c2"))).await;
        assert!(fixture.chats().is_empty());
        assert_eq!(
            fixture.older(LEFT).await,
            Ok(Vec::new()),
            "the new conversation's start"
        );

        // never announced, the file shorter than the cursor this time: the resume is in c2, and the reset says c3
        chat.end(FAILED).await;
        settle().await;
        tokio::time::sleep(Duration::from_secs(2)).await;
        settle().await;
        let chat = fixture.chat(&resumed(Side::Left, 600, "c2"));
        feed(
            &chat,
            &[reset(), conversation("c3", 40), assistant("line-40", 40, "z"), live(80)],
        )
        .await;
        assert_eq!(
            texts(&fixture.reported().0),
            ["reset", "conversation:c3", "line-40:z", "live"]
        );
        fixture
            .actor
            .transport
            .script(&paging(40, "c3"), Script::ok(&assistant("line-0", 0, "w")));
        assert_eq!(texts(&fixture.older(LEFT).await.unwrap()), ["line-0:w"]);
        chat.end(FAILED).await;
        settle().await;
        tokio::time::sleep(Duration::from_secs(2)).await;
        settle().await;
        fixture.chat(&resumed(Side::Left, 80, "c3"));
    });
}

/// An event a subagent's transcript gave, under the task tool `parent`.
fn subagent(id: &str, cursor: u64, parent: &str, text: &str) -> String {
    line(
        id,
        cursor,
        &serde_json::json!({"type": "assistant", "text": text, "parent": parent}),
    )
}

/// A channel lost mid-way.
fn lost() -> ChannelEnd {
    ChannelEnd::Lost("connection reset".to_owned())
}

/// The channel ends with `end`; the chat's retry is due.
async fn retried(chat: ChannelPeer, end: ChannelEnd) {
    chat.end(end).await;
    settle().await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    settle().await;
}

/// The resume cursor moves only once a launch is caught up: a channel lost before the `live` (after the main lines,
/// before the subagents' catch-up) resumes where that launch began, so the origin sends every subagent's lines since
/// then again, and what was delivered is not shown twice. A catch-up begun with no cursor (the first launch, a
/// `reset`) resumes from its first main event, never from a plain backlog that could start later; a catch-up lost
/// before any main event of its own starts over with a plain backlog.
#[test]
fn a_channel_lost_before_live_resumes_where_its_catch_up_began() {
    local(async {
        let mut fixture = Fixture::start().await;
        fixture.open(LEFT).await.unwrap();
        let chat = fixture.chat(&chat_tail(SESSION, Side::Left, None));
        // the first launch: lost after main lines, before the subagents' backlogs and the `live`
        let first = [
            conversation("c1", 50),
            assistant("line-50", 50, "a"),
            assistant("line-150", 150, "b"),
        ];
        feed(&chat, &first).await;
        retried(chat, lost()).await;
        assert_eq!(fixture.reported().1, [ChatState::Reconnecting]);
        let chat = fixture.chat(&resumed(Side::Left, 50, "c1"));
        let again = [subagent("sub-1", 10, "toolu_1", "early"), live(200)];
        feed(&chat, &[first.as_slice(), &again].concat()).await;
        assert_eq!(texts(&fixture.reported().0), ["sub-1:early", "live"]);

        // following moves the cursor by each main event
        feed(&chat, &[assistant("line-200", 200, "c")]).await;
        assert_eq!(texts(&fixture.reported().0), ["line-200:c"]);
        retried(chat, FAILED).await;
        let chat = fixture.chat(&resumed(Side::Left, 200, "c1"));

        // a resume lost after the main lines it caught up on, before a subagent's: it resumes from 200 again
        let main = [
            conversation("c1", 200),
            assistant("line-200", 200, "c"),
            assistant("line-300", 300, "d"),
            assistant("line-400", 400, "e"),
        ];
        feed(&chat, &main).await;
        assert_eq!(texts(&fixture.reported().0), ["line-300:d", "line-400:e"]);
        retried(chat, FAILED).await;
        let chat = fixture.chat(&resumed(Side::Left, 200, "c1"));
        let again = [subagent("sub-2", 20, "toolu_1", "while away"), live(500)];
        feed(&chat, &[main.as_slice(), &again].concat()).await;
        assert_eq!(
            texts(&fixture.reported().0),
            ["sub-2:while away", "live"],
            "the subagent's line written before line-400 arrives, and nothing twice"
        );

        // a `reset` while following starts a catch-up: lost in it, the resume is the new conversation's first event
        let fresh = [
            reset(),
            conversation("c2", 0),
            assistant("line-0", 0, "x"),
            assistant("line-90", 90, "y"),
        ];
        feed(&chat, &fresh).await;
        fixture.reported();
        retried(chat, FAILED).await;
        let chat = fixture.chat(&resumed(Side::Left, 0, "c2"));

        // a `reset` lost before any main event of its own: a plain backlog, the app having dropped what it showed
        feed(&chat, &[reset()]).await;
        assert_eq!(texts(&fixture.reported().0), ["reset"]);
        retried(chat, FAILED).await;
        let chat = fixture.chat(&chat_tail(SESSION, Side::Left, None));
        feed(&chat, &[conversation("c3", 0), assistant("line-0", 0, "z"), live(60)]).await;
        assert_eq!(texts(&fixture.reported().0), ["conversation:c3", "line-0:z", "live"]);
    });
}

/// A chat whose events named no conversation (an agent before its first prompt) resumes with an empty one, which
/// the origin never takes for the pane's, and has no older page to read.
#[test]
fn a_chat_that_named_no_conversation_resumes_with_none_and_pages_nothing() {
    local(async {
        let mut fixture = Fixture::start().await;
        fixture.open(LEFT).await.unwrap();
        let chat = fixture.chat(&chat_tail(SESSION, Side::Left, None));
        feed(&chat, &[live(200)]).await;
        assert_eq!(fixture.older(LEFT).await, Ok(Vec::new()));
        assert!(
            !fixture.chat_execs().iter().any(|command| command.contains("--before")),
            "no page is read"
        );
        chat.end(FAILED).await;
        settle().await;
        tokio::time::sleep(Duration::from_secs(2)).await;
        settle().await;
        fixture.chat(&resumed(Side::Left, 200, ""));
    });
}

/// An older page is stale when the origin answers it with a `reset` (the pane is in another conversation now), and
/// when the chat it was asked for closed, even if the pane's chat was opened again before the answer came: the
/// caller is told `Cancelled` and the cursors stay as they were.
#[test]
fn a_page_of_another_conversation_or_of_a_closed_chat_is_cancelled() {
    local(async {
        let mut fixture = Fixture::start().await;
        fixture.open(LEFT).await.unwrap();
        let chat = fixture.chat(&chat_tail(SESSION, Side::Left, None));
        feed(
            &chat,
            &[conversation("c1", 100), assistant("line-100", 100, "b"), live(200)],
        )
        .await;
        fixture.actor.transport.script_sequence(
            &paging(100, "c1"),
            vec![Script::ok(&reset()), Script::ok(&assistant("line-50", 50, "a"))],
        );
        assert_eq!(fixture.older(LEFT).await, Err(MobileError::Cancelled));
        assert_eq!(
            texts(&fixture.older(LEFT).await.unwrap()),
            ["line-50:a"],
            "still from the same cursor"
        );

        // a page in flight across a close and an open
        let (reply, answer) = oneshot::channel();
        fixture.actor.post(Command::LoadOlder {
            pane: reference(LEFT),
            reply,
        });
        settle().await;
        let page = fixture.chat(&paging(50, "c1"));
        fixture.actor.post(Command::CloseChat(reference(LEFT)));
        settle().await;
        fixture.open(LEFT).await.unwrap();
        let chat = fixture.chat(&chat_tail(SESSION, Side::Left, None));
        feed(
            &chat,
            &[conversation("c1", 100), assistant("line-100", 100, "b"), live(200)],
        )
        .await;
        assert!(page.data(assistant("line-0", 0, "z").into_bytes()).await);
        page.end(ChannelEnd::Exit {
            code: Some(0),
            signal: None,
        })
        .await;
        assert_eq!(answer.await.unwrap(), Err(MobileError::Cancelled));
        fixture
            .actor
            .transport
            .script(&paging(100, "c1"), Script::ok(&assistant("line-50", 50, "a")));
        assert_eq!(
            texts(&fixture.older(LEFT).await.unwrap()),
            ["line-50:a"],
            "the reopened chat pages from its own cursor"
        );
    });
}

/// Suspending reports nothing and so leaves nothing counted as reported: a channel that fails after the resume is
/// reported reconnecting.
#[test]
fn a_chat_failing_after_a_resume_is_reported_reconnecting() {
    local(async {
        let mut fixture = Fixture::start().await;
        fixture.open(LEFT).await.unwrap();
        let chat = fixture.chat(&chat_tail(SESSION, Side::Left, None));
        feed(&chat, &[conversation("c1", 0), assistant("line-0", 0, "a"), live(40)]).await;
        fixture.reported();

        fixture.actor.post(Command::Suspend);
        settle().await;
        assert_eq!(
            fixture.reported(),
            (Vec::new(), Vec::new()),
            "suspending reports nothing"
        );
        fixture.actor.post(Command::Resume);
        settle().await;
        let chat = fixture.chat(&resumed(Side::Left, 40, "c1"));
        chat.end(FAILED).await;
        settle().await;
        assert_eq!(fixture.reported().1, [ChatState::Reconnecting]);
    });
}
