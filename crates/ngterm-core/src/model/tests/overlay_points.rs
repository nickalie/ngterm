//! A floating overlay sized in points per axis, ported from upstream's `OverlayPointSizeTests`: the sizing table, the
//! slot's point sizes through open, resize and close, the refusals, and the tree's read-back from the cached layout.

use uuid::Uuid;

use super::support::add;
use crate::html::{HtmlOverlay, HtmlPageFlags, HtmlSource};
use crate::hud::{HudPaneFrame, HudPaneFrames, HudPanelSize, HudSpec};
use crate::model::{
    AppStore, HudOpening, OverlayPointResizeFailure, OverlayPointSize, OverlayProgram, OverlaySize, content_size,
    panel_size,
};
use crate::projection::session_node;

const BOUNDS: (f64, f64) = (1_000.0, 800.0);

fn points(width: Option<i64>, height: Option<i64>) -> OverlayPointSize {
    OverlayPointSize { width, height }
}

fn height(points: i64) -> OverlayPointSize {
    OverlayPointSize {
        width: None,
        height: Some(points),
    }
}

fn launched() -> (AppStore, Uuid) {
    let mut store = AppStore::new();
    let ws = store.add_workspace("work");
    let id = add(&mut store, ws, "/a");
    (store, id)
}

fn program(size_percent: Option<i64>, points: OverlayPointSize) -> OverlayProgram {
    OverlayProgram {
        command: "htop".to_owned(),
        size_percent,
        points,
        ..OverlayProgram::default()
    }
}

fn page() -> HtmlOverlay {
    HtmlOverlay::new(
        HtmlSource::File {
            path: "/tmp/a/page.html".to_owned(),
            grant_root: None,
        },
        HtmlPageFlags::default(),
    )
}

fn slot_points(store: &AppStore, id: Uuid) -> OverlayPointSize {
    store.session(id).unwrap().overlay.points
}

fn laid_out(store: &mut AppStore, id: Uuid) {
    store.session_mut(id).unwrap().hud_frames = HudPaneFrames {
        detail: Some(HudPaneFrame {
            x: 0.0,
            y: 0.0,
            width: BOUNDS.0,
            height: BOUNDS.1,
        }),
        ..HudPaneFrames::default()
    };
}

fn read_back(store: &AppStore, id: Uuid) -> (Option<f64>, Option<f64>) {
    let node = session_node(store.session(id).unwrap(), true);
    (node.overlay_width, node.overlay_height)
}

#[test]
fn panel_size_overrides_only_the_point_axes_and_caps_them_after_the_chrome() {
    let cases = [
        (OverlayPointSize::default(), 0.0, (600.0, 480.0)),
        (height(240), 0.0, (600.0, 240.0)),
        (height(240), 29.0, (600.0, 269.0)),
        (points(Some(300), None), 0.0, (300.0, 480.0)),
        (points(Some(5_000), Some(5_000)), 0.0, (800.0, 640.0)),
        (height(630), 29.0, (600.0, 640.0)),
    ];
    for (given, chrome, expected) in cases {
        assert_eq!(panel_size(BOUNDS, 60, given, chrome), expected, "{given:?} {chrome}");
    }
}

#[test]
fn a_percent_axis_keeps_the_full_percent_range() {
    assert!((panel_size(BOUNDS, 95, height(100), 0.0).0 - 950.0).abs() < 1e-9);
}

#[test]
fn content_size_leaves_out_the_chrome() {
    assert_eq!(content_size(BOUNDS, 60, height(240), 29.0), (600.0, 240.0));
    assert_eq!(
        content_size((1_000.0, 20.0), 60, height(240), 29.0),
        (600.0, 0.0),
        "never below zero"
    );
}

#[test]
fn merging_replaces_only_the_axes_it_sets() {
    let merged = points(Some(500), Some(200)).merging(height(300));
    assert_eq!(merged, points(Some(500), Some(300)));
}

#[test]
fn a_page_carries_its_strip_and_its_rule_unless_chromeless() {
    assert!((page().chrome() - 29.0).abs() < 1e-9);
    assert!((HtmlOverlay::CHROME_HEIGHT - HtmlOverlay::STRIP_HEIGHT - 1.0).abs() < 1e-9);
    let mut chromeless = page();
    chromeless.flags.chromeless = true;
    assert!(chromeless.chrome().abs() < 1e-9);
}

#[test]
fn open_keeps_points_only_beside_a_percent() {
    let (mut store, id) = launched();
    assert!(store.open_overlay(id, program(Some(60), height(240))));
    assert_eq!(slot_points(&store, id), height(240));
    assert!(store.close_overlay(id));
    assert!(slot_points(&store, id).is_empty());

    assert!(store.open_overlay(id, program(None, height(240))));
    assert!(slot_points(&store, id).is_empty());
}

#[test]
fn a_page_opens_with_its_points() {
    let (mut store, id) = launched();
    let size = OverlaySize {
        percent: Some(60),
        points: points(Some(500), None),
    };
    assert_eq!(store.open_html_overlay_sized(id, None, page(), size, None), Ok(()));
    assert_eq!(slot_points(&store, id), points(Some(500), None));
}

