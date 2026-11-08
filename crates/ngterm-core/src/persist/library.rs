//! The window library on disk (upstream's `WindowLibrary` init, `WindowLibrary+Index.swift`, the persistence half of
//! `loadStore`/`closeWindow`/`removeWindow`, and `AppStore+Persistence.swift`).
//!
//! The model records what it wants written in its outboxes; [`LibraryPersistence::commit`] drains them after a
//! mutation, writes the immediate saves and the index, and hands back the events and the debounced saves. A
//! debounced save is armed on the main loop ([`arm_debounced_saves`]) and lands through
//! [`LibraryPersistence::fire_debounced`] unless a later save of the same window superseded it, which is upstream's
//! `Debouncer` (a new schedule or an immediate `save()` cancels the pending one). No save happens on a cwd report:
//! the live cwd reaches disk with the next structural or debounced save. A soft close drops the window's armed
//! debounced save unwritten, and the commit applies the model's Open Recent changes to `recent-closed.json`, which
//! the bootstrap loads into the library.
//!
//! Recovery never fails: a corrupt, version-mismatched or empty index counts as absent, and the launch then recovers
//! the surviving `windows/<ID>.json` files, else migrates a legacy `workspaces.json`, else seeds one window. A
//! missing or corrupt window file opens as an empty tree. Write failures are logged and leave the model as it is; a
//! failed index write is retried once a snapshot write shows the disk takes writes again. A failed index write marks
//! the library ([`LibraryPersistence::index_unsaved`]) and a failed snapshot save its window
//! ([`LibraryPersistence::snapshot_unsaved`]) until a write of the same file lands.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ngterm_host::MainLoop;
use uuid::Uuid;

use super::parse_uuid_string;
use super::recent_closed::RecentClosedStore;
use super::snapshot::{PersistenceStore, Snapshot, WindowEntry, WindowsIndex};
use crate::model::{
    AppStore, PendingCloseTimer, RecentClosedChange, SaveRequest, WatermarkRemoval, WindowEvent, WindowInfo,
    WindowLibrary,
};
use crate::pane_env::uuid_string;
use crate::persist::settings::FlaggedViewLayout;
use crate::watermark;

/// The window index's file name in the state directory.
pub const INDEX_FILE_NAME: &str = "windows.json";
/// The directory of the per-window snapshots.
pub const WINDOWS_SUBDIRECTORY: &str = "windows";
/// The single-window snapshot of builds before the window library.
pub const LEGACY_FILE_NAME: &str = "workspaces.json";
/// The quiet window that coalesces selection and font-size writes (upstream's `saveDebounceInterval`).
pub const SAVE_DEBOUNCE: Duration = Duration::from_millis(300);

/// A debounced snapshot write waiting out [`SAVE_DEBOUNCE`]; stale once a later save of its window was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DebouncedSave {
    pub window: Uuid,
    token: u64,
}

/// What a commit leaves to the caller.
#[derive(Debug, Default)]
pub struct Commit {
    /// The model's events, in emission order, for the event ring.
    pub events: Vec<WindowEvent>,
    /// The debounced writes to arm on the main loop.
    pub debounced: Vec<DebouncedSave>,
    /// The soft closes whose grace timers to arm on the main loop.
    pub pending_closes: Vec<PendingCloseTimer>,
    /// The pane identities removed for good, whose live-session daemons the caller ends.
    pub finalized_panes: Vec<Uuid>,
}

/// The persistence of one window library, rooted at the state directory.
#[derive(Debug)]
pub struct LibraryPersistence {
    directory: PathBuf,
    /// Set from a failed index write until one lands (upstream's `indexUnsaved`).
    index_unsaved: bool,
    /// The open windows whose last snapshot save failed, each until one lands (upstream's per-store
    /// `snapshotUnsaved`). The launch's writes mark it as upstream's `store.save()` calls in its seed, migration and
    /// recovery do, all but the ones in [`Self::launch_unchecked`].
    snapshot_unsaved: HashSet<Uuid>,
    /// The windows whose only launch write is the rewrite after a failed capture strip, which upstream does not make
    /// (it just disarms), so it marks nothing; empty outside [`Self::bootstrap`].
    launch_unchecked: HashSet<Uuid>,
    /// The live debounced write of each window, by token.
    pending: HashMap<Uuid, u64>,
    next_token: u64,
}

