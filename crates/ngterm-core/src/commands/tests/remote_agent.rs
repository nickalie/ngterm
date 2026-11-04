//! The presentation stream's `agent` kind (`remote::agents`): what each pane runs, from the agent record, sent only to
//! a viewer whose hello lists it, over a [`FakeSessionHost`] and fake streams.
//!
//! [`FakeSessionHost`]: ngterm_host::fakes::FakeSessionHost

use std::cell::RefCell;
use std::rc::Rc;

use ngterm_host::ProcessStat;
use ngterm_host::fakes::FakeProcessTable;
use ngterm_proto::command::{AgentCmd, LiveCmd, SessionCmd};
use ngterm_proto::{AgentReport, Command, Request};
use serde_json::{Value, json};
use uuid::Uuid;

use super::live::{Live, live};
use super::remote::{Viewer, node, subscribe_listing};
use super::{args, wire};
use crate::Core;
use crate::live::daemon_name;
use crate::remote::LineChannel;
use crate::remote::client::viewer_kinds;
use crate::remote::hub::AGENT_KIND;

const CLAUDE_A: &str = "/home/user/.claude/projects/-home-user-project/aaaa.jsonl";
const CLAUDE_B: &str = "/home/user/.claude/projects/-home-user-project/bbbb.jsonl";
const CODEX_C: &str = "/home/user/.codex/sessions/2026/11/03/rollout-2026-11-03T23-01-46-cccc.jsonl";

/// The pid of the pane's shell, its daemon's leader.
const SHELL: i32 = 100;
/// The pid of the agent the shell started, leading its own group.
const AGENT: i32 = 101;

/// The kinds the phone's hello lists: the mobile viewer's, with the agent kind.
fn phone_kinds() -> Vec<String> {
    let mut kinds = viewer_kinds(false);
    kinds.push(AGENT_KIND.to_owned());
    kinds
}

fn subscribe(live: &mut Live, kinds: &[String]) -> Viewer {
    let kinds: Vec<&str> = kinds.iter().map(String::as_str).collect();
    subscribe_listing(&mut live.fixture.core, live.session, "mirror", &kinds).1
}

/// Reports `event` of a conversation from the pane `identity`, its role `role`, as `ngtermctl agent hook` does, and
/// commits.
fn report(live: &mut Live, identity: Uuid, role: &str, event: &str, agent: &str, conversation: (&str, &str)) {
    let mut request = args(|a| {
        a.pane_id = Some(wire(identity));
        a.pane = Some(role.to_owned());
    });
    request.report = Some(Box::new(AgentReport {
        agent: agent.to_owned(),
        event: event.to_owned(),
        transcript_path: Some(conversation.0.to_owned()),
        conversation_id: Some(conversation.1.to_owned()),
        approval: None,
    }));
    let response = live
        .fixture
        .send(Command::Agent(AgentCmd::Report), Some(&wire(live.session)), request);
    assert!(response.ok, "{response:?}");
    live.fixture.core.commit();
}

fn agent_frame(identity: Uuid, agent: Option<&str>, conversation: Option<&str>) -> Value {
    let mut frame = json!({"pane": wire(identity), "agent": agent});
    if let Some(conversation) = conversation {
        frame["conversationId"] = json!(conversation);
    }
    frame
}

/// The payloads of the agent frames `viewer` got, in order.
fn agent_frames(viewer: &Viewer) -> Vec<Value> {
    viewer
        .frames(AGENT_KIND)
        .into_iter()
        .map(|frame| frame["agent"].clone())
        .collect()
}

fn stat(pid: i32, parent: i32, group: i32) -> ProcessStat {
    ProcessStat {
        pid,
        ppid: parent,
        pgrp: group,
        session: SHELL,
    }
}

/// Installs a process table whose pane shell (`sh`) hands its terminal to `foreground`: [`AGENT`] (`claude`) or
/// [`SHELL`] once the agent exited.
fn foreground(live: &mut Live, foreground: i32) {
    let table = FakeProcessTable::default();
    table.insert(stat(SHELL, 1, SHELL), Some(&["sh"]));
    table.insert(stat(AGENT, SHELL, AGENT), Some(&["claude"]));
    table.set_terminal_foreground(SHELL, foreground);
    live.fixture
        .core
        .set_process_table(Box::new(table), Some("sh".to_owned()));
}

/// A live session whose main pane's daemon leads [`SHELL`], running `claude` in the foreground.
fn live_agent() -> Live {
    let mut live = live();
    live.host
        .set_record(&daemon_name(live.left), Some(1), Some(SHELL), None);
    foreground(&mut live, AGENT);
    live
}

