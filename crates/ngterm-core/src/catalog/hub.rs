//! The catalog hub (upstream's `CatalogHub.swift`): numbers a headless origin's catalog within one epoch and streams
//! it to its viewers, each getting the snapshot, then every change as a delta with its own revision. The caller hands
//! it the projected catalog at each step; its epoch and revision are the hub's.

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::time::{Duration, Instant};

use ngterm_proto::CatalogVersions;

use super::frames::{self, CatalogFrame, CatalogHello, CatalogSnapshot, SKIP_KIND};

/// Where the hub hands a subscriber's frames; the bounded queue and the socket live behind it.
pub trait CatalogSink {
    /// `false` when the consumer's queue is full, which the hub treats as a stalled subscriber.
    fn offer(&self, frame: &CatalogFrame) -> bool;
    fn close(&self, reason: CloseReason);
}

/// Why the hub dropped a subscriber.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    Stalled,
    Stale,
    Unsendable,
}

/// Why a viewer's hello was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscribeError {
    UnsupportedVersion(i64),
}

impl fmt::Display for SubscribeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion(version) => write!(f, "unsupported catalog version {version}"),
        }
    }
}

pub type SubscriberId = u64;

struct Subscriber {
    sink: Box<dyn CatalogSink>,
    /// Whether the viewer shows page rows; an older one is sent the catalog without them.
    page_rows: bool,
    /// The first ping still unanswered; a late heartbeat sends a ping before it can call the subscriber stale.
    pinged_at: Option<Instant>,
}

pub struct CatalogHub {
    epoch: String,
    rev: i64,
    stale_timeout: Duration,
    now: Box<dyn Fn() -> Instant>,
    current: Option<CatalogSnapshot>,
    subscribers: BTreeMap<SubscriberId, Subscriber>,
    last_id: SubscriberId,
}

