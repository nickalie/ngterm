//! Sidebar drag and drop (upstream's `WorkspaceSidebar+DragDrop.swift`): what a drag hovering over the rows proposes,
//! as pure functions over the rows, their vertical spans and the window snapshot. The index arithmetic itself is the
//! host-free [`ngterm_core::sidebar_drop`], shared with `session.move --after/--before`.
//!
//! - A session drag lands on a workspace row (appended), on the upper half of a session row (before it) or on its
//!   lower half (just after it); a selected row drags the whole selection, in visible order.
//! - The flagged view drags session rows in every layout (upstream's #833). The flat and plain lists reorder the saved
//!   flagged order across workspaces; the flagged tree moves sessions in the real workspace order like the ordinary
//!   tree, its rows addressed by their full indices so a hidden unflagged sibling keeps its side, and a move that
//!   changes no visible row proposes nothing.
//! - A workspace drag ignores session rows: the slot is the count of workspace rows whose midpoint sits above the
//!   cursor, mapped onto the full workspace list through the focus filter's visible indices; the ordinary tree only.
//! - A file-manager folder drop opens a session per directory in the hovered row's workspace, else the sole focused
//!   one, else the current one; the tree only.
//!
//! Ported from the GTK build's `sidebar/drag.rs`; the payload is a plain value GPUI's drag carries.

use std::collections::HashMap;

use ngterm_core::model::{FlaggedOrderPlacement, SessionLocation};
use ngterm_core::sidebar_drop::{
    self, DropIndex, FlaggedTree, SessionDropTarget, SessionSource, resolve_flagged_reorder, resolve_sessions,
    resolve_workspace, workspace_insert_index,
};
use uuid::Uuid;

use super::rows::{RowKind, SidebarRow};

/// What a sidebar row drag carries; local to the app, so no other drop target (a terminal) takes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DragPayload {
    /// A workspace drag (one id) or a session drag (the dragged block, in visible order).
    pub workspace: bool,
    pub ids: Vec<Uuid>,
}

/// A row's vertical extent in the list's coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RowSpan {
    pub top: f64,
    pub height: f64,
}

impl RowSpan {
    fn contains(self, y: f64) -> bool {
        y >= self.top && y < self.top + self.height
    }

    fn mid(self) -> f64 {
        self.top + self.height / 2.0
    }
}

/// How a proposal is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Highlight {
    /// The row is the drop target.
    OnRow(usize),
    /// A line above the row.
    Above(usize),
    /// A line below the row.
    Below(usize),
}

/// What dropping here would do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Proposal {
    Sessions {
        sessions: Vec<Uuid>,
        workspace: Uuid,
        destination: usize,
    },
    /// A move in the flat or plain flagged list's saved order.
    Flagged {
        sessions: Vec<Uuid>,
        placement: FlaggedOrderPlacement,
        anchor: Uuid,
    },
    Workspace {
        workspace: Uuid,
        destination: usize,
    },
    Directories {
        workspace: Uuid,
    },
}

/// Which list the rows are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DropView {
    /// The ordinary workspace tree.
    #[default]
    Tree,
    /// The flagged view's tree layout: workspace rows with only their flagged sessions.
    FlaggedTree,
    /// The flagged view's flat or plain list: session rows in the saved flagged order.
    FlaggedList,
}

impl DropView {
    /// Whether the rows include workspace rows, which a drag can spring-load open.
    #[must_use]
    pub fn has_workspace_rows(self) -> bool {
        self != Self::FlaggedList
    }
}

/// What the drop math reads of the window.
#[derive(Debug, Clone, Default)]
pub struct DropContext {
    pub locations: HashMap<Uuid, SessionLocation>,
    /// Every workspace in tree order, with its session count.
    pub workspaces: Vec<(Uuid, usize)>,
    pub view: DropView,
    /// The flat and plain lists' rows: the saved flagged order.
    pub flagged_order: Vec<Uuid>,
    /// The workspaces' sessions and which are flagged, for the flagged tree's no-op check.
    pub flagged_tree: FlaggedTree,
    pub sole_focus: Option<Uuid>,
    pub current_workspace: Option<Uuid>,
}

