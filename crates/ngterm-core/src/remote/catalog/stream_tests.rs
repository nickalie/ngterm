//! The catalog stream's frames and states (upstream's `RemoteCatalogMirrorTests` stream cases).

use ngterm_proto::CatalogVersions;
use uuid::Uuid;

use super::{CatalogState, CatalogStream, version_refusal};
use crate::catalog::fixture::{CATALOG, session, snapshot, versions, workspace};
use crate::catalog::frames::{self, CatalogFrame, CatalogHello};

fn bound(_: Uuid) -> bool {
    true
}

fn hello(epoch: &str, versions: CatalogVersions) -> CatalogFrame {
    CatalogFrame::Hello(CatalogHello {
        version: 1,
        epoch: Some(epoch.to_owned()),
        versions: Some(versions),
        page_rows: None,
    })
}

#[test]
fn each_state_has_its_wire_name_and_window_notice() {
    let cases = [
        (
            CatalogState::Connecting,
            ("connecting", None),
            Some("Connecting to box…"),
        ),
        (CatalogState::Connected, ("connected", None), None),
        (
            CatalogState::Disconnected("gone".to_owned()),
            ("disconnected", Some("gone")),
            Some("Disconnected: gone. Retrying."),
        ),
        (
            CatalogState::Refused("no".to_owned()),
            ("refused", Some("no")),
            Some("no"),
        ),
    ];
    for (state, wire, notice) in cases {
        assert_eq!(state.wire(), wire);
        assert_eq!(state.window_notice("box").as_deref(), notice);
    }
}

#[test]
fn an_origin_without_a_catalog_is_named_by_what_it_runs() {
    assert_eq!(
        version_refusal("box", None, None).as_deref(),
        Some(
            "box runs an ngterm without a catalog, an app with windows rather than ngterm-headless; a remote window \
             needs ngterm-headless"
        )
    );
    let old = CatalogVersions {
        protocol: 0,
        ..versions()
    };
    assert_eq!(
        version_refusal("box", Some(&old), None).as_deref(),
        Some("box speaks presentation 0, this Mac speaks presentation 1")
    );
    assert_eq!(version_refusal("box", Some(&versions()), None), None);
}

#[test]
fn a_hello_speaking_too_old_a_presentation_refuses_for_good() {
    let mut stream = CatalogStream::new("box");
    let old = CatalogVersions {
        protocol: 0,
        ..versions()
    };
    stream.receive(hello("e1", old), &mut bound);
    assert!(stream.state().is_refused());
    let received = stream.receive(CatalogFrame::Ping, &mut bound);
    assert_eq!(received.reply, None, "a refused stream answers nothing");
    stream.subscribing();
    stream.connection_lost("x".to_owned());
    assert!(stream.state().is_refused());
}

#[test]
fn a_snapshot_of_another_epoch_than_the_hello_asks_for_a_fresh_one() {
    let mut stream = CatalogStream::new("box");
    stream.receive(hello("other", versions()), &mut bound);
    let received = stream.receive(CatalogFrame::Snapshot(snapshot(Vec::new())), &mut bound);
    assert_eq!(received.moved, None);
    assert!(stream.needs_snapshot());
    assert!(stream.catalog().is_none());
}

#[test]
fn a_chunked_snapshot_applies_only_at_its_commit() {
    let mut stream = CatalogStream::new("box");
    let whole = snapshot(vec![workspace(Uuid::new_v4(), "w", vec![session("a")])]);
    let frames = frames::snapshot_frames(&whole).unwrap();
    let mut moved = Vec::new();
    for frame in frames {
        moved.push(stream.receive(frame, &mut bound).moved);
    }
    assert_eq!(moved.last(), Some(&Some(true)));
    assert_eq!(stream.catalog(), Some(&whole));
    assert_eq!(stream.state(), &CatalogState::Connected);
}

#[test]
fn bad_chunks_or_a_commit_without_them_ask_for_a_snapshot() {
    let mut stream = CatalogStream::new("box");
    stream.receive(
        CatalogFrame::Chunk {
            index: 3,
            data: "x".to_owned(),
        },
        &mut bound,
    );
    assert!(stream.needs_snapshot());
    let mut stream = CatalogStream::new("box");
    stream.receive(
        CatalogFrame::Commit {
            epoch: "e1".to_owned(),
            rev: 0,
            chunks: 2,
        },
        &mut bound,
    );
    assert!(stream.needs_snapshot());
}

#[test]
fn a_catalog_another_window_shows_refuses_the_stream() {
    let mut stream = CatalogStream::new("box");
    let mut seen = None;
    let mut taken = |catalog| {
        seen = Some(catalog);
        false
    };
    stream.receive(CatalogFrame::Snapshot(snapshot(Vec::new())), &mut taken);
    assert_eq!(seen, Some(CATALOG));
    assert_eq!(
        stream.state(),
        &CatalogState::Refused("another window already shows the catalog of box".to_owned())
    );
    assert!(stream.catalog().is_none());
}

#[test]
fn a_revision_already_seen_is_dropped_without_asking_for_a_snapshot() {
    let mut stream = CatalogStream::new("box");
    stream.receive(CatalogFrame::Snapshot(snapshot(Vec::new())), &mut bound);
    let received = stream.receive(
        CatalogFrame::Unknown {
            kind: "x".to_owned(),
            epoch: Some("e1".to_owned()),
            rev: Some(0),
        },
        &mut bound,
    );
    assert_eq!(received.moved, None);
    assert!(!stream.needs_snapshot());
    assert_eq!(
        stream.receive(CatalogFrame::Ack, &mut bound),
        super::Received::default()
    );
}
