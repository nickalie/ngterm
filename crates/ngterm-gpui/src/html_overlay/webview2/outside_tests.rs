//! The queue on fresh Win32 message threads, including a timer dispatched inside an item's nested loop.

use std::rc::Rc;
use std::sync::mpsc::{self, TryRecvError};
use std::time::{Duration, Instant};

use windows_sys::Win32::UI::WindowsAndMessaging::{DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, WM_QUIT, WM_TIMER};

use super::*;

/// Dispatches one real thread timer, with a deadline so a lost wake fails the test.
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
}

#[test]
fn timers_run_work_in_order_without_reentering_an_item() {
    std::thread::spawn(|| {
        let order = Rc::new(RefCell::new(Vec::new()));
        let first = Rc::clone(&order);
        run(Box::new(move || {
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
        }));
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
fn draining_runs_every_item_and_leaves_an_armed_timer_nothing_to_do() {
    std::thread::spawn(|| {
        let (sender, receiver) = mpsc::channel();
        for value in [1, 2, 3] {
            let sender = sender.clone();
            run(Box::new(move || sender.send(value).unwrap()));
        }
        drain();
        assert_eq!(receiver.try_iter().collect::<Vec<_>>(), [1, 2, 3]);
        dispatch_timer();
        assert_eq!(receiver.try_recv(), Err(TryRecvError::Empty));
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
        run(Box::new(|| {
            quit();
            // SAFETY: only this fresh test thread is asked to quit; its nested wait consumes the message.
            unsafe { PostQuitMessage(37) };
            assert_eq!(take_quit(), 37);
        }));
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