impl LibraryPersistence {
    #[must_use]
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
            index_unsaved: false,
            snapshot_unsaved: HashSet::new(),
            launch_unchecked: HashSet::new(),
            pending: HashMap::new(),
            next_token: 0,
        }
    }

    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    #[must_use]
    pub fn index_path(&self) -> PathBuf {
        self.directory.join(INDEX_FILE_NAME)
    }

    fn windows_directory(&self) -> PathBuf {
        self.directory.join(WINDOWS_SUBDIRECTORY)
    }

    /// The snapshot file of window `id`: `windows/<ID>.json`, the id upper case as Foundation spells it.
    #[must_use]
    pub fn window_path(&self, id: Uuid) -> PathBuf {
        self.windows_directory().join(format!("{}.json", uuid_string(id)))
    }

    fn window_store(&self, id: Uuid) -> PersistenceStore {
        PersistenceStore::new(self.window_path(id))
    }

    /// Window `id`'s snapshot as its file holds it, read without writing anything.
    ///
    /// # Errors
    ///
    /// The file could not be read or decoded.
    pub fn read_window_checked(&self, id: Uuid) -> Result<Snapshot, super::snapshot::LoadError> {
        self.window_store(id).load_checked()
    }

    /// The window files on disk the index does not list, sorted by id: a stale index hides a surviving window's file,
    /// whose panes would otherwise read as unclaimed. `None` when the directory itself cannot be read, which is not
    /// the answer "none". A missing directory holds none.
    #[must_use]
    pub fn stray_window_ids(&self, indexed: &std::collections::HashSet<Uuid>) -> Option<Vec<Uuid>> {
        let entries = match fs::read_dir(self.windows_directory()) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Some(Vec::new()),
            Err(_) => return None,
        };
        let mut strays: Vec<Uuid> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let path = entry.path();
                if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                    return None;
                }
                parse_uuid_string(path.file_stem()?.to_str()?)
            })
            .filter(|id| !indexed.contains(id))
            .collect();
        strays.sort_by_key(|id| uuid_string(*id));
        Some(strays)
    }

    /// Whether the last index write failed and no later one landed.
    #[must_use]
    pub fn index_unsaved(&self) -> bool {
        self.index_unsaved
    }

    /// Whether open window `id`'s last snapshot save failed and no later one landed (`tree.snapshotUnsaved`).
    #[must_use]
    pub fn snapshot_unsaved(&self, id: Uuid) -> bool {
        self.snapshot_unsaved.contains(&id)
    }

    // MARK: - Bootstrap

    /// Builds the library at launch: a valid index reopens its open set (never windowless: the frontmost, else the
    /// first window), else surviving window files are recovered, else a legacy file migrates, else one window is
    /// seeded with a session in `home`. The launch emits no events; what it wants written is written now. The stores
    /// take the default flagged layout; [`Self::bootstrap_with_flagged_layout`] takes the saved setting.
    pub fn bootstrap(&mut self, home: &str) -> WindowLibrary {
        self.bootstrap_with_flagged_layout(home, FlaggedViewLayout::default())
    }

    /// [`Self::bootstrap`] with the saved flagged layout, which the restored stores need before they pick their
    /// selections.
    pub fn bootstrap_with_flagged_layout(&mut self, home: &str, flagged_layout: FlaggedViewLayout) -> WindowLibrary {
        let mut library = WindowLibrary::new(home);
        library.set_flagged_layout(flagged_layout);
        library.set_recent_closed_items(self.recent_closed_store().load());
        if let Some(index) = WindowsIndex::load(&self.index_path()) {
            for entry in &index.windows {
                library.insert_window(WindowInfo {
                    id: entry.id,
                    name: entry.name.clone(),
                });
            }
            library.frontmost_window_id = index.frontmost;
            self.reopen(&mut library, &index);
        } else if !self.recover_orphaned_windows(&mut library) && !self.migrate_legacy(&mut library) {
            library.new_window(None);
        }
        self.write_launch_saves(&mut library);
        library
    }

    /// Writes what the launch asked for, then the index. A failed write marks its window as upstream's
    /// `saveChecked` does, except a write upstream never makes ([`Self::launch_unchecked`]).
    fn write_launch_saves(&mut self, library: &mut WindowLibrary) {
        let unchecked = std::mem::take(&mut self.launch_unchecked);
        let effects = library.drain_effects();
        for (id, _) in effects.window_saves {
            if unchecked.contains(&id) {
                self.write_window(library, id);
            } else {
                self.save_window(library, id);
            }
        }
        self.save_index(library);
    }

    fn reopen(&mut self, library: &mut WindowLibrary, index: &WindowsIndex) {
        for entry in index.windows.iter().filter(|entry| entry.is_open) {
            self.load_store(library, entry.id, true);
        }
        if !library.open_ids().is_empty() {
            return;
        }
        let frontmost = index
            .frontmost
            .filter(|id| library.windows().iter().any(|info| info.id == *id));
        if let Some(fallback) = frontmost.or_else(|| library.windows().first().map(|info| info.id)) {
            self.load_store(library, fallback, true);
        }
    }

    /// Recovers every (upper-case) UUID-named `windows/*.json` into an open window named "window N", in file name order, the
    /// first frontmost. A recovered file may be stale, so its one-shot captures are disarmed; the sticky override
    /// stays armed. False when nothing is recoverable.
    fn recover_orphaned_windows(&mut self, library: &mut WindowLibrary) -> bool {
        let ids = window_file_ids(&self.windows_directory()).unwrap_or_default();
        if ids.is_empty() {
            return false;
        }
        for (offset, id) in ids.iter().enumerate() {
            library.insert_window(WindowInfo {
                id: *id,
                name: format!("window {}", offset + 1),
            });
        }
        for id in &ids {
            if !self.load_store(library, *id, true) {
                continue;
            }
            if let Some(store) = library.store_mut(*id) {
                disarm_captures(store);
            }
        }
        library.frontmost_window_id = ids.first().copied();
        true
    }

    /// Wraps a non-empty legacy `workspaces.json` into "window 1", open and frontmost; the legacy file stays.
    fn migrate_legacy(&mut self, library: &mut WindowLibrary) -> bool {
        let snapshot = PersistenceStore::new(self.directory.join(LEGACY_FILE_NAME)).load();
        if snapshot.workspaces.is_empty() {
            return false;
        }
        let id = Uuid::new_v4();
        let mut store = library.blank_store();
        store.restore(&snapshot, true);
        store.request_save(SaveRequest::Now);
        library.insert_window(WindowInfo {
            id,
            name: library.default_window_name(),
        });
        library.open_window(id, store);
        library.frontmost_window_id = Some(id);
        true
    }

    // MARK: - Loading

    /// Opens a closed window at runtime (upstream's `loadStore` from a window select or reveal): nothing armed, a
    /// stale capture scrubbed from the file, and every session announced. False for an id with no window; true for
    /// one already open. Commit afterwards.
    pub fn load_window(&mut self, library: &mut WindowLibrary, id: Uuid) -> bool {
        self.load_store(library, id, false)
    }

    fn load_store(&mut self, library: &mut WindowLibrary, id: Uuid, launch_restore: bool) -> bool {
        if !library.windows().iter().any(|info| info.id == id) {
            return false;
        }
        if library.is_open(id) {
            return true;
        }
        let snapshot = self.window_store(id).load();
        let mut store = library.blank_store();
        store.restore(&snapshot, launch_restore);
        let carried_captures = snapshot
            .workspaces
            .iter()
            .flat_map(|workspace| &workspace.sessions)
            .any(|session| session.foreground_command.is_some() || session.split_foreground_command.is_some());
        if launch_restore {
            // the armed copies live in the transient slots; strip the file so nothing can replay them twice, and
            // disarm when the strip cannot land; upstream writes nothing then, so a failure of the write the disarm
            // asks for marks nothing, unless the restore asked for one of its own
            if carried_captures && !self.strip_captures(id, &snapshot) {
                let asked_before = store.outbox().save != SaveRequest::None;
                if disarm_captures(&mut store) && !asked_before {
                    self.launch_unchecked.insert(id);
                }
            }
        } else {
            if carried_captures {
                store.request_save(SaveRequest::Now);
            }
            store.announce_sessions();
        }
        library.open_window(id, store);
        true
    }

    /// Rewrites a window's file without the one-shot captures, keeping everything else. Whether it landed.
    fn strip_captures(&self, id: Uuid, snapshot: &Snapshot) -> bool {
        let mut stripped = snapshot.clone();
        for session in stripped
            .workspaces
            .iter_mut()
            .flat_map(|workspace| workspace.sessions.iter_mut())
        {
            session.foreground_command = None;
            session.split_foreground_command = None;
        }
        match self.window_store(id).save(&stripped) {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(%error, window = %id, "stripping the captured commands failed");
                false
            }
        }
    }

    // MARK: - Saving

    /// Drains the library's effects: writes the immediate saves, a changed index and the Open Recent changes (reloading
    /// the library's list from `recent-closed.json` after them), and returns the events and the debounced saves to arm.
    pub fn commit(&mut self, library: &mut WindowLibrary) -> Commit {
        let effects = library.drain_effects();
        for window in &effects.cancelled_saves {
            self.pending.remove(window);
        }
        if !effects.recent_closed.is_empty() {
            // reload as upstream's `refreshRecentClosedItems` after each `RecentClosedStore` call does, so a write that
            // failed shows as what the file still holds rather than as the change the model made
            self.apply_recent_closed(&effects.recent_closed);
            library.set_recent_closed_items(self.recent_closed_store().load());
        }
        self.remove_watermarks(&effects.removed_watermarks);
        let mut debounced = Vec::new();
        for (id, request) in effects.window_saves {
            match request {
                SaveRequest::Now => {
                    self.save_window(library, id);
                }
                SaveRequest::Debounced => debounced.push(self.debounce(id)),
                SaveRequest::None => {}
            }
        }
        if effects.index_save {
            self.save_index(library);
        }
        Commit {
            events: effects.events,
            debounced,
            pending_closes: effects.pending_closes,
            finalized_panes: effects.finalized_panes,
        }
    }

    /// The Open Recent file of the state directory.
    #[must_use]
    pub fn recent_closed_store(&self) -> RecentClosedStore {
        RecentClosedStore::new(&self.directory)
    }

    /// Writes the Open Recent changes `library` recorded since the last drain to `recent-closed.json`, then reloads the
    /// library's list from the file, so a write that failed shows as the entry it left behind (upstream's
    /// `refreshRecentClosedItems` after each `RecentClosedStore` call, whose `save` logs a failure and returns). The
    /// other effects wait for the next [`LibraryPersistence::commit`].
    pub fn sync_recent_closed(&self, library: &mut WindowLibrary) {
        let changes = library.take_recent_closed_changes();
        self.apply_recent_closed(&changes);
        library.set_recent_closed_items(self.recent_closed_store().load());
    }

    /// Applies the model's Open Recent changes to `recent-closed.json`, in order; a failed write is logged and the
    /// next change starts from what the file holds.
    fn apply_recent_closed(&self, changes: &[RecentClosedChange]) {
        if changes.is_empty() {
            return;
        }
        let store = self.recent_closed_store();
        for change in changes {
            let written = match change {
                RecentClosedChange::Record(item) => store.record(item.as_ref().clone()),
                RecentClosedChange::Remove(id) => store.remove(*id),
                RecentClosedChange::Clear => store.clear(),
            };
            if let Err(error) = written {
                tracing::warn!(%error, "saving the recent closed list failed");
            }
        }
    }

    /// Deletes the rendered text watermarks the model let go of, under `<state dir>/watermarks`.
    fn remove_watermarks(&self, removals: &[WatermarkRemoval]) {
        for removal in removals {
            match removal {
                WatermarkRemoval::Session(session) => watermark::remove_all_rendered_text(&self.directory, *session),
                WatermarkRemoval::Pane { session, key } => {
                    watermark::remove_rendered_text(&self.directory, *session, Some(key.as_str()));
                }
            }
        }
    }

    fn debounce(&mut self, window: Uuid) -> DebouncedSave {
        self.next_token += 1;
        self.pending.insert(window, self.next_token);
        DebouncedSave {
            window,
            token: self.next_token,
        }
    }

    /// Lands a debounced save whose quiet window elapsed, unless a later save superseded it or its window closed.
    /// Whether it wrote.
    pub fn fire_debounced(&mut self, library: &WindowLibrary, save: DebouncedSave) -> bool {
        if self.pending.get(&save.window) != Some(&save.token) {
            return false;
        }
        self.save_window(library, save.window)
    }

    /// Writes an open window's snapshot now, cancelling its debounced write (upstream's `saveChecked`). Whether it
    /// landed; false for a closed window. A failed write marks the window [`Self::snapshot_unsaved`] and one that lands
    /// clears it.
    pub fn save_window(&mut self, library: &WindowLibrary, id: Uuid) -> bool {
        let Some(landed) = self.write_window(library, id) else {
            return false;
        };
        if landed {
            self.snapshot_unsaved.remove(&id);
        } else {
            self.snapshot_unsaved.insert(id);
        }
        landed
    }

    /// Writes an open window's snapshot now, cancelling its debounced write, without touching its unsaved mark.
    /// Whether it landed; `None` for a closed window.
    fn write_window(&mut self, library: &WindowLibrary, id: Uuid) -> Option<bool> {
        self.pending.remove(&id);
        let store = library.store(id)?;
        match self.window_store(id).save(&store.snapshot()) {
            Ok(()) => {
                if self.index_unsaved {
                    self.save_index(library);
                }
                Some(true)
            }
            Err(error) => {
                tracing::warn!(%error, window = %id, "saving the window failed");
                Some(false)
            }
        }
    }

    /// Writes `windows.json`: the window list with open flags, and the frontmost window. Whether it landed.
    pub fn save_index(&mut self, library: &WindowLibrary) -> bool {
        let entries = library
            .windows()
            .iter()
            .map(|info| WindowEntry {
                id: info.id,
                name: info.name.clone(),
                is_open: library.is_open(info.id),
            })
            .collect();
        let index = WindowsIndex::new(library.frontmost_window_id, entries);
        match super::json::save_json(&self.index_path(), &index) {
            Ok(()) => {
                self.index_unsaved = false;
                true
            }
            Err(error) => {
                tracing::warn!(%error, "saving the window index failed");
                self.index_unsaved = true;
                false
            }
        }
    }

    /// Writes every open window's snapshot, then the index, attempting all; whether every write landed (the quit
    /// flush and `restore.capture`).
    pub fn save_all_checked(&mut self, library: &WindowLibrary) -> bool {
        let mut landed = true;
        for id in library.open_ids() {
            landed &= self.save_window(library, id);
        }
        self.save_index(library) && landed
    }

    // MARK: - Window lifecycle

    /// Closes a window: writes its snapshot, then drops its store (upstream's window `willClose` save before
    /// `closeWindow`) and its unsaved mark with it. A no-op while terminating, so the open set survives for the next
    /// launch. Commit afterwards.
    pub fn close_window(&mut self, library: &mut WindowLibrary, id: Uuid) {
        if library.is_terminating || !library.is_open(id) {
            return;
        }
        self.save_window(library, id);
        library.close_window(id);
        if !library.is_open(id) {
            self.snapshot_unsaved.remove(&id);
        }
    }

    /// Removes a window for good: its pending write is dropped and its file deleted. Keeps the last window. Commit
    /// afterwards.
    pub fn remove_window(&mut self, library: &mut WindowLibrary, id: Uuid) {
        if !library.can_remove_window() || !library.windows().iter().any(|info| info.id == id) {
            return;
        }
        self.pending.remove(&id);
        self.snapshot_unsaved.remove(&id);
        // a closed window's sessions are destroyed for good too, so their rendered text goes now, read from its file;
        // an open window's leave through the library's effects
        if !library.is_open(id) {
            for workspace in self.window_store(id).load().workspaces {
                for session in workspace.sessions {
                    watermark::remove_all_rendered_text(&self.directory, session.id);
                    // and its live-session daemons
                    library.note_finalized_panes(session.pane_identity.into_iter().chain(session.split_pane_identity));
                }
            }
        }
        library.remove_window(id);
        // best effort: a window that never wrote its file has nothing to remove
        let _ = fs::remove_file(self.window_path(id));
    }

    /// Clears every session's font size in every window: open ones live, closed ones by rewriting their file when
    /// it holds any. Commit afterwards.
    pub fn reset_session_font_sizes_all_windows(&mut self, library: &mut WindowLibrary) {
        library.reset_session_font_sizes_all_windows();
        for info in library.windows() {
            if library.is_open(info.id) {
                continue;
            }
            let store = self.window_store(info.id);
            let mut snapshot = store.load();
            let mut changed = false;
            for session in snapshot
                .workspaces
                .iter_mut()
                .flat_map(|workspace| workspace.sessions.iter_mut())
            {
                changed |= session.font_size.take().is_some();
            }
            if changed && let Err(error) = store.save(&snapshot) {
                tracing::warn!(%error, window = %info.id, "clearing a closed window's font sizes failed");
            }
        }
    }
}

