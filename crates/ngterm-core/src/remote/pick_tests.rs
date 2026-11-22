//! The pick frames' wire shape (upstream's `PresentationFramesTests` pick cases).

use serde_json::json;

use super::*;
use crate::remote::frames::{PresentationFrame, decode, encode};

fn pick() -> PendingPick {
    PendingPick {
        id: "k1".to_owned(),
        items: vec![PickItem {
            id: "a".to_owned(),
            label: "A".to_owned(),
            subtitle: Some("s".to_owned()),
            ..PickItem::default()
        }],
        prompt: Some("p".to_owned()),
        query: Some("q".to_owned()),
        allow_custom: true,
        selection: Some("a".to_owned()),
    }
}

fn reference() -> PresentationPickRef {
    PresentationPickRef {
        id: "k1".to_owned(),
        owner: 3,
    }
}

fn answer(item: Option<&str>, query: Option<&str>) -> PresentationPickAnswer {
    PresentationPickAnswer {
        id: "k1".to_owned(),
        owner: 3,
        item: item.map(str::to_owned),
        query: query.map(str::to_owned),
    }
}

#[test]
fn the_pick_frames_round_trip_under_the_pick_key() {
    for body in [
        Body::PickRequest(PresentationPick::new(&pick(), 3, true)),
        Body::PickResolve(answer(Some("a"), None)),
        Body::PickResolve(answer(None, Some("typed"))),
        Body::PickResolve(answer(None, None)),
        Body::PickRejected(reference()),
        Body::PickDismiss(reference()),
    ] {
        let frame = PresentationFrame {
            generation: 1,
            rev: 34,
            body,
        };
        let line = encode(&frame).unwrap();
        assert_eq!(decode(&line).unwrap(), frame);
        let wire: serde_json::Value = serde_json::from_slice(&line).unwrap();
        assert!(wire.get("pick").is_some(), "{wire}");
    }
}

#[test]
fn a_request_carries_upstreams_keys_and_omits_what_is_unset() {
    let request = PresentationPick::new(&pick(), 3, true);
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        json!({"id": "k1", "owner": 3, "items": [{"id": "a", "label": "A", "subtitle": "s"}], "prompt": "p",
            "query": "q", "allowCustom": true, "selection": "a", "follow": true})
    );
    let plain = PendingPick {
        allow_custom: false,
        prompt: None,
        query: None,
        selection: None,
        ..pick()
    };
    let request = PresentationPick::new(&plain, 3, false);
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        json!({"id": "k1", "owner": 3, "items": [{"id": "a", "label": "A", "subtitle": "s"}]})
    );
    assert_eq!(request.pending(), plain);
}

#[test]
fn a_request_too_large_for_a_frame_does_not_fit() {
    let rows = (0..1_000)
        .map(|n| PickItem {
            id: format!("{n}"),
            label: "x".repeat(2_000),
            ..PickItem::default()
        })
        .collect();
    let large = PendingPick { items: rows, ..pick() };
    assert!(!PresentationPick::new(&large, 3, false).fits_frame());
    assert!(PresentationPick::new(&pick(), 3, false).fits_frame());
}

/// Upstream's `anAnswerIsReadAgainstTheStoredItems` and `anAnswerThePickCannotProduceIsRefused`.
#[test]
fn an_answer_is_read_against_the_stored_items() {
    let pick = PendingPick {
        items: vec![
            PickItem {
                id: "a".to_owned(),
                label: "Alpha".to_owned(),
                ..PickItem::default()
            },
            PickItem {
                id: "b".to_owned(),
                label: "Beta".to_owned(),
                ..PickItem::default()
            },
        ],
        ..pick()
    };
    assert_eq!(pick.validated_result(Some("b"), None), pick.picked(1));
    assert_eq!(
        pick.validated_result(None, None),
        Some(ControlPickResult::outcome(ControlPickOutcome::Cancelled))
    );
    assert_eq!(
        pick.validated_result(None, Some("free text")),
        Some(PendingPick::custom("free text"))
    );
    let strict = PendingPick {
        allow_custom: false,
        ..pick.clone()
    };
    for (item, query) in [(Some("zzz"), None), (None, Some("text")), (Some("a"), Some("text"))] {
        assert_eq!(strict.validated_result(item, query), None, "{item:?} {query:?}");
    }
}

#[test]
fn an_answer_stands_for_the_result_it_carries() {
    let picked = ControlPickResult {
        id: Some("a".to_owned()),
        ..ControlPickResult::outcome(ControlPickOutcome::Picked)
    };
    assert_eq!(PresentationPickAnswer::new("k1", 3, &picked), answer(Some("a"), None));
    assert_eq!(
        PresentationPickAnswer::new("k1", 3, &PendingPick::custom("q")),
        answer(None, Some("q"))
    );
    for outcome in [ControlPickOutcome::Cancelled, ControlPickOutcome::Pending] {
        assert_eq!(
            PresentationPickAnswer::new("k1", 3, &ControlPickResult::outcome(outcome)),
            answer(None, None)
        );
    }
}
