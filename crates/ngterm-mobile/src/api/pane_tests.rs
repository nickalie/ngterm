//! A [`Pane`] handle on a real stream terminal, as the apps call it through `UniFFI`: the test feeds the terminal what
//! the machine would send and reads what the handle's input sends back, with no core behind the handle.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use futures_lite::future::block_on;
use ngterm_term::{GridSize, Outgoing, StreamFeed, StreamOut};
use tokio::sync::mpsc;

use super::*;
use crate::chat::matchers::Agent;
use crate::chat::tests::matchers::{Capture, draft_drawn};

const DEADLINE: Duration = Duration::from_secs(10);
/// How long the drain of the handle's output waits for one more piece once it has the bytes it expected.
const PIECE: Duration = Duration::from_millis(50);

const SIZE: PaneSize = PaneSize {
    cols: 40,
    rows: 5,
    cell_width: 9,
    cell_height: 18,
};

/// A handle and the far ends of its terminal; `sized` counts the slot's wakes for a grid given with no terminal.
struct Shown {
    pane: Pane,
    feed: StreamFeed,
    out: StreamOut,
    sized: Arc<AtomicUsize>,
}

/// A slot with no terminal, its handle and the count of its wakes.
fn empty() -> (Pane, Arc<PaneSlot>, Arc<AtomicUsize>) {
    empty_at(SIZE)
}

fn empty_at(size: PaneSize) -> (Pane, Arc<PaneSlot>, Arc<AtomicUsize>) {
    let sized = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&sized);
    let slot = Arc::new(PaneSlot::new(
        size.into(),
        Arc::new(move || {
            counted.fetch_add(1, Ordering::SeqCst);
        }),
    ));
    let pane = Pane::new("B1".to_owned(), Arc::clone(&slot));
    (pane, slot, sized)
}

fn terminal(size: GridSize) -> (ngterm_term::Pane, StreamFeed, StreamOut) {
    ngterm_term::Pane::stream(&crate::pane::config(size, Arc::new(|| {}))).unwrap()
}

impl Shown {
    fn new() -> Self {
        Self::sized(SIZE)
    }

    fn sized(size: PaneSize) -> Self {
        let (pane, slot, sized) = empty_at(size);
        let (terminal, feed, out) = terminal(size.into());
        slot.install(Arc::new(terminal), size.into());
        Self { pane, feed, out, sized }
    }

    /// A handle whose terminal shows `agent`'s recorded screen at `state`, with `more` drawn over it.
    fn showing(agent: &str, state: &str, more: &[u8]) -> Self {
        let capture = Capture::load(&format!("{agent}-44x36"));
        let shown = Self::sized(PaneSize {
            cols: capture.cols,
            rows: capture.rows,
            ..SIZE
        });
        shown.feed(capture.at(state));
        shown.feed(more);
        shown.sent();
        shown
    }

    /// Feeds the machine's `bytes` and waits until the terminal took them: a cursor report asked for after them comes
    /// back. What the terminal answered, the report last, is taken, so what the handle sends next is all
    /// [`Self::sent`] sees.
    fn feed(&self, bytes: &[u8]) -> Vec<u8> {
        assert!(self.feed.deliver(bytes));
        assert!(self.feed.deliver(b"\x1b[6n"));
        let deadline = Instant::now() + DEADLINE;
        let mut sent = Vec::new();
        while !sent.ends_with(b"R") {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "no cursor report: {sent:?}");
            if let Ok(Some(Outgoing::Bytes(bytes))) = self.out.next(Some(left)) {
                sent.extend(bytes);
            }
        }
        sent
    }

    /// The bytes the handle sent the machine since the last call.
    fn sent(&self) -> Vec<u8> {
        let mut sent = Vec::new();
        while let Ok(Some(outgoing)) = self.out.next(Some(PIECE)) {
            if let Outgoing::Bytes(bytes) = outgoing {
                sent.extend(bytes);
            }
        }
        sent
    }
}

fn key(name: &str, mods: u8, text: &str) -> PaneKey {
    PaneKey {
        action: PaneKeyAction::Press,
        key: name.to_owned(),
        mods,
        consumed: 0,
        composing: false,
        text: text.to_owned(),
        unshifted: None,
    }
}

