//! Moving and reordering sessions (upstream's `AppStore+SessionMoves.swift`). A move keeps the session itself, and
//! so its live terminals. A parent carries its companions as one block, a top-level session lands outside any group,
//! and a companion moves only among its siblings: any other destination is refused with nothing changed.

use std::collections::HashSet;

use uuid::Uuid;

use super::companions::group_end;
use super::session::Session;
use super::store::{AppStore, Location};
use super::workspace::ReorderDirection;

impl AppStore {
    /// Moves a session to workspace `target`, or within its own. A parent carries its companions; a companion moves
    /// only among its siblings, and any other destination returns false with nothing changed. `index` counts the
    /// target with the moved session alone removed, the slot a caller computes for one row, and a parent's block
    /// lands where that single row would; `None` appends. Moving the selected session out of the marked set
    /// suspends the focus filter, keeping the set.
    pub fn move_session(&mut self, id: Uuid, target: Uuid, index: Option<usize>) -> bool {
        let Some(source) = self.location(id) else {
            return false;
        };
        let Some(target_index) = self.workspace_index(target) else {
            return false;
        };
        if self.workspaces[source.workspace].sessions[source.session]
            .companion
            .parent
            .is_some()
        {
            return target_index == source.workspace && self.move_companions(&[id], index);
        }
        let block = self.group_ids(source);
        let anchor = self.insertion_anchor(&HashSet::from([id]), &block, target_index, index);
        self.relocate(&block, target_index, anchor);
        true
    }

    /// Moves the selection in tree order and returns how many sessions moved. A selected parent carries its
    /// companions; siblings selected without their parent reorder among themselves, and any other companion in a
    /// batch stays put. With `index` `None`, groups already in the target stay; an explicit `index` counts the target
    /// with every selected session removed. A single session moves as [`AppStore::move_session`] does.
    pub fn move_sessions(&mut self, ids: &[Uuid], target: Uuid, index: Option<usize>) -> usize {
        let Some(target_index) = self.workspace_index(target) else {
            return 0;
        };
        let selected = self.ordered_session_ids(ids);
        if let [id] = selected.as_slice() {
            let id = *id;
            return if self.move_session(id, target, index) {
                1 + self.companions(id).len()
            } else {
                0
            };
        }
        let mut parents = self.top_level_ids(&selected);
        if parents.is_empty() && self.owner_of(selected.first().copied()) == Some(target) {
            return if self.move_companions(&selected, index) {
                selected.len()
            } else {
                0
            };
        }
        if index.is_none() {
            parents.retain(|id| self.owner_of(Some(*id)) != Some(target));
        }
        if parents.is_empty() {
            return 0;
        }
        let block: Vec<Uuid> = parents
            .iter()
            .filter_map(|id| self.location(*id))
            .flat_map(|at| self.group_ids(at))
            .collect();
        let moving: HashSet<Uuid> = selected.into_iter().collect();
        let anchor = self.insertion_anchor(&moving, &block, target_index, index);
        self.relocate(&block, target_index, anchor);
        block.len()
    }

    /// Steps a session `up`/`down`/`top`/`bottom`: a companion among its siblings, any other session among its
    /// workspace's top-level groups, stepping over a whole group. No write at an end.
    pub fn reorder_session(&mut self, id: Uuid, direction: ReorderDirection) {
        let Some(at) = self.location(id) else { return };
        let workspace = &self.workspaces[at.workspace];
        let workspace_id = workspace.id();
        let sessions = &workspace.sessions;
        if let Some(parent_id) = sessions[at.session].companion.parent
            && let Some(parent) = sessions.iter().position(|session| session.id() == parent_id)
        {
            let siblings = group_end(parent, sessions) - parent - 1;
            if let Some(destination) = direction.destination_index(at.session - parent - 1, siblings) {
                self.move_session(id, workspace_id, Some(parent + 1 + destination));
            }
            return;
        }
        let units: Vec<usize> = (0..sessions.len())
            .filter(|index| sessions[*index].companion.parent.is_none())
            .collect();
        let Some(unit) = units.iter().position(|index| *index == at.session) else {
            return;
        };
        let Some(destination) = direction.destination_index(unit, units.len()) else {
            return;
        };
        let remaining: Vec<usize> = units.into_iter().filter(|index| *index != at.session).collect();
        let flat = match remaining.get(destination) {
            Some(&index) if index > at.session => index - 1,
            Some(&index) => index,
            None => sessions.len() - 1,
        };
        self.move_session(id, workspace_id, Some(flat));
    }

    /// Whether [`AppStore::move_sessions`] accepts this destination, without moving: the sidebar asks it so a drop
    /// indicator or a Move to entry never offers a destination the move refuses.
    #[must_use]
    pub fn can_move_sessions(&self, ids: &[Uuid], target: Uuid, index: Option<usize>) -> bool {
        if self.workspace(target).is_none() {
            return false;
        }
        let selected = self.ordered_session_ids(ids);
        if let [id] = selected.as_slice()
            && self
                .session(*id)
                .is_some_and(|session| session.companion.parent.is_none())
        {
            return true;
        }
        let mut parents = self.top_level_ids(&selected);
        if parents.is_empty() {
            return self.owner_of(selected.first().copied()) == Some(target)
                && self.companion_destination(&selected, index).is_some();
        }
        if index.is_none() {
            parents.retain(|id| self.owner_of(Some(*id)) != Some(target));
        }
        !parents.is_empty()
    }

