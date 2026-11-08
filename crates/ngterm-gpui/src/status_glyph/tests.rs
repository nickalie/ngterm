//! The status glyph: the resolver and the blink, ported from the GTK build's `status_glyph.rs` tests; the silhouettes'
//! coverage; and the glyph in GPUI's test window (its slot, its blink running only where animations are on).

use gpui_kit::{Context, Render, TestAppContext, Window, div};

use super::*;

fn indicator(status: AgentStatus) -> AgentIndicator {
    AgentIndicator {
        status,
        ..AgentIndicator::default()
    }
}

fn look(status: AgentStatus, settings: &AppSettings) -> GlyphLook {
    resolve(&indicator(status), settings).expect("a non-idle status has a look")
}

#[test]
fn idle_renders_nothing() {
    let settings = AppSettings {
        active_status_color_hex: Some("#112233".into()),
        ..AppSettings::default()
    };
    let idle = AgentIndicator {
        blink: true,
        color: Some("#abcdef".into()),
        shape: Some(StatusShape::Star),
        ..AgentIndicator::default()
    };
    assert_eq!(resolve(&idle, &settings), None);
    assert_eq!(default_rgb(AgentStatus::Idle), None);
}

#[test]
fn unset_settings_resolve_to_the_default_tints_and_a_circle() {
    let settings = AppSettings::default();
    let active = look(AgentStatus::Active, &settings);
    assert_eq!(active.rgb, rgb8(0xDB, 0xD9, 0xE6));
    assert_eq!(active.shape, StatusShape::Circle);
    assert!(!active.blink);
    assert_eq!(look(AgentStatus::Blocked, &settings).rgb, rgb8(0xFF, 0x95, 0x00));
    assert_eq!(look(AgentStatus::Completed, &settings).rgb, rgb8(0x34, 0xC7, 0x59));
    assert_eq!(default_rgb(AgentStatus::Blocked), Some(DEFAULT_BLOCKED));
}

#[test]
fn configured_colors_and_shapes_apply_per_status() {
    let settings = AppSettings {
        active_status_color_hex: Some("#102030".into()),
        blocked_status_color_hex: Some("405060".into()),
        completed_status_color_hex: Some(" #708090 ".into()),
        active_status_shape: Some("square".into()),
        blocked_status_shape: Some("triangle".into()),
        completed_status_shape: Some("star".into()),
        ..AppSettings::default()
    };
    let active = look(AgentStatus::Active, &settings);
    assert_eq!(
        (active.rgb, active.shape),
        (rgb8(0x10, 0x20, 0x30), StatusShape::Square)
    );
    let blocked = look(AgentStatus::Blocked, &settings);
    assert_eq!(
        (blocked.rgb, blocked.shape),
        (rgb8(0x40, 0x50, 0x60), StatusShape::Triangle)
    );
    let completed = look(AgentStatus::Completed, &settings);
    assert_eq!(
        (completed.rgb, completed.shape),
        (rgb8(0x70, 0x80, 0x90), StatusShape::Star)
    );
}

#[test]
fn malformed_configured_values_fall_back_to_the_defaults() {
    let settings = AppSettings {
        blocked_status_color_hex: Some("#12345".into()),
        blocked_status_shape: Some("hexagon".into()),
        ..AppSettings::default()
    };
    let blocked = look(AgentStatus::Blocked, &settings);
    assert_eq!((blocked.rgb, blocked.shape), (DEFAULT_BLOCKED, StatusShape::Circle));
}

#[test]
fn a_valid_per_call_override_wins_over_the_settings() {
    let settings = AppSettings {
        completed_status_color_hex: Some("#102030".into()),
        completed_status_shape: Some("square".into()),
        ..AppSettings::default()
    };
    let indicator = AgentIndicator {
        status: AgentStatus::Completed,
        color: Some("#FFfe01".into()),
        shape: Some(StatusShape::Capsule),
        ..AgentIndicator::default()
    };
    let completed = resolve(&indicator, &settings).expect("completed has a look");
    assert_eq!(
        (completed.rgb, completed.shape),
        (rgb8(0xFF, 0xFE, 0x01), StatusShape::Capsule)
    );
}

