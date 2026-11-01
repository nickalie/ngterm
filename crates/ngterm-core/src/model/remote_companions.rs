//! The store half of a remote row's companions (upstream's `addReplicaPageCompanion` in `AppStore+ReplicaPage.swift`
//! and the parent placement of `attach(_:from:host:placement:)`): a companion the origin of a remote row created is
//! attached here under that row, after its group, never created here.

use uuid::Uuid;

use super::companions::{SessionContent, group_end};
use super::html_overlay::HtmlOverlayOpenFailure;
use super::store::{AppStore, NewSession};
use crate::html::HtmlOverlay;

/// Where an attached companion of the remote row `parent` goes: its workspace and the end of its group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteCompanionPlace {
    pub workspace: Uuid,
    pub index: usize,
}

/// Why another machine's page companion was not shown here (upstream's `ReplicaPageRefusal` messages).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplicaPageRefusal {
    /// "the parent row here is gone": no top-level remote row `parent` in this store.
    ParentGone,
    /// The page's slot refused it; the row was not kept.
    NotOpened(HtmlOverlayOpenFailure),
}

impl AppStore {
    /// Where a companion attached under `parent` goes: `parent` must be a top-level row attached from another machine,
    /// which a row here created never is.
    #[must_use]
    pub fn remote_companion_place(&self, parent: Uuid) -> Option<RemoteCompanionPlace> {
        let row = self.session(parent)?;
        if row.parent_id().is_some() || row.remote_host().is_none() {
            return None;
        }
        let at = self.location(parent)?;
        let workspace = &self.workspaces[at.workspace];
        Some(RemoteCompanionPlace {
            workspace: workspace.id(),
            index: group_end(at.session, &workspace.sessions),
        })
    }

    /// Shows another machine's page companion `overlay` (its `replica_of` set) as a companion of `parent`, a row
    /// attached from that machine (upstream's `addReplicaPageCompanion`): the row carries the parent's host, the page is
    /// its own full-size slot, and the page stays the other machine's, so closing the row here ends nothing there. The
    /// caller registers the page's outcome.
    ///
    /// # Errors
    ///
    /// The parent is gone or not a top-level remote row, or the page could not take the row's slot (no row is kept).
    pub fn add_replica_page_companion(
        &mut self,
        parent: Uuid,
        overlay: HtmlOverlay,
        background_color: Option<String>,
        select: bool,
        home: &str,
    ) -> Result<Uuid, ReplicaPageRefusal> {
        let place = self
            .remote_companion_place(parent)
            .ok_or(ReplicaPageRefusal::ParentGone)?;
        let host = self
            .session(parent)
            .and_then(|row| row.remote_host())
            .map(str::to_owned);
        let spec = NewSession {
            at: Some(place.index),
            select,
            remote_host: host,
            parent: Some(parent),
            content: SessionContent::Page,
            ..NewSession::at_cwd(home)
        };
        let row = self
            .add_session(place.workspace, spec)
            .ok_or(ReplicaPageRefusal::ParentGone)?;
        let overlay = HtmlOverlay { row: true, ..overlay };
        if let Err(failure) = self.open_html_overlay(row, None, overlay, None, background_color) {
            self.close_session(row);
            return Err(ReplicaPageRefusal::NotOpened(failure));
        }
        Ok(row)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::{HtmlPageFlags, HtmlSource};

    fn page() -> HtmlOverlay {
        let source = HtmlSource::Replica {
            path: "/srv/report.html".to_owned(),
        };
        let mut overlay = HtmlOverlay::new(source, HtmlPageFlags::default());
        overlay.replica_of = Some(Uuid::new_v4());
        overlay
    }

    fn store_with(remote: bool) -> (AppStore, Uuid) {
        let mut store = AppStore::new();
        let workspace = store.add_workspace("work");
        let spec = NewSession {
            remote_host: remote.then(|| "buildbox".to_owned()),
            ..NewSession::at_cwd("/home/u")
        };
        let parent = store.add_session(workspace, spec).unwrap();
        (store, parent)
    }

    /// Upstream's `aReplicaPageCompanionClosesOnceWithItsPage`: closing the page closes its row once, never the parent.
    #[test]
    fn a_replica_page_companion_closes_once_with_its_page() {
        let (mut store, parent) = store_with(true);
        let row = store
            .add_replica_page_companion(parent, page(), None, false, "/home/u")
            .unwrap();
        let found = store.session(row).unwrap();
        assert_eq!(found.parent_id(), Some(parent));
        assert_eq!(found.content(), SessionContent::Page);
        assert_eq!(found.remote_host(), Some("buildbox"));
        assert_ne!(store.selected_session_id(), Some(row));

        assert!(store.close_overlay(row));

        assert!(store.session(row).is_none());
        assert!(store.session(parent).is_some());
    }

    #[test]
    fn only_a_top_level_remote_row_takes_another_machines_page_companion() {
        let (mut local, parent) = store_with(false);
        assert_eq!(
            local.add_replica_page_companion(parent, page(), None, true, "/home/u"),
            Err(ReplicaPageRefusal::ParentGone)
        );
        let (mut store, parent) = store_with(true);
        let row = store
            .add_replica_page_companion(parent, page(), None, true, "/home/u")
            .unwrap();
        assert_eq!(store.selected_session_id(), Some(row));
        assert_eq!(
            store.add_replica_page_companion(row, page(), None, true, "/home/u"),
            Err(ReplicaPageRefusal::ParentGone),
            "a companion has no companions of its own"
        );
        assert_eq!(store.remote_companion_place(Uuid::new_v4()), None);
    }
}
