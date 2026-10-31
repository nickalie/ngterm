//! [`ThreadMainLoop`]: the [`MainLoop`] of a process with no toolkit, the thread that calls [`ThreadMainLoop::run`]
//! owning the state. Tasks run one at a time in posting order; a delayed task joins the queue once its delay passed,
//! after the tasks posted before that moment. [`ThreadMainLoop::close`] stops it and drops what waits unrun, as the
//! GPUI frontend's loop does (`ngterm-gpui`'s `main_loop.rs`), so a waiter sees the loop closed.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, VecDeque};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use ngterm_host::{MainLoop, MainLoopClosed, MainTask};

/// A delayed task, ordered by its deadline, then by when it was posted.
struct Delayed<S> {
    due: Instant,
    serial: u64,
    task: MainTask<S>,
}

impl<S> PartialEq for Delayed<S> {
    fn eq(&self, other: &Self) -> bool {
        self.due == other.due && self.serial == other.serial
    }
}

impl<S> Eq for Delayed<S> {}

impl<S> PartialOrd for Delayed<S> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Reversed, so the heap's top is the earliest.
impl<S> Ord for Delayed<S> {
    fn cmp(&self, other: &Self) -> Ordering {
        other.due.cmp(&self.due).then_with(|| other.serial.cmp(&self.serial))
    }
}

struct Queue<S> {
    closed: bool,
    ready: VecDeque<MainTask<S>>,
    delayed: BinaryHeap<Delayed<S>>,
    serial: u64,
}

impl<S> Queue<S> {
    /// Moves every delayed task whose deadline passed by `now` into the ready queue, earliest first.
    fn promote(&mut self, now: Instant) {
        while self.delayed.peek().is_some_and(|next| next.due <= now) {
            if let Some(next) = self.delayed.pop() {
                self.ready.push_back(next.task);
            }
        }
    }
}

/// The main loop of the thread that runs it (module docs).
pub struct ThreadMainLoop<S> {
    queue: Mutex<Queue<S>>,
    wake: Condvar,
}

impl<S> Default for ThreadMainLoop<S> {
    fn default() -> Self {
        Self {
            queue: Mutex::new(Queue {
                closed: false,
                ready: VecDeque::new(),
                delayed: BinaryHeap::new(),
                serial: 0,
            }),
            wake: Condvar::new(),
        }
    }
}

impl<S> ThreadMainLoop<S> {
    fn lock(&self) -> MutexGuard<'_, Queue<S>> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Runs the tasks on this thread with `state`, calling `after` once each task returned, until the loop is closed
    /// (by a task, by `after`, or by another thread).
    pub fn run(&self, state: &mut S, mut after: impl FnMut(&mut S)) {
        while let Some(task) = self.next() {
            task(state);
            after(state);
        }
    }

    /// The next task to run, waiting for one; `None` once the loop is closed.
    fn next(&self) -> Option<MainTask<S>> {
        let mut queue = self.lock();
        loop {
            if queue.closed {
                return None;
            }
            let now = Instant::now();
            queue.promote(now);
            if let Some(task) = queue.ready.pop_front() {
                return Some(task);
            }
            queue = match queue.delayed.peek().map(|next| next.due.saturating_duration_since(now)) {
                Some(wait) => {
                    self.wake
                        .wait_timeout(queue, wait)
                        .unwrap_or_else(PoisonError::into_inner)
                        .0
                }
                None => self.wake.wait(queue).unwrap_or_else(PoisonError::into_inner),
            };
        }
    }

    /// Stops accepting tasks, drops the queued and the delayed ones unrun, and ends [`ThreadMainLoop::run`] once the
    /// task running now returns.
    pub fn close(&self) {
        let (ready, delayed) = {
            let mut queue = self.lock();
            queue.closed = true;
            (std::mem::take(&mut queue.ready), std::mem::take(&mut queue.delayed))
        };
        // dropped outside the lock: a task's captures may post (and fail) as they go
        drop((ready, delayed));
        self.wake.notify_all();
    }
}

impl<S> MainLoop<S> for ThreadMainLoop<S> {
    fn post(&self, task: MainTask<S>) -> Result<(), MainLoopClosed> {
        let mut queue = self.lock();
        if queue.closed {
            return Err(MainLoopClosed);
        }
        queue.ready.push_back(task);
        drop(queue);
        self.wake.notify_one();
        Ok(())
    }

    fn post_after(&self, delay: Duration, task: MainTask<S>) -> Result<(), MainLoopClosed> {
        let mut queue = self.lock();
        if queue.closed {
            return Err(MainLoopClosed);
        }
        queue.serial += 1;
        let serial = queue.serial;
        queue.delayed.push(Delayed {
            due: Instant::now() + delay,
            serial,
            task,
        });
        drop(queue);
        self.wake.notify_one();
        Ok(())
    }
}

#[cfg(test)]
mod tests;
