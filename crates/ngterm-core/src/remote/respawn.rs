//! A parent's respawn carried to its viewers (upstream's `RemoteRespawn.swift` and `ControlServer+Respawn.swift`).
//!
//! On the origin, a parent whose exiting shell restarts while it has companions (`close_primary_pane`'s respawn)
//! keeps its pane identity, so its live-session daemon comes back under the same name. Once a different live leader
//! runs under that name, the origin publishes `pane.respawn` with the pane's new count, and every snapshot carries
//! all of the session's counts under this app run's epoch.
//!
//! On a viewer, [`RemoteRespawnBook`] joins what an origin reports with each local pane's held exit: a clean exit the
//! origin has respawned since the pane's baseline is attached again, attach-only; one with no respawn keeps its exit
//! prompt.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use uuid::Uuid;

use super::frames::{Body, PresentationRespawns};
use crate::Core;
use crate::live::daemon_name;
use crate::live::lead::LeadAttachment;
use crate::model::StatusPane;

/// How long the origin waits for a new live leader under a respawned daemon's name (upstream's `respawnLeaderWait`).
pub const RESPAWN_LEADER_WAIT: Duration = Duration::from_secs(10);
/// The pause between two listings while waiting for it.
const LEADER_POLL: Duration = Duration::from_millis(100);

/// What an origin reported about one pane: its count in one origin app run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mark {
    pub epoch: String,
    pub generation: i64,
}

/// One attach of a pane, and the mark it started against (`None` before the first snapshot).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attempt {
    pub mark: Option<Mark>,
}

/// One local pane's join (upstream's `RemoteRespawnBook.Entry`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub session: Uuid,
    pub baseline: Option<Mark>,
    pub latest: Option<Mark>,
    pub held: bool,
    pub attempt: Option<Attempt>,
    /// Attaches for `latest` that ended before reporting a role.
    pub failures: u32,
}

impl Entry {
    fn new(session: Uuid) -> Self {
        Self {
            session,
            baseline: None,
            latest: None,
            held: false,
            attempt: None,
            failures: 0,
        }
    }

    fn due(&self) -> bool {
        if !self.held || self.attempt.is_some() || self.failures >= RemoteRespawnBook::MAX_ATTEMPTS {
            return false;
        }
        let Some(latest) = &self.latest else {
            return false;
        };
        match &self.baseline {
            Some(baseline) if latest.epoch == baseline.epoch => latest.generation > baseline.generation,
            // no baseline, or another origin run: any respawn of the run is new
            _ => latest.generation > 0,
        }
    }
}

/// Joins what an origin reports about respawning a pane's shell with that pane's held exit here, one entry per LOCAL
/// pane: two rows attached to one origin pane each reattach on their own (upstream's `RemoteRespawnBook`).
///
/// Every attach of a remote pane starts an attempt carrying the latest mark known then, and only that attempt's role
/// report moves the baseline, to that mark. An entry has no expiry while its row lives; [`RemoteRespawnBook::prune`]
/// drops the entries of rows that are gone.
#[derive(Debug, Default)]
pub struct RemoteRespawnBook {
    entries: HashMap<Uuid, Entry>,
}

impl RemoteRespawnBook {
    /// Attaches tried for one generation before the pane stays on its exit prompt.
    pub const MAX_ATTEMPTS: u32 = 3;

