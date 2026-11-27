//! A remote window's rows, brought to its server's catalog (upstream's `AppStore+Catalog.swift`). The catalog is the
//! truth: a row is removed only when its session leaves the catalog, an ended session keeps its row and last screen,
//! and the identities of rows, panes and workspaces are the server's own, so a row is found again by its id.
//!
//! What a pane runs comes from the caller ([`CatalogRowCommands`]): the attach of a server pane, the attach of a
//! row's server scratch, and the line an ended row holds in place of a shell.

use std::collections::HashSet;

use ngterm_proto::EventKind;
use uuid::Uuid;

use super::companions::SessionContent;
use super::naming::{sanitized, trimmed_or_none};
use super::pane::SplitAxis;
use super::remote_presentation::{RemoteBinding, RemoteOrigin, RemotePresentationState};
use super::session::{CatalogScratchMark, Session, SessionSeed};
use super::store::AppStore;
use super::workspace::Workspace;
use crate::catalog::frames::{
    CatalogPane, CatalogScratch, CatalogSession, CatalogSessionFields, CatalogWorkspace, Lifecycle,
};
use crate::pane_env::uuid_string;

/// The commands a remote window's panes run (upstream's `RemoteCatalogEffects` attach commands).
pub trait CatalogRowCommands {
    /// The attach of server pane `pane`.
    fn attach(&mut self, pane: &CatalogPane) -> String;
    /// The attach of a row's server scratch `pane`.
    fn scratch(&mut self, pane: &CatalogPane) -> String;
    /// What a row whose session ended runs in place of an attach.
    fn ended(&mut self) -> String;
}

/// What building or updating a row needs beyond its catalog entry (upstream's `CatalogRowContext`).
pub struct CatalogRowContext<'a> {
    pub host: &'a str,
    /// The server's presentation protocol version.
    pub protocol: i64,
    pub commands: &'a mut dyn CatalogRowCommands,
}

impl Session {
    /// Whether a remote window's row keeps its presentation stream (upstream's `presentsCatalog`): live, or ended
    /// with its server scratch still running.
    #[must_use]
    pub fn presents_catalog(&self) -> bool {
        !self.ended || self.catalog_scratch.is_some()
    }
}

impl AppStore {
    /// Removes the rows of sessions `catalog` no longer has and adds or renames its workspaces, a new one expanded
    /// unless `collapsed` names it (upstream's `prepareCatalogWorkspaces`). Workspaces it no longer has leave in
    /// [`AppStore::finish_catalog_workspaces`], once their surviving rows moved out. The removed rows' ids.
    pub fn prepare_catalog_workspaces(&mut self, catalog: &[CatalogWorkspace], collapsed: &HashSet<Uuid>) -> Vec<Uuid> {
        let wanted: HashSet<Uuid> = catalog
            .iter()
            .flat_map(|workspace| &workspace.sessions)
            .map(CatalogSession::id)
            .collect();
        let gone: Vec<Uuid> = self
            .sessions()
            .map(Session::id)
            .filter(|id| !wanted.contains(id))
            .collect();
        for id in &gone {
            self.remove_catalog_row(*id);
        }
        let mut changed = false;
        for (index, entry) in catalog.iter().enumerate() {
            let name = sanitized(&entry.name);
            if let Some(existing) = self.workspace_index(entry.id) {
                if self.workspaces[existing].name != name {
                    self.workspaces[existing].name = name;
                    changed = true;
                }
                continue;
            }
            let mut workspace = Workspace::with_id(entry.id, name);
            workspace.is_expanded = !collapsed.contains(&entry.id);
            let at = index.min(self.workspaces.len());
            self.workspaces.insert(at, workspace);
            changed = true;
        }
        if changed {
            self.schedule_tree_changed();
        }
        gone
    }

    /// Drops the workspaces `catalog` no longer has and puts the rest in its order (upstream's
    /// `finishCatalogWorkspaces`).
    pub fn finish_catalog_workspaces(&mut self, catalog: &[CatalogWorkspace]) {
        let order: Vec<Uuid> = catalog.iter().map(|workspace| workspace.id).collect();
        let before: Vec<Uuid> = self.workspaces.iter().map(Workspace::id).collect();
        self.workspaces.retain(|workspace| order.contains(&workspace.id()));
        self.workspaces
            .sort_by_key(|workspace| order.iter().position(|id| *id == workspace.id()));
        if self.fresh_workspace_id.is_some_and(|fresh| !order.contains(&fresh)) {
            self.fresh_workspace_id = None;
        }
        if self.workspaces.iter().map(Workspace::id).ne(before) {
            self.schedule_tree_changed();
        }
    }

