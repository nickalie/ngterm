//! The tree invalidation an overlay slot change owes, ported from upstream's `AppStoreEventTests` (the overlay, page,
//! HUD, split close, promotion and swap cases) and `AppStoreRemoteOverlayTests` (the reservation cases). Upstream counts
//! the coalesced `tree.changed` events after a flush, so a step here counts as one when it scheduled any.

use ngterm_host::fakes::FakeSurface;
use ngterm_proto::EventKind;
use uuid::Uuid;

use super::support::add;
use crate::html::{HtmlOverlay, HtmlPageFlags, HtmlSource};
use crate::hud::{HudPanelSize, HudSpec};
use crate::model::{
    AppStore, HudOpening, OverlayPane, OverlayProgram, RemoteOverlayOutcome, RemoteOverlaySlot, StatusPane,
};

/// A store with one workspace holding one selected session, its outbox drained (upstream's `launched`).
fn launched() -> (AppStore, Uuid) {
    let mut store = AppStore::new();
    let ws = store.add_workspace("work");
    let id = add(&mut store, ws, "/a");
    store.take_outbox();
    (store, id)
}

/// The coalesced `tree.changed` count `action` leaves: 1 when it scheduled any, else 0.
fn tree_changes(store: &mut AppStore, action: impl FnOnce(&mut AppStore)) -> usize {
    store.take_outbox();
    action(store);
    usize::from(
        store
            .take_outbox()
            .events
            .iter()
            .any(|event| event.kind == EventKind::TreeChanged),
    )
}