    /// `pane`'s entry.
    #[must_use]
    pub fn entry(&self, pane: Uuid) -> Option<&Entry> {
        self.entries.get(&pane)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Records what the origin reports for `pane` of `session`; a snapshot (`seed`) becomes the baseline only when
    /// there is none, so a reconnect compares against it and recovers a respawn whose frame was missed. True when the
    /// pane should be attached again now.
    pub fn observe(&mut self, mark: Mark, pane: Uuid, session: Uuid, seed: bool) -> bool {
        let entry = self.entries.entry(pane).or_insert_with(|| Entry::new(session));
        if seed && entry.baseline.is_none() {
            entry.baseline = Some(mark.clone());
        }
        if entry.latest.as_ref() != Some(&mark) {
            entry.failures = 0;
        }
        entry.latest = Some(mark);
        entry.due()
    }

    /// Records that the pane's attach exited cleanly. An attempt that ends before reporting a role counts as a
    /// failure only when it was for the generation still latest. True when the pane should be attached again now.
    pub fn held(&mut self, pane: Uuid, session: Uuid) -> bool {
        let entry = self.entries.entry(pane).or_insert_with(|| Entry::new(session));
        if let Some(attempt) = entry.attempt.take()
            && attempt.mark == entry.latest
        {
            entry.failures += 1;
        }
        entry.held = true;
        entry.due()
    }

    /// Records any new attach of the pane, against the latest mark known now.
    pub fn attach_started(&mut self, pane: Uuid, session: Uuid) {
        let entry = self.entries.entry(pane).or_insert_with(|| Entry::new(session));
        entry.attempt = Some(Attempt {
            mark: entry.latest.clone(),
        });
        entry.held = false;
    }

    /// Moves the baseline to the mark the reporting attachment started against, never past it.
    pub fn acknowledged(&mut self, pane: Uuid) {
        let Some(entry) = self.entries.get_mut(&pane) else {
            return;
        };
        let Some(attempt) = entry.attempt.take() else {
            return;
        };
        if let Some(mark) = attempt.mark {
            entry.baseline = Some(mark);
        }
        if entry.baseline == entry.latest {
            entry.failures = 0;
        }
    }

    /// Drops the entries whose pane `keep` no longer vouches for.
    pub fn prune(&mut self, mut keep: impl FnMut(Uuid, Uuid) -> bool) {
        self.entries.retain(|pane, entry| keep(*pane, entry.session));
    }
}

/// How a wait for a respawned daemon's new leader ended.
#[derive(Debug, Clone, Copy)]
enum WaitEnd {
    Leader(i32),
    TimedOut,
}

/// A respawned daemon whose new leader is being waited for, off the main thread.
#[derive(Debug)]
struct Watch {
    session: Uuid,
    daemon: String,
    /// Filled once the wait ends.
    answer: Arc<Mutex<Option<WaitEnd>>>,
}

/// The origin's side: this app run's epoch, each session's counts, and the waits in flight.
#[derive(Debug)]
pub(crate) struct OriginRespawns {
    /// Names this app run in respawn counts, so a viewer never reads a restarted counter as older.
    epoch: String,
    generations: HashMap<Uuid, BTreeMap<String, i64>>,
    watches: Vec<Watch>,
}

impl Default for OriginRespawns {
    fn default() -> Self {
        Self {
            epoch: Uuid::new_v4().to_string(),
            generations: HashMap::new(),
            watches: Vec::new(),
        }
    }
}

impl OriginRespawns {
    /// Every count of `session`, for its snapshot.
    pub(crate) fn snapshot(&self, session: Uuid) -> PresentationRespawns {
        PresentationRespawns {
            epoch: self.epoch.clone(),
            generations: self.generations.get(&session).cloned().unwrap_or_default(),
        }
    }

    pub(crate) fn waiting(&self) -> bool {
        !self.watches.is_empty()
    }
}

impl Core {
    // MARK: - the origin

    /// The leader of the live pane `identity`'s daemon as last listed, read before a respawn replaces it (upstream's
    /// `priorLeader`).
    pub(crate) fn prior_leader(&self, identity: Uuid) -> Option<i32> {
        self.live.leaders.borrow().leader(&daemon_name(identity))
    }

    /// Publishes the respawn of the live pane `identity` of `session` to the session's viewers once a live leader
    /// other than `prior` runs under its daemon's name; nothing is published when none shows up in time (upstream's
    /// `ownerRespawned`). The wait runs off the main thread; the remote tick picks its answer up.
    pub(crate) fn owner_respawned(&mut self, session: Uuid, identity: Uuid, prior: Option<i32>) {
        let Some(host) = self.live.host().cloned() else {
            return;
        };
        let daemon = daemon_name(identity);
        let answer = Arc::new(Mutex::new(None));
        let (slot, name) = (Arc::clone(&answer), daemon.clone());
        let started = std::thread::Builder::new()
            .name("ngterm.respawn-leader".to_owned())
            .spawn(move || {
                let deadline = Instant::now() + RESPAWN_LEADER_WAIT;
                let leader = loop {
                    let listed = host.list().ok().and_then(|records| {
                        records
                            .into_iter()
                            .find(|record| record.name == name)
                            .and_then(|record| record.leader_pid)
                            .filter(|pid| Some(*pid) != prior)
                    });
                    if listed.is_some() || Instant::now() >= deadline {
                        break listed;
                    }
                    std::thread::sleep(LEADER_POLL);
                };
                let end = leader.map_or(WaitEnd::TimedOut, WaitEnd::Leader);
                *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(end);
            });
        if started.is_err() {
            tracing::warn!(%session, "no respawn published: the leader wait could not start");
            return;
        }
        self.remote.respawns.watches.push(Watch {
            session,
            daemon,
            answer,
        });
    }

