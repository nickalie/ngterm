//! The sidebar's rows as a pure function of the model (upstream's coordinator `workspaceProjection`,
//! `rebuildAndReload`, the cell builder in `WorkspaceSidebar+RowRendering.swift` and `WorkspaceSidebar+FlaggedLayout.swift`),
//! and the set of workspace rows the outline shows open ([`Expansion`]). Testable without a display. Ported unchanged
//! from the GTK build's `sidebar/rows.rs`.
//!
//! Three shapes: the ordinary tree (the workspaces the focus filter shows, each with its sessions while open), the
//! flagged tree (every workspace holding a flagged session, with only those sessions), and the flat flagged list
//! (the flagged sessions alone, labelled `session : workspace` under the `flat` layout, by name under `plain`).
//!
//! Companions (upstream's `WorkspaceSidebar+Companions.swift`) nest one level under their parent row in the ordinary
//! tree only, the parent row folding them away with its own disclosure; a folded parent names the companions that need
//! attention for its marker. The flagged layouts list a companion unindented, labelled `parent › child`
//! (`AppStore::list_label`), and the tree keeps the bare name.

use std::collections::{HashMap, HashSet};

use ngterm_core::model::{AppStore, Session, SidebarMode, SidebarTint, Workspace};
use ngterm_core::persist::AppSettings;
use ngterm_core::persist::settings::FlaggedViewLayout;
use uuid::Uuid;

use crate::row_icon::Glyph;
use crate::status_glyph::{self, GlyphLook};

/// One sidebar row.
#[derive(Debug, Clone, PartialEq)]
pub struct SidebarRow {
    pub id: Uuid,
    pub kind: RowKind,
    /// A workspace's name; a session's display name, with ` : workspace` under the flat flagged layout.
    pub label: String,
    /// The unseen-notification count the trailing badge shows; 0 hides it.
    pub badge: usize,
    /// The leading glyph.
    pub icon: Glyph,
    /// The agent-status glyph; `None` hides it (idle, and every workspace row).
    pub status: Option<GlyphLook>,
    /// The leading glyph's tooltip: what a remote row's presentation stream is missing, `None` otherwise.
    pub icon_tooltip: Option<String>,
    /// The row's own tint, coloring its icon and/or name.
    pub tint: Option<SidebarTint>,
    /// A folded parent's companions that need attention (blocked, completed or unseen), by name: its companion
    /// marker shows while any does. Empty for every other row.
    pub folded_attention: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    /// A workspace row, rendered open or folded.
    Workspace { expanded: bool },
    /// A session row in `workspace`, `nested` under its workspace row (the trees) or not (the flat flagged list),
    /// `selected` while it is in the sidebar selection, with its place in a companion group.
    Session {
        workspace: Uuid,
        nested: bool,
        selected: bool,
        group: RowGroup,
    },
}

/// A session row's place in a companion group, as the sidebar shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RowGroup {
    /// A session with no companions, or a parent where companions do not nest (the flagged layouts).
    #[default]
    Plain,
    /// A parent with companions in the ordinary tree: its disclosure shows them (`expanded`) or folds them away.
    Parent { expanded: bool },
    /// A companion, `nested` one level under its parent row (the ordinary tree) or not (the flagged layouts).
    Companion { nested: bool },
}

impl SidebarRow {
    #[must_use]
    pub fn is_workspace(&self) -> bool {
        matches!(self.kind, RowKind::Workspace { .. })
    }

    /// What decides whether the list is rebuilt rather than updated in place: the row's id, its kind and its depth
    /// (upstream's `TreeShape`, its `parentIDs` while companions nest).
    #[must_use]
    pub fn shape(&self) -> (Uuid, bool, usize) {
        (self.id, self.is_workspace(), self.depth())
    }

    /// How many levels the row is indented: a workspace row and a flat-list row none, a session under its workspace
    /// one, a companion under its parent two.
    #[must_use]
    pub fn depth(&self) -> usize {
        match self.kind {
            RowKind::Workspace { .. } | RowKind::Session { nested: false, .. } => 0,
            RowKind::Session {
                group: RowGroup::Companion { nested: true },
                ..
            } => 2,
            RowKind::Session { .. } => 1,
        }
    }

    /// The row's companion group, `Plain` for a workspace row.
    #[must_use]
    pub fn group(&self) -> RowGroup {
        match self.kind {
            RowKind::Session { group, .. } => group,
            RowKind::Workspace { .. } => RowGroup::Plain,
        }
    }
}

/// What the outline shows open (upstream's coordinator `expandedWorkspaceIDs` and `expandedParentIDs`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Open {
    /// The workspace rows shown open.
    pub workspaces: HashSet<Uuid>,
    /// The parent rows whose companions show.
    pub parents: HashSet<Uuid>,
}

/// What the rows read beyond the store: the app-wide settings.
#[derive(Debug, Clone, Copy)]
pub struct RowSettings<'a> {
    pub settings: &'a AppSettings,
}

