//! The catalog's changes (upstream's `CatalogDiff.swift`): [`CatalogSnapshot::changes`] lists what turns one catalog
//! into the next, and [`CatalogSnapshot::apply`] makes one change, the same way on the server that diffed it and on a
//! viewer reading it.

use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use super::frames::{CatalogChange, CatalogSession, CatalogSnapshot, CatalogWorkspace};

/// `index` clamped into `0..=len`.
fn clamped(index: i64, len: usize) -> usize {
    usize::try_from(index.max(0)).unwrap_or(usize::MAX).min(len)
}

fn index_of(index: usize) -> i64 {
    i64::try_from(index).unwrap_or(i64::MAX)
}

impl CatalogSnapshot {
    /// Makes one change; a change naming something absent is a no-op.
    pub fn apply(&mut self, change: &CatalogChange) {
        match change {
            CatalogChange::WorkspaceAdded { workspace, index } => {
                self.workspaces.retain(|existing| existing.id != workspace.id);
                let at = clamped(*index, self.workspaces.len());
                self.workspaces.insert(at, workspace.clone());
            }
            CatalogChange::WorkspaceChanged { id, name } => {
                if let Some(workspace) = self.workspaces.iter_mut().find(|workspace| workspace.id == *id) {
                    workspace.name.clone_from(name);
                }
            }
            CatalogChange::WorkspaceRemoved(id) => self.workspaces.retain(|workspace| workspace.id != *id),
            CatalogChange::WorkspaceOrder(ids) => ordered(&mut self.workspaces, ids, |workspace| workspace.id),
            CatalogChange::SessionAdded {
                session,
                workspace,
                index,
            } => {
                self.remove_session(session.id());
                self.insert_session(session.clone(), *workspace, *index);
            }
            CatalogChange::SessionChanged(fields) => self.update_session(fields.id, |session| {
                session.fields = fields.clone();
                session.normalize();
            }),
            CatalogChange::SessionRemoved(id) => self.remove_session(*id),
            CatalogChange::SessionMoved { id, workspace, index } => {
                let Some(moving) = self.session(*id).cloned() else {
                    return;
                };
                if !self.workspaces.iter().any(|candidate| candidate.id == *workspace) {
                    return;
                }
                self.remove_session(*id);
                self.insert_session(moving, *workspace, *index);
            }
            CatalogChange::SessionOrder { ids, workspace } => {
                if let Some(workspace) = self.workspaces.iter_mut().find(|candidate| candidate.id == *workspace) {
                    ordered(&mut workspace.sessions, ids, CatalogSession::id);
                }
            }
            CatalogChange::PaneAdded { pane, session } | CatalogChange::PaneChanged { pane, session } => {
                self.update_session(*session, |session| {
                    match session
                        .panes
                        .iter_mut()
                        .find(|existing| existing.identity == pane.identity)
                    {
                        Some(existing) => *existing = pane.clone(),
                        None => session.panes.push(pane.clone()),
                    }
                    session.normalize();
                });
            }
            CatalogChange::PaneRemoved { identity, session } => {
                self.update_session(*session, |session| {
                    session.panes.retain(|pane| pane.identity != *identity);
                });
            }
        }
    }

    /// What turns `self` into `next`, each change applied to a working copy as it is listed, so applying the list in
    /// order lands exactly on `next`'s workspaces.
    #[must_use]
    pub fn changes(&self, next: &Self) -> Vec<CatalogChange> {
        let mut diff = Diff {
            state: self.clone(),
            list: Vec::new(),
        };
        let next_ids: HashSet<Uuid> = sessions(&next.workspaces).map(CatalogSession::id).collect();
        let gone: Vec<Uuid> = sessions(&diff.state.workspaces)
            .map(CatalogSession::id)
            .filter(|id| !next_ids.contains(id))
            .collect();
        for id in gone {
            diff.emit(CatalogChange::SessionRemoved(id));
        }
        diff.workspaces(next);
        diff.placements(next);
        diff.contents(next);
        let wanted: HashSet<Uuid> = next.workspaces.iter().map(|workspace| workspace.id).collect();
        let stale: Vec<Uuid> = diff
            .state
            .workspaces
            .iter()
            .map(|workspace| workspace.id)
            .filter(|id| !wanted.contains(id))
            .collect();
        for id in stale {
            diff.emit(CatalogChange::WorkspaceRemoved(id));
        }
        let order: Vec<Uuid> = next.workspaces.iter().map(|workspace| workspace.id).collect();
        if diff
            .state
            .workspaces
            .iter()
            .map(|workspace| workspace.id)
            .ne(order.iter().copied())
        {
            diff.emit(CatalogChange::WorkspaceOrder(order));
        }
        diff.list
    }

    /// The session `id`, in whichever workspace holds it.
    #[must_use]
    pub fn session(&self, id: Uuid) -> Option<&CatalogSession> {
        sessions(&self.workspaces).find(|session| session.id() == id)
    }

    fn insert_session(&mut self, session: CatalogSession, workspace: Uuid, index: i64) {
        if let Some(workspace) = self.workspaces.iter_mut().find(|candidate| candidate.id == workspace) {
            let at = clamped(index, workspace.sessions.len());
            workspace.sessions.insert(at, session);
        }
    }

    fn remove_session(&mut self, id: Uuid) {
        for workspace in &mut self.workspaces {
            workspace.sessions.retain(|session| session.id() != id);
        }
    }

