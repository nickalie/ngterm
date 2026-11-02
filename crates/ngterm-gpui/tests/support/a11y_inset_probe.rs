//! Test-only surface for the private-Xvfb accessibility probe; never launch on a desktop.
#[cfg(target_os = "linux")]
fn main() {
    use gpui_kit::prelude::*;
    use gpui_kit::{App, Context, IntoElement, ParentElement, Render, Styled, Window, div, px};
    struct Probe;
    impl Render for Probe {
        fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            window.set_client_inset(px(12.0));
            div().size_full().p(px(12.0)).child(
                div()
                    .id("inset-control")
                    .accessibility_id("inset-control")
                    .role(gpui_kit::Role::Button)
                    .w(px(100.0))
                    .h(px(40.0)),
            )
        }
    }
    assert_eq!(std::env::var("CONFORMANCE_A11Y_INSET_PROBE").as_deref(), Ok("1"));
    gpui_kit::application().run(|cx: &mut App| {
        let options = gpui_kit::WindowOptions {
            window_decorations: Some(gpui_kit::WindowDecorations::Client),
            ..Default::default()
        };
        if let Err(error) = cx.open_window(options, |_, cx| cx.new(|_| Probe)) {
            eprintln!("{error}");
            cx.quit();
        }
    });
}

#[cfg(not(target_os = "linux"))]
fn main() {}