fn program(command: &str) -> OverlayProgram {
    OverlayProgram {
        command: command.to_owned(),
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

fn open_page(store: &mut AppStore, id: Uuid, pane: OverlayPane) {
    store.open_html_overlay(id, Some(pane), page(), None, None).unwrap();
}

/// Installs a fake terminal in pane `pane`, as upstream's tests set a `SpySurface`.
fn install(store: &mut AppStore, id: Uuid, pane: StatusPane) {
    store
        .session_mut(id)
        .unwrap()
        .pane_slot_mut(pane)
        .replace(Box::new(FakeSurface::new("t")));
}

fn hud_opening(dir: &tempfile::TempDir) -> HudOpening {
    HudOpening {
        file: dir.path().join("hud"),
        size: size(),
        pane_identity: None,
        font_size: None,
    }
}

fn size() -> HudPanelSize {
    HudPanelSize {
        width_percent: 30,
        height_percent: 9,
        height_points: None,
    }
}

fn slot(job: &str, pane: Option<OverlayPane>, wait: bool) -> RemoteOverlaySlot {
    RemoteOverlaySlot {
        job: job.to_owned(),
        pane,
        owner: 1,
        size_percent: None,
        wait,
        ended: false,
        page: None,
    }
}

#[test]
fn a_program_overlay_invalidates_the_tree_on_open_and_close_only() {
    let (mut store, id) = launched();
    assert_eq!(
        tree_changes(&mut store, |s| assert!(s.open_overlay(id, program("top")))),
        1
    );
    assert_eq!(
        tree_changes(&mut store, |s| assert!(!s.open_overlay(id, program("refused")))),
        0
    );
    assert_eq!(tree_changes(&mut store, |s| assert!(s.resize_overlay(id, Some(40)))), 0);
    assert_eq!(tree_changes(&mut store, |s| assert!(s.close_overlay(id))), 1);
    assert_eq!(tree_changes(&mut store, |s| assert!(!s.close_overlay(id))), 0);
}

#[test]
fn a_pane_overlay_invalidates_the_tree_on_open_and_close_only() {
    let (mut store, id) = launched();
    let left = OverlayPane::Left;
    assert_eq!(
        tree_changes(&mut store, |s| s.open_pane_overlay(id, left, program("top")).unwrap()),
        1
    );
    assert_eq!(
        tree_changes(&mut store, |s| assert!(
            s.open_pane_overlay(id, left, program("refused")).is_err()
        )),
        0
    );
    assert_eq!(tree_changes(&mut store, |s| assert!(s.close_pane_overlay(id, left))), 1);
    assert_eq!(
        tree_changes(&mut store, |s| assert!(!s.close_pane_overlay(id, left))),
        0
    );
}

#[test]
fn an_html_page_invalidates_the_tree_on_open_and_close() {
    for pane in [None, Some(OverlayPane::Left)] {
        let (mut store, id) = launched();
        assert_eq!(
            tree_changes(&mut store, |s| s
                .open_html_overlay(id, pane, page(), None, None)
                .unwrap()),
            1,
            "{pane:?}"
        );
        assert_eq!(
            tree_changes(&mut store, |s| assert!(match pane {
                Some(pane) => s.close_pane_overlay(id, pane),
                None => s.close_overlay(id),
            })),
            1,
            "{pane:?}"
        );
    }
}

#[test]
fn a_hud_invalidates_the_tree_on_open_and_close_but_not_on_update() {
    let (mut store, id) = launched();
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        tree_changes(&mut store, |s| assert!(s.open_hud(
            id,
            HudSpec::new("one"),
            hud_opening(&dir)
        ))),
        1
    );
    assert_eq!(
        tree_changes(&mut store, |s| assert!(s.update_hud(
            id,
            &HudSpec::new("two"),
            size(),
            None
        ))),
        0
    );
    assert_eq!(
        tree_changes(&mut store, |s| assert!(s.set_hud_size(id, size(), Some(40)))),
        0
    );
    assert_eq!(
        tree_changes(&mut store, |s| assert!(s.open_hud(
            id,
            HudSpec::new("three"),
            hud_opening(&dir)
        ))),
        1,
        "a HUD replacing a HUD"
    );
    assert_eq!(tree_changes(&mut store, |s| assert!(s.close_hud(id))), 1);
    assert_eq!(tree_changes(&mut store, |s| assert!(!s.close_hud(id))), 0);
}

#[test]
fn closing_a_split_invalidates_the_tree_only_for_a_page_it_takes_down() {
    for with_page in [true, false] {
        let (mut store, id) = launched();
        store.toggle_split(id, None);
        if with_page {
            open_page(&mut store, id, OverlayPane::Right);
        }
        assert_eq!(
            tree_changes(&mut store, |s| s.close_split(id)),
            usize::from(with_page),
            "with a page: {with_page}"
        );
        assert!(store.session(id).unwrap().pane_overlay(OverlayPane::Right).is_none());
    }
}

#[test]
fn promoting_the_split_invalidates_the_tree_only_for_a_page_that_changes_pane() {
    for with_page in [true, false] {
        let (mut store, id) = launched();
        store.toggle_split(id, None);
        install(&mut store, id, StatusPane::Right);
        if with_page {
            open_page(&mut store, id, OverlayPane::Right);
        }
        assert_eq!(
            tree_changes(&mut store, |s| {
                s.close_primary_pane(id);
            }),
            usize::from(with_page),
            "with a page: {with_page}"
        );
        assert_eq!(
            store.session(id).unwrap().pane_overlay(OverlayPane::Left).is_some(),
            with_page
        );
    }
}

#[test]
fn swapping_panes_invalidates_the_tree_only_for_a_page_that_changes_pane() {
    for with_page in [true, false] {
        let (mut store, id) = launched();
        install(&mut store, id, StatusPane::Left);
        store.toggle_split(id, None);
        install(&mut store, id, StatusPane::Right);
        if with_page {
            open_page(&mut store, id, OverlayPane::Left);
        }
        assert_eq!(
            tree_changes(&mut store, |s| s.swap_panes(id).unwrap()),
            usize::from(with_page),
            "with a page: {with_page}"
        );
        assert_eq!(
            store.session(id).unwrap().pane_overlay(OverlayPane::Right).is_some(),
            with_page
        );
    }
}

#[test]
fn a_remote_overlay_invalidates_the_tree_when_its_slot_is_taken_and_freed() {
    let (mut store, id) = launched();
    assert_eq!(
        tree_changes(&mut store, |s| assert!(
            s.reserve_remote_overlay(id, slot("j", None, false))
        )),
        1
    );
    assert_eq!(
        tree_changes(&mut store, |s| s.resize_remote_overlay(id, "j", Some(40))),
        0
    );
    assert_eq!(
        tree_changes(&mut store, |s| s.finish_remote_overlay(
            id,
            "j",
            &RemoteOverlayOutcome::Exited(0)
        )),
        1
    );
    assert_eq!(
        tree_changes(&mut store, |s| s.finish_remote_overlay(
            id,
            "j",
            &RemoteOverlayOutcome::Exited(0)
        )),
        0,
        "the slot is gone already"
    );
}

#[test]
fn a_held_remote_slot_invalidates_the_tree_only_once_it_is_freed() {
    let (mut store, id) = launched();
    assert!(store.reserve_remote_overlay(id, slot("j", None, true)));
    assert_eq!(
        tree_changes(&mut store, |s| s.finish_remote_overlay(
            id,
            "j",
            &RemoteOverlayOutcome::Exited(0)
        )),
        0
    );
    assert_eq!(
        tree_changes(&mut store, |s| s.remote_overlay_surface_closed(id, "j")),
        1
    );
}

#[test]
fn closing_rejecting_or_losing_a_remote_overlay_invalidates_the_tree() {
    let (mut store, id) = launched();
    assert!(store.reserve_remote_overlay(id, slot("a", None, false)));
    assert_eq!(
        tree_changes(&mut store, |s| assert!(s.close_remote_overlay(id, None))),
        0
    );
    assert_eq!(
        tree_changes(&mut store, |s| s.finish_remote_overlay(
            id,
            "a",
            &RemoteOverlayOutcome::Exited(1)
        )),
        1,
        "a running job keeps its slot until it ends"
    );
    assert!(store.reserve_remote_overlay(id, slot("b", None, true)));
    assert_eq!(
        tree_changes(&mut store, |s| assert!(s.reject_remote_overlay(id, "b"))),
        0,
        "the refused job's slot waits for its failure"
    );
    assert_eq!(
        tree_changes(&mut store, |s| s.finish_remote_overlay(
            id,
            "b",
            &RemoteOverlayOutcome::Failed("launch-failed".to_owned())
        )),
        1
    );
    assert!(store.reserve_remote_overlay(id, slot("c", Some(OverlayPane::Left), true)));
    store.finish_remote_overlay(id, "c", &RemoteOverlayOutcome::Exited(0));
    assert_eq!(
        tree_changes(&mut store, |s| {
            s.remote_overlay_presenter_lost(id);
        }),
        1,
        "the held surface went with its presenter"
    );
}

#[test]
fn promoting_the_split_invalidates_the_tree_for_a_reservation_that_changes_pane() {
    let (mut store, id) = launched();
    store.toggle_split(id, None);
    install(&mut store, id, StatusPane::Right);
    assert!(store.reserve_remote_overlay(id, slot("j", Some(OverlayPane::Right), false)));
    assert_eq!(
        tree_changes(&mut store, |s| {
            s.close_primary_pane(id);
        }),
        1
    );
    assert!(
        store
            .session(id)
            .unwrap()
            .remote_overlays()
            .slot(Some(OverlayPane::Left))
            .is_some()
    );
}

#[test]
fn swapping_panes_invalidates_the_tree_for_a_reservation_that_changes_pane() {
    let (mut store, id) = launched();
    install(&mut store, id, StatusPane::Left);
    store.toggle_split(id, None);
    install(&mut store, id, StatusPane::Right);
    assert!(store.reserve_remote_overlay(id, slot("j", Some(OverlayPane::Right), false)));
    assert_eq!(tree_changes(&mut store, |s| s.swap_panes(id).unwrap()), 1);
    assert!(
        store
            .session(id)
            .unwrap()
            .remote_overlays()
            .slot(Some(OverlayPane::Left))
            .is_some()
    );
}