#[test]
fn a_malformed_per_call_color_falls_back_to_the_configured_then_the_default() {
    let indicator = AgentIndicator {
        status: AgentStatus::Active,
        color: Some("#zzzzzz".into()),
        ..AgentIndicator::default()
    };
    let configured = AppSettings {
        active_status_color_hex: Some("#102030".into()),
        ..AppSettings::default()
    };
    let resolved = resolve(&indicator, &configured).expect("active has a look");
    assert_eq!(resolved.rgb, rgb8(0x10, 0x20, 0x30));
    let unset = resolve(&indicator, &AppSettings::default()).expect("active has a look");
    assert_eq!(unset.rgb, DEFAULT_ACTIVE);
}

#[test]
fn blink_and_tooltip_follow_the_status() {
    let indicator = AgentIndicator {
        status: AgentStatus::Blocked,
        blink: true,
        ..AgentIndicator::default()
    };
    let blocked = resolve(&indicator, &AppSettings::default()).expect("blocked has a look");
    assert!(blocked.blink);
    assert_eq!(blocked.status, AgentStatus::Blocked);
    assert_eq!(blocked.tooltip, "Agent status: Blocked");
    assert_eq!(
        look(AgentStatus::Active, &AppSettings::default()).tooltip,
        "Agent status: Active"
    );
    assert_eq!(
        look(AgentStatus::Completed, &AppSettings::default()).tooltip,
        "Agent status: Completed"
    );
}

#[test]
fn hex_parsing_accepts_six_digits_with_an_optional_hash() {
    assert_eq!(parse_hex("#000000"), Some((0.0, 0.0, 0.0)));
    assert_eq!(parse_hex("ffffff"), Some((1.0, 1.0, 1.0)));
    for malformed in ["", "#", "#fff", "#1234567", "#12345g", "red"] {
        assert_eq!(parse_hex(malformed), None, "{malformed:?}");
    }
}

#[test]
fn the_blink_fades_to_a_fifth_and_back_every_second() {
    assert!((blink_opacity(0) - 1.0).abs() < 1e-9);
    assert!((blink_opacity(250_000) - 0.6).abs() < 1e-9);
    assert!((blink_opacity(500_000) - BLINK_LOW).abs() < 1e-9);
    assert!((blink_opacity(750_000) - 0.6).abs() < 1e-9);
    assert!((blink_opacity(1_000_000) - 1.0).abs() < 1e-9);
    assert!((blink_opacity(3_500_000) - BLINK_LOW).abs() < 1e-9);
    // the animation's progress through its one-second period
    assert!((blink_at(0.0) - 1.0).abs() < 1e-6);
    assert!((blink_at(0.5) - 0.2).abs() < 1e-6);
    assert!((blink_at(1.0) - 1.0).abs() < 1e-6);
}

#[test]
fn star_points_alternate_outer_and_inner_and_center_vertically() {
    let points = star_points(6.5);
    let top = points.iter().map(|point| point.1).fold(f64::INFINITY, f64::min);
    let bottom = points.iter().map(|point| point.1).fold(f64::NEG_INFINITY, f64::max);
    assert!((top + bottom).abs() < 1e-9, "top {top}, bottom {bottom}");
    assert!(
        (points[0].0).abs() < 1e-9 && points[0].1 < 0.0,
        "the first point is the top one"
    );
}

/// Every shape's coverage at 2x scale (32 pixels to the 16-point slot), in [`StatusShape::ALL`]'s order.
fn rendered() -> Vec<Vec<u8>> {
    StatusShape::ALL.iter().map(|&shape| rasterize(shape, 32)).collect()
}

