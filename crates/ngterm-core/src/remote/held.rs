//! What a session holds for a presenter (upstream's `AppStore+Interactions.swift` and agterm-server's
//! `presenterGranted`): its ask, pick, pages and overlay jobs, read back as the session node's `interactions`, and on a
//! headless origin handed to the next presenter that can show them.
//!
//! A headless origin has no screen, so what opens while no viewer presents a session waits there, unpresented. A
//! presenter that lists [`HELD_KINDS`] is handed all of it at once when it takes the session: one batch of pending
//! chunks and a commit ([`super::hub::PresentationHub::commit_batch`]), which it applies together or not at all. An older presenter
//! sees only what opens while it holds the session. A page with no viewer is parked as before (`pages-survive`) and
//! handed on by its own rules; inside the batch when the presenter shows pages.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ngterm_proto::{ControlInteraction, ControlTree};
use uuid::Uuid;

use super::frames::{self, Body, PresentationAsk, PresentationPane};
use super::hub::HELD_KINDS;
use crate::Core;
use crate::html::HtmlSource;
use crate::modal::{INTERACTION_CLAIMED, INTERACTION_OFFERED, INTERACTION_UNPRESENTED};
use crate::model::{RemoteOverlaySlot, Session};
use crate::overlay_jobs::OverlayJobState;

/// `instant` as epoch seconds, read against `now` on the monotonic clock.
fn epoch_seconds(instant: Instant, now: Instant) -> f64 {
    let wall = SystemTime::now();
    let at = match instant.checked_duration_since(now) {
        Some(ahead) => wall + ahead,
        None => wall - now.saturating_duration_since(instant),
    };
    at.duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO).as_secs_f64()
}

fn interaction(kind: &str, id: &str, state: &str, deadline: Option<f64>) -> ControlInteraction {
    ControlInteraction {
        kind: kind.to_owned(),
        id: id.to_owned(),
        state: state.to_owned(),
        deadline,
    }
}

impl Core {
    /// `session`'s `interactions` read-back (upstream's `interactionNodes`): its ask while a presenter holds it or, on a
    /// headless origin, while one is awaited; its pick likewise; and every page and overlay job in its slots. `None`
    /// when it holds nothing.
    pub(crate) fn interactions(&self, session: &Session) -> Option<Vec<ControlInteraction>> {
        let now = Instant::now();
        let deadline = |id: &str| self.modals.deadline(id).map(|at| epoch_seconds(at, self.modals.now()));
        let mut nodes = Vec::new();
        if let Some(pending) = session.ask().filter(|pending| pending.replica.is_none())
            && (pending.remote_owner.is_some() || self.presenter_only())
        {
            let state = if pending.remote_owner.is_some() {
                INTERACTION_OFFERED
            } else {
                INTERACTION_UNPRESENTED
            };
            nodes.push(interaction("ask", &pending.ask.id, state, deadline(&pending.ask.id)));
        }
        nodes.extend(self.modals.session_picks.interaction(session.id(), deadline));
        for slot in session.remote_overlays().slots() {
            nodes.extend(self.slot_interaction(slot, now));
        }
        (!nodes.is_empty()).then_some(nodes)
    }

    fn slot_interaction(&self, slot: &RemoteOverlaySlot, now: Instant) -> Option<ControlInteraction> {
        if let Some(page) = &slot.page {
            let (state, deadline) = if page.parked {
                (INTERACTION_UNPRESENTED, None)
            } else if page.claimed {
                (INTERACTION_CLAIMED, None)
            } else {
                // a url page loads with no channel, so only a channel page has a claim to wait for
                let channel = !matches!(page.overlay.source, HtmlSource::Url(_));
                (
                    INTERACTION_OFFERED,
                    channel.then(|| epoch_seconds(page.claim_deadline, now)),
                )
            };
            return Some(interaction("page", &slot.job, state, deadline));
        }
        let (state, deadline) = match self.jobs.jobs.job(&slot.job)?.state {
            OverlayJobState::Unpresented => (INTERACTION_UNPRESENTED, None),
            OverlayJobState::Unclaimed { deadline } => (INTERACTION_OFFERED, Some(epoch_seconds(deadline, now))),
            OverlayJobState::Claimed { deadline } => (INTERACTION_CLAIMED, Some(epoch_seconds(deadline, now))),
            OverlayJobState::Running => (INTERACTION_CLAIMED, None),
            OverlayJobState::Finished(_) => return None,
        };
        Some(interaction("job", &slot.job, state, deadline))
    }

    /// The `interactions` read-back of each session in `tree`.
    pub(crate) fn project_interactions(&self, tree: &mut ControlTree) {
        for node in tree
            .workspaces
            .iter_mut()
            .flat_map(|workspace| workspace.sessions.iter_mut())
        {
            node.interactions = frames::parse_uuid(&node.id)
                .and_then(|id| self.presented_session(id))
                .and_then(|found| self.interactions(found));
        }
    }

    /// A viewer just took `session`'s presenter role (upstream's `presenterGranted`): on a headless origin, a presenter
    /// listing [`HELD_KINDS`] is handed what the session holds, at once.
    pub(crate) fn presenter_granted(&mut self, session: Uuid) {
        let takes_held = HELD_KINDS
            .iter()
            .all(|kind| self.remote.hub.presenter_supports(kind, session));
        if !self.presenter_only() || !takes_held {
            return;
        }
        self.remote.hub.open_batch(session);
        self.offer_held_ask(session);
        self.offer_held_pick(session);
        self.hand_on_parked_pages(session);
        self.offer_held_jobs(session);
        self.remote.hub.commit_batch(session);
    }

    /// Hands `session`'s unpresented ask to its presenter under the current generation (upstream's
    /// `offerPendingAsk`), ending it first when its deadline passed, so an expired ask is never shown. It stays
    /// unpresented when the presenter cannot show it.
    fn offer_held_ask(&mut self, session: Uuid) {
        let Some(pending) = self
            .presented_session(session)
            .and_then(Session::ask)
            .filter(|pending| pending.remote_owner.is_none() && pending.replica.is_none())
        else {
            return;
        };
        let (ask, pane) = (pending.ask.clone(), pending.pane_identity);
        if self.modals.expired(&ask.id) {
            drop(crate::commands::ask::cancel(self, &ask.id, None));
            return;
        }
        let owner = self.remote.hub.presenter_generation(session);
        let request = PresentationAsk::new(&ask, pane.map(PresentationPane::Identity), owner);
        if !super::handoff::presenter_can_show(&self.remote.hub, &request, session) {
            return;
        }
        if let Some(store) = self.library_mut().store_for_session_mut(session) {
            store.hand_session_ask_over(session, owner);
        }
        self.remote.hub.send_to_presenter(Body::AskRequest(request), session);
    }
}
