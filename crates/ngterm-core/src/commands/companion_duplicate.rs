//! `session.companion.duplicate` and the GUI's Duplicate as Companion (upstream's `duplicateAsCompanion` and
//! `duplicateCompanion` in `agterm/Control/ControlServer+Companions.swift`, the GUI half in
//! `AppActions+Companions.swift`): a companion copying the source's split layout, each pane starting another copy of
//! the program holding it ([`crate::duplicate_replay`]) as its ending creation command, or a plain shell where a shell
//! holds it. Every pane is planned from one fresh live-leader listing before anything is created, so one refused pane
//! leaves the tree as it was.
//!
//! The stored creation command is upstream's POSIX line on macOS and Linux, whatever the login shell (fish's `eval`
//! reads it alike, a backslash being refused already), and `cmd.exe /D /S /C`'s (`ShellDialect::Cmd`) on Windows. The
//! copy keeps the argv beside it ([`crate::model::PaneLaunch::argv`]), which each launch quotes for the shell that
//! runs it then ([`crate::restore::spawn_plan`]), so a change of restore mode or login shell later does not hand the
//! line to a shell that reads it differently. An argv that line, or the live login shell when new panes run in Live
//! sessions now, cannot carry is refused as unreplayable. A remote source answers upstream's local-path refusal until
//! remote companions are ported.

use std::path::Path;

use ngterm_host::Surface;
use uuid::Uuid;

use super::{Outcome, args_of, session_store, with_id};
use crate::Core;
use crate::duplicate_replay::{Lookup, Planner, Reason, Refusal, canonical_executable};
use crate::error::ControlError;
use crate::live::daemon_name;
use crate::model::{CompanionCopy, OverlayPane, PaneCopy, Session, SessionContent, StatusPane};
use crate::pane_env::uuid_string;
use crate::persist::RestoreMode;
use crate::restart_replay::{self, Observation};
use crate::restore::capture::ForegroundReader;
use crate::restore::spawn_plan::ORDINARY_DIALECT;
use crate::shell_dialect::ShellDialect;
use ngterm_proto::Request;

/// `session.companion.duplicate`: `{id, parent}` of the copy.
pub(crate) fn duplicate(core: &mut Core, request: &Request) -> Outcome {
    let args = args_of(request);
    let select = args.no_select != Some(true);
    let (_, id) = session_store(core, request.target.as_deref(), args.window.as_deref())?;
    let copy = core.duplicate_companion(id, select).map_err(ControlError::rejected)?;
    let mut result = with_id(copy);
    result.parent = core
        .library()
        .store_for_session(copy)
        .and_then(|store| store.session(copy))
        .and_then(Session::parent_id)
        .map(uuid_string);
    Ok(result)
}

impl Core {
    /// Plans every pane of `source` and creates the copy (upstream's `duplicateCompanion`); the refusal names why
    /// nothing was created.
    pub(crate) fn duplicate_companion(&mut self, source: Uuid, select: bool) -> Result<Uuid, String> {
        let copy = self.plan_companion_copy(source, select)?;
        let home = self.library().home().to_owned();
        self.library_mut()
            .store_for_session_mut(source)
            .ok_or_else(|| "no such session".to_owned())?
            .duplicate_as_companion(source, copy, &home)
            .map_err(|_| "could not create companion".to_owned())
    }

    fn plan_companion_copy(&self, source: Uuid, select: bool) -> Result<CompanionCopy, String> {
        let session = self
            .library()
            .store_for_session(source)
            .and_then(|store| store.session(source))
            .ok_or_else(|| "no such session".to_owned())?;
        if session.remote_host().is_some() {
            return Err("companions of a remote session are not supported yet".to_owned());
        }
        if session.content() == SessionContent::Page {
            return Err("a page companion has no terminal to duplicate".to_owned());
        }
        // a visible program over the panes is what the user sees running; copying the covered shell instead is wrong
        if session.program_overlay_visible() {
            return Err("a program overlay covers the session; nothing was created".to_owned());
        }
        self.refresh_live_leaders();
        let left = self.plan_pane_copy(session, StatusPane::Left)?;
        let right = if session.split.exists {
            Some(self.plan_pane_copy(session, StatusPane::Right)?)
        } else {
            None
        };
        Ok(CompanionCopy {
            left,
            right,
            axis: session.split.axis,
            ratio: session.split.ratio,
            select,
        })
    }

