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

/// Where another machine's top-level page session goes here: a workspace, the origin's host and the row's name.
pub struct ReplicaPageRoot {
    pub workspace: Uuid,
    pub host: String,
    pub name: Option<String>,
}

/// A remote page and its origin session id, fixed before row creation.
pub struct ReplicaPageSeed {
    pub overlay: HtmlOverlay,
    pub background_color: Option<String>,
    pub remote_session_id: Option<String>,
}

/// Why another machine's page companion was not shown here (upstream's `ReplicaPageRefusal` messages).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplicaPageRefusal {
    /// "the parent row here is gone": no top-level remote row `parent` in this store.
    ParentGone,
    /// "the workspace here is gone": no workspace for a top-level page row.
    WorkspaceGone,
    /// The page's slot refused it; the row was not kept.
    NotOpened(HtmlOverlayOpenFailure),
}

impl ReplicaPageRefusal {
    /// Upstream's `ReplicaPageRefusal` message; a slot's refusal is worded by its caller.
    #[must_use]
    pub fn message(&self) -> Option<&'static str> {
        match self {
            Self::ParentGone => Some("the parent row here is gone"),
            Self::WorkspaceGone => Some("the workspace here is gone"),
            Self::NotOpened(_) => None,
        }
    }
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

    /// Shows another machine's page companion (`page.overlay.replica_of` set) as a companion of `parent`, a row
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
        page: ReplicaPageSeed,
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
            remote_session_id: page.remote_session_id.clone(),
            parent: Some(parent),
            content: SessionContent::Page,
            ..NewSession::at_cwd(home)
        };
        let row = self
            .add_session(place.workspace, spec)
            .ok_or(ReplicaPageRefusal::ParentGone)?;
        self.show_replica_page(row, page)
    }

    /// Shows another machine's top-level page session as a top-level row at the end of `root.workspace`
    /// (upstream's `addReplicaPageRoot`): like a replica page companion, it stays the other machine's page.
    ///
    /// # Errors
    ///
    /// The workspace is gone, or the page could not take the row's slot (no row is kept).
    pub fn add_replica_page_root(
        &mut self,
        root: ReplicaPageRoot,
        page: ReplicaPageSeed,
        select: bool,
        home: &str,
    ) -> Result<Uuid, ReplicaPageRefusal> {
        let spec = NewSession {
            name: root.name,
            select,
            remote_host: Some(root.host),
            remote_session_id: page.remote_session_id.clone(),
            content: SessionContent::Page,
            ..NewSession::at_cwd(home)
        };
        let row = self
            .add_session(root.workspace, spec)
            .ok_or(ReplicaPageRefusal::WorkspaceGone)?;
        self.show_replica_page(row, page)
    }

    fn show_replica_page(&mut self, row: Uuid, page: ReplicaPageSeed) -> Result<Uuid, ReplicaPageRefusal> {
        let overlay = HtmlOverlay {
            row: true,
            ..page.overlay
        };
        if let Err(failure) = self.open_html_overlay(row, None, overlay, None, page.background_color) {
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

    fn page() -> ReplicaPageSeed {
        let source = HtmlSource::Replica {
            path: "/srv/report.html".to_owned(),
            content: crate::html::ReplicaContent::Html,
        };
        let mut overlay = HtmlOverlay::new(source, HtmlPageFlags::default());
        overlay.replica_of = Some(Uuid::new_v4());
        ReplicaPageSeed {
            overlay,
            background_color: None,
            remote_session_id: None,
        }
    }

    fn root_in(workspace: Uuid, name: Option<&str>) -> ReplicaPageRoot {
        ReplicaPageRoot {
            workspace,
            host: "buildbox".to_owned(),
            name: name.map(str::to_owned),
        }
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
            .add_replica_page_companion(parent, page(), false, "/home/u")
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
            local.add_replica_page_companion(parent, page(), true, "/home/u"),
            Err(ReplicaPageRefusal::ParentGone)
        );
        let (mut store, parent) = store_with(true);
        let row = store
            .add_replica_page_companion(parent, page(), true, "/home/u")
            .unwrap();
        assert_eq!(store.selected_session_id(), Some(row));
        assert_eq!(
            store.add_replica_page_companion(row, page(), true, "/home/u"),
            Err(ReplicaPageRefusal::ParentGone),
            "a companion has no companions of its own"
        );
        assert_eq!(store.remote_companion_place(Uuid::new_v4()), None);
    }

    /// Upstream's `aReplicaPageRootIsATopLevelRowAtTheEndOfItsWorkspace`.
    #[test]
    fn a_replica_page_root_is_a_top_level_row_at_the_end_of_its_workspace() {
        let (mut store, local) = store_with(false);
        let workspace = store.workspace_for_session(local).unwrap().id();
        let seed = page();
        let origin = seed.overlay.replica_of;
        let row = store
            .add_replica_page_root(root_in(workspace, Some("dash")), seed, true, "/home/u")
            .unwrap();
        let found = store.session(row).unwrap();
        assert_eq!(found.parent_id(), None);
        assert_eq!(found.content(), SessionContent::Page);
        assert_eq!(found.remote_host(), Some("buildbox"));
        assert_eq!(found.display_name(), "dash");
        let ids: Vec<Uuid> = store.sessions().map(crate::model::Session::id).collect();
        assert_eq!(ids, [local, row]);
        assert_eq!(store.selected_session_id(), Some(row));
        let shown = found.overlay.page().unwrap();
        assert_eq!(shown.replica_of, origin);
        assert!(shown.row);
    }

    /// Upstream's `aReplicaPageRootAndItsCompanionsAreAbsentFromTheSnapshot`.
    #[test]
    fn a_replica_page_root_and_its_companions_are_absent_from_the_snapshot() {
        let (mut store, local) = store_with(false);
        let workspace = store.workspace_for_session(local).unwrap().id();
        let root = store
            .add_replica_page_root(root_in(workspace, None), page(), false, "/home/u")
            .unwrap();
        let child = store
            .add_replica_page_companion(root, page(), false, "/home/u")
            .unwrap();
        let terminal = store
            .add_session(
                workspace,
                NewSession {
                    remote_host: Some("buildbox".to_owned()),
                    parent: Some(root),
                    ..NewSession::at_cwd("/home/u")
                },
            )
            .unwrap();

        let companions: Vec<Uuid> = store.companions(root).iter().map(|row| row.id()).collect();
        assert_eq!(companions, [child, terminal]);
        let saved: Vec<Uuid> = store.snapshot().workspaces[0]
            .sessions
            .iter()
            .map(|session| session.id)
            .collect();
        assert_eq!(saved, [local]);
    }

    /// Upstream's `aReplicaPageRootIntoAMissingWorkspaceCreatesNothing`.
    #[test]
    fn a_replica_page_root_into_a_missing_workspace_creates_nothing() {
        let (mut store, _) = store_with(false);
        let before = store.sessions().count();
        let result = store.add_replica_page_root(root_in(Uuid::new_v4(), None), page(), true, "/home/u");
        assert_eq!(result, Err(ReplicaPageRefusal::WorkspaceGone));
        assert_eq!(
            ReplicaPageRefusal::WorkspaceGone.message(),
            Some("the workspace here is gone")
        );
        assert_eq!(store.sessions().count(), before);
    }
}