fn mouse(action: PaneMouseAction, button: Option<PaneMouseButton>) -> PaneMouse {
    // the middle of the cell at column 4, row 3 (1-based)
    PaneMouse {
        action,
        button,
        mods: 0,
        x: 9.0 * 3.0 + 4.0,
        y: 18.0 * 2.0 + 9.0,
    }
}

fn wheel(rows: i32) -> PaneWheel {
    PaneWheel {
        rows,
        columns: 0,
        mods: 0,
        x: 4.0,
        y: 9.0,
    }
}

fn at(x: u16, y: u32) -> CellPoint {
    CellPoint { x, y }
}

fn refused(reason: &str) -> PaneInputError {
    PaneInputError::Refused {
        reason: reason.to_owned(),
    }
}

#[test]
fn a_handle_with_no_terminal_refuses_input_as_closed() {
    let (pane, _slot, _sized) = empty();
    let closed = Err(PaneInputError::Closed);
    assert_eq!(pane.id(), "B1");
    assert_eq!(pane.key(key("key_a", 0, "a")), closed);
    assert_eq!(pane.type_text("a".to_owned()), closed);
    assert_eq!(
        pane.mouse(mouse(PaneMouseAction::Press, Some(PaneMouseButton::Left))),
        closed
    );
    assert_eq!(pane.wheel(wheel(1)), Err(PaneInputError::Closed));
    assert_eq!(pane.paste("a".to_owned(), false), Err(PaneInputError::Closed));
    assert_eq!(pane.focus(true), closed);
    assert!(!pane.mouse_tracking());
    assert_eq!(PaneInputError::Closed.to_string(), "the pane is closed");
}

#[test]
fn a_handle_with_no_terminal_has_no_selection_to_make_or_copy() {
    let (pane, _slot, _sized) = empty();
    let closed = Err(PaneInputError::Closed);
    assert_eq!(pane.select(at(0, 0), at(1, 0), false), Err(PaneInputError::Closed));
    assert_eq!(pane.select_word(at(0, 0)), closed);
    assert_eq!(pane.select_line(at(0, 0)), closed);
    assert_eq!(pane.select_all(), closed);
    assert_eq!(pane.clear_selection(), Err(PaneInputError::Closed));
    assert_eq!(pane.selection_text(), None);
}

#[test]
fn the_chat_actions_of_a_handle_with_no_core_are_closed_after_their_own_checks() {
    let (pane, _slot, _sized) = empty();
    assert_eq!(block_on(pane.send_prompt("hi".to_owned())), Err(PaneInputError::Closed));
    assert_eq!(block_on(pane.interrupt()), Err(PaneInputError::Closed));
    assert_eq!(block_on(pane.approval_labels()), Err(PaneInputError::Closed));
    assert_eq!(
        block_on(pane.send_prompt(String::new())),
        Err(refused("there is nothing to send")),
        "empty text is refused before anything is asked"
    );

    // a core that stopped: the handle's weak queue no longer reaches it
    let (commands, queue) = mpsc::unbounded_channel();
    let (_, slot, _) = empty();
    let reference = PaneRef {
        machine_id: "m1".to_owned(),
        session_id: "s1".to_owned(),
        pane_id: "B1".to_owned(),
    };
    let acting = Pane::new("B1".to_owned(), slot).acting(reference, commands.downgrade());
    drop((commands, queue));
    assert_eq!(block_on(acting.interrupt()), Err(PaneInputError::Closed));
}

#[test]
fn a_grid_given_with_no_terminal_is_kept_and_wakes_the_core_and_an_empty_one_is_refused() {
    let (pane, _slot, sized) = empty();
    assert_eq!(
        pane.resize(PaneSize { rows: 0, ..SIZE }),
        Err(refused("a pane needs at least one column and one row"))
    );
    assert_eq!(pane.size(), SIZE, "an empty grid is not kept");
    assert_eq!(sized.load(Ordering::SeqCst), 0);

    let wider = PaneSize { cols: 100, ..SIZE };
    assert_eq!(pane.resize(wider), Ok(()));
    assert_eq!(pane.size(), wider);
    assert_eq!(sized.load(Ordering::SeqCst), 1, "a pane waiting for a grid attaches");
}

