//! Overlays handed to the viewer presenting a session, on the origin (upstream's `RemoteOverlays.swift` and the model
//! half of `AppStore+RemoteOverlay.swift`).
//!
//! A slot reserved for a remote job, or for a page the presenter shows (a [`RemotePage`], upstream's
//! `AppStore+RemotePage.swift` model half), covers nothing here, but a second overlay cannot open on it until the job
//! or the page ends. What a change needs beyond the model (telling the presenter, cancelling the job, settling the
//! page's outcome) is queued as a [`RemoteOverlayChange`] for the core, as the store cannot reach the presentation
//! hub, the job table or the page outcomes.

use std::time::Instant;

use uuid::Uuid;

use super::{AppStore, OverlayPane, Session};
use crate::html::{HtmlOverlay, HtmlPageInfo};
use crate::remote::frames::{PresentationPageAction, PresentationPageCommand, PresentationPageState};

/// A session overlay slot reserved for a job a viewer presents (upstream's `RemoteOverlaySlot`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteOverlaySlot {
    pub job: String,
    /// The pane the slot covers, `None` for the session-wide slot. Follows its pane across a swap.
    pub pane: Option<OverlayPane>,
    /// The presenter generation the job was handed to; only that stream can still reach its surface.
    pub owner: i64,
    /// The size requested for a session-wide overlay, `None` for full. What the viewer applied is not known.
    pub size_percent: Option<i64>,
    /// The viewer's surface stays up after the program ends (`--wait`), which keeps the slot held until it closes.
    pub wait: bool,
    /// The program ended, its result recorded, while a held surface keeps the slot.
    pub ended: bool,
    /// What the slot holds instead of a program; `job` is then the page's id.
    pub page: Option<RemotePage>,
}

/// A visibility sent to the presenter and not yet echoed back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VisibilityRequest {
    hidden: bool,
    seq: i64,
}

/// A page a reserved slot holds (upstream's `RemotePage`): the origin's record of what it opened, and what the
/// presenter last reported it shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemotePage {
    pub overlay: HtmlOverlay,
    /// Claims the page channel once; never reported.
    pub token: String,
    /// Until when a file page's channel may be claimed.
    pub claim_deadline: Instant,
    pub claimed: bool,
    /// The visibility the presenter last reported.
    pub hidden: bool,
    requested: Option<VisibilityRequest>,
    last_seq: i64,
}

impl RemotePage {
    #[must_use]
    pub fn new(overlay: HtmlOverlay, token: String, claim_deadline: Instant) -> Self {
        Self {
            overlay,
            token,
            claim_deadline,
            claimed: false,
            hidden: false,
            requested: None,
            last_seq: 0,
        }
    }

    /// What a toggle and a visibility reply go by (upstream's `effectiveHidden`): an unacknowledged request, else the
    /// presenter's last report, so a hide made on the presenter turns the next toggle into a show.
    #[must_use]
    pub fn effective_hidden(&self) -> bool {
        self.requested.map_or(self.hidden, |requested| requested.hidden)
    }

    /// The next command's sequence number.
    pub(crate) fn next_seq(&mut self) -> i64 {
        self.last_seq += 1;
        self.last_seq
    }

    /// Records a visibility request and gives its sequence number.
    pub(crate) fn request(&mut self, hidden: bool) -> i64 {
        let seq = self.next_seq();
        self.requested = Some(VisibilityRequest { hidden, seq });
        seq
    }

    /// Takes the presenter's report (upstream's `apply`); whether the reported visibility changed. A report echoing
    /// the pending request's sequence settles it; an older one leaves the request pending.
    pub(crate) fn apply(&mut self, state: &PresentationPageState) -> bool {
        let was_hidden = self.hidden;
        self.overlay.load_state = state.load;
        self.overlay.load_error.clone_from(&state.error);
        self.overlay.current = state.address.as_ref().map(|address| HtmlPageInfo {
            page: address.clone(),
            title: state.title.clone(),
            can_go_back: state.can_go_back,
            can_go_forward: state.can_go_forward,
        });
        self.hidden = state.hidden;
        if self
            .requested
            .is_some_and(|requested| state.applied_seq >= requested.seq)
        {
            self.requested = None;
        }
        was_hidden != self.hidden
    }
}

/// A session's remote overlay state on the origin: the reserved slots, and per slot the failure a remote job ended
/// with, which `session.overlay.result` reports until the next open there (upstream's `RemoteOverlays`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RemoteOverlays {
    slots: Vec<RemoteOverlaySlot>,
    failures: [Option<String>; 3],
}

