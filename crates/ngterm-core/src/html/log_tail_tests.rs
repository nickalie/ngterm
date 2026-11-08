//! Upstream's `LogTailTests.swift`.

use super::*;

/// One case: its name, the bytes read, whether they start at the file's start, the line cap, and the expected text,
/// line count and clip (`None` for binary data).
type Case = (
    &'static str,
    &'static [u8],
    bool,
    usize,
    Option<(&'static str, usize, bool)>,
);

/// Upstream's `LogTailTests.cases`; `\xC3\xA9` is `é`.
const CASES: &[Case] = &[
    ("whole file", b"a\nb\n", true, 10, Some(("a\nb\n", 2, false))),
    (
        "partial first line dropped",
        b"xx\na\nb\n",
        false,
        10,
        Some(("a\nb\n", 2, false)),
    ),
    (
        "window starting on a line boundary keeps that line",
        b"\na\nb\n",
        false,
        10,
        Some(("a\nb\n", 2, false)),
    ),
    (
        "no LF in the window keeps the clipped line",
        b"tail of a line",
        false,
        10,
        Some(("tail of a line", 1, true)),
    ),
    (
        "line longer than the window ending in LF at its last byte",
        b"end of a long line\n",
        false,
        10,
        Some(("end of a long line\n", 1, true)),
    ),
    (
        "clipped line starts at a whole character",
        b"\xA9\xC3\xA9 line",
        false,
        10,
        Some(("\u{e9} line", 1, true)),
    ),
    (
        "trailing partial character left for the next read",
        b"a\nb\xC3",
        true,
        10,
        Some(("a\nb", 2, false)),
    ),
    (
        "trailing partial four-byte character",
        b"a\n\xF0\x9F\x98",
        true,
        10,
        Some(("a\n", 1, false)),
    ),
    (
        "trailing partial three-byte character",
        b"a\n\xE2\x82",
        true,
        10,
        Some(("a\n", 1, false)),
    ),
    ("trailing C0 never starts a character", b"a\xC0", true, 10, None),
    ("trailing F5 never starts a character", b"a\xF5\x80", true, 10, None),
    ("trailing E0 80 is already invalid", b"a\xE0\x80", true, 10, None),
    ("trailing ED A0 is a surrogate", b"a\xED\xA0", true, 10, None),
    ("trailing F4 90 is past the last scalar", b"a\xF4\x90", true, 10, None),
    (
        "final line without LF counts",
        b"a\nb",
        true,
        10,
        Some(("a\nb", 2, false)),
    ),
    (
        "only the last maxLines lines",
        b"1\n2\n3\n4\n",
        true,
        2,
        Some(("3\n4\n", 2, false)),
    ),
    (
        "CRLF counts the same lines and keeps the CR",
        b"a\r\nb\r\n",
        true,
        10,
        Some(("a\r\nb\r\n", 2, false)),
    ),
    ("empty data", b"", false, 10, Some(("", 0, false))),
    (
        "NUL in a dropped line is not binary",
        b"a\0b\nc\nd\n",
        true,
        2,
        Some(("c\nd\n", 2, false)),
    ),
    (
        "truncation hole ahead of the first line is dropped",
        b"\0\0\0new\n",
        true,
        10,
        Some(("new\n", 1, false)),
    ),
    ("a window of nothing but NUL stays binary", b"\0\0\0\0", true, 10, None),
    (
        "a hole followed only by more NUL after a line feed stays binary",
        b"\0\0\n\0",
        true,
        10,
        None,
    ),
    (
        "a NUL inside a kept line stays binary",
        b"ok\nbad\0line\n",
        true,
        10,
        None,
    ),
    ("NUL marks binary", b"a\n\0b\n", true, 10, None),
    ("invalid UTF-8 marks binary", b"a\n\xFF\xFEb\n", true, 10, None),
    ("a zero cap keeps nothing", b"a\nb\n", true, 0, Some(("", 0, false))),
];

/// One offset case: its name, the bytes read, the file offset they start at, the line cap and the expected start.
type OffsetCase = (&'static str, &'static [u8], u64, usize, u64);

/// Upstream's `LogTailTests.offsetCases`.
const OFFSET_CASES: &[OffsetCase] = &[
    ("whole file starts at zero", b"a\nb\n", 0, 10, 0),
    ("the partial first line is past the start", b"xx\na\nb\n", 40, 10, 43),
    (
        "a window on a line boundary starts after the byte before it",
        b"\na\n",
        40,
        10,
        41,
    ),
    ("the line cap moves the start", b"1\n2\n3\n", 0, 1, 4),
    (
        "a clipped line starts at its first whole character",
        b"\xA9\xC3\xA9 line",
        40,
        10,
        41,
    ),
    (
        "a trimmed unfinished character leaves the start alone",
        b"xx\na\nb\xC3",
        40,
        10,
        43,
    ),
    ("a dropped NUL hole moves the start", b"\0\0new\n", 0, 10, 2),
];

/// Upstream's `snapshotKeepsTheLastWholeLines`.
#[test]
fn snapshot_keeps_the_last_whole_lines() {
    for (name, bytes, from_file_start, max_lines, expected) in CASES {
        let shown = snapshot(bytes, if *from_file_start { 0 } else { 100 }, *max_lines)
            .map(|snapshot| (snapshot.text, snapshot.lines, snapshot.clipped));
        let expected = expected.map(|(text, lines, clipped)| (text.to_owned(), lines, clipped));
        assert_eq!(shown, expected, "{name}");
    }
}

/// Upstream's `startOffsetIsTheFilePositionOfTheFirstShownByte`.
#[test]
fn start_offset_is_the_file_position_of_the_first_shown_byte() {
    for (name, bytes, offset, max_lines, start) in OFFSET_CASES {
        let snapshot = snapshot(bytes, *offset, *max_lines).unwrap();
        assert_eq!(snapshot.start_offset, *start, "{name}");
    }
}
