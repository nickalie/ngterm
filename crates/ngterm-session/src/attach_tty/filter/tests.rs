use super::*;

fn filtered(bytes: &[u8]) -> Vec<u8> {
    let mut filter = OutputFilter::new();
    let mut out = Vec::new();
    filter.feed(bytes, &mut out);
    out
}

/// `bytes` fed in two reads cut at every byte, and one byte at a time: the same output as whole.
fn assert_split_safe(bytes: &[u8], expected: &[u8]) {
    assert_eq!(filtered(bytes), expected, "whole: {:?}", String::from_utf8_lossy(bytes));
    for cut in 1..bytes.len() {
        let mut filter = OutputFilter::new();
        let mut out = Vec::new();
        filter.feed(&bytes[..cut], &mut out);
        filter.feed(&bytes[cut..], &mut out);
        assert_eq!(out, expected, "cut at {cut}: {:?}", String::from_utf8_lossy(bytes));
    }
    let mut filter = OutputFilter::new();
    let mut out = Vec::new();
    for byte in bytes {
        filter.feed(std::slice::from_ref(byte), &mut out);
    }
    assert_eq!(out, expected, "byte by byte");
}

/// Every read boundary agrees with the real parser about where attach may insert its own sequences.
fn assert_ground_matches_terminal(bytes: &[u8], expected: &[u8]) {
    assert_split_safe(bytes, expected);
    for cut in 0..=bytes.len() {
        let mut filter = OutputFilter::new();
        let mut terminal = ngterm_ghostty_vt::Terminal::new(20, 4).unwrap();
        let mut out = Vec::new();
        filter.feed(&bytes[..cut], &mut out);
        terminal.vt_write(&bytes[..cut]);
        assert_eq!(filter.at_ground(), terminal.at_ground(), "prefix {cut}: {bytes:?}");
        let taken = if filter.at_ground() {
            0
        } else {
            let taken = filter.feed_until_ground(&bytes[cut..], &mut out);
            let expected = terminal
                .vt_write_until_ground(&bytes[cut..])
                .unwrap_or(bytes.len() - cut);
            assert_eq!(taken, expected, "ground after prefix {cut}: {bytes:?}");
            assert_eq!(
                filter.at_ground(),
                terminal.at_ground(),
                "after {taken} more bytes: {bytes:?}"
            );
            taken
        };
        filter.feed(&bytes[cut + taken..], &mut out);
        assert_eq!(out, expected, "output after prefix {cut}: {bytes:?}");
    }
}

#[test]
fn high_bytes_stay_inside_an_apc_until_its_real_terminator() {
    for byte in 0xA0..=0xFF {
        for end in [b"\x1b\\".as_slice(), b"\x9c"] {
            let mut stream = b"\x1b_G".to_vec();
            stream.push(byte);
            stream.extend_from_slice(b"payload");
            // The unterminated prefix must never offer an insertion point.
            assert_ground_matches_terminal(&stream, &stream);
            stream.extend_from_slice(end);
            stream.extend_from_slice(b"tail");
            assert_ground_matches_terminal(&stream, &stream);
        }
    }
}

#[test]
fn every_apc_exit_matches_the_real_terminal() {
    for byte in 0..=u8::MAX {
        let mut stream = b"\x1b_G".to_vec();
        stream.push(byte);
        // Includes C1 self-introducers, CAN/SUB, ST, ESC and C1 aborts into CSI/OSC/DCS.
        stream.extend_from_slice(b"payload\x1b\\tail");
        assert_ground_matches_terminal(&stream, &stream);
    }
}

#[test]
fn apc_c1_aborts_still_filter_the_query_they_introduce() {
    for (query, kept) in [
        (b"\x9b6n".as_slice(), b"\x1b_G\xff\x1b\\tail".as_slice()),
        (b"\x9d11;?\x07", b"\x1b_G\xff\x1b\\tail"),
        (b"\x90$qm\x1b\\", b"\x1b_G\xff\x1b\\tail"),
    ] {
        let mut stream = b"\x1b_G\xff".to_vec();
        stream.extend_from_slice(query);
        stream.extend_from_slice(b"tail");
        assert_ground_matches_terminal(&stream, kept);
    }
}

