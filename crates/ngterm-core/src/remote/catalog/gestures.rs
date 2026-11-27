//! A remote window's own controls (upstream's `AppActions+RemoteCatalog.swift` and the remote branches of
//! `WorkspaceSidebar+ContextMenu.swift` and `WorkspaceSidebar+DragDrop.swift`): a sidebar gesture that makes, changes
//! or ends a row becomes the operation the server makes; one that would make a shell or a row on this machine does
//! nothing.

use uuid::Uuid;

use super::operation::{RemoteOperation, RemoteSessionPlacement, RemoteShellOptions};
use crate::Core;
use crate::model::AppStore;
use crate::persist::settings::NewSessionPlacementSetting;
use crate::resolve::trimmed;
use crate::sidebar_actions::SidebarAction;

impl Core {
    /// Performs `action`, one that changes rows ([`SidebarAction::changes_rows`]), in remote window `window`; false
    /// when it sent nothing.
    pub(crate) fn remote_sidebar_action(&mut self, window: Uuid, action: SidebarAction) -> bool {
        let Some(store) = self.library.store(window) else {
            return false;
        };
        let operations = match action {
            SidebarAction::RenameSession { session, name } => vec![RemoteOperation::RenameSession { session, name }],
            SidebarAction::RenameWorkspace { workspace, name } => match trimmed(Some(&name)) {
                Some(name) => vec![RemoteOperation::RenameWorkspace {
                    workspace,
                    name: name.to_owned(),
                }],
                None => Vec::new(),
            },
            SidebarAction::Duplicate(id) => {
                return duplicate(store, id).is_some_and(|operation| self.perform_remote(window, operation, true));
            }
            SidebarAction::NewCompanion(parent) => {
                let operation = RemoteOperation::NewCompanion {
                    parent,
                    shell: RemoteShellOptions::default(),
                };
                return self.perform_remote(window, operation, true);
            }
            SidebarAction::NewSession { workspace } => return self.new_remote_session(window, workspace),
            SidebarAction::NewSessionInCurrent => {
                return store
                    .current_workspace_id()
                    .is_some_and(|workspace| self.new_remote_session(window, workspace));
            }
            SidebarAction::NewWorkspace => vec![RemoteOperation::NewWorkspace { name: None }],
            SidebarAction::DeleteWorkspace(workspace) => vec![RemoteOperation::DeleteWorkspace(workspace)],
            SidebarAction::MoveSessions { sessions, workspace } => sessions
                .into_iter()
                .map(|session| RemoteOperation::MoveSession {
                    session,
                    to: RemoteSessionPlacement::Workspace(workspace),
                })
                .collect(),
            SidebarAction::CloseSessions(ids) => close_roots(store, &ids)
                .into_iter()
                .map(RemoteOperation::CloseSession)
                .collect(),
            SidebarAction::DropSessions {
                sessions,
                workspace,
                destination,
            } => drop_sessions(store, &sessions, workspace, destination)
                .into_iter()
                .collect(),
            SidebarAction::DropWorkspace { workspace, destination } => {
                drop_workspace(store, workspace, destination).into_iter().collect()
            }
            // a shell or a row on this machine, which a remote window shows only from its server
            _ => Vec::new(),
        };
        let mut sent = false;
        for operation in operations {
            sent |= self.perform_remote(window, operation, false);
        }
        sent
    }

    /// Whether window `window` can make, change or end rows now: a local window always, a remote one while its
    /// connection is up, as nothing is queued for its server.
    #[must_use]
    pub fn structural_actions_enabled(&self, window: Uuid) -> bool {
        !self.library.is_remote_window(window)
            || self
                .remote
                .viewer
                .catalogs
                .get(&window)
                .is_some_and(|run| *run.stream.state() == super::CatalogState::Connected)
    }

