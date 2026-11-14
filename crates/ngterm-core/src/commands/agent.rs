//! `agent.report`, `agent.get` and `agent.locate` (Rust-only, `conformance/docs/rust-build.md` > Rust-only
//! extensions): the origin's per-pane agent record, which the agent-status hooks fill through `ngtermctl agent hook`
//! and the Android chat view reads (`docs/plans/20261103-android-agent-chat.md`).
//!
//! A record is keyed by pane identity (`NGTERM_PANE_ID`), so it follows its pane through a swap or a promotion. It
//! holds the agent, its conversation (the transcript and the id) and the last permission prompt the agent asked for,
//! as a candidate. It lives in memory only: never in the snapshot or the `tree`, and dropped once its pane identity
//! leaves the model ([`Core::sync_agent_records`], run on each commit). A pane an undoable close holds has not left
//! it: an undo puts back the same pane, so its record is dropped only once the close is final.
//!
//! - `SessionStart` and `UserPromptSubmit` set the conversation, or re-assert it unchanged (an origin restarted under
//!   a running agent learns it on the next prompt). Either drops a candidate: no permission dialog is open then.
//! - `PermissionRequest` stores the candidate. Claude's `AskUserQuestion` fires one as well, carrying its questions:
//!   Claude Code writes the call to its transcript only once it is answered, so this candidate is what shows the
//!   question while its picker waits (`agent_chat::approvals`).
//! - `SessionEnd` clears the record when it names the record's conversation: Codex ends every thread it loaded when
//!   the CLI exits, earlier runs' included.
//!
//! A candidate is an approval only while the pane's status is `blocked`, which the existing adapters set and clear;
//! once its pane was seen holding the block, the status leaving `blocked` ends it. The other pane's block taking the
//! session's one status over ends nothing: it only hides the candidate until its own pane writes `blocked` again (the
//! status is shared, so its clearing then is the other pane's). A `session.status` write from the pane that is not
//! `blocked` ends it too, shown or not ([`Core::note_agent_status_write`]): the agent moved on, so no dialog of its
//! is open. That ends a call Codex's Auto Review approved, which fires `PermissionRequest` but never sets `blocked`
//! (the watcher reports `blocked` only for a prompt on screen): the command's `PostToolUse` writes `active` before a
//! later, unrelated block (a question at `Stop`, a `request_user_input` prompt) could make it live. One case is left
//! open: a call the guardian denies runs no tool, so a turn that ends at once with a question finds it still there.
//!
//! `agent.locate` finds the pane by its conversation, for a hook that runs outside the pane: Claude Code can carry a
//! conversation on in a background process of its daemon, whose hooks inherit the daemon's `NGTERM_*`, not the
//! pane's (`agent_chat::moved`). The pane is the one whose record holds the conversation, or holds the one it moved
//! from; the record then follows the move, so the chat view and the next hook find the conversation where it is.
//!
//! Whether the agent still runs is read lazily: `agent.get` on a pane whose foreground is back to a shell clears the
//! record, since no event fires when the program exits, and so does building the presentation stream's `agent` frame
//! ([`Core::presented_agent`], `remote::agents`). A PowerShell on Linux or macOS settles nothing: it has no job
//! control, so its group reads as the shell while the programs it started run in it (`restore::capture`).

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::path::Path;
use std::time::SystemTime;

use ngterm_proto::command::AgentCmd;
use ngterm_proto::{AgentApproval, AgentReport, ControlAgent, ControlAgentApproval, ControlResult, Request};
use uuid::Uuid;

use super::{Outcome, args_of, not_found, parse_pane, unknown_pane_id, with_id};
use crate::Core;
use crate::agent_chat::hook::{HookEvent, check_transcript};
use crate::agent_chat::moved::moved_to;
use crate::agent_chat::{AgentKind, NO_AGENT};
use crate::command_restore::PaneForeground;
use crate::error::{ControlError, Noun};
use crate::live::shell_integration::is_powershell;
use crate::model::{AgentStatus, AppStore, Session, StatusPane, WindowLibrary};
use crate::pane_env::uuid_string;
use crate::projection::epoch_seconds;
use crate::resolve::{ACTIVE, Resolver};

/// Every pane's agent record, by pane identity.
#[derive(Debug, Default)]
pub(crate) struct AgentRecords {
    by_pane: HashMap<Uuid, AgentRecord>,
}

