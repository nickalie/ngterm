//! The picks bound to a session rather than to a window (upstream's `SessionPickBook` in `SessionPick.swift`).
//!
//! On an origin each one is drawn by the session's presenter or in the modal slot of the session's own window, and its
//! outcome stays readable after it ends. On a viewer the book maps the replicas this machine draws for an origin back
//! to their stream. What must reach a presenter or an origin waits in the book's outbox until the core sends it
//! ([`crate::Core::flush_session_picks`]).

use std::collections::{HashMap, VecDeque};

use ngterm_proto::tree::ControlSessionPick;
use ngterm_proto::{ControlPickOutcome, ControlPickResult};
use uuid::Uuid;

use super::{Modals, PendingPick};
use crate::remote::frames::{PresentationPick, PresentationPickAnswer, PresentationPickRef};

/// Session pick results kept after they ended, as the window picks' app-wide cap.
const RETAINED_LIMIT: usize = 32;

/// `pick already pending`: the session, or the window slot a pick needs, holds one.
pub const PICK_ALREADY_PENDING: &str = "pick already pending";
/// An interaction's state while a headless origin holds it for a presenter (upstream's `ControlInteraction.unpresented`).
pub const INTERACTION_UNPRESENTED: &str = "unpresented";
/// An interaction's state while it is handed to a presenter (upstream's `ControlInteraction.offered`).
pub const INTERACTION_OFFERED: &str = "offered";
/// An interaction's state once its helper or channel holds it (upstream's `ControlInteraction.claimed`).
pub const INTERACTION_CLAIMED: &str = "claimed";

/// Where a session's pick in flight is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// By the session's presenter, handed over under that presenter generation.
    Presenter { owner: i64 },
    /// In this window's modal slot.
    Local(Uuid),
    /// Held by a headless origin until a presenter can show it.
    Unpresented,
}

#[derive(Debug)]
struct Entry {
    session: Uuid,
    pick: PendingPick,
    placement: Placement,
    window: Option<Uuid>,
    /// `--follow`, which each presenter it is offered to is asked again.
    follow: bool,
}

#[derive(Debug, Clone, Copy)]
struct Replica {
    session: Uuid,
    owner: i64,
    window: Uuid,
}

#[derive(Debug)]
struct Retained {
    id: String,
    result: ControlPickResult,
    window: Option<Uuid>,
}

/// What the core sends once a change settled: presenters' dismissals and origins' answers, each by session.
pub type Outbox = (Vec<(Uuid, PresentationPickRef)>, Vec<(Uuid, PresentationPickAnswer)>);

/// The session-bound picks, their retained outcomes and the replicas shown for origins.
#[derive(Debug, Default)]
pub struct SessionPicks {
    entries: HashMap<String, Entry>,
    retained: VecDeque<Retained>,
    replicas: HashMap<String, Replica>,
    dismissals: Vec<(Uuid, PresentationPickRef)>,
    answers: Vec<(Uuid, PresentationPickAnswer)>,
}

impl SessionPicks {
    fn entry_of(&self, session: Uuid) -> Option<(&String, &Entry)> {
        self.entries.iter().find(|(_, entry)| entry.session == session)
    }

    /// Where `session`'s pick in flight is drawn; `None` when it has none.
    #[must_use]
    pub fn placement(&self, session: Uuid) -> Option<Placement> {
        self.entry_of(session).map(|(_, entry)| entry.placement)
    }

    /// Books `pick` as drawn by `session`'s presenter under `owner`; false when the session already has one.
    pub(crate) fn open_remote(&mut self, pick: PendingPick, session: Uuid, owner: i64, window: Option<Uuid>) -> bool {
        if self.placement(session).is_some() {
            return false;
        }
        let entry = Entry {
            session,
            pick,
            placement: Placement::Presenter { owner },
            window,
            follow: false,
        };
        self.entries.insert(entry.pick.id.clone(), entry);
        true
    }

    /// Books `pick` for `session` on a headless origin, which holds it until a presenter can show it (upstream's
    /// `openUnpresented`); false when the session already has one.
    pub(crate) fn open_unpresented(&mut self, pick: PendingPick, session: Uuid, follow: bool) -> bool {
        if self.placement(session).is_some() {
            return false;
        }
        let entry = Entry {
            session,
            pick,
            placement: Placement::Unpresented,
            window: None,
            follow,
        };
        self.entries.insert(entry.pick.id.clone(), entry);
        true
    }

