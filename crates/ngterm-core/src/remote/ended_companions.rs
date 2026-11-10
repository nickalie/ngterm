//! A terminal companion that ended on its origin closes here too (this build's addition: upstream leaves the row on its
//! held exit line). A companion is a worker under its parent, so a finished one has nothing left to show.
//!
//! The ssh of a replica ending says only that the attach stopped, so once every pane of a companion row is held the
//! origin's `live list` is read, and the row closes when a complete inventory no longer has the session. That is the
//! read, not `live tree`: `live tree` leaves out whatever cannot be attached right now, a session being restarted too.
//! An origin that does not answer proves nothing and the row keeps its line, as does a session still there: the origin
//! may not have noticed yet, so it is read a few times first. Closing a remote row only detaches it, so this can end
//! nothing on the origin; a local scratch or overlay on the row would go with it, so one of those keeps the row.

use std::time::Instant;

use uuid::Uuid;

use super::origin_replies::{PAUSE, reply_of};
use super::ssh::control_command;
use super::tree::{RemoteCommandResult, TREE_DEADLINE};
use super::viewer::{Answer, run_off_thread, take_answer};
use crate::Core;
use crate::model::{OverlayPane, Session};

/// How many inventories may still show the session before the row is left on its exit line.
const READS: u32 = 3;

/// A companion row whose panes are all held, and the read that decides it.
pub(crate) struct EndedCheck {
    row: Uuid,
    host: String,
    origin_session: String,
    read: Option<Answer>,
    due: Instant,
    reads_left: u32,
}

/// Whether the row holds something of this machine's that closing it would end: a scratch shell or an overlay.
fn holds_local_work(row: &Session) -> bool {
    row.scratch_surface.is_present()
        || [None, Some(OverlayPane::Left), Some(OverlayPane::Right)]
            .into_iter()
            .any(|pane| row.local_overlay_holds(pane))
}

/// The host and origin session of a remote terminal companion every pane of which is held; a pane running here (a local
/// split too) keeps the row, as does [`holds_local_work`].
fn held_companion(row: &Session) -> Option<(String, String)> {
    row.parent_id()?;
    if holds_local_work(row) {
        return None;
    }
    let host = row.remote_host()?;
    let state = row.remote_presentation()?;
    let split = row.split_pane_identity.filter(|_| row.split.exists);
    [Some(row.pane_identity), split]
        .into_iter()
        .flatten()
        .all(|pane| state.held_panes.contains(&pane))
        .then(|| (host.to_owned(), state.binding.remote_session_id.clone()))
}

/// Whether the origin's `live list` answer has the session; `None` when the answer cannot tell.
fn session_exists(result: &RemoteCommandResult, origin_session: &str) -> Option<bool> {
    let reply = reply_of(result).filter(|reply| result.status == 0 && reply.ok)?;
    let inventory = reply.result?.live?;
    let exists = inventory
        .entries
        .iter()
        .any(|entry| entry.session_id.as_deref() == Some(origin_session));
    (exists || inventory.inventory_complete).then_some(exists)
}

impl Core {
    fn held_companion_row(&self, row: Uuid) -> Option<(String, String)> {
        let store = self.library().store_for_session(row)?;
        held_companion(store.session(row)?)
    }

    /// A pane of row `row` was held: starts checking its origin when that leaves the row a companion with nothing running.
    pub(crate) fn check_ended_companion(&mut self, row: Uuid) {
        if self.remote.viewer.ended_companions.iter().any(|check| check.row == row) {
            return;
        }
        if let Some((host, origin_session)) = self.held_companion_row(row) {
            self.remote.viewer.ended_companions.push(EndedCheck {
                row,
                host,
                origin_session,
                read: None,
                due: Instant::now(),
                reads_left: READS,
            });
        }
    }

    /// Moves every check on; a row attached again or closed here meanwhile drops out.
    pub(crate) fn advance_ended_companions(&mut self, now: Instant) {
        for check in std::mem::take(&mut self.remote.viewer.ended_companions) {
            if self.held_companion_row(check.row).is_none() {
                continue;
            }
            if let Some(check) = self.step_ended_check(check, now) {
                self.remote.viewer.ended_companions.push(check);
            }
        }
    }

    fn step_ended_check(&mut self, mut check: EndedCheck, now: Instant) -> Option<EndedCheck> {
        let Some(answer) = check.read.take() else {
            if check.due <= now {
                let argv = control_command(&check.host, &["live", "list"]).ok()?;
                check.read = Some(run_off_thread(&self.remote.runner, argv, TREE_DEADLINE)?);
            }
            return Some(check);
        };
        let Some(result) = take_answer(&answer) else {
            check.read = Some(answer);
            return Some(check);
        };
        if !session_exists(&result, &check.origin_session)? {
            if let Some(store) = self.library_mut().store_for_session_mut(check.row) {
                store.close_session(check.row);
            }
            return None;
        }
        check.reads_left = check.reads_left.saturating_sub(1);
        check.due = now + PAUSE;
        (check.reads_left > 0).then_some(check)
    }
}
