//! Ghostty `c/terminal.zig`'s twelve write-until cases and stream split/exit equivalence.

use ngterm_ghostty_vt::{
    Boundaries as B, Effects, Error, Format, FormatExtras, FormatOptions, Region, StringEnd as E,
    StringIntroducer as I, StringKind as K, Terminal, WriteStop,
};
use ngterm_ghostty_vt_sys as ffi;
use std::sync::{Arc, Mutex};

#[derive(Default, Debug, PartialEq, Eq)]
struct Events {
    replies: Vec<u8>,
    titles: Vec<String>,
    bells: usize,
}
struct Capture(Arc<Mutex<Events>>);
impl Effects for Capture {
    fn write_pty(&mut self, bytes: &[u8]) {
        self.0.lock().unwrap().replies.extend_from_slice(bytes);
    }
    fn title_changed(&mut self, title: &str) {
        self.0.lock().unwrap().titles.push(title.to_owned());
    }
    fn bell(&mut self) {
        self.0.lock().unwrap().bells += 1;
    }
}
fn terminal() -> (Terminal, Arc<Mutex<Events>>) {
    let mut t = Terminal::new(20, 3).unwrap();
    t.set_continuation_limit(10_000).unwrap();
    t.set_kitty_image_storage_limit(10_000).unwrap();
    let events = Arc::new(Mutex::new(Events::default()));
    t.set_effects(Box::new(Capture(events.clone()))).unwrap();
    (t, events)
}
fn stopped(t: &mut Terminal, input: &[u8], want: B, consumed: usize, reached: B) -> WriteStop {
    let stop = t.vt_write_until(input, want).unwrap();
    assert_eq!((stop.consumed, stop.reached), (consumed, reached), "{input:?}");
    stop
}

#[test]
fn ground_stops_at_the_first_ground_byte_and_distinguishes_exhaustion() {
    let (mut t, _) = terminal();
    let stop = stopped(&mut t, b"untouched", B::GROUND | B::APC_END, 0, B::GROUND);
    assert_eq!(
        (stop.kind, stop.introducer, stop.end, stop.byte),
        (None, None, None, None)
    );
    t.vt_write(b"\x1b[31");
    stopped(&mut t, b"mABC\x1b[", B::GROUND, 1, B::GROUND);
    assert_eq!(t.cursor(), (0, 0));
    t.vt_write(b"ABC\x1b[");
    stopped(&mut t, b"123", B::GROUND, 3, B::empty());
    assert!(!t.at_ground());
    stopped(&mut t, b"m", B::GROUND, 1, B::GROUND);
    assert!(t.at_ground());
}

#[test]
fn ground_handles_utf8_malformed_continuations_and_csi_cancellation() {
    let (mut t, _) = terminal();
    for (prefix, rest) in [
        (b"\xf0".as_slice(), b"\x9f\x98\x84".as_slice()),
        (b"\xe0\xa0", b"A"),
        (b"\x1b[123", b"\x18"),
    ] {
        t.vt_write(prefix);
        assert!(!t.at_ground());
        stopped(&mut t, rest, B::GROUND, rest.len(), B::GROUND);
        assert!(t.at_ground());
    }
}

#[test]
fn callbacks_run_only_for_consumed_input() {
    let (mut t, events) = terminal();
    t.vt_write(b"\x1b[31");
    stopped(&mut t, b"m\x07", B::GROUND, 1, B::GROUND);
    assert_eq!(events.lock().unwrap().bells, 0);
    t.vt_write(b"\x07");
    assert_eq!(events.lock().unwrap().bells, 1);
}

