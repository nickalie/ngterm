//! Sessions of another machine made and ended from here, an extension beyond upstream, where a viewer only attaches and
//! detaches. Both run that machine's `ngtermctl` once over ssh ([`super::ssh::control_command`], never retried, never
//! done here), as a remote companion does ([`super::companions`]).
//!
//! - New Session, the last row of Attach Remote's session picker and an item of a remote row's menu: the origin creates a
//!   session of its own, keeping its own selection, once it says it runs live sessions, the origin is listed until the
//!   session is attachable, and it attaches here selected. It runs as a step of the Attach Remote flow
//!   ([`super::attach`]), whose panels it shows.
//! - Duplicate Session asks the origin to duplicate its source in its current focused directory, then attaches the
//!   fresh shell beside the source here. The origin selects its copy too, as its plain duplicate command does.
//! - End Session, an item of a remote row's menu the frontend confirms first: the origin closes the session with no undo
//!   window, its programs and companions with it, then the row closes here. Closing a remote row any other way only
//!   detaches.

use std::time::Instant;

use ngterm_proto::{ControlRemoteTree, ControlResult, Response};
use uuid::Uuid;

use super::attach::{ActionPanel, AttachFlow, PROGRESS_SECONDS, Placement, Step, read_remote_tree};
use super::companions::{
    COMPANION_DEADLINE, NOT_ATTACHABLE, PAUSE, READY_BUDGET, origin_created, reply_of, unattached,
};
use super::ssh::control_command;
use super::tree::{RemoteCommandResult, TREE_DEADLINE};
use super::viewer::{Answer, run_off_thread, take_answer};
use crate::Core;

/// The id of the session picker's New Session row, which a far session's upper-case UUID never equals.
pub(super) const NEW_SESSION_ID: &str = "new-session";
/// The label of that row, and what the panels of a create are headed by.
pub(super) const NEW_SESSION: &str = "New Session";
const END_SESSION: &str = "End Session";
/// What a created session is called in the messages about it.
const SESSION_NOUN: &str = "session";
/// The restore mode `restore.mode` reports while each pane runs in a live session.
const LIVE_MODE: &str = "live";

/// One session the origin is creating for this machine.
#[derive(Debug)]
pub(super) struct CreateRun {
    host: String,
    /// The workspace of the row it was asked from; the window's current one otherwise.
    workspace: Option<Uuid>,
    request: CreateRequest,
    stage: CreateStage,
}

#[derive(Debug)]
enum CreateRequest {
    New,
    Duplicate { source: Uuid, origin: String },
}

impl CreateRequest {
    fn action(&self) -> &'static str {
        match self {
            Self::New => NEW_SESSION,
            Self::Duplicate { .. } => "Duplicate Session",
        }
    }

    fn arguments(&self) -> Vec<&str> {
        match self {
            Self::New => vec!["session", "new", "--no-select"],
            Self::Duplicate { origin, .. } => vec!["session", "duplicate", "--target", origin],
        }
    }
}

#[derive(Debug)]
enum CreateStage {
    /// The origin is asked whether it runs live sessions: outside them a session it creates has no daemon, so it could
    /// never attach here and would stay behind there.
    Checking(Answer),
    Creating(Answer),
    Waiting(Waiting),
}

/// The origin is listed until the created session is attachable: every pane's daemon must run first.
#[derive(Debug)]
struct Waiting {
    created: String,
    deadline: Instant,
    /// When the next listing starts.
    due: Instant,
    /// Why it is not attached yet, for the failure if it never is.
    reason: String,
    read: Option<Answer>,
}

/// One End Session waiting for the origin's answer, and the panel it opened over the row.
pub(crate) struct EndRun {
    window: Uuid,
    session: Uuid,
    host: String,
    owned_hud: Option<u64>,
    answer: Answer,
}

/// The text after "gave no answer": what ssh said, if anything.
fn no_answer(result: &RemoteCommandResult, host: &str, rest: &str) -> String {
    let stderr = result.stderr.trim();
    if stderr.is_empty() {
        format!("{host} gave no answer; {rest}")
    } else {
        format!("{host} gave no answer ({stderr}); {rest}")
    }
}

/// The far CLI refused its command line, before sending anything.
fn too_old(host: &str) -> String {
    format!("the ngterm on {host} is too old for this; nothing was created")
}