#[test]
fn keys_are_sent_as_the_layout_types_them_and_a_lone_modifier_or_release_sends_nothing() {
    let shown = Shown::new();
    assert_eq!(shown.pane.key(key("key_a", 0, "a")), Ok(true));
    assert_eq!(shown.sent(), b"a");
    let repeat = PaneKey {
        action: PaneKeyAction::Repeat,
        ..key("key_a", 0, "a")
    };
    assert_eq!(shown.pane.key(repeat), Ok(true));
    assert_eq!(shown.sent(), b"a", "a held key repeats");
    let release = PaneKey {
        action: PaneKeyAction::Release,
        ..key("key_a", 0, "a")
    };
    assert_eq!(shown.pane.key(release), Ok(false));
    // ctrl (bit 2) with c: the control character
    assert_eq!(shown.pane.key(key("key_c", 2, "c")), Ok(true));
    assert_eq!(shown.sent(), b"\x03");
    assert_eq!(shown.pane.key(key("shift_left", 1, "")), Ok(false));
    // a key Ghostty doesn't name still types its text
    assert_eq!(shown.pane.key(key("no-such-key", 0, "é")), Ok(true));
    assert_eq!(shown.sent(), "é".as_bytes());
    assert_eq!(shown.sent(), b"");
}

#[test]
fn a_lone_left_arrow_is_not_sent_to_claudes_empty_prompt() {
    // Claude Code moves the conversation to a background process on it, at once when idle and at the turn's end
    // when working
    for state in ["idle", "working"] {
        let shown = Shown::showing("claude", state, b"");
        assert_eq!(shown.pane.key(key("arrow_left", 0, "")), Ok(false), "{state}");
        let repeat = PaneKey {
            action: PaneKeyAction::Repeat,
            ..key("arrow_left", 0, "")
        };
        assert_eq!(shown.pane.key(repeat), Ok(false), "{state}");
        // the locks are no modifiers: caps lock (16) and num lock (32)
        assert_eq!(shown.pane.key(key("arrow_left", 48, "")), Ok(false), "{state}");
        assert_eq!(shown.sent(), b"", "{state}");
        assert_eq!(shown.pane.key(key("arrow_right", 0, "")), Ok(true), "{state}");
        assert_eq!(shown.sent(), b"\x1b[C", "{state}");
    }
}

#[test]
fn a_left_arrow_reaches_claude_with_a_draft_a_modifier_or_a_dialog_and_every_other_program() {
    let left = b"\x1b[D".as_slice();
    let capture = Capture::load("claude-44x36");
    let (draft, _) = draft_drawn(&capture, Agent::Claude, "idle", "fix the build");
    let shown = Shown::showing("claude", "idle", &draft);
    assert_eq!(shown.pane.key(key("arrow_left", 0, "")), Ok(true));
    assert_eq!(shown.sent(), left, "the cursor moves in a draft");

    let shown = Shown::showing("claude", "idle", b"");
    // shift (1): Claude Code moves the cursor and nothing else
    assert_eq!(shown.pane.key(key("arrow_left", 1, "")), Ok(true));
    assert!(shown.sent().ends_with(b"D"));

    for (agent, state) in [("claude", "permission"), ("claude", "question"), ("codex", "idle")] {
        let shown = Shown::showing(agent, state, b"");
        assert_eq!(shown.pane.key(key("arrow_left", 0, "")), Ok(true), "{agent} {state}");
        assert!(shown.sent().ends_with(b"D"), "{agent} {state}");
    }

    let shown = Shown::new();
    shown.feed(b"$ ");
    assert_eq!(shown.pane.key(key("arrow_left", 0, "")), Ok(true));
    assert_eq!(shown.sent(), left, "a shell");
}

#[test]
fn the_kitty_protocol_reports_the_unshifted_key_and_releases() {
    let shown = Shown::new();
    // disambiguate (1), report event types (2), report all keys as escapes (8)
    shown.feed(b"\x1b[>11u");
    let shifted = PaneKey {
        consumed: 1,
        unshifted: Some("a".to_owned()),
        ..key("key_a", 1, "A")
    };
    assert_eq!(shown.pane.key(shifted.clone()), Ok(true));
    assert_eq!(
        shown.sent(),
        b"\x1b[97;2u",
        "the key code is the unshifted a, shift reported"
    );
    let release = PaneKey {
        action: PaneKeyAction::Release,
        ..shifted
    };
    assert_eq!(shown.pane.key(release), Ok(true));
    assert_eq!(shown.sent(), b"\x1b[97;2:3u");
}