    /// `session`'s unpresented pick and its `--follow`.
    #[must_use]
    pub(crate) fn unpresented(&self, session: Uuid) -> Option<(&PendingPick, bool)> {
        self.entry_of(session)
            .filter(|(_, entry)| entry.placement == Placement::Unpresented)
            .map(|(_, entry)| (&entry.pick, entry.follow))
    }

    /// Marks `session`'s unpresented pick as handed to its presenter under `owner` (upstream's `offer`).
    pub(crate) fn offer(&mut self, session: Uuid, owner: i64) {
        if let Some(entry) = self
            .entries
            .values_mut()
            .find(|entry| entry.session == session && entry.placement == Placement::Unpresented)
        {
            entry.placement = Placement::Presenter { owner };
        }
    }

    /// Takes `session`'s presenter-held pick back as unpresented after its presenter was lost or refused it
    /// (`refused`, which must name it under its owner); an answer from that presenter is then stale (upstream's
    /// `returnToPending`).
    pub(crate) fn return_to_pending(&mut self, session: Uuid, refused: Option<&PresentationPickRef>) {
        let Some(entry) = self.entries.values_mut().find(|entry| entry.session == session) else {
            return;
        };
        let Placement::Presenter { owner } = entry.placement else {
            return;
        };
        if refused.is_some_and(|reference| reference.id != entry.pick.id || reference.owner != owner) {
            return;
        }
        entry.placement = Placement::Unpresented;
    }

    /// `session`'s pick as `interactions` lists it while a presenter holds it or one is awaited, with its deadline
    /// from `deadline`; `None` for none or one drawn in a window here (upstream's `interaction`).
    pub(crate) fn interaction(
        &self,
        session: Uuid,
        deadline: impl Fn(&str) -> Option<f64>,
    ) -> Option<ngterm_proto::ControlInteraction> {
        let (id, entry) = self.entry_of(session)?;
        let state = match entry.placement {
            Placement::Presenter { .. } => INTERACTION_OFFERED,
            Placement::Unpresented => INTERACTION_UNPRESENTED,
            Placement::Local(_) => return None,
        };
        Some(ngterm_proto::ControlInteraction {
            kind: "pick".to_owned(),
            id: id.clone(),
            state: state.to_owned(),
            deadline: deadline(id),
        })
    }

    /// Pick `id`'s outcome: pending while booked, then the retained answer.
    #[must_use]
    pub fn result(&self, id: &str) -> Option<ControlPickResult> {
        if self.entries.contains_key(id) {
            return Some(ControlPickResult::outcome(ControlPickOutcome::Pending));
        }
        self.retained
            .iter()
            .rev()
            .find(|retained| retained.id == id)
            .map(|retained| retained.result.clone())
    }

    /// The window pick `id` was opened for, booked or retained; `None` for an unknown id or one with no window.
    #[must_use]
    pub fn window_of(&self, id: &str) -> Option<Uuid> {
        match self.entries.get(id) {
            Some(entry) => entry.window,
            None => self
                .retained
                .iter()
                .rev()
                .find(|retained| retained.id == id)
                .and_then(|retained| retained.window),
        }
    }

    /// `session`'s pick read-back: its own pick, or the replica this machine draws for its origin.
    #[must_use]
    pub fn node(&self, session: Uuid) -> Option<ControlSessionPick> {
        if let Some((id, entry)) = self.entry_of(session) {
            let remote = matches!(entry.placement, Placement::Presenter { .. });
            return Some(ControlSessionPick {
                id: id.clone(),
                remote: remote.then_some(true),
                replica: None,
            });
        }
        self.replicas
            .iter()
            .find(|(_, replica)| replica.session == session)
            .map(|(id, _)| ControlSessionPick {
                id: id.clone(),
                remote: None,
                replica: Some(true),
            })
    }