/// The outer terminal sees only filtered bytes; a suppressed introducer must still end the forwarded string.
fn assert_forwarded_string_ends(stream: &[u8], expected: &[u8]) {
    assert_ground_matches_terminal(stream, expected);
    for cut in 0..=stream.len() {
        let mut filter = OutputFilter::new();
        let mut terminal = ngterm_ghostty_vt::Terminal::new(20, 4).unwrap();
        let mut out = Vec::new();
        filter.feed(&stream[..cut], &mut out);
        terminal.vt_write(&out);
        out.clear();
        let taken = filter.feed_until_ground(&stream[cut..], &mut out);
        terminal.vt_write(&out);
        assert!(filter.at_ground(), "filter at cut {cut}: {stream:?}");
        assert!(terminal.at_ground(), "outer terminal at cut {cut}: {stream:?}");
        out.clear();
        filter.feed(&stream[cut + taken..], &mut out);
        filter.feed(b"later", &mut out);
        terminal.vt_write(&out);
        let text: String = terminal
            .active_row(0)
            .unwrap()
            .cells
            .iter()
            .map(|cell| cell.text.as_str())
            .collect();
        assert_eq!(text, "taillater", "visible output at cut {cut}: {stream:?}");
    }
}

#[test]
fn suppressing_a_query_preserves_the_exit_from_a_forwarded_string() {
    for start in [b"\x1b_G".as_slice(), b"\x1b_G\xff", b"\x1bXpayload", b"\x1b^payload"] {
        for query in [
            b"\x9b6n".as_slice(),
            b"\x9d11;?\x07",
            b"\x9d11;?\x1b\\",
            b"\x90$qm\x1b\\",
            b"\x1b[6n",
            b"\x1b]11;?\x07",
            b"\x1b]11;?\x1b\\",
            b"\x1bP$qm\x1b\\",
            b"\x1bZ",
            b"\x1b[?2031h",
        ] {
            let stream = [start, query, b"tail"].concat();
            let expected = [start, b"\x1b\\", b"tail"].concat();
            assert_forwarded_string_ends(&stream, &expected);
        }
    }
    // DCS payloads override C1 transitions, but ESC ends a forwarded DCS too.
    for query in [b"\x1b[6n".as_slice(), b"\x1b]11;?\x07", b"\x1bP$qm\x1b\\", b"\x1bZ"] {
        let stream = [b"\x1bPqpayload".as_slice(), query, b"tail"].concat();
        assert_forwarded_string_ends(&stream, b"\x1bPqpayload\x1b\\tail");
    }
}

#[test]
fn a_forwarded_string_exit_is_not_repeated_for_later_queries() {
    for end in [
        b"\x1b[31m".as_slice(),
        b"\x9d2;title\x07",
        b"\x1bPqpayload\x1b\\",
        b"\x1bP:ignored\x1b\\",
        b"\x1b[1\x18",
        b"\x1b]2;abandoned\x18",
    ] {
        let stream = [b"\x1b_G".as_slice(), end, b"\x1b[6ntail"].concat();
        let expected = [b"\x1b_G".as_slice(), end, b"tail"].concat();
        assert_split_safe(&stream, &expected);
    }
    let mut filter = OutputFilter::new();
    let mut out = Vec::new();
    filter.feed(b"\x1b_G\x1b[", &mut out);
    filter.cancel(&mut out);
    filter.feed(b"\x1b[6ntail", &mut out);
    assert_eq!(out, b"\x1b_G\x1b[\x18tail");
}

#[test]
fn rewriting_a_query_preserves_the_exit_from_a_forwarded_string() {
    for (query, rewritten) in [
        (b"\x9b6\x07n".as_slice(), b"\x07".as_slice()),
        (b"\x1b[?2031;25h", b"\x1b[?25h"),
    ] {
        let stream = [b"\x1b_G\xff".as_slice(), query, b"tail"].concat();
        let expected = [b"\x1b_G\xff\x1b\\".as_slice(), rewritten, b"tail"].concat();
        assert_forwarded_string_ends(&stream, &expected);
    }
    // An OSC rewrite keeps its original introducer, which still terminates the APC itself.
    assert_forwarded_string_ends(
        b"\x1b_G\xff\x9d4;1;?;2;rgb:ff/00/00\x07tail",
        b"\x1b_G\xff\x9d4;2;rgb:ff/00/00\x07tail",
    );
}

