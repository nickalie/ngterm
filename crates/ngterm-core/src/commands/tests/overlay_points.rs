//! `session.overlay.open|resize --width/--height` (upstream's `ControlDispatcherOverlayTests` point cases and the
//! control server's `resizeSessionOverlay` points arm): the refusals in upstream's order with nothing changed, the
//! sizes a program or page keeps, and the read-back once the session is laid out.

use ngterm_proto::command::SessionCmd;
use ngterm_proto::{Args, Command, Response};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{Fixture, answered_id, args, failure, wire};
use crate::html::HtmlHost;
use crate::hud::{HudPaneFrame, HudPaneFrames};
use crate::model::OverlayPointSize;

const NEEDS_PERCENT: &str = "session.overlay.open: --width/--height need --size-percent";
const NOT_POSITIVE: &str = "session.overlay: --width/--height must be positive";
const FULL_CONFLICT: &str = "session.overlay.resize: --full is mutually exclusive with --width/--height";
const PANE_CONFLICT: &str = "session.overlay.open: --pane is mutually exclusive with --width/--height";
const PANE_RESIZE: &str = "session.overlay.resize: --pane is not supported (pane overlays are always full)";
const NOT_FLOATING: &str = "session.overlay.resize: --width/--height need a floating overlay; pass --size-percent too";
const HUD: &str = "a hud measures its own height: --width/--height do not apply";

fn send(fixture: &mut Fixture, cmd: SessionCmd, id: Uuid, value: Value) -> Response {
    let args: Args = serde_json::from_value(value).expect("valid args");
    fixture.session(cmd, Some(&wire(id)), args)
}

fn slot_points(fixture: &Fixture, id: Uuid) -> OverlayPointSize {
    fixture.store().session(id).unwrap().overlay.points
}

fn size_percent(fixture: &Fixture, id: Uuid) -> Option<i64> {
    fixture.store().session(id).unwrap().overlay.size_percent
}

fn points(width: Option<i64>, height: Option<i64>) -> OverlayPointSize {
    OverlayPointSize { width, height }
}

fn node(fixture: &mut Fixture, id: Uuid) -> Value {
    let response = fixture.send(Command::Tree, None, Args::default());
    let tree = response.result.expect("a tree")["tree"].clone();
    tree["workspaces"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|workspace| workspace["sessions"].as_array().cloned().unwrap_or_default())
        .find(|node| node["id"] == json!(wire(id)))
        .expect("the session's node")
}

/// Web views that can run.
struct Host;

impl HtmlHost for Host {
    fn persistent_store_failure(&self) -> Option<String> {
        None
    }

