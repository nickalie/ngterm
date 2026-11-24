use std::cell::{Cell, RefCell};
use std::rc::Rc;

use uuid::Uuid;

use super::super::fixture::{self, companion, page_row, session, snapshot, workspace};
use super::super::frames::{CatalogChange, CatalogWorkspace};
use super::*;

/// A subscriber's side: what it was sent and why it was closed.
#[derive(Default)]
struct Received {
    frames: RefCell<Vec<CatalogFrame>>,
    closed: RefCell<Vec<CloseReason>>,
    refuses: Cell<bool>,
}

impl Received {
    fn deltas(&self) -> Vec<(i64, CatalogChange)> {
        self.frames
            .borrow()
            .iter()
            .filter_map(|frame| match frame {
                CatalogFrame::Delta { rev, change, .. } => Some((*rev, change.clone())),
                _ => None,
            })
            .collect()
    }

    fn revs(&self) -> Vec<i64> {
        self.deltas().into_iter().map(|(rev, _)| rev).collect()
    }

    fn last(&self) -> Option<CatalogFrame> {
        self.frames.borrow().last().cloned()
    }
}

struct Sink(Rc<Received>);

impl CatalogSink for Sink {
    fn offer(&self, frame: &CatalogFrame) -> bool {
        if self.0.refuses.get() {
            return false;
        }
        self.0.frames.borrow_mut().push(frame.clone());
        true
    }

    fn close(&self, reason: CloseReason) {
        self.0.closed.borrow_mut().push(reason);
    }
}

/// A hub at epoch `e1` on a clock the test moves.
fn hub() -> (CatalogHub, Rc<Cell<Instant>>) {
    let clock = Rc::new(Cell::new(Instant::now()));
    let reading = Rc::clone(&clock);
    let hub = CatalogHub::new(
        "e1".to_owned(),
        Duration::from_secs(30),
        Box::new(move || reading.get()),
    );
    (hub, clock)
}

fn hello(page_rows: Option<bool>) -> CatalogHello {
    CatalogHello {
        version: 1,
        page_rows,
        ..CatalogHello::default()
    }
}

fn subscribe(
    hub: &mut CatalogHub,
    workspaces: &[CatalogWorkspace],
    page_rows: Option<bool>,
) -> (SubscriberId, Rc<Received>) {
    let received = Rc::new(Received::default());
    let id = hub
        .subscribe(
            &hello(page_rows),
            Box::new(Sink(Rc::clone(&received))),
            fixture::versions(),
            snapshot(workspaces.to_vec()),
        )
        .unwrap();
    (id, received)
}

fn advance(clock: &Cell<Instant>, seconds: u64) {
    clock.set(clock.get() + Duration::from_secs(seconds));
}

/// Upstream's `aSubscriberGetsHelloThenTheSnapshotThenNumberedDeltas`.
#[test]
fn a_subscriber_gets_hello_then_the_snapshot_then_numbered_deltas() {
    let (mut hub, _) = hub();
    let mut workspaces = vec![workspace(Uuid::new_v4(), "w", vec![session("a")])];
    let (_, sink) = subscribe(&mut hub, &workspaces, None);
    workspaces[0].sessions.push(session("b"));
    hub.refresh(snapshot(workspaces.clone()));
    workspaces[0].name = "dev".to_owned();
    hub.refresh(snapshot(workspaces));
    let frames = sink.frames.borrow().clone();
    assert_eq!(frames.len(), 4, "{frames:?}");
    assert_eq!(
        frames[0],
        CatalogFrame::Hello(CatalogHello {
            version: 1,
            epoch: Some("e1".to_owned()),
            versions: Some(fixture::versions()),
            page_rows: None,
        })
    );
    let CatalogFrame::Snapshot(first) = &frames[1] else {
        panic!("no snapshot: {frames:?}")
    };
    assert_eq!((first.epoch.as_str(), first.rev), ("e1", 0));
    assert_eq!(sink.revs(), [1, 2]);
    let kinds: Vec<&str> = sink.deltas().iter().map(|(_, change)| change.kind()).collect();
    assert_eq!(kinds, ["session.added", "workspace.changed"]);
}

