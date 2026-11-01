//! The index arithmetic of sidebar drag and drop (upstream's `SidebarDrop.swift`): where a dragged session or
//! workspace lands, the post-removal off-by-one, a drop on a row redirected to just after it, the cross-workspace and
//! same-parent index spaces, and which drops change nothing. Host-free, so the sidebar's drop handler and the
//! control channel's `session.move --after/--before` share it; the caller resolves ids to indices and applies the
//! result through [`crate::model::AppStore::move_session`], [`crate::model::AppStore::move_sessions`] or
//! [`crate::model::AppStore::move_workspace`].
//!
//! The flagged view: a drop on the flat or plain list is a move in the saved flagged order
//! ([`resolve_flagged_reorder`], applied through [`crate::model::AppStore::move_flagged`], which shares
//! [`reordered_flagged`] with `session.flag move`); the flagged tree moves sessions in the real workspace order, a
//! move that changes no visible row counting as a no-op ([`FlaggedTree::changes_view`]).

use std::collections::HashSet;
use std::hash::BuildHasher;

use uuid::Uuid;

use crate::model::{FlaggedOrderPlacement, SessionLocation, SidebarMode};

/// Prevents an accidental file-manager multi-selection from spawning an unbounded number of shells.
pub const MAXIMUM_DIRECTORY_IMPORT_COUNT: usize = 20;

/// Where a drop lands among a parent's children: on the parent row itself (`NSOutlineViewDropOnItemIndex` upstream),
/// or into the slot before child `n` (`n` equal to the child count is the slot after the last).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DropIndex {
    OnItem,
    Before(usize),
}

/// The workspace a dropped file-manager directory opens in. Folder import is tree-only; a drop on a row lands in
/// that row's workspace, and a drop on empty space takes `fallback` when the caller can name an unambiguous one (the
/// sole focus member, the only state where the tree renders exactly one workspace), else the current workspace.
#[must_use]
pub fn resolve_directory_workspace(
    sidebar_mode: SidebarMode,
    row_workspace: Option<Uuid>,
    fallback: Option<Uuid>,
    current: Option<Uuid>,
) -> Option<Uuid> {
    if sidebar_mode != SidebarMode::Tree {
        return None;
    }
    row_workspace.or(fallback).or(current)
}

/// The move a session drop performs: the target workspace, the pre-removal slot to highlight
/// (`drop_child_index`, [`DropIndex::OnItem`] for an append), and the post-removal index the store's moves take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionResolution {
    pub workspace: Uuid,
    pub drop_child_index: DropIndex,
    pub destination: usize,
}

/// A dragged session's location before a move removes anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionSource {
    pub workspace: Uuid,
    pub index: usize,
}

/// The row a session was dropped onto. A workspace row takes the child index as is; a session row redirects to its
/// owning workspace, landing just after the row when dropped on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionDropTarget {
    WorkspaceRow {
        id: Uuid,
        session_count: usize,
    },
    SessionRow {
        workspace: Uuid,
        session_index: usize,
        session_count: usize,
    },
}

impl SessionDropTarget {
    /// The target workspace, its session count, and the drop slot with a drop on a session row redirected.
    fn landing(self, child_index: DropIndex) -> (Uuid, usize, DropIndex) {
        match self {
            Self::WorkspaceRow { id, session_count } => (id, session_count, child_index),
            Self::SessionRow {
                workspace,
                session_index,
                session_count,
            } => {
                let slot = match child_index {
                    DropIndex::OnItem => DropIndex::Before(session_index + 1),
                    before @ DropIndex::Before(_) => before,
                };
                (workspace, session_count, slot)
            }
        }
    }
}

