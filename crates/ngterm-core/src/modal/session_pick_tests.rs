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