#[test]
fn typed_text_goes_to_the_program_and_a_full_queue_pushes_back() {
    let shown = Shown::new();
    assert_eq!(shown.pane.type_text("ls".to_owned()), Ok(true));
    // the queue holds `ls` the machine has not taken: input past the write limit waits
    let flood = "x".repeat(2 * 1024 * 1024);
    assert_eq!(shown.pane.type_text(flood), Err(PaneInputError::Backpressure));
    assert_eq!(
        PaneInputError::Backpressure.to_string(),
        "the pane's input queue is full"
    );
    assert_eq!(shown.sent(), b"ls");
}

#[test]
fn pointer_events_reach_a_program_only_while_it_tracks_the_mouse() {
    let shown = Shown::new();
    let press = mouse(PaneMouseAction::Press, Some(PaneMouseButton::Left));
    assert!(!shown.pane.mouse_tracking());
    assert_eq!(shown.pane.mouse(press), Ok(false));

    // button tracking, SGR coordinates
    shown.feed(b"\x1b[?1000h\x1b[?1006h");
    assert!(shown.pane.mouse_tracking());
    assert_eq!(shown.pane.mouse(press), Ok(true));
    let release = mouse(PaneMouseAction::Release, Some(PaneMouseButton::Left));
    assert_eq!(shown.pane.mouse(release), Ok(true));
    assert_eq!(shown.sent(), b"\x1b[<0;4;3M\x1b[<0;4;3m");
    let middle = mouse(PaneMouseAction::Press, Some(PaneMouseButton::Middle));
    let right = mouse(PaneMouseAction::Press, Some(PaneMouseButton::Right));
    assert_eq!(shown.pane.mouse(middle), Ok(true));
    assert_eq!(shown.pane.mouse(right), Ok(true));
    assert_eq!(shown.sent(), b"\x1b[<1;4;3M\x1b[<2;4;3M");
    // button tracking reports no motion without a button
    assert_eq!(shown.pane.mouse(mouse(PaneMouseAction::Motion, None)), Ok(false));
}

#[test]
fn a_wheel_turn_is_reported_sent_as_cursor_keys_or_scrolls_the_view() {
    let shown = Shown::new();
    let sideways = PaneWheel {
        rows: 0,
        columns: 2,
        ..wheel(0)
    };
    assert_eq!(
        shown.pane.wheel(sideways),
        Ok(PaneWheelOutcome::Nothing),
        "a view scrolls by rows only"
    );

    shown.feed(b"\x1b[?1000h\x1b[?1006h");
    assert_eq!(shown.pane.wheel(wheel(1)), Ok(PaneWheelOutcome::Reported));
    assert_eq!(shown.sent(), b"\x1b[<64;1;1M");

    // the alternate screen with no tracking: the wheel is the arrow keys
    shown.feed(b"\x1b[?1000l\x1b[?1049h");
    assert_eq!(shown.pane.wheel(wheel(1)), Ok(PaneWheelOutcome::CursorKeys));
    assert_eq!(shown.sent(), b"\x1b[A");

    // back on the main screen with rows above it
    shown.feed(b"\x1b[?1049l\r\n1\r\n2\r\n3\r\n4\r\n5\r\n6\r\n7");
    assert_eq!(shown.pane.wheel(wheel(1)), Ok(PaneWheelOutcome::Scrolled));
    assert_eq!(shown.sent(), b"");
}

#[test]
fn a_paste_is_framed_for_the_program_and_unsafe_text_waits_for_the_users_word() {
    let shown = Shown::new();
    assert_eq!(shown.pane.paste(String::new(), false), Ok(PaneSendOutcome::Empty));
    assert_eq!(
        shown.pane.paste("echo hi".to_owned(), false),
        Ok(PaneSendOutcome::Written)
    );
    assert_eq!(shown.sent(), b"echo hi");
    assert_eq!(
        shown.pane.paste("rm -rf build\n".to_owned(), false),
        Ok(PaneSendOutcome::Unsafe)
    );
    assert_eq!(shown.sent(), b"", "nothing goes until the user allows it");
    assert_eq!(
        shown.pane.paste("rm -rf build\n".to_owned(), true),
        Ok(PaneSendOutcome::Written)
    );
    assert_eq!(shown.sent(), b"rm -rf build\r");

    shown.feed(b"\x1b[?2004h");
    assert_eq!(
        shown.pane.paste("two\nlines".to_owned(), false),
        Ok(PaneSendOutcome::Written)
    );
    assert_eq!(shown.sent(), b"\x1b[200~two\nlines\x1b[201~");
}