/// `shape`'s coverage among [`rendered`]'s.
fn of(rendered: &[Vec<u8>], shape: StatusShape) -> &[u8] {
    &rendered[StatusShape::ALL.iter().position(|&each| each == shape).unwrap()]
}

/// Whether pixel `index` of a 32-pixel canvas lies in the slot's outer 1.5 points, outside the 13-point extent.
fn in_the_margin(index: usize) -> bool {
    let inside = 2..=29;
    !inside.contains(&(index / 32)) || !inside.contains(&(index % 32))
}

#[test]
fn every_shape_fills_its_own_silhouette_inside_the_slot() {
    let rendered = rendered();
    for (shape, alpha) in StatusShape::ALL.iter().zip(&rendered) {
        // the slot's center is inside every silhouette, its corners and margin outside all
        assert_eq!(alpha[16 * 32 + 16], 255, "{shape:?} is filled at its center");
        let inked_outside = (0..32 * 32).filter(|&index| alpha[index] > 0 && in_the_margin(index));
        assert_eq!(inked_outside.count(), 0, "{shape:?} stays inside its extent");
    }
    for (i, first) in rendered.iter().enumerate() {
        for second in rendered.iter().skip(i + 1) {
            assert_ne!(first, second, "{:?} looks like another shape", StatusShape::ALL[i]);
        }
    }
}

#[test]
fn each_silhouette_has_its_own_outline() {
    let rendered = rendered();
    // the capsule is wide and flat: design (8, 3) is outside it, (2, 8) inside
    let capsule = of(&rendered, StatusShape::Capsule);
    assert_eq!((capsule[6 * 32 + 16], capsule[16 * 32 + 4]), (0, 255));
    // play points right: its right tip is inked at mid-height, not above it, and its left side is flat
    let play = of(&rendered, StatusShape::Play);
    assert!(play[16 * 32 + 26] > 0 && play[6 * 32 + 26] == 0);
    assert_eq!(play[8 * 32 + 8], 255);
    // the bell's clapper hangs below its rim, apart from it at the sides
    let bell = of(&rendered, StatusShape::Bell);
    assert_eq!((bell[27 * 32 + 16], bell[27 * 32 + 8]), (255, 0));
    // the bolt's two strokes leave its upper left and lower right empty
    let bolt = of(&rendered, StatusShape::Bolt);
    assert_eq!((bolt[6 * 32 + 6], bolt[26 * 32 + 26]), (0, 0));
}

#[test]
fn the_down_triangle_is_the_triangle_upside_down() {
    let rendered = rendered();
    let (up, down) = (
        of(&rendered, StatusShape::Triangle),
        of(&rendered, StatusShape::TriangleDown),
    );
    for (index, &a) in up.iter().enumerate() {
        let (row, column) = (index / 32, index % 32);
        let b = down[(31 - row) * 32 + column];
        assert!(a.abs_diff(b) <= 1, "row {row}, column {column}: {a} against {b}");
    }
}

#[test]
fn the_seal_scallops_its_rim_and_stays_round() {
    let points = seal_points(6.5);
    let radii: Vec<f64> = points.iter().map(|(x, y)| x.hypot(*y)).collect();
    let (low, high) = radii.iter().fold((f64::INFINITY, 0.0_f64), |(low, high), &radius| {
        (low.min(radius), high.max(radius))
    });
    assert!((high - 6.5).abs() < 1e-9, "outer {high}");
    assert!(low > 5.5 && low < 6.3, "inner {low}");
}

/// A view showing one glyph.
struct Shown {
    look: GlyphLook,
    animate: bool,
}

impl Render for Shown {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui_kit::IntoElement {
        div().child(glyph(&self.look, self.animate))
    }
}

