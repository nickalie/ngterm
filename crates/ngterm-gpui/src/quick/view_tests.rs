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