/// Upstream's `aChangeMadeBeforeSubscribingIsInTheSnapshotAndNeverRepeatedAsADelta`.
#[test]
fn a_change_made_before_subscribing_is_in_the_snapshot_and_never_repeated_as_a_delta() {
    let (mut hub, _) = hub();
    let mut workspaces = vec![workspace(Uuid::new_v4(), "w", vec![])];
    let (_, early) = subscribe(&mut hub, &workspaces, None);
    workspaces[0].sessions.push(session("s"));
    let (_, late) = subscribe(&mut hub, &workspaces, None);
    let Some(CatalogFrame::Snapshot(snapshot)) = late.last() else {
        panic!("no snapshot")
    };
    assert_eq!(snapshot.rev, 1);
    assert_eq!(snapshot.workspaces, workspaces);
    assert!(late.deltas().is_empty());
    assert_eq!(early.revs(), [1]);
}

/// Upstream's `aRefreshThatChangesNothingPublishesNothing`.
#[test]
fn a_refresh_that_changes_nothing_publishes_nothing() {
    let (mut hub, _) = hub();
    let workspaces = vec![workspace(Uuid::new_v4(), "w", vec![session("s")])];
    let (_, sink) = subscribe(&mut hub, &workspaces, None);
    hub.refresh(snapshot(workspaces.clone()));
    hub.refresh(snapshot(workspaces));
    assert_eq!(sink.frames.borrow().len(), 2);
    assert_eq!(hub.rev(), 0);
}

/// Upstream's `aViewerSpeakingNoVersionIsRefused`.
#[test]
fn a_viewer_speaking_no_version_is_refused() {
    let (mut hub, _) = hub();
    let refused = hub.subscribe(
        &CatalogHello::default(),
        Box::new(Sink(Rc::default())),
        fixture::versions(),
        snapshot(vec![]),
    );
    assert_eq!(refused, Err(SubscribeError::UnsupportedVersion(0)));
    assert_eq!(refused.unwrap_err().to_string(), "unsupported catalog version 0");
    assert_eq!(hub.subscriber_count(), 0);
}

/// Upstream's `aStalledSubscriberIsDroppedAndTheOthersKeepTheirDeltas`.
#[test]
fn a_stalled_subscriber_is_dropped_and_the_others_keep_their_deltas() {
    let (mut hub, _) = hub();
    let (_, stalled) = subscribe(&mut hub, &[], None);
    let (_, healthy) = subscribe(&mut hub, &[], None);
    stalled.refuses.set(true);
    hub.refresh(snapshot(vec![workspace(Uuid::new_v4(), "w", vec![])]));
    assert_eq!(*stalled.closed.borrow(), [CloseReason::Stalled]);
    assert_eq!(healthy.deltas().len(), 1);
    assert_eq!(hub.subscriber_count(), 1);
}

/// Upstream's `aSubscriberThatStopsAckingGoesStaleAndAnAckingOneIsPinged`.
#[test]
fn a_subscriber_that_stops_acking_goes_stale_and_an_acking_one_is_pinged() {
    let (mut hub, clock) = hub();
    let (_, quiet) = subscribe(&mut hub, &[], None);
    let (id, acking) = subscribe(&mut hub, &[], None);
    hub.heartbeat();
    advance(&clock, 20);
    hub.receive(&CatalogFrame::Ack, id);
    advance(&clock, 20);
    hub.heartbeat();
    assert_eq!(*quiet.closed.borrow(), [CloseReason::Stale]);
    assert_eq!(acking.last(), Some(CatalogFrame::Ping));
    assert!(acking.closed.borrow().is_empty());
}

