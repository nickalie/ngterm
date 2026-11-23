//! A split change made on a viewer reaches its origin, as upstream's remote window sends `setSplit` and `closeSplit`
//! to its server (`RemoteCatalogActions.swift`). A viewer's split is a pane of the origin ([`super::split`]), so a
//! change made only here comes back on the next attach.

use std::collections::VecDeque;

use uuid::Uuid;

use super::origin_replies::{COMPANION_DEADLINE, no_answer, reply_of};
use super::peer::RemotePeer;
use super::ssh::control_command;
use super::tree::RemoteCommandResult;
use super::viewer::{Answer, run_off_thread, take_answer};
use crate::Core;
use crate::model::SplitAxis;

/// The names upstream's remote window gives these actions in its failure panel.
const CLOSE_SPLIT: &str = "Close Split";
const SHOW_SPLIT: &str = "Show Split";
const HIDE_SPLIT: &str = "Hide Split";

/// What a viewer changed in a row's split.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SplitChange {
    Close,
    Visibility { shown: bool, axis: SplitAxis },
}

/// The origin lines of one split change, run in order until one fails.
struct Change {
    action: &'static str,
    lines: VecDeque<Vec<String>>,
}

/// A row's split changes, sent to its origin one line at a time in the order the viewer made them.
pub(crate) struct MirrorRun {
    session: Uuid,
    host: String,
    peer: RemotePeer,
    changes: VecDeque<Change>,
    answer: Option<Answer>,
}

impl MirrorRun {
    /// The origin lines that close the split a pending viewer split asked row `id`'s origin for.
    pub(super) fn abandoned(id: Uuid, host: String, peer: RemotePeer, origin: &str) -> Self {
        Self {
            session: id,
            host,
            peer,
            changes: VecDeque::from([Change {
                action: CLOSE_SPLIT,
                lines: VecDeque::from([["session", "split", "close", "--target", origin]
                    .map(str::to_owned)
                    .to_vec()]),
            }]),
            answer: None,
        }
    }
}

impl Core {
    /// The origin lines that make `change` on row `id`'s origin, read before a close drops the split here. `None` when
    /// the row's split is not an attached origin pane.
    pub(crate) fn split_mirror(&self, id: Uuid, change: SplitChange) -> Option<MirrorRun> {
        let session = self.library().held_session(id)?;
        let host = session.remote_host()?.to_owned();
        let state = session.remote_presentation()?;
        let split = session.split_pane_identity.filter(|_| session.split.exists)?;
        let remote = state.binding.remote_pane(split)?;
        let origin = state.binding.remote_session_id.as_str();
        let line = |words: &[&str]| words.iter().map(|word| (*word).to_owned()).collect::<Vec<_>>();
        let (action, lines) = match change {
            SplitChange::Close => {
                // a swap made only here put the origin's primary in this split
                let swapped = state
                    .layout
                    .as_ref()
                    .is_some_and(|layout| layout.panes.len() == 2 && layout.primary == remote);
                let mut lines = VecDeque::new();
                if swapped {
                    lines.push_back(line(&["session", "swap", "--target", origin]));
                }
                lines.push_back(line(&["session", "split", "close", "--target", origin]));
                (CLOSE_SPLIT, lines)
            }
            SplitChange::Visibility { shown, axis } => {
                let mode = if shown { "on" } else { "off" };
                let mut words = line(&["session", "split", "visibility", mode, "--target", origin]);
                if shown {
                    words.extend(line(&["--axis", axis.wire_name()]));
                }
                (if shown { SHOW_SPLIT } else { HIDE_SPLIT }, VecDeque::from([words]))
            }
        };
        let peer = self.remote_row_peer(id, &host);
        Some(MirrorRun {
            session: id,
            host,
            peer,
            changes: VecDeque::from([Change { action, lines }]),
            answer: None,
        })
    }