    /// The sessions the book holds a pick or a replica for.
    pub(crate) fn sessions(&self) -> Vec<Uuid> {
        let mut sessions: Vec<Uuid> = self
            .entries
            .values()
            .map(|entry| entry.session)
            .chain(self.replicas.values().map(|replica| replica.session))
            .collect();
        sessions.sort_unstable();
        sessions.dedup();
        sessions
    }

    /// Takes what waits to be sent.
    pub(crate) fn take_outbox(&mut self) -> Outbox {
        (std::mem::take(&mut self.dismissals), std::mem::take(&mut self.answers))
    }

    /// A window's slot let pick `id` go (an answer, Esc, a cancel, the window's teardown): it settles the booked pick
    /// or the replica shown there.
    fn controller_resolved(&mut self, id: &str, result: &ControlPickResult) {
        if let Some(replica) = self.replicas.remove(id) {
            self.answers
                .push((replica.session, PresentationPickAnswer::new(id, replica.owner, result)));
        }
        if self
            .entries
            .get(id)
            .is_some_and(|entry| matches!(entry.placement, Placement::Local(_)))
        {
            self.finish(id, result.clone());
        }
    }

    /// Detaches the replicas shown in `window` without answering, before its slot is cancelled: the origin takes them
    /// back when the window's streams end.
    fn window_closing(&mut self, window: Uuid) {
        self.replicas.retain(|_, replica| replica.window != window);
    }

    fn finish(&mut self, id: &str, result: ControlPickResult) {
        let window = self.entries.remove(id).and_then(|entry| entry.window);
        self.retained.retain(|retained| retained.id != id);
        self.retained.push_back(Retained {
            id: id.to_owned(),
            result,
            window,
        });
        while self.retained.len() > RETAINED_LIMIT {
            self.retained.pop_front();
        }
    }
}