#[test]
fn osc_and_dcs_high_bytes_stay_inside_their_strings() {
    for byte in 0xA0..=0xFF {
        for start in [b"\x1b]2;".as_slice(), b"\x1bPq"] {
            let mut stream = start.to_vec();
            stream.push(byte);
            stream.extend_from_slice(b"payload");
            // OSC is held for query classification until its terminator; DCS q passes as it comes.
            let expected = if start == b"\x1b]2;" { &[][..] } else { &stream };
            assert_ground_matches_terminal(&stream, expected);
            stream.extend_from_slice(b"\x1b\\tail");
            assert_ground_matches_terminal(&stream, &stream);
        }
    }
}

#[test]
fn the_queries_the_server_answers_are_dropped_wherever_the_reads_cut_them() {
    let queries: [&[u8]; 21] = [
        b"\x1b[c",
        b"\x1b[0c",
        b"\x1b[>c",
        b"\x1b[>0c",
        b"\x1b[=c",
        b"\x1b[5n",
        b"\x1b[6n",
        b"\x1b[?6n",
        b"\x1b[?996n",
        b"\x1b[?2026$p",
        b"\x1b[4$p",
        b"\x1b[>q",
        b"\x1b[?u",
        b"\x1b[14t",
        b"\x1b[16t",
        b"\x1b[18t",
        b"\x1b[21t",
        b"\x1bP$qm\x1b\\",
        b"\x1bP+q544e\x1b\\",
        b"\x1bZ",
        b"\x05",
    ];
    for query in queries {
        let mut stream = b"ab".to_vec();
        stream.extend_from_slice(query);
        stream.extend_from_slice(b"cd");
        assert_split_safe(&stream, b"abcd");
    }
}

#[test]
fn the_colour_and_clipboard_queries_are_dropped_with_either_terminator() {
    let queries: [&[u8]; 9] = [
        b"\x1b]10;?\x07",
        b"\x1b]11;?\x1b\\",
        b"\x1b]12;?\x07",
        b"\x1b]4;1;?\x07",
        b"\x1b]4;1;?;2;?\x1b\\",
        b"\x1b]21;foreground=?;background=?\x1b\\",
        b"\x1b]52;c;?\x07",
        b"\x1b]52;;?\x1b\\",
        b"\x1b]5522;type=read;dGV4dC9wbGFpbg==\x1b\\",
    ];
    for query in queries {
        let mut stream = b"ab".to_vec();
        stream.extend_from_slice(query);
        stream.extend_from_slice(b"cd");
        assert_split_safe(&stream, b"abcd");
    }
}

#[test]
fn everything_else_passes_unchanged() {
    let kept: [&[u8]; 16] = [
        b"plain text \xe2\x82\xac\r\n",
        b"\x1b[1;31mred\x1b[0m",
        b"\x1b[2J\x1b[H",
        b"\x1b[?1049h\x1b[?25l",
        b"\x1b[22;0t",
        b"\x1b[>1u\x1b[<u",
        b"\x1b[u",
        b"\x1b]0;title\x07",
        b"\x1b]2;title\x1b\\",
        b"\x1b]10;#ffffff\x07",
        b"\x1b]4;1;rgb:ff/00/00\x07",
        b"\x1b]52;c;aGVsbG8=\x07",
        b"\x1b]133;B\x07",
        b"\x1bPq#0;2;0;0;0#1~~@@\x1b\\",
        b"\x1b_Ga=T,f=100;AAAA\x1b\\",
        b"\x1b(B\x1b7\x1b8",
    ];
    for bytes in kept {
        assert_split_safe(bytes, bytes);
    }
}

#[test]
fn an_osc_4_keeps_its_settings_and_loses_its_queries() {
    assert_split_safe(b"\x1b]4;1;red;2;?;3;blue\x07", b"\x1b]4;1;red;3;blue\x07");
}

#[test]
fn decset_loses_the_modes_the_server_reports_and_win32_input() {
    assert_split_safe(b"\x1b[?2031h", b"");
    assert_split_safe(b"\x1b[?2048l", b"");
    assert_split_safe(b"\x1b[?1049;2031;2004h", b"\x1b[?1049;2004h");
    // what a Windows session's pseudoconsole asks of its terminal as it starts
    assert_split_safe(b"\x1b[?9001h\x1b[?1004h", b"\x1b[?1004h");
    assert_split_safe(b"\x1b[?9001;1004l", b"\x1b[?1004l");
    assert_split_safe(b"\x1b[?1000h", b"\x1b[?1000h");
}

