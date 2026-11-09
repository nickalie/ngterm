//! The queue on fresh Win32 message threads, including a timer dispatched inside an item's nested loop.

use std::any::Any;
use std::panic::{self, AssertUnwindSafe};
use std::rc::Rc;
use std::sync::mpsc::{self, TryRecvError};
use std::time::{Duration, Instant};

use windows_sys::Win32::UI::WindowsAndMessaging::{DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, WM_QUIT, WM_TIMER};

use super::*;

thread_local! {
    /// What failed inside a checked item, until [`dispatch_timer`] is back on the test's own frames.
    static FAILED_CHECK: Cell<Option<Box<dyn Any + Send>>> = const { Cell::new(None) };
}

/// Queues an item whose assertions fail the test: `fire` cannot unwind, so a panic through it would abort every test
/// in the binary.
fn run_checked(check: impl FnOnce() + 'static) {
    run(Box::new(move || {
        if let Err(failure) = panic::catch_unwind(AssertUnwindSafe(check)) {
            FAILED_CHECK.set(Some(failure));
        }
    }));
}

/// Dispatches one real thread timer, with a deadline so a lost wake fails the test, and fails with what a checked item
/// it ran failed on.
#[track_caller]
fn dispatch_timer() {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut message = MSG::default();
    // SAFETY: this thread's message buffer and timer messages only, removed before dispatch.
    while unsafe { PeekMessageW(&raw mut message, std::ptr::null_mut(), WM_TIMER, WM_TIMER, PM_REMOVE) } == 0 {
        assert!(Instant::now() < deadline, "the queued work's timer did not arrive");
        std::thread::sleep(Duration::from_millis(1));
    }
    // SAFETY: a timer message just removed from this thread's queue.
    unsafe { DispatchMessageW(&raw const message) };
    if let Some(failure) = FAILED_CHECK.take() {
        panic::resume_unwind(failure);
    }
}

#[test]
fn timers_run_work_in_order_without_reentering_an_item() {
    std::thread::spawn(|| {
        let order = Rc::new(RefCell::new(Vec::new()));
        let first = Rc::clone(&order);
        run_checked(move || {
            first.borrow_mut().push("start");
            let nested = Rc::clone(&first);
            run(Box::new(move || nested.borrow_mut().push("nested")));
            drain();
            assert_eq!(*first.borrow(), ["start"], "draining cannot reenter the work");
            // SAFETY: a second real thread timer dispatches the same callback during this nested loop.
            let timer = unsafe { SetTimer(std::ptr::null_mut(), 0, 0, Some(fire)) };
            assert_ne!(timer, 0);
            dispatch_timer();
            assert_eq!(*first.borrow(), ["start"], "a nested timer cannot run the next item");
            first.borrow_mut().push("end");
        });
        let second = Rc::clone(&order);
        run(Box::new(move || second.borrow_mut().push("second")));
        assert!(order.borrow().is_empty(), "work waits for the message loop");
        while order.borrow().len() < 4 {
            dispatch_timer();
        }
        assert_eq!(*order.borrow(), ["start", "end", "second", "nested"]);
    })
    .join()
    .unwrap();
}

#[test]
fn draining_runs_every_item_and_the_spent_timer_lets_new_work_arm_another() {
    std::thread::spawn(|| {
        let (sender, receiver) = mpsc::channel();
        for value in [1, 2, 3] {
            let sender = sender.clone();
            run(Box::new(move || sender.send(value).unwrap()));
        }
        drain();
        assert_eq!(receiver.try_iter().collect::<Vec<_>>(), [1, 2, 3]);
        dispatch_timer();
        run(Box::new(move || sender.send(4).unwrap()));
        dispatch_timer();
        assert_eq!(receiver.try_recv(), Ok(4));
    })
    .join()
    .unwrap();
}

/// Takes this thread's quit message, as a nested browser wait may do.
fn take_quit() -> usize {
    let mut message = MSG::default();
    // SAFETY: this thread's own message queue and writable buffer; no window is accessed.
    let found = unsafe { PeekMessageW(&raw mut message, std::ptr::null_mut(), WM_QUIT, WM_QUIT, PM_REMOVE) };
    assert_ne!(found, 0, "the main loop must still receive its quit message");
    assert_eq!(message.message, WM_QUIT);
    message.wParam
}

#[test]
fn quitting_drops_pending_and_future_work_and_reposts_a_consumed_quit() {
    std::thread::spawn(|| {
        run_checked(|| {
            quit();
            // SAFETY: only this fresh test thread is asked to quit; its nested wait consumes the message.
            unsafe { PostQuitMessage(37) };
            assert_eq!(take_quit(), 37);
        });
        let (queued, pending) = mpsc::channel::<()>();
        run(Box::new(move || queued.send(()).unwrap()));
        dispatch_timer();
        assert_eq!(pending.try_recv(), Err(TryRecvError::Disconnected));
        assert_eq!(take_quit(), 0, "quit is posted again after the nested wait");
        let (future, refused) = mpsc::channel::<()>();
        run(Box::new(move || future.send(()).unwrap()));
        drain();
        assert_eq!(refused.try_recv(), Err(TryRecvError::Disconnected));
    })
    .join()
    .unwrap();
}
