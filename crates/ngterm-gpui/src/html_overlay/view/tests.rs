//! The element in GPUI's test window against a recording engine (`testing::Recorder`, the registry's stand-in for
//! `ngterm_wpe::Host`): the page made and loaded at its mount, its geometry and visibility, the pointer and the keys
//! in the content's coordinates, a paste key handing the clipboard over first, a click a GPUI popover still sees, the
//! frames' answers and the fall-back, the `<select>` popup and the context menu.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, ClipboardItem, Context, Entity, InteractiveElement as _, IntoElement, Modifiers, MouseButton,
    ParentElement as _, Render, Styled as _, TestAppContext, VisualTestContext, Window, div, point, px, size,
};
use ngterm_core::html::{HtmlOverlay, HtmlPageFlags, HtmlSource};
use ngterm_wpe::{
    Area, BufferId, ContextMenu, DmaBuf, EditingCommand, Event, Frame, FrameContent, Geometry, Input, Load, MenuToken,
    OptionItem, OptionKind, OptionMenu, Plane, PointerKind, Rect, SessionSpec, ShmRegion, ViewId,
};

use super::PageView;
use crate::html_overlay::registry;
use crate::html_overlay::testing::{self, Call, Recorder};

/// The window's content: a stand-in popover (a strip on top that closes on any press outside it) over the page.
struct Root {
    element: Option<Entity<PageView>>,
    popover_open: Rc<Cell<bool>>,
}

impl Render for Root {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let open = Rc::clone(&self.popover_open);
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("popover")
                    .h(px(40.))
                    .flex_shrink_0()
                    .on_mouse_down_out(move |_, _, _| open.set(false)),
            )
            .child(div().flex_1().min_h(px(0.)).children(self.element.clone()))
    }
}

const TOP: f32 = 40.;
/// The strip and its separator above the content.
const STRIP: f32 = 29.;
const START: &str = "https://example.invalid/start";

struct Harness {
    web: Rc<Recorder>,
    root: Entity<Root>,
    element: Entity<PageView>,
    overlay: HtmlOverlay,
}

fn page_overlay() -> HtmlOverlay {
    HtmlOverlay::new(
        HtmlSource::Url(url::Url::parse(START).unwrap()),
        HtmlPageFlags::default(),
    )
}

/// A page mounted and shown in an 800x500 test window.
fn harness(cx: &mut TestAppContext) -> (Harness, &mut VisualTestContext) {
    harness_of(page_overlay(), cx)
}

/// `overlay` mounted and shown in an 800x500 test window.
fn harness_of(overlay: HtmlOverlay, cx: &mut TestAppContext) -> (Harness, &mut VisualTestContext) {
    cx.update(gpui_kit::init);
    let web = testing::install();
    let popover_open = Rc::new(Cell::new(true));
    let open = Rc::clone(&popover_open);
    let (root, cx) = cx.add_window_view(move |_, _| Root {
        element: None,
        popover_open: open,
    });
    cx.simulate_resize(size(px(800.), px(500.)));
    // focus changes reach their listeners in an active window
    cx.update(|window, _| window.activate_window());
    let element = cx
        .update(|window, cx| registry::mount(&overlay, None, window, cx))
        .unwrap();
    root.update(cx, |root, cx| {
        root.element = Some(element.clone());
        cx.notify();
    });
    cx.update(|_, cx| registry::apply(&overlay, true, cx));
    cx.run_until_parked();
    let harness = Harness {
        web,
        root,
        element,
        overlay,
    };
    (harness, cx)
}

impl Harness {
    /// Gives the page the keyboard, as a press or a key into it does.
    fn focus_page(&self, cx: &mut VisualTestContext) {
        let focus = self.element.read_with(cx, |element, _| element.focus().clone());
        cx.update(|window, cx| window.focus(&focus, cx));
        self.root.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
    }

    fn popover_open(&self, cx: &mut VisualTestContext) -> bool {
        self.root.read_with(cx, |root, _| root.popover_open.get())
    }

    fn view(&self, cx: &mut VisualTestContext) -> ViewId {
        self.element
            .read_with(cx, |element, _| element.view_id())
            .expect("the page was realized")
    }

    fn dispatch(event: &Event, cx: &mut VisualTestContext) {
        cx.update(|_, cx| registry::dispatch(event, cx));
        cx.run_until_parked();
    }
}

fn pointers(calls: &[Call]) -> Vec<(PointerKind, f64, f64)> {
    calls
        .iter()
        .filter_map(|call| match call {
            Call::Input(Input::Pointer(pointer)) => Some((pointer.kind, pointer.x, pointer.y)),
            _ => None,
        })
        .collect()
}

#[gpui_kit::test]
fn a_mounted_page_is_made_loaded_shown_and_laid_out_at_its_content(cx: &mut TestAppContext) {
    let (harness, cx) = harness(cx);
    let calls = harness.web.take();
    assert_eq!(calls[0], Call::CreateSession(SessionSpec::Ephemeral), "{calls:?}");
    assert!(
        calls.contains(&Call::CreateView {
            handlers: 0,
            transparent: false,
        }),
        "a URL page: no bridge, the browser's opaque canvas: {calls:?}"
    );
    assert!(calls.contains(&Call::Load(Load::Uri(START.to_owned()))), "{calls:?}");
    assert_eq!(
        calls.iter().filter(|call| **call == Call::Visible(true)).count(),
        1,
        "shown once the model puts it on screen: {calls:?}"
    );
    let scale = cx.update(|window, _| window.scale_factor());
    let geometry = calls.iter().rev().find_map(|call| match call {
        Call::Geometry(geometry) => Some(*geometry),
        _ => None,
    });
    assert_eq!(
        geometry,
        Some(Geometry {
            width: 800,
            height: 431, // 500 less the popover strip (40) and the page strip with its separator (29)
            scale: f64::from(scale),
        }),
        "the content's size, below the strip, at the window's scale"
    );
    // off screen: the engine renders nothing, and gets no size while the page is out of sight
    cx.update(|_, cx| registry::apply(&harness.overlay, false, cx));
    assert!(harness.web.take().contains(&Call::Visible(false)));
    cx.simulate_scale_factor_change(1.5);
    harness.root.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    let hidden = harness.web.take();
    assert!(
        !hidden.iter().any(|call| matches!(call, Call::Geometry(_))),
        "{hidden:?}"
    );
    // back on screen, the new scale reaches the engine
    cx.update(|_, cx| registry::apply(&harness.overlay, true, cx));
    harness.root.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    let rescaled = harness.web.take();
    assert!(
        rescaled
            .iter()
            .any(|call| matches!(call, Call::Geometry(geometry) if (geometry.scale - 1.5).abs() < f64::EPSILON)),
        "{rescaled:?}"
    );
}