#[test]
fn a_prompt_mark_gets_redraw_0() {
    assert_split_safe(b"\x1b]133;A\x07", b"\x1b]133;A;redraw=0\x07");
    assert_split_safe(b"\x1b]133;A\x1b\\", b"\x1b]133;A;redraw=0\x1b\\");
    assert_split_safe(
        b"\x1b]133;A;aid=14;cl=line\x07",
        b"\x1b]133;A;aid=14;cl=line;redraw=0\x07",
    );
    assert_split_safe(b"\x1b]133;A;redraw=1\x07", b"\x1b]133;A;redraw=0\x07");
    assert_split_safe(b"\x1b]133;A;redraw=0\x07", b"\x1b]133;A;redraw=0\x07");
}

#[test]
fn a_long_string_passes_as_it_comes() {
    let mut write = b"\x1b]52;c;".to_vec();
    write.extend(std::iter::repeat_n(b'A', STRING_LIMIT * 2));
    write.extend_from_slice(b"\x07");
    assert_eq!(filtered(&write), write);
    let mut filter = OutputFilter::new();
    let mut out = Vec::new();
    filter.feed(&write[..STRING_LIMIT + 100], &mut out);
    assert!(out.len() > STRING_LIMIT, "a long OSC is passed on before its end");
}

#[test]
fn a_c1_byte_in_ground_is_text_as_ghostty_decodes_it() {
    assert_split_safe(b"a\x9b>cb", b"a\x9b>cb");
}

#[test]
fn an_abandoned_sequence_passes_as_the_terminal_abandons_it() {
    assert_split_safe(b"\x1b[1\x18x", b"\x1b[1\x18x");
    assert_split_safe(b"\x1b]11;?\x18x", b"\x1b]11;?\x18x");
}

#[test]
fn the_ground_places_are_where_no_sequence_or_character_is_cut() {
    let stream = b"\x1b[1;31mA\x1b]0;title\x1b\\B\x1bPq#0\x1b\\C\xe2\x82\xacD";
    for cut in 1..stream.len() {
        let mut filter = OutputFilter::new();
        let mut out = Vec::new();
        filter.feed(&stream[..cut], &mut out);
        let at = out.len();
        let mut marked = out.clone();
        if filter.at_ground() {
            filter.feed(&stream[cut..], &mut marked);
        } else {
            let taken = filter.feed_until_ground(&stream[cut..], &mut marked);
            assert!(filter.at_ground());
            let mut fresh = OutputFilter::new();
            let mut ignored = Vec::new();
            fresh.feed(&stream[..cut + taken], &mut ignored);
            assert!(fresh.at_ground(), "cut {cut} + {taken}");
            filter.feed(&stream[cut + taken..], &mut marked);
        }
        assert_eq!(marked, stream, "cut at {cut} (output {at})");
    }
}

#[test]
fn cancel_ends_the_sequence_held() {
    let mut filter = OutputFilter::new();
    let mut out = Vec::new();
    filter.feed(b"\x1b[3", &mut out);
    assert!(!filter.at_ground());
    filter.cancel(&mut out);
    assert!(filter.at_ground());
    assert_eq!(out, b"\x1b[3\x18");
    filter.feed(b"1mx", &mut out);
    assert_eq!(out, b"\x1b[3\x181mx");
}

#[test]
fn a_dropped_string_takes_its_st_and_nothing_after() {
    assert_split_safe(b"\x1b]11;?\x1b[1m", b"\x1b[1m");
    assert_split_safe(b"\x1bP$qm\x1b[2m", b"\x1b[2m");
}

#[test]
fn a_query_with_controls_inside_is_dropped_and_its_controls_kept() {
    // Ghostty executes the controls as it parses the CSI, and still dispatches the query
    assert_split_safe(b"\x1b[6\x00n", b"\x00");
    assert_split_safe(b"a\x1b[?99\x086n", b"a\x08");
    assert_split_safe(b"\x1b[\r>0c", b"\r");
    assert_split_safe(b"\x1b[?20\x7f31;25h", b"\x1b[?25h");
    assert_split_safe(b"\x1b[?2031;\x0725h", b"\x07\x1b[?25h");
    // anything else keeps its controls in place
    assert_split_safe(b"\x1b[1\r;31m", b"\x1b[1\r;31m");
}

