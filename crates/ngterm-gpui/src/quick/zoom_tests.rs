use super::*;

#[test]
fn a_zoom_requests_once_and_is_done_when_the_window_maximizes() {
    let mut zoom = ZoomRequest::new(true);
    assert_eq!(zoom.settle(false, false), Settle::Request);
    assert_eq!(
        zoom.settle(false, false),
        Settle::Wait,
        "no second request before the first one's answer"
    );
    assert_eq!(zoom.settle(true, true), Settle::Done);
}

#[test]
fn a_window_already_as_wanted_needs_no_request() {
    assert_eq!(ZoomRequest::new(false).settle(false, false), Settle::Done);
    assert_eq!(ZoomRequest::new(true).settle(true, true), Settle::Done);
}

#[test]
fn an_unzoom_during_the_zooms_request_restores_the_window_once_it_maximized() {
    let mut zoom = ZoomRequest::new(true);
    assert_eq!(zoom.settle(false, false), Settle::Request);
    zoom.want(false);
    // the window still reads unmaximized, as wanted now, but the maximize is on its way
    assert_eq!(zoom.settle(false, false), Settle::Wait);
    assert_eq!(
        zoom.settle(true, true),
        Settle::Request,
        "the window maximized: restore it"
    );
    assert_eq!(zoom.settle(true, true), Settle::Wait);
    assert_eq!(zoom.settle(false, false), Settle::Done);
}

#[test]
fn an_unzoom_waits_for_the_frame_that_commits_the_maximize() {
    let mut zoom = ZoomRequest::new(true);
    assert_eq!(zoom.settle(false, false), Settle::Request);
    zoom.want(false);
    // the maximize's configure was acked after the last frame: a restore now would come before its commit
    assert_eq!(zoom.settle(false, true), Settle::Again);
    assert_eq!(zoom.settle(true, true), Settle::Request, "the next frame committed it");
}

#[test]
fn a_maximize_not_committed_yet_needs_nothing_when_it_is_wanted() {
    let mut zoom = ZoomRequest::new(false);
    zoom.want(true);
    assert_eq!(zoom.settle(false, true), Settle::Done);
}

#[test]
fn a_rezoom_before_the_unzoom_is_sent_needs_nothing() {
    let mut zoom = ZoomRequest::new(false);
    zoom.want(true);
    assert_eq!(zoom.settle(true, true), Settle::Done);
}

const PANEL: Rect = [410., 190., 1100., 700.];
const VISIBLE: Rect = [0., 0., 1920., 1055.];

#[test]
#[expect(clippy::float_cmp, reason = "a frame is handed back as it was kept, never computed")]
fn a_zoom_fills_the_visible_frame_and_its_unzoom_gives_the_panels_frame_back() {
    let mut restore = Restore::default();
    assert!(!restore.filled());
    assert_eq!(restore.target(PANEL, VISIBLE), VISIBLE);
    assert!(restore.filled());
    assert_eq!(restore.target(VISIBLE, VISIBLE), PANEL);
    assert_eq!(restore, Restore::default(), "the frame is given back once");
}

#[test]
#[expect(clippy::float_cmp, reason = "a frame is handed back as it was kept, never computed")]
fn an_unzoom_gives_the_panels_frame_back_after_the_screens_visible_frame_changed() {
    let without_dock: Rect = [0., 0., 1920., 1175.];
    let mut restore = Restore::default();
    assert_eq!(restore.target(PANEL, VISIBLE), VISIBLE);
    // the window no longer has its screen's visible size, and the kept frame still reads as the zoom
    assert_eq!(
        ZoomRequest::new(false).settle(restore.filled(), restore.filled()),
        Settle::Request
    );
    assert_eq!(restore.target(VISIBLE, without_dock), PANEL);
    assert_eq!(
        ZoomRequest::new(false).settle(restore.filled(), restore.filled()),
        Settle::Done
    );
    assert_eq!(
        restore.target(PANEL, without_dock),
        without_dock,
        "the next zoom keeps the panel's own frame"
    );
    assert_eq!(restore.target(without_dock, without_dock), PANEL);
}

#[test]
#[expect(clippy::float_cmp, reason = "a frame is handed back as it was kept, never computed")]
fn a_zoom_after_an_unzoom_keeps_the_frame_the_window_has_then() {
    let moved: Rect = [100., 80., 900., 600.];
    let mut restore = Restore::default();
    assert_eq!(restore.target(PANEL, VISIBLE), VISIBLE);
    assert_eq!(restore.target(VISIBLE, VISIBLE), PANEL);
    assert_eq!(restore.target(moved, VISIBLE), VISIBLE);
    assert_eq!(restore.target(VISIBLE, VISIBLE), moved);
}
