//! Work that waits on `WebView2` in a nested message loop, run from the main thread's message loop instead of from a
//! GPUI task (`registry`'s pump hands it here).
//!
//! wry and `webview2-com` wait for `WebView2`'s answers (a web view's controller, a script added, the browser process
//! started by the first page) by pumping the thread's messages until it comes, which takes seconds for an instance's
//! first page (its browser process starts). Waited for inside a GPUI task, nothing else of GPUI's ran meanwhile, the
//! control socket's requests included: while a task runs, GPUI's Windows dispatcher posts no wake for the tasks
//! queued after it (`gpui-pre-windows`' `run_foreground_task` holds its wake flag), so the nested loop had nothing
//! of GPUI's to dispatch. Run from a thread timer's callback, which the main loop dispatches outside every task, the
//! same wait dispatches GPUI's wakes as they come, and its tasks keep running.
//!
//! The work runs one item at a time: a timer that fires inside an item's own nested loop leaves the queue to that
//! item, which takes the next one once it returns. Nothing here holds a borrow across an item, which may come back
//! here (a page asking for another web view).
//!
//! A nested loop takes the thread's quit message too, ending its wait: once the app quits ([`quit`]), what is queued
//! is dropped, and the quit message is posted again after the item that may have taken it, for GPUI's loop.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{KillTimer, PostQuitMessage, SetTimer};

/// One item of work.
pub type Work = Box<dyn FnOnce()>;

thread_local! {
    static QUEUE: RefCell<VecDeque<Work>> = RefCell::default();
    /// The thread timer that runs the queue, while one is set.
    static TIMER: Cell<usize> = const { Cell::new(0) };
    /// An item runs (its nested loop may dispatch the timer).
    static RUNNING: Cell<bool> = const { Cell::new(false) };
    /// The app is quitting.
    static QUITTING: Cell<bool> = const { Cell::new(false) };
}

/// The app quits: nothing queued runs any more (module docs).
pub fn quit() {
    QUITTING.set(true);
    QUEUE.with(|queue| queue.borrow_mut().clear());
}

/// Runs `work` from the main thread's message loop, after what was queued before it; nothing once the app quits.
pub fn run(work: Work) {
    if QUITTING.get() {
        return;
    }
    QUEUE.with(|queue| queue.borrow_mut().push_back(work));
    arm();
}

/// Sets the timer unless it is set, or an item runs (which takes the next one itself).
fn arm() {
    if TIMER.get() != 0 || RUNNING.get() {
        return;
    }
    // SAFETY: a thread timer (no window) whose callback is a plain function; USER_TIMER_MINIMUM stands for zero.
    let timer = unsafe { SetTimer(std::ptr::null_mut(), 0, 0, Some(fire)) };
    if timer == 0 {
        tracing::warn!("no timer for the web views' work: it runs in place");
        drain();
    } else {
        TIMER.set(timer);
    }
}

/// The timer's callback: the next item, then the timer again for the one after it.
unsafe extern "system" fn fire(_: HWND, _: u32, timer: usize, _: u32) {
    // SAFETY: the thread timer this callback was set for.
    unsafe { KillTimer(std::ptr::null_mut(), timer) };
    if TIMER.get() == timer {
        TIMER.set(0);
    }
    if RUNNING.get() {
        return;
    }
    let next = QUEUE.with(|queue| queue.borrow_mut().pop_front());
    if let Some(work) = next {
        run_item(work);
    }
    if QUEUE.with(|queue| !queue.borrow().is_empty()) {
        arm();
    }
}

/// Runs every queued item in place: only when no timer can be set.
fn drain() {
    if RUNNING.get() {
        return;
    }
    while let Some(work) = QUEUE.with(|queue| queue.borrow_mut().pop_front()) {
        run_item(work);
    }
}

/// Runs one item; a quit while it ran is posted again, since its nested loop may have taken the quit message.
fn run_item(work: Work) {
    RUNNING.set(true);
    work();
    RUNNING.set(false);
    if QUITTING.get() {
        // SAFETY: posts the thread's quit message, as GPUI's own quit does.
        unsafe { PostQuitMessage(0) };
    }
}

#[cfg(test)]
#[path = "outside_tests.rs"]
mod tests;