#[test]
fn a_reset_that_passes_is_followed_by_what_comes_after_it() {
    let mut filter = OutputFilter::new();
    filter.after_reset(b"\x1b[?2031h");
    let mut out = Vec::new();
    filter.feed(b"a\x1bcb\x1b\x1bc", &mut out);
    filter.feed(b"\x1b", &mut out);
    filter.feed(b"c", &mut out);
    assert_eq!(out, b"a\x1bc\x1b[?2031hb\x1b\x1bc\x1b[?2031h\x1bc\x1b[?2031h");
    assert!(filter.at_ground());
    // an escape with an intermediate is no RIS
    let mut out = Vec::new();
    filter.feed(b"\x1b#c", &mut out);
    assert_eq!(out, b"\x1b#c");
    assert_eq!(filtered(b"\x1bc"), b"\x1bc", "nothing after it unless asked");
}

#[test]
fn a_default_filter_is_a_new_one() {
    let mut filter = OutputFilter::default();
    assert!(filter.at_ground());
    let mut out = Vec::new();
    filter.feed(b"a\x1b[cb", &mut out);
    assert_eq!(out, b"ab");
}

#[test]
fn a_csi_past_its_limit_passes_whole_even_when_it_ends_as_a_query() {
    let mut long = b"\x1b[".to_vec();
    long.extend(b"1;".repeat(CSI_LIMIT));
    long.push(b'c');
    assert_split_safe(&long, &long);
}

#[test]
fn a_c1_introducer_inside_a_sequence_abandons_it_and_starts_its_own() {
    // the CSI held passes, unfinished, and the C1 CSI after it is classified on its own
    assert_split_safe(b"\x1b[1\x9b6nx", b"\x1b[1x");
    assert_split_safe(b"\x1b[1\x9b1mx", b"\x1b[1\x9b1mx");
    // a C1 DCS and a C1 OSC, queries both, dropped with what ends them
    assert_split_safe(b"\x1b[\x90$qm\x1b\\x", b"\x1b[x");
    assert_split_safe(b"\x1b[\x9d11;?\x07x", b"\x1b[x");
    // a C1 SOS passes as it comes, up to the C1 ST that ends it
    assert_split_safe(b"\x1b[1\x98ab\x9cx", b"\x1b[1\x98ab\x9cx");
    // an escape with an intermediate is abandoned the same way
    assert_split_safe(b"\x1b(\x18x", b"\x1b(\x18x");
    assert_split_safe(b"\x1b(\x9b5nx", b"\x1b(x");
}

#[test]
fn a_dcs_the_terminal_ignores_passes_unchanged() {
    for ignored in [b"\x1bP:x\x1b\\y".as_slice(), b"\x1bP1<x\x1b\\y", b"\x1bP$0x\x1b\\y"] {
        assert_split_safe(ignored, ignored);
    }
}

#[test]
fn a_dcs_header_ignores_del_and_is_abandoned_by_can() {
    // DEL inside the header is ignored: still DECRQSS, dropped
    assert_split_safe(b"\x1bP$\x7fqm\x1b\\z", b"z");
    // CAN abandons the header, and the string after it, unfinished, passes
    assert_split_safe(b"\x1bP1\x18z", b"\x1bP1\x18z");
    // CAN ends a DCS string that is no query, which passed as it came
    assert_split_safe(b"\x1bPqab\x18z", b"\x1bPqab\x18z");
}

#[test]
fn a_prompt_mark_of_another_kind_is_not_rewritten() {
    assert_split_safe(b"\x1b]133;AB\x07", b"\x1b]133;AB\x07");
}

#[test]
fn a_utf8_character_of_four_bytes_holds_the_ground_until_its_last_byte() {
    let emoji = "\u{1f600}".as_bytes();
    let mut filter = OutputFilter::new();
    let mut out = Vec::new();
    for byte in &emoji[..3] {
        filter.feed(std::slice::from_ref(byte), &mut out);
        assert!(!filter.at_ground());
    }
    filter.feed(&emoji[3..], &mut out);
    assert!(filter.at_ground());
    assert_eq!(out, emoji);
}

