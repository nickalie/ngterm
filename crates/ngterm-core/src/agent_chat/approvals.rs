//! The permission prompts `ngtermctl agent chat` interleaves with a transcript's events. A transcript never records
//! a dialog that is waiting, so a prompt comes from the pane's agent record (`agent.get`'s live approval) and is shown
//! as an `approval` event; an `approval_done` with the same id ends it.
//!
//! - The record re-read without it, or with another candidate, ends the shown prompt: the pane's status left
//!   `blocked`, or the agent asked again.
//! - The transcript answers it sooner: the call's result written (a denial writes one, as an error) or an interrupt
//!   notice (Esc). Claude has no "permission answered" hook, so its status may hold `blocked` well after the answer;
//!   the candidate the transcript answered is never shown again, however long the record keeps it.
//! - The transcript may answer it before it is shown: a backlog or a poll holding the result can be read before the
//!   record shows the candidate. So the calls read are kept (since the last `reset`; a finished one until the main
//!   agent's next prompt), and a candidate whose call they hold only finished is not shown. That is decided again at
//!   each look at the record, not latched: a new prompt for the same call as an earlier finished one shows once the
//!   transcript has written its call.
//!
//! A `PermissionRequest` payload names no tool use (Task 1's fixtures), so a result is matched to the prompt by the
//! call: the tool's kind and its one-line title, which both read from the call's command, path or URL. The name is not
//! compared, since Codex reports a `Bash` prompt for a call its rollout names `exec_command`. When the payload does
//! name the tool use, the id alone decides.
//!
//! The payload does not say whether the main agent or a subagent asked either, so a title match also weighs the
//! prompt's owner: the transcript (the main one, or a subagent's) holding a running call that matches, the main one
//! first. Only the owner's result answers the prompt; while no running call matches, any transcript's result does.
//!
//! An approval's `cursor` is where the stream stands: the newest position the main transcript's events reached, so
//! paging by cursor never skips a line before it.

use std::collections::HashMap;

use ngterm_proto::ControlAgentApproval;

use super::claude::tool_kind;
use super::{Approval, ChatEvent, EventBody, NoticeKind, ToolKind, ToolStatus, title_line};

/// The prompts shown so far in one chat stream.
#[derive(Debug, Default)]
pub struct Approvals {
    shown: Option<Shown>,
    /// The last candidate the transcript answered.
    answered: Option<String>,
    /// Where the stream stands.
    cursor: u64,
    /// The calls read since the last `reset`, by tool use id; a finished one is dropped at the main agent's next
    /// prompt.
    calls: HashMap<String, Call>,
}

/// The prompt shown now.
#[derive(Debug, Clone)]
struct Shown {
    id: String,
    approval: Approval,
    owner: Owner,
}

/// Whose call a prompt is: which transcript's result answers it by title.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Owner {
    /// No running call matches it yet: any transcript's result does.
    Unknown,
    /// The main agent's (`None`), or the subagent's under that task tool.
    Known(Option<String>),
}

/// A call the transcript holds.
#[derive(Debug)]
struct Call {
    /// The task tool a subagent's call is under; `None` for the main agent's.
    parent: Option<String>,
    kind: ToolKind,
    title: String,
    running: bool,
}

/// What the calls read hold of a candidate's call.
enum Seen {
    /// Nothing: the call is not in the window, or not written yet.
    Nothing,
    /// The call, still running, in the main transcript (`None`) or a subagent's.
    Running(Option<String>),
    /// The call, finished: the prompt was answered.
    Finished,
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
            self.calls.clear();
            return None;
        }
        if event.parent.is_none() {
            self.cursor = self.cursor.max(event.cursor);
        }
        self.note(event);
        let shown = self.shown.take_if(|shown| answers(shown, event))?;
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
            let approval = Approval {
                tool: live.tool.clone(),
                title: live.title.clone(),
                detail: live.detail.clone(),
                tool_use_id: live.tool_use_id.clone(),
            };
            let owner = match self.seen(&approval) {
                Seen::Finished => return events,
                Seen::Running(parent) => Owner::Known(parent),
                Seen::Nothing => Owner::Unknown,
            };
            let shown = Shown { id, approval, owner };
            events.push(self.event(shown.clone(), false));
            self.shown = Some(shown);
        }
        events
    }

    /// Keeps what `event` says of the calls: a tool's state, an interrupt ending the running ones (the main
    /// agent's ends every transcript's), the main agent's prompt starting a turn. A running call matching the shown
    /// prompt names its owner, the main agent's over a subagent's.
    fn note(&mut self, event: &ChatEvent) {
        match &event.body {
            EventBody::Tool(tool) => {
                let running = tool.status == ToolStatus::Running;
                if let Some(shown) = self.shown.as_mut()
                    && running
                    && shown.owner != Owner::Known(None)
                    && same_call(&shown.approval, tool.kind, &tool.title)
                {
                    shown.owner = Owner::Known(event.parent.clone());
                }
                let call = Call {
                    parent: event.parent.clone(),
                    kind: tool.kind,
                    title: tool.title.clone(),
                    running,
                };
                self.calls.insert(event.id.clone(), call);
            }
            EventBody::Notice {
                kind: NoticeKind::Interrupt,
                ..
            } => {
                let main = event.parent.is_none();
                for call in self.calls.values_mut() {
                    if main || call.parent == event.parent {
                        call.running = false;
                    }
                }
            }
            EventBody::User { .. } if event.parent.is_none() => self.calls.retain(|_, call| call.running),
            _ => {}
        }
    }

    /// What the calls read hold of `approval`'s call: by its tool use id when known, else by kind and title, a
    /// running main call before a subagent's.
    fn seen(&self, approval: &Approval) -> Seen {
        if let Some(id) = &approval.tool_use_id {
            return match self.calls.get(id) {
                Some(call) if call.running => Seen::Running(call.parent.clone()),
                Some(_) => Seen::Finished,
                None => Seen::Nothing,
            };
        }
        let matching: Vec<&Call> = self
            .calls
            .values()
            .filter(|call| same_call(approval, call.kind, &call.title))
            .collect();
        let running = || matching.iter().filter(|call| call.running);
        match running()
            .find(|call| call.parent.is_none())
            .or_else(|| running().next())
        {
            Some(call) => Seen::Running(call.parent.clone()),
            None if matching.is_empty() => Seen::Nothing,
            None => Seen::Finished,
        }
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

/// Whether `event` answers the prompt `shown`: an interrupt, or the call's result (by title, only its owner's).
fn answers(shown: &Shown, event: &ChatEvent) -> bool {
    match &event.body {
        EventBody::Notice {
            kind: NoticeKind::Interrupt,
            ..
        } => true,
        EventBody::Tool(tool) if tool.status != ToolStatus::Running => match &shown.approval.tool_use_id {
            Some(id) => event.id == *id,
            None => shown.owns(event) && same_call(&shown.approval, tool.kind, &tool.title),
        },
        _ => false,
    }
}

impl Shown {
    /// Whether `event` comes from the transcript the prompt's call is in, as far as that is known.
    fn owns(&self, event: &ChatEvent) -> bool {
        match &self.owner {
            Owner::Unknown => true,
            Owner::Known(parent) => *parent == event.parent,
        }
    }
}

/// Whether a call of `kind` titled `title` is the one `approval` asks about.
fn same_call(approval: &Approval, kind: ToolKind, title: &str) -> bool {
    kind == tool_kind(&approval.tool) && title == title_line(approval.detail.as_deref().unwrap_or(&approval.tool))
}