/// What the hooks reported about the agent one pane runs.
#[derive(Debug, Clone, PartialEq)]
struct AgentRecord {
    agent: AgentKind,
    transcript: String,
    conversation: String,
    /// When the conversation was last set.
    updated_at: SystemTime,
    candidate: Option<Candidate>,
}

/// The last permission prompt the agent asked for.
#[derive(Debug, Clone, PartialEq)]
struct Candidate {
    approval: AgentApproval,
    /// When the hook reported it.
    since: SystemTime,
    /// The pane held the session's `blocked` status when last seen: its dialog is on screen, so the status leaving
    /// `blocked` answered it.
    held: bool,
}

impl AgentRecord {
    fn holds(&self, agent: AgentKind, transcript: &str, conversation: &str) -> bool {
        self.agent == agent && self.transcript == transcript && self.conversation == conversation
    }

    /// The candidate against the pane's status now: held while its pane holds the block, it ends when the status
    /// leaves `blocked` from there; the other pane's block only hides it.
    fn observe(&mut self, block: Block) {
        let ended = match self.candidate.as_mut() {
            Some(candidate) => {
                let ended = candidate.held && block == Block::None;
                candidate.held = block == Block::This;
                ended
            }
            None => false,
        };
        if ended {
            self.candidate = None;
        }
    }

    /// A `session.status` write of `status` from this record's pane: anything but `blocked` ends the candidate.
    fn note_status_write(&mut self, status: AgentStatus) {
        if status != AgentStatus::Blocked {
            self.candidate = None;
        }
    }

    /// The approval live now: a candidate while the pane is `blocked`.
    fn live_approval(&self, blocked: bool) -> Option<&Candidate> {
        self.candidate.as_ref().filter(|_| blocked)
    }
}

/// The pane a request addresses.
struct Addressed {
    session: Uuid,
    identity: Uuid,
    role: StatusPane,
}

/// Which pane of a session holds its `blocked` status, as one pane sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Block {
    This,
    Other,
    None,
}

/// Which pane of `session` holds a `blocked` status, seen from `role`'s (an untagged one is the main pane's).
fn block_of(session: &Session, role: StatusPane) -> Block {
    let indicator = &session.agent_indicator;
    if indicator.status != AgentStatus::Blocked {
        Block::None
    } else if indicator
        .normalized_pane(session.split.exists)
        .unwrap_or(StatusPane::Left)
        == role
    {
        Block::This
    } else {
        Block::Other
    }
}

/// Whether `role`'s pane of `session` owns a `blocked` status.
fn pane_blocked(session: &Session, role: StatusPane) -> bool {
    block_of(session, role) == Block::This
}

/// The identity of `session`'s pane a status tag names, as [`AgentIndicator::normalized_pane`] folds it (an untagged
/// one is the main pane's); `None` for the scratch pane.
///
/// [`AgentIndicator::normalized_pane`]: crate::model::AgentIndicator::normalized_pane
pub(crate) fn status_writer(session: &Session, tag: Option<StatusPane>) -> Option<Uuid> {
    match (tag, session.split_pane_identity) {
        (Some(StatusPane::Right), Some(split)) if session.split.exists => Some(split),
        (None | Some(StatusPane::Left | StatusPane::Right), _) => Some(session.pane_identity),
        (Some(StatusPane::Scratch), _) => None,
    }
}

/// Whether `foreground` shows that the pane's agent exited: a shell holds the foreground again. A PowerShell on Linux
/// or macOS has no job control, so it reads as its shell while the agent it started runs in its group
/// (`restore::capture`'s module docs); on Windows the pane's stand-in is its oldest program, so a shell there is one.
pub(super) fn agent_exited(foreground: Option<&PaneForeground>, windows: bool) -> bool {
    match foreground {
        Some(PaneForeground::ForegroundShell(name)) => windows || !is_powershell(name),
        Some(PaneForeground::Program(_)) | None => false,
    }
}

/// The open session and role of the pane `identity` names.
fn locate(library: &WindowLibrary, identity: Uuid) -> Option<(&Session, StatusPane)> {
    library.all_open_sessions().into_iter().find_map(|session| {
        if session.pane_identity == identity {
            Some((session, StatusPane::Left))
        } else if session.split_pane_identity == Some(identity) {
            Some((session, StatusPane::Right))
        } else {
            None
        }
    })
}