/// Upstream's accessibility identifiers (`conformance/ui-ids.yaml`, `htmlOverlay.*`) name the page's elements: the
/// navigation strip of a URL page, then a file page's Show in Files, then the failure over a page that did not load.
#[gpui_kit::test]
fn the_page_and_its_strip_carry_upstreams_accessibility_ids(cx: &mut TestAppContext) {
    let mut overlay = page_overlay();
    overlay.flags.navigation = true;
    let (harness, cx) = harness_of(overlay, cx);
    for id in [
        "htmlOverlay.back",
        "htmlOverlay.forward",
        "htmlOverlay.reload",
        "htmlOverlay.identity",
        "htmlOverlay.browser",
        "htmlOverlay.copy",
        "htmlOverlay.close",
        "htmlOverlay.page",
    ] {
        assert!(cx.debug_bounds(id).is_some(), "{id} is drawn");
    }
    assert!(
        cx.debug_bounds("htmlOverlay.finder").is_none(),
        "a URL page copies its link"
    );
    assert!(cx.debug_bounds("htmlOverlay.error").is_none(), "the page did not fail");
    let mut failed = harness.overlay.clone();
    failed.source = HtmlSource::File {
        path: "/nonexistent/page.html".to_owned(),
        grant_root: None,
    };
    failed.load_state = ngterm_core::html::HtmlLoadState::Failed;
    failed.load_error = Some("not found".to_owned());
    cx.update(|_, cx| registry::apply(&failed, true, cx));
    harness.root.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("htmlOverlay.finder").is_some(),
        "a file page shows its file"
    );
    assert!(cx.debug_bounds("htmlOverlay.copy").is_none());
    assert!(
        cx.debug_bounds("htmlOverlay.error").is_some(),
        "the failure is drawn over the page"
    );
    // upstream's `.textSelection(.enabled)`: the error is text a click focuses and Ctrl+A, Ctrl+C copy
    let shown = harness.element.read_with(cx, PageView::error_shown);
    assert_eq!(shown.as_deref(), Some("not found"));
    // a short error is as wide as its text and centered under the heading, not one character wide
    let card = cx.debug_bounds("htmlOverlay.error").expect("the failure is drawn");
    let text = cx
        .debug_bounds("htmlOverlay.error-text-box")
        .expect("the error is drawn");
    assert!(
        text.size.width > px(20.) && text.size.width < card.size.width / 2.,
        "{text:?} in {card:?}"
    );
    assert!(
        (text.center().x - card.center().x).abs() < px(2.),
        "centered: {text:?} in {card:?}"
    );
    let one_line = text.size.height;
    assert!(one_line < px(40.), "on one line: {text:?}");
    let at = cx
        .debug_bounds("html-failure-error")
        .expect("the error is drawn")
        .center();
    cx.simulate_click(at, Modifiers::default());
    cx.simulate_keystrokes("ctrl-a ctrl-c");
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()).as_deref(),
        Some("not found")
    );

    // a long one wraps at the card's width, inside its padding
    failed.load_error = Some("Error receiving data: Connection reset by peer. ".repeat(6));
    cx.update(|_, cx| registry::apply(&failed, true, cx));
    harness.root.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    let text = cx
        .debug_bounds("htmlOverlay.error-text-box")
        .expect("the error is drawn");
    assert!(
        (text.size.width - (card.size.width - px(48.))).abs() < px(1.),
        "capped at the card: {text:?} in {card:?}"
    );
    assert!(text.size.height > one_line, "wrapped onto more lines: {text:?}");
}

#[gpui_kit::test]
fn a_press_focuses_the_page_goes_in_at_its_content_and_still_closes_a_popover(cx: &mut TestAppContext) {
    let (harness, cx) = harness(cx);
    harness.web.take();
    assert!(harness.popover_open(cx));
    let at = point(px(100.), px(TOP + STRIP + 10.));
    cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::none());
    // the focus listeners run with the next frame
    harness.root.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    let calls = harness.web.take();
    assert!(
        calls.contains(&Call::Focused(true)),
        "a press gives the page the keyboard: {calls:?}"
    );
    let pressed = pointers(&calls);
    assert!(
        pressed.contains(&(PointerKind::Down(ngterm_wpe::Button::Primary), 100., 10.)),
        "in the content's coordinates: {pressed:?}"
    );
    assert!(pressed.contains(&(PointerKind::Up(ngterm_wpe::Button::Primary), 100., 10.)));
    assert!(
        !harness.popover_open(cx),
        "GPUI's own handlers saw the press: a popover elsewhere closed"
    );
    // a press on the strip above the content is not the page's
    cx.simulate_mouse_down(point(px(400.), px(TOP + 10.)), MouseButton::Left, Modifiers::none());
    assert!(pointers(&harness.web.take()).is_empty());
}

#[gpui_kit::test]
fn keys_go_in_while_the_page_holds_the_keyboard_and_a_paste_hands_the_clipboard_over_first(cx: &mut TestAppContext) {
    let (harness, cx) = harness(cx);
    let key = |text: &str, cx: &mut VisualTestContext| {
        let keystroke = gpui_kit::Keystroke::parse(text).unwrap();
        let source = crate::terminal::input::Source::Keystroke;
        let stroke = crate::terminal::input::KeyStroke::new(&keystroke, &source, false);
        cx.update(|window, cx| registry::key_down(&keystroke, &stroke, window, cx))
    };
    assert!(!key("a", cx), "not focused: the page takes no key");
    let focus = harness.element.read_with(cx, |element, _| element.focus().clone());
    cx.update(|window, cx| window.focus(&focus, cx));
    harness.web.take();
    assert!(key("a", cx));
    let calls = harness.web.take();
    assert!(
        matches!(&calls[..], [Call::Input(Input::Key(key))] if key.pressed && key.keyval == u32::from('a')),
        "{calls:?}"
    );
    cx.write_to_clipboard(ClipboardItem::new_string("from the desktop".to_owned()));
    assert!(key("ctrl-v", cx));
    let calls = harness.web.take();
    assert_eq!(
        calls.first(),
        Some(&Call::Clipboard("from the desktop".to_owned())),
        "{calls:?}"
    );
    assert!(matches!(calls.get(1), Some(Call::Input(Input::Key(key))) if key.keyval == u32::from('v')));
}

fn shm_frame(view: ViewId, buffer: u64) -> Event {
    Event::Frame(Frame {
        view,
        buffer: BufferId::from_raw(buffer),
        width: 2,
        height: 2,
        damage: Vec::new(),
        content: FrameContent::Shm(vec![ShmRegion {
            rect: Rect {
                x: 0,
                y: 0,
                width: 2,
                height: 2,
            },
            pixels: vec![255; 16],
        }]),
    })
}

