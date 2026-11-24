//! Session-bound picks handed to the phone, and the pending batch of a headless origin.

use ngterm_core::remote::frames::{
    PendingBatch, PresentationPendingChunk, PresentationPick, PresentationPickAnswer, PresentationPickRef,
};
use ngterm_proto::PickItem;

use super::*;

fn pick(id: &str, owner: i64) -> PresentationPick {
    PresentationPick {
        id: id.to_owned(),
        owner,
        items: vec![
            PickItem {
                id: "api".to_owned(),
                label: "api".to_owned(),
                subtitle: Some("~/src/api".to_owned()),
                status: Some("blocked".to_owned()),
                status_color: Some("#ff8800".to_owned()),
                status_shape: Some("checkmark".to_owned()),
            },
            PickItem {
                id: "web".to_owned(),
                label: "web".to_owned(),
                subtitle: None,
                status: None,
                status_color: None,
                status_shape: None,
            },
        ],
        prompt: Some("Switch to".to_owned()),
        query: Some("a".to_owned()),
        allow_custom: Some(true),
        selection: Some("web".to_owned()),
        follow: Some(true),
    }
}

fn pick_dismiss(pick_id: &str) -> MobileEvent {
    MobileEvent::PickDismiss {
        machine_id: MACHINE.to_owned(),
        session_id: SESSION.to_owned(),
        pick_id: pick_id.to_owned(),
    }
}

fn answer(harness: &Harness, pick_id: &str, reply: PickReply) -> bool {
    harness
        .presentations
        .borrow_mut()
        .answer_pick(MACHINE, SESSION, pick_id, reply)
}

fn resolve(id: &str, owner: i64, item: Option<&str>, query: Option<&str>) -> Body {
    Body::PickResolve(PresentationPickAnswer {
        id: id.to_owned(),
        owner,
        item: item.map(str::to_owned),
        query: query.map(str::to_owned),
    })
}

async fn connected() -> Harness {
    let mut harness = Harness::attached(Some(1)).await;
    harness
        .connect(PresentationMode::Presenter, PresentationSnapshot::default())
        .await;
    harness.received().await;
    harness.take_events();
    harness
}

#[test]
fn a_pick_reaches_the_app_as_the_origin_sent_it_and_is_not_refused() {
    local(async {
        let mut harness = connected().await;

        harness.deliver(Body::PickRequest(pick("p1", 5))).await;

        assert_eq!(
            harness.take_events(),
            [MobileEvent::Pick {
                machine_id: MACHINE.to_owned(),
                session_id: SESSION.to_owned(),
                pick: RemotePick {
                    id: "p1".to_owned(),
                    items: vec![
                        RemotePickItem {
                            id: "api".to_owned(),
                            label: "api".to_owned(),
                            subtitle: Some("~/src/api".to_owned()),
                            status: Some("blocked".to_owned()),
                            status_color: Some("#ff8800".to_owned()),
                            status_shape: Some("checkmark".to_owned()),
                        },
                        RemotePickItem {
                            id: "web".to_owned(),
                            label: "web".to_owned(),
                            subtitle: None,
                            status: None,
                            status_color: None,
                            status_shape: None,
                        },
                    ],
                    prompt: Some("Switch to".to_owned()),
                    query: Some("a".to_owned()),
                    allow_custom: true,
                    selection: Some("web".to_owned()),
                },
            }]
        );
        assert!(harness.received().await.bodies.is_empty());
    });
}

#[test]
fn a_picks_answer_echoes_its_owner_once_for_an_item_a_custom_text_or_a_cancel() {
    local(async {
        let mut harness = connected().await;
        for (id, owner) in [("p1", 5), ("p2", 6), ("p3", 7)] {
            harness.deliver(Body::PickRequest(pick(id, owner))).await;
        }
        harness.take_events();

        assert!(answer(&harness, "p1", PickReply::Picked { item: "api".to_owned() }));
        assert!(answer(
            &harness,
            "p2",
            PickReply::Custom {
                query: "docs".to_owned()
            }
        ));
        assert!(answer(&harness, "p3", PickReply::Cancelled));

        assert_eq!(
            harness.received().await.bodies,
            [
                resolve("p1", 5, Some("api"), None),
                resolve("p2", 6, None, Some("docs")),
                resolve("p3", 7, None, None),
            ]
        );
        assert!(!answer(&harness, "p1", PickReply::Cancelled));
        assert!(!answer(&harness, "p9", PickReply::Cancelled));
        assert!(harness.received().await.bodies.is_empty());
    });
}

