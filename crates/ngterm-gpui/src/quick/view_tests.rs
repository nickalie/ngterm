use super::*;
use crate::terminal::{TerminalConfig, TerminalSpec};
use gpui_kit::{TestAppContext, size};

#[gpui_kit::test]
fn a_detached_panel_closes_without_waiting_for_a_model(cx: &mut TestAppContext) {
    let terminal =
        cx.new(|cx| Terminal::gated(TerminalSpec::default(), TerminalConfig::default(), Box::new(|| {}), cx));
    let links = Rc::new(Links::default());
    let (view, cx) = cx.add_window_view(|window, cx| {
        QuickView::new(
            &terminal,
            Rgb::BLACK,
            Rc::clone(&links),
            crate::state::detached_poster(),
            window,
            cx,
        )
    });
    cx.simulate_resize(size(px(400.), px(200.)));
    cx.run_until_parked();
    assert!(links.landed.get().is_some());
    // the window draws the long-lived terminal through a view of its own, which owns its keyboard and size
    let (drawn, role) = view.read_with(cx, |view, cx| {
        let terminal_view = view.terminal().read(cx);
        (terminal_view.terminal().entity_id(), terminal_view.role())
    });
    assert_eq!(drawn, terminal.entity_id());
    assert_eq!(role, ViewRole::Owner);
    assert!(cx.simulate_close(), "no model remains to defer the close to");
}

#[gpui_kit::test]
fn bounds_changes_reach_the_panel_and_owned_close_clears_its_focus(cx: &mut TestAppContext) {
    let terminal =
        cx.new(|cx| Terminal::gated(TerminalSpec::default(), TerminalConfig::default(), Box::new(|| {}), cx));
    let links = Rc::new(Links::default());
    let (view, cx) = cx.add_window_view(|window, cx| {
        QuickView::new(
            &terminal,
            Rgb::BLACK,
            Rc::clone(&links),
            crate::state::detached_poster(),
            window,
            cx,
        )
    });
    cx.simulate_resize(size(px(640.), px(300.)));
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |view, _| view.size), Some(size(px(640.), px(300.))));
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    assert!(view.read_with(cx, |view, _| view.holds_key));
    assert!(cx.update(|_, cx| holds_keyboard(cx)), "the panel holds the keyboard");
    // the panel closes the window: losing the keyboard then is no focus loss, and what holds it is the panel's to
    // settle
    links.closing.set(true);
    cx.deactivate_window();
    cx.run_until_parked();
    assert!(!view.read_with(cx, |view, _| view.holds_key), "its focus is cleared");
    assert!(
        cx.update(|_, cx| holds_keyboard(cx)),
        "the closing window leaves the record alone"
    );
}

fn zoom_view(cx: &mut TestAppContext) -> (Entity<QuickView>, &mut gpui_kit::VisualTestContext) {
    let terminal =
        cx.new(|cx| Terminal::gated(TerminalSpec::default(), TerminalConfig::default(), Box::new(|| {}), cx));
    cx.add_window_view(|window, cx| {
        QuickView::new(
            &terminal,
            Rgb::BLACK,
            Rc::new(Links::default()),
            crate::state::detached_poster(),
            window,
            cx,
        )
    })
}

// The test window never maximizes, runs no frame callbacks, and panics when asked to maximize: these settle the zoom
// as its second frame callback does, and a request would panic. The frames themselves are the weston process test's.

#[gpui_kit::test]
fn an_unzoom_of_a_window_already_restored_settles_without_a_request(cx: &mut TestAppContext) {
    let (view, cx) = zoom_view(cx);
    view.update_in(cx, |view, window, cx| {
        view.zoom(false, window, cx);
        assert!(view.zoom.is_some(), "nothing settles before the frames");
        view.settle_zoom(false, window, cx);
        assert_eq!(view.zoom, None);
    });
}

#[gpui_kit::test]
fn a_zoom_whose_maximize_changed_since_the_frame_waits_for_the_next_one(cx: &mut TestAppContext) {
    let (view, cx) = zoom_view(cx);
    view.update_in(cx, |view, window, cx| {
        view.zoom(true, window, cx);
        // the first frame read the window maximized, which reads restored now: a configure is acked, not committed
        view.settle_zoom(true, window, cx);
        assert_eq!(view.zoom, Some(ZoomRequest::new(true)), "no request went out");
    });
}