#[test]
fn result_contract_resets_fields_and_rejects_invalid_arguments_without_consuming() {
    {
        let mut t = std::ptr::null_mut();
        assert_eq!(
            // SAFETY: this contract probe passes valid storage or the deliberately invalid argument under test.
            unsafe { ffi::ghostty_terminal_new(std::ptr::null(), &raw mut t, 20, 3) },
            ffi::GHOSTTY_SUCCESS
        );
        let stale = ffi::GhosttyTerminalWriteStop {
            size: size_of::<ffi::GhosttyTerminalWriteStop>(),
            consumed: 99,
            reached: 99,
            kind: ffi::GHOSTTY_TERMINAL_STRING_KIND_OSC,
            introducer: ffi::GHOSTTY_TERMINAL_STRING_INTRODUCER_C1,
            end: ffi::GHOSTTY_TERMINAL_STRING_END_ABORTED,
            byte: 99,
        };
        let mut stop;
        let ground = B::GROUND.bits();
        for (handle, data, len, bits) in [
            (std::ptr::null_mut(), b"x".as_ptr(), 1, ground),
            (t, std::ptr::null(), 1, ground),
            (t, b"x".as_ptr(), 1, 0),
            (t, b"x".as_ptr(), 1, 1 << 7),
            (t, b"x".as_ptr(), 1, 1 << 31),
            (t, b"x".as_ptr(), 1, ground | (1 << 7)),
        ] {
            stop = stale;
            assert_eq!(
                // SAFETY: this contract probe passes valid storage or the deliberately invalid argument under test.
                unsafe { ffi::ghostty_terminal_vt_write_until(handle, data, len, bits, &raw mut stop) },
                ffi::GHOSTTY_INVALID_VALUE
            );
            assert_eq!(
                (
                    stop.consumed,
                    stop.reached,
                    stop.kind,
                    stop.introducer,
                    stop.end,
                    stop.byte
                ),
                (0, 0, 0, 0, 0, 0)
            );
        }
        assert_eq!(
            // SAFETY: this contract probe passes valid storage or the deliberately invalid argument under test.
            unsafe { ffi::ghostty_terminal_vt_write_until(t, b"x".as_ptr(), 1, ground, std::ptr::null_mut()) },
            ffi::GHOSTTY_INVALID_VALUE
        );
        stop = stale;
        stop.size -= 1;
        assert_eq!(
            // SAFETY: this contract probe passes valid storage or the deliberately invalid argument under test.
            unsafe { ffi::ghostty_terminal_vt_write_until(t, b"x".as_ptr(), 1, ground, &raw mut stop) },
            ffi::GHOSTTY_INVALID_VALUE
        );
        assert_eq!(stop.consumed, 99);
        stop = stale;
        assert_eq!(
            // SAFETY: this contract probe passes valid storage or the deliberately invalid argument under test.
            unsafe { ffi::ghostty_terminal_vt_write_until(t, std::ptr::null(), 0, B::APC_START.bits(), &raw mut stop) },
            ffi::GHOSTTY_NO_VALUE
        );
        assert_eq!((stop.consumed, stop.reached, stop.byte), (0, 0, 0));
        // SAFETY: this contract probe passes valid storage or the deliberately invalid argument under test.
        unsafe { ffi::ghostty_terminal_free(t) };
    }
    let (mut t, _) = terminal();
    assert_eq!(t.vt_write_until(b"x", B::empty()), Err(Error::InvalidValue));
    assert_eq!(t.cursor(), (0, 0));
    stopped(&mut t, b"hello", B::APC_START | B::APC_END, 5, B::empty());
}

#[test]
fn apc_dcs_and_osc_starts_consume_the_introducer_and_esc_end_leaves_backslash() {
    for (intro, start, end, kind) in [
        (b'_', B::APC_START, B::APC_END, K::Apc),
        (b'X', B::APC_START, B::APC_END, K::Apc),
        (b'^', B::APC_START, B::APC_END, K::Apc),
        (b'P', B::DCS_START, B::DCS_END, K::Dcs),
        (b']', B::OSC_START, B::OSC_END, K::Osc),
    ] {
        let (mut t, _) = terminal();
        let input = [b"\x1b".as_slice(), &[intro], b"1;2|data\x1b\\"].concat();
        let stop = stopped(&mut t, &input, start | end, 2, start);
        assert_eq!(
            (stop.kind, stop.introducer, stop.end, stop.byte),
            (Some(kind), Some(I::Esc), None, Some(intro))
        );
        let stop = stopped(&mut t, &input[2..], start | end, input.len() - 3, end);
        assert_eq!(
            (stop.kind, stop.end, stop.byte),
            (Some(kind), Some(E::StEsc), Some(0x1b))
        );
        assert!(!t.at_ground());
        stopped(&mut t, b"\\", B::GROUND | start | end, 1, B::GROUND);
        assert!(t.at_ground());
    }
}

#[test]
fn split_and_c1_introducers_only_stop_where_recognized() {
    for (final_byte, c1, flag) in [
        (b'_', 0x9f, B::APC_START),
        (b'X', 0x98, B::APC_START),
        (b'^', 0x9e, B::APC_START),
        (b'P', 0x90, B::DCS_START),
        (b']', 0x9d, B::OSC_START),
    ] {
        let (mut t, _) = terminal();
        stopped(&mut t, b"\x1b", flag, 1, B::empty());
        let stop = stopped(&mut t, &[final_byte, b'a'], flag, 1, flag);
        assert_eq!(stop.introducer, Some(I::Esc));
        let (mut t, _) = terminal();
        stopped(&mut t, &[c1], flag, 1, B::empty());
        t.vt_write(b"\x1b[1");
        let stop = stopped(&mut t, &[c1, b'a'], flag, 1, flag);
        assert_eq!(stop.introducer, Some(I::C1));
    }
}

