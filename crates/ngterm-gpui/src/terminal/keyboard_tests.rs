//! Input-method refusals and focus handoff at the terminal owner boundary.

use gpui_kit::{AppContext as _, Keystroke, TestAppContext};

use super::super::input::{KeyStroke, Source};
use super::super::surface::tests::{ECHO, Fixture, config};
use super::super::{Terminal, TerminalConfig, TerminalSpec, ViewRole};
use super::*;

#[gpui_kit::test]
fn an_unstarted_view_refuses_keys_and_has_no_candidate_rectangle(cx: &mut TestAppContext) {
    let terminal =
        cx.new(|cx| Terminal::gated(TerminalSpec::default(), TerminalConfig::default(), Box::new(|| {}), cx));
    let (view, cx) = cx.add_window_view(|_, cx| TerminalView::new(&terminal, ViewRole::Viewer, cx));
    let stroke = KeyStroke::new(
        &Keystroke::parse("a").unwrap().with_simulated_ime(),
        &Source::Keystroke,
        false,
    );
    cx.update(|window, cx| {
        assert!(!key_down(&stroke, window, cx));
        assert!(typing_surface(window, cx).is_none());
        view.update(cx, |view, cx| {
            assert!(!view.key_down(&stroke, window, cx));
            view.key_up(
                &KeyUpEvent {
                    keystroke: Keystroke::parse("a").unwrap(),
                },
                window,
                cx,
            );
            assert_eq!(view.text_for_range(0..1, &mut None, window, cx), None);
            assert_eq!(view.character_index_for_point(point(px(1.), px(1.)), window, cx), None);
            assert_eq!(view.bounds_for_range(0..0, Fixture::bounds(), window, cx), None);
            view.replace_text_in_range(None, "lost", window, cx);
            assert_eq!(view.release_focus_reports(), None);
        });
    });
    assert_eq!(view.read_with(cx, |view, _| view.shared.preedit()), None);
}

#[gpui_kit::test]
#[cfg_attr(windows, ignore = "drives its pane with a POSIX sh script")]
fn empty_and_readonly_ime_commits_end_preedit_without_typing(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx, ECHO, "");
    let view = f.view.clone();
    f.shared().set_preedit(Some("discard".into()));
    f.cx.update(|window, cx| view.update(cx, |view, cx| view.replace_text_in_range(None, "", window, cx)));
    assert_eq!(f.shared().preedit(), None);
    assert!(f.perform("toggle_readonly"));
    f.shared().set_preedit(Some("discard".into()));
    f.cx.update(|window, cx| view.update(cx, |view, cx| view.replace_text_in_range(None, "blocked", window, cx)));
    assert_eq!(f.shared().preedit(), None);
    assert!(f.perform("toggle_readonly"));
    f.cx.update(|window, cx| view.update(cx, |view, cx| view.replace_text_in_range(None, "barrier", window, cx)));
    f.wait("barrier");
    assert!(!f.text().contains("blocked") && !f.text().contains("discard"));
}

#[gpui_kit::test]
#[cfg_attr(windows, ignore = "drives its pane with a POSIX sh script")]
fn candidate_rectangle_tracks_the_cursor_with_nonzero_element_origin(cx: &mut TestAppContext) {
    let f = Fixture::new(cx, "printf 'ready'; exec cat", "");
    let view = f.view.clone();
    let bounds = Bounds::new(point(px(100.), px(200.)), size(px(640.), px(480.)));
    let rectangle =
        f.cx.update(|window, cx| view.update(cx, |view, cx| view.bounds_for_range(0..0, bounds, window, cx)));
    assert_eq!(
        rectangle,
        Some(Bounds::new(point(px(152.), px(202.)), size(px(10.), px(21.))))
    );
}

#[gpui_kit::test]
#[cfg_attr(windows, ignore = "drives its pane with a POSIX sh script")]
fn cursor_hiding_changes_with_mouse_configuration(cx: &mut TestAppContext) {
    let f = Fixture::new(cx, ECHO, "mouse-hide-while-typing = true\n");
    assert_eq!(f.cx.update(|_, cx| cx.cursor_hide_mode()), CursorHideMode::OnTyping);
    f.terminal.update(f.cx, |terminal, cx| {
        terminal.set_config(config("mouse-hide-while-typing = false\n"), cx);
    });
    f.cx.run_until_parked();
    assert_eq!(f.cx.update(|_, cx| cx.cursor_hide_mode()), CursorHideMode::Never);
}