/// Resolves a session drop into the move it performs, `None` for a no-op. A same-workspace downward move subtracts
/// one, since the slot shifts up once the session is out; cross-workspace and upward moves pass through. A
/// same-workspace move that lands the session back in its slot (appending the last one included) changes nothing.
#[must_use]
pub fn resolve_session(
    source: SessionSource,
    target: SessionDropTarget,
    child_index: DropIndex,
) -> Option<SessionResolution> {
    let (workspace, count, drop_child_index) = target.landing(child_index);
    let same_workspace = source.workspace == workspace;
    let destination = match drop_child_index {
        DropIndex::OnItem => count,
        DropIndex::Before(slot) if same_workspace && source.index < slot => slot - 1,
        DropIndex::Before(slot) => slot,
    };
    // the move removes the source first, shrinking the list to `count - 1`, then clamps
    if same_workspace && destination.min(count.saturating_sub(1)) == source.index {
        return None;
    }
    Some(SessionResolution {
        workspace,
        drop_child_index,
        destination,
    })
}

/// Resolves a multi-session drop into one remove-all, insert-block move, `None` for a no-op. `sources` are in the
/// dragged block's order; the destination is the target index once every dragged session is out. A contiguous block
/// dropped inside itself changes nothing.
#[must_use]
pub fn resolve_sessions(
    sources: &[SessionSource],
    target: SessionDropTarget,
    child_index: DropIndex,
) -> Option<SessionResolution> {
    if sources.is_empty() {
        return None;
    }
    let (workspace, count, drop_child_index) = target.landing(child_index);
    let raw_destination = match drop_child_index {
        DropIndex::OnItem => count,
        DropIndex::Before(slot) => slot,
    };
    let mut in_target: Vec<usize> = sources
        .iter()
        .filter(|source| source.workspace == workspace)
        .map(|source| source.index)
        .collect();
    in_target.sort_unstable();
    let removed_before = in_target.iter().filter(|&&index| index < raw_destination).count();
    let destination = raw_destination - removed_before;
    if let Some(&first) = in_target.first()
        && in_target.len() == sources.len()
        && in_target
            .iter()
            .enumerate()
            .all(|(offset, &index)| index == first + offset)
        && destination == first
    {
        return None;
    }
    Some(SessionResolution {
        workspace,
        drop_child_index,
        destination,
    })
}

/// Resolves an anchor-relative placement (`session.move --after/--before <sid>`) into the move a drop on the anchor
/// performs: after lands just after the anchor row (a drop on it), before takes the anchor's slot. The drop math is
/// reused as is, so the post-removal off-by-one and the anchor-is-source no-op fall out unchanged.
#[must_use]
pub fn resolve_relative(
    source: SessionSource,
    anchor: SessionLocation,
    place_after: bool,
) -> Option<SessionResolution> {
    let target = SessionDropTarget::SessionRow {
        workspace: anchor.workspace,
        session_index: anchor.index,
        session_count: anchor.count,
    };
    let child_index = if place_after {
        DropIndex::OnItem
    } else {
        DropIndex::Before(anchor.index)
    };
    resolve_session(source, target, child_index)
}

/// The move a workspace drop performs: the pre-removal slot to highlight and the post-removal index
/// [`crate::model::AppStore::move_workspace`] takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkspaceResolution {
    pub drop_child_index: usize,
    pub destination: usize,
}

/// Maps a drop slot read off the rendered workspace rows onto the full workspace list. `visible_indices` holds the
/// full-list indices of the rendered rows in order (under the focus filter, the marked set's, possibly
/// non-contiguous); `slot` counts the rendered rows above the cursor. The drop lands right next to the visible row
/// it aimed at, never across hidden workspaces: above the first rendered row it takes that row's index, otherwise
/// the index just after the last rendered row above the cursor. 0 when nothing is rendered.
#[must_use]
pub fn workspace_insert_index(visible_indices: &[usize], slot: usize) -> usize {
    let Some(&first_visible) = visible_indices.first() else {
        return 0;
    };
    if slot == 0 {
        return first_visible;
    }
    visible_indices[slot.min(visible_indices.len()) - 1] + 1
}

/// Resolves a top-level workspace reorder, `None` for a no-op; `count` is the workspace count before the move. The
/// same downward minus one as sessions.
#[must_use]
pub fn resolve_workspace(source_index: usize, count: usize, child_index: DropIndex) -> Option<WorkspaceResolution> {
    let drop_child_index = match child_index {
        DropIndex::OnItem => count,
        DropIndex::Before(slot) => slot,
    };
    let destination = if source_index < drop_child_index {
        drop_child_index - 1
    } else {
        drop_child_index
    };
    (source_index != destination).then_some(WorkspaceResolution {
        drop_child_index,
        destination,
    })
}

