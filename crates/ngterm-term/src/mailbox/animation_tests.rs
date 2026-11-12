use std::sync::mpsc;
use std::time::Duration;

use super::*;

#[test]
fn animation_wake_interrupts_an_indefinite_or_later_wait_once() {
    for previous in [None, Some(Instant::now() + Duration::from_secs(60))] {
        let mailbox = Arc::new(Mailbox::new(ReadPause::new()));
        mailbox.animation_deadline(previous);
        let (tx, rx) = mpsc::channel();
        let (waiting, parked) = mpsc::channel();
        *mailbox.before_wait.lock().unwrap() = Some(waiting);
        let worker = {
            let mailbox = Arc::clone(&mailbox);
            std::thread::spawn(move || {
                tx.send(mailbox.next(None)).unwrap();
                tx.send(mailbox.next(None)).unwrap();
            })
        };
        // The signal is sent under the state lock just before the condvar atomically releases it and sleeps.
        // Rearming therefore reaches a waiting worker, rather than winning the race before it enters next().
        let entered_wait = parked.recv_timeout(Duration::from_secs(5));
        mailbox.animation_deadline(Some(Instant::now()));
        let first = rx.recv_timeout(Duration::from_secs(5));
        let repeated = rx.recv_timeout(Duration::from_millis(20));
        mailbox.shutdown();
        let end = rx.recv_timeout(Duration::from_secs(5));
        worker.join().unwrap();
        assert!(entered_wait.is_ok(), "the worker must have entered its wait");
        assert_eq!(first.unwrap(), Mail::Idle);
        assert!(
            matches!(repeated, Err(mpsc::RecvTimeoutError::Timeout)),
            "a wake is consumed once"
        );
        assert_eq!(end.unwrap(), Mail::End);
    }
}

#[test]
fn animation_wake_is_cancelled_and_shutdown_cannot_rearm_it() {
    let mailbox = Mailbox::new(ReadPause::new());
    mailbox.animation_deadline(Some(Instant::now()));
    mailbox.animation_deadline(None);
    mailbox.deliver(b"still live");
    assert!(matches!(mailbox.next(None), Mail::Output(_)));
    mailbox.shutdown();
    mailbox.animation_deadline(Some(Instant::now()));
    assert_eq!(mailbox.lock().animation_deadline, None);
    assert_eq!(mailbox.next(None), Mail::End);
}

#[test]
fn animation_deadline_precedes_queued_output_and_survives_eof() {
    let mailbox = Mailbox::new(ReadPause::new());
    mailbox.deliver(b"output");
    mailbox.animation_deadline(Some(Instant::now()));
    assert_eq!(mailbox.next(None), Mail::Idle);
    assert!(matches!(mailbox.next(None), Mail::Output(_)));
    mailbox.end_of_output();
    assert_eq!(mailbox.next(None), Mail::Eof);
    mailbox.animation_deadline(Some(Instant::now()));
    assert_eq!(mailbox.next(None), Mail::Idle);
    mailbox.shutdown();
    assert_eq!(mailbox.next(None), Mail::End);
}

#[test]
fn animation_and_hold_wakes_also_interrupt_a_clipboard_wait() {
    let mailbox = Mailbox::new(ReadPause::new());
    mailbox.animation_deadline(Some(Instant::now() + Duration::from_secs(60)));
    assert_eq!(mailbox.await_answer(1, Some(Instant::now())), Answer::Idle);
    assert!(
        mailbox.lock().animation_deadline.is_some(),
        "the hold must not consume a future animation wake"
    );
    mailbox.animation_deadline(Some(Instant::now()));
    assert_eq!(mailbox.await_answer(1, None), Answer::Idle);
    assert_eq!(mailbox.lock().animation_deadline, None);
    mailbox.answer(1, ClipboardAnswer::Denied);
    assert_eq!(mailbox.await_answer(1, None), Answer::Given(ClipboardAnswer::Denied));
}