    /// Publishes each finished wait whose leader is alive (the end of upstream's `ownerRespawned` task).
    pub(crate) fn tick_respawn_watches(&mut self) {
        let watches = std::mem::take(&mut self.remote.respawns.watches);
        for watch in watches {
            let finished = watch.answer.lock().unwrap_or_else(PoisonError::into_inner).take();
            let Some(end) = finished else {
                self.remote.respawns.watches.push(watch);
                continue;
            };
            let alive = match end {
                WaitEnd::Leader(pid) => self.restore.table().is_none_or(|table| table.stat(pid).is_some()),
                WaitEnd::TimedOut => false,
            };
            if !alive {
                continue;
            }
            let counts = self.remote.respawns.generations.entry(watch.session).or_default();
            let generation = counts.get(&watch.daemon).copied().unwrap_or_default() + 1;
            counts.insert(watch.daemon.clone(), generation);
            let respawns = PresentationRespawns {
                epoch: self.remote.respawns.epoch.clone(),
                generations: BTreeMap::from([(watch.daemon, generation)]),
            };
            self.remote.hub.publish(&Body::Respawn(respawns), watch.session);
        }
        self.run_hub_events();
    }

    // MARK: - the viewer

    /// Feeds what the origin reported into the book, for each local pane of the row; a frame names one daemon and
    /// leaves the others as they were (upstream's `applyRemoteRespawns`).
    pub(crate) fn apply_remote_respawns(&mut self, respawns: &PresentationRespawns, seed: bool, session: Uuid) {
        let Some(daemons) = self
            .library()
            .store_for_session(session)
            .and_then(|store| store.session(session))
            .and_then(|found| found.remote_presentation())
            .map(|state| state.binding.daemons_by_pane().clone())
        else {
            return;
        };
        let mut due = Vec::new();
        for (pane, daemon) in daemons {
            let generation = respawns.generations.get(&daemon).copied();
            if !seed && generation.is_none() {
                continue;
            }
            let mark = Mark {
                epoch: respawns.epoch.clone(),
                generation: generation.unwrap_or_default(),
            };
            if self.remote.viewer.respawns.observe(mark, pane, session, seed) {
                due.push(pane);
            }
        }
        for pane in due {
            self.reattach_respawned(session, pane);
        }
    }

    /// A remote pane's clean exit (upstream's `remotePaneHeld`): it attaches again when the origin respawned the pane.
    /// A local pane never enters the book.
    pub(crate) fn respawn_pane_held(&mut self, session: Uuid, token: &str) {
        let Ok(pane) = Uuid::parse_str(token) else {
            return;
        };
        let remote = self.library().held_session(session).is_some_and(|found| {
            found.remote_presentation().is_some()
                && matches!(
                    found.pane_role_for_token(token),
                    Some(StatusPane::Left | StatusPane::Right)
                )
        });
        if remote && self.remote.viewer.respawns.held(pane, session) {
            self.reattach_respawned(session, pane);
        }
    }

    /// Drops the entries of panes no open or soft-closed row holds any more (upstream's `pruneRespawnBook`); run from
    /// the tick only, so a soft close's pending record is in place first.
    pub(crate) fn prune_respawn_book(&mut self) {
        let library = &self.library;
        self.remote.viewer.respawns.prune(|pane, session| {
            library
                .held_session(session)
                .is_some_and(|row| row.pane_identity == pane || row.split_pane_identity == Some(pane))
        });
    }

    /// Attaches the respawned `pane` again, attach-only; the new attach records its own attempt. One that could not
    /// even start leaves the exit prompt.
    fn reattach_respawned(&mut self, session: Uuid, pane: Uuid) {
        self.reattach_remote_pane(session, pane, LeadAttachment::new(false), true);
    }
}

/// Upstream's `RemoteRespawnTests.swift`, case for case.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::frames::{self, PresentationFrame, PresentationSnapshot};

