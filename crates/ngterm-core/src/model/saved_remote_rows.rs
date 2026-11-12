//! The rows attached from other machines as a window's snapshot keeps them, so the next launch attaches them again
//! (an extension beyond upstream, whose relaunch drops them; the core's side is `remote::relaunch`): the attached rows
//! written from the live tree, and the saved ones a restore read back, held here until they attach or are dropped. A
//! saved row a quit catches before its machine answered is written again as read, so an offline launch loses nothing.
//!
//! A row is placed by the local row before it, which a restore brings back under the same id, and by its order among
//! the remote rows after that one: an index would count rows a restore never brings back (pages, other remote rows)
//! and shift whenever another saved row is dropped.

use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use super::companions::SessionContent;
use super::session::Session;
use super::store::AppStore;
use super::workspace::Workspace;
use crate::persist::RemoteSessionSnapshot;
use crate::remote::peer::RemotePeer;

/// Where a reattached row was saved: the local row it followed and its order after that one. Kept so the next
/// snapshot writes the same order and a later saved row of the same run goes in before it.
pub(crate) type SavedPlace = (Option<Uuid>, usize);

impl AppStore {
    /// What the snapshot writes: the attached terminal rows in tree order, then the saved rows still waiting whose
    /// workspace is still here and that no attached row shows. `None` when there is neither, so a window with no
    /// remote row writes what it did before.
    pub(crate) fn remote_sessions_snapshot(&self) -> Option<Vec<RemoteSessionSnapshot>> {
        let mut rows: Vec<RemoteSessionSnapshot> = self
            .workspaces
            .iter()
            .flat_map(|workspace| self.attached_rows(workspace))
            .collect();
        let attached: HashSet<(String, String)> = rows.iter().map(saved_key).collect();
        rows.extend(
            self.saved_remote_rows
                .iter()
                .filter(|row| self.workspace(row.workspace_id).is_some() && !attached.contains(&saved_key(row)))
                .cloned(),
        );
        (!rows.is_empty()).then_some(rows)
    }

    /// Takes the saved rows a restore read, each machine's session once.
    pub(crate) fn restore_saved_remote_rows(&mut self, rows: Option<&[RemoteSessionSnapshot]>) {
        let mut seen: HashSet<(String, String)> = HashSet::new();
        self.saved_remote_rows = rows
            .unwrap_or_default()
            .iter()
            .filter(|row| seen.insert(saved_key(row)))
            .cloned()
            .collect();
        self.reattached_places.clear();
    }

    /// The saved rows still waiting to attach again.
    #[must_use]
    pub fn saved_remote_rows(&self) -> &[RemoteSessionSnapshot] {
        &self.saved_remote_rows
    }

    /// Drops `host`'s saved row of `session`, whether it attached (here or by hand), is gone or cannot attach;
    /// whether there was one.
    pub(crate) fn forget_saved_remote_row(&mut self, host: &str, session: &str) -> bool {
        let before = self.saved_remote_rows.len();
        self.saved_remote_rows
            .retain(|row| !(row.host == host && row.session_id == session));
        before != self.saved_remote_rows.len()
    }

    /// `peer` on `host` could not be listed at `now` (epoch seconds): its saved rows start waiting if they were not
    /// already, and with a `limit`, the ones that have waited longer than that many seconds are dropped. The names of
    /// the dropped rows.
    pub(crate) fn wait_saved_remote_rows(
        &mut self,
        (host, peer): (&str, RemotePeer),
        now: f64,
        limit: Option<f64>,
    ) -> Vec<String> {
        let mut expired = Vec::new();
        self.saved_remote_rows.retain_mut(|row| {
            if row.host != host || row.peer() != peer {
                return true;
            }
            let since = *row.waiting_since.get_or_insert(now);
            if limit.is_some_and(|limit| now - since > limit) {
                expired.push(row.name.clone());
                return false;
            }
            true
        });
        expired
    }

