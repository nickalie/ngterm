//! `pane.focused` (upstream's `AppStore+PaneFocus.swift`): which split pane each session has selected, by the pane's
//! stable identity, and the settled changes of it.
//!
//! Keyed on identity, not the left/right role, so a swap, which exchanges both identities and toggles the split's
//! focus, reads as no change. Upstream observes the store and re-arms a 100 ms trailing debounce on every change; here
//! [`Core::commit`](crate::Core::commit), which every writer of the split's focus passes through, asks
//! [`AppStore::observe_pane_focus`] whether the identities moved and arms the window's timer, and the fired timer runs
//! [`AppStore::emit_pane_focus_changes`] against the settled baseline, so a move and back inside the window is silent.

use std::collections::HashMap;

use ngterm_proto::events::EventKind;
use uuid::Uuid;

use super::outbox::EventPayload;
use super::pane::StatusPane;
use super::session::Session;
use super::store::AppStore;

/// Each session's selected pane identity, by session id.
pub(crate) type PaneIdentities = HashMap<Uuid, Uuid>;

/// What `pane.focused` diffs: the settled selection, and the one the last commit saw. A store observes from its
/// creation, as upstream's starts at its init: both start empty, so what a new store gains before the core first
/// commits it (a new window's default session) is new to the baseline and only joins it at the debounce's check.
#[derive(Debug, Default)]
pub(crate) struct PaneFocusTracking {
    /// The settled selected-pane identity per session (upstream's `paneFocusBaseline`).
    pub(crate) baseline: PaneIdentities,
    /// The identities the last [`AppStore::observe_pane_focus`] saw, so only a change re-arms the debounce.
    observed: PaneIdentities,
}

/// The pane session `session` has selected: the split pane while it is the focused one, else the primary.
fn selected_pane(session: &Session) -> StatusPane {
    if session.focuses_split() {
        StatusPane::Right
    } else {
        StatusPane::Left
    }
}

/// The identity of the pane session `session` has selected; the primary's while a focused split has none yet.
fn selected_identity(session: &Session) -> Uuid {
    match selected_pane(session) {
        StatusPane::Right => session.split_pane_identity.unwrap_or(session.pane_identity),
        StatusPane::Left | StatusPane::Scratch => session.pane_identity,
    }
}

impl AppStore {
    /// The stable identity of the pane each session has selected (upstream's `selectedPaneIdentities`).
    #[must_use]
    pub(crate) fn selected_pane_identities(&self) -> PaneIdentities {
        self.sessions()
            .map(|session| (session.id(), selected_identity(session)))
            .collect()
    }

    /// Whether the selected identities moved since the last call (or since the store's creation or last restore):
    /// the commit's question, whose yes re-arms the window's debounce. A session new to the store (a new window's
    /// default session, created, moved in) joins the baseline only at the debounce's check, as upstream's
    /// ([`AppStore::emit_pane_focus_changes`]), so a split within 100 ms of the create goes unreported.
    pub(crate) fn observe_pane_focus(&mut self) -> bool {
        let current = self.selected_pane_identities();
        if self.pane_focus.observed == current {
            return false;
        }
        self.pane_focus.observed = current;
        true
    }

    /// Takes the current selection as settled without reporting it: a reload is not a selection (upstream's
    /// rebaseline at the end of `restore(from:)`).
    pub(crate) fn rebaseline_pane_focus(&mut self) {
        let current = self.selected_pane_identities();
        self.pane_focus.baseline.clone_from(&current);
        self.pane_focus.observed = current;
    }

    /// Emits one `pane.focused` per session whose selected pane identity moved since the last check, carrying the
    /// display name, the pane (`left` or `right`) and that pane's surface token as `paneID`, absent before its surface
    /// exists. A session new to this store (created, moved in) or gone from it only updates the baseline (upstream's
    /// `emitPaneFocusChanges`).
    pub(crate) fn emit_pane_focus_changes(&mut self) {
        let current = self.selected_pane_identities();
        let mut moved: Vec<(Uuid, Uuid, EventPayload)> = Vec::new();
        for workspace in &self.workspaces {
            for session in &workspace.sessions {
                let (Some(previous), Some(identity)) =
                    (self.pane_focus.baseline.get(&session.id()), current.get(&session.id()))
                else {
                    continue;
                };
                if previous == identity {
                    continue;
                }
                let pane = selected_pane(session);
                let payload = EventPayload {
                    name: Some(session.display_name()),
                    pane: Some(pane.as_str().to_owned()),
                    pane_id: crate::projection::pane_token(session.pane_slot(pane)),
                    ..EventPayload::default()
                };
                moved.push((workspace.id(), session.id(), payload));
            }
        }
        self.pane_focus.baseline = current;
        for (workspace, session, payload) in moved {
            self.emit(EventKind::PaneFocused, Some(workspace), Some(session), payload);
        }
    }
}