#[gpui_kit::test]
fn every_frame_is_answered_and_a_buffer_that_does_not_import_falls_back_to_shm(cx: &mut TestAppContext) {
    let (harness, cx) = harness(cx);
    let view = harness.view(cx);
    Harness::dispatch(&shm_frame(view, 1), cx);
    Harness::dispatch(&shm_frame(view, 2), cx);
    assert!(
        harness.web.presented().contains(&BufferId::from_raw(1)),
        "a replaced frame is answered: {:?}",
        harness.web.presented()
    );
    harness.web.take();
    // GPUI's test renderer opened no device: no dmabuf imports, so the page goes to SHM, and the frame is answered
    let file = tempfile::tempfile().unwrap();
    let dmabuf = Event::Frame(Frame {
        view,
        buffer: BufferId::from_raw(3),
        width: 2,
        height: 2,
        damage: Vec::new(),
        content: FrameContent::DmaBuf(Arc::new(DmaBuf {
            width: 2,
            height: 2,
            fourcc: ngterm_wpe::fourcc(b"XR24"),
            modifier: ngterm_wpe::MODIFIER_LINEAR,
            planes: vec![Plane {
                fd: file.into(),
                offset: 0,
                stride: 8,
            }],
        })),
    });
    Harness::dispatch(&dmabuf, cx);
    assert!(harness.web.take().contains(&Call::FallBack));
    assert!(harness.web.presented().contains(&BufferId::from_raw(3)));
    // the fall-back is asked once
    Harness::dispatch(&dmabuf, cx);
    assert!(!harness.web.take().contains(&Call::FallBack));
}

#[gpui_kit::test]
fn a_select_popup_is_drawn_and_answered_by_click_or_closed_by_escape(cx: &mut TestAppContext) {
    let (harness, cx) = harness(cx);
    let view = harness.view(cx);
    let item = |label: &str| OptionItem {
        label: label.to_owned(),
        tooltip: None,
        kind: OptionKind::Option,
        enabled: true,
        selected: label == "one",
    };
    let menu = |token| {
        Event::OptionMenu(OptionMenu {
            token: MenuToken::from_raw(token),
            view,
            area: Area {
                x: 10,
                y: 10,
                width: 120,
                height: 24,
            },
            items: vec![item("one"), item("two"), item("three")],
        })
    };
    harness.focus_page(cx);
    Harness::dispatch(&menu(7), cx);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let row = cx.debug_bounds("html-popup-row-2").expect("the popup is drawn");
    harness.web.take();
    cx.simulate_click(row.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(harness.web.take(), [Call::Choose(MenuToken::from_raw(7), Some(2))]);

    Harness::dispatch(&menu(8), cx);
    let escape = gpui_kit::Keystroke::parse("escape").unwrap();
    let stroke = crate::terminal::input::KeyStroke::new(&escape, &crate::terminal::input::Source::Keystroke, false);
    assert!(cx.update(|window, cx| registry::key_down(&escape, &stroke, window, cx)));
    let calls = harness.web.take();
    assert!(calls.contains(&Call::Choose(MenuToken::from_raw(8), None)), "{calls:?}");
    assert!(
        !calls.iter().any(|call| matches!(call, Call::Input(_))),
        "the popup's key never reaches the page: {calls:?}"
    );
}

#[gpui_kit::test]
fn the_context_menu_copies_a_link_and_pastes_after_handing_the_clipboard_over(cx: &mut TestAppContext) {
    let (harness, cx) = harness(cx);
    let view = harness.view(cx);
    let hit = |link: Option<&str>, editable| {
        Event::ContextMenu(ContextMenu {
            view,
            x: 20,
            y: 20,
            link: link.map(str::to_owned),
            image: None,
            media: None,
            editable,
            selection: false,
        })
    };
    harness.focus_page(cx);
    Harness::dispatch(&hit(Some("https://link.invalid/x"), false), cx);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let row = cx.debug_bounds("html-popup-row-0").expect("the menu is drawn");
    cx.simulate_click(row.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()).as_deref(),
        Some("https://link.invalid/x")
    );

    cx.write_to_clipboard(ClipboardItem::new_string("pasted".to_owned()));
    Harness::dispatch(&hit(None, true), cx);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    harness.web.take();
    // Cut, Copy, Paste, Select All
    let paste = cx.debug_bounds("html-popup-row-2").expect("the field's menu");
    cx.simulate_click(paste.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        harness.web.take(),
        [
            Call::Clipboard("pasted".to_owned()),
            Call::Editing(EditingCommand::Paste)
        ]
    );
}

#[gpui_kit::test]
fn a_released_page_closes_its_engine_page(cx: &mut TestAppContext) {
    let (harness, cx) = harness(cx);
    let view = harness.view(cx);
    let page = harness.element.read_with(cx, |element, _| Rc::clone(element.page()));
    cx.update(|_, cx| page.close(cx));
    assert!(harness.web.take().contains(&Call::CloseView(view)));
    // the model reaching it changes nothing any more
    cx.update(|_, cx| registry::apply(&harness.overlay, false, cx));
    cx.update(|_, cx| registry::apply(&harness.overlay, true, cx));
    assert_eq!(harness.web.take(), [], "a released page sends nothing");
}

/// A focus handle of the window's own, standing in for a modal that takes the keyboard (Settings, an alert, a picker).
struct Modal {
    focus: gpui_kit::FocusHandle,
}

impl Render for Modal {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().track_focus(&self.focus)
    }
}

fn context_hit(view: ViewId) -> Event {
    Event::ContextMenu(ContextMenu {
        view,
        x: 20,
        y: 20,
        link: Some("https://link.invalid/x".to_owned()),
        image: None,
        media: None,
        editable: false,
        selection: false,
    })
}