impl RowSettings<'_> {
    fn badges(self) -> bool {
        self.settings.notification_badge_enabled.unwrap_or(true)
    }

    fn layout(self) -> FlaggedViewLayout {
        self.settings.effective_flagged_view_layout()
    }
}

/// The flagged view's layout while the sidebar shows it, `None` in the ordinary tree: a change to this value, not to
/// the setting, reshapes the outline (upstream's coordinator `flaggedLayout`).
#[must_use]
pub fn shown_flagged_layout(store: &AppStore, settings: &AppSettings) -> Option<FlaggedViewLayout> {
    (store.sidebar().mode == SidebarMode::Flagged).then(|| settings.effective_flagged_view_layout())
}

/// The workspace rows and their session rows, in store order: the focus filter's workspaces with all their sessions
/// in the ordinary tree, every workspace holding a flagged session with only those in the flagged tree.
fn workspace_projection(store: &AppStore) -> Vec<(&Workspace, Vec<&Session>)> {
    if store.sidebar().mode == SidebarMode::Tree {
        return store
            .visible_workspaces()
            .into_iter()
            .map(|workspace| (workspace, workspace.sessions.iter().collect()))
            .collect();
    }
    store
        .workspaces()
        .iter()
        .filter_map(|workspace| {
            let flagged: Vec<&Session> = workspace.sessions.iter().filter(|session| session.flagged).collect();
            (!flagged.is_empty()).then_some((workspace, flagged))
        })
        .collect()
}

/// The rows of `store`'s sidebar, workspace rows open where `open` says (or forced open as the ordinary tree's sole
/// focused workspace), and a parent's companions shown where it says.
#[must_use]
pub fn rows(store: &AppStore, open: &Open, settings: RowSettings<'_>) -> Vec<SidebarRow> {
    let selection = selection_ids(store);
    let builder = RowBuilder {
        store,
        settings,
        selection: &selection,
        open_parents: &open.parents,
    };
    if !store.renders_workspace_rows(settings.layout()) {
        // the flat and plain lists show the saved flagged order
        return store
            .ordered_flagged_sessions()
            .into_iter()
            .map(|session| builder.session_row(session, false, RowGroup::of_flagged(session)))
            .collect();
    }
    let forced = (store.sidebar().mode == SidebarMode::Tree)
        .then(|| store.sole_focused_workspace_id())
        .flatten();
    let mut rows = Vec::new();
    for (workspace, sessions) in workspace_projection(store) {
        let shown = open.workspaces.contains(&workspace.id()) || forced == Some(workspace.id());
        rows.push(builder.workspace_row(workspace, &sessions, shown));
        if shown {
            builder.session_rows(&sessions, &mut rows);
        }
    }
    rows
}

impl RowGroup {
    /// A row's group in a flagged layout, where companions never nest.
    fn of_flagged(session: &Session) -> Self {
        if session.parent_id().is_some() {
            Self::Companion { nested: false }
        } else {
            Self::Plain
        }
    }
}

/// The session rows the sidebar shows selected: the multi-selection, else the selected session alone.
fn selection_ids(store: &AppStore) -> Vec<Uuid> {
    let selection = store.sidebar_selection_ids();
    if selection.is_empty() {
        store.selected_session_id().into_iter().collect()
    } else {
        selection
    }
}

struct RowBuilder<'a> {
    store: &'a AppStore,
    settings: RowSettings<'a>,
    selection: &'a [Uuid],
    open_parents: &'a HashSet<Uuid>,
}

