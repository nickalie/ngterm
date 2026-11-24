use uuid::Uuid;

use super::super::fixture::{self, companion, page_row, pane, session, session_with, snapshot, workspace};
use super::*;

fn round_trip(frame: &CatalogFrame) -> CatalogFrame {
    let line = encode(frame).unwrap();
    assert_eq!(line.last(), Some(&b'\n'));
    decode(&line[..line.len() - 1]).unwrap()
}

/// Upstream's `anOversizedSnapshotGoesAsChunksAppliedOnlyAtTheirCommit`.
#[test]
fn an_oversized_snapshot_goes_as_chunks_applied_only_at_their_commit() {
    let sessions = (0..1500)
        .map(|index| session(&format!("session {index} {}", "x".repeat(100))))
        .collect();
    let big = snapshot(vec![workspace(Uuid::new_v4(), "w", sessions)]);
    let frames = snapshot_frames(&big).unwrap();
    let mut chunks = CatalogChunks::default();
    let mut assembled = None;
    for frame in &frames {
        match round_trip(frame) {
            CatalogFrame::Chunk { index, data } => chunks.add(index, &data).unwrap(),
            CatalogFrame::Commit {
                epoch,
                rev,
                chunks: count,
            } => {
                assembled = Some(chunks.commit(&epoch, rev, count).unwrap());
            }
            other => panic!("unexpected frame {other:?}"),
        }
    }
    assert!(frames.len() > 2);
    assert_eq!(assembled, Some(big));
}

/// Upstream's `aSmallSnapshotGoesWhole`.
#[test]
fn a_small_snapshot_goes_whole() {
    let small = snapshot(vec![workspace(Uuid::new_v4(), "w", vec![session("s")])]);
    assert_eq!(snapshot_frames(&small).unwrap(), [CatalogFrame::Snapshot(small)]);
}

/// Upstream's `chunksOutOfOrderOrShortOfTheirCommitAreRefused`.
#[test]
fn chunks_out_of_order_or_short_of_their_commit_are_refused() {
    let mut skipped = CatalogChunks::default();
    assert_eq!(skipped.add(1, "AA=="), Err(ChunkError::OutOfOrder(1)));
    let mut bad = CatalogChunks::default();
    assert_eq!(bad.add(0, "not base64!"), Err(ChunkError::OutOfOrder(0)));
    let mut short = CatalogChunks::default();
    short
        .add(0, &base64::engine::general_purpose::STANDARD.encode("{}"))
        .unwrap();
    assert_eq!(
        short.commit("e1", 0, 2),
        Err(ChunkError::Incomplete { expected: 2, got: 1 })
    );
    let mut garbage = CatalogChunks::default();
    garbage
        .add(0, &base64::engine::general_purpose::STANDARD.encode("{}"))
        .unwrap();
    assert!(matches!(garbage.commit("e1", 0, 1), Err(ChunkError::Malformed(_))));
}

/// Upstream's `aCommitForAnotherRevisionIsRefused`.
#[test]
fn a_commit_for_another_revision_is_refused() {
    let data = serde_json::to_vec(&snapshot(vec![])).unwrap();
    let mut chunks = CatalogChunks::default();
    chunks
        .add(0, &base64::engine::general_purpose::STANDARD.encode(data))
        .unwrap();
    assert_eq!(chunks.commit("e1", 7, 1), Err(ChunkError::Mismatched));
}

/// Upstream's `everyFrameRoundTrips`.
#[test]
fn every_frame_round_trips() {
    let frames = [
        CatalogFrame::Hello(CatalogHello {
            version: 1,
            epoch: Some("e1".to_owned()),
            versions: Some(fixture::versions()),
            page_rows: None,
        }),
        CatalogFrame::Ping,
        CatalogFrame::Ack,
        CatalogFrame::Chunk {
            index: 2,
            data: "AA==".to_owned(),
        },
        CatalogFrame::Commit {
            epoch: "e1".to_owned(),
            rev: 4,
            chunks: 3,
        },
        CatalogFrame::Snapshot(CatalogSnapshot {
            rev: 3,
            ..snapshot(vec![])
        }),
        CatalogFrame::Hello(CatalogHello {
            version: 1,
            page_rows: Some(true),
            ..CatalogHello::default()
        }),
        CatalogFrame::Unknown {
            kind: SKIP_KIND.to_owned(),
            epoch: Some("e1".to_owned()),
            rev: Some(14),
        },
    ];
    for frame in frames {
        assert_eq!(round_trip(&frame), frame);
    }
}

/// Upstream's `everyFrameRoundTrips`, its deltas.
#[test]
fn every_delta_round_trips() {
    let id = Uuid::new_v4;
    let delta = |rev, change| CatalogFrame::Delta {
        epoch: "e1".to_owned(),
        rev,
        change,
    };
    let frames = [
        delta(
            1,
            CatalogChange::WorkspaceAdded {
                workspace: workspace(id(), "w", vec![session("s")]),
                index: 0,
            },
        ),
        delta(
            2,
            CatalogChange::WorkspaceChanged {
                id: id(),
                name: "dev".to_owned(),
            },
        ),
        delta(3, CatalogChange::WorkspaceRemoved(id())),
        delta(4, CatalogChange::WorkspaceOrder(vec![id(), id()])),
        delta(
            5,
            CatalogChange::SessionAdded {
                session: session_with(id(), "s", vec![pane(), pane()]),
                workspace: id(),
                index: 1,
            },
        ),
        delta(6, CatalogChange::SessionChanged(companion("c", id()).fields)),
        delta(7, CatalogChange::SessionRemoved(id())),
        delta(
            8,
            CatalogChange::SessionMoved {
                id: id(),
                workspace: id(),
                index: 2,
            },
        ),
        delta(
            9,
            CatalogChange::SessionOrder {
                ids: vec![id()],
                workspace: id(),
            },
        ),
        delta(
            10,
            CatalogChange::PaneAdded {
                pane: pane(),
                session: id(),
            },
        ),
    ];
    for frame in frames {
        assert_eq!(round_trip(&frame), frame);
    }
}

