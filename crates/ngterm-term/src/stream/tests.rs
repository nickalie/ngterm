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
