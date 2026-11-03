//! A split opened on a viewer is made on its origin before the new terminal can spawn. The origin's live daemon
//! is then attached with the same leadership and reconnect contract as the session's original panes.
//! This build extends upstream, which opens a viewer's new split as a local shell.

use std::time::Instant;

use ngterm_proto::ControlRemoteTree;
use uuid::Uuid;

use super::attach::start_tree_read_within;
use super::frames::PresentationPane;
use super::origin_replies::{COMPANION_DEADLINE, PAUSE, READY_BUDGET, read_remote_tree, reply_of};
use super::ssh::{PaneLabel, attach_pane_command, control_command};
use super::tree::TREE_DEADLINE;
use super::viewer::{Answer, run_off_thread, take_answer};
use crate::Core;
use crate::command_restore::shell_quoted_line;
use crate::live::lead::LeadAttachment;
use crate::model::{PaneLaunch, Session, SplitAxis, StatusPane};

pub(super) struct SplitRun {
    target: Target,
    stage: Stage,
}

struct Target {
    session: Uuid,
    pane: Uuid,
    host: String,
    origin: String,
    excluded_daemons: Vec<String>,
}

enum Stage {
    Creating(Answer),
    Listing {
        answer: Option<Answer>,
        due: Instant,
        deadline: Instant,
        reason: String,
    },
}

