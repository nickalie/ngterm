//! Paint output for terminal decorations, padding, and cursor shapes.

use gpui_kit::{TestAppContext, point};

use super::super::surface::tests::Fixture;
use super::*;

fn colored_quads(f: &mut Fixture<'_>, rgb: u32) -> Vec<gpui_kit::Quad> {
    let color: gpui_kit::Hsla = gpui_kit::rgb(rgb).into();
    f.cx.update(|window, _| {
        window
            .painted_quads()
            .into_iter()
            .filter(|quad| quad.background.as_solid() == Some(color))
            .collect()
    })
}

#[gpui_kit::test]
#[cfg_attr(windows, ignore = "drives its pane with a POSIX sh script")]
fn decorated_cells_paint_distinct_underline_styles_and_overlines(cx: &mut TestAppContext) {
    let script = "printf '\\033[?25l\\033[38;2;255;0;0m\\033[4:2mA\\033[4:4mB\\033[4:5mC\\033[4:3mD\\033[4:0;53mE\\033[0mready'; exec cat";
    let mut f = Fixture::new(cx, script, "");
    let quads = colored_quads(&mut f, 0x00ff_0000);
    let at_column = |col: u16| {
        let x = 2.0 + f32::from(col) * 10.0;
        quads
            .iter()
            .filter(|quad| (x..x + 10.0).contains(&quad.bounds.origin.x.0))
            .collect::<Vec<_>>()
    };
    let double = at_column(0);
    assert_eq!(double.len(), 2, "double underline paints two stripes");
    assert_ne!(double[0].bounds.origin.y, double[1].bounds.origin.y);
    let dotted = at_column(1);
    assert!(dotted.len() >= 2, "dots span the cell");
    assert!(dotted.iter().all(|quad| quad.corner_radii.top_left.0 > 0.0));
    let dashed = at_column(2);
    assert_eq!(dashed.len(), 2);
    assert!(dashed.iter().all(|quad| quad.bounds.size.width.0 < 10.0));
    let overline = at_column(4);
    assert_eq!(overline.len(), 1);
    assert!(overline[0].bounds.origin.y.0 < dotted[0].bounds.origin.y.0);
    f.view.read_with(f.cx, |view, _| {
        assert!(
            view.row_layout(0)
                .unwrap()
                .decorations
                .iter()
                .any(|span| span.start == 3 && span.value.line == Line::Underline(Underline::Curly))
        );
        assert!(
            view.painted_layers()
                .iter()
                .any(|(layer, _)| *layer == Layer::Underlines)
        );
    });
}

#[gpui_kit::test]
#[cfg_attr(windows, ignore = "drives its pane with a POSIX sh script")]
fn padding_extension_reaches_window_edges_and_background_mode_keeps_padding(cx: &mut TestAppContext) {
    let script = "printf '\\033[?25l\\033[48;2;255;0;0m\\033[2J\\033[Hready'; exec cat";
    let mut f = Fixture::new(cx, script, "window-padding-color = extend\n");
    let quads = colored_quads(&mut f, 0x00ff_0000);
    assert!(
        quads
            .iter()
            .any(|quad| quad.bounds.origin == point(gpui_kit::ScaledPixels(0.), gpui_kit::ScaledPixels(0.)))
    );
    assert!(quads.iter().any(|quad| (quad.bounds.bottom().0 - 480.0).abs() < 0.01));
    assert!(quads.iter().all(|quad| (quad.bounds.size.width.0 - 640.0).abs() < 0.01));
    f.configure("window-padding-color = background\n");
    let quads = colored_quads(&mut f, 0x00ff_0000);
    assert!(
        quads
            .iter()
            .all(|quad| quad.bounds.origin.x.0 >= 2.0 && quad.bounds.origin.y.0 >= 2.0)
    );
    assert!(
        quads
            .iter()
            .all(|quad| quad.bounds.right().0 <= 638.0 && quad.bounds.bottom().0 <= 478.0)
    );
    f.configure("window-padding-color = extend-always\n");
    assert!(
        colored_quads(&mut f, 0x00ff_0000)
            .iter()
            .any(|quad| quad.bounds.origin.y.0 == 0.0)
    );
}

#[gpui_kit::test]
#[cfg_attr(windows, ignore = "drives its pane with a POSIX sh script")]
fn powerline_row_does_not_extend_vertically_under_conditional_padding(cx: &mut TestAppContext) {
    let script = "printf '\\033[?25l\\033[48;2;255;0;0m\\033[2J\\033[Hready\u{e0b0}'; exec cat";
    let mut f = Fixture::new(cx, script, "window-padding-color = extend\n");
    f.wait("\u{e0b0}");
    let quads = colored_quads(&mut f, 0x00ff_0000);
    assert!(quads.iter().all(|quad| quad.bounds.origin.y.0 >= 2.0));
    f.configure("window-padding-color = extend-always\n");
    assert!(
        colored_quads(&mut f, 0x00ff_0000)
            .iter()
            .any(|quad| quad.bounds.origin.y.0 == 0.0)
    );
}

#[gpui_kit::test]
#[cfg_attr(windows, ignore = "drives its pane with a POSIX sh script")]
fn focused_cursor_shapes_paint_a_bar_and_an_underline(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx, "printf '\\033[6 qready'; exec cat", "cursor-color = #00ff00\n");
    let view = f.view.clone();
    f.cx.update(|window, cx| window.focus(&view.read(cx).focus.clone(), cx));
    f.cx.run_until_parked();
    let bar = colored_quads(&mut f, 0x0000_ff00);
    assert_eq!(bar.len(), 1);
    assert!(bar[0].bounds.size.width.0 < bar[0].bounds.size.height.0);
    f.configure("cursor-color = #00ff00\ncursor-style = underline\n");
    assert!(f.perform("reset"));
    // the reset's repaint comes from the pump, which parking alone doesn't promise has run
    f.settle(|f| {
        colored_quads(f, 0x0000_ff00)
            .first()
            .is_some_and(|quad| quad.bounds.size.width.0 > quad.bounds.size.height.0)
    });
    assert_eq!(colored_quads(&mut f, 0x0000_ff00).len(), 1);
}

#[test]
fn pixel_conversion_floors_sizes_and_saturates_device_limits() {
    assert_eq!(device_from(3.9, 2.0), 7);
    assert_eq!(device_from(-4.0, 2.0), 0);
    assert!((device_to(9, 2.0) - 4.5).abs() < f32::EPSILON);
    assert_eq!(device_pixels(u32::MAX), DevicePixels(i32::MAX));
}