    /// Swap Panes (`swap`) or Close Split from the menu or the palette on remote window `window`'s row `session`
    /// (upstream's `swapActiveSessionPanes` and `closeSplit`): a shown scratch is hidden first, so the panes come
    /// into view before Close Split ends one.
    pub(crate) fn remote_pane_command(&mut self, window: Uuid, session: Uuid, swap: bool) {
        let Some(row) = self.library.store(window).and_then(|store| store.session(session)) else {
            return;
        };
        let operation = if swap {
            RemoteOperation::Swap(session)
        } else if row.scratch_active {
            RemoteOperation::SetScratch {
                session,
                shown: false,
                command: None,
            }
        } else if row.split.exists && !row.full_overlay_visible() {
            RemoteOperation::CloseSplit(session)
        } else {
            return;
        };
        self.perform_remote(window, operation, false);
    }

    /// New Session in `workspace` of remote window `window`, with the directory and placement settings (upstream's
    /// `newRemoteSession`); the created row is selected once it arrives.
    pub(crate) fn new_remote_session(&mut self, window: Uuid, workspace: Uuid) -> bool {
        let Some(store) = self.library.store(window) else {
            return false;
        };
        let source = store
            .new_session_directory_source(workspace)
            .map(|session| session.focused_cwd().to_owned());
        let cwd = self
            .settings()
            .remote_new_session_cwd(source.as_deref())
            .map(str::to_owned);
        let after_current =
            self.settings().effective_new_session_placement() == NewSessionPlacementSetting::AfterCurrent;
        let anchor = store
            .selected_session_id()
            .filter(|id| after_current && store.session_location(*id).is_some_and(|at| at.workspace == workspace))
            .map(RemoteSessionPlacement::After);
        let operation = RemoteOperation::NewSession {
            workspace,
            anchor,
            shell: RemoteShellOptions {
                cwd,
                ..RemoteShellOptions::default()
            },
        };
        self.perform_remote(window, operation, true)
    }
}

/// Duplicate Session in a remote window: a fresh shell on the server after the row, in its directory.
fn duplicate(store: &AppStore, id: Uuid) -> Option<RemoteOperation> {
    let workspace = store.session_location(id)?.workspace;
    let cwd = store.session(id).map(|session| session.focused_cwd().to_owned());
    Some(RemoteOperation::NewSession {
        workspace,
        anchor: Some(RemoteSessionPlacement::After(id)),
        shell: RemoteShellOptions {
            cwd,
            ..RemoteShellOptions::default()
        },
    })
}

/// The rows a close ends on the server: a companion that closes with its parent goes with the parent's close.
fn close_roots(store: &AppStore, ids: &[Uuid]) -> Vec<Uuid> {
    ids.iter()
        .copied()
        .filter(|id| {
            store
                .session(*id)
                .and_then(crate::model::Session::parent_id)
                .is_none_or(|parent| !ids.contains(&parent))
        })
        .collect()
}

/// A drop of one row as the anchor the server places it by; a remote window moves one row at a time.
fn drop_sessions(store: &AppStore, sessions: &[Uuid], workspace: Uuid, destination: usize) -> Option<RemoteOperation> {
    let [session] = sessions else {
        return None;
    };
    let rows: Vec<Uuid> = store
        .workspace(workspace)?
        .sessions
        .iter()
        .map(crate::model::Session::id)
        .collect();
    let to = RemoteOperation::drop_placement(*session, workspace, &rows, destination);
    Some(RemoteOperation::MoveSession { session: *session, to })
}

/// A workspace drop as the one move step that lands it there; none when no single step does.
fn drop_workspace(store: &AppStore, workspace: Uuid, destination: usize) -> Option<RemoteOperation> {
    let workspaces = store.workspaces();
    let source = workspaces.iter().position(|candidate| candidate.id() == workspace)?;
    let step = RemoteOperation::workspace_step(source, destination, workspaces.len())?;
    Some(RemoteOperation::MoveWorkspace {
        workspace,
        to: step.to_owned(),
    })
}