#[test]
fn feeding_until_ground_takes_everything_when_the_sequence_stays_open() {
    let mut filter = OutputFilter::new();
    let mut out = Vec::new();
    assert_eq!(filter.feed_until_ground(b"\x1b[1;3", &mut out), 5);
    assert!(!filter.at_ground());
    assert_eq!(out, b"", "an unfinished CSI is held");
    assert_eq!(filter.feed_until_ground(b"1mxyz", &mut out), 2);
    assert_eq!(out, b"\x1b[1;31m");
}

#[test]
fn delegated_clipboard_filter_drops_large_writes_and_preserves_apc_bytes_and_string_exits() {
    let large = [b"before\x1b]52;c;".as_slice(), &vec![b'A'; 8192], b"\x07after"].concat();
    for size in [1, 7, large.len()] {
        let mut filter = OutputFilter::new();
        let mut out = Vec::new();
        filter.delegated_clipboards(true, &mut out);
        for chunk in large.chunks(size) {
            filter.feed(chunk, &mut out);
        }
        assert_eq!(out, b"beforeafter");
    }
    for byte in 0xa0..=0xff {
        let input = [b"\x1b_Gpayload".as_slice(), &[byte], b"52;c;not-a-query\x07\x1b\\after"].concat();
        let mut filter = OutputFilter::new();
        let mut out = Vec::new();
        filter.delegated_clipboards(true, &mut out);
        filter.feed(&input, &mut out);
        assert_eq!(out, input, "high byte {byte:02x}");
    }
    let mut filter = OutputFilter::new();
    let mut out = Vec::new();
    filter.delegated_clipboards(true, &mut out);
    filter.feed(b"\x1b_Gpayload\x1b]5\x002;c;aGk=\x07after", &mut out);
    assert_eq!(out, b"\x1b_Gpayload\x1b\\after");
}

#[test]
fn ignored_osc_controls_cannot_exhaust_clipboard_prefix_classification() {
    for prefix in [b"52;c;".as_slice(), b"5522;type=write;", b"1337;Copy=:"] {
        let padded: Vec<_> = prefix
            .iter()
            .flat_map(|byte| [vec![0; STRING_LIMIT + 1], vec![*byte]].concat())
            .collect();
        let input = [b"before\x1b]".as_slice(), &padded, b"aGk=\x1b\\after"].concat();
        for size in [1, 127, input.len()] {
            let mut filter = OutputFilter::new();
            let mut out = Vec::new();
            filter.delegated_clipboards(true, &mut out);
            for chunk in input.chunks(size) {
                filter.feed(chunk, &mut out);
            }
            assert_eq!(out, b"beforeafter");
        }
    }
}

#[test]
fn live_filter_preserves_queries_modes_and_prompt_redraw() {
    let controls =
        b"\x05\x1bZ\x1b[6n\x1b[?2031;2048;9001h\x1b[?2031;2048;9001l\x1bP$qm\x1b\\\x1b]11;?\x07\x1b]133;A\x1b\\";
    let input = [controls.as_slice(), b"\x1b]52;c;aGk=\x1b\\after"].concat();
    for cut in 0..=input.len() {
        let mut filter = OutputFilter::clipboard_only();
        let mut out = Vec::new();
        filter.delegated_clipboards(true, &mut out);
        filter.feed(&input[..cut], &mut out);
        filter.feed(&input[cut..], &mut out);
        assert_eq!(out, [controls.as_slice(), b"after"].concat(), "cut {cut}");
    }
    let mut filter = OutputFilter::clipboard_only();
    let mut out = Vec::new();
    filter.feed(b"\x1b]52;c;aGk=\x07", &mut out);
    assert_eq!(out, b"\x1b]52;c;aGk=\x07", "snapshot recovery is retained");
}

fn snapshot_clipboard_continuation(snapshot: &[u8], suffix: &[u8]) {
    let mut filter = OutputFilter::new();
    let mut first = Vec::new();
    filter.feed(snapshot, &mut first);
    let mut terminal = ngterm_ghostty_vt::Terminal::new(40, 4).unwrap();
    terminal.vt_write(&first);
    let mut next = Vec::new();
    filter.delegated_clipboards(true, &mut next);
    filter.feed(suffix, &mut next);
    terminal.vt_write(&next);
    let text: String = terminal
        .active_row(0)
        .unwrap()
        .cells
        .iter()
        .map(|cell| cell.text.as_str())
        .collect();
    assert!(text.starts_with("beforeafter"), "{text:?}");
    assert!(terminal.at_ground());
}

