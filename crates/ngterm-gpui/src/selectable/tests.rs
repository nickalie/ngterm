//! A selectable text as assistive technology reads it.

use gpui_kit::base::test_support::snapshots;
use gpui_kit::{Context, ParentElement as _, Render, TestAppContext, div, px};

use super::*;

const PATH: &str = "/tmp/state";
const DIAGNOSTICS: &str = "line 1: oops\nline 2: again";

/// A window drawing a one-line and a multi-line selectable text.
struct Probe(Selectable, Selectable);

impl Render for Probe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let ink = crate::chrome::color(ngterm_core::chrome::Rgb::WHITE, 1.0);
        div()
            .child(self.0.element("settings-path", ink, px(12.), true))
            .child(self.1.element("settings-diagnostics", ink, px(12.), false))
    }
}

#[gpui_kit::test]
fn a_selectable_text_reads_as_text_valued_with_what_it_holds_not_as_a_field(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let (_, cx) = cx.add_window_view(|window, cx| {
        Probe(
            Selectable::line(PATH, window, cx),
            Selectable::lines(DIAGNOSTICS, window, cx),
        )
    });
    cx.run_until_parked();
    let drawn = cx.update(|window, _| snapshots(window));
    for text in [PATH, DIAGNOSTICS] {
        let field = drawn
            .iter()
            .find(|snapshot| snapshot.value() == Some(text))
            .unwrap_or_else(|| panic!("{text:?} is drawn: {drawn:?}"));
        assert_eq!(
            field.role(),
            Some(Role::Label),
            "upstream's selectable text is static text, not an editable field: {field:?}"
        );
    }
}