    /// Puts row `id` at `index` of workspace `workspace`, keeping its terminals (upstream's `moveCatalogSession`).
    pub fn move_catalog_session(&mut self, id: Uuid, workspace: Uuid, index: usize) {
        let Some(destination) = self.workspace_index(workspace) else {
            return;
        };
        let current = self.workspaces[destination]
            .sessions
            .iter()
            .position(|session| session.id() == id);
        if current == Some(index) {
            return;
        }
        let Some(at) = self.location(id) else {
            return;
        };
        let session = self.workspaces[at.workspace].sessions.remove(at.session);
        let sessions = &mut self.workspaces[destination].sessions;
        sessions.insert(index.min(sessions.len()), session);
        self.schedule_tree_changed();
    }

    /// Creates the row of catalog session `entry` at `index` of workspace `workspace`, each pane running its attach
    /// (upstream's `insertCatalogSession`). An ended session gets a row holding the ended line, and no attach.
    pub fn insert_catalog_session(
        &mut self,
        entry: &CatalogSession,
        workspace: Uuid,
        index: usize,
        context: &mut CatalogRowContext<'_>,
    ) -> Option<Uuid> {
        let at = self.workspace_index(workspace)?;
        let fields = &entry.fields;
        let mut session = Session::new(SessionSeed {
            id: Some(fields.id),
            initial_cwd: sanitized(&fields.cwd),
            custom_name: catalog_custom_name(fields),
            pane_identity: Some(entry.panes.first().map_or(fields.primary, |pane| pane.identity)),
            remote_host: Some(context.host.to_owned()),
            remote_session_id: Some(uuid_string(fields.id)),
            parent_id: fields.parent,
            content: if fields.page.is_some() {
                SessionContent::Page
            } else {
                SessionContent::Terminal
            },
            ..SessionSeed::default()
        });
        if fields.lifecycle == Lifecycle::Ended {
            session.ended = true;
            session.launch.command = Some(context.commands.ended());
            session.launch.wait = true;
        } else if let Some(primary) = entry.panes.first() {
            session.launch.command = Some(context.commands.attach(primary));
            session.launch.wait = true;
        }
        let split = (fields.lifecycle == Lifecycle::Live)
            .then(|| entry.panes.get(1))
            .flatten();
        if let Some(pane) = split {
            add_catalog_split(&mut session, pane, fields, context);
        }
        session.remote_presentation = Some(RemotePresentationState::new(catalog_binding(
            entry,
            context.host,
            context.protocol,
        )));
        let id = session.id();
        let sessions = &mut self.workspaces[at].sessions;
        sessions.insert(index.min(sessions.len()), session);
        self.emit_session_created(id, workspace);
        if split.is_some() && fields.shown {
            self.emit_pane_visibility(EventKind::PaneSplit, id, true);
        }
        self.apply_catalog_scratch(id, fields.scratch.as_ref(), context);
        Some(id)
    }

    /// Moves row `id` to catalog session `entry`: its name, cwd and parent, its lifecycle, then its panes (upstream's
    /// `updateCatalogSession`). A live session listing no pane is between two catalog changes and keeps its layout;
    /// an ended one keeps its terminals, which hold the last screen. A pane whose server pane moved to another
    /// daemon or endpoint attaches again: a running terminal is replaced by a fresh attach.
    pub fn update_catalog_session(&mut self, id: Uuid, entry: &CatalogSession, context: &mut CatalogRowContext<'_>) {
        let fields = &entry.fields;
        let Some(session) = self.session_mut(id) else {
            return;
        };
        let name = catalog_custom_name(fields);
        let mut changed = session.custom_name != name;
        session.custom_name = name;
        let cwd = sanitized(&fields.cwd);
        if session.current_cwd.as_deref() != Some(cwd.as_str()) {
            session.current_cwd = Some(cwd);
        }
        if session.companion.parent != fields.parent {
            session.companion.parent = fields.parent;
            changed = true;
        }
        if changed {
            self.schedule_tree_changed();
        }
        self.apply_catalog_scratch(id, fields.scratch.as_ref(), context);
        if fields.lifecycle == Lifecycle::Ended {
            self.end_catalog_row(id, fields.scratch.as_ref(), context);
            return;
        }
        let previous = self.session_mut(id).and_then(|session| {
            session.ended = false;
            session.remote_presentation.as_ref().map(|state| state.binding.clone())
        });
        let Some(primary) = entry.panes.first() else {
            self.refresh_catalog_binding(id, entry, context);
            return;
        };
        self.follow_catalog_primary(id, entry, primary, context);
        self.follow_catalog_split(id, entry, context);
        let endpoint_moved = previous
            .as_ref()
            .and_then(|binding| binding.origin.as_ref())
            .is_some_and(|origin| origin.endpoint != primary.endpoint);
        for pane in &entry.panes {
            let daemon_moved = previous
                .as_ref()
                .and_then(|binding| binding.daemon(pane.identity))
                .is_some_and(|daemon| daemon != pane.daemon.as_str());
            if endpoint_moved || daemon_moved {
                self.attach_catalog_pane_again(id, pane, context);
            }
        }
        self.refresh_catalog_binding(id, entry, context);
    }