impl fmt::Debug for CatalogHub {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CatalogHub")
            .field("epoch", &self.epoch)
            .field("rev", &self.rev)
            .field("subscribers", &self.subscribers.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl CatalogHub {
    #[must_use]
    pub fn new(epoch: String, stale_timeout: Duration, now: Box<dyn Fn() -> Instant>) -> Self {
        Self {
            epoch,
            rev: 0,
            stale_timeout,
            now,
            current: None,
            subscribers: BTreeMap::new(),
            last_id: 0,
        }
    }

    #[must_use]
    pub fn epoch(&self) -> &str {
        &self.epoch
    }

    #[must_use]
    pub const fn rev(&self) -> i64 {
        self.rev
    }

    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        self.subscribers.len()
    }

    /// The catalog as of the current revision, after publishing what `source` changed.
    pub fn snapshot(&mut self, source: CatalogSnapshot) -> CatalogSnapshot {
        self.publish(source)
    }

    /// Publishes what changed since the last step, one revision per change; nothing changed publishes nothing. A viewer
    /// without page rows gets the same revisions: the changes of its filtered catalog first, then a skip for each one
    /// left.
    pub fn refresh(&mut self, source: CatalogSnapshot) {
        self.publish(source);
    }

    fn publish(&mut self, source: CatalogSnapshot) -> CatalogSnapshot {
        let next = self.stamped(source);
        let Some(previous) = self.current.take() else {
            self.current = Some(next.clone());
            return next;
        };
        if previous.workspaces == next.workspaces {
            self.current = Some(next.clone());
            return next;
        }
        let first = self.rev + 1;
        let mut state = previous.clone();
        let mut deltas = Vec::new();
        for change in previous.changes(&next) {
            self.rev += 1;
            state.apply(&change);
            deltas.push(CatalogFrame::Delta {
                epoch: self.epoch.clone(),
                rev: self.rev,
                change,
            });
        }
        state.rev = self.rev;
        let older = if self.subscribers.values().any(|subscriber| !subscriber.page_rows) {
            self.without_page_row_frames(&previous, &state, first)
        } else {
            Vec::new()
        };
        self.current = Some(state.clone());
        let ids: Vec<SubscriberId> = self.subscribers.keys().copied().collect();
        for id in ids {
            let page_rows = self.subscribers.get(&id).is_some_and(|subscriber| subscriber.page_rows);
            for frame in if page_rows { &deltas } else { &older } {
                if !self.send(frame, id) {
                    break;
                }
            }
        }
        state
    }

    /// Revisions `first..=rev` for a viewer without page rows. A filtered diff longer than the full one cannot fit
    /// those revisions, so that viewer takes the filtered snapshot at `rev` instead.
    fn without_page_row_frames(
        &self,
        previous: &CatalogSnapshot,
        next: &CatalogSnapshot,
        first: i64,
    ) -> Vec<CatalogFrame> {
        let filtered = without_page_rows(next);
        let changes = without_page_rows(previous).changes(&filtered);
        let count = i64::try_from(changes.len()).unwrap_or(i64::MAX);
        if count > self.rev - first + 1 {
            return frames::snapshot_frames(&filtered).unwrap_or_default();
        }
        let deltas = changes
            .into_iter()
            .zip(first..)
            .map(|(change, rev)| CatalogFrame::Delta {
                epoch: self.epoch.clone(),
                rev,
                change,
            });
        let skips = (first + count..=self.rev).map(|rev| CatalogFrame::Unknown {
            kind: SKIP_KIND.to_owned(),
            epoch: Some(self.epoch.clone()),
            rev: Some(rev),
        });
        deltas.chain(skips).collect()
    }

    /// Registers a viewer and sends it hello, then the snapshot. Pending changes go to everyone else first, so the
    /// snapshot and the deltas after it meet at the same revision.
    ///
    /// # Errors
    ///
    /// The viewer speaks no catalog version this hub knows.
    pub fn subscribe(
        &mut self,
        hello: &CatalogHello,
        sink: Box<dyn CatalogSink>,
        versions: CatalogVersions,
        source: CatalogSnapshot,
    ) -> Result<SubscriberId, SubscribeError> {
        if hello.version < 1 {
            return Err(SubscribeError::UnsupportedVersion(hello.version));
        }
        let page_rows = hello.page_rows == Some(true);
        let snapshot = self.snapshot(source);
        let state = if page_rows {
            snapshot
        } else {
            without_page_rows(&snapshot)
        };
        self.last_id += 1;
        let id = self.last_id;
        self.subscribers.insert(
            id,
            Subscriber {
                sink,
                page_rows,
                pinged_at: None,
            },
        );
        let answer = CatalogHello {
            version: hello.version.min(frames::VERSION),
            epoch: Some(self.epoch.clone()),
            versions: Some(versions),
            page_rows: None,
        };
        let Ok(snapshot) = frames::snapshot_frames(&state) else {
            self.drop_subscriber(id, CloseReason::Unsendable);
            return Ok(id);
        };
        for frame in std::iter::once(CatalogFrame::Hello(answer)).chain(snapshot) {
            if !self.send(&frame, id) {
                break;
            }
        }
        Ok(id)
    }

    pub fn unsubscribe(&mut self, id: SubscriberId) {
        self.subscribers.remove(&id);
    }

    /// A frame a subscriber sent: an ack clears its ping, a ping is answered.
    pub fn receive(&mut self, frame: &CatalogFrame, id: SubscriberId) {
        match frame {
            CatalogFrame::Ack => {
                if let Some(subscriber) = self.subscribers.get_mut(&id) {
                    subscriber.pinged_at = None;
                }
            }
            CatalogFrame::Ping => {
                self.send(&CatalogFrame::Ack, id);
            }
            _ => {}
        }
    }

    /// Closes every subscriber that left a ping unanswered past the stale timeout and pings the rest.
    pub fn heartbeat(&mut self) {
        let now = (self.now)();
        let ids: Vec<SubscriberId> = self.subscribers.keys().copied().collect();
        for id in ids {
            let Some(subscriber) = self.subscribers.get_mut(&id) else {
                continue;
            };
            if subscriber
                .pinged_at
                .is_some_and(|pinged| now.saturating_duration_since(pinged) > self.stale_timeout)
            {
                self.drop_subscriber(id, CloseReason::Stale);
                continue;
            }
            subscriber.pinged_at.get_or_insert(now);
            self.send(&CatalogFrame::Ping, id);
        }
    }

    /// Closes every subscriber's stream (the quit).
    pub fn shutdown(&mut self) {
        let ids: Vec<SubscriberId> = self.subscribers.keys().copied().collect();
        for id in ids {
            self.drop_subscriber(id, CloseReason::Unsendable);
        }
    }

    fn stamped(&self, mut snapshot: CatalogSnapshot) -> CatalogSnapshot {
        self.epoch.clone_into(&mut snapshot.epoch);
        snapshot.rev = self.rev;
        snapshot
    }

    fn send(&mut self, frame: &CatalogFrame, id: SubscriberId) -> bool {
        let Some(subscriber) = self.subscribers.get(&id) else {
            return false;
        };
        if subscriber.sink.offer(frame) {
            return true;
        }
        self.drop_subscriber(id, CloseReason::Stalled);
        false
    }

    fn drop_subscriber(&mut self, id: SubscriberId, reason: CloseReason) {
        if let Some(subscriber) = self.subscribers.remove(&id) {
            subscriber.sink.close(reason);
        }
    }
}

/// `snapshot` as a viewer without page rows sees it: a companion of a page row shows top-level.
#[must_use]
pub fn without_page_rows(snapshot: &CatalogSnapshot) -> CatalogSnapshot {
    let pages: HashSet<_> = snapshot
        .workspaces
        .iter()
        .flat_map(|workspace| workspace.sessions.iter())
        .filter(|session| session.fields.page.is_some())
        .map(super::frames::CatalogSession::id)
        .collect();
    let mut filtered = snapshot.clone();
    if pages.is_empty() {
        return filtered;
    }
    for workspace in &mut filtered.workspaces {
        workspace.sessions.retain(|session| !pages.contains(&session.id()));
        for session in &mut workspace.sessions {
            if session.fields.parent.is_some_and(|parent| pages.contains(&parent)) {
                session.fields.parent = None;
            }
        }
    }
    filtered
}

#[cfg(test)]
#[path = "hub_tests.rs"]
mod tests;