// MARK: - the flagged view

/// A drop on the flat or plain flagged list, resolved into the arguments of
/// [`crate::model::AppStore::move_flagged`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlaggedReorder {
    /// The dragged rows, in the list's order.
    pub ids: Vec<Uuid>,
    pub placement: FlaggedOrderPlacement,
    pub anchor: Uuid,
}

/// Resolves a drop on the flat or plain flagged list, whose rows are `order` (upstream's `resolveFlaggedReorder`). A
/// drop on a row (`on_row`) lands after it. A drop into the gap before row `slot` lands before the first undragged row
/// at or below the gap, else after the last undragged row above it. `None` when the order would not change.
#[must_use]
pub fn resolve_flagged_reorder(
    order: &[Uuid],
    dragged: &[Uuid],
    on_row: Option<Uuid>,
    slot: usize,
) -> Option<FlaggedReorder> {
    let moving: HashSet<Uuid> = dragged.iter().copied().collect();
    let block: Vec<Uuid> = order.iter().copied().filter(|id| moving.contains(id)).collect();
    if block.is_empty() {
        return None;
    }
    let (placement, anchor) = if let Some(row) = on_row {
        if moving.contains(&row) || !order.contains(&row) {
            return None;
        }
        (FlaggedOrderPlacement::After, row)
    } else {
        let slot = slot.min(order.len());
        let undragged = |id: &&Uuid| !moving.contains(*id);
        if let Some(below) = order[slot..].iter().find(undragged) {
            (FlaggedOrderPlacement::Before, *below)
        } else {
            (
                FlaggedOrderPlacement::After,
                *order[..slot].iter().rev().find(undragged)?,
            )
        }
    };
    reordered_flagged(order, &moving, placement, anchor)?;
    Some(FlaggedReorder {
        ids: block,
        placement,
        anchor,
    })
}

/// `order` with the `moving` rows, kept in their order, taken out and put just before or after `anchor`; `None` when
/// the anchor is not left in the list or nothing changes. The one reorder step a drop and `session.flag move` share,
/// so the sidebar never accepts a drop the store would refuse.
#[must_use]
pub fn reordered_flagged<S: BuildHasher>(
    order: &[Uuid],
    moving: &HashSet<Uuid, S>,
    placement: FlaggedOrderPlacement,
    anchor: Uuid,
) -> Option<Vec<Uuid>> {
    let (block, mut reordered): (Vec<Uuid>, Vec<Uuid>) = order.iter().partition(|id| moving.contains(id));
    let at = reordered.iter().position(|id| *id == anchor)?;
    let at = match placement {
        FlaggedOrderPlacement::Before => at,
        FlaggedOrderPlacement::After => at + 1,
    };
    reordered.splice(at..at, block);
    (reordered != order).then_some(reordered)
}

/// Maps a gap among a flagged-tree workspace's visible rows onto the full workspace's pre-removal child index that
/// [`resolve_sessions`] takes (upstream's `flaggedTreeChildIndex`). `visible_indices` are the flagged rows' full
/// indices, ascending. A gap above a visible row lands right before it and the gap after the last one right after it,
/// so a hidden unflagged sibling never crosses the drop; a drop on the workspace row passes through and appends after
/// the workspace's last session, flagged or not.
#[must_use]
pub fn flagged_tree_child_index(visible_indices: &[usize], slot: DropIndex) -> DropIndex {
    let DropIndex::Before(slot) = slot else {
        return DropIndex::OnItem;
    };
    let Some(&last) = visible_indices.last() else {
        return DropIndex::Before(0);
    };
    DropIndex::Before(visible_indices.get(slot).copied().unwrap_or(last + 1))
}