#[test]
fn delegated_filter_follows_small_and_large_snapshot_clipboard_continuations() {
    snapshot_clipboard_continuation(b"before\x1b]5", b"2;c;aGk=\x07after");
    let snapshot = [b"before\x1b]52;c;".as_slice(), &vec![b'A'; STRING_LIMIT + 1]].concat();
    snapshot_clipboard_continuation(&snapshot, b"AAAA\x07after");
}

#[test]
fn snapshot_recovery_writes_pass_once_and_ordinary_delegated_writes_are_removed() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct Capture(Arc<AtomicUsize>);
    impl ngterm_ghostty_vt::Effects for Capture {
        fn clipboard_write(
            &mut self,
            _: &ngterm_ghostty_vt::ClipboardWriteRequest<'_>,
        ) -> ngterm_ghostty_vt::ClipboardWriteResult {
            self.0.fetch_add(1, Ordering::SeqCst);
            ngterm_ghostty_vt::ClipboardWriteResult::Success
        }
    }
    let writes = Arc::new(AtomicUsize::new(0));
    let mut terminal = ngterm_ghostty_vt::Terminal::new(40, 4).unwrap();
    terminal.set_effects(Box::new(Capture(writes.clone()))).unwrap();
    let mut filter = OutputFilter::new();
    let mut snapshot = Vec::new();
    filter.feed(b"\x1b]52;c;aGk=\x1b\\before", &mut snapshot);
    terminal.vt_write(&snapshot);
    assert_eq!(writes.load(Ordering::SeqCst), 1);
    let mut ordinary = Vec::new();
    filter.delegated_clipboards(true, &mut ordinary);
    filter.feed(b"\x1b]52;c;aGk=\x1b\\after", &mut ordinary);
    terminal.vt_write(&ordinary);
    assert_eq!(writes.load(Ordering::SeqCst), 1);
}

#[test]
fn a_c_api_viewer_dispatches_apc_before_its_barrier_without_a_second_reply_at_st() {
    use std::sync::{Arc, Mutex};
    struct Replies(Arc<Mutex<Vec<u8>>>);
    impl ngterm_ghostty_vt::Effects for Replies {
        fn write_pty(&mut self, bytes: &[u8]) {
            self.0.lock().unwrap().extend_from_slice(bytes);
        }
    }
    let replies = Arc::new(Mutex::new(Vec::new()));
    let mut terminal = ngterm_ghostty_vt::Terminal::new(40, 4).unwrap();
    terminal.set_kitty_image_storage_limit(1024).unwrap();
    terminal.set_effects(Box::new(Replies(replies.clone()))).unwrap();
    let mut filter = OutputFilter::new();
    let mut prefix = Vec::new();
    filter.delegated_clipboards(true, &mut prefix);
    filter.feed(b"\x1b_Ga=q,i=31,s=1,v=1,t=d,f=24;AAAA\x1b", &mut prefix);
    terminal.vt_write(&prefix);
    assert!(replies.lock().unwrap().is_empty());
    terminal.vt_write(filter.apc_dispatch_prefix());
    assert_eq!(*replies.lock().unwrap(), b"\x1b_Gi=31;OK\x1b\\");
    replies.lock().unwrap().clear();
    let mut suffix = Vec::new();
    filter.feed(b"\\after", &mut suffix);
    terminal.vt_write(&suffix);
    assert!(replies.lock().unwrap().is_empty());
    assert!(terminal.at_ground());
    let text: String = terminal
        .active_row(0)
        .unwrap()
        .cells
        .iter()
        .map(|cell| cell.text.as_str())
        .collect();
    assert!(text.starts_with("after"));
}

#[test]
fn a_programs_attach_bridge_envelope_never_leaves_the_filter() {
    let mut out = Vec::new();
    OutputFilter::new().feed(b"before\x1b]777;ngterm-\x01attach;Zm9yZ2Vk\x1b\\after", &mut out);
    assert!(
        !out.windows(b"ngterm-attach".len()).any(|part| part == b"ngterm-attach"),
        "{:?}",
        String::from_utf8_lossy(&out)
    );
    assert!(
        out.starts_with(b"before") && out.ends_with(b"after"),
        "{:?}",
        String::from_utf8_lossy(&out)
    );
}