#[test]
fn every_late_delta_round_trips() {
    let id = Uuid::new_v4;
    let delta = |rev, change| CatalogFrame::Delta {
        epoch: "e1".to_owned(),
        rev,
        change,
    };
    let frames = [
        delta(
            11,
            CatalogChange::PaneChanged {
                pane: pane(),
                session: id(),
            },
        ),
        delta(
            12,
            CatalogChange::PaneRemoved {
                identity: id(),
                session: id(),
            },
        ),
        delta(
            13,
            CatalogChange::SessionAdded {
                session: page_row(),
                workspace: id(),
                index: 0,
            },
        ),
    ];
    for frame in frames {
        assert_eq!(round_trip(&frame), frame);
    }
}

#[test]
fn ids_travel_as_foundation_spells_them() {
    let id = Uuid::from_u128(0xabcd_ef01_2345_4678_9abc_def0_1234_5678);
    let line = encode(&CatalogFrame::Delta {
        epoch: "e1".to_owned(),
        rev: 1,
        change: CatalogChange::SessionRemoved(id),
    })
    .unwrap();
    let text = String::from_utf8(line).unwrap();
    assert_eq!(
        text,
        "{\"kind\":\"session.removed\",\"epoch\":\"e1\",\"rev\":1,\"id\":\"ABCDEF01-2345-4678-9ABC-DEF012345678\"}\n"
    );
}

/// Upstream's `anUnknownKindDecodesAsUnknown`.
#[test]
fn an_unknown_kind_decodes_as_unknown() {
    let line = br#"{"kind":"session.teleported","epoch":"e1","rev":3,"id":"x"}"#;
    assert_eq!(
        decode(line),
        Ok(CatalogFrame::Unknown {
            kind: "session.teleported".to_owned(),
            epoch: Some("e1".to_owned()),
            rev: Some(3),
        })
    );
    assert_eq!(
        decode(br#"{"kind":"later"}"#),
        Ok(CatalogFrame::Unknown {
            kind: "later".to_owned(),
            epoch: None,
            rev: None,
        })
    );
}

#[test]
fn a_line_that_is_no_frame_is_refused() {
    for line in [
        &b"[]"[..],
        b"{}",
        b"not json",
        br#"{"kind":"session.removed","epoch":"e1","rev":1}"#,
    ] {
        assert!(matches!(decode(line), Err(FrameError::Malformed(_))), "{line:?}");
    }
    let long = vec![b' '; MAX_FRAME_BYTES + 1];
    assert_eq!(decode(&long), Err(FrameError::Oversize(MAX_FRAME_BYTES + 1)));
    let huge = CatalogFrame::Chunk {
        index: 0,
        data: "x".repeat(MAX_FRAME_BYTES),
    };
    assert!(matches!(encode(&huge), Err(FrameError::Oversize(_))));
}

/// Upstream's `aHelloAndASessionWithoutPageFieldsDecodeAsBefore`.
#[test]
fn a_hello_and_a_session_without_page_fields_decode_as_before() {
    assert_eq!(
        decode(br#"{"kind":"hello","hello":{"version":1}}"#),
        Ok(CatalogFrame::Hello(CatalogHello {
            version: 1,
            ..CatalogHello::default()
        }))
    );
    let encoded = serde_json::to_string(&session("s").fields).unwrap();
    assert!(!encoded.contains("page"), "{encoded}");
    let decoded: CatalogSessionFields = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded.page, None);
}

/// Upstream's `anEmptyCustomNameTravelsAsEmptyAndAnOlderServerOmitsIt`.
#[test]
fn an_empty_custom_name_travels_as_empty_and_an_older_server_omits_it() {
    let mut unnamed = session("s").fields;
    unnamed.custom_name = Some(String::new());
    let decoded: CatalogSessionFields = serde_json::from_str(&serde_json::to_string(&unnamed).unwrap()).unwrap();
    assert_eq!(decoded.custom_name.as_deref(), Some(""));
    let legacy = serde_json::to_string(&session("s").fields).unwrap();
    assert!(!legacy.contains("customName"), "{legacy}");
    let decoded: CatalogSessionFields = serde_json::from_str(&legacy).unwrap();
    assert_eq!(decoded.custom_name, None);
}

/// Upstream's `versionsTravelWithTheProtocolKeyword`.
#[test]
fn versions_travel_with_the_protocol_keyword() {
    let json = serde_json::to_string(&fixture::versions()).unwrap();
    assert!(json.contains(r#""protocol":1"#), "{json}");
}

#[test]
fn a_session_decodes_with_its_primary_pane_first() {
    let (left, right) = (pane(), pane());
    let mut wire = serde_json::to_value(session_with(Uuid::new_v4(), "s", vec![left.clone(), right.clone()])).unwrap();
    wire["primary"] = serde_json::Value::String(crate::pane_env::uuid_string(right.identity));
    let decoded: CatalogSession = serde_json::from_value(wire).unwrap();
    assert_eq!(decoded.panes, [right, left]);
}