/// The workspace tree as the flagged tree layout sees it: every session in its workspace, and which are flagged.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlaggedTree {
    pub workspaces: Vec<(Uuid, Vec<Uuid>)>,
    pub flagged: HashSet<Uuid>,
}

impl FlaggedTree {
    /// Whether moving `dragged` as `resolution` describes changes any workspace's flagged rows or their order. A move
    /// that only shifts hidden unflagged siblings, which the full-tree no-op check of [`resolve_sessions`] cannot see,
    /// changes nothing on screen.
    #[must_use]
    pub fn changes_view(&self, dragged: &[Uuid], resolution: &SessionResolution) -> bool {
        let mut after: Vec<(Uuid, Vec<Uuid>)> = self
            .workspaces
            .iter()
            .map(|(id, sessions)| {
                let kept = sessions.iter().copied().filter(|id| !dragged.contains(id)).collect();
                (*id, kept)
            })
            .collect();
        let Some((_, target)) = after.iter_mut().find(|(id, _)| *id == resolution.workspace) else {
            return false;
        };
        let destination = resolution.destination.min(target.len());
        target.splice(destination..destination, dragged.iter().copied());
        self.flagged_rows(&after) != self.flagged_rows(&self.workspaces)
    }

    fn flagged_rows(&self, tree: &[(Uuid, Vec<Uuid>)]) -> Vec<Vec<Uuid>> {
        tree.iter()
            .map(|(_, sessions)| {
                sessions
                    .iter()
                    .copied()
                    .filter(|id| self.flagged.contains(id))
                    .collect()
            })
            .collect()
    }
}

/// Ported from upstream's `SidebarDropTests.swift`.
#[cfg(test)]
mod tests {
    use super::*;

    const WS_A: Uuid = Uuid::from_u128(0xA);
    const WS_B: Uuid = Uuid::from_u128(0xB);
    const ON_ITEM: DropIndex = DropIndex::OnItem;

    fn at(index: usize) -> DropIndex {
        DropIndex::Before(index)
    }

    fn source(workspace: Uuid, index: usize) -> SessionSource {
        SessionSource { workspace, index }
    }

    fn session_row(workspace: Uuid, session_index: usize, session_count: usize) -> SessionDropTarget {
        SessionDropTarget::SessionRow {
            workspace,
            session_index,
            session_count,
        }
    }

    fn workspace_row(id: Uuid, session_count: usize) -> SessionDropTarget {
        SessionDropTarget::WorkspaceRow { id, session_count }
    }

    fn moved(workspace: Uuid, drop_child_index: DropIndex, destination: usize) -> SessionResolution {
        SessionResolution {
            workspace,
            drop_child_index,
            destination,
        }
    }

    fn anchor(workspace: Uuid, index: usize, count: usize) -> SessionLocation {
        SessionLocation {
            workspace,
            index,
            count,
        }
    }

    #[test]
    fn a_directory_drop_takes_the_row_then_the_fallback_then_the_current_workspace() {
        let tree = SidebarMode::Tree;
        assert_eq!(
            resolve_directory_workspace(tree, Some(WS_B), Some(WS_A), Some(WS_A)),
            Some(WS_B)
        );
        assert_eq!(
            resolve_directory_workspace(tree, None, Some(WS_B), Some(WS_A)),
            Some(WS_B)
        );
        assert_eq!(resolve_directory_workspace(tree, None, None, Some(WS_A)), Some(WS_A));
        assert_eq!(
            resolve_directory_workspace(SidebarMode::Flagged, Some(WS_B), None, Some(WS_A)),
            None
        );
    }