#[gpui_kit::test]
fn the_glyph_keeps_its_slot_and_blinks_only_while_animations_run(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    assert!(cx.update(|cx| animations_enabled(cx)), "on while unknown");
    let mut blocked = look(AgentStatus::Blocked, &AppSettings::default());
    blocked.blink = true;
    let (view, cx) = cx.add_window_view(|_, _| Shown {
        look: blocked,
        animate: true,
    });
    cx.run_until_parked();
    let bounds = cx.debug_bounds("agent-status").expect("the glyph is laid out");
    assert_eq!(bounds.size, gpui_kit::size(px(SLOT), px(SLOT)));
    assert!(
        cx.debug_bounds("agent-status-blinking").is_some(),
        "a blinking status blinks"
    );
    assert!(cx.debug_bounds("agent-status-still").is_none());
    view.update(cx, |view, cx| {
        view.animate = false;
        cx.notify();
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("agent-status-still").is_some(), "no animations: still");
    assert!(cx.debug_bounds("agent-status-blinking").is_none());
    let bounds = cx.debug_bounds("agent-status").expect("the glyph is laid out");
    assert_eq!(bounds.size, gpui_kit::size(px(SLOT), px(SLOT)), "the same slot");

    // a status that does not blink stays still with animations on
    view.update(cx, |view, cx| {
        view.animate = true;
        view.look.blink = false;
        cx.notify();
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("agent-status-still").is_some());
    cx.update(|_, cx| {
        cx.set_global(Motion { animations: false });
        assert!(!animations_enabled(cx));
    });
}

#[test]
fn the_slot_reads_as_text_valued_with_the_status_name() {
    use gpui_kit::{Element as _, Role, accesskit::Node};
    let blocked = look(AgentStatus::Blocked, &AppSettings::default());
    let element = slot(&blocked);
    assert_eq!(element.a11y_role(), Some(Role::Label));
    let mut node = Node::new(Role::Label);
    element.write_a11y_info(&mut node);
    assert_eq!(node.value(), Some("blocked"));
    assert_eq!(node.description(), Some(blocked.tooltip.as_str()));
}

#[gpui_kit::test]
fn watching_motion_keeps_an_existing_preference_and_set_motion_reports_both_states(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_global(Motion { animations: false }));
    cx.update(watch_motion);
    assert!(!cx.update(|cx| animations_enabled(cx)));
    let mut app = cx.to_async();
    set_motion(&mut app, true);
    assert!(cx.update(|cx| animations_enabled(cx)));
    set_motion(&mut app, true);
    set_motion(&mut app, false);
    assert!(!cx.update(|cx| animations_enabled(cx)));
}

#[cfg(windows)]
#[test]
fn animation_notifications_are_followed_in_an_isolated_process() {
    crate::system_window::tests::run_child(
        "status_glyph::tests::windows_setting_changes_refresh_the_animation_preference",
    );
}

#[cfg(windows)]
#[gpui_kit::test]
#[ignore = "run by animation_notifications_are_followed_in_an_isolated_process"]
fn windows_setting_changes_refresh_the_animation_preference(cx: &mut TestAppContext) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW, WM_SETTINGCHANGE,
    };

    if !crate::system_window::tests::is_child(
        "status_glyph::tests::windows_setting_changes_refresh_the_animation_preference",
    ) {
        return;
    }
    // The real window thread wakes the foreground task when its notification arrives.
    cx.executor().allow_parking();
    let _events = crate::system_window::tests::EVENTS.lock().unwrap();
    let mut on: windows_sys::core::BOOL = 0;
    // SAFETY: a writable BOOL, exactly the output SPI_GETCLIENTAREAANIMATION requires.
    let read = unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&raw mut on).cast(), 0) };
    assert_ne!(read, 0);
    let expected = on != 0;
    assert_eq!(client_area_animation(), expected);
    cx.update(watch_motion);
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| animations_enabled(cx)), expected);
    cx.update(|cx| cx.set_global(Motion { animations: !expected }));
    crate::system_window::tests::send(WM_SETTINGCHANGE, SPI_GETCLIENTAREAANIMATION as usize, 0);
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| animations_enabled(cx)), expected);
}