    /// An ended session's row: its terminals keep the last screen, a pane that never started holds the ended line,
    /// and an ended row with a running scratch points its presentation at the scratch's server.
    fn end_catalog_row(&mut self, id: Uuid, scratch: Option<&CatalogScratch>, context: &mut CatalogRowContext<'_>) {
        let Some(session) = self.session_mut(id) else {
            return;
        };
        if !session.ended {
            session.ended = true;
            // a pane that never started has no last screen to keep, and its daemon is gone
            if !session.surface.is_present() {
                session.launch.command = Some(context.commands.ended());
            }
            if session.split.exists && !session.split_surface.is_present() {
                session.split_launch.command = Some(context.commands.ended());
            }
            self.schedule_tree_changed();
        }
        let Some(scratch) = scratch else {
            return;
        };
        let Some(state) = self
            .session_mut(id)
            .and_then(|session| session.remote_presentation.as_mut())
        else {
            return;
        };
        let origin = RemoteOrigin {
            host: context.host.to_owned(),
            endpoint: scratch.pane.endpoint.clone(),
            session_name: uuid_string(id),
        };
        if state.binding.origin.as_ref() != Some(&origin) {
            state.binding.origin = Some(origin);
        }
    }

    /// Row `id`'s primary slot follows the catalog's first pane: the server's swap, its promotion of the split after
    /// the primary went, or a new primary pane, which attaches.
    fn follow_catalog_primary(
        &mut self,
        id: Uuid,
        entry: &CatalogSession,
        primary: &CatalogPane,
        context: &mut CatalogRowContext<'_>,
    ) {
        let Some(session) = self.session(id) else {
            return;
        };
        if session.pane_identity == primary.identity {
            return;
        }
        if session.split_pane_identity == Some(primary.identity) {
            let swapped = entry
                .panes
                .get(1)
                .is_some_and(|split| split.identity == session.pane_identity);
            if swapped {
                self.swap_panes_unrealized(id);
            } else {
                self.promote_split(id);
            }
            return;
        }
        let command = context.commands.attach(primary);
        let Some(session) = self.session_mut(id) else {
            return;
        };
        session.surface.teardown();
        session.pane_identity = primary.identity;
        session.launch.command = Some(command);
        session.launch.wait = true;
        self.schedule_tree_changed();
    }

    /// Row `id`'s split follows the catalog's second pane: a new one opens, a gone one closes, and the shown one
    /// takes the server's axis and visibility.
    fn follow_catalog_split(&mut self, id: Uuid, entry: &CatalogSession, context: &mut CatalogRowContext<'_>) {
        let fields = &entry.fields;
        let Some(session) = self.session(id) else {
            return;
        };
        let Some(split) = entry.panes.get(1) else {
            if session.split.exists {
                self.close_split(id);
            }
            return;
        };
        if session.split_pane_identity != Some(split.identity) {
            if session.split.exists {
                self.close_split(id);
            }
            let Some(session) = self.session_mut(id) else {
                return;
            };
            add_catalog_split(session, split, fields, context);
            if fields.shown {
                self.emit_pane_visibility(EventKind::PaneSplit, id, true);
            }
            self.schedule_tree_changed();
            return;
        }
        let axis = fields.axis.as_deref().and_then(SplitAxis::from_wire);
        if session.split.shown != fields.shown {
            self.set_split_visibility(id, fields.shown, axis);
        } else if let Some(axis) = axis.filter(|axis| *axis != session.split.axis)
            && let Some(session) = self.session_mut(id)
        {
            session.split.axis = axis;
            self.schedule_tree_changed();
        }
    }

    /// Pane `pane` of row `id` attaches again: a slot whose terminal started gets a fresh one, one not started yet
    /// gets the new command.
    fn attach_catalog_pane_again(&mut self, id: Uuid, pane: &CatalogPane, context: &mut CatalogRowContext<'_>) {
        let command = context.commands.attach(pane);
        let Some(session) = self.session_mut(id) else {
            return;
        };
        if session.pane_identity == pane.identity {
            session.surface.teardown();
            session.launch.command = Some(command);
        } else if session.split_pane_identity == Some(pane.identity) {
            session.split_surface.teardown();
            session.split_launch.command = Some(command);
        }
    }

