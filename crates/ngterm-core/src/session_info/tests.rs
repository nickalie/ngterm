use super::*;

fn info(source: &str) -> SessionInfo {
    SessionInfo::make(source, true, 0.0).unwrap()
}
fn span(text: &str) -> SessionInfoSpan {
    SessionInfoSpan {
        text: text.to_owned(),
        ..SessionInfoSpan::default()
    }
}

#[test]
fn plain_text_keeps_markdown_literal() {
    let raw = "~/dev/*scratch*/notes_v2.md **x** ^[a](color: \"red\")";
    assert_eq!(SessionInfo::make(raw, false, 0.0).unwrap().spans, vec![span(raw)]);
}

#[test]
fn inline_markdown_maps_to_traits() {
    assert_eq!(
        info("**3** agents *on* `rr`").spans,
        vec![
            SessionInfoSpan {
                bold: true,
                ..span("3")
            },
            span(" agents "),
            SessionInfoSpan {
                italic: true,
                ..span("on")
            },
            span(" "),
            SessionInfoSpan {
                code: true,
                ..span("rr")
            },
        ]
    );
}

#[test]
fn strikethrough_keeps_its_text_and_other_inline_traits() {
    for source in ["~done~ 3 agents", "~~done~~ 3 agents"] {
        assert_eq!(info(source).spans, vec![span("done 3 agents")]);
    }
    assert_eq!(
        info("**~~done~~ now** after").spans,
        vec![
            SessionInfoSpan {
                bold: true,
                ..span("done now")
            },
            span(" after"),
        ]
    );
    assert_eq!(
        info("~~^[done](color: 'green')~~").spans,
        vec![SessionInfoSpan {
            color: Some(TintColor::Green),
            ..span("done")
        }]
    );
    assert_eq!(
        info("`~~done~~`").spans,
        vec![SessionInfoSpan {
            code: true,
            ..span("~~done~~")
        }]
    );
    assert_eq!(
        SessionInfo::make("~~done~~", false, 0.0).unwrap().spans,
        vec![span("~~done~~")]
    );
}

#[test]
fn color_and_dim_attributes_map_to_spans() {
    assert_eq!(
        info("^[running](color: \"green\") ^[idle](dim: true) ^[x](color: 'Blue', dim: true)").spans,
        vec![
            SessionInfoSpan {
                color: Some(TintColor::Green),
                ..span("running")
            },
            span(" "),
            SessionInfoSpan {
                dim: true,
                ..span("idle")
            },
            span(" "),
            SessionInfoSpan {
                color: Some(TintColor::Blue),
                dim: true,
                ..span("x")
            },
        ]
    );
}

#[test]
fn bold_inside_color_block_combines_traits() {
    assert_eq!(
        info("^[**done** now](color: \"green\")").spans,
        vec![
            SessionInfoSpan {
                bold: true,
                color: Some(TintColor::Green),
                ..span("done")
            },
            SessionInfoSpan {
                color: Some(TintColor::Green),
                ..span(" now")
            },
        ]
    );
}

#[test]
fn links_keep_labels_only_and_unknown_attributes_are_unstyled() {
    assert_eq!(
        info("see [PR 861](https://example.com/861) now").spans,
        vec![span("see PR 861 now")]
    );
    assert_eq!(info("^[a](colour: \"red\") b").spans, vec![span("a b")]);
}

#[test]
fn link_metadata_never_interprets_attributes_but_visible_labels_do() {
    for source in [
        "[PR](https://example.com \"^[x](color: red)\")",
        "[PR](https://example.com '^[x](color: \"maroon\")')",
        "[PR](https://example.com (^[x]\\(dim: 42\\)))",
        "[PR](https://example.com/^[x](color:red))",
    ] {
        let parsed = SessionInfo::make(source, true, 0.0).unwrap_or_else(|error| panic!("{source}: {error}"));
        assert_eq!(parsed.spans, vec![span("PR")], "{source}");
    }
    assert_eq!(
        info("[^[**PR**](color: 'green')](https://example.com \"^[x](color: red)\")").spans,
        vec![SessionInfoSpan {
            bold: true,
            color: Some(TintColor::Green),
            ..span("PR")
        }]
    );
    assert_eq!(
        info("<https://example.com/^[x](color:red)>").spans,
        vec![span("https://example.com/^[x](color:red)")]
    );
    assert_eq!(
        info("[a `]` b](https://example.com \"^[x](dim: 42)\")").spans,
        vec![
            span("a "),
            SessionInfoSpan {
                code: true,
                ..span("]")
            },
            span(" b")
        ]
    );
    assert_eq!(
        SessionInfo::make("[^[PR](color: red)](https://example.com)", true, 0.0).unwrap_err(),
        INVALID_MARKDOWN
    );
    assert_eq!(
        SessionInfo::make("[PR](https://example.com) ^[x](color: red)", true, 0.0).unwrap_err(),
        INVALID_MARKDOWN
    );
}