    /// Records where `row`, a row just attached again, was saved.
    pub(crate) fn record_reattached_place(&mut self, row: Uuid, place: SavedPlace) {
        self.reattached_places.insert(row, place);
    }

    /// Where saved top-level row `row` goes back among its workspace's rows: after the local row it followed (at the
    /// workspace's start when it followed none, at its end when that row is gone), past the rows no restore brings
    /// back that follow it, but before a row attached again from the same run with a later order. `None` when its
    /// workspace is gone.
    pub(crate) fn saved_remote_row_index(&self, row: &RemoteSessionSnapshot) -> Option<usize> {
        let sessions = &self.workspace(row.workspace_id)?.sessions;
        let mut at = match row.after_session_id {
            None => 0,
            Some(anchor) => sessions
                .iter()
                .position(|session| session.id() == anchor)
                .map_or(sessions.len(), |anchor| anchor + 1),
        };
        while let Some(next) = sessions.get(at).filter(|next| !next.is_persistable()) {
            let later = self
                .reattached_places
                .get(&next.id())
                .is_some_and(|(after, order)| *after == row.after_session_id && *order > row.order);
            if later {
                break;
            }
            at += 1;
        }
        Some(at)
    }

    /// `workspace`'s attached terminal rows that can be attached again, as saved rows: a page row is its origin's to
    /// show and comes back with it, and a row without its origin cannot be attached again. A row attached again from
    /// a saved one keeps its saved order while it still follows the same local row.
    fn attached_rows(&self, workspace: &Workspace) -> Vec<RemoteSessionSnapshot> {
        let parents: HashMap<Uuid, &str> = workspace
            .sessions
            .iter()
            .filter_map(|row| remote_origin(row).map(|(_, session)| (row.id(), session)))
            .collect();
        let mut rows = Vec::new();
        let mut after: Option<Uuid> = None;
        let mut next_order = 0;
        for row in &workspace.sessions {
            if row.is_persistable() {
                after = Some(row.id());
                next_order = 0;
                continue;
            }
            let Some((origin, session)) = remote_origin(row) else {
                continue;
            };
            let kept = self
                .reattached_places
                .get(&row.id())
                .filter(|(saved_after, _)| *saved_after == after)
                .map(|(_, order)| *order);
            let order = kept.map_or(next_order, |kept| kept.max(next_order));
            next_order = order + 1;
            rows.push(RemoteSessionSnapshot {
                host: origin.host.clone(),
                session_id: session.to_owned(),
                name: origin.session_name.clone(),
                local_name: row
                    .remote_presentation()
                    .filter(|state| state.name_overridden)
                    .map(|_| row.custom_name.clone().unwrap_or_default()),
                workspace_id: workspace.id(),
                after_session_id: after,
                order,
                parent_session_id: row
                    .parent_id()
                    .and_then(|parent| parents.get(&parent))
                    .map(|parent| (*parent).to_owned()),
                waiting_since: None,
                peer: Some(origin.endpoint.peer()).filter(|peer| *peer != RemotePeer::Ngterm),
            });
        }
        rows
    }
}

/// A terminal row's origin and the origin's id of its session; `None` for a local row, a page or a row bound without
/// an origin.
fn remote_origin(row: &Session) -> Option<(&super::remote_presentation::RemoteOrigin, &str)> {
    if row.remote_host().is_none() || row.content() != SessionContent::Terminal {
        return None;
    }
    let binding = &row.remote_presentation()?.binding;
    Some((binding.origin.as_ref()?, binding.remote_session_id.as_str()))
}

