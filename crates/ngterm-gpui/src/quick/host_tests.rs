//! The non-Linux panel host always uses an ordinary platform window.

use super::*;

#[test]
fn no_compositor_selects_or_refuses_a_layer_surface() {
    for compositor in ["Wayland", "X11", "", "Windows"] {
        assert_eq!(first_host(compositor), QuickHost::Window);
    }
    assert!(!refuses_layer_shell(&Ok(())));
    let failed: gpui_kit::Result<()> = Err(std::io::Error::other("no GPU").into());
    assert!(!refuses_layer_shell(&failed));
}

#[test]
fn a_requested_layer_surface_falls_back_to_the_window_host() {
    let target = Target {
        id: DisplayId::new(7),
        bounds: Bounds {
            origin: point(px(100.), px(50.)),
            size: size(px(1920.), px(1080.)),
        },
    };
    for zoomed in [false, true] {
        for display in [None, Some(&target)] {
            for named in [false, true] {
                let fit = Fit {
                    size_percent: Some(60),
                    zoomed,
                };
                let fallback = window_options(QuickHost::LayerShell, fit, display, named, "test.panel").unwrap();
                let ordinary = window_options(QuickHost::Window, fit, display, named, "test.panel").unwrap();
                assert_eq!(fallback.window_bounds, ordinary.window_bounds);
                assert_eq!(fallback.display_id, ordinary.display_id);
                assert_eq!(fallback.app_id.as_deref(), Some("test.panel"));
                assert!(matches!(fallback.kind, WindowKind::PopUp));
                assert!(!fallback.is_resizable && !fallback.is_minimizable && !fallback.is_movable);
                assert!(fallback.focus && fallback.show);
            }
        }
    }
}