/// The failure index of a slot: the session-wide one, then the panes.
fn failure_index(pane: Option<OverlayPane>) -> usize {
    match pane {
        None => 0,
        Some(OverlayPane::Left) => 1,
        Some(OverlayPane::Right) => 2,
    }
}

impl RemoteOverlays {
    #[must_use]
    pub fn slots(&self) -> &[RemoteOverlaySlot] {
        &self.slots
    }

    #[must_use]
    pub fn slot(&self, pane: Option<OverlayPane>) -> Option<&RemoteOverlaySlot> {
        self.slots.iter().find(|slot| slot.pane == pane)
    }

    #[must_use]
    pub fn slot_of_job(&self, job: &str) -> Option<&RemoteOverlaySlot> {
        self.slots.iter().find(|slot| slot.job == job)
    }

    /// Runs `change` on the page held under `job` (upstream's `updatePage`); `None` when no slot holds one.
    pub(crate) fn update_page<R>(&mut self, job: &str, change: impl FnOnce(&mut RemotePage) -> R) -> Option<R> {
        self.slots
            .iter_mut()
            .find(|slot| slot.job == job)
            .and_then(|slot| slot.page.as_mut())
            .map(change)
    }

    #[must_use]
    pub fn failure(&self, pane: Option<OverlayPane>) -> Option<&str> {
        self.failures[failure_index(pane)].as_deref()
    }

    pub(crate) fn reserve(&mut self, slot: RemoteOverlaySlot) {
        self.failures[failure_index(slot.pane)] = None;
        self.slots.push(slot);
    }

    /// The program ended: the slot is freed, unless a held surface still occupies it on the viewer.
    pub(crate) fn end(&mut self, job: &str) -> Option<RemoteOverlaySlot> {
        let index = self.slots.iter().position(|slot| slot.job == job)?;
        let slot = self.slots[index].clone();
        if slot.wait {
            self.slots[index].ended = true;
        } else {
            self.slots.remove(index);
        }
        Some(slot)
    }

    /// The viewer's surface is gone: a slot whose program ended is freed; a running one is freed when it ends.
    pub(crate) fn surface_gone(&mut self, job: &str) {
        let Some(index) = self.slots.iter().position(|slot| slot.job == job) else {
            return;
        };
        if self.slots[index].ended {
            self.slots.remove(index);
        } else {
            self.slots[index].wait = false;
        }
    }

    pub(crate) fn resize(&mut self, job: &str, size_percent: Option<i64>) {
        if let Some(slot) = self.slots.iter_mut().find(|slot| slot.job == job) {
            slot.size_percent = size_percent;
        }
    }

    pub(crate) fn remove(&mut self, job: &str) {
        self.slots.retain(|slot| slot.job != job);
    }

    /// The origin swapped its panes: each pane's slot and result go with it.
    pub(crate) fn swap_panes(&mut self) {
        for slot in &mut self.slots {
            slot.pane = slot.pane.map(|pane| match pane {
                OverlayPane::Left => OverlayPane::Right,
                OverlayPane::Right => OverlayPane::Left,
            });
        }
        self.failures.swap(1, 2);
    }

    /// The split was promoted to the primary pane, whose own slot is already dropped.
    pub(crate) fn promote_right(&mut self) {
        for slot in &mut self.slots {
            if slot.pane == Some(OverlayPane::Right) {
                slot.pane = Some(OverlayPane::Left);
            }
        }
        self.failures[1] = self.failures[2].take();
    }

    pub(crate) fn record_failure(&mut self, name: &str, pane: Option<OverlayPane>) {
        self.failures[failure_index(pane)] = Some(name.to_owned());
    }

    /// A local open on the slot starts a new result, as the local exit code does.
    pub(crate) fn clear_failure(&mut self, pane: Option<OverlayPane>) {
        self.failures[failure_index(pane)] = None;
    }
}

/// What the model changed about a slot a viewer holds, for the core to carry out, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteOverlayChange {
    /// A program overlay closed: the presenter takes its surface down and the job is cancelled.
    Closed { session: Uuid, job: String },
    /// A page closed here: the presenter takes it down, and its outcome is dismissed unless it was answered.
    PageClosed { session: Uuid, page: Uuid },
    /// A page's visibility was asked for (upstream's `sendRemotePageCommand` from `setOverlaysHidden`).
    PageCommand {
        session: Uuid,
        command: PresentationPageCommand,
    },
}