    /// One fresh listing of the live daemons' leaders, so every pane is read against the same one.
    fn refresh_live_leaders(&self) {
        if let Some(host) = self.live.host() {
            let leaders = host
                .list()
                .ok()
                .map(|records| crate::live::lifecycle::leaders(&records));
            self.live.leaders.borrow_mut().accept(leaders);
        }
    }

    fn plan_pane_copy(&self, session: &Session, pane: StatusPane) -> Result<PaneCopy, String> {
        let name = pane.as_str();
        let overlay = if pane == StatusPane::Right {
            OverlayPane::Right
        } else {
            OverlayPane::Left
        };
        if session
            .pane_overlay(overlay)
            .is_some_and(|covering| covering.html.is_none() && !covering.hidden)
        {
            return Err(format!(
                "{name} pane is covered by a program overlay; nothing was created"
            ));
        }
        let surface = session
            .pane_slot(pane)
            .get()
            .filter(|surface| surface.is_realized())
            .ok_or_else(|| format!("{name} pane is not realized; nothing was created"))?;
        let observation = self.replay_observation(surface);
        let denylist: Vec<String> = self.restore.denylist().iter().cloned().collect();
        let lookup = Lookup::host();
        let resolved_file = |path: &str| canonical_executable(path);
        let planner = Planner {
            shell: self.restore.shell(),
            denylist: &denylist,
            lookup: &lookup,
            resolved_file: &resolved_file,
        };
        let plan = planner
            .resolve(&observation, name, session.cwd_for(pane))
            .map_err(|refusal| refusal.message())?;
        let command = match plan.argv() {
            Some(argv) => Some(self.creation_line(argv).ok_or_else(|| {
                Refusal {
                    pane: name.to_owned(),
                    reason: Reason::Replay(restart_replay::Refusal::Unreplayable),
                }
                .message()
            })?),
            None => None,
        };
        Ok(PaneCopy {
            cwd: plan.working_directory().to_owned(),
            command,
            argv: plan.argv().map(<[String]>::to_vec),
        })
    }

    /// What holds `surface`'s foreground, read for a copy of it (upstream's `ForegroundProcess.replayObservation`).
    fn replay_observation(&self, surface: &dyn Surface) -> Observation {
        let empty = Observation {
            foreground: None,
            is_daemon_leader: false,
            working_directory: None,
            executable_path: None,
            search_path: None,
        };
        let Some(table) = self.restore.table() else {
            return empty;
        };
        let live = |token: &str| super::live::launch::live_foreground_group(&self.live, table, token);
        let reader = ForegroundReader::new(table, self.restore.shell()).with_live(&live);
        let Some((pid, foreground)) = reader.group_of(surface).and_then(|pgid| reader.replayed(pgid)) else {
            return empty;
        };
        let leader = surface
            .is_live_session()
            .then(|| Uuid::parse_str(surface.pane_token()).ok())
            .flatten()
            .and_then(|identity| self.live.leaders.borrow().leader(&daemon_name(identity)));
        Observation {
            foreground: Some(foreground),
            is_daemon_leader: leader == Some(pid),
            working_directory: table
                .working_directory(pid)
                .filter(|directory| Path::new(directory).is_dir()),
            executable_path: table.executable_path(pid),
            search_path: table.search_path(pid),
        }
    }

    /// `argv` as the creation command of a new pane (module docs); `None` when it cannot be carried.
    fn creation_line(&self, argv: &[String]) -> Option<String> {
        let live_shell = self
            .live
            .login_shell()
            .filter(|_| self.restore.decision.active == RestoreMode::Live);
        creation_line(ORDINARY_DIALECT, live_shell, argv)
    }
}

/// `argv` as a copy's stored creation command: the line the `ordinary` dialect (`/bin/sh -c`'s, `cmd.exe`'s) reads,
/// and only when `live_shell`, the login shell new panes run in Live sessions now, can carry it too; `None` otherwise.
pub(crate) fn creation_line(ordinary: ShellDialect, live_shell: Option<&str>, argv: &[String]) -> Option<String> {
    if let Some(shell) = live_shell {
        ShellDialect::of_shell(shell).line(argv)?;
    }
    ordinary.line(argv)
}
