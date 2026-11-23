//! The live text page frames (upstream's `PresentationFramesTests` text cases, #978): the text actions and their fields,
//! `page.result`, the text state in `page.state` and a page request's log and seed, each left out when unset, so an
//! older peer decodes the frames it knows.

use serde_json::{Value, json};

use super::*;
use crate::html::text_page::{MAX_QUERY, TextChanges, TextLog, TextLogReport};

fn wire(body: &Body) -> Value {
    let mut line = encode(&PresentationFrame {
        generation: 1,
        rev: 1,
        body: body.clone(),
    })
    .unwrap();
    line.pop();
    assert_eq!(&decode(&line).unwrap().body, body, "round trip");
    serde_json::from_slice(&line).unwrap()
}

fn command(action: PresentationPageAction) -> PresentationPageCommand {
    PresentationPageCommand {
        page: "p1".to_owned(),
        seq: 4,
        action,
        size_percent: None,
        width: None,
        height: None,
        merge: None,
        toggle: None,
        query: None,
        step: None,
    }
}

fn state() -> PresentationTextState {
    PresentationTextState {
        following: Some(true),
        log: Some(TextLogReport {
            lines: 3,
            clipped: true,
            waiting: false,
            unreadable: false,
        }),
        query: Some("err".to_owned()),
        revision: 2,
        matches: Some(5),
        current: Some(1),
        wrap: true,
        changes: Some(TextChanges { count: 4, current: 2 }),
    }
}

#[test]
fn text_actions_carry_their_toggle_query_and_step() {
    let follow = PresentationPageCommand {
        toggle: Some(PresentationToggle::Toggle),
        ..command(PresentationPageAction::Follow)
    };
    assert_eq!(
        wire(&Body::PageCommand(follow))["page"],
        json!({"page": "p1", "seq": 4, "action": "follow", "toggle": "toggle"})
    );
    let search = PresentationPageCommand {
        query: Some("main()".to_owned()),
        ..command(PresentationPageAction::Search)
    };
    assert_eq!(wire(&Body::PageCommand(search))["page"]["query"], "main()");
    let step = PresentationPageCommand {
        step: Some(PresentationTextStep::Prev),
        ..command(PresentationPageAction::Changes)
    };
    assert_eq!(
        wire(&Body::PageCommand(step))["page"],
        json!({"page": "p1", "seq": 4, "action": "changes", "step": "prev"})
    );
    let wrap = wire(&Body::PageCommand(PresentationPageCommand {
        toggle: Some(PresentationToggle::Off),
        ..command(PresentationPageAction::Wrap)
    }));
    assert_eq!(wrap["page"]["toggle"], "off");
    assert!(
        [
            PresentationPageAction::Follow,
            PresentationPageAction::Search,
            PresentationPageAction::Wrap,
            PresentationPageAction::Changes
        ]
        .into_iter()
        .all(PresentationPageAction::answers_with_result)
    );
    assert!(!PresentationPageAction::Hide.answers_with_result());
    assert!(!PresentationPageAction::Resize.answers_with_result());
}

/// Upstream's `anUnknownActionFromANewerPeerIsNotDecodedAsAnOldOne`.
#[test]
fn an_unknown_action_is_not_decoded_as_another() {
    let line = br#"{"kind":"page.command","gen":1,"rev":1,"page":{"page":"p","seq":1,"action":"zoom"}}"#;
    assert!(decode(line).is_err());
}

#[test]
fn a_page_result_round_trips_and_cuts_its_text() {
    let follow = PresentationPageCommand {
        toggle: Some(PresentationToggle::On),
        ..command(PresentationPageAction::Follow)
    };
    let result = PresentationPageResult::new(&follow, Ok(Some("following".to_owned())), Some(state()));
    let frame = wire(&Body::PageResult(result));
    assert_eq!(frame["kind"], "page.result");
    assert_eq!(
        frame["page"],
        json!({"page": "p1", "seq": 4, "ok": true, "text": "following", "state": {"following": true,
            "log": {"lines": 3, "clipped": true, "waiting": false, "unreadable": false}, "query": "err",
            "revision": 2, "matches": 5, "current": 1, "wrap": true, "changes": {"count": 4, "current": 2}}})
    );
    let long = "e".repeat(PresentationPageResult::MAX_TEXT + 10);
    let refused = PresentationPageResult::new(&follow, Err(long), None);
    assert!(!refused.ok && refused.text.is_none());
    assert_eq!(
        refused.error.as_deref().map(str::len),
        Some(PresentationPageResult::MAX_TEXT)
    );
    assert_eq!(
        wire(&Body::PageResult(refused))["page"].as_object().unwrap().len(),
        4,
        "no text and no state"
    );
}

#[test]
fn a_text_state_leaves_out_what_it_does_not_have_and_cuts_a_query() {
    assert_eq!(
        serde_json::to_value(PresentationTextState::default()).unwrap(),
        json!({"revision": 0, "wrap": false})
    );
    let long = "q".repeat(MAX_QUERY + 5);
    let decoded: PresentationTextState = serde_json::from_value(json!({"query": long})).unwrap();
    assert_eq!(decoded.query.map(|query| query.chars().count()), Some(MAX_QUERY));
    assert_eq!(decoded.revision, 0, "an absent revision reads as none");
}

/// Upstream's `aPageStateWithoutTextDecodes` and `aPageRequestCarriesItsLogAndSeed`.
#[test]
fn a_page_state_and_request_carry_text_only_when_set() {
    let mut page_state = PresentationPageState::new("p1", PageShown::default(), false, 2);
    assert!(wire(&Body::PageState(page_state.clone()))["page"].get("text").is_none());
    page_state.text = Some(state());
    assert_eq!(wire(&Body::PageState(page_state))["page"]["text"]["revision"], 2);
    let line = br#"{"kind":"page.request","gen":1,"rev":1,"page":{"page":"p","token":"t","source":{"text":{"path":"/a.log"}},"identity":"a.log","chromeless":false,"javascript":false,"navigation":false,"browse":false,"persistent":false,"follow":false,"textLog":{"tail":200,"follow":true},"textSeed":{"following":false,"revision":1,"wrap":true}}}"#;
    let Body::PageRequest(request) = decode(line).unwrap().body else {
        panic!("not a request");
    };
    assert_eq!(
        request.text_log,
        Some(TextLog {
            tail: 200,
            follow: true
        })
    );
    let seed = request.text_seed.unwrap();
    assert_eq!((seed.following, seed.wrap, seed.revision), (Some(false), true, 1));
}