#[gpui_kit::test]
fn a_page_popup_closes_when_a_modal_takes_the_keyboard_and_none_opens_without_it(cx: &mut TestAppContext) {
    let (harness, cx) = harness(cx);
    let view = harness.view(cx);
    let modal = cx.new(|cx| Modal {
        focus: cx.focus_handle(),
    });
    harness.focus_page(cx);
    Harness::dispatch(&context_hit(view), cx);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    assert!(cx.debug_bounds("html-popup").is_some(), "the menu is drawn");
    // a modal takes the keyboard: the page's popup goes with it, never drawn over the modal
    let focus = modal.read_with(cx, |modal, _| modal.focus.clone());
    cx.update(|window, cx| window.focus(&focus, cx));
    harness.root.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("html-popup").is_none(),
        "the menu closed with the focus"
    );
    // while the modal holds the keyboard, a popup the page asks for is refused, a select's answered unchanged
    harness.web.take();
    Harness::dispatch(
        &Event::OptionMenu(OptionMenu {
            token: MenuToken::from_raw(5),
            view,
            area: Area {
                x: 0,
                y: 0,
                width: 100,
                height: 20,
            },
            items: Vec::new(),
        }),
        cx,
    );
    Harness::dispatch(&context_hit(view), cx);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    assert!(cx.debug_bounds("html-popup").is_none(), "no popup over the modal");
    assert_eq!(harness.web.take(), [Call::Choose(MenuToken::from_raw(5), None)]);
}

#[gpui_kit::test]
fn a_renderer_side_import_failure_falls_back_without_another_frame(cx: &mut TestAppContext) {
    let (harness, cx) = harness(cx);
    let view = harness.view(cx);
    // a device that imports the frame's format, so the import is left to the renderer
    let caps = gpui_kit::DmaBufImportCaps {
        adapter: "test".to_owned(),
        render_node: None,
        formats: vec![gpui_kit::DmaBufFormat {
            fourcc: ngterm_wpe::fourcc(b"XR24"),
            modifiers: vec![gpui_kit::DmaBufModifier {
                modifier: ngterm_wpe::MODIFIER_LINEAR,
                plane_count: 1,
            }],
        }],
        unsupported: None,
    };
    harness
        .element
        .update(cx, |element, _| element.use_caps(Arc::new(caps)));
    let file = tempfile::tempfile().unwrap();
    let frame = Event::Frame(Frame {
        view,
        buffer: BufferId::from_raw(4),
        width: 2,
        height: 2,
        damage: Vec::new(),
        content: FrameContent::DmaBuf(Arc::new(DmaBuf {
            width: 2,
            height: 2,
            fourcc: ngterm_wpe::fourcc(b"XR24"),
            modifier: ngterm_wpe::MODIFIER_LINEAR,
            planes: vec![Plane {
                fd: file.into(),
                offset: 0,
                stride: 8,
            }],
        })),
    });
    Harness::dispatch(&frame, cx);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    assert!(!harness.web.take().contains(&Call::FallBack), "nothing failed yet");
    let surface = harness
        .element
        .read_with(cx, |element, _| element.shown_surface())
        .expect("the frame is shown");
    // the renderer fails it later (on a thread of its own in the app; GPUI's test scheduler takes wakes from its own
    // thread only); the page sends no further frame
    let gpui_kit::ExternalSurface::DmaBuf(shown) = surface else {
        panic!("not a dmabuf frame");
    };
    shown
        .completion
        .complete(gpui_kit::FrameOutcome::Failed("vkCreateImage failed".to_owned()));
    cx.run_until_parked();
    assert!(
        harness.web.take().contains(&Call::FallBack),
        "the page falls back to SHM without waiting for a frame"
    );
    assert!(
        harness.web.presented().contains(&BufferId::from_raw(4)),
        "the frame was answered"
    );
}

#[gpui_kit::test]
fn a_key_release_carries_the_physical_key_its_press_had(cx: &mut TestAppContext) {
    let (harness, cx) = harness(cx);
    harness.focus_page(cx);
    harness.web.take();
    // AZERTY's `a` sits on the US `q` key (XKB keycode 24)
    let keystroke = gpui_kit::Keystroke::parse("a").unwrap();
    let native = crate::terminal::input::Source::Native(crate::terminal::input::NativeKey {
        keycode: 24,
        ..Default::default()
    });
    let press = crate::terminal::input::KeyStroke::new(&keystroke, &native, false);
    let release = crate::terminal::input::KeyStroke::new(&keystroke, &native, true);
    harness.element.update(cx, |element, cx| {
        assert!(element.key_down(&keystroke, &press, &native, cx));
        assert!(element.key_released(&keystroke, &release));
    });
    let keys: Vec<(bool, Option<u32>)> = harness
        .web
        .take()
        .iter()
        .filter_map(|call| match call {
            Call::Input(Input::Key(key)) => Some((key.pressed, key.keycode)),
            _ => None,
        })
        .collect();
    assert_eq!(keys, [(true, Some(24)), (false, Some(24))]);
}

/// The window's content for the hit test: a terminal's stand-in that counts its presses, and the page over it, hidden
/// as the deck hides one (`Styled::invisible`) while its slot does not show.
struct Stack {
    element: Entity<PageView>,
    shown: bool,
    terminal_presses: Rc<Cell<u32>>,
}

impl Render for Stack {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let presses = Rc::clone(&self.terminal_presses);
        div()
            .size_full()
            .relative()
            .child(
                div()
                    .id("terminal")
                    .absolute()
                    .inset_0()
                    .on_mouse_down(MouseButton::Left, move |_, _, _| presses.set(presses.get() + 1)),
            )
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .when(!self.shown, gpui_kit::Styled::invisible)
                    .child(self.element.clone()),
            )
    }
}

#[gpui_kit::test]
fn the_terminal_under_a_hidden_page_gets_the_pointer(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let _web = testing::install();
    let presses = Rc::new(Cell::new(0));
    let overlay = page_overlay();
    let counted = Rc::clone(&presses);
    let mounted = overlay.clone();
    let (stack, cx) = cx.add_window_view(move |window, cx| Stack {
        element: registry::mount(&mounted, None, window, cx).unwrap(),
        shown: true,
        terminal_presses: counted,
    });
    cx.simulate_resize(size(px(800.), px(500.)));
    cx.update(|window, _| window.activate_window());
    cx.update(|_, cx| registry::apply(&overlay, true, cx));
    cx.run_until_parked();
    let at = point(px(300.), px(300.));
    cx.simulate_click(at, Modifiers::none());
    assert_eq!(presses.get(), 0, "a page on screen takes the press");
    // the slot hides the page: the terminal under it gets the pointer back
    cx.update(|_, cx| registry::apply(&overlay, false, cx));
    stack.update(cx, |stack, cx| {
        stack.shown = false;
        cx.notify();
    });
    cx.run_until_parked();
    cx.simulate_click(at, Modifiers::none());
    assert_eq!(presses.get(), 1, "a hidden page leaves the press to the terminal");
}

#[gpui_kit::test]
fn an_engine_that_ends_while_the_app_runs_fails_every_page_for_good(cx: &mut TestAppContext) {
    let (harness, cx) = harness(cx);
    let page = harness.element.read_with(cx, |element, _| Rc::clone(element.page()));
    assert_eq!(page.failure(), None);
    cx.update(|_, _| registry::engine_ended());
    let failure = page.failure().expect("the page failed");
    assert!(failure.contains("restart ngterm"), "{failure}");
    assert_eq!(
        registry::refusal().as_deref(),
        Some(failure.as_str()),
        "new pages are refused with it"
    );
}