fn saved_key(row: &RemoteSessionSnapshot) -> (String, String) {
    (row.host.clone(), row.session_id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::NewSession;

    fn saved_ids(rows: &[RemoteSessionSnapshot]) -> Vec<&str> {
        rows.iter().map(|row| row.session_id.as_str()).collect()
    }

    fn saved(session: &str, workspace: Uuid, after: Option<Uuid>, order: usize) -> RemoteSessionSnapshot {
        RemoteSessionSnapshot {
            host: "studio".to_owned(),
            session_id: session.to_owned(),
            name: session.to_lowercase(),
            local_name: None,
            workspace_id: workspace,
            after_session_id: after,
            order,
            parent_session_id: None,
            waiting_since: None,
            peer: None,
        }
    }

    /// A window with one workspace of `count` local rows.
    fn window(count: usize) -> (AppStore, Uuid, Vec<Uuid>) {
        let mut store = AppStore::new();
        let workspace = store.add_workspace("work");
        let rows = (0..count)
            .map(|_| store.add_session(workspace, NewSession::at_cwd("/tmp")).unwrap())
            .collect();
        (store, workspace, rows)
    }

    #[test]
    fn a_restore_keeps_each_saved_session_once_and_a_snapshot_writes_the_ones_still_waiting() {
        let (mut store, workspace, _) = window(1);
        store.restore_saved_remote_rows(Some(&[
            saved("A", workspace, None, 0),
            saved("A", workspace, None, 3),
            saved("B", workspace, None, 1),
            saved("C", Uuid::from_u128(9), None, 0),
        ]));
        assert_eq!(saved_ids(store.saved_remote_rows()), ["A", "B", "C"]);
        assert_eq!(
            store.remote_sessions_snapshot().as_deref().map(saved_ids),
            Some(vec!["A", "B"]),
            "a row whose workspace is gone is not written"
        );
        assert!(store.forget_saved_remote_row("studio", "A"));
        assert!(!store.forget_saved_remote_row("studio", "A"));
        assert!(!store.forget_saved_remote_row("other", "B"));
        store.forget_saved_remote_row("studio", "B");
        store.forget_saved_remote_row("studio", "C");
        assert_eq!(store.remote_sessions_snapshot(), None);
    }

    #[test]
    fn a_saved_row_goes_back_after_the_local_row_it_followed() {
        let (store, workspace, rows) = window(3);
        let index = |row: &RemoteSessionSnapshot| store.saved_remote_row_index(row);
        assert_eq!(index(&saved("A", workspace, None, 0)), Some(0));
        assert_eq!(index(&saved("A", workspace, Some(rows[1]), 4)), Some(2));
        assert_eq!(
            index(&saved("A", workspace, Some(Uuid::from_u128(7)), 0)),
            Some(3),
            "its local row is gone"
        );
        assert_eq!(
            index(&saved("A", Uuid::from_u128(8), None, 0)),
            None,
            "its workspace is gone"
        );
    }

    #[test]
    fn a_machine_that_keeps_failing_drops_its_rows_once_they_waited_past_the_limit() {
        let (mut store, workspace, _) = window(1);
        let mut other = saved("B", workspace, None, 1);
        other.host = "other".to_owned();
        let mut agterm = saved("C", workspace, None, 2);
        agterm.peer = Some(RemotePeer::Agterm);
        store.restore_saved_remote_rows(Some(&[saved("A", workspace, None, 0), other, agterm]));
        assert!(
            store
                .wait_saved_remote_rows(("studio", RemotePeer::Ngterm), 100.0, Some(50.0))
                .is_empty()
        );
        assert_eq!(store.saved_remote_rows()[0].waiting_since, Some(100.0));
        assert!(
            store
                .wait_saved_remote_rows(("studio", RemotePeer::Ngterm), 150.0, Some(50.0))
                .is_empty(),
            "the first failure starts the wait"
        );
        assert!(
            store
                .wait_saved_remote_rows(("studio", RemotePeer::Ngterm), 900.0, None)
                .is_empty(),
            "no limit drops nothing"
        );
        assert_eq!(
            store.wait_saved_remote_rows(("studio", RemotePeer::Ngterm), 151.0, Some(50.0)),
            ["a"]
        );
        assert_eq!(
            saved_ids(store.saved_remote_rows()),
            ["B", "C"],
            "another machine's rows, and another app's on the same one, are their own"
        );
        assert!(store.saved_remote_rows().iter().all(|row| row.waiting_since.is_none()));
    }
}