/// How a remote job ended, as the slot's result records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteOverlayOutcome {
    Exited(i32),
    /// A failure name (`canceled`, `launch-failed`, `unknown`).
    Failed(String),
}

impl Session {
    /// The overlay slots a viewer's presenter holds and the outcomes of remote jobs.
    #[must_use]
    pub fn remote_overlays(&self) -> &RemoteOverlays {
        &self.remote_overlays
    }

    /// Whether the slot `pane` addresses is taken by an overlay here (upstream's `localOverlayHolds`).
    #[must_use]
    pub fn local_overlay_holds(&self, pane: Option<OverlayPane>) -> bool {
        match pane {
            None => self.cover_overlay_active(),
            Some(pane) => self.pane_overlay(pane).is_some(),
        }
    }
}

impl AppStore {
    /// Reserves the slot `slot.pane` of session `session` for a remote job and clears that slot's last result, as a
    /// local open clears it. False for an unknown session.
    pub fn reserve_remote_overlay(&mut self, session: Uuid, slot: RemoteOverlaySlot) -> bool {
        let Some(found) = self.session_mut(session) else {
            return false;
        };
        let pane = slot.pane;
        found.remote_overlays.reserve(slot);
        match pane {
            Some(pane) => found.pane_overlays.slot_mut(pane).exit_code = None,
            None => found.overlay.exit_code = None,
        }
        self.schedule_tree_changed();
        true
    }

    /// Records a remote job's first outcome into its slot's result, and frees the slot unless a held `--wait` surface
    /// still occupies it on the viewer (upstream's `finishRemoteOverlay`).
    pub fn finish_remote_overlay(&mut self, session: Uuid, job: &str, outcome: &RemoteOverlayOutcome) {
        let Some(slot) = self
            .tracking_remote_slots(session, |overlays| overlays.end(job))
            .flatten()
        else {
            return;
        };
        let Some(found) = self.session_mut(session) else {
            return;
        };
        match outcome {
            RemoteOverlayOutcome::Exited(code) => match slot.pane {
                Some(pane) => found.pane_overlays.slot_mut(pane).exit_code = Some(*code),
                None => found.overlay.exit_code = Some(*code),
            },
            RemoteOverlayOutcome::Failed(name) => found.remote_overlays.record_failure(name, slot.pane),
        }
    }

    /// Closes the remote overlay on a slot (upstream's `closeRemoteOverlay`): its surface counts as gone and the core
    /// is asked to take the presenter's surface down and cancel the job. It reports the request, not the program's end.
    /// False when no viewer holds the slot.
    pub fn close_remote_overlay(&mut self, session: Uuid, pane: Option<OverlayPane>) -> bool {
        let Some(found) = self.session_mut(session) else {
            return false;
        };
        let Some(slot) = found.remote_overlays.slot(pane) else {
            return false;
        };
        let job = slot.job.clone();
        if slot.page.is_some() {
            if let Some(page) = self.end_remote_page(session, &job) {
                self.remote_overlay_changes
                    .push(RemoteOverlayChange::PageClosed { session, page });
            }
            return true;
        }
        self.tracking_remote_slots(session, |overlays| overlays.surface_gone(&job));
        self.remote_overlay_changes
            .push(RemoteOverlayChange::Closed { session, job });
        true
    }

    /// Drops the reservation of the page held under `job` (upstream's `endRemotePage`, its outcome left to the
    /// caller); the page's id, `None` when no slot holds that page.
    pub fn end_remote_page(&mut self, session: Uuid, job: &str) -> Option<Uuid> {
        let page = self
            .session(session)?
            .remote_overlays
            .slot_of_job(job)?
            .page
            .as_ref()?
            .overlay
            .id;
        self.tracking_remote_slots(session, |overlays| overlays.remove(job));
        Some(page)
    }

    /// Applies what the presenter reported about page `state.page` (upstream's `receiveRemotePageFrame` state arm):
    /// the tree changes only when the reported visibility did.
    pub fn apply_remote_page_state(&mut self, session: Uuid, state: &PresentationPageState) {
        let changed = self
            .session_mut(session)
            .and_then(|found| found.remote_overlays.update_page(&state.page, |page| page.apply(state)))
            .unwrap_or(false);
        if changed {
            self.schedule_tree_changed();
        }
    }

