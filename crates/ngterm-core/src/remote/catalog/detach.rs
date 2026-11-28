//! Detach and reattach in a remote window (upstream's `RemoteCatalogMirror+Detach.swift`): a detached session or
//! workspace is hidden on this machine and left running on the server. Nothing is sent, so it works while the window
//! is disconnected; the detached ids are saved in the window's view state, so a detach outlives the app.

use std::collections::BTreeSet;

use uuid::Uuid;

use super::view_state::{RemoteWindowViewState, SessionView};
use crate::Core;
use crate::catalog::frames::{CatalogSession, CatalogSnapshot, CatalogWorkspace, Lifecycle};
use crate::model::AppStore;
use crate::model::naming::sanitized;

/// Which of a remote window's detached sets an item is in (upstream's `RemoteDetachedItem.Kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetachedKind {
    Session,
    Workspace,
}

/// A detached workspace or session a remote window can reattach (upstream's `RemoteDetachedItem`): its record is
/// still in the catalog, and its workspace and parent are shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetachedItem {
    pub kind: DetachedKind,
    pub id: Uuid,
    pub name: String,
    pub ended: bool,
}

/// What closing a remote row reaches on the server (upstream's `RemoteCloseScope`): the row and, for a parent, every
/// companion in the catalog, detached ones included; `live` is whether any of them still runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseScope {
    pub sessions: Vec<Uuid>,
    pub live: bool,
}

impl Core {
    /// Hides sessions `ids` of remote window `window` here and leaves them running on the server; a parent takes its
    /// companions with it.
    pub(crate) fn detach_sessions(&mut self, window: Uuid, ids: &[Uuid]) {
        if let Some(store) = self.library.store_mut(window) {
            store.catalog_detached_sessions.extend(ids.iter().copied());
        }
        self.detach_changed(window);
    }

    /// Hides workspace `id` of remote window `window` and every row in it.
    pub(crate) fn detach_workspace(&mut self, window: Uuid, id: Uuid) {
        if let Some(store) = self.library.store_mut(window) {
            store.catalog_detached_workspaces.insert(id);
        }
        self.detach_changed(window);
    }

    /// Shows detached session `id` again, or answers why it cannot while its workspace or parent stays detached.
    pub(crate) fn reattach_session(&mut self, window: Uuid, id: Uuid) -> Result<(), String> {
        let catalog = self
            .remote
            .viewer
            .catalogs
            .get(&window)
            .and_then(|run| run.stream.catalog());
        let store = self
            .library
            .store_mut(window)
            .filter(|store| store.catalog_detached_sessions.contains(&id))
            .ok_or("the session is not detached")?;
        if let Some(catalog) = catalog {
            let workspace = catalog
                .workspaces
                .iter()
                .find(|workspace| workspace.sessions.iter().any(|session| session.id() == id));
            if workspace.is_some_and(|workspace| store.catalog_detached_workspaces.contains(&workspace.id)) {
                return Err("its workspace is detached; reattach the workspace".to_owned());
            }
            let fields = catalog.session(id).map(|session| &session.fields);
            if fields
                .and_then(|fields| fields.parent)
                .is_some_and(|parent| store.catalog_detached_sessions.contains(&parent))
            {
                return Err("its parent is detached; reattach the parent".to_owned());
            }
            if fields.is_some_and(|fields| fields.lifecycle == Lifecycle::Ended) {
                return Err("the session has ended; session.close removes its record".to_owned());
            }
        }
        store.catalog_detached_sessions.remove(&id);
        self.detach_changed(window);
        Ok(())
    }

    /// Shows detached workspace `id` again.
    pub(crate) fn reattach_workspace(&mut self, window: Uuid, id: Uuid) -> Result<(), String> {
        let store = self
            .library
            .store_mut(window)
            .filter(|store| store.catalog_detached_workspaces.contains(&id))
            .ok_or("the workspace is not detached")?;
        store.catalog_detached_workspaces.remove(&id);
        self.detach_changed(window);
        Ok(())
    }

    /// What remote window `window` can reattach, in catalog order. A session under a detached workspace or parent is
    /// not listed: reattaching it is refused until that ancestor is reattached.
    #[must_use]
    pub fn detached_items(&self, window: Uuid) -> Vec<DetachedItem> {
        let (Some(catalog), Some(store)) = (self.window_catalog(window), self.library.store(window)) else {
            return Vec::new();
        };
        let mut items = Vec::new();
        for workspace in &catalog.workspaces {
            if store.catalog_detached_workspaces.contains(&workspace.id) {
                items.push(DetachedItem {
                    kind: DetachedKind::Workspace,
                    id: workspace.id,
                    name: sanitized(&workspace.name),
                    ended: false,
                });
                continue;
            }
            for session in &workspace.sessions {
                let detached = &store.catalog_detached_sessions;
                if !detached.contains(&session.id()) || session.fields.parent.is_some_and(|p| detached.contains(&p)) {
                    continue;
                }
                items.push(DetachedItem {
                    kind: DetachedKind::Session,
                    id: session.id(),
                    name: sanitized(&session.fields.name),
                    ended: session.fields.lifecycle == Lifecycle::Ended,
                });
            }
        }
        items
    }