/// Upstream's `aLongGapBetweenHeartbeatsLeavesTheSubscriberAPingToAnswer`.
#[test]
fn a_long_gap_between_heartbeats_leaves_the_subscriber_a_ping_to_answer() {
    let (mut hub, clock) = hub();
    let (id, sink) = subscribe(&mut hub, &[], None);
    advance(&clock, 60);
    hub.heartbeat();
    assert!(sink.closed.borrow().is_empty());
    hub.receive(&CatalogFrame::Ack, id);
    advance(&clock, 60);
    hub.heartbeat();
    assert!(sink.closed.borrow().is_empty());
    let pings = sink
        .frames
        .borrow()
        .iter()
        .filter(|frame| **frame == CatalogFrame::Ping)
        .count();
    assert_eq!(pings, 2);
}

/// Upstream's `anUnansweredPingExpiresFromItsFirstProbe`.
#[test]
fn an_unanswered_ping_expires_from_its_first_probe() {
    let (mut hub, clock) = hub();
    let (_, sink) = subscribe(&mut hub, &[], None);
    hub.heartbeat();
    advance(&clock, 20);
    hub.heartbeat();
    assert!(sink.closed.borrow().is_empty());
    advance(&clock, 11);
    hub.heartbeat();
    assert_eq!(*sink.closed.borrow(), [CloseReason::Stale]);
}

/// A viewer that applies what it is sent and notes a revision it never got.
#[derive(Default)]
struct OlderViewer {
    state: Option<CatalogSnapshot>,
    gap: bool,
}

impl OlderViewer {
    fn take(&mut self, frames: &[CatalogFrame]) {
        for frame in frames {
            match frame {
                CatalogFrame::Snapshot(snapshot) => self.state = Some(snapshot.clone()),
                CatalogFrame::Delta { rev, change, .. } => {
                    self.step(*rev);
                    if let Some(state) = &mut self.state {
                        state.apply(change);
                    }
                }
                CatalogFrame::Unknown { rev: Some(rev), .. } => self.step(*rev),
                _ => {}
            }
        }
    }

    fn step(&mut self, rev: i64) {
        let Some(state) = &mut self.state else {
            return;
        };
        if rev != state.rev + 1 {
            self.gap = true;
        }
        state.rev = rev;
    }
}

/// Upstream's `aViewerWithoutPageRowsFollowsEveryRevisionAndNeverSeesAPage`.
#[test]
fn a_viewer_without_page_rows_follows_every_revision_and_never_sees_a_page() {
    let (mut hub, _) = hub();
    let (a, b, c) = (session("a"), session("b"), session("c"));
    let page = page_row();
    let mut workspaces = vec![
        workspace(Uuid::new_v4(), "w1", vec![a.clone(), page.clone(), b.clone()]),
        workspace(Uuid::new_v4(), "w2", vec![]),
    ];
    let (_, older) = subscribe(&mut hub, &workspaces, None);
    let (_, newer) = subscribe(&mut hub, &workspaces, Some(true));
    let mut step = |hub: &mut CatalogHub, change: &dyn Fn(&mut Vec<CatalogWorkspace>)| {
        change(&mut workspaces);
        hub.refresh(snapshot(workspaces.clone()));
        workspaces.clone()
    };
    step(&mut hub, &|w| "docs".clone_into(&mut w[0].sessions[1].fields.name));
    step(&mut hub, &|w| w[0].sessions.insert(2, c.clone()));
    step(&mut hub, &|w| {
        w[0].sessions = vec![b.clone(), page.clone(), c.clone(), a.clone()];
    });
    step(&mut hub, &|w| {
        w[0].sessions.retain(|session| session.id() != a.id());
        w[1].sessions = vec![a.clone()];
    });
    let last = step(&mut hub, &|w| w[0].sessions.retain(|session| session.id() != page.id()));
    let mut viewer = OlderViewer::default();
    viewer.take(&older.frames.borrow());
    assert!(!viewer.gap);
    let page_id = crate::pane_env::uuid_string(page.id());
    for frame in older.frames.borrow().iter() {
        let line = String::from_utf8(frames::encode(frame).unwrap()).unwrap();
        assert!(!line.contains(&page_id), "{line}");
    }
    let state = viewer.state.unwrap();
    assert_eq!(state.rev, hub.rev());
    assert_eq!(state.workspaces, without_page_rows(&snapshot(last)).workspaces);
    assert_eq!(newer.revs(), (1..=hub.rev()).collect::<Vec<_>>());
    assert!(
        newer
            .deltas()
            .iter()
            .any(|(_, change)| matches!(change, CatalogChange::SessionChanged(fields) if fields.page.is_some()))
    );
}