    #[test]
    fn same_workspace_session_drops() {
        // [a, b, c]: c onto a's row lands just after a
        assert_eq!(
            resolve_session(source(WS_A, 2), session_row(WS_A, 0, 3), ON_ITEM),
            Some(moved(WS_A, at(1), 1))
        );
        // [a, b, c, d]: a down onto c's row inserts after c at 3, minus one for the removal
        assert_eq!(
            resolve_session(source(WS_A, 0), session_row(WS_A, 2, 4), ON_ITEM),
            Some(moved(WS_A, at(3), 2))
        );
        // a into the slot before d: downward, minus one
        assert_eq!(
            resolve_session(source(WS_A, 0), workspace_row(WS_A, 4), at(3)),
            Some(moved(WS_A, at(3), 2))
        );
        // [a, b, c]: c into the slot before b is upward, no adjustment
        assert_eq!(
            resolve_session(source(WS_A, 2), workspace_row(WS_A, 3), at(1)),
            Some(moved(WS_A, at(1), 1))
        );
        // b into the slot before c lands in its own slot
        assert_eq!(resolve_session(source(WS_A, 1), workspace_row(WS_A, 3), at(2)), None);
        // the last session dropped on its own header (append) stays where it is
        assert_eq!(resolve_session(source(WS_A, 2), workspace_row(WS_A, 3), ON_ITEM), None);
        // the first one appended really moves
        assert_eq!(
            resolve_session(source(WS_A, 0), workspace_row(WS_A, 3), ON_ITEM),
            Some(moved(WS_A, ON_ITEM, 3))
        );
    }

    #[test]
    fn cross_workspace_session_drops_never_adjust() {
        assert_eq!(
            resolve_session(source(WS_A, 0), workspace_row(WS_B, 3), at(2)),
            Some(moved(WS_B, at(2), 2))
        );
        assert_eq!(
            resolve_session(source(WS_A, 0), session_row(WS_B, 1, 3), ON_ITEM),
            Some(moved(WS_B, at(2), 2))
        );
        assert_eq!(
            resolve_session(source(WS_A, 0), workspace_row(WS_B, 2), ON_ITEM),
            Some(moved(WS_B, ON_ITEM, 2))
        );
    }

    #[test]
    fn batch_session_drops() {
        let pair = [source(WS_A, 0), source(WS_A, 1)];
        assert_eq!(
            resolve_sessions(&pair, workspace_row(WS_B, 3), at(2)),
            Some(moved(WS_B, at(2), 2))
        );
        // [a, b, c, d, e]: [a, b] before e; with both out, [c, d, e] takes the block at 2
        assert_eq!(
            resolve_sessions(&pair, workspace_row(WS_A, 5), at(4)),
            Some(moved(WS_A, at(4), 2))
        );
        // [a, b, c, d]: contiguous [b, c] dropped inside itself
        assert_eq!(
            resolve_sessions(&[source(WS_A, 1), source(WS_A, 2)], workspace_row(WS_A, 4), at(2)),
            None
        );
        // non-contiguous [b, d] before b compacts the selection: a real move
        assert_eq!(
            resolve_sessions(&[source(WS_A, 1), source(WS_A, 3)], workspace_row(WS_A, 5), at(1)),
            Some(moved(WS_A, at(1), 1))
        );
        // [a from A, x from B] before z of [x, y, z]: x leaves first, so 2 becomes 1
        assert_eq!(
            resolve_sessions(&[source(WS_A, 0), source(WS_B, 0)], workspace_row(WS_B, 3), at(2)),
            Some(moved(WS_B, at(2), 1))
        );
        assert_eq!(resolve_sessions(&[], workspace_row(WS_B, 3), at(2)), None);
    }

    #[test]
    fn anchor_relative_placement() {
        // [a, b, c]: c after a lands right after a
        assert_eq!(
            resolve_relative(source(WS_A, 2), anchor(WS_A, 0, 3), true),
            Some(moved(WS_A, at(1), 1))
        );
        // [a, b, c, d]: a after c is the downward off-by-one
        assert_eq!(
            resolve_relative(source(WS_A, 0), anchor(WS_A, 2, 4), true),
            Some(moved(WS_A, at(3), 2))
        );
        assert_eq!(
            resolve_relative(source(WS_A, 2), anchor(WS_A, 1, 3), false),
            Some(moved(WS_A, at(1), 1))
        );
        assert_eq!(
            resolve_relative(source(WS_A, 0), anchor(WS_B, 1, 3), true),
            Some(moved(WS_B, at(2), 2))
        );
        assert_eq!(
            resolve_relative(source(WS_A, 0), anchor(WS_B, 1, 3), false),
            Some(moved(WS_B, at(1), 1))
        );
        // a session placed before or after itself stays
        assert_eq!(resolve_relative(source(WS_A, 1), anchor(WS_A, 1, 3), false), None);
        assert_eq!(resolve_relative(source(WS_A, 1), anchor(WS_A, 1, 3), true), None);
    }

