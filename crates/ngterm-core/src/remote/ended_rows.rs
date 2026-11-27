//! A remote row whose session ended on its origin: both are this build's additions, upstream leaves the row on its held
//! exit line with its presentation stream retrying for as long as the row exists.
//!
//! - A terminal companion closes here too. A companion is a worker under its parent, so a finished one has nothing left
//!   to show.
//! - Any other row keeps its exit line, but its presentation stream stops: each try would only fail with "no such
//!   session" until the row closes. A reattach of one of its panes ([`Core::reattach_remote_pane`]) starts it again.
//!
//! The ssh of a replica ending says only that the attach stopped, so once every pane of a remote row is held, and again
//! each time the stream of such a row ends, the origin's `live list` is read. The row ends when a complete inventory no
//! longer has the session. That is the read, not `live tree`: `live tree` leaves out whatever cannot be attached right
//! now, a session being restarted too. An origin that does not answer proves nothing, and neither does a session still
//! there: the origin may not have noticed yet, so it is read a few times first. A stream that ends with "no such
//! session" proves nothing alone either: an origin app that restarts lists the same sessions again.
//!
//! Closing a remote row only detaches it, so this can end nothing on the origin; a local scratch or overlay on a
//! companion row would go with it, so one of those keeps the row, and only its stream stops.

use std::time::Instant;

use uuid::Uuid;

use super::origin_replies::{PAUSE, reply_of};
use super::ssh::control_command;
use super::tree::{RemoteCommandResult, TREE_DEADLINE};
use super::viewer::{Answer, run_off_thread, take_answer};
use crate::Core;
use crate::model::Session;

/// How many inventories may still show the session before the row is left as it is.
const READS: u32 = 3;

/// What a row does once its origin no longer has its session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ending {
    /// A terminal companion with nothing local on it closes.
    Close,
    /// Any other row keeps its exit line, and its presentation stream stops.
    StopStream,
}

/// A remote row with every pane held, and the read that decides it.
pub(crate) struct EndedCheck {
    row: Uuid,
    host: String,
    origin_session: String,
    read: Option<Answer>,
    due: Instant,
    reads_left: u32,
}

/// The host, the origin session and the ending of a remote row every pane of which is held; a pane running here (a
/// local split too) keeps the row, as does a row with nothing left to end (no stream, and no companion to close).
fn ended_row(row: &Session, streams: bool) -> Option<(String, String, Ending)> {
    if !row.every_remote_pane_held() {
        return None;
    }
    let ending = if row.parent_id().is_some() && !row.holds_local_work() {
        Ending::Close
    } else if streams {
        Ending::StopStream
    } else {
        return None;
    };
    let host = row.remote_host()?;
    let state = row.remote_presentation()?;
    Some((host.to_owned(), state.binding.remote_session_id.clone(), ending))
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
    fn ended_row_of(&self, row: Uuid) -> Option<(String, String, Ending)> {
        let store = self.library().store_for_session(row)?;
        ended_row(store.session(row)?, self.remote.viewer.streams(row))
    }

    /// A pane of row `row` was held, or the row's stream ended: starts checking its origin when that leaves a remote row
    /// with nothing running.
    pub(crate) fn check_ended_row(&mut self, row: Uuid) {
        if self.remote.viewer.ended_rows.iter().any(|check| check.row == row) {
            return;
        }
        if let Some((host, origin_session, _)) = self.ended_row_of(row) {
            self.remote.viewer.ended_rows.push(EndedCheck {
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
    pub(crate) fn advance_ended_rows(&mut self, now: Instant) {
        for check in std::mem::take(&mut self.remote.viewer.ended_rows) {
            if let Some(check) = self.step_ended_check(check, now) {
                self.remote.viewer.ended_rows.push(check);
            }
        }
    }

    fn step_ended_check(&mut self, mut check: EndedCheck, now: Instant) -> Option<EndedCheck> {
        let (_, _, ending) = self.ended_row_of(check.row)?;
        let Some(answer) = check.read.take() else {
            if check.due <= now {
                let peer = self.remote_row_peer(check.row, &check.host);
                let argv = control_command(&check.host, peer, &["live", "list"]).ok()?;
                check.read = Some(run_off_thread(&self.remote.runner, argv, TREE_DEADLINE)?);
            }
            return Some(check);
        };
        let Some(result) = take_answer(&answer) else {
            check.read = Some(answer);
            return Some(check);
        };
        if !session_exists(&result, &check.origin_session)? {
            match ending {
                Ending::Close => {
                    if let Some(store) = self.library_mut().store_for_session_mut(check.row) {
                        store.close_session(check.row);
                    }
                }
                Ending::StopStream => self.stop_remote_presentation(check.row),
            }
            return None;
        }
        check.reads_left = check.reads_left.saturating_sub(1);
        check.due = now + PAUSE;
        (check.reads_left > 0).then_some(check)
    }
}