    fn navigate(&self, _: Uuid, _: crate::html::HtmlNavigation) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn open_routes_points_beside_the_percent() {
    let mut fixture = Fixture::new(1);
    let id = fixture.sessions[0];
    let opened = send(
        &mut fixture,
        SessionCmd::OverlayOpen,
        id,
        json!({"command": "cat", "sizePercent": 60, "width": 500, "height": 240}),
    );
    assert_eq!(answered_id(&opened), Some(id));
    assert_eq!(size_percent(&fixture, id), Some(60));
    assert_eq!(slot_points(&fixture, id), points(Some(500), Some(240)));
}

#[test]
fn open_refuses_bad_points_in_order_and_opens_nothing() {
    let mut fixture = Fixture::new(1);
    let id = fixture.sessions[0];
    for (request, error) in [
        (json!({"command": "cat", "height": 240}), NEEDS_PERCENT),
        (json!({"command": "cat", "sizePercent": 60, "height": 0}), NOT_POSITIVE),
        (json!({"command": "cat", "sizePercent": 60, "width": -5}), NOT_POSITIVE),
        (json!({"command": "cat", "pane": "right", "width": 300}), PANE_CONFLICT),
        (
            json!({"command": "cat", "pane": "right", "sizePercent": 60, "width": 300}),
            "session.overlay.open: --pane is mutually exclusive with --size-percent",
        ),
        (
            json!({"command": "cat", "sizePercent": 0, "width": 0}),
            "session.overlay.open: --size-percent must be 1...100",
        ),
        (json!({"command": "cat", "width": 0}), NOT_POSITIVE),
    ] {
        assert_eq!(
            send(&mut fixture, SessionCmd::OverlayOpen, id, request.clone()),
            failure(error),
            "{request}"
        );
    }
    assert_eq!(node(&mut fixture, id)["overlay"], json!(false));
}

#[test]
fn a_page_opens_with_its_points() {
    let mut fixture = Fixture::new(1);
    fixture.core.set_html_host(Box::new(Host));
    let id = fixture.sessions[0];
    let opened = send(
        &mut fixture,
        SessionCmd::OverlayOpen,
        id,
        json!({"html": "/tmp/a/page.html", "sizePercent": 60, "height": 240}),
    );
    assert!(opened.ok, "{opened:?}");
    assert_eq!(slot_points(&fixture, id), points(None, Some(240)));
    assert_eq!(
        send(
            &mut fixture,
            SessionCmd::OverlayOpen,
            id,
            json!({"html": "/tmp/a/page.html", "height": 240})
        ),
        failure(NEEDS_PERCENT)
    );
}

#[test]
fn resize_routes_points_with_and_without_a_percent() {
    let mut fixture = Fixture::new(1);
    let id = fixture.sessions[0];
    let resize = |fixture: &mut Fixture, value: Value| send(fixture, SessionCmd::OverlayResize, id, value);
    assert_eq!(resize(&mut fixture, json!({"height": 240})), failure("no overlay"));
    assert!(
        send(
            &mut fixture,
            SessionCmd::OverlayOpen,
            id,
            json!({"command": "cat", "sizePercent": 60, "width": 500})
        )
        .ok
    );

    assert_eq!(answered_id(&resize(&mut fixture, json!({"height": 240}))), Some(id));
    assert_eq!(
        (size_percent(&fixture, id), slot_points(&fixture, id)),
        (Some(60), points(Some(500), Some(240)))
    );

    assert_eq!(
        answered_id(&resize(&mut fixture, json!({"sizePercent": 70, "width": 700}))),
        Some(id)
    );
    assert_eq!(
        (size_percent(&fixture, id), slot_points(&fixture, id)),
        (Some(70), points(Some(700), None))
    );

    assert!(resize(&mut fixture, json!({"sizePercent": 50})).ok);
    assert!(slot_points(&fixture, id).is_empty(), "a percent alone drops them");

    assert!(resize(&mut fixture, json!({"sizePercent": 50, "height": 300})).ok);
    assert!(resize(&mut fixture, json!({"full": true})).ok);
    assert!(slot_points(&fixture, id).is_empty(), "full drops them");
    assert_eq!(resize(&mut fixture, json!({"height": 240})), failure(NOT_FLOATING));
    assert_eq!(size_percent(&fixture, id), None);
}

#[test]
fn resize_refuses_bad_points_and_changes_nothing() {
    let mut fixture = Fixture::new(1);
    let id = fixture.sessions[0];
    assert!(
        send(
            &mut fixture,
            SessionCmd::OverlayOpen,
            id,
            json!({"command": "cat", "sizePercent": 60, "width": 500})
        )
        .ok
    );
    for (request, error) in [
        (json!({"full": true, "height": 240}), FULL_CONFLICT),
        (json!({"height": 0}), NOT_POSITIVE),
        (json!({"sizePercent": 60, "width": -5}), NOT_POSITIVE),
        (json!({"pane": "right", "height": 240}), PANE_RESIZE),
        (
            json!({"full": true, "sizePercent": 50, "height": 240}),
            "session.overlay.resize: --full is mutually exclusive with --size-percent",
        ),
        (
            json!({"sizePercent": 101, "height": 0}),
            "session.overlay.resize: --size-percent must be 1...100",
        ),
    ] {
        assert_eq!(
            send(&mut fixture, SessionCmd::OverlayResize, id, request.clone()),
            failure(error),
            "{request}"
        );
    }
    assert_eq!(
        (size_percent(&fixture, id), slot_points(&fixture, id)),
        (Some(60), points(Some(500), None))
    );
}

#[test]
fn a_hud_measures_its_own_height() {
    let mut fixture = Fixture::new(1);
    let id = fixture.sessions[0];
    let dir = tempfile::tempdir().unwrap();
    let opening = crate::model::HudOpening {
        command: "hud.sh".to_owned(),
        file: dir.path().join("hud"),
        size: crate::hud::HudPanelSize::new(35, 12),
        pane_identity: None,
        font_size: None,
    };
    assert!(
        fixture
            .store_mut()
            .unwrap()
            .open_hud(id, crate::hud::HudSpec::new("working"), opening)
    );
    let resized = send(
        &mut fixture,
        SessionCmd::OverlayResize,
        id,
        json!({"sizePercent": 40, "height": 240}),
    );
    assert_eq!(resized, failure(HUD));
    assert_ne!(size_percent(&fixture, id), Some(40));
}

#[test]
fn the_tree_reads_the_content_size_back_once_laid_out() {
    let mut fixture = Fixture::new(1);
    fixture.core.set_html_host(Box::new(Host));
    let id = fixture.sessions[0];
    let opened = send(
        &mut fixture,
        SessionCmd::OverlayOpen,
        id,
        json!({"html": "/tmp/a/page.html", "sizePercent": 60, "height": 240}),
    );
    assert!(opened.ok, "{opened:?}");
    let read = node(&mut fixture, id);
    assert!(read.get("overlayWidth").is_none() && read.get("overlayHeight").is_none());
    let frames = HudPaneFrames {
        detail: Some(HudPaneFrame {
            x: 0.0,
            y: 0.0,
            width: 1_000.0,
            height: 800.0,
        }),
        ..HudPaneFrames::default()
    };
    fixture.core.report_hud_frames(id, &frames);
    let read = node(&mut fixture, id);
    assert_eq!(
        (read["overlayWidth"].as_f64(), read["overlayHeight"].as_f64()),
        (Some(600.0), Some(240.0))
    );
    assert!(send(&mut fixture, SessionCmd::OverlayResize, id, json!({"height": 5_000})).ok);
    let read = node(&mut fixture, id);
    assert_eq!(
        read["overlayHeight"].as_f64(),
        Some(640.0 - 29.0),
        "capped with the strip, read without it"
    );
    assert!(send(&mut fixture, SessionCmd::OverlayResize, id, json!({"full": true})).ok);
    assert!(node(&mut fixture, id).get("overlayWidth").is_none());
}

#[test]
fn a_page_shown_here_for_another_machine_takes_no_point_sizes() {
    let mut fixture = Fixture::new(1);
    let id = fixture.sessions[0];
    let mut replica = crate::html::HtmlOverlay::new(
        crate::html::HtmlSource::Replica {
            path: "/tmp/a/page.html".to_owned(),
        },
        crate::html::HtmlPageFlags::default(),
    );
    replica.replica_of = Some(Uuid::new_v4());
    let store = fixture.store_mut().unwrap();
    assert_eq!(store.open_html_overlay(id, None, replica, Some(60), None), Ok(()));
    assert_eq!(
        fixture.session(
            SessionCmd::OverlayResize,
            Some(&wire(id)),
            args(|a| a.height = Some(240))
        ),
        failure("--width/--height are not supported for an overlay shown on another Mac")
    );
    assert!(slot_points(&fixture, id).is_empty());
}
