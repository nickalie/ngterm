//! Companion sessions (upstream's `AppStore+Companions.swift`): a session created under another one, its parent,
//! sitting right after it in the parent's workspace. A parent and its companions form a group the sidebar nests and
//! folds, a move carries whole and an insertion never splits. A parent's close takes its companions along into one
//! Open Recent entry and, when soft, one undo (`store.rs`, `pending_close.rs`); a lone companion reopens only under
//! its parent (`window_library.rs`).
//!
//! The tree stays flat: a group is a top-level session followed by the run of companions naming it, so every
//! placement rule here keeps a companion inside its parent's run and a top-level session outside any run. Companions
//! have no companions of their own: a companion target means its parent.
//!
//! A local parent whose last pane exits while it has companions restarts a plain login shell in its last cwd instead
//! of closing ([`AppStore::close_primary_pane_at`], `panes.rs`), unless its previous restart is under
//! [`RESPAWN_MINIMUM_UPTIME`] old. The pane keeps its identity, and [`Session::primary_respawns`] counts the restarts
//! so a frontend mounts a fresh terminal in the emptied primary slot.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use uuid::Uuid;

use super::session::{AgentStatus, Session};
use super::store::{AppStore, NewSession};
use super::tint::SidebarTint;

/// What a session shows: a terminal, or an HTML page with no shell under it (a page companion).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum SessionContent {
    #[default]
    Terminal,
    Page,
}

/// A session's place in a companion group (upstream's `Session.parentID` and `isExpanded`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompanionState {
    /// The session this one is a companion of; set at creation and never changed, except that a rebuilt workspace
    /// drops a link naming no top-level session in it. Persisted.
    pub(crate) parent: Option<Uuid>,
    /// Whether a parent row shows its companions in the sidebar tree. Persisted only when folded.
    pub expanded: bool,
}

/// How long a respawned shell must run before its exit respawns it again (upstream's `respawnMinimumUptime`): one
/// exiting sooner (a broken shell or profile, a failing live attach) would otherwise respawn forever.
pub const RESPAWN_MINIMUM_UPTIME: Duration = Duration::from_secs(2);

/// A parent's primary-pane restarts (upstream's `primaryRespawns` and `lastRespawnAt`). Ephemeral.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct RespawnState {
    /// How many times the primary pane restarted; a frontend mounts a fresh terminal when it moves.
    pub(crate) count: u64,
    /// When the latest restart happened, on the monotonic clock.
    pub(crate) last_at: Option<Instant>,
}

/// What a primary pane's exit did to its session (upstream's `PrimaryPaneExit`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimaryPaneExit {
    /// The session closed (with its companions, for a parent).
    Closed,
    /// A split survivor moved into the primary slot.
    Promoted,
    /// The pane restarts a plain login shell under the same identity.
    Respawned,
}

/// How a new companion starts (upstream's `CompanionOptions`); a `None` cwd opens it in the parent's focused cwd.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompanionOptions {
    pub cwd: Option<String>,
    pub command: Option<String>,
    pub name: Option<String>,
    pub wait: bool,
    /// Select the companion; `false` creates it in the background.
    pub select: bool,
    /// The creation tint.
    pub tint: Option<SidebarTint>,
}

impl Default for CompanionOptions {
    fn default() -> Self {
        Self {
            cwd: None,
            command: None,
            name: None,
            wait: false,
            select: true,
            tint: None,
        }
    }
}

/// Why a companion was not created (upstream's `CompanionRefusal`; the page refusal joins with page companions).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompanionRefusal {
    NoSuchSession,
    /// The parent is attached from another machine.
    RemoteParent,
}

/// Where a companion goes: the parent's workspace, the end of its group, and the cwd it starts in.
struct CompanionSlot {
    workspace: Uuid,
    index: usize,
    parent: Uuid,
    cwd: String,
}

impl AppStore {
    /// A parent's companions in sidebar order; none for a companion or an unknown id.
    #[must_use]
    pub fn companions(&self, parent: Uuid) -> Vec<&Session> {
        let Some(at) = self.location(parent) else {
            return Vec::new();
        };
        let sessions = &self.workspaces[at.workspace].sessions;
        if sessions[at.session].companion.parent.is_some() {
            return Vec::new();
        }
        sessions[at.session + 1..group_end(at.session, sessions)]
            .iter()
            .collect()
    }

    /// Whether `parent` has a companion.
    #[must_use]
    pub fn has_companions(&self, parent: Uuid) -> bool {
        !self.companions(parent).is_empty()
    }

    /// Whether session `id`'s last pane exiting restarts its primary shell rather than closing it: a local session with
    /// companions whose previous restart (if any) is at least [`RESPAWN_MINIMUM_UPTIME`] before `now`. A remote row is
    /// restarted by its owner on the other machine, never here.
    #[must_use]
    pub fn respawns_on_last_pane_exit(&self, id: Uuid, now: Instant) -> bool {
        let Some(session) = self.session(id) else {
            return false;
        };
        if session.remote_host().is_some() || !self.has_companions(id) {
            return false;
        }
        session
            .respawn
            .last_at
            .is_none_or(|last| now.saturating_duration_since(last) >= RESPAWN_MINIMUM_UPTIME)
    }

    /// The parent of `session`; `None` for a top-level session.
    #[must_use]
    pub fn parent_of(&self, session: &Session) -> Option<&Session> {
        session.companion.parent.and_then(|id| self.session(id))
    }

