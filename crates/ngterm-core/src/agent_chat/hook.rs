//! An agent's hook payload, read for the origin's per-pane agent record: `ngtermctl agent hook <agent> <event>` reads
//! the payload the agent writes on a hook's stdin, makes an [`AgentReport`] of it with [`report`] and sends that as
//! `agent.report`.
//!
//! What the payloads carry is recorded in `tests/fixtures/agent_chat/README.md`; the parts this reads:
//!
//! - Both agents name the conversation as `session_id` and its transcript as `transcript_path`. A Codex thread with no
//!   rollout (its memory consolidation) has a null `transcript_path` and is not a conversation the chat can show.
//! - Codex fires a subagent's events in the parent, with `agent_id` and the subagent's own `transcript_path`: such a
//!   payload must not re-point the pane's record, so only its permission prompt is kept.
//! - `PermissionRequest` carries `tool_name` and `tool_input`, and no `tool_use_id`.
//!
//! A transcript path is checked against the agent's tree ([`config_dir`]): Claude writes
//! `<config>/projects/<project>/<id>.jsonl`, Codex `<home>/sessions/<y>/<m>/<d>/rollout-….jsonl`. A path outside such
//! a tree is refused, since the origin later reads the file it names for the phone.

use std::path::{Component, Path, PathBuf};

use ngterm_proto::{AgentApproval, AgentReport};
use serde_json::Value;

use super::claude::{tool_kind, tool_subject, tool_title};
use super::event::OUTPUT_CAP;
use super::{AgentKind, cap, non_empty, title_line};

/// The hook events that reach the agent record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HookEvent {
    /// The agent started, resumed, cleared or compacted a conversation: sets the record.
    SessionStart,
    /// The user sent a prompt: re-asserts the record (an origin restarted under a running agent learns it here).
    UserPromptSubmit,
    /// The agent asks for a permission: the record's approval candidate.
    PermissionRequest,
    /// The conversation ended: clears the record if it is the record's conversation.
    SessionEnd,
}

impl HookEvent {
    /// Every event, in the order the messages list them.
    pub const ALL: [Self; 4] = [
        Self::SessionStart,
        Self::UserPromptSubmit,
        Self::PermissionRequest,
        Self::SessionEnd,
    ];

    /// The agents' own name for the event.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::SessionStart => "SessionStart",
            Self::UserPromptSubmit => "UserPromptSubmit",
            Self::PermissionRequest => "PermissionRequest",
            Self::SessionEnd => "SessionEnd",
        }
    }

    /// Parses an event name.
    ///
    /// # Errors
    ///
    /// `unknown agent event: <name> (…)`, listing the events.
    pub fn parse(name: &str) -> Result<Self, String> {
        Self::ALL.into_iter().find(|event| event.name() == name).ok_or_else(|| {
            let names: Vec<&str> = Self::ALL.into_iter().map(Self::name).collect();
            format!("unknown agent event: {name} ({})", names.join("|"))
        })
    }
}

/// What a hook payload reports for the agent record; `None` for a payload that has nothing for it: a thread with no
/// transcript, or a subagent's event other than a permission prompt.
///
/// # Errors
///
/// The payload is not a JSON object, lacks a field the event needs (`session_id`, `transcript_path`, a permission's
/// `tool_name`), or names a transcript outside the agent's tree.
pub fn report(agent: AgentKind, event: HookEvent, payload: &[u8]) -> Result<Option<AgentReport>, String> {
    let payload: Value = serde_json::from_slice(payload).map_err(|_| "hook payload is not a JSON object".to_owned())?;
    if !payload.is_object() {
        return Err("hook payload is not a JSON object".to_owned());
    }
    let approval = match event {
        HookEvent::PermissionRequest => Some(approval(&payload)?),
        _ => None,
    };
    let mut report = AgentReport {
        agent: agent.name().to_owned(),
        event: event.name().to_owned(),
        approval,
        ..AgentReport::default()
    };
    if non_empty(&payload, "/agent_id").is_some() {
        // a subagent's payload names the subagent's transcript: only its permission prompt is the pane's
        return Ok(report.approval.is_some().then_some(report));
    }
    let transcript = match payload.get("transcript_path") {
        Some(Value::Null) => return Ok(None),
        Some(Value::String(path)) if !path.is_empty() => path,
        _ => return Err("hook payload has no transcript_path".to_owned()),
    };
    check_transcript(agent, transcript)?;
    let conversation = non_empty(&payload, "/session_id").ok_or_else(|| "hook payload has no session_id".to_owned())?;
    report.transcript_path = Some(transcript.clone());
    report.conversation_id = Some(conversation.to_owned());
    Ok(Some(report))
}

/// A `PermissionRequest` payload's prompt: the tool, a title (the agent's own description of the call when it gave
/// one, else the command, path or URL), and that command, path or URL cut to [`OUTPUT_CAP`] bytes as the detail.
fn approval(payload: &Value) -> Result<AgentApproval, String> {
    let tool = non_empty(payload, "/tool_name").ok_or_else(|| "hook payload has no tool_name".to_owned())?;
    let input = payload.get("tool_input").unwrap_or(&Value::Null);
    let kind = tool_kind(tool);
    let title = non_empty(input, "/description").map_or_else(|| tool_title(kind, tool, input), title_line);
    Ok(AgentApproval {
        tool: tool.to_owned(),
        title,
        detail: tool_subject(kind, input).map(|subject| cap(subject, OUTPUT_CAP).0),
        tool_use_id: non_empty(payload, "/tool_use_id").map(str::to_owned),
    })
}

/// Checks that `transcript` lies in `agent`'s tree ([`config_dir`]).
///
/// # Errors
///
/// `transcript path is outside a Claude projects/ tree: <path>`, or the Codex `sessions/` form.
pub fn check_transcript(agent: AgentKind, transcript: &str) -> Result<(), String> {
    if config_dir(agent, Path::new(transcript)).is_some() {
        return Ok(());
    }
    Err(outside_tree(agent, transcript))
}

/// [`check_transcript`]'s refusal of `transcript`.
pub(super) fn outside_tree(agent: AgentKind, transcript: &str) -> String {
    let tree = match agent {
        AgentKind::Claude => "a Claude projects/",
        AgentKind::Codex => "a Codex sessions/",
    };
    format!("transcript path is outside {tree} tree: {transcript}")
}

/// The agent's config directory a transcript lies in, which holds its skills and commands too: for Claude the
/// directory above `projects/<project>/<id>.jsonl` (`CLAUDE_CONFIG_DIR`), for Codex the one above the nearest
/// `sessions/` (`CODEX_HOME`). Deriving it from the transcript keeps the agent's own environment, which a process
/// under another login's environment (`ngtermctl` under ssh) does not see. `None` for a path that is not rooted, has a
/// `.` or `..` component, is not a `.jsonl` file, or lies outside such a tree.
#[must_use]
pub fn config_dir(agent: AgentKind, transcript: &Path) -> Option<PathBuf> {
    let plain = transcript
        .components()
        .all(|component| !matches!(component, Component::CurDir | Component::ParentDir));
    if !transcript.has_root() || !plain || transcript.extension().is_none_or(|ext| ext != "jsonl") {
        return None;
    }
    let tree = match agent {
        AgentKind::Claude => transcript.parent()?.parent().filter(|dir| named(dir, "projects"))?,
        AgentKind::Codex => transcript.ancestors().skip(1).find(|dir| named(dir, "sessions"))?,
    };
    tree.parent().map(Path::to_path_buf)
}

fn named(dir: &Path, name: &str) -> bool {
    dir.file_name().is_some_and(|file| file == name)
}