impl RowBuilder<'_> {
    fn badge(&self, count: usize) -> usize {
        if self.settings.badges() { count } else { 0 }
    }

    fn workspace_row(&self, workspace: &Workspace, shown: &[&Session], expanded: bool) -> SidebarRow {
        // a flagged-tree header sums only the sessions it holds (upstream's `displayedUnseen(for:)`)
        let unseen = if self.store.sidebar().mode == SidebarMode::Flagged {
            shown.iter().map(|session| session.unseen_count).sum()
        } else {
            workspace.unseen_count()
        };
        SidebarRow {
            id: workspace.id(),
            kind: RowKind::Workspace { expanded },
            label: workspace.name.clone(),
            badge: self.badge(unseen),
            icon: Glyph::Workspace {
                focus_member: self.store.focus().ids().contains(&workspace.id()),
            },
            status: None,
            icon_tooltip: None,
            tint: workspace.tint,
            folded_attention: Vec::new(),
        }
    }

    /// A workspace's session rows under its open workspace row: in the ordinary tree each companion nests under the
    /// parent row it follows (upstream's `sessionNodes`), hidden while that parent is folded; the flagged tree lists
    /// them unindented.
    fn session_rows(&self, sessions: &[&Session], rows: &mut Vec<SidebarRow>) {
        if self.store.sidebar().mode != SidebarMode::Tree {
            rows.extend(
                sessions
                    .iter()
                    .map(|session| self.session_row(session, true, RowGroup::of_flagged(session))),
            );
            return;
        }
        let parents: HashMap<Uuid, bool> = sessions
            .iter()
            .filter(|session| session.parent_id().is_none() && self.store.has_companions(session.id()))
            .map(|session| (session.id(), self.open_parents.contains(&session.id())))
            .collect();
        // the top-level row a companion nests under: the last one listed
        let mut last_top: Option<Uuid> = None;
        for session in sessions {
            let group = match session.parent_id() {
                Some(parent) if last_top == Some(parent) => {
                    if parents.get(&parent) == Some(&false) {
                        continue;
                    }
                    RowGroup::Companion { nested: true }
                }
                _ => {
                    last_top = Some(session.id());
                    parents
                        .get(&session.id())
                        .map_or(RowGroup::Plain, |expanded| RowGroup::Parent { expanded: *expanded })
                }
            };
            rows.push(self.session_row(session, true, group));
        }
    }

    fn session_row(&self, session: &Session, nested: bool, group: RowGroup) -> SidebarRow {
        let workspace = self.store.workspace_for_session(session.id());
        // the tree names a row bare; a flagged layout, which drops the tree, names a companion after its parent
        let label = match shown_flagged_layout(self.store, self.settings.settings) {
            Some(FlaggedViewLayout::Flat) => format!(
                "{} : {}",
                self.store.list_label(session),
                workspace.map_or("", |workspace| workspace.name.as_str())
            ),
            Some(_) => self.store.list_label(session),
            None => session.display_name(),
        };
        let folded_attention = if group == (RowGroup::Parent { expanded: false }) {
            self.store
                .companions_needing_attention(session.id())
                .into_iter()
                .map(Session::display_name)
                .collect()
        } else {
            Vec::new()
        };
        SidebarRow {
            id: session.id(),
            kind: RowKind::Session {
                workspace: workspace.map_or_else(Uuid::nil, Workspace::id),
                nested,
                selected: self.selection.contains(&session.id()),
                group,
            },
            label,
            badge: self.badge(session.unseen_count),
            icon: session_icon(session, self.store.sidebar().mode == SidebarMode::Tree),
            status: status_glyph::resolve(&session.agent_indicator, self.settings.settings),
            icon_tooltip: session.presentation_notice(),
            tint: session.tint,
            folded_attention,
        }
    }
}

/// A session row's glyph: the split arrangement while the session has a split (shown or hidden), filled while
/// flagged in the tree (every flagged-view row is flagged, so the fill would be noise there); a remote session's
/// cloud marks a split by weight instead, slashed while its presentation stream is not up; a page companion's page
/// glyph has no split to mark.
fn session_icon(session: &Session, tree: bool) -> Glyph {
    if session.content() == ngterm_core::model::SessionContent::Page {
        return Glyph::Page {
            flagged: tree && session.flagged,
        };
    }
    let split = session.split.exists.then_some(session.split.axis);
    if session.remote_host().is_some() {
        return Glyph::Remote {
            split: split.is_some(),
            disconnected: session.presentation_notice().is_some(),
        };
    }
    Glyph::Session {
        split,
        flagged: tree && session.flagged,
    }
}

/// The workspace rows the outline renders open (upstream's coordinator `expandedWorkspaceIDs`): seeded from the
/// persisted `Workspace::is_expanded`, kept in step with the model on every refresh (gone workspaces drop out, any
/// the model marks open join, a user fold stays folded, a per-row request opens or folds its row), and opened for
/// view only when the selection moves into a folded workspace (`syncSelection`'s reveal, which never persists).
///
/// The parent rows showing their companions follow the same rules (upstream's `expandedParentIDs`): seeded from the
/// persisted fold, a change of it in the model followed (a user toggle), a per-row request (`session.collapse`/
/// `expand`) applied, and opened for view only when the selection moves onto a companion of a folded parent.
#[derive(Debug, Default)]
pub struct Expansion {
    open: Open,
    /// Each session's persisted fold as of the last refresh: a parent row follows only a change of it, so a reveal
    /// holds until the model moves.
    folds: HashMap<Uuid, bool>,
    /// The selection the last reveal ran for: only a selection change, or a switch of the view, reveals.
    revealed: Option<Uuid>,
    /// The view the rows were last built for (mode and flagged layout).
    view: Option<(SidebarMode, Option<FlaggedViewLayout>)>,
}

impl Expansion {
    /// Takes `store`'s pending per-row open and fold requests (upstream's `setWorkspaceExpandedNotified` and
    /// `setSessionExpandedNotified`): a workspace or parent row follows even where the persisted flag did not change,
    /// as after a reveal.
    pub fn apply_requests(&mut self, store: &mut AppStore) {
        for (id, expanded) in store.take_sidebar_expansion_requests() {
            if store.session(id).is_some() {
                self.set_parent(id, expanded);
            } else {
                self.set(id, expanded);
            }
        }
    }

