//! The hook payloads Task 1 recorded, read for the agent record.

use std::path::{Path, PathBuf};

use ngterm_proto::{AgentApproval, AgentReport};
use serde_json::{Value, json};

use crate::agent_chat::AgentKind;
use crate::agent_chat::hook::{HookEvent, check_transcript, config_dir, report};

const CLAUDE_MAIN: &str = "f559ab9f-a5bd-4b2f-91e5-72590d662bf4";
const CLAUDE_TRANSCRIPT: &str =
    "/home/user/.claude/projects/-home-user-project/f559ab9f-a5bd-4b2f-91e5-72590d662bf4.jsonl";
const CODEX_MAIN: &str = "01a1a300-e2e3-76a1-abcf-961cbbb2eb3c";
const CODEX_TRANSCRIPT: &str =
    "/home/user/.codex/sessions/2026/11/03/rollout-2026-11-03T23-01-46-01a1a300-e2e3-76a1-abcf-961cbbb2eb3c.jsonl";

fn hooks(dir: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/agent_chat")
        .join(dir)
}

fn payload(dir: &str, name: &str) -> Vec<u8> {
    std::fs::read(hooks(dir).join(name)).unwrap()
}

fn read(agent: AgentKind, event: HookEvent, dir: &str, name: &str) -> Option<AgentReport> {
    report(agent, event, &payload(dir, name)).unwrap()
}

fn conversation(agent: AgentKind, event: HookEvent, transcript: &str, id: &str) -> AgentReport {
    AgentReport {
        agent: agent.name().to_owned(),
        event: event.name().to_owned(),
        transcript_path: Some(transcript.to_owned()),
        conversation_id: Some(id.to_owned()),
        approval: None,
    }
}

/// `edit` applied to the JSON of a recorded payload.
fn edited(dir: &str, name: &str, edit: impl FnOnce(&mut serde_json::Map<String, Value>)) -> Vec<u8> {
    let mut value: Value = serde_json::from_slice(&payload(dir, name)).unwrap();
    edit(value.as_object_mut().unwrap());
    serde_json::to_vec(&value).unwrap()
}

#[test]
fn claude_session_events_carry_the_conversation_and_its_transcript() {
    use HookEvent::{SessionEnd, SessionStart, UserPromptSubmit};
    let claude = AgentKind::Claude;
    assert_eq!(
        read(claude, SessionStart, "claude/hooks", "01-SessionStart.json"),
        Some(conversation(claude, SessionStart, CLAUDE_TRANSCRIPT, CLAUDE_MAIN))
    );
    assert_eq!(
        read(claude, UserPromptSubmit, "claude/hooks", "02-UserPromptSubmit.json"),
        Some(conversation(claude, UserPromptSubmit, CLAUDE_TRANSCRIPT, CLAUDE_MAIN))
    );
    // /clear: the old conversation ends, a new one starts in a new file
    assert_eq!(
        read(claude, SessionEnd, "claude/hooks", "59-SessionEnd.json"),
        Some(conversation(claude, SessionEnd, CLAUDE_TRANSCRIPT, CLAUDE_MAIN))
    );
    let cleared = "2897645c-5554-4c74-ab67-20ec0af2f264";
    assert_eq!(
        read(claude, SessionStart, "claude/hooks", "60-SessionStart.json"),
        Some(conversation(
            claude,
            SessionStart,
            &format!("/home/user/.claude/projects/-home-user-project/{cleared}.jsonl"),
            cleared
        ))
    );
}

#[test]
fn claude_permission_requests_carry_the_tool_its_description_and_its_command() {
    let claude = AgentKind::Claude;
    let read = read(
        claude,
        HookEvent::PermissionRequest,
        "claude/hooks",
        "33-PermissionRequest.json",
    );
    assert_eq!(
        read,
        Some(AgentReport {
            approval: Some(AgentApproval {
                tool: "Bash".to_owned(),
                title: "Create empty file allowed.txt".to_owned(),
                detail: Some("touch allowed.txt".to_owned()),
                tool_use_id: None,
            }),
            ..conversation(claude, HookEvent::PermissionRequest, CLAUDE_TRANSCRIPT, CLAUDE_MAIN)
        })
    );
    // AskUserQuestion fires one too: reported as such, for the record to tell it from an approval
    let question = report(
        claude,
        HookEvent::PermissionRequest,
        &payload("claude/hooks", "27-PermissionRequest.json"),
    )
    .unwrap()
    .and_then(|report| report.approval)
    .unwrap();
    assert_eq!(question.tool, "AskUserQuestion");
}