#[test]
fn a_percent_resize_replaces_the_points_whole_and_full_clears_them() {
    let (mut store, id) = launched();
    assert!(store.open_overlay(id, program(Some(60), points(Some(500), Some(240)))));

    assert!(store.resize_overlay_with_points(id, Some(70), height(300)));
    assert_eq!(store.session(id).unwrap().overlay.size_percent, Some(70));
    assert_eq!(slot_points(&store, id), height(300));

    assert!(store.resize_overlay(id, Some(50)));
    assert!(slot_points(&store, id).is_empty());

    assert!(store.resize_overlay_with_points(id, Some(50), height(300)));
    assert!(store.resize_overlay(id, None));
    assert!(slot_points(&store, id).is_empty());
    assert!(store.session(id).unwrap().full_overlay_active());
}

#[test]
fn points_alone_change_their_axes_and_keep_the_rest() {
    let (mut store, id) = launched();
    assert!(store.open_overlay(id, program(Some(60), points(Some(500), None))));

    assert_eq!(store.resize_overlay_in_points(id, None, height(240)), Ok(()));

    assert_eq!(store.session(id).unwrap().overlay.size_percent, Some(60));
    assert_eq!(slot_points(&store, id), points(Some(500), Some(240)));
}

#[test]
fn points_alone_need_a_floating_program_or_page() {
    let (mut store, id) = launched();
    assert_eq!(
        store.resize_overlay_in_points(id, None, height(240)),
        Err(OverlayPointResizeFailure::NoOverlay)
    );
    assert_eq!(
        store.resize_overlay_in_points(Uuid::new_v4(), None, height(240)),
        Err(OverlayPointResizeFailure::NoOverlay)
    );

    assert!(store.open_overlay(id, program(None, OverlayPointSize::default())));
    assert_eq!(
        store.resize_overlay_in_points(id, None, height(240)),
        Err(OverlayPointResizeFailure::NotFloating)
    );
    assert_eq!(store.resize_overlay_in_points(id, Some(60), height(240)), Ok(()));
    assert_eq!(slot_points(&store, id), height(240));
}

#[test]
fn a_hud_takes_no_point_sizes() {
    let (mut store, id) = launched();
    let dir = tempfile::tempdir().unwrap();
    let opening = HudOpening {
        command: "hud.sh".to_owned(),
        file: dir.path().join("hud"),
        size: HudPanelSize::new(35, 12),
        pane_identity: None,
        font_size: None,
    };
    assert!(store.open_hud(id, HudSpec::new("working"), opening));

    assert_eq!(
        store.resize_overlay_in_points(id, Some(40), height(240)),
        Err(OverlayPointResizeFailure::Hud)
    );

    assert!(slot_points(&store, id).is_empty());
    assert_ne!(store.session(id).unwrap().overlay.size_percent, Some(40));
}

#[test]
fn a_replica_of_another_machines_page_takes_no_point_sizes() {
    let (mut store, id) = launched();
    let mut replica = page();
    replica.replica_of = Some(Uuid::new_v4());
    assert_eq!(store.open_html_overlay(id, None, replica, Some(60), None), Ok(()));

    assert_eq!(
        store.resize_overlay_in_points(id, None, height(240)),
        Err(OverlayPointResizeFailure::Remote)
    );

    assert!(slot_points(&store, id).is_empty());
}

#[test]
fn the_tree_reports_the_content_size_from_the_last_layout() {
    let (mut store, id) = launched();
    let size = OverlaySize {
        percent: Some(60),
        points: height(240),
    };
    assert_eq!(store.open_html_overlay_sized(id, None, page(), size, None), Ok(()));
    assert_eq!(read_back(&store, id), (None, None), "not laid out yet");

    laid_out(&mut store, id);
    assert_eq!(read_back(&store, id), (Some(600.0), Some(240.0)));

    assert!(store.resize_overlay(id, None));
    assert_eq!(read_back(&store, id), (None, None), "a full overlay");
}

#[test]
fn the_tree_reports_a_percent_panels_content_too() {
    let (mut store, id) = launched();
    laid_out(&mut store, id);
    assert!(store.open_overlay(id, program(Some(50), OverlayPointSize::default())));
    assert_eq!(read_back(&store, id), (Some(500.0), Some(400.0)));
}

#[test]
fn the_tree_leaves_out_a_hud_and_an_empty_slot() {
    let (mut store, id) = launched();
    laid_out(&mut store, id);
    assert_eq!(read_back(&store, id), (None, None));
    let dir = tempfile::tempdir().unwrap();
    let opening = HudOpening {
        command: "hud.sh".to_owned(),
        file: dir.path().join("hud"),
        size: HudPanelSize::new(35, 12),
        pane_identity: None,
        font_size: None,
    };
    assert!(store.open_hud(id, HudSpec::new("working"), opening));
    assert_eq!(read_back(&store, id), (None, None));
}
