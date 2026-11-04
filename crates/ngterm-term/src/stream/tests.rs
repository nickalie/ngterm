use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use crate::{ClipboardAnswer, ExitPolicy, ExitStatus, Pane, PaneConfig, PaneEvent, TextRegion};

use super::*;

const SIZE: GridSize = GridSize {
    cols: 20,
    rows: 4,
    cell_width: 8,
    cell_height: 16,
};
const DEADLINE: Duration = Duration::from_secs(10);

struct Fixture {
    pane: Pane,
    feed: StreamFeed,
    out: StreamOut,
    woken: Receiver<()>,
}

fn stream(config: PaneConfig) -> Fixture {
    let (wake, woken) = mpsc::channel();
    let wake = Mutex::new(wake);
    let config = PaneConfig {
        wake: Some(Arc::new(move || {
            let _ = lock(&wake).send(());
        })),
        ..config
    };
    let (pane, feed, out) = Pane::stream(&config).unwrap();
    Fixture { pane, feed, out, woken }
}

impl Fixture {
    /// Bytes sent to the channel until they hold `want`.
    fn sent_until(&self, want: &[u8]) -> Vec<u8> {
        let deadline = Instant::now() + DEADLINE;
        let mut sent = Vec::new();
        while !sent.windows(want.len()).any(|window| window == want) {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "never sent {want:?}: {sent:?}");
            if let Ok(Some(Outgoing::Bytes(bytes))) = self.out.next(Some(left)) {
                sent.extend(bytes);
            }
        }
        sent
    }

    fn event(&self, matches: impl Fn(&PaneEvent) -> bool) -> PaneEvent {
        let deadline = Instant::now() + DEADLINE;
        loop {
            if let Some(event) = self.pane.take_events().into_iter().find(&matches) {
                return event;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "no such event");
            let _ = self.woken.recv_timeout(left);
        }
    }

    fn screen_until(&self, want: &str) {
        let deadline = Instant::now() + DEADLINE;
        while !self.pane.read_text(TextRegion::Viewport).unwrap().contains(want) {
            assert!(Instant::now() < deadline, "never showed {want:?}");
            let _ = self.woken.recv_timeout(Duration::from_millis(50));
        }
    }
}

#[test]
fn output_reaches_the_screen_and_the_terminal_answers_through_the_channel() {
    let fixture = stream(PaneConfig::new(SIZE));
    assert!(fixture.feed.deliver(b"hello\x1b[6n"));
    fixture.screen_until("hello");
    // the cursor position report, after "hello" on the first row
    fixture.sent_until(b"\x1b[1;6R");
    let frame = fixture.pane.frame().frame;
    assert_eq!(frame.rows.len(), usize::from(SIZE.rows));
}

#[test]
fn input_and_resizes_go_out_in_order() {
    let fixture = stream(PaneConfig::new(SIZE));
    fixture.pane.write(b"ls\r").unwrap();
    let bigger = GridSize { cols: 40, ..SIZE };
    fixture.pane.resize(bigger).unwrap();
    fixture.pane.write(b"x").unwrap();
    assert_eq!(fixture.out.next(None), Ok(Some(Outgoing::Bytes(b"ls\r".to_vec()))));
    assert_eq!(fixture.out.next(None), Ok(Some(Outgoing::Resize(bigger))));
    assert_eq!(fixture.out.next(None), Ok(Some(Outgoing::Bytes(b"x".to_vec()))));
    assert_eq!(fixture.out.next(Some(Duration::from_millis(10))), Ok(None));
}

#[test]
fn input_past_the_write_limit_is_refused_and_replies_are_not() {
    let fixture = stream(PaneConfig {
        write_limit: 4,
        ..PaneConfig::new(SIZE)
    });
    fixture.pane.write(b"abcd").unwrap();
    assert_eq!(fixture.pane.write(b"e"), Err(PaneError::Backpressure));
    assert_eq!(fixture.pane.queued_input(), 4);
    fixture.feed.deliver(b"\x1b[5n");
    fixture.sent_until(b"abcd\x1b[0n");
    fixture.pane.write(b"e").unwrap();
}