    /// Brings the open sets in line with `store` shown under `layout` (see the type docs) and returns them.
    pub fn sync(&mut self, store: &AppStore, layout: Option<FlaggedViewLayout>) -> &Open {
        let existing: HashSet<Uuid> = store.workspaces().iter().map(Workspace::id).collect();
        let workspaces = &mut self.open.workspaces;
        workspaces.retain(|id| existing.contains(id));
        workspaces.extend(
            store
                .workspaces()
                .iter()
                .filter(|workspace| workspace.is_expanded)
                .map(Workspace::id),
        );
        self.sync_parents(store);
        // a switch of the view moves the selected row, maybe under a folded workspace: reveal it again
        let view = Some((store.sidebar().mode, layout));
        if self.view != view {
            self.view = view;
            self.revealed = None;
        }
        let selected = store.selected_session_id();
        if selected != self.revealed {
            if let Some(owner) = selected.and_then(|id| store.workspace_for_session(id)) {
                self.open.workspaces.insert(owner.id());
            }
            if let Some(parent) = selected.and_then(|id| store.session(id)).and_then(Session::parent_id) {
                self.open.parents.insert(parent);
            }
            self.revealed = selected;
        }
        &self.open
    }

    /// The parent rows: gone sessions drop out, and a session whose persisted fold changed (or is new) follows it.
    fn sync_parents(&mut self, store: &AppStore) {
        let folds: HashMap<Uuid, bool> = store
            .sessions()
            .map(|session| (session.id(), session.companion.expanded))
            .collect();
        self.open.parents.retain(|id| folds.contains_key(id));
        for (id, expanded) in &folds {
            if self.folds.get(id) != Some(expanded) {
                self.set_parent(*id, *expanded);
            }
        }
        self.folds = folds;
    }

    /// Records an open or fold of a workspace row.
    pub fn set(&mut self, id: Uuid, expanded: bool) {
        if expanded {
            self.open.workspaces.insert(id);
        } else {
            self.open.workspaces.remove(&id);
        }
    }

    /// Records an open or fold of a parent row's companions.
    pub fn set_parent(&mut self, id: Uuid, expanded: bool) {
        if expanded {
            self.open.parents.insert(id);
        } else {
            self.open.parents.remove(&id);
        }
    }

    /// Whether workspace `id`'s row is open.
    #[cfg(test)]
    #[must_use]
    pub fn contains(&self, id: Uuid) -> bool {
        self.open.workspaces.contains(&id)
    }

    /// Whether parent `id`'s companions show.
    #[cfg(test)]
    #[must_use]
    pub fn shows_companions(&self, id: Uuid) -> bool {
        self.open.parents.contains(&id)
    }
}

/// The badge's count, capped as upstream's `BadgeView` caps it.
#[must_use]
pub fn badge_text(count: usize) -> String {
    if count > 99 {
        "99+".to_owned()
    } else {
        count.to_string()
    }
}

#[cfg(test)]
mod tests {
    use ngterm_core::html::{HtmlOverlay, HtmlPageFlags, HtmlSource};
    use ngterm_core::model::{AgentIndicator, AgentStatus, NewSession, PageCompanionOptions, WindowLibrary};

    use super::*;

    /// A store of two workspaces: "one" with two sessions (the second selected), "two" with one.
    fn store() -> (AppStore, [Uuid; 2], [Uuid; 3]) {
        let mut library = WindowLibrary::seeded("/home/u");
        let window = library.active_window_id().unwrap();
        let mut store = std::mem::take(library.store_mut(window).unwrap());
        let one = store.workspaces()[0].id();
        let first = store.sessions().next().unwrap().id();
        let second = store.add_session(one, NewSession::at_cwd("/srv")).unwrap();
        let two = store.add_workspace("two");
        let third = store.add_session(two, NewSession::at_cwd("/var")).unwrap();
        store.select_session(Some(second));
        (store, [one, two], [first, second, third])
    }

    fn built(store: &AppStore, expansion: &mut Expansion, settings: &AppSettings) -> Vec<SidebarRow> {
        let layout = shown_flagged_layout(store, settings);
        let open = expansion.sync(store, layout).clone();
        rows(store, &open, RowSettings { settings })
    }

    fn shape(rows: &[SidebarRow]) -> Vec<(Uuid, bool)> {
        rows.iter().map(|row| (row.id, row.is_workspace())).collect()
    }

    fn selected(rows: &[SidebarRow]) -> Vec<Uuid> {
        rows.iter()
            .filter(|row| matches!(row.kind, RowKind::Session { selected: true, .. }))
            .map(|row| row.id)
            .collect()
    }

    #[test]
    fn open_workspaces_list_their_sessions_and_the_selection_is_highlighted() {
        let (mut store, [one, two], [first, second, third]) = store();
        let settings = AppSettings::default();
        let mut expansion = Expansion::default();
        let rows = built(&store, &mut expansion, &settings);
        assert_eq!(
            shape(&rows),
            [
                (one, true),
                (first, false),
                (second, false),
                (two, true),
                (third, false)
            ]
        );
        assert_eq!(rows[2].label, "srv");
        assert_eq!(rows[3].label, "two");
        assert_eq!(selected(&rows), [second]);
        assert!(matches!(rows[1].kind, RowKind::Session { nested: true, workspace, .. } if workspace == one));

        store.select_session_in_sidebar(Some(third), vec![second, third]);
        let rows = built(&store, &mut expansion, &settings);
        assert_eq!(selected(&rows), [second, third]);
    }