/// Upstream's `aPageRootsCompanionShowsTopLevelToAViewerWithoutPageRows`.
#[test]
fn a_page_roots_companion_shows_top_level_to_a_viewer_without_page_rows() {
    let (mut hub, _) = hub();
    let root = page_row();
    let mut workspaces = vec![workspace(Uuid::new_v4(), "w", vec![root.clone()])];
    let (_, older) = subscribe(&mut hub, &workspaces, None);
    let shell = companion("shell", root.id());
    workspaces[0].sessions.push(shell.clone());
    hub.refresh(snapshot(workspaces));
    let frames = older.frames.borrow().clone();
    let CatalogFrame::Snapshot(first) = &frames[1] else {
        panic!("no snapshot: {frames:?}")
    };
    assert!(first.workspaces[0].sessions.is_empty());
    let Some((_, CatalogChange::SessionAdded { session: added, .. })) = older.deltas().first().cloned() else {
        panic!("no added companion: {frames:?}")
    };
    assert_eq!((added.id(), added.fields.parent), (shell.id(), None));
}

/// Upstream's `aFilteredChangeListLongerThanTheRevisionsGoesAsAFilteredSnapshot`.
#[test]
fn a_filtered_change_list_longer_than_the_revisions_goes_as_a_filtered_snapshot() {
    let (mut hub, _) = hub();
    let root = page_row();
    let (one, two) = (companion("one", root.id()), companion("two", root.id()));
    let mut workspaces = vec![workspace(Uuid::new_v4(), "w", vec![root, one, two])];
    let (_, older) = subscribe(&mut hub, &workspaces, None);
    workspaces[0].sessions.remove(0);
    hub.refresh(snapshot(workspaces.clone()));
    assert_eq!(hub.rev(), 1);
    let Some(CatalogFrame::Snapshot(filtered)) = older.last() else {
        panic!("no filtered snapshot: {:?}", older.frames.borrow())
    };
    assert_eq!(filtered.rev, 1);
    assert_eq!(filtered.workspaces, without_page_rows(&snapshot(workspaces)).workspaces);
}

/// Upstream's `aPingIsAnsweredWithAnAck`.
#[test]
fn a_ping_is_answered_with_an_ack() {
    let (mut hub, _) = hub();
    let (id, sink) = subscribe(&mut hub, &[], None);
    hub.receive(&CatalogFrame::Ping, id);
    assert_eq!(sink.last(), Some(CatalogFrame::Ack));
}

#[test]
fn an_unsubscribed_viewer_gets_nothing_more() {
    let (mut hub, _) = hub();
    let (gone, left) = subscribe(&mut hub, &[], None);
    let (_, kept) = subscribe(&mut hub, &[], None);
    hub.unsubscribe(gone);
    hub.receive(&CatalogFrame::Ping, gone);
    hub.refresh(snapshot(vec![workspace(Uuid::new_v4(), "w", vec![])]));
    assert_eq!(left.frames.borrow().len(), 2);
    assert_eq!(kept.deltas().len(), 1);
    assert_eq!(hub.subscriber_count(), 1);
}