#[test]
fn an_empty_queue_takes_input_past_the_write_limit() {
    // a paste larger than the limit goes out whole, as on a PTY or live pane
    let fixture = stream(PaneConfig {
        write_limit: 4,
        ..PaneConfig::new(SIZE)
    });
    fixture.pane.write(b"abcde").unwrap();
    assert_eq!(fixture.pane.queued_input(), 5);
    assert_eq!(fixture.pane.write(b"f"), Err(PaneError::Backpressure));
    assert_eq!(fixture.out.next(None), Ok(Some(Outgoing::Bytes(b"abcde".to_vec()))));
    fixture.pane.write(b"f").unwrap();
}

#[test]
fn a_clipboard_read_waits_for_the_frontends_answer() {
    let fixture = stream(PaneConfig::new(SIZE));
    fixture.feed.deliver(b"\x1b]52;c;?\x07after");
    let PaneEvent::ClipboardRead(request) = fixture.event(|event| matches!(event, PaneEvent::ClipboardRead(_))) else {
        unreachable!()
    };
    // the output after the read waits for its answer
    assert!(!fixture.pane.read_text(TextRegion::Viewport).unwrap().contains("after"));
    fixture
        .pane
        .answer_clipboard_read(request.id, ClipboardAnswer::Text(Some("hi".to_owned())));
    fixture.sent_until(b"\x1b]52;c;aGk=");
    fixture.screen_until("after");
}

#[test]
fn the_programs_end_is_reported_once_and_the_pane_drop_ends_both_sides() {
    let fixture = stream(PaneConfig::new(SIZE));
    fixture.feed.deliver(b"bye");
    fixture.feed.end_of_output();
    fixture.feed.exited(Some(3), None);
    fixture.feed.exited(Some(4), None);
    let exited = fixture.event(|event| matches!(event, PaneEvent::Exited(_)));
    assert_eq!(
        exited,
        PaneEvent::Exited(ExitStatus {
            code: 3,
            signal: None,
            held: false
        })
    );
    let Fixture { pane, feed, out, .. } = fixture;
    drop(pane);
    assert_eq!(out.next(None), Err(PaneError::Closed));
    assert!(!feed.deliver(b"late"));
}

#[test]
fn a_reader_past_the_high_mark_waits_until_the_pane_catches_up() {
    let fixture = stream(PaneConfig::new(SIZE));
    // an unanswered clipboard read holds the output back, so the mailbox fills
    fixture.feed.deliver(b"\x1b]52;c;?\x07");
    let PaneEvent::ClipboardRead(request) = fixture.event(|event| matches!(event, PaneEvent::ClipboardRead(_))) else {
        unreachable!()
    };
    let Fixture { pane, feed, out, .. } = fixture;
    let feed = Arc::new(feed);
    let reader = {
        let feed = Arc::clone(&feed);
        std::thread::spawn(move || feed.deliver(&vec![b'x'; crate::mailbox::HIGH_WATER]))
    };
    std::thread::sleep(Duration::from_millis(100));
    assert!(!reader.is_finished(), "the reader went on past the high mark");
    pane.answer_clipboard_read(request.id, ClipboardAnswer::Denied);
    assert!(reader.join().unwrap());
    drop(out);
}

#[test]
fn a_reply_flood_is_capped_as_the_pty_actors_is() {
    // the program floods queries and reads no input: replies past the cap are dropped, never queued without bound
    let outbound = Outbound::new(4);
    outbound.push_reply(&vec![0; REPLY_CAP]);
    outbound.push_reply(b"one too many");
    assert_eq!(outbound.queued(), REPLY_CAP);
    // a write limit above the cap raises it, as the actor's does
    let roomy = Outbound::new(REPLY_CAP + 8);
    roomy.push_reply(&vec![0; REPLY_CAP]);
    roomy.push_reply(b"8 bytes!");
    assert_eq!(roomy.queued(), REPLY_CAP + 8);
    roomy.push_reply(b"x");
    assert_eq!(roomy.queued(), REPLY_CAP + 8);
    // once the writer takes them, replies queue again
    assert!(matches!(outbound.next(None), Ok(Some(Outgoing::Bytes(_)))));
    outbound.push_reply(b"\x1b[0n");
    assert_eq!(outbound.next(None), Ok(Some(Outgoing::Bytes(b"\x1b[0n".to_vec()))));
}