#[gpui_kit::test]
fn a_start_off_the_main_thread_leaves_pages_waiting_and_its_failure_fails_them(cx: &mut TestAppContext) {
    let (harness, cx) = harness(cx);
    let finish = registry::set_starting();
    // a page mounted while the engine starts waits for it
    let overlay = page_overlay();
    let element = cx
        .update(|window, cx| registry::mount(&overlay, None, window, cx))
        .unwrap();
    harness.root.update(cx, |root, cx| {
        root.element = Some(element.clone());
        cx.notify();
    });
    cx.run_until_parked();
    let page = element.read_with(cx, |element, _| Rc::clone(element.page()));
    assert!(page.web().is_none(), "not realized while the engine starts");
    assert_eq!(page.failure(), None, "nor failed");
    assert_eq!(registry::refusal(), None, "pages are still accepted");
    finish(Err(ngterm_wpe::StartError::Worker("no bwrap".to_owned())));
    let failure = page.failure().expect("the waiting page failed");
    assert!(
        failure.contains("did not start") && failure.contains("no bwrap"),
        "{failure}"
    );
}

#[gpui_kit::test]
fn a_page_given_the_keyboard_while_the_engine_starts_holds_the_engine_s_once_realized(cx: &mut TestAppContext) {
    use gpui_kit::EntityInputHandler as _;
    let (harness, cx) = harness(cx);
    // a start that takes its time (a worker's): the page mounted meanwhile waits for it
    let _gated = registry::set_starting();
    let overlay = page_overlay();
    let element = cx
        .update(|window, cx| registry::mount(&overlay, None, window, cx))
        .unwrap();
    harness.root.update(cx, |root, cx| {
        root.element = Some(element.clone());
        cx.notify();
    });
    cx.update(|_, cx| registry::apply(&overlay, true, cx));
    cx.run_until_parked();
    harness.web.take();
    // the deck gives it the keyboard, and an input method starts composing in it, before the engine runs
    let focus = element.read_with(cx, |element, _| element.focus().clone());
    cx.update(|window, cx| window.focus(&focus, cx));
    harness.root.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    cx.update(|window, cx| {
        element.update(cx, |element, cx| {
            element.replace_and_mark_text_in_range(None, "かな", None, window, cx);
        });
    });
    let page = element.read_with(cx, |element, _| Rc::clone(element.page()));
    assert!(page.web().is_none(), "not realized while the engine starts");
    assert!(
        cx.update(|window, _| focus.is_focused(window)),
        "GPUI holds the keyboard on the page"
    );
    assert_eq!(
        harness.web.take(),
        [],
        "nothing reaches an engine that does not run yet"
    );
    // the engine runs: the page's engine page is made holding the keyboard, with the composition in it
    let web: Rc<dyn crate::html_overlay::engine::Web> = Rc::clone(&harness.web) as _;
    cx.update(|_, _| registry::finish_start(web));
    cx.run_until_parked();
    let calls = harness.web.take();
    let at = |wanted: &Call| calls.iter().position(|call| call == wanted);
    let created = calls
        .iter()
        .position(|call| matches!(call, Call::CreateView { .. }))
        .unwrap_or_else(|| panic!("no page was made: {calls:?}"));
    let focused =
        at(&Call::Focused(true)).unwrap_or_else(|| panic!("the engine's page never got the keyboard: {calls:?}"));
    let composing =
        at(&Call::Preedit("かな".to_owned())).unwrap_or_else(|| panic!("the composition was lost: {calls:?}"));
    assert!(created < focused && focused < composing, "{calls:?}");
    // the keyboard leaving reaches the engine's page, as before
    cx.update(Window::blur);
    harness.root.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    let calls = harness.web.take();
    assert!(calls.contains(&Call::Focused(false)), "{calls:?}");
}

gpui_kit::actions!(html_overlay_tests, [WindowEscape]);

/// Upstream's #753 (`testEscapeOnAPageWithNoKeyHandlerNeverReachesTheWindow`,
/// `testEscapeStillReachesAPageListeningForItAndNeverReachesTheWindow`): an Esc on a page goes to its engine, press and
/// release, and nothing behind the page sees it: on Linux the page consumes every key it forwards, so a window-level
/// binding never fires, and full screen changes only through the app's own action.
#[gpui_kit::test]
fn escape_goes_to_the_page_and_no_further(cx: &mut TestAppContext) {
    let (harness, cx) = harness(cx);
    cx.update(|_, cx| crate::keymap_dispatch::init(cx));
    let fired = Rc::new(Cell::new(0_u32));
    let counted = Rc::clone(&fired);
    cx.update(|_, cx| {
        cx.bind_keys([gpui_kit::KeyBinding::new("escape", WindowEscape, None)]);
        cx.on_action(move |_: &WindowEscape, _| counted.set(counted.get() + 1));
    });
    let escape = || gpui_kit::Keystroke::parse("escape").unwrap();
    let press_and_release = |cx: &mut VisualTestContext| {
        cx.update(|window, cx| window.dispatch_keystroke(escape(), cx));
        cx.simulate_event(gpui_kit::KeyUpEvent { keystroke: escape() });
        cx.run_until_parked();
    };
    let fullscreen = cx.update(|window, _| window.is_fullscreen());
    // the binding works while the page does not hold the keyboard
    press_and_release(cx);
    assert_eq!(
        fired.get(),
        1,
        "the window's binding sees an Esc the page does not take"
    );
    harness.focus_page(cx);
    harness.web.take();
    press_and_release(cx);
    let keys: Vec<(bool, u32)> = harness
        .web
        .take()
        .iter()
        .filter_map(|call| match call {
            Call::Input(Input::Key(key)) => Some((key.pressed, key.keyval)),
            _ => None,
        })
        .collect();
    assert_eq!(
        keys,
        [(true, 0xff1b), (false, 0xff1b)],
        "the engine gets the Esc and its release"
    );
    assert_eq!(fired.get(), 1, "nothing behind the page sees it");
    assert_eq!(
        cx.update(|window, _| window.is_fullscreen()),
        fullscreen,
        "full screen unchanged"
    );
}