    #[test]
    fn workspace_reorders() {
        let moved = |drop_child_index, destination| {
            Some(WorkspaceResolution {
                drop_child_index,
                destination,
            })
        };
        assert_eq!(resolve_workspace(2, 3, at(1)), moved(1, 1));
        assert_eq!(resolve_workspace(0, 3, at(2)), moved(2, 1));
        assert_eq!(resolve_workspace(0, 3, ON_ITEM), moved(3, 2));
        assert_eq!(resolve_workspace(1, 3, at(2)), None);
        assert_eq!(resolve_workspace(2, 3, ON_ITEM), None);
    }

    #[test]
    fn insert_index_maps_a_filtered_slot_next_to_the_row_it_aimed_at() {
        let all = [0, 1, 2, 3];
        assert!((0..=4).all(|slot| workspace_insert_index(&all, slot) == slot));
        // {B(1), D(3)} of A B C D: above B is 1, not 0, which would jump the hidden A
        assert_eq!(workspace_insert_index(&[1, 3], 0), 1);
        assert_eq!(workspace_insert_index(&[1, 3], 1), 2);
        assert_eq!(workspace_insert_index(&[1, 3], 2), 4);
        assert_eq!(workspace_insert_index(&[0, 2], 9), 3);
        assert_eq!(workspace_insert_index(&[], 0), 0);
        assert_eq!(workspace_insert_index(&[], 3), 0);
    }

    #[test]
    fn a_filtered_workspace_drop_keeps_hidden_workspaces_in_place() {
        // D above B with {B, D} marked lands at 1, leaving the hidden A first
        let insert = workspace_insert_index(&[1, 3], 0);
        assert_eq!(
            resolve_workspace(3, 4, at(insert)),
            Some(WorkspaceResolution {
                drop_child_index: 1,
                destination: 1
            })
        );
        // B released just below its own midpoint stays
        let insert = workspace_insert_index(&[1, 3], 1);
        assert_eq!(resolve_workspace(1, 4, at(insert)), None);
    }

    // MARK: - the flagged view

    const FLAT: [Uuid; 4] = [
        Uuid::from_u128(0x10),
        Uuid::from_u128(0x11),
        Uuid::from_u128(0x12),
        Uuid::from_u128(0x13),
    ];

    fn flat(indices: &[usize]) -> Vec<Uuid> {
        indices.iter().map(|index| FLAT[*index]).collect()
    }

    fn flat_drop(dragged: &[usize], on_row: Option<usize>, slot: usize) -> Option<FlaggedReorder> {
        resolve_flagged_reorder(&FLAT, &flat(dragged), on_row.map(|row| FLAT[row]), slot)
    }