    /// Creates a companion after the parent's last one and returns its id; a companion target means its parent.
    /// `home` stands in for a parent's remote cwd, as a duplicate's does.
    ///
    /// # Errors
    ///
    /// [`CompanionRefusal::NoSuchSession`] for an unknown target, [`CompanionRefusal::RemoteParent`] for a parent
    /// attached from another machine.
    pub fn add_companion(
        &mut self,
        target: Uuid,
        options: CompanionOptions,
        home: &str,
    ) -> Result<Uuid, CompanionRefusal> {
        let slot = self.companion_slot(target, options.cwd, home)?;
        let spec = NewSession {
            command: options.command,
            name: options.name,
            wait: options.wait,
            at: Some(slot.index),
            select: options.select,
            tint: options.tint,
            parent: Some(slot.parent),
            ..NewSession::at_cwd(slot.cwd)
        };
        self.add_session(slot.workspace, spec)
            .ok_or(CompanionRefusal::NoSuchSession)
    }

    fn companion_slot(&self, target: Uuid, cwd: Option<String>, home: &str) -> Result<CompanionSlot, CompanionRefusal> {
        let target = self.session(target).ok_or(CompanionRefusal::NoSuchSession)?;
        let parent = self.parent_of(target).unwrap_or(target);
        let at = self.location(parent.id()).ok_or(CompanionRefusal::NoSuchSession)?;
        if parent.remote_host().is_some() {
            return Err(CompanionRefusal::RemoteParent);
        }
        let workspace = &self.workspaces[at.workspace];
        let cwd = cwd.unwrap_or_else(|| parent.local_working_directory(parent.focused_cwd(), home));
        Ok(CompanionSlot {
            workspace: workspace.id(),
            index: group_end(at.session, &workspace.sessions),
            parent: parent.id(),
            cwd,
        })
    }

    /// Folds or unfolds a parent's companions and saves; false for a session without any. An unchanged value
    /// answers true without a save.
    pub fn set_session_expanded(&mut self, id: Uuid, expanded: bool) -> bool {
        if !self.has_companions(id) {
            return false;
        }
        let Some(session) = self.session_mut(id) else {
            return false;
        };
        if session.companion.expanded == expanded {
            return true;
        }
        session.companion.expanded = expanded;
        self.schedule_tree_changed();
        self.save();
        true
    }

    /// [`AppStore::set_session_expanded`] plus a request that the outline open or fold the parent row to match, even
    /// on an unchanged value, since a reveal may have opened a row the saved fold calls folded (upstream's
    /// `AppActions.setSessionExpanded` posting `.agtermSetSessionExpanded`). False for a session without companions.
    pub fn set_session_expanded_in_outline(&mut self, id: Uuid, expanded: bool) -> bool {
        if !self.set_session_expanded(id, expanded) {
            return false;
        }
        self.sidebar.expansion_requests.insert(id, expanded);
        true
    }

    /// A parent's companions that are blocked, completed or carry unseen notifications: the ones a folded parent row
    /// must still surface.
    #[must_use]
    pub fn companions_needing_attention(&self, parent: Uuid) -> Vec<&Session> {
        self.companions(parent)
            .into_iter()
            .filter(|session| {
                matches!(
                    session.agent_indicator.status,
                    AgentStatus::Blocked | AgentStatus::Completed
                ) || session.unseen_count > 0
            })
            .collect()
    }

    /// Where `session` goes when inserted near `index` of `sessions`: a companion inside its parent's group, from
    /// right after the parent to the group's end; a top-level session past any group `index` falls inside.
    pub(crate) fn insertion_index(session: &Session, index: usize, sessions: &[Session]) -> usize {
        let parent = session
            .companion
            .parent
            .and_then(|parent| sessions.iter().position(|candidate| candidate.id() == parent));
        match parent {
            Some(parent) => index.max(parent + 1).min(group_end(parent, sessions)),
            None => top_level_insertion_index(index, sessions),
        }
    }
}

/// The index just past the group starting at `index`: the first session after it that is not a companion.
pub(crate) fn group_end(index: usize, sessions: &[Session]) -> usize {
    let mut end = index + 1;
    while end < sessions.len() && sessions[end].companion.parent.is_some() {
        end += 1;
    }
    end
}

/// A top-level slot: `index`, clamped, moved past the rest of a group it falls inside.
pub(crate) fn top_level_insertion_index(index: usize, sessions: &[Session]) -> usize {
    let mut slot = index.min(sessions.len());
    while slot < sessions.len() && sessions[slot].companion.parent.is_some() {
        slot += 1;
    }
    slot
}

/// One workspace's sessions regrouped: each parent followed by its companions in their order, a link naming no
/// top-level session of the list (only a hand-edited snapshot makes one) dropped, its session becoming top-level.
pub(crate) fn regrouped_companions(sessions: Vec<Session>) -> Vec<Session> {
    let parents: HashSet<Uuid> = sessions
        .iter()
        .filter(|session| session.companion.parent.is_none())
        .map(Session::id)
        .collect();
    let mut top: Vec<Session> = Vec::new();
    let mut by_parent: HashMap<Uuid, Vec<Session>> = HashMap::new();
    for mut session in sessions {
        match session.companion.parent {
            Some(parent) if parents.contains(&parent) => by_parent.entry(parent).or_default().push(session),
            Some(_) => {
                session.companion.parent = None;
                top.push(session);
            }
            None => top.push(session),
        }
    }
    let mut regrouped = Vec::with_capacity(top.len());
    for session in top {
        let companions = by_parent.remove(&session.id()).unwrap_or_default();
        regrouped.push(session);
        regrouped.extend(companions);
    }
    regrouped
}