    /// Numbers a command for the page in `pane`'s slot (upstream's `sendRemotePageCommand` model half): a hide or a
    /// show is recorded as the pending visibility. The page's id on the wire and the command's number, `None` without
    /// a page there.
    pub(crate) fn number_remote_page_command(
        &mut self,
        session: Uuid,
        pane: Option<OverlayPane>,
        action: PresentationPageAction,
    ) -> Option<(String, i64)> {
        let found = self.session_mut(session)?;
        let job = found
            .remote_overlays
            .slot(pane)
            .filter(|slot| slot.page.is_some())?
            .job
            .clone();
        let seq = found.remote_overlays.update_page(&job, |page| match action {
            PresentationPageAction::Hide => page.request(true),
            PresentationPageAction::Show => page.request(false),
            _ => page.next_seq(),
        })?;
        Some((job, seq))
    }

    /// The viewer closed its surface for `job`, so a slot held only for that surface is freed.
    pub fn remote_overlay_surface_closed(&mut self, session: Uuid, job: &str) {
        self.tracking_remote_slots(session, |overlays| overlays.surface_gone(job));
    }

    /// Records the size asked of a session-wide remote overlay.
    pub fn resize_remote_overlay(&mut self, session: Uuid, job: &str, size_percent: Option<i64>) {
        if let Some(found) = self.session_mut(session) {
            found.remote_overlays.resize(job, size_percent);
        }
    }

    /// The viewer presenting `session` is gone, and every overlay it held goes with it: each program slot's surface
    /// counts as gone, and each page ends. The program jobs, whose unclaimed ones the caller cancels, and the ended
    /// pages, whose outcomes the caller settles, are returned.
    pub fn remote_overlay_presenter_lost(&mut self, session: Uuid) -> (Vec<String>, Vec<Uuid>) {
        self.tracking_remote_slots(session, |overlays| {
            let pages: Vec<(String, Uuid)> = overlays
                .slots()
                .iter()
                .filter_map(|slot| slot.page.as_ref().map(|page| (slot.job.clone(), page.overlay.id)))
                .collect();
            for (job, _) in &pages {
                overlays.remove(job);
            }
            let jobs: Vec<String> = overlays.slots().iter().map(|slot| slot.job.clone()).collect();
            for job in &jobs {
                overlays.surface_gone(job);
            }
            (jobs, pages.into_iter().map(|(_, page)| page).collect())
        })
        .unwrap_or_default()
    }

    /// The presenter refused to show `job`: no surface was ever shown, so a `--wait` slot has nothing to hold it.
    /// Whether the session holds that job.
    pub fn reject_remote_overlay(&mut self, session: Uuid, job: &str) -> bool {
        let Some(found) = self.session_mut(session) else {
            return false;
        };
        if found.remote_overlays.slot_of_job(job).is_none() {
            return false;
        }
        self.tracking_remote_slots(session, |overlays| overlays.surface_gone(job));
        true
    }

    /// `pane` is gone here: its job is ended and its slot and result dropped, since nothing is left for a late outcome
    /// to describe (upstream's `dropRemoteOverlay`).
    pub(crate) fn drop_remote_overlay(&mut self, session: Uuid, pane: OverlayPane) {
        let job = self
            .session(session)
            .and_then(|found| found.remote_overlays.slot(Some(pane)))
            .map(|slot| slot.job.clone());
        if let Some(job) = job {
            self.close_remote_overlay(session, Some(pane));
            self.tracking_remote_slots(session, |overlays| overlays.remove(&job));
        }
        if let Some(found) = self.session_mut(session) {
            found.remote_overlays.clear_failure(Some(pane));
        }
    }

    /// Ends every overlay `session` handed out before it leaves the store, so an undo brings back no reservation.
    pub(crate) fn release_remote_overlays(&mut self, session: Uuid) {
        let panes: Vec<Option<OverlayPane>> = self
            .session(session)
            .map(|found| found.remote_overlays.slots().iter().map(|slot| slot.pane).collect())
            .unwrap_or_default();
        for pane in panes {
            self.close_remote_overlay(session, pane);
        }
        if let Some(found) = self.session_mut(session) {
            found.remote_overlays = RemoteOverlays::default();
        }
    }

