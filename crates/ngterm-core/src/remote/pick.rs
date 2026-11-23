//! Session-bound picks across machines (upstream's `SessionPick.swift`: `PresentationPick`, its answer and reference,
//! `presentPickRemotely`; `ControlServer+Pick.swift`'s `takeBackRemotePick` and `showReplicaPick`).
//!
//! On the origin, a pick bound to a session that presents remotely goes to the session's presenter when its hello lists
//! `pick.request`; the book ([`crate::modal::SessionPicks`]) keeps it and its outcome here. A presenter that refuses it
//! or goes away hands it back into the session's window. On a viewer, a pick its origin handed over is a replica in the
//! window holding the attached row, and its outcome goes back on the stream.

use ngterm_proto::{ControlPickOutcome, ControlPickResult, PickItem};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::frames::{Body, fits};
use crate::Core;
use crate::modal::PendingPick;
use crate::windows::WindowRequest;

/// A session-bound pick the origin hands to the viewer presenting the session. `owner` is the presenter generation it
/// went out under, so an answer from a presenter that has since lost the role is dropped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentationPick {
    pub id: String,
    pub owner: i64,
    pub items: Vec<PickItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_custom: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<String>,
    /// Raises the presenter's window holding the row, as `--follow` raises the origin's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub follow: Option<bool>,
}

impl PresentationPick {
    /// `pick` as handed to the presenter of generation `owner`.
    #[must_use]
    pub fn new(pick: &PendingPick, owner: i64, follow: bool) -> Self {
        Self {
            id: pick.id.clone(),
            owner,
            items: pick.items.clone(),
            prompt: pick.prompt.clone(),
            query: pick.query.clone(),
            allow_custom: pick.allow_custom.then_some(true),
            selection: pick.selection.clone(),
            follow: follow.then_some(true),
        }
    }

    /// The pick as the presenter's own window shows it, under the origin's id.
    #[must_use]
    pub fn pending(&self) -> PendingPick {
        PendingPick {
            id: self.id.clone(),
            items: self.items.clone(),
            prompt: self.prompt.clone(),
            query: self.query.clone(),
            allow_custom: self.allow_custom == Some(true),
            selection: self.selection.clone(),
        }
    }

    /// Whether the request encodes under the frame cap; a thousand long rows may not.
    #[must_use]
    pub fn fits_frame(&self) -> bool {
        fits(Body::PickRequest(self.clone()))
    }
}

/// The presenter's outcome: `item` for a picked row, `query` for a custom answer, neither for a cancel. The origin
/// derives the label and index from the items it stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentationPickAnswer {
    pub id: String,
    pub owner: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
}

impl PresentationPickAnswer {
    /// The answer that stands for `result` on pick `id` under `owner`.
    #[must_use]
    pub fn new(id: &str, owner: i64, result: &ControlPickResult) -> Self {
        let (item, query) = match result.result {
            ControlPickOutcome::Picked => (result.id.clone(), None),
            ControlPickOutcome::Custom => (None, Some(result.query.clone().unwrap_or_default())),
            ControlPickOutcome::Cancelled | ControlPickOutcome::Pending => (None, None),
        };
        Self {
            id: id.to_owned(),
            owner,
            item,
            query,
        }
    }
}

/// Names one handed-over pick, for a refusal or a dismissal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentationPickRef {
    pub id: String,
    pub owner: i64,
}

impl PendingPick {
    /// The outcome a presenter's answer stands for; `None` when it names a row this pick does not have or a custom
    /// answer it does not accept (upstream's `validatedResult`).
    #[must_use]
    pub fn validated_result(&self, item: Option<&str>, query: Option<&str>) -> Option<ControlPickResult> {
        match (item, query) {
            (None, None) => Some(ControlPickResult::outcome(ControlPickOutcome::Cancelled)),
            (Some(item), None) => self
                .items
                .iter()
                .position(|candidate| candidate.id == item)
                .and_then(|index| self.picked(index)),
            (None, Some(query)) => self.allow_custom.then(|| Self::custom(query)),
            (Some(_), Some(_)) => None,
        }
    }
}

// MARK: - the origin

