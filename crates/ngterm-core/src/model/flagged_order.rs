//! The flagged view's saved order (upstream's `AppStore+FlaggedOrder.swift`): each window keeps the order its flat and
//! plain flagged layouts show, apart from the workspace order. [`Session::flagged`] stays the membership; the order
//! only arranges it, and the flagged tree layout ignores it.
//!
//! - A session flagged, reopened from Open Recent or brought back by an undo after its slot was cleared is appended,
//!   recorded as it arrives rather than derived from the tree later, so a session placed earlier in the tree by a
//!   later arrival cannot jump ahead of one that arrived before it.
//! - An unflag, a hard close, a workspace removal or a pending close made final drops the id; `session.flag clear`
//!   empties the order.
//! - An id parked in a pending close keeps its slot, so the undo returns it in place.
//!
//! The app-wide flagged layout is mirrored into every store ([`AppStore::flagged_layout`]), so navigation and close
//! reselection follow the order the view shows; [`super::WindowLibrary`] seeds it before a restore picks a selection.

use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use super::session::Session;
use super::store::AppStore;
use super::workspace::SidebarMode;
use crate::pane_env::uuid_string;
use crate::persist::settings::FlaggedViewLayout;
use crate::sidebar_drop::reordered_flagged;

/// Which side of the anchor a flagged-order move puts the moved block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlaggedOrderPlacement {
    Before,
    After,
}

impl AppStore {
    /// The saved flagged order, parked pending-close ids included.
    #[must_use]
    pub fn flagged_order(&self) -> &[Uuid] {
        &self.flagged_order
    }

    /// The app-wide flagged layout as this store mirrors it.
    #[must_use]
    pub fn flagged_layout(&self) -> FlaggedViewLayout {
        self.flagged_layout
    }

    /// Mirrors the app-wide flagged layout; not persisted.
    pub(crate) fn set_flagged_layout(&mut self, layout: FlaggedViewLayout) {
        self.flagged_layout = layout;
    }

    /// The flat and plain layouts' rows: the saved ids still flagged and in the tree, then any flagged session the
    /// order lacks, in tree order.
    #[must_use]
    pub fn ordered_flagged_sessions(&self) -> Vec<&Session> {
        let flagged = self.flagged_sessions();
        let mut unplaced: HashMap<Uuid, &Session> = flagged.iter().map(|session| (session.id(), *session)).collect();
        let mut ordered: Vec<&Session> = self.flagged_order.iter().filter_map(|id| unplaced.remove(id)).collect();
        ordered.extend(
            flagged
                .into_iter()
                .filter(|session| unplaced.contains_key(&session.id())),
        );
        ordered
    }

    /// The ids of [`Self::ordered_flagged_sessions`].
    #[must_use]
    pub fn ordered_flagged_session_ids(&self) -> Vec<Uuid> {
        self.ordered_flagged_sessions().into_iter().map(Session::id).collect()
    }

    /// Puts `ids`, kept in their current order, just before or after `anchor` (upstream's `moveFlagged`). Every id and
    /// the anchor must be flagged sessions of this store and the anchor not among the ids; an invalid or unchanged move
    /// returns false. Ids parked in a pending close keep their slots. Saves and signals a tree change only on a real
    /// move.
    pub fn move_flagged(&mut self, ids: &[Uuid], placement: FlaggedOrderPlacement, anchor: Uuid) -> bool {
        let live = self.ordered_flagged_session_ids();
        let moving: HashSet<Uuid> = ids.iter().copied().collect();
        if moving.is_empty() || moving.contains(&anchor) || !moving.iter().all(|id| live.contains(id)) {
            return false;
        }
        let Some(reordered) = reordered_flagged(&live, &moving, placement, anchor) else {
            return false;
        };
        self.flagged_order = self.writing_live_order(reordered);
        self.schedule_tree_changed();
        self.save();
        true
    }

    /// The tree's `flaggedOrder`: the rows of the flat and plain layouts as ids, `None` when nothing is flagged.
    #[must_use]
    pub fn flagged_order_readback(&self) -> Option<Vec<String>> {
        let ids: Vec<String> = self
            .ordered_flagged_sessions()
            .into_iter()
            .map(|session| uuid_string(session.id()))
            .collect();
        (!ids.is_empty()).then_some(ids)
    }

    /// Appends the flagged sessions the order lacks, in tree order.
    pub(crate) fn append_flagged_arrivals(&mut self) {
        let known: HashSet<Uuid> = self.flagged_order.iter().copied().collect();
        let arrivals: Vec<Uuid> = self
            .flagged_sessions()
            .into_iter()
            .map(Session::id)
            .filter(|id| !known.contains(id))
            .collect();
        self.flagged_order.extend(arrivals);
    }

    /// Drops `ids` from the order.
    pub(crate) fn drop_from_flagged_order(&mut self, ids: &[Uuid]) {
        self.flagged_order.retain(|id| !ids.contains(id));
    }

    /// Empties the order, parked ids too, so an undo after `session.flag clear` appends.
    pub(crate) fn clear_flagged_order(&mut self) {
        self.flagged_order.clear();
    }

    /// Rebuilds the order from a snapshot's: the flagged ids it names, once each, then the flagged sessions it lacks.
    /// A snapshot written before the order existed carries none and restores in tree order.
    pub(crate) fn restore_flagged_order(&mut self, saved: Option<&[Uuid]>) {
        let flagged: HashSet<Uuid> = self.flagged_sessions().into_iter().map(Session::id).collect();
        let mut seen: HashSet<Uuid> = HashSet::new();
        self.flagged_order = saved
            .unwrap_or_default()
            .iter()
            .copied()
            .filter(|id| flagged.contains(id) && seen.insert(*id))
            .collect();
        self.append_flagged_arrivals();
    }

    /// The flat and plain lists' positional neighbour of `removed` in the saved order, within `scope`: the next one,
    /// else the previous one. The removed id is still in the order here, since the removal paths drop it only after
    /// reselecting. `None` outside the flagged view's flat and plain layouts.
    pub(crate) fn nearest_in_saved_flagged_order(&self, removed: Uuid, scope: &HashSet<Uuid>) -> Option<Uuid> {
        if self.sidebar.mode != SidebarMode::Flagged || self.flagged_layout == FlaggedViewLayout::Tree {
            return None;
        }
        let slot = self.flagged_order.iter().position(|id| *id == removed)?;
        self.flagged_order[slot + 1..]
            .iter()
            .find(|id| scope.contains(id))
            .or_else(|| self.flagged_order[..slot].iter().rev().find(|id| scope.contains(id)))
            .copied()
    }

    /// `live_order` written over the saved order: each live slot takes the next live id, a parked id keeps its slot.
    fn writing_live_order(&self, live_order: Vec<Uuid>) -> Vec<Uuid> {
        let live: HashSet<Uuid> = live_order.iter().copied().collect();
        let mut remaining = live_order.into_iter();
        let mut written: Vec<Uuid> = self
            .flagged_order
            .iter()
            .map(|id| {
                if live.contains(id) {
                    remaining.next().unwrap_or(*id)
                } else {
                    *id
                }
            })
            .collect();
        written.extend(remaining);
        written
    }
}
