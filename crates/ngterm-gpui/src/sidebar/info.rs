//! Session info typography from upstream SidebarInfoCell.swift. Each grapheme keeps its font traits;
//! emoji presentation gets its own size and baseline, which a single GPUI `StyledText` cannot express.
use gpui_kit::{
    FontStyle, FontWeight, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Point, ShapedLine,
    SharedString, StatefulInteractiveElement as _, Styled as _, TextAlign, TextRun, Window, canvas, div, point,
    prelude::FluentBuilder as _, px,
};
use ngterm_core::session_info::SessionInfoSpan;
use unicode_properties::{UnicodeEmoji as _, emoji::EmojiStatus};
use unicode_segmentation::UnicodeSegmentation as _;

use super::look::{Look, color, tint};

pub(super) fn font_size(look: Look) -> f32 {
    (look.font_size - 1.5).max(9.)
}

/// The font's actual ascent, descent and leading determine the extra line's height.
pub(super) fn line_height(look: Look, window: &Window) -> f32 {
    let system = window.text_system();
    let id = system.resolve_font(&window.text_style().font());
    let metrics = system.font_metrics(id);
    let size = px(font_size(look));
    f32::from(metrics.ascent(size) - metrics.descent(size) + metrics.line_gap(size)).ceil()
}

pub(super) fn ink(span: &SessionInfoSpan, look: Look, selected: bool) -> Hsla {
    if let Some(value) = span.color {
        let mut ink = tint(value, look.dark());
        ink.a = if span.dim { 0.55 } else { 1. };
        return ink;
    }
    let alpha = match (selected, span.dim) {
        (true, false) => 0.85,
        (true, true) => 0.60,
        (false, false) => 0.62,
        (false, true) => 0.38,
    };
    color(
        if selected {
            look.colors.selection_foreground
        } else {
            look.colors.foreground
        },
        alpha,
    )
}

fn emoji(grapheme: &str) -> bool {
    if grapheme.contains('\u{fe0e}') {
        return false;
    }
    grapheme.chars().next().is_some_and(|first| {
        first.is_emoji_char()
            && (grapheme.contains('\u{fe0f}')
                || matches!(
                    first.emoji_status(),
                    EmojiStatus::EmojiPresentation
                        | EmojiStatus::EmojiPresentationAndModifierBase
                        | EmojiStatus::EmojiPresentationAndEmojiComponent
                        | EmojiStatus::EmojiPresentationAndModifierAndEmojiComponent
                ))
    })
}

#[derive(Clone)]
struct Piece {
    text: SharedString,
    run: TextRun,
    size: Pixels,
    lift: Pixels,
}

fn pieces(spans: &[SessionInfoSpan], look: Look, selected: bool, window: &Window) -> Vec<Piece> {
    let size = px(font_size(look));
    let mut pieces = Vec::new();
    for span in spans {
        let mut run = window.text_style().to_run(0);
        run.color = ink(span, look, selected);
        run.font.weight = if span.bold {
            FontWeight::SEMIBOLD
        } else {
            FontWeight::NORMAL
        };
        run.font.style = if span.italic {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        };
        if span.code {
            run.font.family = "monospace".into();
        }
        let font_id = window.text_system().resolve_font(&run.font);
        let cap = window.text_system().cap_height(font_id, size);
        for (text, small) in emoji_runs(&span.text) {
            let piece_size = if small { size * 0.7 } else { size };
            pieces.push(Piece {
                run: TextRun {
                    len: text.len(),
                    ..run.clone()
                },
                text: text.into(),
                size: piece_size,
                lift: if small { cap / 2. - piece_size * 0.375 } else { px(0.) },
            });
        }
    }
    pieces
}