#[test]
fn an_edit_permission_names_the_file() {
    let bytes = edited("claude/hooks", "33-PermissionRequest.json", |payload| {
        payload.insert("tool_name".to_owned(), json!("Write"));
        payload.insert(
            "tool_input".to_owned(),
            json!({"file_path": "/home/user/project/notes.txt", "content": "hi\n"}),
        );
        payload.insert("tool_use_id".to_owned(), json!("toolu_1"));
    });
    let approval = report(AgentKind::Claude, HookEvent::PermissionRequest, &bytes)
        .unwrap()
        .and_then(|report| report.approval);
    assert_eq!(
        approval,
        Some(AgentApproval {
            tool: "Write".to_owned(),
            title: "/home/user/project/notes.txt".to_owned(),
            detail: Some("/home/user/project/notes.txt".to_owned()),
            tool_use_id: Some("toolu_1".to_owned()),
        })
    );
}

#[test]
fn codex_events_carry_the_thread_and_its_rollout() {
    use HookEvent::{PermissionRequest, SessionEnd, SessionStart};
    let codex = AgentKind::Codex;
    assert_eq!(
        read(codex, SessionStart, "codex/hooks/main", "01-SessionStart.json"),
        Some(conversation(codex, SessionStart, CODEX_TRANSCRIPT, CODEX_MAIN))
    );
    assert_eq!(
        read(
            codex,
            PermissionRequest,
            "codex/hooks/main",
            "25-PermissionRequest.json"
        ),
        Some(AgentReport {
            approval: Some(AgentApproval {
                tool: "Bash".to_owned(),
                title: "Do you want to allow this network request to example.com?".to_owned(),
                detail: Some("curl -sI https://example.com".to_owned()),
                tool_use_id: None,
            }),
            ..conversation(codex, PermissionRequest, CODEX_TRANSCRIPT, CODEX_MAIN)
        })
    );
    // the CLI's exit ends every thread it loaded, an earlier run's included
    let other = read(codex, SessionEnd, "codex/hooks/main", "28-SessionEnd.json").unwrap();
    assert_eq!(
        other.conversation_id.as_deref(),
        Some("01a1a300-7e7c-7e10-adfe-2268a316dac4")
    );
}

#[test]
fn a_codex_subagents_events_leave_the_record_and_keep_its_permission_prompt() {
    let codex = AgentKind::Codex;
    assert_eq!(
        read(
            codex,
            HookEvent::UserPromptSubmit,
            "codex/hooks/main",
            "18-UserPromptSubmit.json"
        ),
        None
    );
    let bytes = edited("codex/hooks/main", "25-PermissionRequest.json", |payload| {
        payload.insert("agent_id".to_owned(), json!("01a1a301-8737-70e2-bc57-72fa1f1f57c7"));
    });
    let read = report(codex, HookEvent::PermissionRequest, &bytes).unwrap().unwrap();
    assert_eq!((read.transcript_path, read.conversation_id), (None, None));
    assert_eq!(read.approval.map(|approval| approval.tool), Some("Bash".to_owned()));
}

#[test]
fn a_thread_with_no_transcript_is_ignored() {
    let bytes = edited("codex/hooks/main", "01-SessionStart.json", |payload| {
        payload.insert("transcript_path".to_owned(), Value::Null);
    });
    assert_eq!(report(AgentKind::Codex, HookEvent::SessionStart, &bytes), Ok(None));
}

#[test]
fn every_recorded_payload_of_the_four_events_reads() {
    for (agent, dir) in [
        (AgentKind::Claude, "claude/hooks"),
        (AgentKind::Codex, "codex/hooks/main"),
        (AgentKind::Codex, "codex/hooks/auto-review"),
    ] {
        let mut read = 0;
        for entry in std::fs::read_dir(hooks(dir)).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let Some(event) = name
                .strip_suffix(".json")
                .and_then(|stem| stem.split_once('-'))
                .and_then(|(_, event)| HookEvent::parse(event).ok())
            else {
                continue;
            };
            let bytes = std::fs::read(&path).unwrap();
            assert!(report(agent, event, &bytes).is_ok(), "{dir}/{name}");
            read += 1;
        }
        assert!(read >= 4, "{dir}: {read} payloads");
    }
}