    #[test]
    fn a_folded_workspace_hides_its_sessions_until_the_selection_moves_in() {
        let (mut store, [one, two], [first, _, third]) = store();
        let settings = AppSettings::default();
        store.set_workspace_expanded(two, false);
        let mut expansion = Expansion::default();
        let _ = built(&store, &mut expansion, &settings);
        expansion.set(two, false);
        let rows = built(&store, &mut expansion, &settings);
        assert_eq!(shape(&rows)[3..], [(two, true)]);
        assert_eq!(rows[3].kind, RowKind::Workspace { expanded: false });

        // selecting into the folded workspace opens it for view, without persisting
        store.select_session(Some(third));
        let rows = built(&store, &mut expansion, &settings);
        assert_eq!(rows.last().map(|row| row.id), Some(third));
        assert!(!store.workspace(two).unwrap().is_expanded);

        expansion.set(one, false);
        store.set_workspace_expanded(one, false);
        let _ = built(&store, &mut expansion, &settings);
        assert!(!expansion.contains(one), "a fold holds while the selection stays");
        store.select_session(Some(first));
        let _ = built(&store, &mut expansion, &settings);
        assert!(expansion.contains(one), "a new selection reveals again");
    }

    #[test]
    fn a_collapse_or_expand_request_folds_or_opens_the_row_even_over_a_reveal() {
        let (mut store, [one, two], [first, _, third]) = store();
        store.set_workspace_expanded(two, false);
        store.select_session(Some(third));
        let mut expansion = Expansion::default();
        assert!(expansion.sync(&store, None).workspaces.contains(&two));
        store.set_workspace_expanded_in_outline(two, false);
        expansion.apply_requests(&mut store);
        assert!(!expansion.sync(&store, None).workspaces.contains(&two));
        store.set_workspace_expanded_in_outline(two, true);
        store.set_workspace_expanded_in_outline(one, false);
        expansion.apply_requests(&mut store);
        let open = expansion.sync(&store, None).clone();
        assert!(open.workspaces.contains(&two) && !open.workspaces.contains(&one));
        assert!(store.take_sidebar_expansion_requests().is_empty());
        store.select_session(Some(first));
        assert!(expansion.sync(&store, None).workspaces.contains(&one));
    }

    #[test]
    fn gone_workspaces_leave_and_new_ones_open() {
        let (mut store, [_, two], _) = store();
        let mut expansion = Expansion::default();
        let _ = expansion.sync(&store, None);
        store.remove_workspace(two);
        let three = store.add_workspace("three");
        let open = expansion.sync(&store, None);
        assert!(!open.workspaces.contains(&two));
        assert!(open.workspaces.contains(&three));
    }

    #[test]
    fn badges_count_unseen_notifications_when_enabled() {
        let (mut store, [one, _], [first, _, _]) = store();
        store.session_mut(first).unwrap().unseen_count = 3;
        let mut expansion = Expansion::default();
        let shown = built(&store, &mut expansion, &AppSettings::default());
        let badge_of = |rows: &[SidebarRow], id| rows.iter().find(|row| row.id == id).unwrap().badge;
        assert_eq!(badge_of(&shown, first), 3);
        assert_eq!(badge_of(&shown, one), 3, "the workspace rolls its sessions up");
        let off = AppSettings {
            notification_badge_enabled: Some(false),
            ..AppSettings::default()
        };
        let hidden = built(&store, &mut expansion, &off);
        assert_eq!(badge_of(&hidden, first), 0);
        assert_eq!(badge_of(&hidden, one), 0);
        assert_eq!(badge_text(7), "7");
        assert_eq!(badge_text(100), "99+");
    }

