//! What a headless origin holds for a presenter (upstream's `AppStoreRemoteAskTests`, `AppStoreRemoteOverlayTests`,
//! `PresentationHubTests` and agterm-server's `ServerStreams` cases): an ask, a pick or an overlay job opened while no
//! viewer presents its session waits there, unpresented, is read back as the session's `interactions`, and is handed at
//! once, as one pending batch, to the next presenter that lists the held kinds.

use std::time::Duration;

use ngterm_proto::command::{AskCmd, PickCmd, SessionCmd};
use ngterm_proto::{Args, AskButton, Command, PickItem, Response};
use serde_json::{Value, json};

use super::live::{Live, live};
use super::remote::{Viewer, frame, node, subscribe_listing};
use super::remote_page::Origin;
use super::{args, failure, wire};
use crate::remote::frames::{Body, PendingBatch, PresentationPendingChunk};
use crate::remote::hub::{HELD_KINDS, SUPPORTED_KINDS};

const PAGE_FILE: &str = "/tmp/docs/index.html";

/// A live session on a headless origin no viewer presents yet.
fn headless() -> Live {
    let mut live = live();
    live.fixture.core.set_presenter_only();
    live
}

/// The overlay request `viewer`'s pending batch holds alone.
fn offered_program(viewer: &Viewer) -> crate::remote::frames::PresentationOverlay {
    match batch(viewer).as_slice() {
        [Body::OverlayRequest(request)] => request.clone(),
        other => panic!("{other:?}"),
    }
}

/// A viewer listing `kinds` that takes `live`'s session as its presenter.
fn present(live: &mut Live, kinds: &[&str]) -> (u64, Viewer) {
    let (serial, viewer) = subscribe_listing(&mut live.fixture.core, live.session, "presenter", kinds);
    let acquire = frame(&viewer, &json!({"kind": "presenter.acquire"}));
    live.fixture.core.presentation_line(serial, &acquire);
    live.fixture.core.commit();
    (serial, viewer)
}

/// What `viewer` was handed in its pending batch, applied at the commit.
fn batch(viewer: &Viewer) -> Vec<Body> {
    assert_eq!(viewer.kinds().last().map(String::as_str), Some("pending.commit"));
    let mut pending = PendingBatch::default();
    for chunk in viewer.frames("pending.chunk") {
        let chunk: PresentationPendingChunk = serde_json::from_value(chunk["pending"].clone()).unwrap();
        pending.add(&chunk);
    }
    pending.commit().unwrap()
}

fn interactions(live: &mut Live) -> Value {
    let session = live.session;
    node(&mut live.fixture, session)
        .get("interactions")
        .cloned()
        .unwrap_or(Value::Null)
}