    fn refresh_catalog_binding(&mut self, id: Uuid, entry: &CatalogSession, context: &CatalogRowContext<'_>) {
        let Some(state) = self
            .session_mut(id)
            .and_then(|session| session.remote_presentation.as_mut())
        else {
            return;
        };
        let version = state.binding.presentation_version.unwrap_or(context.protocol);
        let binding = catalog_binding(entry, context.host, version);
        if state.binding != binding {
            state.binding = binding;
        }
    }

    /// Keeps row `id`'s scratch on the server's (upstream's `applyCatalogScratch`): a shown one attaches its server
    /// pane, a hidden one keeps its terminal, and a replaced or gone one closes it.
    fn apply_catalog_scratch(
        &mut self,
        id: Uuid,
        scratch: Option<&CatalogScratch>,
        context: &mut CatalogRowContext<'_>,
    ) {
        let identity = scratch.map(|scratch| scratch.pane.identity);
        let Some(session) = self.session(id) else {
            return;
        };
        let current = session.catalog_scratch;
        if current.is_some_and(|current| Some(current.identity) != identity)
            && !self.close_scratch(id, true)
            && self.session(id).is_some_and(|session| session.scratch_active)
        {
            // between two attaches there is no terminal to close, and the shown scratch still hides
            self.toggle_scratch(id, true);
        }
        let next = scratch.map(|scratch| CatalogScratchMark {
            identity: scratch.pane.identity,
            shown: scratch.shown,
        });
        let Some(session) = self.session_mut(id) else {
            return;
        };
        if current.map(|current| current.identity) != identity {
            session.scratch_command = None;
        }
        if session.catalog_scratch != next {
            session.catalog_scratch = next;
            self.schedule_tree_changed();
        }
        let Some(scratch) = scratch else {
            return;
        };
        let Some(session) = self.session(id) else {
            return;
        };
        if scratch.shown && !session.scratch_surface.is_present() && session.scratch_command.is_none() {
            let command = context.commands.scratch(&scratch.pane);
            if let Some(session) = self.session_mut(id) {
                session.scratch_command = Some(command);
            }
        }
        if self
            .session(id)
            .is_some_and(|session| session.scratch_active != scratch.shown)
        {
            self.toggle_scratch(id, true);
        }
    }

    /// Removes row `id` because its server no longer has the session: no Open Recent entry, and nothing ends on the
    /// server.
    pub fn remove_catalog_row(&mut self, id: Uuid) {
        self.close_one_session(id, None);
    }
}

/// A row's manual name (upstream's `catalogCustomName`): a terminal row takes the server's `customName`, so an
/// unnamed one follows its terminal title, and a server that sends none gives its `name`; a page row keeps the label
/// in `name`, which its own page cannot always supply.
fn catalog_custom_name(fields: &CatalogSessionFields) -> Option<String> {
    let name = if fields.page.is_some() {
        &fields.name
    } else {
        fields.custom_name.as_ref().unwrap_or(&fields.name)
    };
    trimmed_or_none(&sanitized(name))
}

fn add_catalog_split(
    session: &mut Session,
    pane: &CatalogPane,
    fields: &CatalogSessionFields,
    context: &mut CatalogRowContext<'_>,
) {
    session.split.exists = true;
    session.split.shown = fields.shown;
    session.split.focused = false;
    session.split.axis = fields
        .axis
        .as_deref()
        .and_then(SplitAxis::from_wire)
        .unwrap_or_default();
    session.split_pane_identity = Some(pane.identity);
    session.split_launch.command = Some(context.commands.attach(pane));
    session.split_launch.wait = true;
}

/// Ties a row to its catalog session (upstream's `catalogBinding`): local and server identities are the same, the
/// row is the catalog's, and its origin is headless.
fn catalog_binding(entry: &CatalogSession, host: &str, protocol: i64) -> RemoteBinding {
    let mut daemons = std::collections::HashMap::new();
    for pane in &entry.panes {
        daemons.entry(pane.identity).or_insert_with(|| pane.daemon.clone());
    }
    let endpoint = entry
        .panes
        .first()
        .or(entry.fields.scratch.as_ref().map(|scratch| &scratch.pane))
        .map(|pane| &pane.endpoint)
        .or(entry.fields.page.as_ref().map(|page| &page.endpoint));
    let origin = endpoint.map(|endpoint| RemoteOrigin {
        host: host.to_owned(),
        endpoint: endpoint.clone(),
        session_name: uuid_string(entry.id()),
    });
    let mut binding = RemoteBinding::new(uuid_string(entry.id()), daemons, Some(protocol), origin);
    binding.catalog_owned = true;
    binding.origin_headless = true;
    binding
}

#[cfg(test)]
#[path = "catalog_rows_tests.rs"]
mod tests;