#[test]
fn a_payload_missing_a_field_is_refused_with_its_name() {
    let no_session = edited("claude/hooks", "01-SessionStart.json", |payload| {
        payload.remove("session_id");
    });
    let no_transcript = edited("claude/hooks", "01-SessionStart.json", |payload| {
        payload.remove("transcript_path");
    });
    let no_tool = edited("claude/hooks", "33-PermissionRequest.json", |payload| {
        payload.remove("tool_name");
    });
    let claude = AgentKind::Claude;
    for (event, bytes, error) in [
        (HookEvent::SessionStart, no_session, "hook payload has no session_id"),
        (
            HookEvent::SessionStart,
            no_transcript,
            "hook payload has no transcript_path",
        ),
        (HookEvent::PermissionRequest, no_tool, "hook payload has no tool_name"),
        (
            HookEvent::SessionStart,
            b"not json".to_vec(),
            "hook payload is not a JSON object",
        ),
        (
            HookEvent::SessionStart,
            b"[1]".to_vec(),
            "hook payload is not a JSON object",
        ),
    ] {
        assert_eq!(report(claude, event, &bytes), Err(error.to_owned()));
    }
}

#[test]
fn a_transcript_outside_the_agents_tree_is_refused() {
    for (agent, path, error) in [
        (
            AgentKind::Claude,
            "/home/user/project/notes.jsonl",
            "transcript path is outside a Claude projects/ tree: /home/user/project/notes.jsonl",
        ),
        (
            AgentKind::Claude,
            "/home/user/.claude/projects/p/../../../etc/x.jsonl",
            "transcript path is outside a Claude projects/ tree: /home/user/.claude/projects/p/../../../etc/x.jsonl",
        ),
        (
            AgentKind::Claude,
            "/home/user/.claude/projects/p/x.txt",
            "transcript path is outside a Claude projects/ tree: /home/user/.claude/projects/p/x.txt",
        ),
        (
            AgentKind::Claude,
            "home/user/.claude/projects/p/x.jsonl",
            "transcript path is outside a Claude projects/ tree: home/user/.claude/projects/p/x.jsonl",
        ),
        // Codex's tree is not Claude's
        (
            AgentKind::Claude,
            CODEX_TRANSCRIPT,
            &format!("transcript path is outside a Claude projects/ tree: {CODEX_TRANSCRIPT}"),
        ),
        (
            AgentKind::Codex,
            "/tmp/rollout.jsonl",
            "transcript path is outside a Codex sessions/ tree: /tmp/rollout.jsonl",
        ),
    ] {
        assert_eq!(check_transcript(agent, path), Err(error.to_owned()), "{path}");
    }
    let outside = edited("claude/hooks", "01-SessionStart.json", |payload| {
        payload.insert("transcript_path".to_owned(), json!("/etc/passwd.jsonl"));
    });
    assert_eq!(
        report(AgentKind::Claude, HookEvent::SessionStart, &outside),
        Err("transcript path is outside a Claude projects/ tree: /etc/passwd.jsonl".to_owned())
    );
}

#[test]
fn the_config_dir_is_the_trees_parent() {
    assert_eq!(
        config_dir(AgentKind::Claude, Path::new(CLAUDE_TRANSCRIPT)),
        Some(PathBuf::from("/home/user/.claude"))
    );
    assert_eq!(
        config_dir(AgentKind::Codex, Path::new(CODEX_TRANSCRIPT)),
        Some(PathBuf::from("/home/user/.codex"))
    );
    // a subagent's file is not a conversation's transcript
    let subagent = "/home/user/.claude/projects/-home-user-project/f559ab9f/subagents/agent-a.jsonl";
    assert_eq!(config_dir(AgentKind::Claude, Path::new(subagent)), None);
}

#[test]
fn event_names_parse_and_an_unknown_one_lists_them() {
    for event in HookEvent::ALL {
        assert_eq!(HookEvent::parse(event.name()), Ok(event));
    }
    assert_eq!(
        HookEvent::parse("Stop"),
        Err("unknown agent event: Stop (SessionStart|UserPromptSubmit|PermissionRequest|SessionEnd)".to_owned())
    );
}