    struct Fixture {
        book: RemoteRespawnBook,
        pane: Uuid,
        session: Uuid,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                book: RemoteRespawnBook::default(),
                pane: Uuid::new_v4(),
                session: Uuid::new_v4(),
            }
        }

        fn observe_in(&mut self, generation: i64, epoch: &str, seed: bool, pane: Uuid) -> bool {
            let mark = Mark {
                epoch: epoch.to_owned(),
                generation,
            };
            self.book.observe(mark, pane, self.session, seed)
        }

        fn observe(&mut self, generation: i64, seed: bool) -> bool {
            self.observe_in(generation, "e1", seed, self.pane)
        }

        fn held_in(&mut self, pane: Uuid) -> bool {
            self.book.held(pane, self.session)
        }

        fn held(&mut self) -> bool {
            self.held_in(self.pane)
        }

        fn attach(&mut self) {
            self.book.attach_started(self.pane, self.session);
        }

        fn acknowledge(&mut self) {
            self.book.acknowledged(self.pane);
        }
    }

    #[test]
    fn a_clean_exit_with_no_respawn_keeps_the_prompt() {
        let mut f = Fixture::new();
        assert!(!f.observe(0, true));
        assert!(!f.held());
    }

    #[test]
    fn a_respawn_before_or_after_the_held_exit_reattaches() {
        let mut before = Fixture::new();
        before.observe(0, true);
        assert!(!before.observe(1, false));
        assert!(before.held());

        let mut after = Fixture::new();
        after.observe(0, true);
        assert!(!after.held());
        assert!(after.observe(1, false));
    }

    #[test]
    fn a_reconnect_snapshot_recovers_a_missed_respawn_frame() {
        let mut f = Fixture::new();
        f.observe(2, true);
        assert!(!f.held());
        assert!(
            f.observe(3, true),
            "a later snapshot is compared, never taken as a new baseline"
        );
    }

    #[test]
    fn an_origin_restart_is_never_read_as_older() {
        let mut f = Fixture::new();
        let pane = f.pane;
        f.observe_in(5, "old", true, pane);
        f.held();
        assert!(!f.observe_in(0, "new", true, pane));
        assert!(f.observe_in(1, "new", false, pane));
    }

    #[test]
    fn two_local_views_of_one_origin_pane_reattach_independently() {
        let mut f = Fixture::new();
        let other = Uuid::new_v4();
        for local in [f.pane, other] {
            f.observe_in(0, "e1", true, local);
            f.held_in(local);
        }
        assert!(f.observe(1, false));
        f.attach();
        f.acknowledge();
        assert!(
            f.observe_in(1, "e1", false, other),
            "the first view consuming it leaves the second due"
        );
    }

    #[test]
    fn a_generation_is_consumed_only_by_an_acknowledged_attach_and_failures_are_bounded() {
        let mut f = Fixture::new();
        f.observe(0, true);
        f.observe(1, false);
        assert!(f.held());
        for _ in 1..RemoteRespawnBook::MAX_ATTEMPTS {
            f.attach();
            assert!(
                f.held(),
                "an attach that exits before reporting retries the same generation"
            );
        }
        f.attach();
        assert!(!f.held(), "the attempts for one generation are bounded");
        assert!(f.observe(2, false), "a newer respawn starts a fresh budget");
        f.attach();
        f.acknowledge();
        assert!(!f.held());
    }

    #[test]
    fn an_acknowledgement_consumes_only_the_generation_its_attach_started_for() {
        let mut f = Fixture::new();
        f.observe(0, true);
        f.observe(1, false);
        f.held();
        f.attach();
        assert!(!f.observe(2, false), "an attach in flight is not started twice");
        f.acknowledge();
        assert!(f.held(), "respawn 2 is still owed after the attach for 1 reported");
    }

    #[test]
    fn a_failure_of_an_older_attach_is_not_charged_to_a_newer_generation() {
        let mut f = Fixture::new();
        f.observe(0, true);
        f.observe(1, false);
        f.held();
        for _ in 1..RemoteRespawnBook::MAX_ATTEMPTS {
            f.attach();
            f.held();
        }
        f.attach();
        f.observe(2, false);
        assert!(
            f.held(),
            "the last attempt for 1 failing leaves 2 with its whole budget"
        );
    }

    #[test]
    fn a_network_reconnects_acknowledgement_also_moves_the_baseline() {
        let mut f = Fixture::new();
        f.observe(0, true);
        f.observe(1, false);
        f.attach();
        f.acknowledge();
        assert!(!f.held(), "a clean exit with no new respawn keeps the prompt");
    }

    #[test]
    fn the_held_fact_outlives_any_stream_outage_until_pruned() {
        let mut f = Fixture::new();
        f.observe(0, true);
        f.held();
        assert!(
            f.observe(1, true),
            "no clock: a snapshot after a long outage still reattaches"
        );
        f.book.prune(|_, _| false);
        assert!(f.book.is_empty());
    }

    #[test]
    fn the_respawn_frame_and_snapshot_round_trip_and_an_older_snapshot_has_none() {
        let respawns = PresentationRespawns {
            epoch: "e1".to_owned(),
            generations: BTreeMap::from([("ngterm-a".to_owned(), 2)]),
        };
        let frame = PresentationFrame {
            generation: 1,
            rev: 3,
            body: Body::Respawn(respawns.clone()),
        };
        let mut line = frames::encode(&frame).unwrap();
        line.pop();
        assert_eq!(frames::decode(&line).unwrap(), frame);
        let snapshot = PresentationSnapshot {
            respawns: Some(respawns),
            ..PresentationSnapshot::default()
        };
        let encoded = serde_json::to_string(&snapshot).unwrap();
        assert_eq!(
            serde_json::from_str::<PresentationSnapshot>(&encoded).unwrap(),
            snapshot
        );
        let older: PresentationSnapshot = serde_json::from_str("{}").unwrap();
        assert_eq!(older.respawns, None);
    }
}