/// Why `host`'s answer to `restore.mode` rules a new session out; `None` when it runs live sessions.
fn not_live(result: &RemoteCommandResult, host: &str) -> Option<String> {
    if result.refused_arguments() {
        return Some(too_old(host));
    }
    let Some(reply): Option<Response<ControlResult>> = reply_of(result) else {
        return Some(no_answer(result, host, "nothing was created"));
    };
    if !reply.ok {
        let reason = reply
            .error
            .unwrap_or_else(|| format!("the remote command failed on {host}"));
        return Some(format!("{reason}; nothing was created"));
    }
    let active = reply
        .result
        .and_then(|result| result.restore)
        .map(|restore| restore.active);
    match active.as_deref() {
        Some(LIVE_MODE) => None,
        Some(mode) => Some(format!(
            "{host} does not run live sessions (restore mode {mode}), so a session made there could not attach; \
             nothing was created"
        )),
        None => Some(format!("{host} did not name its restore mode; nothing was created")),
    }
}

/// What the origin's close said: `None` when the session ended there, else why it did not. A full receipt counts
/// whatever the transport did afterwards, since it names work the origin did.
fn end_failure(result: &RemoteCommandResult, host: &str) -> Option<String> {
    let Some(reply): Option<Response<ControlResult>> = reply_of(result) else {
        return Some(no_answer(result, host, "whether the session ended there is unknown"));
    };
    (!reply.ok).then(|| {
        reply
            .error
            .unwrap_or_else(|| format!("the remote command failed on {host}"))
    })
}

impl Core {
    /// Duplicate a remote row on its host, where the source's current focused directory is known.
    pub(crate) fn begin_duplicate_remote_session(&mut self, window: Uuid, session: Uuid) -> bool {
        let Some(store) = self.library().store(window) else {
            return false;
        };
        let Some(row) = store.session(session) else {
            return false;
        };
        let Some((host, presentation)) = row.remote_host().zip(row.remote_presentation()) else {
            self.action_failed(
                "Duplicate Session",
                window,
                Some(session),
                "the remote session is not attached",
            );
            return true;
        };
        let host = host.to_owned();
        let request = CreateRequest::Duplicate {
            source: session,
            origin: presentation.binding.remote_session_id.clone(),
        };
        let workspace = store.workspace_for_session(session).map(crate::model::Workspace::id);
        let flow = AttachFlow::new(window, Some(session));
        if let Some(flow) = self.start_create_session(flow, host, workspace, request) {
            self.remote.viewer.attach_flows.push(flow);
        }
        true
    }

    /// New Session on the machine remote row `session` of window `window` came from: the new row lands in that row's
    /// workspace, the waits and a failure in a panel over the row. Whether anything changed.
    pub(crate) fn begin_new_remote_session(&mut self, window: Uuid, session: Uuid) -> bool {
        let Some(store) = self.library().store(window) else {
            return false;
        };
        let Some(host) = store.session(session).and_then(|row| row.remote_host()) else {
            return false;
        };
        let host = host.to_owned();
        let workspace = store.workspace_for_session(session).map(crate::model::Workspace::id);
        let flow = AttachFlow::new(window, Some(session));
        if let Some(flow) = self.start_origin_session(flow, host, workspace) {
            self.remote.viewer.attach_flows.push(flow);
        }
        true
    }

    /// Runs `arguments` of `host`'s `ngtermctl` off the main thread.
    fn ask_origin(&self, host: &str, arguments: &[&str]) -> Option<Answer> {
        let argv = control_command(host, arguments).ok()?;
        run_off_thread(&self.remote.runner, argv, COMPANION_DEADLINE)
    }

    /// Asks `host` for a session, as `flow`'s next step; `None` once the flow ended.
    pub(super) fn start_origin_session(
        &mut self,
        flow: AttachFlow,
        host: String,
        workspace: Option<Uuid>,
    ) -> Option<AttachFlow> {
        self.start_create_session(flow, host, workspace, CreateRequest::New)
    }

