//! Mouse shapes, passive links and selection autoscroll through real owner views.

use gpui_kit::{Modifiers, TestAppContext, point, px};

use super::super::surface::tests::{ECHO, Fixture};
use super::*;

#[test]
fn program_mouse_shapes_keep_their_platform_cursor_meaning() {
    for (shape, expected) in [
        (MouseShape::Text, CursorStyle::IBeam),
        (MouseShape::VerticalText, CursorStyle::IBeamCursorForVerticalLayout),
        (MouseShape::Cell, CursorStyle::Crosshair),
        (MouseShape::Crosshair, CursorStyle::Crosshair),
        (MouseShape::ContextMenu, CursorStyle::ContextualMenu),
        (MouseShape::Alias, CursorStyle::DragLink),
        (MouseShape::Copy, CursorStyle::DragCopy),
        (MouseShape::NoDrop, CursorStyle::OperationNotAllowed),
        (MouseShape::NotAllowed, CursorStyle::OperationNotAllowed),
        (MouseShape::Grab, CursorStyle::OpenHand),
        (MouseShape::Grabbing, CursorStyle::ClosedHand),
        (MouseShape::Move, CursorStyle::ClosedHand),
        (MouseShape::AllScroll, CursorStyle::ClosedHand),
        (MouseShape::ColResize, CursorStyle::ResizeColumn),
        (MouseShape::RowResize, CursorStyle::ResizeRow),
        (MouseShape::NsResize, CursorStyle::ResizeUpDown),
        (MouseShape::NResize, CursorStyle::ResizeUp),
        (MouseShape::SResize, CursorStyle::ResizeDown),
        (MouseShape::EResize, CursorStyle::ResizeRight),
        (MouseShape::WResize, CursorStyle::ResizeLeft),
        (MouseShape::NeResize, CursorStyle::ResizeUpRightDownLeft),
        (MouseShape::SwResize, CursorStyle::ResizeUpRightDownLeft),
        (MouseShape::NeswResize, CursorStyle::ResizeUpRightDownLeft),
        (MouseShape::NwResize, CursorStyle::ResizeUpLeftDownRight),
        (MouseShape::SeResize, CursorStyle::ResizeUpLeftDownRight),
        (MouseShape::NwseResize, CursorStyle::ResizeUpLeftDownRight),
        (MouseShape::Help, CursorStyle::Arrow),
        (MouseShape::Progress, CursorStyle::Arrow),
        (MouseShape::Wait, CursorStyle::Arrow),
        (MouseShape::ZoomIn, CursorStyle::Arrow),
        (MouseShape::ZoomOut, CursorStyle::Arrow),
    ] {
        assert_eq!(cursor_style(shape), expected, "{shape:?}");
    }
    assert_eq!(
        scroll_multiplier(Some("precision:bad,unknown:8,discrete:20000")),
        (1.0, 10_000.0)
    );
    assert_eq!(scroll_multiplier(Some(" precision : 2.5 , discrete : -5")), (2.5, 0.01));
}

#[gpui_kit::test]
#[cfg_attr(windows, ignore = "drives its pane with a POSIX sh script")]
fn passive_link_hover_and_click_require_a_modifier_and_in_bounds_position(cx: &mut TestAppContext) {
    let f = Fixture::new(cx, "printf 'https://example.com ready'; exec cat", "");
    let view = f.view.clone();
    let bounds = Fixture::bounds();
    let ctrl = Modifiers {
        control: true,
        ..Modifiers::default()
    };
    f.cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            assert!(!view.hover_link_under(Some(Fixture::at(2, 0)), bounds, Modifiers::default(), cx));
            assert!(view.hover_link_under(Some(Fixture::at(2, 0)), bounds, ctrl, cx));
            assert!(view.hover_link_under(Some(Fixture::at(2, 0)), bounds, ctrl, cx));
            assert_eq!(
                view.hovered_link().map(|link| link.url.as_str()),
                Some("https://example.com")
            );
            view.open_link_under(Fixture::at(2, 0), bounds, ctrl, cx);
            view.open_link_under(point(px(-10.), px(-10.)), bounds, ctrl, cx);
            assert!(!view.hover_link_under(Some(point(px(-10.), px(-10.))), bounds, ctrl, cx));
            assert!(!view.hover_link_under(None, bounds, ctrl, cx));
        });
    });
    f.cx.run_until_parked();
    let opened: Vec<_> = f
        .events
        .borrow()
        .iter()
        .filter_map(|event| match event {
            TerminalEvent::OpenUrl { url, browser, .. } => Some((url.clone(), *browser)),
            _ => None,
        })
        .collect();
    assert_eq!(opened, [("https://example.com".to_owned(), false)]);
    assert!(view.read_with(f.cx, |view, _| view.hovered_link().is_none()));
}