impl Core {
    /// An explicit page request may bring up a split the origin created after this row was attached. Ordinary
    /// layout frames still never create panes. Only a pane in the latest shown layout can be attached here.
    pub(super) fn ensure_replica_page_pane(&mut self, id: Uuid, remote: &PresentationPane) -> Result<(), &'static str> {
        let missing = crate::commands::hud::PANE_NOT_VISIBLE;
        let session = self.library().held_session(id).ok_or(missing)?;
        let state = session.remote_presentation().ok_or(missing)?;
        let reveal =
            session.split.exists && !session.split.shown && state.layout.as_ref().is_some_and(|layout| layout.shown);
        if session
            .local_pane(Some(remote))
            .is_some_and(|pane| session.renders_overlay_pane(pane))
            && !reveal
        {
            return Ok(());
        }
        let PresentationPane::Identity(identity) = remote else {
            return Err(missing);
        };
        let layout = state
            .layout
            .as_ref()
            .filter(|layout| layout.shown && layout.panes.len() == 2 && layout.panes.contains(identity))
            .ok_or(missing)?;
        let axis = layout.axis.as_deref().and_then(SplitAxis::from_wire);
        if session.local_pane(Some(remote)).is_some() {
            self.library_mut()
                .store_for_session_mut(id)
                .ok_or(missing)?
                .set_split_visibility(id, true, axis);
            return Ok(());
        }
        // Never replace a viewer's own split or an attachment still in progress.
        if session.split.exists || session.split_pane_identity.is_some() {
            return Err(missing);
        }
        let primary = state.binding.remote_pane(session.pane_identity).ok_or(missing)?;
        if !layout.panes.contains(&primary) || primary == *identity {
            return Err(missing);
        }
        let origin = state.binding.origin.as_ref().ok_or(missing)?;
        let focused = session.split.focused;
        let lead = LeadAttachment::new(true);
        let daemon = crate::live::daemon_name(*identity);
        let command = attach_pane_command(
            &origin.host,
            &origin.endpoint,
            &daemon,
            PaneLabel {
                session: &origin.session_name,
                pane: "right",
            },
            Some(&lead),
        )
        .map_err(|_| missing)?;
        let store = self.library_mut().store_for_session_mut(id).ok_or(missing)?;
        store.set_split_visibility(id, true, axis);
        let session = store.session_mut(id).ok_or(missing)?;
        let pane = session.split_pane_identity.ok_or(missing)?;
        session.split.focused = focused;
        session.split_launch = PaneLaunch {
            command: Some(command),
            wait: true,
            ..PaneLaunch::default()
        };
        session
            .remote_presentation
            .as_mut()
            .ok_or(missing)?
            .binding
            .insert_pane(pane, daemon);
        self.live.leads.begin(pane, lead, false);
        self.remote.viewer.split_layouts.insert(pane, id);
        Ok(())
    }

    /// Holds an unconfigured remote pane's spawn until its origin daemon is attached. Existing attached panes keep
    /// their commands, including reconnect's replacement attachments.
    pub(crate) fn prepare_remote_pane(&mut self, id: Uuid, pane: Uuid) -> bool {
        let Some(session) = self.library().held_session(id) else {
            return false;
        };
        let Some(host) = session.remote_host().map(str::to_owned) else {
            return false;
        };
        let Some(role) = role_of(session, pane) else {
            return false;
        };
        if session.pane_launch(role).is_some_and(|launch| launch.command.is_some()) {
            return false;
        }
        if self.remote.viewer.split_flows.iter().any(|run| run.target.pane == pane) {
            return true;
        }
        // A promoted, still-pending split can gain another split. Serialize their origin mutations and binding,
        // so the later request sees its sibling's daemon and can actually create the origin's next split.
        if self
            .remote
            .viewer
            .split_flows
            .iter()
            .any(|run| run.target.session == id)
        {
            self.remote.viewer.queued_split_panes.insert(pane, id);
            return true;
        }
        let Some(binding) = session.remote_presentation().map(|state| &state.binding) else {
            self.fail_remote_split(id, pane, "the remote session has no origin binding");
            return true;
        };
        let origin = binding.remote_session_id.clone();
        // Capture the sibling now: it may leave or trade roles while the new daemon starts. Ignore bindings for
        // closed panes, so reopening a split can still reuse the origin daemon it detached from.
        let excluded_daemons = [Some(session.pane_identity), session.split_pane_identity]
            .into_iter()
            .flatten()
            .filter(|identity| *identity != pane)
            .filter_map(|identity| binding.daemon(identity).map(str::to_owned))
            .collect();
        let argv = control_command(
            &host,
            &[
                "session",
                "split",
                "on",
                "--target",
                &origin,
                "--axis",
                session.split.axis.wire_name(),
            ],
        );
        let answer = argv
            .ok()
            .and_then(|argv| run_off_thread(&self.remote.runner, argv, COMPANION_DEADLINE));
        let Some(answer) = answer else {
            self.fail_remote_split(id, pane, "the remote split command could not start");
            return true;
        };
        self.remote.viewer.split_flows.push(SplitRun {
            target: Target {
                session: id,
                pane,
                host,
                origin,
                excluded_daemons,
            },
            stage: Stage::Creating(answer),
        });
        true
    }

    pub(super) fn advance_remote_splits(&mut self) {
        for mut run in std::mem::take(&mut self.remote.viewer.split_flows) {
            if self
                .library()
                .held_session(run.target.session)
                .is_none_or(|session| role_of(session, run.target.pane).is_none())
            {
                continue;
            }
            match self.step_remote_split(&mut run) {
                Ok(true) => self.remote.viewer.split_flows.push(run),
                Ok(false) => {}
                Err(reason) => self.fail_remote_split(run.target.session, run.target.pane, &reason),
            }
        }
        for (pane, session) in std::mem::take(&mut self.remote.viewer.queued_split_panes) {
            if self
                .remote
                .viewer
                .split_flows
                .iter()
                .any(|run| run.target.session == session)
            {
                self.remote.viewer.queued_split_panes.insert(pane, session);
            } else {
                self.grant_remote_pane_spawn(pane);
            }
        }
        self.reconcile_split_layouts();
    }

    /// Whether the pane is still waiting. The mutation is issued once, then only the read is retried.
    fn step_remote_split(&mut self, run: &mut SplitRun) -> Result<bool, String> {
        let now = self.remote_now();
        match &mut run.stage {
            Stage::Creating(answer) => {
                let Some(result) = take_answer(answer) else {
                    return Ok(true);
                };
                let reply = reply_of(&result).ok_or_else(|| {
                    format!(
                        "{} did not confirm the split: {}",
                        run.target.host,
                        result.stderr.trim()
                    )
                })?;
                if !reply.ok {
                    return Err(reply.error.unwrap_or_else(|| "the origin refused the split".to_owned()));
                }
                run.stage = Stage::Listing {
                    answer: None,
                    due: now,
                    deadline: now + READY_BUDGET,
                    reason: "the remote split did not become attachable".to_owned(),
                };
            }
            Stage::Listing {
                answer,
                due,
                deadline,
                reason,
            } => {
                if let Some(pending) = answer {
                    let Some(result) = take_answer(pending) else {
                        return Ok(true);
                    };
                    *answer = None;
                    let attached = match read_remote_tree(&result, &run.target.host) {
                        Ok(tree) => self.attach_remote_split(&run.target, &tree)?,
                        Err(error) => {
                            *reason = error;
                            false
                        }
                    };
                    if attached {
                        return Ok(false);
                    }
                    *due = now + PAUSE;
                }
                let remaining = deadline.saturating_duration_since(now);
                if remaining.is_zero() {
                    return Err(reason.clone());
                }
                if now >= *due {
                    *answer = Some(
                        start_tree_read_within(self, &run.target.host, remaining.min(TREE_DEADLINE))
                            .map_err(|error| error.to_string())?,
                    );
                }
            }
        }
        Ok(true)
    }

    fn attach_remote_split(&mut self, target: &Target, tree: &ControlRemoteTree) -> Result<bool, String> {
        let (id, pane) = (target.session, target.pane);
        let session = self.library().held_session(id).ok_or("the viewer session closed")?;
        let role = role_of(session, pane).ok_or("the viewer pane closed")?;
        let binding = &session
            .remote_presentation()
            .ok_or("the origin binding was lost")?
            .binding;
        if binding
            .origin
            .as_ref()
            .is_none_or(|origin| origin.endpoint != tree.endpoint)
        {
            return Err("the origin endpoint changed; reattach the remote session".to_owned());
        }
        let Some(remote) = tree.sessions.iter().find(|remote| remote.id == target.origin) else {
            return Ok(false);
        };
        let active_daemons: Vec<&str> = [Some(session.pane_identity), session.split_pane_identity]
            .into_iter()
            .flatten()
            .filter(|identity| *identity != pane)
            .filter_map(|identity| binding.daemon(identity))
            .collect();
        let Some(remote_pane) = remote.panes.iter().find(|pane| {
            !target.excluded_daemons.contains(&pane.daemon) && !active_daemons.contains(&pane.daemon.as_str())
        }) else {
            return Ok(false);
        };
        let lead = LeadAttachment::new(true);
        let command = attach_pane_command(
            &target.host,
            &tree.endpoint,
            &remote_pane.daemon,
            PaneLabel {
                session: &remote.name,
                pane: role.as_str(),
            },
            Some(&lead),
        )
        .map_err(|_| "the origin reported an invalid pane endpoint")?;
        let session = self
            .library_mut()
            .held_session_mut(id)
            .ok_or("the viewer session closed")?;
        set_launch(
            session,
            role,
            PaneLaunch {
                command: Some(command),
                wait: true,
                ..PaneLaunch::default()
            },
        );
        if let Some(state) = session.remote_presentation.as_mut() {
            state.binding.insert_pane(pane, remote_pane.daemon.clone());
        }
        // A primary removal may have been deferred while this split was unbound. Only reconsider layouts that
        // already know the new daemon, since an older one-pane layout would otherwise close the split just bound.
        let layout = session.remote_presentation().and_then(|state| {
            let layout = state.layout.as_ref()?;
            layout
                .panes
                .contains(&state.binding.remote_pane(pane)?)
                .then(|| layout.clone())
        });
        self.live.leads.begin(pane, lead, false);
        self.grant_remote_pane_spawn(pane);
        self.remote.viewer.split_layouts.insert(pane, id);
        if let Some(layout) = layout {
            self.apply_remote_layout(&layout, id);
        }
        Ok(true)
    }

    /// The origin may have reordered the panes while their attach was pending. Its unchanged layout will not be
    /// published again, so apply it once all surviving terminals are realized, including after a soft-close undo.
    fn reconcile_split_layouts(&mut self) {
        for (pane, id) in std::mem::take(&mut self.remote.viewer.split_layouts) {
            let Some(session) = self.library().held_session(id) else {
                continue;
            };
            if role_of(session, pane).is_none() {
                continue;
            }
            let realized = |slot: &crate::model::SurfaceSlot| slot.get().is_some_and(ngterm_host::Surface::is_realized);
            if self.library().store_for_session(id).is_none()
                || !realized(&session.surface)
                || (session.split.exists && !realized(&session.split_surface))
            {
                self.remote.viewer.split_layouts.insert(pane, id);
                continue;
            }
            let layout = session.remote_presentation().and_then(|state| {
                let layout = state.layout.as_ref()?;
                layout
                    .panes
                    .contains(&state.binding.remote_pane(pane)?)
                    .then(|| layout.clone())
            });
            if let Some(layout) = layout {
                self.apply_remote_layout(&layout, id);
            }
        }
    }

    fn fail_remote_split(&mut self, id: Uuid, pane: Uuid, reason: &str) {
        let message = format!("Could not open the remote pane: {reason}");
        if let Some(session) = self.library_mut().held_session_mut(id)
            && let Some(role) = role_of(session, pane)
        {
            // Keep the terminal useful as an error receipt; it must never fall back to a local login shell.
            let command = if cfg!(windows) {
                "echo Could not open remote pane. See notification for details. & exit /b 1".to_owned()
            } else {
                format!("printf '%s\\n' {}; exit 1", shell_quoted_line(&[message.as_str()]))
            };
            set_launch(
                session,
                role,
                PaneLaunch {
                    command: Some(command),
                    wait: true,
                    ..PaneLaunch::default()
                },
            );
            self.grant_remote_pane_spawn(pane);
            self.post_app_banner(format!("remote-split-{pane}"), "Remote Split", message);
        }
    }
}

fn role_of(session: &Session, pane: Uuid) -> Option<StatusPane> {
    if session.pane_identity == pane {
        Some(StatusPane::Left)
    } else if session.split_pane_identity == Some(pane) {
        Some(StatusPane::Right)
    } else {
        None
    }
}

fn set_launch(session: &mut Session, role: StatusPane, launch: PaneLaunch) {
    if role == StatusPane::Left {
        session.launch = launch;
    } else {
        session.split_launch = launch;
    }
}