    fn update_session(&mut self, id: Uuid, change: impl FnOnce(&mut CatalogSession)) {
        if let Some(session) = self
            .workspaces
            .iter_mut()
            .flat_map(|workspace| workspace.sessions.iter_mut())
            .find(|session| session.id() == id)
        {
            change(session);
        }
    }

    fn workspace_holds(&self, workspace: Uuid, session: Uuid) -> bool {
        self.workspaces
            .iter()
            .find(|candidate| candidate.id == workspace)
            .is_some_and(|workspace| workspace.sessions.iter().any(|candidate| candidate.id() == session))
    }

    fn session_order(&self, workspace: Uuid) -> Option<Vec<Uuid>> {
        self.workspaces
            .iter()
            .find(|candidate| candidate.id == workspace)
            .map(|workspace| workspace.sessions.iter().map(CatalogSession::id).collect())
    }
}

fn sessions(workspaces: &[CatalogWorkspace]) -> impl Iterator<Item = &CatalogSession> {
    workspaces.iter().flat_map(|workspace| workspace.sessions.iter())
}

/// A diff in progress: the working copy and the changes that made it.
struct Diff {
    state: CatalogSnapshot,
    list: Vec<CatalogChange>,
}

impl Diff {
    fn emit(&mut self, change: CatalogChange) {
        self.state.apply(&change);
        self.list.push(change);
    }

    /// A new workspace arrives empty and its sessions follow one by one, so a session moving into it is a move that
    /// keeps its row on the viewer, never a removal and an addition.
    fn workspaces(&mut self, next: &CatalogSnapshot) {
        for (index, workspace) in next.workspaces.iter().enumerate() {
            let current = self
                .state
                .workspaces
                .iter()
                .find(|candidate| candidate.id == workspace.id);
            match current {
                None => self.emit(CatalogChange::WorkspaceAdded {
                    workspace: CatalogWorkspace {
                        id: workspace.id,
                        name: workspace.name.clone(),
                        sessions: Vec::new(),
                    },
                    index: index_of(index),
                }),
                Some(current) if current.name != workspace.name => self.emit(CatalogChange::WorkspaceChanged {
                    id: workspace.id,
                    name: workspace.name.clone(),
                }),
                Some(_) => {}
            }
        }
    }

    /// Every session reaches its workspace before any workspace is reordered, so no order lists a row that is about to
    /// leave.
    fn placements(&mut self, next: &CatalogSnapshot) {
        for workspace in &next.workspaces {
            for (index, session) in workspace.sessions.iter().enumerate() {
                if self.state.session(session.id()).is_none() {
                    self.emit(CatalogChange::SessionAdded {
                        session: session.clone(),
                        workspace: workspace.id,
                        index: index_of(index),
                    });
                } else if !self.state.workspace_holds(workspace.id, session.id()) {
                    self.emit(CatalogChange::SessionMoved {
                        id: session.id(),
                        workspace: workspace.id,
                        index: index_of(index),
                    });
                }
            }
        }
    }

    fn contents(&mut self, next: &CatalogSnapshot) {
        for workspace in &next.workspaces {
            for session in &workspace.sessions {
                let Some(current) = self.state.session(session.id()).cloned() else {
                    continue;
                };
                for change in pane_changes(&current, session) {
                    self.emit(change);
                }
                if current.fields != session.fields {
                    self.emit(CatalogChange::SessionChanged(session.fields.clone()));
                }
            }
            let order: Vec<Uuid> = workspace.sessions.iter().map(CatalogSession::id).collect();
            if self.state.session_order(workspace.id).as_ref() != Some(&order) {
                self.emit(CatalogChange::SessionOrder {
                    ids: order,
                    workspace: workspace.id,
                });
            }
        }
    }
}

fn pane_changes(current: &CatalogSession, next: &CatalogSession) -> Vec<CatalogChange> {
    let session = next.id();
    let mut list: Vec<CatalogChange> = current
        .panes
        .iter()
        .filter(|pane| !next.panes.iter().any(|kept| kept.identity == pane.identity))
        .map(|pane| CatalogChange::PaneRemoved {
            identity: pane.identity,
            session,
        })
        .collect();
    for pane in &next.panes {
        match current.panes.iter().find(|old| old.identity == pane.identity) {
            None => list.push(CatalogChange::PaneAdded {
                pane: pane.clone(),
                session,
            }),
            Some(old) if old != pane => list.push(CatalogChange::PaneChanged {
                pane: pane.clone(),
                session,
            }),
            Some(_) => {}
        }
    }
    list
}

/// Puts the listed ids first in their listed order and keeps anything unlisted after them as it was.
fn ordered<T>(items: &mut Vec<T>, ids: &[Uuid], id: impl Fn(&T) -> Uuid) {
    let mut rank: HashMap<Uuid, usize> = HashMap::new();
    for (index, listed) in ids.iter().enumerate() {
        rank.entry(*listed).or_insert(index);
    }
    let mut ranked: Vec<(usize, T)> = std::mem::take(items)
        .into_iter()
        .enumerate()
        .map(|(offset, item)| (rank.get(&id(&item)).copied().unwrap_or(ids.len() + offset), item))
        .collect();
    ranked.sort_by_key(|(rank, _)| *rank);
    items.extend(ranked.into_iter().map(|(_, item)| item));
}

#[cfg(test)]
#[path = "diff_tests.rs"]
mod tests;