    /// The sessions of `ids` in tree order, unknown ones dropped.
    fn ordered_session_ids(&self, ids: &[Uuid]) -> Vec<Uuid> {
        let wanted: HashSet<Uuid> = ids.iter().copied().collect();
        self.sessions()
            .map(Session::id)
            .filter(|id| wanted.contains(id))
            .collect()
    }

    /// The top-level sessions of `ids`, in their order.
    fn top_level_ids(&self, ids: &[Uuid]) -> Vec<Uuid> {
        ids.iter()
            .copied()
            .filter(|id| {
                self.session(*id)
                    .is_some_and(|session| session.companion.parent.is_none())
            })
            .collect()
    }

    fn owner_of(&self, session: Option<Uuid>) -> Option<Uuid> {
        session
            .and_then(|id| self.workspace_for_session(id))
            .map(super::workspace::Workspace::id)
    }

    /// The ids of the group starting at `at`: the session and, for a parent, its companions.
    fn group_ids(&self, at: Location) -> Vec<Uuid> {
        let sessions = &self.workspaces[at.workspace].sessions;
        sessions[at.session..group_end(at.session, sessions)]
            .iter()
            .map(Session::id)
            .collect()
    }

    /// The first top-level session of the target, from `index` on once `moving` is out, that is not in the moving
    /// block: the block goes before it. `None` appends.
    fn insertion_anchor(
        &self,
        moving: &HashSet<Uuid>,
        block: &[Uuid],
        target_index: usize,
        index: Option<usize>,
    ) -> Option<Uuid> {
        let index = index?;
        let remaining: Vec<&Session> = self.workspaces[target_index]
            .sessions
            .iter()
            .filter(|session| !moving.contains(&session.id()))
            .collect();
        let start = index.min(remaining.len());
        remaining[start..]
            .iter()
            .find(|session| session.companion.parent.is_none() && !block.contains(&session.id()))
            .map(|session| session.id())
    }

    /// Takes `block` out of the tree and inserts it before `anchor` in the workspace at `target_index`, or at its end.
    fn relocate(&mut self, block: &[Uuid], target_index: usize, anchor: Option<Uuid>) {
        let before = self.tree_order();
        let mut moving = Vec::with_capacity(block.len());
        for id in block {
            if let Some(at) = self.location(*id) {
                moving.push(self.workspaces[at.workspace].sessions.remove(at.session));
            }
        }
        let sessions = &mut self.workspaces[target_index].sessions;
        let destination = anchor
            .and_then(|anchor| sessions.iter().position(|session| session.id() == anchor))
            .unwrap_or(sessions.len());
        sessions.splice(destination..destination, moving);
        if let Some(selected) = self.selected_session_id
            && block.contains(&selected)
        {
            self.disable_focus_if_selection_outside_set(Some(selected));
        }
        self.prune_sidebar_selection();
        if self.tree_order() != before {
            self.schedule_tree_changed();
        }
        self.save();
    }

    /// Reorders siblings within their group. `index` counts the workspace with the moving siblings removed and must
    /// fall inside the group, from right after the parent to its end; `None` appends to the group. False, with
    /// nothing changed, otherwise.
    fn move_companions(&mut self, ids: &[Uuid], index: Option<usize>) -> bool {
        let Some((workspace, destination)) = self.companion_destination(ids, index) else {
            return false;
        };
        let moving: HashSet<Uuid> = ids.iter().copied().collect();
        let before = self.tree_order();
        let sessions = std::mem::take(&mut self.workspaces[workspace].sessions);
        let (block, mut remaining): (Vec<Session>, Vec<Session>) =
            sessions.into_iter().partition(|session| moving.contains(&session.id()));
        remaining.splice(destination..destination, block);
        self.workspaces[workspace].sessions = remaining;
        self.prune_sidebar_selection();
        if self.tree_order() != before {
            self.schedule_tree_changed();
        }
        self.save();
        true
    }

    /// The workspace index of a sibling move and its destination among the sessions left once the siblings are out;
    /// `None` unless every id is a companion of one parent and the destination falls inside that parent's group.
    fn companion_destination(&self, ids: &[Uuid], index: Option<usize>) -> Option<(usize, usize)> {
        let first = self.location(*ids.first()?)?;
        let sessions = &self.workspaces[first.workspace].sessions;
        let parent_id = sessions[first.session].companion.parent?;
        let siblings = ids.iter().all(|id| {
            sessions
                .iter()
                .any(|session| session.id() == *id && session.companion.parent == Some(parent_id))
        });
        if !siblings {
            return None;
        }
        let remaining: Vec<&Session> = sessions.iter().filter(|session| !ids.contains(&session.id())).collect();
        let parent = remaining.iter().position(|session| session.id() == parent_id)?;
        let mut end = parent + 1;
        while end < remaining.len() && remaining[end].companion.parent.is_some() {
            end += 1;
        }
        let destination = index.unwrap_or(end);
        (destination > parent && destination <= end).then_some((first.workspace, destination))
    }

    fn tree_order(&self) -> Vec<Vec<Uuid>> {
        self.workspaces
            .iter()
            .map(|workspace| workspace.sessions.iter().map(Session::id).collect())
            .collect()
    }
}