#[gpui_kit::test]
#[cfg_attr(windows, ignore = "drives its pane with a POSIX sh script")]
fn context_menu_selects_a_link_and_preserves_an_existing_selection(cx: &mut TestAppContext) {
    let f = Fixture::new(
        cx,
        "printf 'https://example.com ready'; exec cat",
        "right-click-action = context-menu\n",
    );
    f.cx.simulate_mouse_down(
        Fixture::at(2, 0),
        MouseButton::Right,
        Modifiers {
            control: true,
            ..Modifiers::default()
        },
    );
    f.cx.simulate_mouse_up(
        Fixture::at(2, 0),
        MouseButton::Right,
        Modifiers {
            control: true,
            ..Modifiers::default()
        },
    );
    assert_eq!(f.selection().as_deref(), Some("https://example.com"));
    f.cx.run_until_parked();
    // without Ctrl the link is not live, so a re-selection would take the word `//example.com` instead
    f.cx.simulate_mouse_down(Fixture::at(10, 0), MouseButton::Right, Modifiers::default());
    f.cx.simulate_mouse_up(Fixture::at(10, 0), MouseButton::Right, Modifiers::default());
    assert_eq!(f.selection().as_deref(), Some("https://example.com"));
}

#[gpui_kit::test]
#[cfg_attr(windows, ignore = "drives its pane with a POSIX sh script")]
fn dragging_above_the_view_autoscrolls_and_stops_when_back_inside(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx, "seq 1 80; printf 'ready'; exec cat", "");
    let original = f.text();
    let at = Fixture::at(1, 0);
    f.cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::default());
    f.cx.simulate_mouse_move(point(at.x, px(-50.)), Some(MouseButton::Left), Modifiers::default());
    f.cx.executor().advance_clock(AUTOSCROLL_INTERVAL);
    f.settle(|f| f.text().lines().next() != original.lines().next());
    assert!(f.selection().is_some());
    f.cx.simulate_mouse_move(Fixture::at(2, 1), Some(MouseButton::Left), Modifiers::default());
    f.cx.run_until_parked();
    // back inside with the button still held: no more scrolling
    let inside = f.text();
    f.cx.executor().advance_clock(AUTOSCROLL_INTERVAL * 3);
    f.cx.run_until_parked();
    assert_eq!(f.text(), inside);
    f.cx.simulate_mouse_up(Fixture::at(2, 1), MouseButton::Left, Modifiers::default());
    f.cx.run_until_parked();
    let stopped = f.text();
    f.cx.executor().advance_clock(AUTOSCROLL_INTERVAL * 3);
    f.cx.run_until_parked();
    assert_eq!(f.text(), stopped);
    let view = f.view.clone();
    assert!(!f.cx.update(|_, cx| view.update(cx, TerminalView::autoscroll_tick)));
}

#[gpui_kit::test]
#[cfg_attr(windows, ignore = "drives its pane with a POSIX sh script")]
fn readonly_wheel_scrolls_history_without_sending_mouse_reports(cx: &mut TestAppContext) {
    let mut f = Fixture::new(
        cx,
        "seq 1 80; stty raw -echo; printf '\\033[?1000h\\033[?1006hready'; exec cat -v",
        "",
    );
    assert!(f.perform("toggle_readonly"));
    let view = f.view.clone();
    let wheel = ScrollWheelEvent {
        position: Fixture::at(1, 1),
        delta: ScrollDelta::Lines(point(0., 3.)),
        ..ScrollWheelEvent::default()
    };
    let original = f.text();
    f.cx.update(|_, cx| view.update(cx, |view, cx| view.scroll_wheel(&wheel, Fixture::bounds(), cx)));
    f.settle(|f| f.text().lines().next() != original.lines().next());
    assert!(f.perform("toggle_readonly"));
    assert!(f.perform("scroll_to_bottom"));
    f.pane().write(b"barrier").unwrap();
    f.wait("barrier");
    assert!(!f.text().contains("^[[<"));
}

#[gpui_kit::test]
#[cfg_attr(windows, ignore = "drives its pane with a POSIX sh script")]
fn middle_click_in_readonly_mode_preserves_the_program_input(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx, ECHO, "middle-click-action = clipboard-paste\n");
    f.cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("blocked".into()));
    assert!(f.perform("toggle_readonly"));
    f.cx.simulate_mouse_down(Fixture::at(1, 0), MouseButton::Middle, Modifiers::default());
    f.cx.simulate_mouse_up(Fixture::at(1, 0), MouseButton::Middle, Modifiers::default());
    assert!(f.perform("toggle_readonly"));
    f.pane().write(b"barrier").unwrap();
    f.wait("barrier");
    assert!(!f.text().contains("blocked"));
}