impl Core {
    /// Hands a session-bound pick to the presenter of `session` (upstream's `presentPickRemotely`): only while the
    /// session presents remotely and its presenter speaks the pick frames. `None` leaves it to the caller; `Some(false)`
    /// means the session already has a pick.
    pub(crate) fn present_pick_remotely(
        &mut self,
        pick: &PendingPick,
        session: Uuid,
        window: Option<Uuid>,
        follow: bool,
    ) -> Option<bool> {
        let hub = &self.remote.hub;
        if !hub.has_presenter(session) || !hub.presenter_supports("pick.request", session) {
            return None;
        }
        let found = self.presented_session(session)?;
        if !self.presents_remotely(found, None) {
            return None;
        }
        let owner = self.remote.hub.presenter_generation(session);
        let request = PresentationPick::new(pick, owner, follow);
        if !request.fits_frame() {
            return None;
        }
        if !self
            .modals
            .session_picks
            .open_remote(pick.clone(), session, owner, window, follow)
        {
            return Some(false);
        }
        // booked first: a presenter this send drops is taken back as a lost one
        self.remote.hub.send_to_presenter(Body::PickRequest(request), session);
        self.run_hub_events();
        Some(true)
    }

    /// Takes `session`'s presenter-held pick back into the session's window after the presenter refused it
    /// (`refused`) or went away (upstream's `takeBackRemotePick`); one that cannot be shown there ends cancelled. A
    /// headless origin, which draws nothing, holds it for the next presenter instead (upstream's `returnToPending`).
    pub(crate) fn take_back_remote_pick(&mut self, session: Uuid, refused: Option<&PresentationPickRef>) {
        if self.presenter_only() {
            self.modals.session_picks.return_to_pending(session, refused);
            return;
        }
        let window = self.library().window_holding_session(session);
        self.modals.hand_back_session_pick(session, refused, window);
    }

    /// Hands `session`'s unpresented pick to its presenter under the current generation (upstream's
    /// `offerPendingPick`), ending it first when its deadline passed, so an expired pick is never shown. It stays
    /// unpresented when the presenter cannot show it.
    pub(crate) fn offer_held_pick(&mut self, session: Uuid) {
        let Some((pick, follow)) = self
            .modals
            .session_picks
            .unpresented(session)
            .map(|(pick, follow)| (pick.clone(), follow))
        else {
            return;
        };
        if self.modals.expired(&pick.id) {
            drop(crate::commands::pick::cancel(self, &pick.id, None));
            return;
        }
        let hub = &self.remote.hub;
        if !hub.has_presenter(session) || !hub.presenter_supports("pick.request", session) {
            return;
        }
        let owner = hub.presenter_generation(session);
        let request = PresentationPick::new(&pick, owner, follow);
        if !request.fits_frame() {
            return;
        }
        self.modals.session_picks.offer(session, owner);
        self.remote.hub.send_to_presenter(Body::PickRequest(request), session);
    }

    /// Applies the presenter's answer to a pick it was handed; a stale or invalid one changes nothing.
    pub(crate) fn resolve_remote_pick(&mut self, session: Uuid, answer: &PresentationPickAnswer) {
        self.modals.resolve_remote_pick(answer, session);
    }

    /// Sends what the book queued: each origin pick that ended here is taken down on its presenter, and each replica
    /// answered here goes back to its origin.
    pub(crate) fn flush_session_picks(&mut self) {
        let (dismissals, answers) = self.modals.session_picks.take_outbox();
        for (session, reference) in dismissals {
            self.remote.hub.send_to_presenter(Body::PickDismiss(reference), session);
        }
        for (session, answer) in answers {
            self.answer_origin(session, Body::PickResolve(answer));
        }
    }

    // MARK: - the viewer

    /// Shows a pick `session`'s origin handed over in the window holding the attached row (upstream's
    /// `showReplicaPick`). A busy window refuses it, unless the origin is headless and has no screen to take it back
    /// to: there it waits. False when it cannot be shown, which the caller reports as a refusal.
    pub(crate) fn show_replica_pick(&mut self, request: &PresentationPick, session: Uuid) -> bool {
        let Some(window) = self.library().window_holding_session(session) else {
            return false;
        };
        let Some(headless) = self
            .presented_session(session)
            .and_then(|found| found.remote_presentation())
            .map(|state| state.binding.origin_headless)
        else {
            return false;
        };
        if (!headless && self.modal_pending(window)) || !self.modals.open_replica_pick(request, session, window) {
            return false;
        }
        if request.follow == Some(true) {
            self.request_window(WindowRequest::Raise(window));
            self.take_frontmost(window);
        }
        self.sync_modals();
        true
    }

    /// Takes a replica down without answering, once its origin ended the pick.
    pub(crate) fn dismiss_replica_pick(&mut self, reference: &PresentationPickRef, session: Uuid) {
        self.modals.dismiss_replica_pick(reference, session);
        self.sync_modals();
    }
}

#[cfg(test)]
#[path = "pick_tests.rs"]
mod tests;