    /// Runs `change` on session `session`'s remote overlays and invalidates the tree when it took or freed a slot
    /// (upstream's `trackingSlots`): not every call does, as a held `--wait` slot outlives its job and a slot can be
    /// gone already. `None` for an unknown session.
    fn tracking_remote_slots<R>(&mut self, session: Uuid, change: impl FnOnce(&mut RemoteOverlays) -> R) -> Option<R> {
        let found = self.session_mut(session)?;
        let jobs = |overlays: &RemoteOverlays| -> Vec<String> {
            overlays.slots().iter().map(|slot| slot.job.clone()).collect()
        };
        let before = jobs(&found.remote_overlays);
        let result = change(&mut found.remote_overlays);
        if jobs(&found.remote_overlays) != before {
            self.schedule_tree_changed();
        }
        Some(result)
    }

    /// Takes the remote overlay changes since the last take, in order.
    pub fn take_remote_overlay_changes(&mut self) -> Vec<RemoteOverlayChange> {
        std::mem::take(&mut self.remote_overlay_changes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(job: &str, pane: Option<OverlayPane>, wait: bool) -> RemoteOverlaySlot {
        RemoteOverlaySlot {
            job: job.to_owned(),
            pane,
            owner: 1,
            size_percent: None,
            wait,
            ended: false,
            page: None,
        }
    }

    #[test]
    fn a_waiting_slot_outlives_its_program_until_its_surface_goes() {
        let mut overlays = RemoteOverlays::default();
        overlays.reserve(slot("a", None, true));
        assert!(overlays.end("a").is_some());
        assert!(
            overlays.slot(None).is_some_and(|slot| slot.ended),
            "held for the surface"
        );
        overlays.surface_gone("a");
        assert!(overlays.slot(None).is_none());
        overlays.reserve(slot("b", None, true));
        overlays.surface_gone("b");
        assert!(overlays.slot(None).is_some(), "a running job keeps its slot");
        overlays.end("b");
        assert!(
            overlays.slot(None).is_none(),
            "freed when it ends, the surface already gone"
        );
        assert!(overlays.end("missing").is_none());
    }

    fn state(hidden: bool, applied_seq: i64) -> PresentationPageState {
        let shown = crate::remote::frames::PageShown {
            address: Some("/tmp/b.html".to_owned()),
            title: Some("B".to_owned()),
            load: crate::html::HtmlLoadState::Loaded,
            ..crate::remote::frames::PageShown::default()
        };
        PresentationPageState::new("p", shown, hidden, applied_seq)
    }

    /// Upstream's `RemotePage` sequencing: a pending request answers until a report echoes its number, and a report
    /// older than the request leaves it pending.
    #[test]
    fn a_page_goes_by_its_pending_request_until_the_presenter_echoes_it() {
        let overlay = HtmlOverlay::new(
            crate::html::HtmlSource::File {
                path: "/tmp/a.html".to_owned(),
                grant_root: None,
            },
            crate::html::HtmlPageFlags::default(),
        );
        let mut page = RemotePage::new(overlay, "t".to_owned(), Instant::now());
        assert!(!page.effective_hidden());
        assert_eq!(page.request(true), 1);
        assert!(page.effective_hidden(), "the request answers before any report");
        assert!(
            !page.apply(&state(false, 0)),
            "a stale report changes nothing it reported"
        );
        assert!(page.effective_hidden(), "and leaves the request pending");
        assert_eq!(
            page.overlay.current.as_ref().map(|current| current.page.as_str()),
            Some("/tmp/b.html")
        );
        assert!(page.apply(&state(true, 1)));
        assert!(page.effective_hidden() && page.hidden);
        assert_eq!(page.next_seq(), 2);
        assert!(
            page.apply(&state(false, 1)),
            "a hide or show made on the presenter is taken"
        );
        assert!(!page.effective_hidden());
    }

    #[test]
    fn slots_and_results_follow_swaps_and_promotions() {
        let mut overlays = RemoteOverlays::default();
        overlays.reserve(slot("l", Some(OverlayPane::Left), false));
        overlays.record_failure("canceled", Some(OverlayPane::Right));
        overlays.swap_panes();
        assert_eq!(
            overlays.slot(Some(OverlayPane::Right)).map(|slot| slot.job.as_str()),
            Some("l")
        );
        assert_eq!(overlays.failure(Some(OverlayPane::Left)), Some("canceled"));
        overlays.promote_right();
        assert_eq!(
            overlays.slot(Some(OverlayPane::Left)).map(|slot| slot.job.as_str()),
            Some("l")
        );
        assert_eq!(overlays.failure(Some(OverlayPane::Left)), None);
        overlays.record_failure("unknown", None);
        overlays.reserve(slot("n", None, false));
        assert_eq!(overlays.failure(None), None, "a new reservation starts a new result");
    }
}