#[test]
fn a_pick_the_origin_ends_at_its_timeout_is_dismissed_and_no_longer_answered() {
    local(async {
        let mut harness = connected().await;
        harness.deliver(Body::PickRequest(pick("p1", 5))).await;
        harness.take_events();

        // a dismissal under an earlier presenter generation names another hand-over
        harness
            .deliver(Body::PickDismiss(PresentationPickRef {
                id: "p1".to_owned(),
                owner: 4,
            }))
            .await;
        assert!(harness.take_events().is_empty());

        harness
            .deliver(Body::PickDismiss(PresentationPickRef {
                id: "p1".to_owned(),
                owner: 5,
            }))
            .await;
        assert_eq!(harness.take_events(), [pick_dismiss("p1")]);
        assert!(!answer(&harness, "p1", PickReply::Cancelled));
        assert!(harness.received().await.bodies.is_empty());
    });
}

#[test]
fn an_ask_the_origin_ends_at_its_timeout_is_dismissed() {
    local(async {
        let mut harness = connected().await;
        harness.deliver(Body::AskRequest(ask("a1", 5))).await;
        harness.take_events();

        harness
            .deliver(Body::AskDismiss(PresentationAskRef {
                id: "a1".to_owned(),
                owner: 5,
            }))
            .await;

        assert_eq!(
            harness.take_events(),
            [MobileEvent::AskDismiss {
                machine_id: MACHINE.to_owned(),
                session_id: SESSION.to_owned(),
                ask_id: "a1".to_owned(),
            }]
        );
    });
}

#[test]
fn leaving_connected_dismisses_the_open_picks() {
    local(async {
        let mut harness = connected().await;
        harness.deliver(Body::PickRequest(pick("p2", 5))).await;
        harness.deliver(Body::PickRequest(pick("p1", 6))).await;
        harness.take_events();

        harness.end(ChannelEnd::Lost("connection lost".to_owned())).await;

        let events = harness.take_events();
        let dismissed: Vec<&MobileEvent> = events
            .iter()
            .filter(|event| matches!(event, MobileEvent::PickDismiss { .. }))
            .collect();
        assert_eq!(dismissed, [&pick_dismiss("p1"), &pick_dismiss("p2")]);
        assert!(!answer(&harness, "p1", PickReply::Cancelled));
    });
}

fn held_chunks() -> Vec<PresentationPendingChunk> {
    PendingBatch::chunks(&[Body::AskRequest(ask("a1", 5)), Body::PickRequest(pick("p1", 5))]).unwrap()
}

#[test]
fn a_headless_origins_held_ask_and_pick_reach_the_app_together_at_the_batchs_commit() {
    local(async {
        let mut harness = connected().await;

        for chunk in held_chunks() {
            harness.deliver(Body::PendingChunk(chunk)).await;
        }
        assert!(harness.take_events().is_empty(), "nothing before the commit");
        harness.deliver(Body::PendingCommit).await;

        let events = harness.take_events();
        assert!(
            matches!(
                events.as_slice(),
                [MobileEvent::Ask { ask, .. }, MobileEvent::Pick { pick, .. }] if ask.id == "a1" && pick.id == "p1"
            ),
            "{events:?}"
        );
        assert!(answer(&harness, "p1", PickReply::Picked { item: "web".to_owned() }));
        assert_eq!(harness.received().await.bodies, [resolve("p1", 5, Some("web"), None)]);
    });
}

#[test]
fn a_held_batch_that_does_not_decode_applies_nothing() {
    local(async {
        let mut harness = connected().await;
        let mut chunks = held_chunks();
        chunks[0].data = "not base64!".to_owned();

        for chunk in chunks {
            harness.deliver(Body::PendingChunk(chunk)).await;
        }
        harness.deliver(Body::PendingCommit).await;

        assert!(harness.take_events().is_empty());
        assert!(!answer(&harness, "p1", PickReply::Cancelled));
    });
}

#[test]
fn a_held_page_is_refused_as_unsupported() {
    local(async {
        let mut harness = connected().await;
        let chunks = PendingBatch::chunks(&[Body::PageRequest(super::url_page("p1"))]).unwrap();

        for chunk in chunks {
            harness.deliver(Body::PendingChunk(chunk)).await;
        }
        harness.deliver(Body::PendingCommit).await;

        assert!(harness.take_events().is_empty());
        assert_eq!(
            harness.received().await.bodies,
            [Body::PageRejected(PresentationPageChange {
                page: "p1".to_owned(),
                reason: Some(PAGES_UNSUPPORTED.to_owned()),
            })]
        );
    });
}