#[test]
fn bad_colors_and_typed_attributes_are_rejected() {
    assert_eq!(
        SessionInfo::make("^[a](color: \"maroon\")", true, 0.0).unwrap_err(),
        "unknown info color: maroon (red|orange|yellow|green|blue|purple|gray|teal|pink|brown|indigo|mint)"
    );
    // markup keeps the palette names: a hex the row tints take is refused here
    assert_eq!(
        SessionInfo::make("^[a](color: \"#3a7bd5\")", true, 0.0).unwrap_err(),
        "unknown info color: #3a7bd5 (red|orange|yellow|green|blue|purple|gray|teal|pink|brown|indigo|mint)"
    );
    assert_eq!(
        info("^[ok](color: \"pink\")").spans,
        vec![SessionInfoSpan {
            color: Some(TintColor::Pink),
            ..span("ok")
        }]
    );
    for raw in [
        "^[a](color: 42)",
        "^[a](color: red)",
        "^[a](dim: \"true\")",
        "^[a](dim: 42)",
    ] {
        assert_eq!(SessionInfo::make(raw, true, 0.0).unwrap_err(), INVALID_MARKDOWN);
    }
    for color in TintColor::ALL {
        assert_eq!(
            info(&format!("^[x](color: '{}')", color.wire_name().to_uppercase())).spans[0].color,
            Some(color)
        );
    }
}

#[test]
fn make_retains_source_markdown_and_lifetime() {
    let value = SessionInfo::make("**3** ^[ok](color: \"green\")", true, 30.0).unwrap();
    assert_eq!(value.source, "**3** ^[ok](color: \"green\")");
    assert!(value.markdown);
    assert!((value.hide_after - 30.0).abs() < f64::EPSILON);
    assert_eq!(
        value.spans,
        vec![
            SessionInfoSpan {
                bold: true,
                ..span("3")
            },
            span(" "),
            SessionInfoSpan {
                color: Some(TintColor::Green),
                ..span("ok")
            }
        ]
    );
}

#[test]
fn make_caps_utf8_source_and_validates_lifetime_first() {
    for source in ["a".repeat(512), "é".repeat(256)] {
        assert!(SessionInfo::make(&source, false, 0.0).is_ok());
    }
    for source in ["a".repeat(513), "é".repeat(257)] {
        assert_eq!(
            SessionInfo::make(&source, true, -1.0).unwrap_err(),
            "info text is over 512 bytes"
        );
    }
    for seconds in [-1.0, 86401.0, f64::NAN, f64::INFINITY] {
        assert_eq!(
            SessionInfo::make("", true, seconds).unwrap_err(),
            "--hide-after must be 0...86400 seconds"
        );
    }
    for seconds in [0.0, 0.125, 86400.0] {
        assert!(SessionInfo::make("x", false, seconds).is_ok());
    }
}

#[test]
fn blank_multiline_and_control_text_are_rejected_after_decoding() {
    for source in ["", "   ", "&#32;&#32;", "^[ ](dim: true)"] {
        assert_eq!(
            SessionInfo::make(source, true, 0.0).unwrap_err(),
            "info text is empty",
            "{source}"
        );
    }
    for character in ['\n', '\r', '\u{2028}', '\u{2029}', '\u{85}'] {
        assert_eq!(
            SessionInfo::make(&format!("a{character}b"), false, 0.0).unwrap_err(),
            "info text must be one line"
        );
        assert_eq!(
            SessionInfo::make(&format!("a&#{};b", character as u32), true, 0.0).unwrap_err(),
            "info text must be one line"
        );
    }
    for source in ["x &#27;[31m y", "a\tb", "a\u{9b}31mb", "a&#155;b", "a&#0;b"] {
        assert_eq!(
            SessionInfo::make(source, true, 0.0).unwrap_err(),
            "info text contains a control character",
            "{source}"
        );
    }
}

#[test]
fn inline_only_syntax_preserves_blocks_whitespace_and_code_literals() {
    assert_eq!(info("  # title  ").spans, vec![span("  # title  ")]);
    assert_eq!(info("- item").spans, vec![span("- item")]);
    assert_eq!(
        info("`^[x](color: red) &#27;`").spans,
        vec![SessionInfoSpan {
            code: true,
            ..span("^[x](color: red) &#27;")
        }]
    );
}

#[test]
fn nested_attributes_combine_and_can_clear_dim() {
    assert_eq!(
        info("^[**a** ^[b](dim: false)](color: 'red', dim: true)").spans,
        vec![
            SessionInfoSpan {
                bold: true,
                color: Some(TintColor::Red),
                dim: true,
                ..span("a")
            },
            SessionInfoSpan {
                color: Some(TintColor::Red),
                dim: true,
                ..span(" ")
            },
            SessionInfoSpan {
                color: Some(TintColor::Red),
                ..span("b")
            },
        ]
    );
}

#[test]
fn markdown_line_breaks_cannot_disappear_into_block_boundaries() {
    for source in ["a\nb", "a\n\nb", "a\r\nb", "# heading\n\n- item"] {
        assert_eq!(
            SessionInfo::make(source, true, 0.0).unwrap_err(),
            "info text must be one line"
        );
    }
}