/// Upstream's c43bf01 (`testThePageStripOffersHideOnlyWhileTheOverlaysButtonIsOn`): an open page's strip offers Hide
/// only while Settings ▸ Interface ▸ Hide overlays is on, following the setting live; Close stays either way.
#[gpui_kit::test]
fn an_open_pages_strip_offers_hide_only_while_the_overlays_button_is_on(cx: &mut TestAppContext) {
    let (_harness, cx) = harness(cx);
    assert!(cx.debug_bounds("htmlOverlay.close").is_some());
    assert!(
        cx.debug_bounds("htmlOverlay.hide").is_none(),
        "no Hide while the overlays button is off, the default"
    );
    cx.update(|_, cx| crate::html_overlay::sync_strip_hide(true, cx));
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("htmlOverlay.hide").is_some(),
        "enabling the button adds Hide to the open page"
    );
    cx.update(|_, cx| crate::html_overlay::sync_strip_hide(false, cx));
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("htmlOverlay.hide").is_none(),
        "disabling it takes Hide away again"
    );
    assert!(cx.debug_bounds("htmlOverlay.close").is_some());
}

/// Upstream's 00fc83d: while the page holds the keyboard the `undo_close` chord (Ctrl+Shift+Z) goes to its engine
/// and fires nothing; with the keyboard elsewhere it still undoes a pending close, though the page covers the session.
#[gpui_kit::test]
fn the_undo_close_chord_goes_to_a_focused_page_and_undoes_the_close_otherwise(cx: &mut TestAppContext) {
    let (harness, cx) = harness(cx);
    cx.update(|window, cx| {
        crate::keymap_dispatch::init(cx);
        crate::keymap_dispatch::register_window(
            window.window_handle(),
            crate::keymap_dispatch::KeyOrigin::Window(uuid::Uuid::new_v4()),
        );
    });
    let (keymap, _) = ngterm_core::keymap::parse_keymap("", &ngterm_core::keymap::linux_defaults());
    crate::keymap_dispatch::load_keymap(&keymap);
    let chord = || gpui_kit::Keystroke::parse("ctrl-shift-z").unwrap();
    // the release as the page's listener and the window's (`crate::window`, not in this harness) both see it
    let press = |cx: &mut VisualTestContext| {
        cx.update(|window, cx| window.dispatch_keystroke(chord(), cx));
        let up = gpui_kit::KeyUpEvent { keystroke: chord() };
        cx.simulate_event(up.clone());
        cx.update(|window, cx| crate::keymap_dispatch::key_up(&up, window, cx));
        cx.run_until_parked();
    };
    let undo = vec![crate::keymap_dispatch::Fired::Builtin(
        ngterm_core::keymap::BuiltinAction::UndoClose,
    )];
    press(cx);
    assert_eq!(
        crate::keymap_dispatch::take_fired(),
        undo,
        "the page covers the session but does not hold the keyboard"
    );
    harness.focus_page(cx);
    harness.web.take();
    press(cx);
    assert!(crate::keymap_dispatch::take_fired().is_empty(), "nothing fires");
    let keys: Vec<bool> = harness
        .web
        .take()
        .iter()
        .filter_map(|call| match call {
            Call::Input(Input::Key(key)) => Some(key.pressed),
            _ => None,
        })
        .collect();
    assert_eq!(keys, [true, false], "the page's engine gets the chord and its release");
}

#[gpui_kit::test]
fn composition_ranges_use_utf16_and_the_candidate_window_follows_the_engine_caret(cx: &mut TestAppContext) {
    use gpui_kit::{Bounds, EntityInputHandler as _};
    let (harness, cx) = harness(cx);
    harness.web.take();
    let view = harness.view(cx);
    Harness::dispatch(
        &Event::ImeCursorArea {
            view,
            area: Area {
                x: 7,
                y: 9,
                width: 0,
                height: 18,
            },
        },
        cx,
    );
    cx.update(|window, cx| {
        harness.element.update(cx, |element, cx| {
            let bounds = Bounds::new(point(px(100.), px(200.)), size(px(300.), px(400.)));
            assert_eq!(element.selected_text_range(false, window, cx).unwrap().range, 0..0);
            assert_eq!(element.marked_text_range(window, cx), None);
            element.replace_and_mark_text_in_range(None, "a😀", None, window, cx);
            assert_eq!(element.selected_text_range(false, window, cx).unwrap().range, 3..3);
            assert_eq!(element.marked_text_range(window, cx), Some(0..3));
            assert_eq!(element.text_for_range(0..3, &mut None, window, cx), None);
            assert_eq!(
                element.character_index_for_point(point(px(0.), px(0.)), window, cx),
                None
            );
            assert_eq!(
                element.bounds_for_range(0..3, bounds, window, cx),
                Some(Bounds::new(point(px(107.), px(209.)), size(px(1.), px(18.))))
            );
            element.unmark_text(window, cx);
            element.unmark_text(window, cx);
            assert_eq!(element.marked_text_range(window, cx), None);
            element.replace_text_in_range(None, "typed", window, cx);
        });
    });
    assert_eq!(
        harness.web.take(),
        [
            Call::Preedit("a😀".to_owned()),
            Call::Commit("a😀".to_owned()),
            Call::Commit("typed".to_owned())
        ]
    );
}

#[gpui_kit::test]
fn focus_loss_cancels_preedit_and_a_missing_caret_has_no_candidate_bounds(cx: &mut TestAppContext) {
    use gpui_kit::{Bounds, EntityInputHandler as _};
    let (harness, cx) = harness(cx);
    harness.focus_page(cx);
    harness.web.take();
    cx.update(|window, cx| {
        harness.element.update(cx, |element, cx| {
            assert_eq!(element.bounds_for_range(0..0, Bounds::default(), window, cx), None);
            element.replace_and_mark_text_in_range(None, "かな", None, window, cx);
            element.focus_changed(false, cx);
            assert_eq!(element.marked_text_range(window, cx), None);
            element.replace_and_mark_text_in_range(None, "", None, window, cx);
            assert_eq!(element.marked_text_range(window, cx), None);
        });
    });
    assert_eq!(
        harness.web.take(),
        [
            Call::Preedit("かな".to_owned()),
            Call::Focused(false),
            Call::Preedit(String::new()),
            Call::Preedit(String::new())
        ]
    );
}

