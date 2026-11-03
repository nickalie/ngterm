//! Sessions of another machine made and ended from here, an extension beyond upstream, where a viewer only attaches and
//! detaches. Both run that machine's `ngtermctl` once over ssh ([`super::ssh::control_command`], never retried, never
//! done here), as a remote companion does ([`super::companions`]).
//!
//! - New Session, the last row of Attach Remote's session picker and an item of a remote row's menu: the origin creates a
//!   session of its own, keeping its own selection, once it says it runs live sessions, the origin is listed until the
//!   session is attachable, and it attaches here selected. It runs as a step of the Attach Remote flow
//!   ([`super::attach`]), whose panels it shows.
//! - End Session, an item of a remote row's menu the frontend confirms first: the origin closes the session with no undo
//!   window, its programs and companions with it, then the row closes here. Closing a remote row any other way only
//!   detaches.

use ngterm_proto::{ControlRemoteSession, ControlRemoteTree};
use uuid::Uuid;

use super::attach::{ActionPanel, AttachFlow, PROGRESS_SECONDS, Placement, Step};
use super::origin_replies::COMPANION_DEADLINE;
use super::origin_replies::{
    ListingWait, NEW_SESSION_ARGUMENTS, RESTORE_MODE_ARGUMENTS, WaitStep, check_restore_mode, end_failure,
    end_session_arguments, session_created,
};
use super::ssh::control_command;
use super::tree::RemoteCommandResult;
use super::viewer::{Answer, run_off_thread, take_answer};
use crate::Core;

/// The id of the session picker's New Session row, which a far session's upper-case UUID never equals.
pub(super) const NEW_SESSION_ID: &str = "new-session";
/// The label of that row, and what the panels of a create are headed by.
pub(super) const NEW_SESSION: &str = "New Session";
const END_SESSION: &str = "End Session";

/// One session the origin is creating for this machine.
#[derive(Debug)]
pub(super) struct CreateRun {
    host: String,
    /// The workspace of the row it was asked from; the window's current one otherwise.
    workspace: Option<Uuid>,
    stage: CreateStage,
}

#[derive(Debug)]
enum CreateStage {
    /// The origin is asked whether it runs live sessions: outside them a session it creates has no daemon, so it could
    /// never attach here and would stay behind there.
    Checking(Answer),
    Creating(Answer),
    Waiting(Waiting),
}

/// The origin is listed until the created session is attachable, and the listing running, if one is.
#[derive(Debug)]
struct Waiting {
    wait: ListingWait,
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

impl Core {
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
        mut flow: AttachFlow,
        host: String,
        workspace: Option<Uuid>,
    ) -> Option<AttachFlow> {
        flow.action = NEW_SESSION;
        self.attach_progress(&mut flow, &format!("creating a session on {host}…"));
        let Some(answer) = self.ask_origin(&host, &RESTORE_MODE_ARGUMENTS) else {
            self.attach_failed(&mut flow, &format!("the remote command failed on {host}"));
            return None;
        };
        flow.step = Step::CreatingSession(CreateRun {
            host,
            workspace,
            stage: CreateStage::Checking(answer),
        });
        Some(flow)
    }

    /// Moves a create on by what has arrived; `None` once the flow ended.
    pub(super) fn advance_origin_session(&mut self, mut flow: AttachFlow, run: CreateRun) -> Option<AttachFlow> {
        let CreateRun { host, workspace, stage } = run;
        let stage = match stage {
            CreateStage::Checking(answer) => self.step_checking(&mut flow, &host, answer),
            CreateStage::Creating(answer) => self.step_creating(&mut flow, &host, answer),
            CreateStage::Waiting(waiting) => self.step_waiting(&mut flow, &host, workspace, waiting),
        }?;
        flow.step = Step::CreatingSession(CreateRun { host, workspace, stage });
        Some(flow)
    }

    fn step_checking(&mut self, flow: &mut AttachFlow, host: &str, answer: Answer) -> Option<CreateStage> {
        let Some(result) = take_answer(&answer) else {
            return Some(CreateStage::Checking(answer));
        };
        if let Err(reason) = check_restore_mode(&result, host) {
            self.attach_failed(flow, &reason);
            return None;
        }
        let Some(answer) = self.ask_origin(host, &NEW_SESSION_ARGUMENTS) else {
            self.attach_failed(flow, &format!("the remote command failed on {host}"));
            return None;
        };
        Some(CreateStage::Creating(answer))
    }

    fn step_creating(&mut self, flow: &mut AttachFlow, host: &str, answer: Answer) -> Option<CreateStage> {
        let Some(result) = take_answer(&answer) else {
            return Some(CreateStage::Creating(answer));
        };
        let created = match session_created(&result, host) {
            Ok(created) => created,
            Err(reason) => {
                self.attach_failed(flow, &reason);
                return None;
            }
        };
        self.attach_progress(flow, "attaching the session…");
        Some(CreateStage::Waiting(Waiting {
            wait: ListingWait::new(created, self.remote_now()),
            read: None,
        }))
    }

    fn step_waiting(
        &mut self,
        flow: &mut AttachFlow,
        host: &str,
        workspace: Option<Uuid>,
        mut waiting: Waiting,
    ) -> Option<CreateStage> {
        let (remote, tree) = match self.next_listing(host, &mut waiting) {
            Ok(Some(listed)) => listed,
            Ok(None) => return Some(CreateStage::Waiting(waiting)),
            Err(reason) => {
                self.attach_failed(flow, &waiting.wait.unattached(host, &reason));
                return None;
            }
        };
        match self.attach_created_session(flow.window, workspace, host, &remote, &tree) {
            Ok(()) => self.attach_close_hud(flow),
            Err(reason) => self.attach_failed(flow, &waiting.wait.unattached(host, &reason)),
        }
        None
    }

    /// One step of listing the origin: the created session and the listing once it is there, `None` while waiting,
    /// the reason once the wait is over.
    fn next_listing(
        &mut self,
        host: &str,
        waiting: &mut Waiting,
    ) -> Result<Option<(ControlRemoteSession, ControlRemoteTree)>, String> {
        let now = self.remote_now();
        let Some(answer) = waiting.read.take() else {
            let within = match waiting.wait.next(now) {
                WaitStep::Wait => return Ok(None),
                WaitStep::GiveUp(reason) => return Err(reason),
                WaitStep::List(within) => within,
            };
            let answer =
                super::attach::start_tree_read_within(self, host, within).map_err(|error| error.to_string())?;
            waiting.read = Some(answer);
            return Ok(None);
        };
        let Some(result) = take_answer(&answer) else {
            waiting.read = Some(answer);
            return Ok(None);
        };
        Ok(waiting.wait.listed(&result, host, now))
    }

    fn attach_created_session(
        &mut self,
        window: Uuid,
        workspace: Option<Uuid>,
        host: &str,
        remote: &ControlRemoteSession,
        tree: &ControlRemoteTree,
    ) -> Result<(), String> {
        let kept = workspace.filter(|workspace| {
            self.library()
                .store(window)
                .is_some_and(|store| store.workspace(*workspace).is_some())
        });
        let placement = match kept {
            Some(workspace) => Some(Placement {
                window,
                workspace,
                at: None,
                parent: None,
                select: true,
            }),
            None => self.workspace_placement(window, true),
        };
        let placement = placement.ok_or("its window here was closed")?;
        self.attach_remote_session(remote, tree, host, placement)
            .map(drop)
            .map_err(|error| error.to_string())
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
        let answer = self.ask_origin(&host, &end_session_arguments(&origin));
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