#[test]
fn an_agent_viewer_gets_each_panes_agent_at_attach_and_on_each_change() {
    let mut live = live();
    let (left, right) = (live.left, live.split());
    live.fixture.core.commit();
    report(&mut live, left, "left", "SessionStart", "claude", (CLAUDE_A, "aaaa"));
    let phone = subscribe(&mut live, &phone_kinds());
    let tree_before = node(&mut live.fixture, live.session);
    let answer = phone.last("hello").unwrap()["hello"]["kinds"].clone();
    assert!(answer.as_array().unwrap().contains(&json!(AGENT_KIND)), "{answer}");
    // one frame per pane right after the snapshot, by identity, in the identities' order
    assert_eq!(phone.kinds()[..2], ["hello", "snapshot"]);
    let mut attached = vec![
        agent_frame(left, Some("claude"), Some("aaaa")),
        agent_frame(right, None, None),
    ];
    attached.sort_by_key(|frame| frame["pane"].as_str().unwrap().to_owned());
    assert_eq!(agent_frames(&phone), attached);
    assert_eq!(phone.sent(), 4);

    // a re-assert changes nothing, so it publishes nothing; neither does an unchanged commit
    report(
        &mut live,
        left,
        "left",
        "UserPromptSubmit",
        "claude",
        (CLAUDE_A, "aaaa"),
    );
    live.fixture.core.commit();
    assert_eq!(phone.sent(), 4);
    report(&mut live, right, "right", "SessionStart", "codex", (CODEX_C, "cccc"));
    report(&mut live, left, "left", "SessionStart", "claude", (CLAUDE_B, "bbbb"));
    report(&mut live, left, "left", "SessionEnd", "claude", (CLAUDE_B, "bbbb"));
    assert_eq!(
        agent_frames(&phone)[2..],
        [
            agent_frame(right, Some("codex"), Some("cccc")),
            agent_frame(left, Some("claude"), Some("bbbb")),
            agent_frame(left, None, None),
        ]
    );
    // the tree carries none of it
    assert_eq!(node(&mut live.fixture, live.session), tree_before);
}

#[test]
fn a_pane_new_to_the_viewers_gets_its_frame() {
    let mut live = live();
    let phone = subscribe(&mut live, &phone_kinds());
    assert_eq!(agent_frames(&phone), [agent_frame(live.left, None, None)]);
    let right = live.split();
    live.fixture.core.commit();
    assert_eq!(agent_frames(&phone)[1..], [agent_frame(right, None, None)]);
    live.fixture.core.commit();
    assert_eq!(agent_frames(&phone).len(), 2);
}

#[test]
fn the_lazy_foreground_check_clears_a_record_as_the_frame_is_built() {
    let mut live = live_agent();
    let left = live.left;
    report(&mut live, left, "left", "SessionStart", "claude", (CLAUDE_A, "aaaa"));
    // the agent exits; a desktop viewer, which does not list the kind, checks nothing
    foreground(&mut live, SHELL);
    let desktop = subscribe(&mut live, &viewer_kinds(true));
    live.fixture.core.commit();
    assert!(live.fixture.core.presentation_heartbeat());
    assert_eq!(live.fixture.core.agent_record_count(), 1);
    assert!(agent_frames(&desktop).is_empty());

    // a viewer that lists it is checked at attach
    let phone = subscribe(&mut live, &phone_kinds());
    assert_eq!(agent_frames(&phone), [agent_frame(left, None, None)]);
    assert_eq!(live.fixture.core.agent_record_count(), 0);

    // on a commit
    foreground(&mut live, AGENT);
    report(&mut live, left, "left", "SessionStart", "claude", (CLAUDE_A, "aaaa"));
    foreground(&mut live, SHELL);
    live.fixture.core.commit();
    assert_eq!(
        agent_frames(&phone)[1..],
        [
            agent_frame(left, Some("claude"), Some("aaaa")),
            agent_frame(left, None, None)
        ]
    );

    // and on a heartbeat, as no event fires when an agent exits
    foreground(&mut live, AGENT);
    report(&mut live, left, "left", "SessionStart", "claude", (CLAUDE_A, "aaaa"));
    foreground(&mut live, SHELL);
    assert!(live.fixture.core.presentation_heartbeat());
    assert_eq!(agent_frames(&phone).last(), Some(&agent_frame(left, None, None)));
    assert_eq!(agent_frames(&phone).len(), 5);
    assert_eq!(live.fixture.core.agent_record_count(), 0);
    assert!(agent_frames(&desktop).is_empty());
}

#[test]
fn a_viewer_attaching_beside_another_brings_it_the_clear_its_check_made() {
    let mut live = live_agent();
    let left = live.left;
    let first = subscribe(&mut live, &phone_kinds());
    report(&mut live, left, "left", "SessionStart", "claude", (CLAUDE_A, "aaaa"));
    assert_eq!(
        agent_frames(&first)[1..],
        [agent_frame(left, Some("claude"), Some("aaaa"))]
    );
    // exits with no commit before the second attaches: the first viewer hears of it too
    foreground(&mut live, SHELL);
    let second = subscribe(&mut live, &phone_kinds());
    assert_eq!(agent_frames(&second), [agent_frame(left, None, None)]);
    assert_eq!(agent_frames(&first)[2..], [agent_frame(left, None, None)]);
    live.fixture.core.commit();
    assert_eq!((agent_frames(&first).len(), agent_frames(&second).len()), (3, 1));
}