#[cfg(feature = "local")]
#[test]
fn the_reply_cap_is_the_pty_actors() {
    assert_eq!(REPLY_CAP, ngterm_pty::REPLY_CAP);
}

#[test]
fn exited_while_output_is_still_queued_shows_that_output_first() {
    let fixture = stream(PaneConfig {
        exit: ExitPolicy::Launched {
            argv: "sh".to_owned(),
            command: true,
            wait_after_command: true,
            abnormal_runtime: Duration::ZERO,
        },
        ..PaneConfig::new(SIZE)
    });
    // an unanswered clipboard read holds the output after it in the mailbox
    fixture.feed.deliver(b"\x1b]52;c;?\x07tail");
    fixture.event(|event| matches!(event, PaneEvent::ClipboardRead(_)));
    assert!(!fixture.pane.read_text(TextRegion::Viewport).unwrap().contains("tail"));
    // the program's end denies the read and feeds what was queued before the exit message and the report
    fixture.feed.end_of_output();
    fixture.feed.exited(Some(0), None);
    let exited = fixture.event(|event| matches!(event, PaneEvent::Exited(_)));
    assert_eq!(
        exited,
        PaneEvent::Exited(ExitStatus {
            code: 0,
            signal: None,
            held: true
        })
    );
    let screen = fixture.pane.read_text(TextRegion::Viewport).unwrap();
    let tail = screen.find("tail").expect("the queued output reached the screen");
    let message = screen.find("Process exited").expect("the exit message is on screen");
    assert!(
        tail < message,
        "the exit message came before the queued output: {screen:?}"
    );
}

#[test]
fn a_signal_exit_reports_code_one() {
    let fixture = stream(PaneConfig::new(SIZE));
    fixture.feed.end_of_output();
    fixture.feed.exited(None, Some("TERM".to_owned()));
    let exited = fixture.event(|event| matches!(event, PaneEvent::Exited(_)));
    assert_eq!(
        exited,
        PaneEvent::Exited(ExitStatus {
            code: 1,
            signal: Some("TERM".to_owned()),
            held: false
        })
    );
}

#[test]
fn deliver_after_the_pane_dropped_returns_false() {
    let Fixture { pane, feed, out, .. } = stream(PaneConfig::new(SIZE));
    drop(pane);
    assert!(!feed.deliver(b"late"));
    assert!(!feed.deliver(b""));
    // the rest of the feed does nothing harmful once the pane is gone
    feed.end_of_output();
    feed.exited(Some(1), None);
    assert_eq!(out.next(Some(Duration::from_millis(10))), Err(PaneError::Closed));
}

#[test]
fn a_reader_waiting_past_the_high_mark_is_released_when_the_pane_drops() {
    let fixture = stream(PaneConfig::new(SIZE));
    fixture.feed.deliver(b"\x1b]52;c;?\x07");
    fixture.event(|event| matches!(event, PaneEvent::ClipboardRead(_)));
    let Fixture { pane, feed, out, .. } = fixture;
    let feed = Arc::new(feed);
    let reader = {
        let feed = Arc::clone(&feed);
        std::thread::spawn(move || feed.deliver(&vec![b'x'; crate::mailbox::HIGH_WATER]))
    };
    std::thread::sleep(Duration::from_millis(100));
    assert!(!reader.is_finished(), "the reader went on past the high mark");
    drop(pane);
    assert!(
        !reader.join().unwrap(),
        "a reader released by the drop must stop reading"
    );
    drop(out);
}

/// The `screen-read` feature's reads and conditional sends.
#[cfg(feature = "screen-read")]
mod screen_read {
    use super::*;