    fn start_create_session(
        &mut self,
        mut flow: AttachFlow,
        host: String,
        workspace: Option<Uuid>,
        request: CreateRequest,
    ) -> Option<AttachFlow> {
        flow.action = request.action();
        self.attach_progress(&mut flow, &format!("creating a session on {host}…"));
        let Some(answer) = self.ask_origin(&host, &["restore", "mode"]) else {
            self.attach_failed(&mut flow, &format!("the remote command failed on {host}"));
            return None;
        };
        flow.step = Step::CreatingSession(CreateRun {
            host,
            workspace,
            request,
            stage: CreateStage::Checking(answer),
        });
        Some(flow)
    }

    /// Moves a create on by what has arrived; `None` once the flow ended.
    pub(super) fn advance_origin_session(&mut self, mut flow: AttachFlow, run: CreateRun) -> Option<AttachFlow> {
        let CreateRun {
            host,
            workspace,
            request,
            stage,
        } = run;
        let stage = match stage {
            CreateStage::Checking(answer) => self.step_checking(&mut flow, &host, &request, answer),
            CreateStage::Creating(answer) => self.step_creating(&mut flow, &host, answer),
            CreateStage::Waiting(waiting) => self.step_waiting(&mut flow, &host, workspace, &request, waiting),
        }?;
        flow.step = Step::CreatingSession(CreateRun {
            host,
            workspace,
            request,
            stage,
        });
        Some(flow)
    }

    fn step_checking(
        &mut self,
        flow: &mut AttachFlow,
        host: &str,
        request: &CreateRequest,
        answer: Answer,
    ) -> Option<CreateStage> {
        let Some(result) = take_answer(&answer) else {
            return Some(CreateStage::Checking(answer));
        };
        if let Some(reason) = not_live(&result, host) {
            self.attach_failed(flow, &reason);
            return None;
        }
        // New Session preserves the origin's selection; Duplicate resolves the source and its directory there.
        let Some(answer) = self.ask_origin(host, &request.arguments()) else {
            self.attach_failed(flow, &format!("the remote command failed on {host}"));
            return None;
        };
        Some(CreateStage::Creating(answer))
    }

    fn step_creating(&mut self, flow: &mut AttachFlow, host: &str, answer: Answer) -> Option<CreateStage> {
        let Some(result) = take_answer(&answer) else {
            return Some(CreateStage::Creating(answer));
        };
        let created = if result.refused_arguments() {
            Err(too_old(host))
        } else {
            origin_created(&result, host, SESSION_NOUN)
        };
        let created = match created {
            Ok(created) => created,
            Err(reason) => {
                self.attach_failed(flow, &reason);
                return None;
            }
        };
        self.attach_progress(flow, "attaching the session…");
        let now = self.remote_now();
        Some(CreateStage::Waiting(Waiting {
            created,
            deadline: now + READY_BUDGET,
            due: now,
            reason: NOT_ATTACHABLE.to_owned(),
            read: None,
        }))
    }

    fn step_waiting(
        &mut self,
        flow: &mut AttachFlow,
        host: &str,
        workspace: Option<Uuid>,
        request: &CreateRequest,
        mut waiting: Waiting,
    ) -> Option<CreateStage> {
        let tree = match self.next_listing(host, &mut waiting) {
            Ok(Some(tree)) => tree,
            Ok(None) => return Some(CreateStage::Waiting(waiting)),
            Err(reason) => {
                self.attach_failed(flow, &unattached(SESSION_NOUN, &waiting.created, host, &reason));
                return None;
            }
        };
        let Some(remote) = tree.sessions.iter().find(|session| session.id == waiting.created) else {
            waiting.due = self.remote_now() + PAUSE;
            return Some(CreateStage::Waiting(waiting));
        };
        let placement = self.created_session_placement(flow.window, workspace, request);
        let result = placement
            .ok_or_else(|| "its window here was closed".to_owned())
            .and_then(|placement| {
                self.attach_remote_session(remote, &tree, host, placement)
                    .map_err(|error| error.to_string())
            });
        match result {
            Ok(_) => self.attach_close_hud(flow),
            Err(reason) => self.attach_failed(flow, &unattached(SESSION_NOUN, &waiting.created, host, &reason)),
        }
        None
    }