/// Whether an undoable close holds the pane `identity` names: out of the tree for the grace, its terminal alive and put
/// back by an undo, so its record waits for the close to become final.
fn held_by_pending_close(library: &WindowLibrary, identity: Uuid) -> bool {
    library
        .open_ids()
        .into_iter()
        .filter_map(|id| library.store(id))
        .flat_map(AppStore::pending_close_members)
        .any(|member| member.session.pane_identity == identity || member.session.split_pane_identity == Some(identity))
}

/// The role of `session`'s pane whose identity `token` spells (its `NGTERM_PANE_ID`).
fn role_of_token(session: &Session, token: &str) -> Option<StatusPane> {
    let identity = Uuid::parse_str(token).ok()?;
    if session.pane_identity == identity {
        Some(StatusPane::Left)
    } else if session.split_pane_identity == Some(identity) {
        Some(StatusPane::Right)
    } else {
        None
    }
}

/// The pane `--pane-id` and `--pane` address in the target session: a live pane identity wins over the role named
/// beside it, an unknown one falls back to that role, and an unknown one alone is refused; neither is the main pane.
/// A `right` role without a split is the main pane, as a status tag folds (a promoted survivor keeps `right` in its
/// environment). The scratch pane keeps no record.
fn addressed(core: &Core, request: &Request) -> Result<Addressed, ControlError> {
    let args = args_of(request);
    let pane = parse_pane(args.pane.as_deref())?;
    let target = request.target.as_deref();
    let resolved = Resolver::new(core.library()).session(target, args.window.as_deref())?;
    let session = core
        .library()
        .store(resolved.window)
        .and_then(|store| store.session(resolved.session))
        .ok_or_else(|| not_found(Noun::Session, target.unwrap_or(ACTIVE)))?;
    let token = args.pane_id.as_deref().filter(|token| !token.is_empty());
    let role = match (token.and_then(|token| role_of_token(session, token)), pane, token) {
        (Some(role), _, _) | (None, Some(role), _) => role,
        (None, None, Some(token)) => return Err(unknown_pane_id(token)),
        (None, None, None) => StatusPane::Left,
    };
    let (identity, role) = match (role, session.split_pane_identity) {
        (StatusPane::Right, Some(split)) => (split, StatusPane::Right),
        (StatusPane::Left | StatusPane::Right, _) => (session.pane_identity, StatusPane::Left),
        (StatusPane::Scratch, _) => {
            return Err(ControlError::rejected(
                "an agent record is kept for the left or right pane",
            ));
        }
    };
    Ok(Addressed {
        session: resolved.session,
        identity,
        role,
    })
}

/// Answers an `agent.*` command.
pub(crate) fn dispatch(core: &mut Core, cmd: AgentCmd, request: &Request) -> Outcome {
    match cmd {
        AgentCmd::Report => report(core, request),
        AgentCmd::Get => get(core, request),
        AgentCmd::Locate => locate_conversation(core, request),
    }
}

/// `agent.locate`'s refusal when no pane's record holds the conversation or the one it moved from.
pub const NO_PANE: &str = "no pane holds this conversation";