#[gpui_kit::test]
fn text_input_commits_directly_and_composing_native_keys_are_left_to_the_input_method(cx: &mut TestAppContext) {
    let (harness, cx) = harness(cx);
    harness.web.take();
    let mut key = gpui_kit::Keystroke::parse("a").unwrap();
    key.key_char = Some("a".to_owned());
    let source = crate::terminal::input::Source::Text;
    let mut native = crate::terminal::input::KeyStroke::new(&key, &source, false);
    harness.element.update(cx, |element, cx| {
        assert!(element.key_down(&key, &native, &source, cx));
        native.composing = true;
        assert!(!element.key_down(&key, &native, &crate::terminal::input::Source::Keystroke, cx));
        let unknown = gpui_kit::Keystroke {
            key: "unknown-key".to_owned(),
            key_char: None,
            ..key.clone()
        };
        native.composing = false;
        assert!(element.key_down(&unknown, &native, &crate::terminal::input::Source::Keystroke, cx));
        assert!(!element.key_released(&unknown, &native));
        element.release(cx);
        assert!(!element.key_down(&key, &native, &source, cx));
    });
    assert_eq!(harness.web.take(), [Call::Commit("a".to_owned())]);
}

#[gpui_kit::test]
fn modifier_changes_emit_only_the_keys_that_changed(cx: &mut TestAppContext) {
    let (harness, cx) = harness(cx);
    harness.web.take();
    let all = Modifiers {
        shift: true,
        control: true,
        alt: true,
        platform: true,
        ..Default::default()
    };
    cx.update(|window, cx| {
        harness.element.update(cx, |element, cx| {
            for modifiers in [all, all, Modifiers::none()] {
                element.modifiers_changed(
                    &gpui_kit::ModifiersChangedEvent {
                        modifiers,
                        ..Default::default()
                    },
                    window,
                    cx,
                );
            }
        });
    });
    let calls = harness.web.take();
    let keys: Vec<_> = calls
        .iter()
        .filter_map(|call| match call {
            Call::Input(Input::Key(key)) => Some((key.pressed, key.keyval, key.modifiers)),
            _ => None,
        })
        .collect();
    let held = super::input::modifiers(all);
    assert_eq!(
        keys,
        [
            (true, 0xffe1, held),
            (true, 0xffe3, held),
            (true, 0xffe9, held),
            (true, 0xffeb, held),
            (false, 0xffe1, ngterm_wpe::Modifiers::empty()),
            (false, 0xffe3, ngterm_wpe::Modifiers::empty()),
            (false, 0xffe9, ngterm_wpe::Modifiers::empty()),
            (false, 0xffeb, ngterm_wpe::Modifiers::empty()),
        ]
    );
}

#[gpui_kit::test]
fn wheel_events_keep_local_coordinates_precision_and_the_end_phase(cx: &mut TestAppContext) {
    let (harness, cx) = harness(cx);
    harness.web.take();
    harness.element.update(cx, |element, _| {
        element.scroll(&gpui_kit::ScrollWheelEvent {
            position: point(px(30.), px(TOP + STRIP + 20.)),
            delta: gpui_kit::ScrollDelta::Pixels(point(px(2.5), px(-3.5))),
            touch_phase: gpui_kit::TouchPhase::Ended,
            ..Default::default()
        });
    });
    let calls = harness.web.take();
    assert!(matches!(&calls[..], [Call::Input(Input::Scroll(scroll))]
        if (scroll.x - 30.).abs() < 1e-9 && (scroll.y - 20.).abs() < 1e-9 && scroll.precise && scroll.stop
        && (scroll.delta_x - 2.5).abs() < 1e-9 && (scroll.delta_y + 3.5).abs() < 1e-9));
}

fn select_menu(view: ViewId, token: u64) -> Event {
    Event::OptionMenu(OptionMenu {
        view,
        token: MenuToken::from_raw(token),
        area: Area {
            x: 0,
            y: 0,
            width: 100,
            height: 20,
        },
        items: ["one", "two"]
            .into_iter()
            .enumerate()
            .map(|(index, label)| OptionItem {
                label: label.to_owned(),
                enabled: true,
                selected: index == 0,
                kind: OptionKind::Option,
                ..Default::default()
            })
            .collect(),
    })
}

#[gpui_kit::test]
fn a_selects_keyboard_moves_and_picks_and_engine_close_does_not_answer_again(cx: &mut TestAppContext) {
    let (harness, cx) = harness(cx);
    harness.focus_page(cx);
    let view = harness.view(cx);
    Harness::dispatch(&select_menu(view, 10), cx);
    harness.web.take();
    harness.element.update(cx, |element, cx| {
        for key in ["down", "up", "down", "enter"] {
            assert!(element.menu_key(&gpui_kit::Keystroke::parse(key).unwrap(), cx));
        }
        assert!(!element.menu_key(&gpui_kit::Keystroke::parse("down").unwrap(), cx));
    });
    assert_eq!(harness.web.take(), [Call::Choose(MenuToken::from_raw(10), Some(1))]);
    Harness::dispatch(&select_menu(view, 11), cx);
    Harness::dispatch(
        &Event::OptionMenuClosed {
            token: MenuToken::from_raw(12),
        },
        cx,
    );
    assert!(harness.element.read_with(cx, |element, _| element.menu.is_some()));
    Harness::dispatch(
        &Event::OptionMenuClosed {
            token: MenuToken::from_raw(11),
        },
        cx,
    );
    assert!(harness.element.read_with(cx, |element, _| element.menu.is_none()));
    assert!(harness.web.take().is_empty());
}

#[gpui_kit::test]
fn context_history_actions_go_to_the_engine_and_empty_paste_clears_its_clipboard(cx: &mut TestAppContext) {
    use super::menus::Action;
    let (harness, cx) = harness(cx);
    harness.web.take();
    harness.element.update(cx, |element, cx| {
        for action in [
            Action::Back,
            Action::Forward,
            Action::Reload,
            Action::Editing(EditingCommand::PasteAsPlainText),
        ] {
            element.run(action, cx);
        }
        element.pick(999, cx);
    });
    assert_eq!(
        harness.web.take(),
        [
            Call::GoBack,
            Call::GoForward,
            Call::Reload,
            Call::Clipboard(String::new()),
            Call::Editing(EditingCommand::PasteAsPlainText)
        ]
    );
}

#[gpui_kit::test]
fn middle_click_hands_the_primary_selection_to_the_engine_before_paste(cx: &mut TestAppContext) {
    let (harness, cx) = harness(cx);
    cx.update(|_, cx| cx.write_to_primary(ClipboardItem::new_string("primary".to_owned())));
    harness.web.take();
    let at = point(px(20.), px(150.));
    cx.update(|window, cx| {
        harness.element.update(cx, |element, cx| {
            element.mouse_down(
                &gpui_kit::MouseDownEvent {
                    button: MouseButton::Middle,
                    position: at,
                    ..Default::default()
                },
                window,
                cx,
            );
        });
    });
    let calls = harness.web.take();
    assert!(
        matches!(&calls[..], [Call::Clipboard(text), Call::Input(Input::Pointer(pointer)), Call::Editing(EditingCommand::Paste), ..]
        if text == "primary" && pointer.kind == PointerKind::Down(ngterm_wpe::Button::Middle)),
        "{calls:?}"
    );
}