    #[test]
    fn the_focus_filter_shows_its_marked_workspace_forced_open_with_a_heavier_glyph() {
        let (mut store, [one, two], [_, _, third]) = store();
        store.set_workspace_expanded(two, false);
        store.select_session(Some(third));
        store.select_workspace(two);
        store.set_focused_workspace(two);
        let rows = rows(
            &store,
            &Open::default(),
            RowSettings {
                settings: &AppSettings::default(),
            },
        );
        assert!(rows.iter().all(|row| row.id != one));
        assert_eq!(rows[0].kind, RowKind::Workspace { expanded: true });
        assert_eq!(rows[0].icon, Glyph::Workspace { focus_member: true });
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn a_remote_rows_cloud_is_slashed_with_the_streams_notice_while_it_is_not_up() {
        use ngterm_core::model::{NewSession, RemoteBinding, RemoteConnection};
        let (mut store, [one, _], _) = store();
        let remote = store
            .add_session(
                one,
                NewSession {
                    remote_host: Some("buildbox".to_owned()),
                    ..NewSession::at_cwd("/tmp")
                },
            )
            .unwrap();
        store.bind_remote(
            RemoteBinding::new("ORIGIN", std::collections::HashMap::new(), Some(1), None),
            remote,
        );
        let row = |store: &AppStore| {
            let mut expansion = Expansion::default();
            built(store, &mut expansion, &AppSettings::default())
                .into_iter()
                .find(|row| row.id == remote)
                .unwrap()
        };
        store.set_remote_connection(RemoteConnection::Failed("exit 255".to_owned()), remote);
        let failed = row(&store);
        assert_eq!(
            failed.icon,
            Glyph::Remote {
                split: false,
                disconnected: true
            }
        );
        assert_eq!(
            failed.icon_tooltip.as_deref(),
            Some("Lost the connection to buildbox (exit 255), retrying. Close and reattach the session to retry now.")
        );
        store.set_remote_connection(RemoteConnection::Connected, remote);
        let connected = row(&store);
        assert_eq!(
            connected.icon,
            Glyph::Remote {
                split: false,
                disconnected: false
            }
        );
        assert_eq!(connected.icon_tooltip, None);
    }

    #[test]
    fn session_glyphs_carry_split_flag_and_status() {
        let (mut store, [one, _], [first, second, _]) = store();
        store.session_mut(first).unwrap().flagged = true;
        {
            let session = store.session_mut(second).unwrap();
            session.split.exists = true;
            session.split.axis = ngterm_core::model::SplitAxis::TopBottom;
            session.agent_indicator = AgentIndicator {
                status: AgentStatus::Blocked,
                ..AgentIndicator::default()
            };
        }
        let mut expansion = Expansion::default();
        let rows = built(&store, &mut expansion, &AppSettings::default());
        let row = |id| rows.iter().find(|row| row.id == id).unwrap();
        assert_eq!(
            row(first).icon,
            Glyph::Session {
                split: None,
                flagged: true
            }
        );
        assert_eq!(
            row(second).icon,
            Glyph::Session {
                split: Some(ngterm_core::model::SplitAxis::TopBottom),
                flagged: false
            }
        );
        assert_eq!(
            row(second).status.as_ref().map(|look| look.status),
            Some(AgentStatus::Blocked)
        );
        assert!(row(first).status.is_none() && row(one).status.is_none());
    }

    /// Upstream's `testAPageCompanionRowShowsThePageIconFilledWhenFlagged`: a page companion nests under its parent
    /// with the page glyph, filled while flagged in the tree.
    #[test]
    fn a_page_companion_row_shows_the_page_glyph_filled_when_flagged() {
        let (mut store, _, [first, _, _]) = store();
        let page = HtmlOverlay::new(
            HtmlSource::File {
                path: "/tmp/p.html".to_owned(),
                grant_root: None,
            },
            HtmlPageFlags::default(),
        );
        let options = PageCompanionOptions {
            select: false,
            ..PageCompanionOptions::default()
        };
        let id = store.add_page_companion(first, page, options, "/home/u").unwrap();
        let mut expansion = Expansion::default();
        let rows = built(&store, &mut expansion, &AppSettings::default());
        let row = rows.iter().find(|row| row.id == id).unwrap();
        assert_eq!(row.icon, Glyph::Page { flagged: false });
        assert_eq!(row.group(), RowGroup::Companion { nested: true });
        assert_eq!(row.label, "p.html");

        store.session_mut(id).unwrap().flagged = true;
        let rows = built(&store, &mut expansion, &AppSettings::default());
        let row = rows.iter().find(|row| row.id == id).unwrap();
        assert_eq!(row.icon, Glyph::Page { flagged: true });
    }

    #[test]
    fn the_flagged_view_lists_flagged_sessions_by_layout() {
        let (mut store, [one, two], [first, second, third]) = store();
        for id in [first, third] {
            store.session_mut(id).unwrap().flagged = true;
        }
        store.set_sidebar_mode(SidebarMode::Flagged);
        // after the switch: it reselects into the flagged set, which clears that session's badge
        store.session_mut(second).unwrap().unseen_count = 5;
        store.session_mut(third).unwrap().unseen_count = 2;
        let mut expansion = Expansion::default();

        let flat = built(&store, &mut expansion, &AppSettings::default());
        assert_eq!(shape(&flat), [(first, false), (third, false)]);
        assert_eq!(flat[0].label, "u : workspace 1");
        assert!(matches!(flat[0].kind, RowKind::Session { nested: false, .. }));
        assert_eq!(
            flat[0].icon,
            Glyph::Session {
                split: None,
                flagged: false
            },
            "no fill in the flagged view"
        );

        let plain = AppSettings {
            flagged_view_layout: Some("plain".into()),
            ..AppSettings::default()
        };
        assert_eq!(built(&store, &mut expansion, &plain)[0].label, "u");

        let tree = AppSettings {
            flagged_view_layout: Some("tree".into()),
            ..AppSettings::default()
        };
        let rows = built(&store, &mut expansion, &tree);
        assert_eq!(shape(&rows), [(one, true), (first, false), (two, true), (third, false)]);
        assert_eq!(rows[0].badge, 0, "the header sums only its flagged sessions");
        assert_eq!(rows[2].badge, 2);
        assert_eq!(rows[1].label, "u");
    }

    /// Upstream's flat and plain rows follow `orderedFlaggedSessions`; the flagged tree keeps tree order.
    #[test]
    fn the_flat_and_plain_lists_show_the_saved_flagged_order() {
        let (mut store, [one, two], [first, second, third]) = store();
        for id in [third, first, second] {
            store.set_flag(true, id);
        }
        assert!(store.move_flagged(&[second], ngterm_core::model::FlaggedOrderPlacement::Before, third));
        store.set_sidebar_mode(SidebarMode::Flagged);
        let mut expansion = Expansion::default();
        for layout in ["flat", "plain"] {
            let settings = AppSettings {
                flagged_view_layout: Some(layout.into()),
                ..AppSettings::default()
            };
            let rows = built(&store, &mut expansion, &settings);
            assert_eq!(
                shape(&rows),
                [(second, false), (third, false), (first, false)],
                "{layout}"
            );
        }
        let tree = AppSettings {
            flagged_view_layout: Some("tree".into()),
            ..AppSettings::default()
        };
        let rows = built(&store, &mut expansion, &tree);
        assert_eq!(
            shape(&rows),
            [
                (one, true),
                (first, false),
                (second, false),
                (two, true),
                (third, false)
            ]
        );
    }

    #[test]
    fn a_view_switch_reveals_the_selection_again() {
        let (mut store, [_, two], [_, _, third]) = store();
        store.session_mut(third).unwrap().flagged = true;
        store.select_session(Some(third));
        let tree = AppSettings {
            flagged_view_layout: Some("tree".into()),
            ..AppSettings::default()
        };
        let mut expansion = Expansion::default();
        let _ = built(&store, &mut expansion, &tree);
        expansion.set(two, false);
        store.set_workspace_expanded(two, false);
        let _ = built(&store, &mut expansion, &tree);
        assert!(!expansion.contains(two));
        store.set_sidebar_mode(SidebarMode::Flagged);
        let rows = built(&store, &mut expansion, &tree);
        assert!(expansion.contains(two));
        assert_eq!(rows.last().map(|row| row.id), Some(third));
    }

    // MARK: - companions (upstream's `WorkspaceSidebarCompanionsTests`)

    /// Upstream's `CompanionRows`: workspace "work" holds "parent" with companions "first" and "second", then "after".
    struct Group {
        store: AppStore,
        work: Uuid,
        parent: Uuid,
        first: Uuid,
        second: Uuid,
        after: Uuid,
    }

    fn group() -> Group {
        let mut store = AppStore::new();
        let work = store.add_workspace("work");
        let named = |store: &mut AppStore, name: &str| {
            let id = store.add_session(work, NewSession::at_cwd("/tmp")).unwrap();
            store.rename_session(id, name);
            id
        };
        let parent = named(&mut store, "parent");
        let after = named(&mut store, "after");
        let companion = |store: &mut AppStore, name: &str| {
            let options = ngterm_core::model::CompanionOptions {
                name: Some(name.to_owned()),
                ..ngterm_core::model::CompanionOptions::default()
            };
            store.add_companion(parent, options, "/home/u").unwrap()
        };
        let first = companion(&mut store, "first");
        let second = companion(&mut store, "second");
        store.select_session(Some(after));
        Group {
            store,
            work,
            parent,
            first,
            second,
            after,
        }
    }

    fn row(rows: &[SidebarRow], id: Uuid) -> &SidebarRow {
        rows.iter().find(|row| row.id == id).expect("the row shows")
    }

    #[test]
    fn companion_rows_nest_one_level_under_their_parent() {
        let g = group();
        let rows = built(&g.store, &mut Expansion::default(), &AppSettings::default());
        assert_eq!(
            shape(&rows),
            [
                (g.work, true),
                (g.parent, false),
                (g.first, false),
                (g.second, false),
                (g.after, false)
            ]
        );
        assert_eq!(row(&rows, g.parent).group(), RowGroup::Parent { expanded: true });
        assert_eq!(row(&rows, g.first).group(), RowGroup::Companion { nested: true });
        assert_eq!(row(&rows, g.after).group(), RowGroup::Plain);
        assert_eq!(row(&rows, g.first).depth(), row(&rows, g.parent).depth() + 1);
        assert_eq!(row(&rows, g.first).label, "first", "the tree keeps the bare name");
        assert!(row(&rows, g.parent).folded_attention.is_empty());
    }

    /// Upstream's `testFlaggedLayoutsListCompanionsAtTheirParentsLevel` and
    /// `testEveryFlaggedLayoutNamesACompanionAfterItsParentAndTheTreeDoesNot`.
    #[test]
    fn flagged_layouts_list_companions_at_their_parents_level_named_after_it() {
        let mut g = group();
        g.store.set_flags(true, &[g.parent, g.first]);
        g.store.set_sidebar_mode(SidebarMode::Flagged);
        for (layout, label) in [
            ("plain", "parent › first"),
            ("flat", "parent › first : work"),
            ("tree", "parent › first"),
        ] {
            let settings = AppSettings {
                flagged_view_layout: Some(layout.into()),
                ..AppSettings::default()
            };
            let rows = built(&g.store, &mut Expansion::default(), &settings);
            let first = row(&rows, g.first);
            assert_eq!(first.label, label, "{layout}");
            assert_eq!(first.depth(), row(&rows, g.parent).depth(), "{layout}");
            assert_eq!(first.group(), RowGroup::Companion { nested: false }, "{layout}");
            assert_eq!(row(&rows, g.parent).group(), RowGroup::Plain, "{layout}: no fold");
        }
    }

    /// Upstream's `testFoldingAParentPersistsAndShowsTheCompanionsThatNeedAttention` and
    /// `testAFoldedParentStaysFoldedAcrossARebuild`, on the rows.
    #[test]
    fn a_folded_parent_hides_its_companions_and_names_the_ones_that_need_attention() {
        let mut g = group();
        g.store.set_agent_indicator(
            AgentIndicator {
                status: AgentStatus::Blocked,
                ..AgentIndicator::default()
            },
            g.first,
        );
        let settings = AppSettings::default();
        let mut expansion = Expansion::default();
        let _ = built(&g.store, &mut expansion, &settings);
        // the user's fold: the row folds at once, the model follows through the action
        expansion.set_parent(g.parent, false);
        assert!(g.store.set_session_expanded(g.parent, false));
        let rows = built(&g.store, &mut expansion, &settings);
        assert_eq!(shape(&rows), [(g.work, true), (g.parent, false), (g.after, false)]);
        let parent = row(&rows, g.parent);
        assert_eq!(parent.group(), RowGroup::Parent { expanded: false });
        assert_eq!(parent.folded_attention, ["first"]);
        // a rebuild keeps it folded
        let rows = built(&g.store, &mut Expansion::default(), &settings);
        assert_eq!(rows.len(), 3);

        // the model unfolding it (`session.expand`) opens the row
        assert!(g.store.set_session_expanded(g.parent, true));
        let rows = built(&g.store, &mut expansion, &settings);
        assert_eq!(rows.len(), 5);
        assert!(
            row(&rows, g.parent).folded_attention.is_empty(),
            "an open parent shows its companions"
        );
        // and folding it there (`session.collapse`) folds the row
        assert!(g.store.set_session_expanded(g.parent, false));
        assert_eq!(built(&g.store, &mut expansion, &settings).len(), 3);
    }

    /// Upstream's `testSelectingACompanionOfAFoldedParentRevealsItWithoutPersisting` and
    /// `testFoldingAParentKeepsTheActiveCompanion`.
    #[test]
    fn selecting_a_companion_of_a_folded_parent_reveals_it_without_persisting() {
        let mut g = group();
        assert!(g.store.set_session_expanded(g.parent, false));
        g.store.set_workspace_expanded(g.work, false);
        let settings = AppSettings::default();
        let mut expansion = Expansion::default();
        let rows = built(&g.store, &mut expansion, &settings);
        assert!(rows.iter().all(|row| row.id != g.second));

        g.store.select_session(Some(g.second));
        let rows = built(&g.store, &mut expansion, &settings);
        assert!(rows.iter().any(|row| row.id == g.second), "revealed");
        assert!(expansion.shows_companions(g.parent));
        assert!(
            !g.store.session(g.parent).unwrap().companion.expanded,
            "a reveal is a view action"
        );
        assert_eq!(g.store.selected_session_id(), Some(g.second));

        // folding the parent over its selected companion keeps the selection
        expansion.set_parent(g.parent, false);
        let rows = built(&g.store, &mut expansion, &settings);
        assert!(rows.iter().all(|row| row.id != g.second));
        assert_eq!(g.store.selected_session_id(), Some(g.second));
    }

    /// `session.collapse` on a parent a reveal opened, whose saved fold is already folded: the row still folds
    /// (upstream's `setSessionExpanded` posting on an unchanged value).
    #[test]
    fn collapsing_a_revealed_parent_folds_its_row() {
        let mut g = group();
        assert!(g.store.set_session_expanded(g.parent, false));
        let settings = AppSettings::default();
        let mut expansion = Expansion::default();
        let _ = built(&g.store, &mut expansion, &settings);
        g.store.select_session(Some(g.second));
        let _ = built(&g.store, &mut expansion, &settings);
        assert!(expansion.shows_companions(g.parent), "revealed");

        assert!(g.store.set_session_expanded_in_outline(g.parent, false));
        expansion.apply_requests(&mut g.store);
        let rows = built(&g.store, &mut expansion, &settings);
        assert!(!expansion.shows_companions(g.parent));
        assert!(rows.iter().all(|row| row.id != g.second), "folded over the selection");

        // and `session.expand` opens it again
        assert!(g.store.set_session_expanded_in_outline(g.parent, true));
        expansion.apply_requests(&mut g.store);
        let rows = built(&g.store, &mut expansion, &settings);
        assert!(rows.iter().any(|row| row.id == g.second));
    }
}