/// Accessible label and one clipped, tail-truncated shaped line, aligned with the name.
pub(super) fn element(
    spans: &[SessionInfoSpan],
    look: Look,
    selected: bool,
    available: f32,
    window: &Window,
) -> impl IntoElement {
    let text: SharedString = spans.iter().map(|span| span.text.as_str()).collect::<String>().into();
    let pieces = pieces(spans, look, selected, window);
    let height = px(line_height(look, window));
    let natural_width: Pixels = pieces
        .iter()
        .map(|piece| {
            window
                .text_system()
                .shape_line(piece.text.clone(), piece.size, std::slice::from_ref(&piece.run), None)
                .width
        })
        .sum();
    div()
        .id("session-info")
        .accessibility_id("session-info")
        .role(gpui_kit::Role::Label)
        .aria_value(text.clone())
        .debug_selector(|| "session-info".to_owned())
        .overflow_hidden()
        .w_full()
        .h(height)
        .child(
            canvas(
                move |bounds, window, _| fitted(&pieces, bounds.size.width, window),
                move |bounds, shaped, window, cx| {
                    let normal = window.text_system().resolve_font(&window.text_style().font());
                    let ascent = window.text_system().ascent(normal, px(font_size(look)));
                    let descent = window.text_system().descent(normal, px(font_size(look)));
                    let target = (height - ascent + descent) / 2. + ascent;
                    for (piece, line, x) in shaped {
                        let baseline = (height - line.ascent - line.descent) / 2. + line.ascent;
                        let origin: Point<Pixels> =
                            point(bounds.origin.x + x, bounds.origin.y + target - baseline - piece.lift);
                        let _ = line.paint(origin, height, TextAlign::Left, None, window, cx);
                    }
                },
            )
            .size_full(),
        )
        .when(natural_width > px(available), |element| {
            element
                .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(text.clone()).build(window, cx))
        })
}

fn emoji_runs(text: &str) -> Vec<(String, bool)> {
    let mut runs: Vec<(String, bool)> = Vec::new();
    for grapheme in text.graphemes(true) {
        let small = emoji(grapheme);
        if let Some(last) = runs.last_mut().filter(|last| last.1 == small) {
            last.0.push_str(grapheme);
        } else {
            runs.push((grapheme.to_owned(), small));
        }
    }
    runs
}

fn shape(piece: &Piece, text: SharedString, window: &Window) -> ShapedLine {
    let run = TextRun {
        len: text.len(),
        ..piece.run.clone()
    };
    window.text_system().shape_line(text, piece.size, &[run], None)
}