    /// Whether the server of remote window `window` would delete a workspace (upstream's `canTerminateWorkspace`): it
    /// keeps at least one, counting the ones this window detached.
    #[must_use]
    pub fn can_terminate_remote_workspace(&self, window: Uuid) -> bool {
        self.window_catalog(window)
            .is_some_and(|catalog| catalog.workspaces.len() > 1)
    }

    /// What closing row `session` of remote window `window` reaches on the server (upstream's `closeScope`); `None`
    /// for a row the catalog does not hold.
    #[must_use]
    pub fn remote_close_scope(&self, window: Uuid, session: Uuid) -> Option<CloseScope> {
        let catalog = self.window_catalog(window)?;
        let entry = catalog.session(session)?;
        let all = catalog.workspaces.iter().flat_map(|workspace| &workspace.sessions);
        let companions = all.filter(|other| entry.fields.parent.is_none() && other.fields.parent == Some(session));
        let group: Vec<_> = std::iter::once(entry).chain(companions).collect();
        Some(CloseScope {
            sessions: group.iter().map(|row| row.id()).collect(),
            // a running scratch is live work, so an ended row that still has one is asked about, never cleaned up
            live: group
                .iter()
                .any(|row| row.fields.lifecycle == Lifecycle::Live || row.fields.scratch.is_some()),
        })
    }

    /// The catalog remote window `window` last took.
    pub(crate) fn window_catalog(&self, window: Uuid) -> Option<&CatalogSnapshot> {
        self.remote.viewer.catalogs.get(&window)?.stream.catalog()
    }

    /// The window's rows follow the new detached sets, and its view state is saved now: a detach must outlive the app.
    fn detach_changed(&mut self, window: Uuid) {
        self.reconcile_catalog(window);
        if let Some(store) = self.library.store_mut(window) {
            store.schedule_tree_changed();
        }
        self.save_window_checked(window);
    }
}

/// `catalog` without what `store`'s window detached (upstream's `shownCatalog`), after forgetting detached ids whose
/// records the catalog no longer holds and keeping the views of the rows and workspaces the detach hides, defaults
/// included. Whether it forgot an id, so the view state is saved.
pub(super) fn shown_catalog(
    catalog: &CatalogSnapshot,
    store: &mut AppStore,
    view: &RemoteWindowViewState,
) -> (Vec<CatalogWorkspace>, bool) {
    let all: BTreeSet<Uuid> = catalog
        .workspaces
        .iter()
        .flat_map(|workspace| workspace.sessions.iter().map(CatalogSession::id))
        .collect();
    let workspace_records: BTreeSet<Uuid> = catalog.workspaces.iter().map(|workspace| workspace.id).collect();
    let sessions: BTreeSet<Uuid> = store.catalog_detached_sessions.intersection(&all).copied().collect();
    let workspaces: BTreeSet<Uuid> = store
        .catalog_detached_workspaces
        .intersection(&workspace_records)
        .copied()
        .collect();
    let forgot = sessions != store.catalog_detached_sessions || workspaces != store.catalog_detached_workspaces;
    store.catalog_detached_sessions.clone_from(&sessions);
    store.catalog_detached_workspaces.clone_from(&workspaces);
    let shown: Vec<CatalogWorkspace> = catalog
        .workspaces
        .iter()
        .filter(|workspace| !workspaces.contains(&workspace.id))
        .map(|workspace| CatalogWorkspace {
            sessions: workspace
                .sessions
                .iter()
                .filter(|session| {
                    !sessions.contains(&session.id())
                        && !session.fields.parent.is_some_and(|parent| sessions.contains(&parent))
                })
                .cloned()
                .collect(),
            ..workspace.clone()
        })
        .collect();
    let shown_ids: BTreeSet<Uuid> = shown
        .iter()
        .flat_map(|workspace| workspace.sessions.iter().map(CatalogSession::id))
        .collect();
    for id in all.difference(&shown_ids) {
        if store.catalog_hidden_views.contains_key(id) {
            continue;
        }
        let kept = store
            .session(*id)
            .map(SessionView::of)
            .or_else(|| view.sessions.get(id).cloned());
        if let Some(kept) = kept {
            store.catalog_hidden_views.insert(*id, kept);
        }
    }
    store.catalog_hidden_views.retain(|id, _| all.contains(id));
    for id in &workspaces {
        if store.catalog_hidden_collapsed.contains_key(id) {
            continue;
        }
        let collapsed = store.workspace(*id).map_or_else(
            || view.collapsed_workspaces.contains(id),
            |workspace| !workspace.is_expanded,
        );
        store.catalog_hidden_collapsed.insert(*id, collapsed);
    }
    store
        .catalog_hidden_collapsed
        .retain(|id, _| workspace_records.contains(id));
    (shown, forgot)
}