fn states(live: &mut Live) -> Vec<(String, String)> {
    interactions(live)
        .as_array()
        .into_iter()
        .flatten()
        .map(|entry| {
            (
                entry["kind"].as_str().unwrap().to_owned(),
                entry["state"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn held(kind: &str, state: &str) -> (String, String) {
    (kind.to_owned(), state.to_owned())
}

fn ask(live: &mut Live, style: &str, edit: impl FnOnce(&mut Args)) -> Response {
    let session = live.session;
    live.fixture.send(
        Command::Ask(AskCmd::Open),
        Some(&wire(session)),
        args(|a| {
            a.title = Some("deploy?".to_owned());
            a.style = Some(style.to_owned());
            a.buttons = Some(vec![AskButton {
                id: "y".to_owned(),
                label: "Yes".to_owned(),
                hotkey: None,
            }]);
            edit(a);
        }),
    )
}

fn opened_id(response: &Response) -> String {
    assert!(response.ok, "{response:?}");
    response.result.as_ref().unwrap()["id"].as_str().unwrap().to_owned()
}

fn pick(live: &mut Live) -> String {
    let session = live.session;
    let response = live.fixture.send(
        Command::Pick(PickCmd::Open),
        Some(&wire(session)),
        args(|a| {
            a.items = Some(vec![PickItem {
                id: "a".to_owned(),
                label: "Alpha".to_owned(),
                ..PickItem::default()
            }]);
        }),
    );
    opened_id(&response)
}

fn program(live: &mut Live) -> Response {
    let session = live.session;
    live.fixture.session(
        SessionCmd::OverlayOpen,
        Some(&wire(session)),
        args(|a| a.command = Some("true".to_owned())),
    )
}

fn claim(live: &mut Live, job: &str, token: Option<&str>) -> Response {
    live.fixture.session(
        SessionCmd::OverlayJobRun,
        Some(job),
        args(|a| a.token = token.map(str::to_owned)),
    )
}

#[test]
fn an_ask_with_no_presenter_waits_and_goes_to_the_next_presenter_in_one_batch() {
    for style in ["terminal", "gui"] {
        let mut live = headless();
        let id = opened_id(&ask(&mut live, style, |_| {}));
        assert_eq!(states(&mut live), [held("ask", "unpresented")], "{style}");
        assert_eq!(interactions(&mut live)[0]["id"], json!(id));
        let (_, presenter) = present(&mut live, &SUPPORTED_KINDS);
        assert!(presenter.last("ask.request").is_none(), "only inside the batch");
        let handed = batch(&presenter);
        let [Body::AskRequest(request)] = handed.as_slice() else {
            panic!("{handed:?}")
        };
        assert_eq!(request.id, id);
        assert_eq!(states(&mut live), [held("ask", "offered")], "{style}");
        let session = live.session;
        assert_eq!(node(&mut live.fixture, session)["ask"]["remote"], json!(true));
    }
}

#[test]
fn a_presenter_without_the_held_kinds_sees_only_what_opens_while_it_presents() {
    let kinds: Vec<&str> = SUPPORTED_KINDS
        .iter()
        .copied()
        .filter(|kind| !HELD_KINDS.contains(kind))
        .collect();
    let mut live = headless();
    opened_id(&ask(&mut live, "terminal", |_| {}));
    let (_, presenter) = present(&mut live, &kinds);
    assert!(presenter.frames("pending.commit").is_empty());
    assert!(presenter.last("ask.request").is_none());
    assert_eq!(states(&mut live), [held("ask", "unpresented")]);
}

#[test]
fn a_lost_or_refusing_presenter_returns_the_ask_to_pending_for_the_next_one() {
    for style in ["terminal", "gui"] {
        let mut live = headless();
        let id = opened_id(&ask(&mut live, style, |_| {}));
        let (serial, presenter) = present(&mut live, &SUPPORTED_KINDS);
        let handed = batch(&presenter);
        let [Body::AskRequest(request)] = handed.as_slice() else {
            panic!("no ask request")
        };
        let rejected = frame(
            &presenter,
            &json!({"kind": "ask.rejected", "ask": {"id": id, "owner": request.owner}}),
        );
        live.fixture.core.presentation_line(serial, &rejected);
        live.fixture.core.commit();
        assert_eq!(states(&mut live), [held("ask", "unpresented")], "{style}: refused");
        live.fixture.core.presentation_closed(serial);
        live.fixture.core.commit();
        assert_eq!(states(&mut live), [held("ask", "unpresented")], "{style}: lost");
        let (_, next) = present(&mut live, &SUPPORTED_KINDS);
        assert!(matches!(batch(&next).as_slice(), [Body::AskRequest(_)]), "{style}");
        assert_eq!(states(&mut live), [held("ask", "offered")], "{style}");
    }
}

#[test]
fn an_ask_past_its_deadline_is_never_shown() {
    let mut live = headless();
    let id = opened_id(&ask(&mut live, "terminal", |a| a.timeout = Some(5.0)));
    let deadline = interactions(&mut live)[0]["deadline"].as_f64().unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    assert!(
        (deadline - now - 5.0).abs() < 2.0,
        "{deadline} is not five seconds from {now}"
    );
    live.fixture.core.modals.advance_clock(Duration::from_secs(6));
    let (_, presenter) = present(&mut live, &SUPPORTED_KINDS);
    assert!(presenter.frames("pending.chunk").is_empty());
    assert!(presenter.last("ask.request").is_none());
    let result = live
        .fixture
        .send(Command::Ask(AskCmd::Result), Some(&id), Args::default());
    assert_eq!(result.result.unwrap()["ask"]["result"], "cancelled");
    assert_eq!(interactions(&mut live), Value::Null);
}

#[test]
fn a_pick_with_no_presenter_waits_and_returns_to_pending_when_refused_or_lost() {
    let mut live = headless();
    let id = pick(&mut live);
    assert_eq!(states(&mut live), [held("pick", "unpresented")]);
    let session = live.session;
    assert_eq!(node(&mut live.fixture, session)["pick"], json!({"id": id}));
    let (serial, presenter) = present(&mut live, &SUPPORTED_KINDS);
    let handed = batch(&presenter);
    let [Body::PickRequest(request)] = handed.as_slice() else {
        panic!("{handed:?}")
    };
    assert_eq!(request.id, id);
    assert_eq!(states(&mut live), [held("pick", "offered")]);
    let rejected = frame(
        &presenter,
        &json!({"kind": "pick.rejected", "pick": {"id": id, "owner": request.owner}}),
    );
    live.fixture.core.presentation_line(serial, &rejected);
    live.fixture.core.commit();
    assert_eq!(states(&mut live), [held("pick", "unpresented")]);
    live.fixture.core.presentation_closed(serial);
    live.fixture.core.commit();
    let (_, next) = present(&mut live, &SUPPORTED_KINDS);
    assert!(matches!(batch(&next).as_slice(), [Body::PickRequest(_)]));
    let cancel = live
        .fixture
        .send(Command::Pick(PickCmd::Cancel), Some(&id), Args::default());
    assert!(cancel.ok);
    live.fixture.core.commit();
    assert_eq!(next.last("pick.dismiss").unwrap()["pick"]["id"], json!(id));
}

#[test]
fn a_program_with_no_presenter_waits_and_its_offer_token_claims_it_once() {
    let mut live = headless();
    assert!(program(&mut live).ok);
    assert_eq!(states(&mut live), [held("job", "unpresented")]);
    let job = interactions(&mut live)[0]["id"].as_str().unwrap().to_owned();
    assert_eq!(claim(&mut live, &job, None), failure("job not claimable"));
    let (_, presenter) = present(&mut live, &SUPPORTED_KINDS);
    let handed = batch(&presenter);
    let [Body::OverlayRequest(request)] = handed.as_slice() else {
        panic!("{handed:?}")
    };
    assert_eq!(request.job, job);
    let token = request.token.clone().expect("an offer token");
    let offered = interactions(&mut live)[0].clone();
    assert_eq!(offered["state"], "offered");
    assert!(
        offered["deadline"].is_f64(),
        "the launch window starts at the offer: {offered}"
    );
    assert_eq!(claim(&mut live, &job, None), failure("job not claimable"));
    assert_eq!(claim(&mut live, &job, Some("other")), failure("job not claimable"));
    assert!(claim(&mut live, &job, Some(&token)).ok);
    assert_eq!(states(&mut live), [held("job", "claimed")]);
    assert_eq!(claim(&mut live, &job, Some(&token)), failure("job not claimable"));
}

#[test]
fn a_lost_presenter_withdraws_an_unclaimed_job_and_voids_its_token() {
    let mut live = headless();
    assert!(program(&mut live).ok);
    let (serial, presenter) = present(&mut live, &SUPPORTED_KINDS);
    let request = offered_program(&presenter);
    live.fixture.core.presentation_closed(serial);
    live.fixture.core.commit();
    assert_eq!(states(&mut live), [held("job", "unpresented")]);
    let stale = request.token.clone().unwrap();
    assert_eq!(
        claim(&mut live, &request.job, Some(&stale)),
        failure("job not claimable")
    );
    let (_, next) = present(&mut live, &SUPPORTED_KINDS);
    let again = offered_program(&next);
    assert_eq!(again.job, request.job);
    assert_ne!(again.token, request.token, "a fresh token per offer");
}

#[test]
fn a_claimed_job_keeps_its_slot_and_is_never_offered_again() {
    let mut live = headless();
    assert!(program(&mut live).ok);
    let (serial, presenter) = present(&mut live, &SUPPORTED_KINDS);
    let request = offered_program(&presenter);
    assert!(claim(&mut live, &request.job, request.token.as_deref()).ok);
    live.fixture.core.presentation_closed(serial);
    live.fixture.core.commit();
    assert_eq!(states(&mut live), [held("job", "claimed")]);
    let (_, next) = present(&mut live, &SUPPORTED_KINDS);
    assert!(next.frames("pending.chunk").is_empty(), "{:?}", next.kinds());
    assert!(next.last("overlay.request").is_none());
}

fn page_entry(live: &mut Live) -> Value {
    let entries = interactions(live);
    assert_eq!(entries.as_array().map(Vec::len), Some(1), "{entries}");
    assert_eq!(entries[0]["kind"], "page");
    entries[0].clone()
}

/// A page a lost presenter showed is parked unpresented, with no deadline, and goes to the next presenter inside its
/// pending batch; only a channel page waits for a claim, so a url page is offered with no deadline.
#[test]
fn a_page_is_listed_parked_and_handed_on_inside_the_batch() {
    let mut origin = Origin::build(false, &SUPPORTED_KINDS, false);
    origin.fixture().core.set_presenter_only();
    let page = origin.opened(PAGE_FILE, |_| {});
    let offered = page_entry(&mut origin.live);
    assert_eq!(
        (offered["id"].as_str(), offered["state"].as_str()),
        (Some(page.as_str()), Some("offered"))
    );
    assert!(
        offered["deadline"].is_f64(),
        "a channel page waits for its claim: {offered}"
    );

    origin.lose_presenter();
    let parked = page_entry(&mut origin.live);
    assert_eq!(parked["state"], "unpresented");
    assert!(parked.get("deadline").is_none(), "{parked}");

    let (_, next) = present(&mut origin.live, &SUPPORTED_KINDS);
    assert!(next.last("page.request").is_none(), "only inside the batch");
    let handed = batch(&next);
    let [Body::PageRequest(request)] = handed.as_slice() else {
        panic!("{handed:?}")
    };
    assert_eq!(request.page, page);
    assert_eq!(states(&mut origin.live), [held("page", "offered")]);

    let mut url = Origin::build(false, &SUPPORTED_KINDS, false);
    url.fixture().core.set_presenter_only();
    url.opened("https://example.com/", |_| {});
    let offered = page_entry(&mut url.live);
    assert_eq!(offered["state"], "offered");
    assert!(offered.get("deadline").is_none(), "a url page has no claim: {offered}");
}

#[test]
fn everything_held_goes_in_one_batch_and_an_app_origin_holds_nothing() {
    let mut live = headless();
    opened_id(&ask(&mut live, "terminal", |_| {}));
    assert!(program(&mut live).ok);
    pick(&mut live);
    let (_, presenter) = present(&mut live, &SUPPORTED_KINDS);
    let kinds: Vec<String> = batch(&presenter).iter().map(|body| body.kind().to_owned()).collect();
    assert_eq!(kinds, ["ask.request", "pick.request", "overlay.request"]);
    assert_eq!(presenter.frames("pending.commit").len(), 1);

    let mut app = live_app();
    let (_, presenter) = present(&mut app, &SUPPORTED_KINDS);
    assert!(presenter.frames("pending.commit").is_empty());
    assert_eq!(interactions(&mut app), Value::Null);
}

fn live_app() -> Live {
    let mut live = live();
    opened_id(&ask(&mut live, "terminal", |_| {}));
    live
}

#[test]
fn a_pending_batch_applies_only_whole_and_in_order() {
    let big = "x".repeat(PendingBatch::CHUNK_BYTES);
    let bodies = vec![Body::Context(Some(big)), Body::PendingCommit];
    let chunks = PendingBatch::chunks(&bodies).unwrap();
    assert!(chunks.len() > 1);
    let mut whole = PendingBatch::default();
    for chunk in &chunks {
        whole.add(chunk);
    }
    assert_eq!(whole.commit().unwrap(), bodies);
    assert_eq!(whole.commit().unwrap(), [], "a commit starts over");
    let mut skipped = PendingBatch::default();
    skipped.add(&chunks[1]);
    skipped.add(&chunks[0]);
    assert!(skipped.commit().is_err());
    let mut garbled = PendingBatch::default();
    garbled.add(&PresentationPendingChunk {
        index: 0,
        data: "e30=".to_owned(),
    });
    assert!(garbled.commit().is_err());
}