    /// Sends `run`'s first line to the origin, or queues its changes behind the row's change still in flight, so the
    /// origin applies them in the viewer's order.
    pub(crate) fn start_split_mirror(&mut self, mut run: MirrorRun) {
        if let Some(pending) = self
            .remote
            .viewer
            .split_mirrors
            .iter_mut()
            .find(|pending| pending.session == run.session)
        {
            pending.changes.append(&mut run.changes);
            return;
        }
        if self.send_next_mirror_line(&mut run) {
            self.remote.viewer.split_mirrors.push(run);
        }
    }

    /// Moves every split change on whose answer arrived: the next line goes out, or a failure shows over the row. A
    /// closed row's changes are dropped.
    pub(super) fn advance_split_mirrors(&mut self) {
        for mut run in std::mem::take(&mut self.remote.viewer.split_mirrors) {
            if self.library().held_session(run.session).is_none() {
                continue;
            }
            let Some(result) = run.answer.as_ref().and_then(take_answer) else {
                self.remote.viewer.split_mirrors.push(run);
                continue;
            };
            if let Some(reason) = mirror_failure(&result, &run.host)
                && let Some(change) = run.changes.pop_front()
            {
                self.mirror_failed(run.session, change.action, &reason);
            }
            if self.send_next_mirror_line(&mut run) {
                self.remote.viewer.split_mirrors.push(run);
            }
        }
    }

    /// Whether a line went out; `false` once none is left. A change whose line could not start is dropped, and the
    /// next change goes out.
    fn send_next_mirror_line(&mut self, run: &mut MirrorRun) -> bool {
        loop {
            let Some(change) = run.changes.front_mut() else {
                return false;
            };
            let Some(words) = change.lines.pop_front() else {
                run.changes.pop_front();
                continue;
            };
            let answer = control_command(&run.host, run.peer, &words)
                .ok()
                .and_then(|argv| run_off_thread(&self.remote.runner, argv, COMPANION_DEADLINE));
            if answer.is_some() {
                run.answer = answer;
                return true;
            }
            let reason = format!("the remote command failed on {}", run.host);
            let action = change.action;
            run.changes.pop_front();
            self.mirror_failed(run.session, action, &reason);
        }
    }

    /// Whether row `id` has split changes on their way to its origin.
    pub(super) fn split_mirror_pending(&self, id: Uuid) -> bool {
        self.remote.viewer.split_mirrors.iter().any(|run| run.session == id)
    }

    fn mirror_failed(&mut self, session: Uuid, action: &'static str, reason: &str) {
        tracing::info!(reason, action, "a split change did not reach the origin");
        let Some(window) = self.library().window_holding_session(session) else {
            return;
        };
        self.action_failed(action, window, Some(session), reason);
    }
}

/// Why the origin did not make the change, `None` when it did.
fn mirror_failure(result: &RemoteCommandResult, host: &str) -> Option<String> {
    let Some(reply) = reply_of(result) else {
        return Some(no_answer(result, host, "the origin may keep its split as it was"));
    };
    (!reply.ok).then(|| {
        reply
            .error
            .unwrap_or_else(|| format!("the remote command failed on {host}"))
    })
}

impl Core {
    /// Closes row `id`'s split here and, for a remote row, on its origin too (upstream's `closeSplit` with its
    /// `performRemote(.closeSplit)`).
    pub(crate) fn close_split_mirrored(&mut self, id: Uuid) {
        let mirror = self.split_mirror(id, SplitChange::Close);
        if let Some(store) = self.library_mut().store_for_session_mut(id) {
            store.close_split(id);
        }
        if let Some(mirror) = mirror {
            self.start_split_mirror(mirror);
        }
    }

    /// Sends row `id`'s split visibility to its origin when it changed from `before` (shown, axis).
    pub(crate) fn mirror_split_visibility(&mut self, id: Uuid, before: Option<(bool, SplitAxis)>) {
        let Some(split) = self.library().held_session(id).map(|session| session.split) else {
            return;
        };
        let changed = before.is_some_and(|(shown, axis)| split.shown != shown || (split.shown && split.axis != axis));
        let change = SplitChange::Visibility {
            shown: split.shown,
            axis: split.axis,
        };
        if changed && let Some(mirror) = self.split_mirror(id, change) {
            self.start_split_mirror(mirror);
        }
    }
}