    /// The dragged rows, the row dropped on and the gap slot.
    type FlatDrop = (&'static [usize], Option<usize>, usize);
    /// The moved rows, the side and the anchor row.
    type FlatMove = (&'static [usize], FlaggedOrderPlacement, usize);

    #[test]
    fn a_flat_drop_resolves_to_a_move_of_the_block_in_list_order() {
        use FlaggedOrderPlacement::{After, Before};
        let cases: [(FlatDrop, FlatMove); 6] = [
            ((&[3], None, 0), (&[3], Before, 0)),
            ((&[0], None, 4), (&[0], After, 3)),
            ((&[0], None, 2), (&[0], Before, 2)),
            ((&[1], Some(3), 0), (&[1], After, 3)),
            ((&[2, 0], None, 2), (&[0, 2], Before, 3)),
            ((&[3, 2], None, 0), (&[2, 3], Before, 0)),
        ];
        for ((dragged, on_row, slot), (ids, placement, anchor)) in cases {
            assert_eq!(
                flat_drop(dragged, on_row, slot),
                Some(FlaggedReorder {
                    ids: flat(ids),
                    placement,
                    anchor: FLAT[anchor]
                }),
                "{dragged:?} {on_row:?} {slot}"
            );
        }
    }

    #[test]
    fn a_flat_drop_that_keeps_the_order_is_a_no_op() {
        let cases: [(&[usize], Option<usize>, usize); 7] = [
            (&[1], None, 1),
            (&[1], None, 2),
            (&[1], Some(1), 0),
            (&[1], Some(0), 0),
            (&[3], None, 4),
            (&[0, 1], None, 0),
            (&[0, 1, 2, 3], None, 2),
        ];
        for (dragged, on_row, slot) in cases {
            assert_eq!(flat_drop(dragged, on_row, slot), None, "{dragged:?} {on_row:?} {slot}");
        }
    }

    #[test]
    fn reordered_flagged_moves_the_block_in_list_order() {
        use FlaggedOrderPlacement::{After, Before};
        let cases: [(FlatMove, Option<&[usize]>); 5] = [
            ((&[3, 1], Before, 0), Some(&[1, 3, 0, 2])),
            ((&[0], After, 3), Some(&[1, 2, 3, 0])),
            ((&[1], Before, 2), None),
            ((&[1], After, 0), None),
            ((&[1], Before, 1), None),
        ];
        for ((moving, placement, anchor), want) in cases {
            let moving: HashSet<Uuid> = flat(moving).into_iter().collect();
            assert_eq!(
                reordered_flagged(&FLAT, &moving, placement, FLAT[anchor]),
                want.map(flat),
                "{moving:?} {placement:?} {anchor}"
            );
        }
        let first: HashSet<Uuid> = [FLAT[0]].into();
        assert_eq!(
            reordered_flagged(&FLAT, &first, FlaggedOrderPlacement::Before, Uuid::new_v4()),
            None,
            "an anchor outside the list"
        );
    }

    #[test]
    fn a_flat_drop_of_rows_outside_the_list_is_a_no_op() {
        assert_eq!(resolve_flagged_reorder(&FLAT, &[Uuid::new_v4()], None, 0), None);
        assert_eq!(resolve_flagged_reorder(&[], &[FLAT[0]], None, 0), None);
    }

    #[test]
    fn a_flagged_tree_gap_maps_next_to_the_visible_row_it_was_aimed_at() {
        let cases: [(&[usize], DropIndex, DropIndex); 6] = [
            (&[1, 3], at(0), at(1)),
            (&[1, 3], at(1), at(3)),
            (&[1, 3], at(2), at(4)),
            (&[0, 1], at(2), at(2)),
            (&[1, 3], ON_ITEM, ON_ITEM),
            (&[], at(0), at(0)),
        ];
        for (visible, slot, want) in cases {
            assert_eq!(flagged_tree_child_index(visible, slot), want, "{visible:?} {slot:?}");
        }
    }

    /// Workspace A holds a, the hidden unflagged h, then b; workspace B holds c, then the hidden k.
    struct FlaggedTreeFixture {
        tree: FlaggedTree,
        a: Uuid,
        b: Uuid,
    }

    #[derive(Clone, Copy)]
    enum TreeTarget {
        Gap {
            workspace: Uuid,
            visible: &'static [usize],
            count: usize,
            slot: usize,
        },
        OnWorkspace(Uuid, usize),
        OnSession {
            workspace: Uuid,
            index: usize,
            count: usize,
        },
    }

    fn flagged_tree() -> FlaggedTreeFixture {
        let [a, hidden_a, b, c, hidden_c] = [0x20, 0x21, 0x22, 0x23, 0x24].map(Uuid::from_u128);
        FlaggedTreeFixture {
            tree: FlaggedTree {
                workspaces: vec![(WS_A, vec![a, hidden_a, b]), (WS_B, vec![c, hidden_c])],
                flagged: [a, b, c].into(),
            },
            a,
            b,
        }
    }

    fn tree_drop(
        fixture: &FlaggedTreeFixture,
        dragged: Uuid,
        from: (Uuid, usize),
        target: TreeTarget,
    ) -> Option<SessionResolution> {
        let sources = [source(from.0, from.1)];
        let resolution = match target {
            TreeTarget::Gap {
                workspace,
                visible,
                count,
                slot,
            } => resolve_sessions(
                &sources,
                workspace_row(workspace, count),
                flagged_tree_child_index(visible, at(slot)),
            ),
            TreeTarget::OnWorkspace(workspace, count) => resolve_sessions(
                &sources,
                workspace_row(workspace, count),
                flagged_tree_child_index(&[], ON_ITEM),
            ),
            TreeTarget::OnSession {
                workspace,
                index,
                count,
            } => resolve_sessions(&sources, session_row(workspace, index, count), ON_ITEM),
        }?;
        fixture.tree.changes_view(&[dragged], &resolution).then_some(resolution)
    }

    fn resolution(workspace: Uuid, drop_child_index: DropIndex, destination: usize) -> SessionResolution {
        SessionResolution {
            workspace,
            drop_child_index,
            destination,
        }
    }

    #[test]
    fn a_flagged_tree_drop_back_above_the_next_flagged_row_across_a_hidden_sibling_is_a_no_op() {
        let f = flagged_tree();
        let gap = TreeTarget::Gap {
            workspace: WS_A,
            visible: &[0, 2],
            count: 3,
            slot: 1,
        };
        assert_eq!(tree_drop(&f, f.a, (WS_A, 0), gap), None);
    }

    #[test]
    fn a_flagged_tree_reorder_inside_a_workspace_keeps_the_hidden_sibling_in_place() {
        let f = flagged_tree();
        let gap = |slot| TreeTarget::Gap {
            workspace: WS_A,
            visible: &[0, 2],
            count: 3,
            slot,
        };
        assert_eq!(tree_drop(&f, f.b, (WS_A, 2), gap(0)), Some(resolution(WS_A, at(0), 0)));
        assert_eq!(tree_drop(&f, f.a, (WS_A, 0), gap(2)), Some(resolution(WS_A, at(3), 2)));
    }

    #[test]
    fn a_flagged_tree_move_to_another_workspace_counts_even_when_the_flat_order_stays_the_same() {
        let f = flagged_tree();
        let gap = TreeTarget::Gap {
            workspace: WS_B,
            visible: &[0],
            count: 2,
            slot: 0,
        };
        assert_eq!(tree_drop(&f, f.b, (WS_A, 2), gap), Some(resolution(WS_B, at(0), 0)));
    }

    #[test]
    fn a_flagged_tree_end_of_group_gap_lands_right_after_the_last_flagged_row() {
        let f = flagged_tree();
        let gap = TreeTarget::Gap {
            workspace: WS_B,
            visible: &[0],
            count: 2,
            slot: 1,
        };
        assert_eq!(tree_drop(&f, f.a, (WS_A, 0), gap), Some(resolution(WS_B, at(1), 1)));
    }

    #[test]
    fn a_flagged_tree_drop_on_a_workspace_row_appends_after_its_last_session() {
        let f = flagged_tree();
        assert_eq!(
            tree_drop(&f, f.a, (WS_A, 0), TreeTarget::OnWorkspace(WS_B, 2)),
            Some(resolution(WS_B, ON_ITEM, 2))
        );
    }

    #[test]
    fn a_flagged_tree_drop_on_a_session_row_lands_right_after_it() {
        let f = flagged_tree();
        let on = TreeTarget::OnSession {
            workspace: WS_B,
            index: 0,
            count: 2,
        };
        assert_eq!(tree_drop(&f, f.a, (WS_A, 0), on), Some(resolution(WS_B, at(1), 1)));
    }

    #[test]
    fn a_flagged_tree_drop_on_its_own_row_is_a_no_op() {
        let f = flagged_tree();
        let on = TreeTarget::OnSession {
            workspace: WS_A,
            index: 2,
            count: 3,
        };
        assert_eq!(tree_drop(&f, f.b, (WS_A, 2), on), None);
    }
}
