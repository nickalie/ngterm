use std::borrow::Cow;

use super::stripping_escapes;

/// Upstream's `strippingEscapesDropsEverySequenceAndKeepsTheText`.
const CASES: &[(&str, &str)] = &[
    ("plain\ttext\n", "plain\ttext\n"),
    ("\u{1b}[1;38;5;208mbold\u{1b}[22m tail", "bold tail"),
    ("\u{1b}]8;;http://x\u{1b}\\link\u{1b}]8;;\u{07}", "link"),
    ("a\u{1b}(Bb\u{1b}=c", "abc"),
    ("end\u{1b}[38;2;1", "end"),
    ("end\u{1b}", "end"),
    ("a\u{1b}P$qm\u{1b}\\b", "ab"),
    ("a\u{1b}Ptmux;\u{1b}\u{1b}[31mhidden\u{1b}\\b", "ab"),
    ("a\u{1b}Ptmux;\u{1b}\u{1b}]52;c;eA==\u{1b}\u{1b}\\\u{1b}\\b", "ab"),
    (
        "a\u{1b}Ptmux;\u{1b}\u{1b}]8;;http://x\u{1b}\u{1b}\\label\u{1b}\u{1b}]8;;\u{1b}\u{1b}\\\u{1b}\\b",
        "ab",
    ),
    ("a\u{1b}_Gf=100;AAAA\u{07}still\u{1b}\\b", "ab"),
    ("end\u{1b}_Gf=100;AAAA", "end"),
];

#[test]
fn stripping_escapes_drops_every_sequence_and_keeps_the_text() {
    for (input, expected) in CASES {
        assert_eq!(stripping_escapes(input), *expected, "{input:?}");
    }
}

#[test]
fn sos_and_pm_strings_run_to_their_string_terminator_too() {
    assert_eq!(stripping_escapes("a\u{1b}Xsos\u{07}x\u{1b}\\b"), "ab");
    assert_eq!(stripping_escapes("a\u{1b}^pm\u{1b}\\b"), "ab");
}

#[test]
fn an_intermediate_form_runs_over_every_intermediate_to_its_final_byte() {
    assert_eq!(stripping_escapes("a\u{1b}$(Bb\u{1b}#8c"), "abc");
}

#[test]
fn a_sequence_keeps_the_text_around_it_whole_characters_included() {
    assert_eq!(stripping_escapes("é\u{1b}[31mж\u{1b}(ü€"), "éж€");
    assert_eq!(stripping_escapes("a\u{1b}ñb"), "ab");
}

#[test]
fn text_with_no_escape_is_borrowed() {
    assert!(matches!(stripping_escapes("plain [31m"), Cow::Borrowed("plain [31m")));
}