/// `agent.locate`: the pane whose record holds the report's conversation, or the one it moved from (the record then
/// follows the move); answers the session id and the record, with the pane's identity.
fn locate_conversation(core: &mut Core, request: &Request) -> Outcome {
    let report = args_of(request)
        .report
        .as_ref()
        .ok_or_else(|| ControlError::rejected("agent.locate requires a report"))?;
    let agent = AgentKind::from_name(&report.agent)
        .ok_or_else(|| ControlError::rejected(format!("invalid agent: {} (claude|codex)", report.agent)))?;
    let (transcript, conversation) = match (report.transcript_path.as_deref(), report.conversation_id.as_deref()) {
        (Some(transcript), Some(conversation)) if !transcript.is_empty() && !conversation.is_empty() => {
            (transcript, conversation)
        }
        _ => {
            return Err(ControlError::rejected(
                "agent.locate requires transcriptPath and conversationId",
            ));
        }
    };
    check_transcript(agent, transcript).map_err(ControlError::rejected)?;
    core.sync_agent_records();
    let no_pane = || ControlError::rejected(NO_PANE);
    let records = &core.agents.by_pane;
    let newest = |held: &dyn Fn(&AgentRecord) -> bool| {
        records
            .iter()
            .filter(|(_, record)| record.agent == agent && held(record))
            .max_by_key(|(_, record)| record.updated_at)
            .map(|(identity, _)| *identity)
    };
    let identity = newest(&|record| record.conversation == conversation)
        .or_else(|| {
            newest(&|record| agent == AgentKind::Claude && moved_to(Path::new(&record.transcript), conversation))
        })
        .ok_or_else(no_pane)?;
    let (session, role) = locate(core.library(), identity)
        .map(|(session, role)| (session.id(), role))
        .ok_or_else(no_pane)?;
    let record = set_conversation(
        &mut core.agents.by_pane,
        identity,
        agent,
        transcript,
        conversation,
        SystemTime::now(),
    );
    Ok(ControlResult {
        agent: Some(ControlAgent {
            agent: record.agent.name().to_owned(),
            pane: role.as_str().to_owned(),
            transcript_path: record.transcript.clone(),
            conversation_id: record.conversation.clone(),
            updated_at: epoch_seconds(record.updated_at),
            approval: None,
            pane_id: Some(uuid_string(identity)),
        }),
        ..with_id(session)
    })
}

/// The report's transcript and conversation id, both or neither; `required` refuses neither.
fn conversation_of(
    report: &AgentReport,
    event: HookEvent,
    required: bool,
) -> Result<Option<(&str, &str)>, ControlError> {
    match (report.transcript_path.as_deref(), report.conversation_id.as_deref()) {
        (Some(transcript), Some(conversation)) if !transcript.is_empty() && !conversation.is_empty() => {
            Ok(Some((transcript, conversation)))
        }
        (None, None) if !required => Ok(None),
        _ => Err(ControlError::rejected(format!(
            "agent.report {} requires transcriptPath and conversationId",
            event.name()
        ))),
    }
}

/// `agent.report`: one hook's report applied to the addressed pane's record; answers the session id.
fn report(core: &mut Core, request: &Request) -> Outcome {
    let report = args_of(request)
        .report
        .as_ref()
        .ok_or_else(|| ControlError::rejected("agent.report requires a report"))?;
    let agent = AgentKind::from_name(&report.agent)
        .ok_or_else(|| ControlError::rejected(format!("invalid agent: {} (claude|codex)", report.agent)))?;
    let event = HookEvent::parse(&report.event).map_err(ControlError::rejected)?;
    if let Some(transcript) = report.transcript_path.as_deref() {
        check_transcript(agent, transcript).map_err(ControlError::rejected)?;
    }
    let pane = addressed(core, request)?;
    core.sync_agent_records();
    let blocked = locate(core.library(), pane.identity).is_some_and(|(session, role)| pane_blocked(session, role));
    let records = &mut core.agents.by_pane;
    let now = SystemTime::now();
    match event {
        HookEvent::SessionStart | HookEvent::UserPromptSubmit => {
            if let Some((transcript, conversation)) = conversation_of(report, event, true)? {
                set_conversation(records, pane.identity, agent, transcript, conversation, now).candidate = None;
            }
        }
        HookEvent::PermissionRequest => {
            let approval = report
                .approval
                .clone()
                .ok_or_else(|| ControlError::rejected("agent.report PermissionRequest requires an approval"))?;
            if let Some((transcript, conversation)) = conversation_of(report, event, false)? {
                set_conversation(records, pane.identity, agent, transcript, conversation, now);
            }
            let record = records
                .get_mut(&pane.identity)
                .ok_or_else(|| ControlError::rejected(NO_AGENT))?;
            record.candidate = Some(Candidate {
                approval,
                since: now,
                held: blocked,
            });
        }
        HookEvent::SessionEnd => {
            let conversation = report
                .conversation_id
                .as_deref()
                .ok_or_else(|| ControlError::rejected("agent.report SessionEnd requires a conversationId"))?;
            if records
                .get(&pane.identity)
                .is_some_and(|record| record.conversation == conversation)
            {
                records.remove(&pane.identity);
            }
        }
    }
    Ok(with_id(pane.session))
}