/// Drops every armed one-shot capture of a store and asks for a write when there was one, which it returns; the sticky
/// overrides stay.
fn disarm_captures(store: &mut AppStore) -> bool {
    let mut stripped = false;
    for session in store.sessions_mut() {
        if session.restore.pending_foreground.is_some() || session.split_restore.pending_foreground.is_some() {
            session.restore.disarm_foreground();
            session.split_restore.disarm_foreground();
            stripped = true;
        }
    }
    if stripped {
        store.request_save(SaveRequest::Now);
    }
    stripped
}

/// The ids of the UUID-named `.json` files in the windows directory, in file name order; `None` when the directory
/// cannot be read.
///
/// Only a stem spelled as [`LibraryPersistence::window_path`] spells it (upper case) counts. Upstream accepts either
/// case, but it runs on a case-insensitive file system where both name one file; on a case-sensitive one the window
/// would load from the upper-case path, find nothing and open empty, stranding the tree in the other spelling. Such a
/// file is left on disk untouched instead.
fn window_file_ids(directory: &Path) -> Option<Vec<Uuid>> {
    let mut names: Vec<String> = fs::read_dir(directory)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    Some(
        names
            .iter()
            .filter_map(|name| name.strip_suffix(".json"))
            .filter_map(|stem| parse_uuid_string(stem).filter(|id| uuid_string(*id) == stem))
            .collect(),
    )
}