/// Fit whole style runs, trimming the final run only on grapheme boundaries.
fn fitted(pieces: &[Piece], width: Pixels, window: &Window) -> Vec<(Piece, ShapedLine, Pixels)> {
    let total: Pixels = pieces
        .iter()
        .map(|piece| shape(piece, piece.text.clone(), window).width)
        .sum();
    let mut x = px(0.);
    let mut result = Vec::new();
    for piece in pieces {
        let line = shape(piece, piece.text.clone(), window);
        let ellipsis = shape(piece, "…".into(), window);
        let reserve = if total > width { ellipsis.width } else { px(0.) };
        if x + line.width + reserve <= width {
            let start = x;
            x += line.width;
            result.push((piece.clone(), line, start));
            continue;
        }
        let mut text = String::new();
        for grapheme in piece.text.graphemes(true) {
            let candidate = format!("{text}{grapheme}…");
            if x + shape(piece, candidate.into(), window).width > width {
                break;
            }
            text.push_str(grapheme);
        }
        text.push('…');
        result.push((piece.clone(), shape(piece, text.into(), window), x));
        break;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use ngterm_core::model::TintColor;

    #[test]
    fn presentation_and_variation_selectors_gate_whole_emoji_graphemes() {
        assert_eq!(
            emoji_runs("a⚠⚠️✔︎👩🏽‍💻🇬🇧中"),
            vec![
                ("a⚠".to_owned(), false),
                ("⚠️".to_owned(), true),
                ("✔︎".to_owned(), false),
                ("👩🏽‍💻🇬🇧".to_owned(), true),
                ("中".to_owned(), false),
            ]
        );
        assert!(!emoji("😀︎"));
        assert!(!emoji("7"));
        assert!(emoji("7️⃣"));
    }

    #[test]
    fn explicit_colors_selection_and_dim_are_independent_of_row_tint() {
        let look = Look::default();
        for selected in [false, true] {
            for dim in [false, true] {
                let plain = SessionInfoSpan {
                    dim,
                    ..SessionInfoSpan::default()
                };
                let expected = match (selected, dim) {
                    (true, true) => 0.60,
                    (true, false) => 0.85,
                    (false, true) => 0.38,
                    _ => 0.62,
                };
                assert!((ink(&plain, look, selected).a - expected).abs() < f32::EPSILON);
                let colored = SessionInfoSpan {
                    color: Some(TintColor::Green),
                    ..plain
                };
                let mut expected = tint(TintColor::Green, look.dark());
                expected.a = if dim { 0.55 } else { 1. };
                assert_eq!(ink(&colored, look, selected), expected);
            }
        }
        assert!((font_size(look) - 11.5).abs() < f32::EPSILON);
        assert!((font_size(Look { font_size: 9., ..look }) - 9.).abs() < f32::EPSILON);
    }

    #[gpui_kit::test]
    fn shaped_info_keeps_combined_traits_and_emoji_size(cx: &mut gpui_kit::TestAppContext) {
        let (_, cx) = cx.add_window_view(|_, _| gpui_kit::EmptyView);
        cx.update(|window, _| {
            let spans = [SessionInfoSpan {
                text: "code👩🏽‍💻".to_owned(),
                bold: true,
                italic: true,
                code: true,
                dim: true,
                color: None,
            }];
            let runs = pieces(&spans, Look::default(), false, window);
            assert_eq!(runs.len(), 2);
            assert_eq!(runs[0].run.font.family.as_ref(), "monospace");
            assert_eq!(runs[0].run.font.weight, FontWeight::SEMIBOLD);
            assert_eq!(runs[0].run.font.style, FontStyle::Italic);
            assert!((runs[0].run.color.a - 0.38).abs() < f32::EPSILON);
            assert_eq!(runs[1].size, runs[0].size * 0.7);
            let metrics_height = line_height(Look::default(), window);
            assert!(metrics_height >= font_size(Look::default()));
            let fitted = fitted(&runs, px(25.), window);
            assert!(fitted.last().unwrap().1.text.ends_with('…'));
            assert!(fitted.iter().map(|(_, line, _)| f32::from(line.width)).sum::<f32>() <= 25.);
        });
    }
    struct InfoView;

    impl gpui_kit::Render for InfoView {
        fn render(&mut self, window: &mut Window, _: &mut gpui_kit::Context<Self>) -> impl IntoElement {
            div().w(px(40.)).child(element(
                &[SessionInfoSpan {
                    text: "a long session description".into(),
                    ..SessionInfoSpan::default()
                }],
                Look::default(),
                false,
                40.,
                window,
            ))
        }
    }

    #[gpui_kit::test]
    fn truncated_info_exposes_its_full_text_in_a_tooltip(cx: &mut gpui_kit::TestAppContext) {
        cx.update(gpui_kit::init);
        let (_, cx) = cx.add_window_view(|_, _| InfoView);
        cx.run_until_parked();
        cx.activate_a11y();
        let tree = cx.a11y_tree_update().unwrap();
        let info = tree
            .nodes
            .iter()
            .find(|(_, node)| node.author_id() == Some("session-info"))
            .unwrap();
        assert_eq!(info.1.value(), Some("a long session description"));
        let bounds = cx.debug_bounds("session-info").expect("the info is drawn");
        assert!(bounds.size.width <= px(40.));
        cx.simulate_mouse_move(bounds.center(), None, gpui_kit::Modifiers::default());
        cx.run_until_parked();
        cx.executor().advance_clock(std::time::Duration::from_secs(1));
        cx.run_until_parked();
        assert!(cx.update(|window, _| window.painted_quads().len()) > 0);
    }
}
