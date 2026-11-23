//! The book over a bare [`Modals`] (upstream's `SessionPickTests` book cases).

use super::*;
use crate::modal::PickController;

fn pick(id: &str) -> PendingPick {
    PendingPick {
        id: id.to_owned(),
        items: Vec::new(),
        prompt: None,
        query: None,
        allow_custom: true,
        selection: None,
    }
}

/// Upstream's `closingTheWindowCancelsTheLocalPickAndKeepsItReadable`.
#[test]
fn closing_the_window_cancels_the_local_pick_and_keeps_it_readable() {
    let mut modals = Modals::default();
    let (session, window) = (Uuid::new_v4(), Uuid::new_v4());
    modals.open_session_pick_local(pick("k1"), session, window).unwrap();
    modals.unregister(window);
    let cancelled = ControlPickResult::outcome(ControlPickOutcome::Cancelled);
    assert_eq!(modals.session_picks.result("k1"), Some(cancelled));
    assert_eq!(modals.session_picks.node(session), None);
    assert_eq!(modals.session_picks.window_of("k1"), Some(window));
}

/// Upstream's `aClosingWindowDetachesItsShownAndQueuedReplicasWithoutAnsweringOrMountingThem`, for a shown one.
#[test]
fn a_closing_window_detaches_its_replica_without_answering() {
    let mut modals = Modals::default();
    let (session, window) = (Uuid::new_v4(), Uuid::new_v4());
    let request = crate::remote::frames::PresentationPick::new(&pick("k1"), 4, false);
    assert!(modals.open_replica_pick(&request, session, window));
    assert!(
        !modals.open_replica_pick(&request, session, window),
        "an id already held"
    );
    modals.unregister(window);
    assert_eq!(modals.session_picks.node(session), None);
    assert_eq!(modals.session_picks.take_outbox(), (Vec::new(), Vec::new()));
}

/// Upstream's `aStaleRefusalUnderTheSameOwnerLeavesTheNewerPick` and `aHeadlessHandbackWithNoWindowCancels`.
#[test]
fn a_stale_refusal_leaves_the_newer_pick_and_a_handback_with_no_window_cancels() {
    let mut modals = Modals::default();
    let (session, window) = (Uuid::new_v4(), Uuid::new_v4());
    assert!(
        modals
            .session_picks
            .open_remote(pick("first"), session, 2, Some(window))
    );
    assert!(modals.cancel_session_pick("first"));
    assert!(
        modals
            .session_picks
            .open_remote(pick("second"), session, 2, Some(window))
    );
    let stale = PresentationPickRef {
        id: "first".to_owned(),
        owner: 2,
    };
    modals.hand_back_session_pick(session, Some(&stale), Some(window));
    assert_eq!(modals.controller(window).and_then(PickController::pending), None);
    assert_eq!(
        modals.session_picks.node(session),
        Some(ControlSessionPick {
            id: "second".to_owned(),
            remote: Some(true),
            replica: None,
        })
    );
    modals.hand_back_session_pick(session, None, None);
    assert_eq!(
        modals.session_picks.result("second"),
        Some(ControlPickResult::outcome(ControlPickOutcome::Cancelled))
    );
    assert!(!modals.cancel_session_pick("unknown"));
}

#[test]
fn retained_results_stop_at_the_limit_dropping_the_oldest() {
    let mut modals = Modals::default();
    let session = Uuid::new_v4();
    for n in 0..=RETAINED_LIMIT {
        assert!(
            modals
                .session_picks
                .open_remote(pick(&format!("p{n}")), session, 1, None)
        );
        assert!(modals.cancel_session_pick(&format!("p{n}")));
    }
    assert_eq!(modals.session_picks.result("p0"), None);
    assert!(modals.session_picks.result("p1").is_some());
}

/// Owners count per session, so only the session check stops another session's presenter from answering.
#[test]
fn an_answer_on_another_sessions_stream_is_ignored() {
    let mut modals = Modals::default();
    let (session, other) = (Uuid::new_v4(), Uuid::new_v4());
    assert!(modals.session_picks.open_remote(pick("k1"), session, 2, None));
    let answer = PresentationPickAnswer {
        id: "k1".to_owned(),
        owner: 2,
        item: None,
        query: Some("typed".to_owned()),
    };
    modals.resolve_remote_pick(&answer, other);
    assert_eq!(
        modals.session_picks.result("k1"),
        Some(ControlPickResult::outcome(ControlPickOutcome::Pending))
    );
    assert_eq!(
        modals.session_picks.node(session),
        Some(ControlSessionPick {
            id: "k1".to_owned(),
            remote: Some(true),
            replica: None,
        })
    );
    modals.resolve_remote_pick(&answer, session);
    assert_eq!(
        modals.session_picks.result("k1").map(|result| result.result),
        Some(ControlPickOutcome::Custom)
    );
}

/// Upstream's `drain`: a queued replica waits while the slot is still held, an id already queued is refused, and a
/// drain in another window leaves it queued.
#[test]
fn a_queued_replica_waits_until_its_own_windows_slot_frees() {
    let mut modals = Modals::default();
    let (session, window) = (Uuid::new_v4(), Uuid::new_v4());
    modals.open_pick(window, pick("local")).unwrap();
    let request = crate::remote::frames::PresentationPick::new(&pick("k1"), 4, false);
    assert!(modals.open_replica_pick(&request, session, window));
    assert!(
        !modals.open_replica_pick(&request, session, window),
        "an id already queued"
    );
    modals.drain_replica_picks(window);
    modals.drain_replica_picks(Uuid::new_v4());
    assert_eq!(
        modals.session_picks.node(session).map(|node| node.id),
        Some("k1".to_owned())
    );
    assert_eq!(modals.session_picks.window_of("k1"), Some(window));
    assert!(
        !modals.slot_pick_is(window, "k1"),
        "the slot still holds the local pick"
    );
    modals.resolve_pick(window, ControlPickResult::outcome(ControlPickOutcome::Cancelled));
    assert!(modals.slot_pick_is(window, "k1"));
}