/// Arms each debounced save on the main loop: after [`SAVE_DEBOUNCE`] the task hands it to `fire`, which reaches the
/// state's [`LibraryPersistence::fire_debounced`]. A closed loop drops it: the quit path writes everything anyway.
pub fn arm_debounced_saves<S: 'static>(
    main_loop: &dyn MainLoop<S>,
    saves: &[DebouncedSave],
    fire: fn(&mut S, DebouncedSave),
) {
    for save in saves.iter().copied() {
        let _ = main_loop.post_after(SAVE_DEBOUNCE, Box::new(move |state: &mut S| fire(state, save)));
    }
}

#[cfg(test)]
mod tests {
    use super::super::snapshot::{SessionSnapshot, WorkspaceSnapshot};
    use super::*;

    /// Recovery rewrites a window whose captures it disarmed after the strip landed (upstream's `store.save()` in
    /// `recoverOrphanedWindows`), so that write failing marks the window. The strip and the rewrite hit one file, so
    /// the failure goes in between them.
    #[test]
    fn a_failed_recovery_rewrite_marks_the_window() {
        let dir = tempfile::tempdir().unwrap();
        let mut persistence = LibraryPersistence::new(dir.path());
        let (window, session) = (Uuid::new_v4(), Uuid::new_v4());
        let snapshot = Snapshot {
            workspaces: vec![WorkspaceSnapshot {
                id: Uuid::new_v4(),
                name: "work".to_owned(),
                sessions: vec![SessionSnapshot {
                    foreground_command: Some(vec!["ssh".to_owned(), "prod".to_owned()]),
                    ..SessionSnapshot::new(session, "/a")
                }],
                collapsed: None,
                tint: None,
            }],
            ..Snapshot::default()
        };
        let file = persistence.window_path(window);
        PersistenceStore::new(&file).save(&snapshot).unwrap();

        let mut library = WindowLibrary::new("/home/user");
        assert!(persistence.recover_orphaned_windows(&mut library));
        let on_disk = PersistenceStore::new(&file).load();
        assert_eq!(
            on_disk.workspaces[0].sessions[0].foreground_command, None,
            "the strip landed"
        );
        fs::remove_file(&file).unwrap();
        fs::create_dir_all(file.join("occupied")).unwrap();
        persistence.write_launch_saves(&mut library);
        assert!(persistence.snapshot_unsaved(window));
    }
}
