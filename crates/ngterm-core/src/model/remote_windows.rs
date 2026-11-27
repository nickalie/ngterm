//! Remote windows in the window set (upstream's `WindowLibrary+Remote.swift`): a window that mirrors one
//! `ngterm-headless` server's catalog instead of holding local sessions. Its rows come with the catalog stream, so it
//! is never seeded and never writes a snapshot of its own; the index keeps its host and the catalog it is bound to.

use uuid::Uuid;

use super::naming::sanitized;
use super::window_library::{WindowInfo, WindowLibrary};

/// The server a remote window mirrors (upstream's `WindowSource`): `catalog_id` is `None` until the first snapshot
/// binds the window to that server's catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteSource {
    pub host: String,
    pub catalog_id: Option<Uuid>,
}

/// What binding a remote window to the catalog its server reported settled to (upstream's `CatalogBinding`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogBinding {
    Bound,
    /// Another window already shows this catalog: the caller raises it and removes the new one.
    Duplicate(Uuid),
    /// The server reports another catalog than the one the window was bound to: it was reinstalled or its state
    /// replaced, so the rows are a different set.
    Rebound {
        previous: Uuid,
    },
}

impl WindowLibrary {
    /// Opens a window for `host`, or answers the one already showing it, opened when it was closed (upstream's
    /// `newRemoteWindow`): `(id, created)`. A new one is empty and frontmost; its rows come with its first snapshot.
    pub fn new_remote_window(&mut self, host: &str) -> (Uuid, bool) {
        if let Some((&id, _)) = self.remote_sources.iter().find(|(_, source)| source.host == host) {
            return (id, false);
        }
        let id = Uuid::new_v4();
        self.windows.push(WindowInfo {
            id,
            name: sanitized(host),
        });
        self.remote_sources.insert(
            id,
            RemoteSource {
                host: host.to_owned(),
                catalog_id: None,
            },
        );
        let store = self.blank_store();
        self.open_window(id, store);
        self.frontmost_window_id = Some(id);
        self.index_dirty = true;
        (id, true)
    }

    /// The server window `id` mirrors; `None` for a local window.
    #[must_use]
    pub fn remote_source(&self, id: Uuid) -> Option<&RemoteSource> {
        self.remote_sources.get(&id)
    }

    /// Whether window `id` mirrors a server.
    #[must_use]
    pub fn is_remote_window(&self, id: Uuid) -> bool {
        self.remote_sources.contains_key(&id)
    }

    /// Records that window `id` mirrors `source` (a launch reading the index).
    pub fn set_remote_source(&mut self, id: Uuid, source: RemoteSource) {
        self.remote_sources.insert(id, source);
    }

    /// The open remote windows, in window order.
    #[must_use]
    pub fn open_remote_windows(&self) -> Vec<Uuid> {
        self.open_ids()
            .into_iter()
            .filter(|id| self.is_remote_window(*id))
            .collect()
    }

    /// Records the catalog window `id`'s server reported, refusing one another window shows (upstream's
    /// `bindCatalog`).
    pub fn bind_catalog(&mut self, catalog_id: Uuid, id: Uuid) -> CatalogBinding {
        let Some(source) = self.remote_sources.get(&id) else {
            return CatalogBinding::Bound;
        };
        let previous = source.catalog_id;
        if let Some((&other, _)) = self
            .remote_sources
            .iter()
            .find(|(other, source)| **other != id && source.catalog_id == Some(catalog_id))
        {
            self.remote_duplicates.insert(id, other);
            return CatalogBinding::Duplicate(other);
        }
        if previous == Some(catalog_id) {
            return CatalogBinding::Bound;
        }
        if let Some(source) = self.remote_sources.get_mut(&id) {
            source.catalog_id = Some(catalog_id);
        }
        self.index_dirty = true;
        previous.map_or(CatalogBinding::Bound, |previous| CatalogBinding::Rebound { previous })
    }

    /// The window showing the catalog of window `id`'s server, once another window turned out to show it.
    #[must_use]
    pub fn remote_duplicate_of(&self, id: Uuid) -> Option<Uuid> {
        self.remote_duplicates.get(&id).copied()
    }
}