/// The pane's record for this conversation: the one it holds when unchanged, else a new one with no candidate.
fn set_conversation<'a>(
    records: &'a mut HashMap<Uuid, AgentRecord>,
    identity: Uuid,
    agent: AgentKind,
    transcript: &str,
    conversation: &str,
    now: SystemTime,
) -> &'a mut AgentRecord {
    let fresh = || AgentRecord {
        agent,
        transcript: transcript.to_owned(),
        conversation: conversation.to_owned(),
        updated_at: now,
        candidate: None,
    };
    match records.entry(identity) {
        Entry::Occupied(entry) if entry.get().holds(agent, transcript, conversation) => entry.into_mut(),
        Entry::Occupied(mut entry) => {
            entry.insert(fresh());
            entry.into_mut()
        }
        Entry::Vacant(entry) => entry.insert(fresh()),
    }
}

/// `agent.get`: the addressed pane's record and its live approval, once the lazy foreground check passed.
fn get(core: &mut Core, request: &Request) -> Outcome {
    let pane = addressed(core, request)?;
    core.sync_agent_records();
    let (session, role) = locate(core.library(), pane.identity).ok_or_else(|| ControlError::rejected(NO_AGENT))?;
    let back_to_shell = agent_exited(core.pane_foreground(session, role).as_ref(), cfg!(windows));
    let blocked = pane_blocked(session, role);
    if back_to_shell {
        core.agents.by_pane.remove(&pane.identity);
    }
    let record = core
        .agents
        .by_pane
        .get(&pane.identity)
        .ok_or_else(|| ControlError::rejected(NO_AGENT))?;
    let approval = record.live_approval(blocked).map(|candidate| ControlAgentApproval {
        tool: candidate.approval.tool.clone(),
        title: candidate.approval.title.clone(),
        detail: candidate.approval.detail.clone(),
        tool_use_id: candidate.approval.tool_use_id.clone(),
        cwd: candidate.approval.cwd.clone(),
        questions: candidate.approval.questions.clone(),
        since: epoch_seconds(candidate.since),
    });
    Ok(ControlResult {
        agent: Some(ControlAgent {
            agent: record.agent.name().to_owned(),
            pane: pane.role.as_str().to_owned(),
            transcript_path: record.transcript.clone(),
            conversation_id: record.conversation.clone(),
            updated_at: epoch_seconds(record.updated_at),
            approval,
            pane_id: None,
        }),
        ..ControlResult::default()
    })
}

impl Core {
    /// Drops the agent records whose pane identity left the model, and holds each candidate against its pane's
    /// status (the module docs). Runs on each commit, so a status that left `blocked` ends its candidate even when it
    /// is back to `blocked` by the next `agent.get`.
    pub(crate) fn sync_agent_records(&mut self) {
        if self.agents.by_pane.is_empty() {
            return;
        }
        let library = &self.library;
        self.agents
            .by_pane
            .retain(|identity, record| match locate(library, *identity) {
                Some((session, role)) => {
                    record.observe(block_of(session, role));
                    true
                }
                None => held_by_pending_close(library, *identity),
            });
    }

    /// A `session.status` write of `status` from the pane `writer` names ([`status_writer`]), applied or refused: one
    /// that is not `blocked` ends that pane's candidate (the module docs).
    pub(crate) fn note_agent_status_write(&mut self, writer: Option<Uuid>, status: AgentStatus) {
        if let Some(record) = writer.and_then(|identity| self.agents.by_pane.get_mut(&identity)) {
            record.note_status_write(status);
        }
    }

    /// The agent and conversation of the pane `identity` names, after the lazy foreground check `agent.get` makes: a
    /// pane back at its shell loses its record. `None` when it holds none. The presentation stream's `agent` frames
    /// are built from it (`remote::agents`); a pane with no record costs no process read.
    pub(crate) fn presented_agent(&mut self, identity: Uuid) -> Option<(AgentKind, String)> {
        let record = self.agents.by_pane.get(&identity)?;
        let present = (record.agent, record.conversation.clone());
        let exited = locate(self.library(), identity)
            .is_some_and(|(session, role)| agent_exited(self.pane_foreground(session, role).as_ref(), cfg!(windows)));
        if exited {
            self.agents.by_pane.remove(&identity);
            return None;
        }
        Some(present)
    }

    /// How many panes hold an agent record (tests).
    #[cfg(test)]
    pub(crate) fn agent_record_count(&self) -> usize {
        self.agents.by_pane.len()
    }
}