    /// The program's screen now, once it is not held.
    fn active(pane: &Pane) -> crate::ActiveScreen {
        let deadline = Instant::now() + DEADLINE;
        loop {
            if let Some(screen) = pane.active_screen().unwrap() {
                return screen;
            }
            assert!(Instant::now() < deadline, "the screen stays held");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// The generation of the pane's screen now.
    fn generation(pane: &Pane) -> u64 {
        active(pane).generation
    }

    /// The view's frame once its first row reads `first`.
    fn view_until(pane: &Pane, first: &str) -> Arc<crate::Frame> {
        let deadline = Instant::now() + DEADLINE;
        loop {
            let shown = pane.frame();
            if !shown.stale && rows(&shown.frame)[0] == first {
                return shown.frame;
            }
            assert!(Instant::now() < deadline, "the view never showed {first:?}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// What a frame's rows read, trailing blanks dropped.
    fn rows(frame: &crate::Frame) -> Vec<String> {
        frame.lines().iter().map(|line| line.trim_end().to_owned()).collect()
    }

    #[test]
    fn a_conditional_key_or_paste_goes_only_to_the_screen_it_was_read_from() {
        use crate::{ClipboardLocation, KeyInput, Paste, PasteSource};
        use ngterm_core::ghostty_config::binds::{Mods, PhysicalKey};

        let fixture = stream(PaneConfig::new(SIZE));
        let enter = KeyInput::press(PhysicalKey::new("enter").unwrap(), Mods::empty(), "");
        let paste = |generation| {
            fixture.pane.paste_if(
                generation,
                "hi",
                PasteSource::Clipboard,
                ClipboardLocation::Standard,
                false,
            )
        };
        assert!(fixture.feed.deliver(b"\x1b[?2004hprompt"));
        fixture.screen_until("prompt");
        let shown = generation(&fixture.pane);
        assert_eq!(fixture.pane.key_if(shown, &enter), Ok(Some(true)));
        // the pane's own input leaves the screen as it was
        assert_eq!(paste(shown), Ok(Some(Paste::Written)));
        assert_eq!(fixture.sent_until(b"\x1b[201~"), b"\r\x1b[200~hi\x1b[201~");

        // the program wrote since (a status query, answered once the terminal took it): nothing goes
        assert!(fixture.feed.deliver(b"\x1b[5n"));
        assert_eq!(fixture.sent_until(b"\x1b[0n"), b"\x1b[0n");
        assert_eq!(fixture.pane.key_if(shown, &enter), Ok(None));
        assert_eq!(paste(shown), Ok(None));
        assert_ne!(generation(&fixture.pane), shown);
    }

    #[test]
    fn the_active_screen_is_the_programs_whatever_the_view_shows() {
        use crate::{KeyInput, Scroll};
        use ngterm_core::ghostty_config::binds::{Mods, PhysicalKey};

        let fixture = stream(PaneConfig::new(SIZE));
        // an old dialog scrolled into the history, the live screen below it
        assert!(
            fixture
                .feed
                .deliver(b"1. Yes\r\n2. No\r\nEsc to cancel\r\n\r\n\r\n\r\n\r\n> \x1b[2mtype\x1b[0m")
        );
        fixture.screen_until("type");
        let live = active(&fixture.pane);
        assert_eq!(rows(&live.frame), ["", "", "", "> type"]);
        assert!(!live.reflowed);
        let faint = live.frame.rows[3].cells()[2];
        assert!(faint.flags.contains(crate::CellFlags::FAINT), "styles are read too");
        let cursor = live.frame.cursor.position.unwrap();
        assert_eq!((cursor.column, cursor.row), (6, 3));

        // the view scrolled back onto the old dialog, everything selected: the program's screen is the same one
        fixture.pane.scroll(Scroll::Top);
        assert!(fixture.pane.select_all().unwrap());
        let shown = view_until(&fixture.pane, "1. Yes");
        assert_eq!(shown.cursor.position, None, "the view shows history");
        let read = active(&fixture.pane);
        assert_eq!(
            read.generation, live.generation,
            "a scroll or a selection is no change of the screen"
        );
        assert_eq!(rows(&read.frame), rows(&live.frame));
        assert!(read.frame.rows.iter().all(|row| row.selection().is_none()));

        // a key sent under it reaches that screen, and brings the view back to it
        let enter = KeyInput::press(PhysicalKey::new("enter").unwrap(), Mods::empty(), "");
        assert_eq!(fixture.pane.key_if(read.generation, &enter), Ok(Some(true)));
        assert_eq!(fixture.sent_until(b"\r"), b"\r");
        assert_eq!(rows(&fixture.pane.frame().frame), rows(&live.frame));
    }

    #[test]
    fn a_resize_or_a_held_screen_sends_no_conditional_key() {
        use crate::KeyInput;
        use ngterm_core::ghostty_config::binds::{Mods, PhysicalKey};

        let fixture = stream(PaneConfig::new(SIZE));
        let enter = KeyInput::press(PhysicalKey::new("enter").unwrap(), Mods::empty(), "");
        // resized since: nothing goes, and the screen is the VT's reflow until the program draws again
        let shown = generation(&fixture.pane);
        let bigger = GridSize { cols: 40, ..SIZE };
        fixture.pane.resize(bigger).unwrap();
        assert_eq!(fixture.pane.key_if(shown, &enter), Ok(None));
        assert_eq!(fixture.out.next(None), Ok(Some(Outgoing::Resize(bigger))));
        assert_eq!(fixture.out.next(Some(Duration::from_millis(20))), Ok(None));
        let reflowed = active(&fixture.pane);
        assert!(reflowed.reflowed);
        assert_eq!(reflowed.frame.cols, 40);
        assert!(fixture.feed.deliver(b"redrawn"));
        fixture.screen_until("redrawn");
        assert!(!active(&fixture.pane).reflowed);

        // the program holds its output (synchronized output): its screen is not read until it lets go
        assert!(fixture.feed.deliver(b"\x1b[?2026hnext"));
        let deadline = Instant::now() + DEADLINE;
        while fixture.pane.active_screen().unwrap().is_some() {
            assert!(Instant::now() < deadline, "never held");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(fixture.feed.deliver(b"\x1b[?2026l"));
        let deadline = Instant::now() + DEADLINE;
        loop {
            if let Some(screen) = fixture.pane.active_screen().unwrap() {
                assert_eq!(rows(&screen.frame)[0], "redrawnnext");
                break;
            }
            assert!(Instant::now() < deadline, "never let go");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn only_a_new_column_or_row_count_leaves_the_screen_reflowed() {
        let fixture = stream(PaneConfig::new(SIZE));
        assert!(fixture.feed.deliver(b"idle"));
        fixture.screen_until("idle");
        let drawn = active(&fixture.pane);
        assert!(!drawn.reflowed);

        // the view measures the same grid again: nothing reflows, the screen and its generation stay as they were
        fixture.pane.resize(SIZE).unwrap();
        let same = active(&fixture.pane);
        assert!(!same.reflowed, "an unchanged grid reflows nothing");
        assert_eq!(same.generation, drawn.generation);
        assert_eq!(rows(&same.frame)[0], "idle");

        // a new cell size alone moves no cell
        let denser = GridSize {
            cell_width: SIZE.cell_width + 2,
            cell_height: SIZE.cell_height + 4,
            ..SIZE
        };
        fixture.pane.resize(denser).unwrap();
        let resized = active(&fixture.pane);
        assert!(!resized.reflowed, "a cell size alone reflows nothing");
        assert_eq!(rows(&resized.frame)[0], "idle");

        // a new column count, then a new row count: each waits for the program to draw again, the same grid again too
        for size in [GridSize { cols: 30, ..denser }, GridSize { rows: 6, ..denser }] {
            fixture.pane.resize(size).unwrap();
            assert!(active(&fixture.pane).reflowed, "{size:?} reflows");
            fixture.pane.resize(size).unwrap();
            assert!(active(&fixture.pane).reflowed, "{size:?} again still waits");
            assert!(fixture.feed.deliver(b"\r\nredrawn"));
            let deadline = Instant::now() + DEADLINE;
            while active(&fixture.pane).reflowed {
                assert!(Instant::now() < deadline, "never redrawn");
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }

    #[test]
    fn a_hold_the_program_never_ends_expires_and_its_screen_is_read() {
        let fixture = stream(PaneConfig::new(SIZE));
        assert!(fixture.feed.deliver(b"\x1b[?2026hstuck"));
        let deadline = Instant::now() + DEADLINE;
        while fixture.pane.active_screen().unwrap().is_some() {
            assert!(Instant::now() < deadline, "never held");
            std::thread::sleep(Duration::from_millis(5));
        }
        // the program prints nothing more and never lets go: the hold ends at its timeout, as a paint would end it
        let screen = active(&fixture.pane);
        assert_eq!(rows(&screen.frame)[0], "stuck");
        assert!(!screen.reflowed);
    }
}