impl DropContext {
    fn session_count(&self, workspace: Uuid) -> usize {
        self.workspaces
            .iter()
            .find(|(id, _)| *id == workspace)
            .map_or(0, |(_, count)| *count)
    }
}

/// The row under `y`, if any.
#[must_use]
pub fn row_at(spans: &[RowSpan], y: f64) -> Option<usize> {
    spans.iter().position(|span| span.contains(y))
}

/// A rows drag of `payload` hovering at `y`, in whichever list the rows are: a workspace drag only in the ordinary
/// tree, a session drag in every view.
#[must_use]
pub fn propose_rows(
    rows: &[SidebarRow],
    spans: &[RowSpan],
    y: f64,
    payload: &DragPayload,
    context: &DropContext,
) -> Option<(Proposal, Highlight)> {
    match (payload.workspace, context.view) {
        (true, DropView::Tree) => propose_workspace(rows, spans, y, *payload.ids.first()?, context),
        (true, _) => None,
        (false, DropView::FlaggedList) => propose_flagged(rows, spans, y, &payload.ids, context),
        (false, _) => propose_sessions(rows, spans, y, &payload.ids, context),
    }
}

/// A session drag of `dragged` over the flat or plain flagged list at `y` (upstream's `resolveFlaggedReorder`): the
/// upper half of a row is the gap before it, the lower half a drop on it, which lands just after it.
#[must_use]
pub fn propose_flagged(
    rows: &[SidebarRow],
    spans: &[RowSpan],
    y: f64,
    dragged: &[Uuid],
    context: &DropContext,
) -> Option<(Proposal, Highlight)> {
    let index = row_at(spans, y)?;
    let row = rows.get(index)?;
    let slot = context.flagged_order.iter().position(|id| *id == row.id)?;
    let (on_row, highlight) = if y < spans[index].mid() {
        (None, Highlight::Above(index))
    } else {
        (Some(row.id), Highlight::Below(index))
    };
    let reorder = resolve_flagged_reorder(&context.flagged_order, dragged, on_row, slot)?;
    Some((
        Proposal::Flagged {
            sessions: reorder.ids,
            placement: reorder.placement,
            anchor: reorder.anchor,
        },
        highlight,
    ))
}

/// A session drag of `dragged` hovering at `y` over the ordinary or the flagged tree: the move it would perform and how
/// to draw it; `None` over empty space or for a drop that changes nothing, in the flagged tree nothing it shows.
#[must_use]
pub fn propose_sessions(
    rows: &[SidebarRow],
    spans: &[RowSpan],
    y: f64,
    dragged: &[Uuid],
    context: &DropContext,
) -> Option<(Proposal, Highlight)> {
    let index = row_at(spans, y)?;
    let row = rows.get(index)?;
    let (target, child_index, highlight) = match row.kind {
        RowKind::Workspace { .. } => (
            SessionDropTarget::WorkspaceRow {
                id: row.id,
                session_count: context.session_count(row.id),
            },
            DropIndex::OnItem,
            Highlight::OnRow(index),
        ),
        RowKind::Session { .. } => {
            let location = context.locations.get(&row.id)?;
            let target = SessionDropTarget::SessionRow {
                workspace: location.workspace,
                session_index: location.index,
                session_count: location.count,
            };
            if y < spans[index].mid() {
                (target, DropIndex::Before(location.index), Highlight::Above(index))
            } else {
                (target, DropIndex::OnItem, Highlight::Below(index))
            }
        }
    };
    let sources: Vec<SessionSource> = dragged
        .iter()
        .filter_map(|id| context.locations.get(id))
        .map(|location| SessionSource {
            workspace: location.workspace,
            index: location.index,
        })
        .collect();
    if sources.len() != dragged.len() {
        return None;
    }
    let resolution = resolve_sessions(&sources, target, child_index)?;
    if context.view == DropView::FlaggedTree && !context.flagged_tree.changes_view(dragged, &resolution) {
        return None;
    }
    Some((
        Proposal::Sessions {
            sessions: dragged.to_vec(),
            workspace: resolution.workspace,
            destination: resolution.destination,
        },
        highlight,
    ))
}