#[gpui_kit::test]
fn a_released_element_draws_nothing_and_ignores_late_engine_events(cx: &mut TestAppContext) {
    let (harness, cx) = harness(cx);
    let view = harness.view(cx);
    harness.element.update(cx, PageView::release);
    Harness::dispatch(&shm_frame(view, 73), cx);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    assert!(
        harness
            .element
            .read_with(cx, |element, _| element.shown_surface().is_none())
    );
    assert_eq!(harness.element.read_with(cx, |element, _| element.frame_stats().shm), 0);
}

fn external_link(view: ViewId) -> Event {
    Event::Policy(ngterm_wpe::PolicyRequest {
        view,
        token: ngterm_wpe::PolicyToken::from_raw(90),
        kind: ngterm_wpe::PolicyKind::Navigation {
            uri: "https://outside.invalid/path".to_owned(),
            navigation: ngterm_wpe::NavigationType::LinkClicked,
            new_window: true,
        },
    })
}

#[gpui_kit::test]
fn an_external_link_asks_once_and_cancellation_silences_it_until_real_input(cx: &mut TestAppContext) {
    let (harness, cx) = harness(cx);
    let view = harness.view(cx);
    Harness::dispatch(&external_link(view), cx);
    let first = cx
        .update(|window, cx| crate::modal::front_alert(window.window_handle(), cx))
        .unwrap();
    Harness::dispatch(&external_link(view), cx);
    let second = cx
        .update(|window, cx| crate::modal::front_alert(window.window_handle(), cx))
        .unwrap();
    assert_eq!(first, second, "a second link cannot stack another prompt");
    cx.update(|window, cx| assert!(crate::modal::answer_front_alert(window, "Cancel", cx)));
    Harness::dispatch(&external_link(view), cx);
    assert!(!cx.update(|window, cx| crate::modal::alert_open(window.window_handle(), cx)));
    let page = harness.element.read_with(cx, |element, _| element.page.clone());
    page.user_input(false);
    Harness::dispatch(&external_link(view), cx);
    assert!(cx.update(|window, cx| crate::modal::alert_open(window.window_handle(), cx)));
    cx.update(|window, cx| assert!(crate::modal::answer_front_alert(window, "Cancel", cx)));
}

#[gpui_kit::test]
fn hiding_a_page_withdraws_its_external_link_prompt(cx: &mut TestAppContext) {
    let (harness, cx) = harness(cx);
    let view = harness.view(cx);
    Harness::dispatch(&external_link(view), cx);
    assert!(cx.update(|window, cx| crate::modal::alert_open(window.window_handle(), cx)));
    // the deck applies visibility while its window is being updated
    cx.update(|_, cx| registry::apply(&harness.overlay, false, cx));
    cx.run_until_parked();
    assert!(!cx.update(|window, cx| crate::modal::alert_open(window.window_handle(), cx)));
    // the withdrawal was no answer: the page asks again once shown
    cx.update(|_, cx| registry::apply(&harness.overlay, true, cx));
    Harness::dispatch(&external_link(view), cx);
    assert!(cx.update(|window, cx| crate::modal::alert_open(window.window_handle(), cx)));
    cx.update(|window, cx| assert!(crate::modal::answer_front_alert(window, "Cancel", cx)));
}

#[gpui_kit::test]
fn a_late_withdrawal_leaves_the_prompt_that_replaced_its_own(cx: &mut TestAppContext) {
    let (harness, cx) = harness(cx);
    let view = harness.view(cx);
    Harness::dispatch(&external_link(view), cx);
    let first = cx
        .update(|window, cx| crate::modal::front_alert(window.window_handle(), cx))
        .unwrap();
    // a hand-off asked before a hide and a show in one flush opens its prompt before the hide withdraws the first
    cx.update(|_, cx| {
        registry::dispatch(&external_link(view), cx);
        registry::apply(&harness.overlay, false, cx);
        registry::apply(&harness.overlay, true, cx);
    });
    cx.run_until_parked();
    let second = cx
        .update(|window, cx| crate::modal::front_alert(window.window_handle(), cx))
        .unwrap();
    assert_ne!(first, second);
    // the page still holds the new prompt: hiding withdraws it, unanswered, so the page asks again once shown
    cx.update(|_, cx| registry::apply(&harness.overlay, false, cx));
    cx.run_until_parked();
    assert!(!cx.update(|window, cx| crate::modal::alert_open(window.window_handle(), cx)));
    cx.update(|_, cx| registry::apply(&harness.overlay, true, cx));
    Harness::dispatch(&external_link(view), cx);
    assert!(cx.update(|window, cx| crate::modal::alert_open(window.window_handle(), cx)));
    cx.update(|window, cx| assert!(crate::modal::answer_front_alert(window, "Cancel", cx)));
}

/// Upstream's `TerminalInputSourceTests` (`testPageAndItsSubviewsHoldTheLock`, `testTextFieldsAndChromeReleaseTheLock`):
/// the keyboard-layout lock holds while the page has the keyboard and lets go when a text field outside the page
/// (Settings, the palette, an ask dialog) takes it or the window loses its activation; each move re-evaluates by
/// itself, with no terminal involved.
#[gpui_kit::test]
fn the_layout_lock_holds_while_the_page_has_the_keyboard_and_not_a_field_outside_it(cx: &mut TestAppContext) {
    use crate::input_source::fake::{RU, US, installed};

    let system = installed(cx, RU);
    let (harness, cx) = harness(cx);
    let field = cx.new(|cx| Modal {
        focus: cx.focus_handle(),
    });
    cx.update(|_, cx| crate::input_source::set_target(Some(US.to_owned()), cx));
    cx.run_until_parked();
    assert!(system.borrow().selected.is_empty(), "nothing holds the keyboard yet");

    harness.focus_page(cx);
    assert_eq!(
        system.borrow().selected,
        [US],
        "the page taking the keyboard engages the lock"
    );
    let focus = field.read_with(cx, |field, _| field.focus.clone());
    cx.update(|window, cx| window.focus(&focus, cx));
    cx.run_until_parked();
    assert_eq!(
        system.borrow().selected,
        [US, RU],
        "a field outside the page releases it"
    );

    harness.focus_page(cx);
    assert_eq!(
        system.borrow().selected,
        [US, RU, US],
        "back in the page, it engages again"
    );
    cx.deactivate_window();
    cx.run_until_parked();
    assert_eq!(
        system.borrow().selected,
        [US, RU, US, RU],
        "an inactive window's page holds nothing"
    );
}
