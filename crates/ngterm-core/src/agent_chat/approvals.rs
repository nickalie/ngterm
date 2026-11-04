//! The permission prompts `ngtermctl agent chat` interleaves with a transcript's events. A transcript never records
//! a dialog that is waiting, so a prompt comes from the pane's agent record (`agent.get`'s live approval) and is shown
//! as an `approval` event; an `approval_done` with the same id ends it.
//!
//! - The record re-read without it, or with another candidate, ends the shown prompt: the pane's status left
//!   `blocked`, or the agent asked again.
//! - The transcript answers it sooner: the call's result written (a denial writes one, as an error) or an interrupt
//!   notice (Esc). Claude has no "permission answered" hook, so its status may hold `blocked` well after the answer;
//!   the candidate the transcript answered is never shown again, however long the record keeps it.
//!
//! A `PermissionRequest` payload names no tool use (Task 1's fixtures), so a result is matched to the prompt by the
//! call: the tool's kind and its one-line title, which both read from the call's command, path or URL. The name is not
//! compared, since Codex reports a `Bash` prompt for a call its rollout names `exec_command`. When the payload does
//! name the tool use, the id alone decides.
//!
//! An approval's `cursor` is where the stream stands: the newest position the main transcript's events reached, so
//! paging by cursor never skips a line before it.

use ngterm_proto::ControlAgentApproval;

use super::claude::tool_kind;
use super::{Approval, ChatEvent, EventBody, NoticeKind, ToolStatus, title_line};

/// The prompts shown so far in one chat stream.
#[derive(Debug, Default)]
pub struct Approvals {
    shown: Option<Shown>,
    /// The last candidate the transcript answered.
    answered: Option<String>,
    /// Where the stream stands.
    cursor: u64,
}

/// The prompt shown now.
#[derive(Debug, Clone)]
struct Shown {
    id: String,
    approval: Approval,
}

impl Approvals {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes an event read from the transcript, in order: the `approval_done` it makes when it answers the prompt
    /// shown, which goes right after it. A `reset` forgets the prompt shown, since the viewer drops what it showed:
    /// one still live shows again.
    pub fn read(&mut self, event: &ChatEvent) -> Option<ChatEvent> {
        if matches!(event.body, EventBody::Reset) {
            self.shown = None;
            self.cursor = 0;
            return None;
        }
        if event.parent.is_none() {
            self.cursor = self.cursor.max(event.cursor);
        }
        let shown = self.shown.take_if(|shown| answers(&shown.approval, event))?;
        self.answered = Some(shown.id.clone());
        Some(self.event(shown, true))
    }

    /// Takes the record's live approval now: the events that make the viewer's prompt match it.
    pub fn observe(&mut self, live: Option<&ControlAgentApproval>) -> Vec<ChatEvent> {
        let id = live.map(|live| format!("approval-{}", live.since));
        if self.shown.as_ref().map(|shown| &shown.id) == id.as_ref() {
            return Vec::new();
        }
        let mut events: Vec<ChatEvent> = self
            .shown
            .take()
            .map(|shown| self.event(shown, true))
            .into_iter()
            .collect();
        if let (Some(live), Some(id)) = (live, id)
            && self.answered.as_ref() != Some(&id)
        {
            let shown = Shown {
                id,
                approval: Approval {
                    tool: live.tool.clone(),
                    title: live.title.clone(),
                    detail: live.detail.clone(),
                    tool_use_id: live.tool_use_id.clone(),
                },
            };
            events.push(self.event(shown.clone(), false));
            self.shown = Some(shown);
        }
        events
    }

    fn event(&self, shown: Shown, done: bool) -> ChatEvent {
        ChatEvent {
            id: shown.id,
            cursor: self.cursor,
            parent: None,
            body: if done {
                EventBody::ApprovalDone(shown.approval)
            } else {
                EventBody::Approval(shown.approval)
            },
        }
    }
}

/// Whether `event` answers `approval`: an interrupt, or the call's result.
fn answers(approval: &Approval, event: &ChatEvent) -> bool {
    match &event.body {
        EventBody::Notice {
            kind: NoticeKind::Interrupt,
            ..
        } => true,
        EventBody::Tool(tool) if tool.status != ToolStatus::Running => match &approval.tool_use_id {
            Some(id) => event.id == *id,
            None => {
                tool.kind == tool_kind(&approval.tool)
                    && tool.title == title_line(approval.detail.as_deref().unwrap_or(&approval.tool))
            }
        },
        _ => false,
    }
}