/// A workspace drag of `dragged` hovering at `y` (upstream's `resolveWorkspaceMove`): session rows are ignored, the
/// slot counts the rendered workspace rows whose midpoint is above the cursor.
#[must_use]
pub fn propose_workspace(
    rows: &[SidebarRow],
    spans: &[RowSpan],
    y: f64,
    dragged: Uuid,
    context: &DropContext,
) -> Option<(Proposal, Highlight)> {
    let order: Vec<Uuid> = context.workspaces.iter().map(|(id, _)| *id).collect();
    let source = order.iter().position(|id| *id == dragged)?;
    // (row index, full index) of each rendered workspace row
    let rendered: Vec<(usize, usize)> = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.is_workspace())
        .filter_map(|(index, row)| Some((index, order.iter().position(|id| *id == row.id)?)))
        .collect();
    let slot = rendered
        .iter()
        .filter(|(index, _)| spans.get(*index).is_some_and(|span| y > span.mid()))
        .count();
    let visible: Vec<usize> = rendered.iter().map(|(_, full)| *full).collect();
    let insert = workspace_insert_index(&visible, slot);
    let resolution = resolve_workspace(source, order.len(), DropIndex::Before(insert))?;
    let highlight = match rendered.get(slot) {
        Some((index, _)) => Highlight::Above(*index),
        None => Highlight::Below(rows.len().checked_sub(1)?),
    };
    Some((
        Proposal::Workspace {
            workspace: dragged,
            destination: resolution.destination,
        },
        highlight,
    ))
}

/// A folder drop hovering at `y`: the workspace that would receive the sessions, highlighted when its row shows.
#[must_use]
pub fn propose_directories(
    rows: &[SidebarRow],
    spans: &[RowSpan],
    y: f64,
    context: &DropContext,
) -> Option<(Proposal, Option<Highlight>)> {
    let row_workspace = row_at(spans, y)
        .and_then(|index| rows.get(index))
        .map(|row| match row.kind {
            RowKind::Workspace { .. } => row.id,
            RowKind::Session { workspace, .. } => workspace,
        });
    let mode = if context.view == DropView::Tree {
        ngterm_core::model::SidebarMode::Tree
    } else {
        ngterm_core::model::SidebarMode::Flagged
    };
    let workspace =
        sidebar_drop::resolve_directory_workspace(mode, row_workspace, context.sole_focus, context.current_workspace)?;
    let highlight = rows
        .iter()
        .position(|row| row.is_workspace() && row.id == workspace)
        .map(Highlight::OnRow);
    Some((Proposal::Directories { workspace }, highlight))
}