#[test]
fn a_long_conversation_id_travels_cut_and_the_stream_stays_up() {
    let mut live = live();
    let phone = subscribe(&mut live, &phone_kinds());
    let id = "é".repeat(300_000);
    let left = live.left;
    report(&mut live, left, "left", "SessionStart", "claude", (CLAUDE_A, &id));
    let frame = agent_frames(&phone).last().cloned().unwrap();
    assert_eq!(frame["conversationId"], json!("é".repeat(512)));
    live.fixture.core.commit();
    assert!(live.fixture.core.presentation_heartbeat());
    assert_eq!(phone.kinds().last().map(String::as_str), Some("ping"));
}

// MARK: - a viewer that does not list the kind

#[derive(Default)]
struct RawSeen {
    lines: Vec<String>,
    closed: bool,
}

/// A presentation connection keeping the exact lines the core sent.
#[derive(Clone, Default)]
struct RawViewer(Rc<RefCell<RawSeen>>);

impl LineChannel for RawViewer {
    fn send(&self, line: Vec<u8>) -> bool {
        let mut seen = self.0.borrow_mut();
        if seen.closed {
            return false;
        }
        seen.lines.push(String::from_utf8(line).unwrap());
        true
    }

    fn shutdown(&self) {
        self.0.borrow_mut().closed = true;
    }
}

/// Subscribes a [`RawViewer`] whose hello lists `kinds`.
fn subscribe_raw(core: &mut Core, session: Uuid, kinds: &[String]) -> RawViewer {
    let response = core.dispatch(&Request {
        cmd: Command::Live(LiveCmd::Present),
        target: Some(wire(session)),
        args: None,
    });
    assert!(response.ok, "{response:?}");
    let viewer = RawViewer::default();
    let serial = core.adopt_presentation_stream(&wire(session), Box::new(viewer.clone()));
    let hello = json!({"kind": "hello", "gen": 0, "rev": 0, "hello": {"version": 1, "kinds": kinds, "mode": "mirror"}});
    core.presentation_line(serial, &serde_json::to_vec(&hello).unwrap());
    core.commit();
    viewer
}

/// What a desktop viewer reads through one script of changes, with a phone beside it that lists the agent kind and
/// agents reported when `agents`, else neither: its exact lines, with this run's random ids (the session, its panes,
/// the respawn epoch) replaced by names.
fn desktop_lines(agents: bool) -> Vec<String> {
    let mut live = live_agent();
    let (session, left) = (live.session, live.left);
    let right = live.split();
    live.fixture.core.commit();
    let desktop = subscribe_raw(&mut live.fixture.core, session, &viewer_kinds(true));
    let phone_kinds = if agents { phone_kinds() } else { viewer_kinds(false) };
    let phone = subscribe(&mut live, &phone_kinds);
    if agents {
        report(&mut live, left, "left", "SessionStart", "claude", (CLAUDE_A, "aaaa"));
        report(&mut live, right, "right", "SessionStart", "codex", (CODEX_C, "cccc"));
    }
    let context = live.fixture.session(
        SessionCmd::Context,
        Some(&wire(session)),
        args(|a| {
            a.mode = Some("set".to_owned());
            a.text = Some("PR 517".to_owned());
        }),
    );
    assert!(context.ok, "{context:?}");
    live.fixture.core.commit();
    if agents {
        report(&mut live, left, "left", "SessionStart", "claude", (CLAUDE_B, "bbbb"));
        foreground(&mut live, SHELL);
        live.fixture.core.commit();
    }
    assert!(live.fixture.core.presentation_heartbeat());
    let notify = live.fixture.send(
        Command::Notify,
        Some(&wire(session)),
        args(|a| {
            a.title = Some("build".to_owned());
            a.body = Some("done".to_owned());
        }),
    );
    assert!(notify.ok);
    // each pane at attach, three reports, and the main pane's agent exiting
    assert_eq!(agent_frames(&phone).len(), if agents { 6 } else { 0 });
    let lines = desktop.0.borrow().lines.clone();
    let snapshot: Value = serde_json::from_str(&lines[1]).unwrap();
    let epoch = snapshot["snapshot"]["respawns"]["epoch"].as_str().unwrap().to_owned();
    lines
        .iter()
        .map(|line| {
            line.replace(&wire(session), "SESSION")
                .replace(&wire(left), "LEFT")
                .replace(&wire(right), "RIGHT")
                .replace(&epoch, "EPOCH")
        })
        .collect()
}

#[test]
fn a_viewer_that_does_not_list_the_kind_gets_the_same_frames_byte_for_byte() {
    let with_agents = desktop_lines(true);
    assert_eq!(with_agents, desktop_lines(false));
    let kinds: Vec<String> = with_agents
        .iter()
        .map(|line| {
            serde_json::from_str::<Value>(line).unwrap()["kind"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(kinds, ["hello", "snapshot", "context", "ping", "notify"]);
    // and its hello answer names no agent kind
    assert!(!with_agents[0].contains(AGENT_KIND), "{}", with_agents[0]);
}
