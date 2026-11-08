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
    assert_ne!(
        view.read_with(cx, |view, _| view.terminal().entity_id()),
        terminal.entity_id()
    );
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
    links.closing.set(true);
    view.update_in(cx, QuickView::activation_changed);
    assert!(!cx.update(|_, cx| holds_keyboard(cx)));
}