#[test]
fn focus_is_reported_only_to_a_program_that_asked() {
    let shown = Shown::new();
    assert_eq!(shown.pane.focus(true), Ok(false));
    shown.feed(b"\x1b[?1004h");
    assert_eq!(shown.pane.focus(false), Ok(true));
    assert_eq!(shown.pane.focus(true), Ok(true));
    assert_eq!(shown.sent(), b"\x1b[O\x1b[I");
}

#[test]
fn selections_take_words_lines_and_ranges_and_copy_their_text() {
    let shown = Shown::new();
    shown.feed(b"hello world\r\nsecond line");

    assert_eq!(shown.pane.select_word(at(7, 0)), Ok(true));
    assert_eq!(shown.pane.selection_text().as_deref(), Some("world"));
    assert_eq!(shown.pane.select_line(at(3, 1)), Ok(true));
    assert_eq!(shown.pane.selection_text().as_deref(), Some("second line"));
    assert_eq!(shown.pane.select(at(4, 1), at(6, 0), false), Ok(()));
    assert_eq!(
        shown.pane.selection_text().as_deref(),
        Some("world\nsecon"),
        "either order, ends included"
    );
    assert_eq!(shown.pane.select(at(0, 0), at(2, 1), true), Ok(()));
    assert_eq!(shown.pane.selection_text().as_deref(), Some("hel\nsec"));
}

#[test]
fn select_all_takes_every_line_and_blanks_select_nothing() {
    let shown = Shown::new();
    assert_eq!(shown.pane.select_all(), Ok(false), "a blank screen");
    shown.feed(b"hello world\r\nsecond line");
    assert_eq!(shown.pane.select_all(), Ok(true));
    assert_eq!(shown.pane.selection_text().as_deref(), Some("hello world\nsecond line"));
    assert_eq!(shown.pane.clear_selection(), Ok(()));
    assert_eq!(shown.pane.selection_text(), None);

    assert_eq!(shown.pane.select_word(at(20, 0)), Ok(false), "no word on a blank");
    assert_eq!(shown.pane.select_line(at(0, 3)), Ok(false), "a blank line");
}

#[test]
fn a_point_outside_the_grid_is_refused_with_the_terminals_reason() {
    let shown = Shown::new();
    assert_eq!(
        shown.pane.select_word(at(0, 99)),
        Err(refused("terminal: libghostty-vt refused an invalid value"))
    );
}

#[test]
fn the_views_theme_is_what_the_terminal_reports_and_more_than_256_colors_are_refused() {
    let (pane, slot, sized) = empty();
    let theme = PaneColors {
        foreground: 0xFF11_2233,
        background: 0x00AA_BBCC,
        cursor: Some(0xFF00_FF00),
        palette: vec![0xFF01_0203],
    };
    // given before any terminal: the next one put in place starts from it
    assert_eq!(pane.set_colors(theme.clone()), Ok(()));
    let (terminal, feed, out) = terminal(SIZE.into());
    slot.install(Arc::new(terminal), SIZE.into());
    let shown = Shown { pane, feed, out, sized };
    let reported = String::from_utf8(shown.feed(b"\x1b]10;?\x1b\\\x1b]11;?\x1b\\\x1b]4;0;?\x1b\\")).unwrap();
    assert!(reported.contains("10;rgb:1111/2222/3333"), "{reported:?}");
    assert!(
        reported.contains("11;rgb:aaaa/bbbb/cccc"),
        "the alpha is ignored: {reported:?}"
    );
    assert!(reported.contains("4;0;rgb:0101/0202/0303"), "{reported:?}");

    let wide = PaneColors {
        palette: vec![0; 257],
        ..theme
    };
    assert_eq!(
        shown.pane.set_colors(wide),
        Err(refused("a palette has at most 256 colors"))
    );
    assert_eq!(shown.sized.load(Ordering::SeqCst), 0);
}

#[test]
fn a_handles_description_names_its_slot() {
    let shown = Shown::new();
    let described = format!("{:?}", shown.pane);
    assert!(described.contains("PaneSlot"), "{described}");
    assert!(described.contains("B1"), "{described}");
}