impl Modals {
    /// Shows `pick` in window `window`'s slot and books it to `session`; refused with what holds the session or the
    /// slot.
    pub(crate) fn open_session_pick_local(
        &mut self,
        pick: PendingPick,
        session: Uuid,
        window: Uuid,
    ) -> Result<(), &'static str> {
        if self.session_picks.placement(session).is_some() {
            return Err(PICK_ALREADY_PENDING);
        }
        let id = pick.id.clone();
        self.open_pick(window, pick.clone())?;
        self.session_picks.entries.insert(
            id,
            Entry {
                session,
                pick,
                placement: Placement::Local(window),
                window: Some(window),
                follow: false,
            },
        );
        Ok(())
    }

    /// Ends pick `id` from this machine: its presenter is told to take it down, a local one is cancelled in its window.
    /// False for an id the book never held; a retained one is a successful no-op.
    pub(crate) fn cancel_session_pick(&mut self, id: &str) -> bool {
        let Some(entry) = self.session_picks.entries.get(id) else {
            return self.session_picks.result(id).is_some();
        };
        let cancelled = ControlPickResult::outcome(ControlPickOutcome::Cancelled);
        match entry.placement {
            Placement::Local(window) if self.slot_pick_is(window, id) => self.resolve_pick(window, cancelled),
            Placement::Local(_) | Placement::Unpresented => self.session_picks.finish(id, cancelled),
            Placement::Presenter { owner } => {
                let session = entry.session;
                self.session_picks.finish(id, cancelled);
                self.session_picks.dismissals.push((
                    session,
                    PresentationPickRef {
                        id: id.to_owned(),
                        owner,
                    },
                ));
            }
        }
        true
    }

    /// Applies a presenter's answer; ignored unless it is for the pick `session`'s presenter holds under that owner and
    /// names a row or a custom answer the pick allows. One past the pick's deadline resolves cancelled.
    pub(crate) fn resolve_remote_pick(&mut self, answer: &PresentationPickAnswer, session: Uuid) {
        let Some(entry) = self.session_picks.entries.get(&answer.id) else {
            return;
        };
        if entry.session != session || entry.placement != (Placement::Presenter { owner: answer.owner }) {
            return;
        }
        let Some(result) = entry
            .pick
            .validated_result(answer.item.as_deref(), answer.query.as_deref())
        else {
            return;
        };
        let result = if self.expired(&answer.id) {
            ControlPickResult::outcome(ControlPickOutcome::Cancelled)
        } else {
            result
        };
        self.session_picks.finish(&answer.id, result);
    }

    /// Moves `session`'s presenter-held pick into window `window`'s slot after the presenter refused it (`refused`,
    /// which must name it under its owner) or went away (`None`). With no window, or a slot that holds another pick or
    /// ask, the pick ends cancelled and stays readable, and the other modal is left alone.
    pub(crate) fn hand_back_session_pick(
        &mut self,
        session: Uuid,
        refused: Option<&PresentationPickRef>,
        window: Option<Uuid>,
    ) {
        let Some((id, entry)) = self.session_picks.entry_of(session) else {
            return;
        };
        let Placement::Presenter { owner } = entry.placement else {
            return;
        };
        if refused.is_some_and(|reference| reference.id != *id || reference.owner != owner) {
            return;
        }
        let (id, pick) = (id.clone(), entry.pick.clone());
        let shown = window.filter(|window| self.open_pick(*window, pick).is_ok());
        match (shown, self.session_picks.entries.get_mut(&id)) {
            (Some(window), Some(entry)) => {
                entry.placement = Placement::Local(window);
                entry.window = Some(window);
            }
            _ => self
                .session_picks
                .finish(&id, ControlPickResult::outcome(ControlPickOutcome::Cancelled)),
        }
    }

    /// Settles everything bound to `session` once it left its store: its own pick is cancelled, and a replica is taken
    /// down without answering, since its origin takes the pick back when the stream ends.
    pub(crate) fn session_ended(&mut self, session: Uuid) {
        let ids: Vec<String> = self
            .session_picks
            .entries
            .iter()
            .filter(|(_, entry)| entry.session == session)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            self.cancel_session_pick(&id);
        }
        self.drop_replica_picks(session);
    }

    /// Shows the pick an origin handed over in window `window`'s slot; false for an id this machine already holds or a
    /// slot already held.
    pub(crate) fn open_replica_pick(&mut self, request: &PresentationPick, session: Uuid, window: Uuid) -> bool {
        if self.session_picks.replicas.contains_key(&request.id) || self.open_pick(window, request.pending()).is_err() {
            return false;
        }
        self.session_picks.replicas.insert(
            request.id.clone(),
            Replica {
                session,
                owner: request.owner,
                window,
            },
        );
        true
    }

    /// Takes the replica `reference` names down without answering, once its origin ended the pick. One under another
    /// owner, or no longer shown, changes nothing.
    pub(crate) fn dismiss_replica_pick(&mut self, reference: &PresentationPickRef, session: Uuid) {
        let matches = self
            .session_picks
            .replicas
            .get(&reference.id)
            .filter(|replica| replica.session == session && replica.owner == reference.owner)
            .copied();
        if let Some(replica) = matches {
            self.close_replica_pick(&reference.id, replica.window);
        }
    }

    /// Takes down `session`'s replicas without answering: its stream is gone, and the origin takes the pick back.
    pub(crate) fn drop_replica_picks(&mut self, session: Uuid) {
        let shown: Vec<(String, Uuid)> = self
            .session_picks
            .replicas
            .iter()
            .filter(|(_, replica)| replica.session == session)
            .map(|(id, replica)| (id.clone(), replica.window))
            .collect();
        for (id, window) in shown {
            self.close_replica_pick(&id, window);
        }
    }

    fn close_replica_pick(&mut self, id: &str, window: Uuid) {
        self.session_picks.replicas.remove(id);
        if self.slot_pick_is(window, id) {
            self.resolve_pick(window, ControlPickResult::outcome(ControlPickOutcome::Cancelled));
        }
    }

    fn slot_pick_is(&self, window: Uuid, id: &str) -> bool {
        self.controller(window)
            .and_then(super::PickController::pending)
            .is_some_and(|pick| pick.id == id)
    }

    /// The book's side of a window slot that let pick `id` go.
    pub(super) fn session_pick_resolved(&mut self, id: &str, result: &ControlPickResult) {
        self.session_picks.controller_resolved(id, result);
    }

    /// The book's side of window `window` closing.
    pub(super) fn session_picks_window_closing(&mut self, window: Uuid) {
        self.session_picks.window_closing(window);
    }
}

#[cfg(test)]
#[path = "session_pick_tests.rs"]
mod tests;