#[test]
fn string_ends_report_ground_and_only_requested_flags() {
    for (byte, end) in [
        (0x18, E::Cancelled),
        (0x1a, E::Cancelled),
        (0x9c, E::StC1),
        (0x9b, E::Aborted),
    ] {
        let (mut t, _) = terminal();
        t.vt_write(b"\x1b_Gab");
        let ground = if byte == 0x9b { B::empty() } else { B::GROUND };
        let stop = stopped(&mut t, &[byte, b'x'], B::GROUND | B::APC_END, 1, ground | B::APC_END);
        assert_eq!((stop.kind, stop.end, stop.byte), (Some(K::Apc), Some(end), Some(byte)));
    }
    let (mut t, _) = terminal();
    stopped(&mut t, b"\x1b]2;title\x07x", B::OSC_START | B::OSC_END, 2, B::OSC_START);
    let stop = stopped(
        &mut t,
        b"2;title\x07x",
        B::GROUND | B::OSC_END,
        8,
        B::GROUND | B::OSC_END,
    );
    assert_eq!((stop.kind, stop.end, stop.byte), (Some(K::Osc), Some(E::Bel), Some(7)));
}

#[test]
fn an_aborting_byte_can_start_a_different_string_and_dcs_header_restart_is_not_a_boundary() {
    let (mut t, _) = terminal();
    t.vt_write(b"\x1b_Ga");
    let stop = stopped(
        &mut t,
        b"\x9d0;x\x07",
        B::APC_END | B::OSC_START | B::OSC_END,
        1,
        B::APC_END | B::OSC_START,
    );
    assert_eq!(
        (stop.kind, stop.introducer, stop.end, stop.byte),
        (Some(K::Apc), Some(I::C1), Some(E::Aborted), Some(0x9d))
    );
    stopped(&mut t, b"0;x\x07", B::OSC_END, 4, B::OSC_END);
    t.vt_write(b"\x1bP1;");
    stopped(&mut t, b"\x902|data", B::DCS_START | B::DCS_END, 7, B::empty());
    stopped(&mut t, b"\x9c", B::DCS_END, 1, B::empty()); // content after the DCS header
    stopped(&mut t, b"\x1b\\", B::DCS_END, 1, B::DCS_END);
}

fn through(t: &mut Terminal, mut bytes: &[u8]) {
    let want = B::all() - B::GROUND;
    while !bytes.is_empty() {
        let stop = t.vt_write_until(bytes, want).unwrap();
        assert!(stop.consumed > 0 && stop.consumed <= bytes.len());
        assert!((stop.reached - want).is_empty());
        bytes = &bytes[stop.consumed..];
    }
}
fn formatted(t: &mut Terminal) -> Vec<u8> {
    t.format(
        Region::Screen,
        FormatOptions {
            format: Format::Vt,
            extras: FormatExtras::RESTORE,
            ..FormatOptions::default()
        },
    )
    .unwrap()
    .unwrap()
}
fn equivalent_at_every_split(stream: &[u8]) {
    let (mut plain, expected) = terminal();
    plain.vt_write(stream);
    let screen = formatted(&mut plain);
    let continuation = plain.continuation().unwrap();
    for split in 0..=stream.len() {
        let (mut bounded, events) = terminal();
        through(&mut bounded, &stream[..split]);
        through(&mut bounded, &stream[split..]);
        assert_eq!(formatted(&mut bounded), screen, "split {split}, stream {stream:?}");
        assert_eq!(bounded.continuation().unwrap(), continuation, "split {split}");
        assert_eq!(bounded.at_ground(), plain.at_ground());
        assert_eq!(*events.lock().unwrap(), *expected.lock().unwrap(), "split {split}");
    }
}

#[test]
fn every_exit_byte_at_every_split_matches_plain_write() {
    // APC, OSC, DCS header, passthrough and ignore: the same byte has different meanings in each state.
    for prefix in [
        b"\x1b_Ga=q,i=7;AAAA".as_slice(),
        b"\x1b]2;title",
        b"\x1bP1;2",
        b"\x1bP1;2|data",
        b"\x1bP:data",
    ] {
        for byte in 0..=255 {
            equivalent_at_every_split(&[prefix, &[byte], b"\x1b\\Z\x1b[c"].concat());
        }
    }
}

#[test]
fn a_payload_over_the_simd_chunk_size_matches_plain_write_at_every_split() {
    let mut input = b"\x1b]2;".to_vec();
    input.extend([b'x'; 4100]);
    input.extend_from_slice(b"\x07\x1b_Ga=q,i=7;AAAA\x1b\\\x1b[c");
    equivalent_at_every_split(&input);
}