    /// One step of listing the origin: a listing once one arrived, `None` while waiting, the reason once the wait is
    /// over.
    fn next_listing(&mut self, host: &str, waiting: &mut Waiting) -> Result<Option<ControlRemoteTree>, String> {
        let now = self.remote_now();
        let Some(answer) = waiting.read.take() else {
            if now < waiting.due {
                return Ok(None);
            }
            let remaining = waiting.deadline.saturating_duration_since(now);
            if remaining.is_zero() {
                return Err(waiting.reason.clone());
            }
            let answer = super::attach::start_tree_read_within(self, host, remaining.min(TREE_DEADLINE))
                .map_err(|error| error.to_string())?;
            waiting.read = Some(answer);
            return Ok(None);
        };
        let Some(result) = take_answer(&answer) else {
            waiting.read = Some(answer);
            return Ok(None);
        };
        match read_remote_tree(&result, host) {
            Ok(tree) => Ok(Some(tree)),
            Err(error) => {
                waiting.reason = error;
                waiting.due = now + PAUSE;
                Ok(None)
            }
        }
    }

    fn created_session_placement(
        &self,
        window: Uuid,
        workspace: Option<Uuid>,
        request: &CreateRequest,
    ) -> Option<Placement> {
        if let CreateRequest::Duplicate { source, .. } = request
            && let Some(location) = self.library().store(window)?.session_location(*source)
        {
            return Some(Placement {
                window,
                workspace: location.workspace,
                at: Some(location.index + 1),
                parent: None,
                select: true,
            });
        }
        let kept = workspace.filter(|workspace| {
            self.library()
                .store(window)
                .is_some_and(|store| store.workspace(*workspace).is_some())
        });
        match kept {
            Some(workspace) => Some(Placement {
                window,
                workspace,
                at: None,
                parent: None,
                select: true,
            }),
            None => self.workspace_placement(window, true),
        }
    }

    // MARK: - End Session

    /// End Session on remote row `session` of window `window`: its origin closes the session with no undo window, with the wait
    /// and a failure in a panel over the row, which closes here once the origin says so. Whether anything changed.
    pub(crate) fn begin_end_remote_session(&mut self, window: Uuid, session: Uuid) -> bool {
        if self.remote.viewer.end_flows.iter().any(|run| run.session == session) {
            return true;
        }
        let row = self.library().store(window).and_then(|store| store.session(session));
        let Some((host, origin)) = row.and_then(|row| {
            let host = row.remote_host()?.to_owned();
            Some((host, row.remote_presentation()?.binding.remote_session_id.clone()))
        }) else {
            return false;
        };
        let panel = ActionPanel {
            action: END_SESSION,
            window,
            session,
        };
        let answer = self.ask_origin(&host, &["session", "close", "--target", &origin]);
        let Some(answer) = answer else {
            let reason = format!("the remote command failed on {host}");
            self.action_failed(END_SESSION, window, Some(session), &reason);
            return true;
        };
        let text = format!("ending the session on {host}…");
        let owned_hud = self.open_action_hud(&panel, &text, PROGRESS_SECONDS, true);
        self.remote.viewer.end_flows.push(EndRun {
            window,
            session,
            host,
            owned_hud,
            answer,
        });
        true
    }

    /// Settles every End Session whose answer arrived.
    pub(crate) fn advance_end_flows(&mut self) {
        let runs = std::mem::take(&mut self.remote.viewer.end_flows);
        for run in runs {
            match take_answer(&run.answer) {
                Some(result) => self.finish_end(&run, &result),
                None => self.remote.viewer.end_flows.push(run),
            }
        }
    }

    fn finish_end(&mut self, run: &EndRun, result: &RemoteCommandResult) {
        if let Some(owned) = run.owned_hud {
            self.close_owned_hud(run.session, owned);
        }
        if let Some(reason) = end_failure(result, &run.host) {
            tracing::info!(reason, "a remote session was not ended");
            self.action_failed(END_SESSION, run.window, Some(run.session), &reason);
            return;
        }
        // the origin's own removal may have closed the row first; one parked by a soft close is made final, as Undo would
        // bring back a session that ended; a companion here goes with its parent, as the origin's did
        if let Some(store) = self.library_mut().store_for_session_mut(run.session) {
            store.close_session(run.session);
        } else if let Some(window) = self.library().window_holding_session(run.session)
            && let Some(store) = self.library_mut().store_mut(window)
        {
            store.finalize_pending_close_of_session(run.session);
        }
    }
}