/// The real directories among a folder drop's paths, at most the import cap: `Err` when there are more.
///
/// # Errors
///
/// More directories than [`sidebar_drop::MAXIMUM_DIRECTORY_IMPORT_COUNT`].
pub fn directories(paths: impl IntoIterator<Item = std::path::PathBuf>) -> Result<Vec<String>, ()> {
    let mut found = Vec::new();
    for path in paths {
        if !path.is_dir() {
            continue;
        }
        if found.len() == sidebar_drop::MAXIMUM_DIRECTORY_IMPORT_COUNT {
            return Err(());
        }
        found.push(path.to_string_lossy().into_owned());
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::row_icon::Glyph;

    fn workspace_row(id: Uuid) -> SidebarRow {
        SidebarRow {
            id,
            kind: RowKind::Workspace { expanded: true },
            label: String::new(),
            badge: 0,
            icon: Glyph::Workspace { focus_member: false },
            status: None,
            icon_tooltip: None,
            tint: None,
        }
    }

    fn session_row(id: Uuid, workspace: Uuid) -> SidebarRow {
        SidebarRow {
            id,
            kind: RowKind::Session {
                workspace,
                nested: true,
                selected: false,
            },
            label: String::new(),
            badge: 0,
            icon: Glyph::Session {
                split: None,
                flagged: false,
            },
            status: None,
            icon_tooltip: None,
            tint: None,
        }
    }

    /// Workspace one (a, b), workspace two (c), every row 20 high.
    fn tree() -> (Vec<SidebarRow>, Vec<RowSpan>, DropContext, [Uuid; 2], [Uuid; 3]) {
        let [one, two] = [Uuid::new_v4(), Uuid::new_v4()];
        let [a, b, c] = [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
        let rows = vec![
            workspace_row(one),
            session_row(a, one),
            session_row(b, one),
            workspace_row(two),
            session_row(c, two),
        ];
        let spans = (0..rows.len())
            .map(|index| RowSpan {
                top: f64::from(u8::try_from(index).unwrap()) * 20.0,
                height: 20.0,
            })
            .collect();
        let location = |workspace, index, count| SessionLocation {
            workspace,
            index,
            count,
        };
        let context = DropContext {
            locations: HashMap::from([
                (a, location(one, 0, 2)),
                (b, location(one, 1, 2)),
                (c, location(two, 0, 1)),
            ]),
            workspaces: vec![(one, 2), (two, 1)],
            view: DropView::Tree,
            flagged_order: Vec::new(),
            flagged_tree: FlaggedTree::default(),
            sole_focus: None,
            current_workspace: Some(one),
        };
        (rows, spans, context, [one, two], [a, b, c])
    }

    #[test]
    fn a_session_lands_before_or_after_a_row_or_appended_to_a_workspace() {
        let (rows, spans, context, [one, two], [a, b, c]) = tree();
        // the upper half of b's row: before b, which is where a already is
        assert_eq!(propose_sessions(&rows, &spans, 42.0, &[a], &context), None);
        // the lower half of b's row: after b
        assert_eq!(
            propose_sessions(&rows, &spans, 55.0, &[a], &context),
            Some((
                Proposal::Sessions {
                    sessions: vec![a],
                    workspace: one,
                    destination: 1
                },
                Highlight::Below(2)
            ))
        );
        assert_eq!(
            propose_sessions(&rows, &spans, 65.0, &[a, b], &context),
            Some((
                Proposal::Sessions {
                    sessions: vec![a, b],
                    workspace: two,
                    destination: 1
                },
                Highlight::OnRow(3)
            ))
        );
        assert_eq!(
            propose_sessions(&rows, &spans, 85.0, &[c], &context),
            None,
            "after itself"
        );
        assert_eq!(
            propose_sessions(&rows, &spans, 500.0, &[c], &context),
            None,
            "empty space"
        );
    }

    #[test]
    fn a_workspace_lands_by_workspace_row_midpoints() {
        let (rows, spans, context, [one, two], _) = tree();
        assert_eq!(
            propose_workspace(&rows, &spans, 5.0, two, &context),
            Some((
                Proposal::Workspace {
                    workspace: two,
                    destination: 0
                },
                Highlight::Above(0)
            ))
        );
        // over one's sessions: still after one, which is where two is
        assert_eq!(propose_workspace(&rows, &spans, 45.0, two, &context), None);
        assert_eq!(
            propose_workspace(&rows, &spans, 95.0, one, &context),
            Some((
                Proposal::Workspace {
                    workspace: one,
                    destination: 1
                },
                Highlight::Below(4)
            ))
        );
    }

    #[test]
    fn folders_land_in_the_hovered_rows_workspace_in_the_tree_only() {
        let (rows, spans, mut context, [one, two], _) = tree();
        assert_eq!(
            propose_directories(&rows, &spans, 85.0, &context),
            Some((Proposal::Directories { workspace: two }, Some(Highlight::OnRow(3))))
        );
        assert_eq!(
            propose_directories(&rows, &spans, 500.0, &context),
            Some((Proposal::Directories { workspace: one }, Some(Highlight::OnRow(0)))),
            "empty space takes the current workspace"
        );
        context.view = DropView::FlaggedTree;
        assert_eq!(propose_directories(&rows, &spans, 85.0, &context), None);
    }

    fn spans_of(count: usize) -> Vec<RowSpan> {
        (0..count)
            .map(|index| RowSpan {
                top: f64::from(u8::try_from(index).unwrap()) * 20.0,
                height: 20.0,
            })
            .collect()
    }

    fn sessions(dragged: &[Uuid]) -> DragPayload {
        DragPayload {
            workspace: false,
            ids: dragged.to_vec(),
        }
    }

    /// The flat list shows the saved order c, a, b across both workspaces of [`tree`].
    #[test]
    fn the_flagged_list_reorders_the_saved_order_and_drags_no_workspace() {
        let (_, _, mut context, [one, two], [a, b, c]) = tree();
        context.view = DropView::FlaggedList;
        context.flagged_order = vec![c, a, b];
        let rows = vec![session_row(c, two), session_row(a, one), session_row(b, one)];
        let spans = spans_of(rows.len());
        let flagged = |sessions: Vec<Uuid>, placement, anchor, highlight| {
            Some((
                Proposal::Flagged {
                    sessions,
                    placement,
                    anchor,
                },
                highlight,
            ))
        };
        // the upper half of c's row: before c
        assert_eq!(
            propose_rows(&rows, &spans, 5.0, &sessions(&[b]), &context),
            flagged(vec![b], FlaggedOrderPlacement::Before, c, Highlight::Above(0))
        );
        // the lower half of b's row: after b
        assert_eq!(
            propose_rows(&rows, &spans, 55.0, &sessions(&[a, c]), &context),
            flagged(vec![c, a], FlaggedOrderPlacement::After, b, Highlight::Below(2))
        );
        // a just below c, which is where it is
        assert_eq!(propose_rows(&rows, &spans, 15.0, &sessions(&[a]), &context), None);
        let workspace = DragPayload {
            workspace: true,
            ids: vec![one],
        };
        assert_eq!(propose_rows(&rows, &spans, 5.0, &workspace, &context), None);
    }

    /// The flagged tree of workspace one (a, the hidden unflagged h, b) and two (c).
    #[test]
    fn the_flagged_tree_moves_in_the_real_order_and_skips_moves_it_does_not_show() {
        let (_, _, mut context, [one, two], [a, b, c]) = tree();
        let h = Uuid::new_v4();
        context.view = DropView::FlaggedTree;
        let location = |workspace, index, count| SessionLocation {
            workspace,
            index,
            count,
        };
        context.locations = HashMap::from([
            (a, location(one, 0, 3)),
            (h, location(one, 1, 3)),
            (b, location(one, 2, 3)),
            (c, location(two, 0, 1)),
        ]);
        context.workspaces = vec![(one, 3), (two, 1)];
        context.flagged_tree = FlaggedTree {
            workspaces: vec![(one, vec![a, h, b]), (two, vec![c])],
            flagged: [a, b, c].into(),
        };
        let rows = vec![
            workspace_row(one),
            session_row(a, one),
            session_row(b, one),
            workspace_row(two),
            session_row(c, two),
        ];
        let spans = spans_of(rows.len());
        // a over the upper half of b: before b, past h, which shows nothing new
        assert_eq!(propose_rows(&rows, &spans, 42.0, &sessions(&[a]), &context), None);
        // b over the upper half of a: before a, h staying put
        assert_eq!(
            propose_rows(&rows, &spans, 22.0, &sessions(&[b]), &context),
            Some((
                Proposal::Sessions {
                    sessions: vec![b],
                    workspace: one,
                    destination: 0
                },
                Highlight::Above(1)
            ))
        );
        // a onto two's row: appended there
        assert_eq!(
            propose_rows(&rows, &spans, 65.0, &sessions(&[a]), &context),
            Some((
                Proposal::Sessions {
                    sessions: vec![a],
                    workspace: two,
                    destination: 1
                },
                Highlight::OnRow(3)
            ))
        );
        let workspace = DragPayload {
            workspace: true,
            ids: vec![two],
        };
        assert_eq!(propose_rows(&rows, &spans, 5.0, &workspace, &context), None);
    }

    #[test]
    fn only_directories_are_imported_up_to_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("file");
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(
            directories([dir.path().to_path_buf(), file]),
            Ok(vec![dir.path().to_string_lossy().into_owned()])
        );
        let many = (0..=sidebar_drop::MAXIMUM_DIRECTORY_IMPORT_COUNT).map(|_| dir.path().to_path_buf());
        assert_eq!(directories(many), Err(()));
    }
}
