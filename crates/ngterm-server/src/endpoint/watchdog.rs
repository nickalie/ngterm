//! Timeouts for named-pipe calls, which Windows gives no per-call timeout: one process-wide timer thread cancels a
//! call still pending past its deadline (`CancelIoEx`), and the call then fails with [`io::ErrorKind::TimedOut`].
//!
//! A connection issues every pipe read and write overlapped and waits for its completion, so a cancel from another
//! thread ends it. The timer cancels that one call, through its own `OVERLAPPED`: a write that times out leaves a read
//! pending on the same pipe alone, and the other way round. A call is armed only once it is pending, so the one
//! cancel finds it.

use std::io;
use std::ptr;
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::IO::{CancelIoEx, OVERLAPPED};

/// A call's timeout with the timer that enforces it, got before the call is issued: once a call is pending, nothing
/// may fail before its wait.
#[derive(Clone, Copy)]
pub(super) struct Deadline {
    timer: &'static Timer,
    timeout: Duration,
}

/// The deadline for a call bounded by `timeout`; `None` for an unbounded one.
///
/// # Errors
///
/// The timer thread could not be started.
pub(super) fn deadline(timeout: Option<Duration>) -> io::Result<Option<Deadline>> {
    timeout
        .map(|timeout| {
            Ok(Deadline {
                timer: timer()?,
                timeout,
            })
        })
        .transpose()
}

/// Waits for the call pending on the pipe `handle` through `overlapped` (`wait`), cancelling it once its `deadline`
/// passes; `None` waits for as long as it takes. A call that fails after its deadline fails with
/// [`io::ErrorKind::TimedOut`], whatever the cancel made of it; one that finishes anyway keeps its result.
///
/// `overlapped` must stay where it is until this returns, and `wait` must not return before the call is done.
pub(super) fn timed<T>(
    handle: HANDLE,
    overlapped: *const OVERLAPPED,
    deadline: Option<Deadline>,
    wait: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    let Some(Deadline { timer, timeout }) = deadline else {
        return wait();
    };
    let id = timer.arm(handle, overlapped, Instant::now() + timeout);
    let result = wait();
    let fired = timer.disarm(id);
    match result {
        Err(error) if fired => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("named pipe call timed out after {timeout:?} ({error})"),
        )),
        result => result,
    }
}

/// The calls with a deadline, and the condition the timer thread waits on for a new one.
struct Timer {
    armed: Mutex<Armed>,
    wake: Condvar,
}

#[derive(Default)]
struct Armed {
    next_id: u64,
    calls: Vec<Call>,
}

struct Call {
    id: u64,
    /// The pipe's handle as an address: a handle is a number, not a pointer into memory.
    handle: usize,
    /// The call's `OVERLAPPED`, which names it to `CancelIoEx`; only its address is used.
    overlapped: usize,
    /// When to cancel it; `None` once it was.
    due: Option<Instant>,
}

fn timer() -> io::Result<&'static Timer> {
    static TIMER: OnceLock<Option<&'static Timer>> = OnceLock::new();
    TIMER
        .get_or_init(|| {
            let timer: &'static Timer = Box::leak(Box::new(Timer {
                armed: Mutex::new(Armed::default()),
                wake: Condvar::new(),
            }));
            thread::Builder::new()
                .name("ngterm-control-pipe-timer".to_owned())
                .spawn(move || timer.run())
                .inspect_err(|error| tracing::error!(%error, "no thread for the control pipe's timeouts"))
                .ok()
                .map(|_| timer)
        })
        .ok_or_else(|| io::Error::other("no thread for the control pipe's timeouts"))
}

impl Timer {
    fn lock(&self) -> MutexGuard<'_, Armed> {
        self.armed.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn arm(&self, handle: HANDLE, overlapped: *const OVERLAPPED, due: Instant) -> u64 {
        let mut armed = self.lock();
        let id = armed.next_id;
        armed.next_id += 1;
        armed.calls.push(Call {
            id,
            handle: handle.addr(),
            overlapped: overlapped.addr(),
            due: Some(due),
        });
        self.wake.notify_one();
        id
    }

    /// Forgets the call `id`; whether its deadline passed.
    fn disarm(&self, id: u64) -> bool {
        let mut armed = self.lock();
        let at = armed.calls.iter().position(|call| call.id == id);
        at.is_some_and(|at| armed.calls.swap_remove(at).due.is_none())
    }

    fn run(&self) {
        let mut armed = self.lock();
        loop {
            let now = Instant::now();
            for call in armed
                .calls
                .iter_mut()
                .filter(|call| call.due.is_some_and(|due| due <= now))
            {
                // SAFETY: the handle is open and the OVERLAPPED in place: the call's thread borrows its pipe and keeps
                // its OVERLAPPED until `disarm`, which takes this lock, so neither goes while the cancel runs. A call
                // that finished meanwhile is no error worth reporting.
                unsafe {
                    CancelIoEx(
                        ptr::without_provenance_mut(call.handle),
                        ptr::without_provenance(call.overlapped),
                    )
                };
                call.due = None;
            }
            let next = armed.calls.iter().filter_map(|call| call.due).min();
            armed = match next {
                Some(due) => {
                    let wait = due.saturating_duration_since(Instant::now());
                    self.wake
                        .wait_timeout(armed, wait)
                        .map_or_else(|poisoned| poisoned.into_inner().0, |(armed, _)| armed)
                }
                None => self.wake.wait(armed).unwrap_or_else(PoisonError::into_inner),
            };
        }
    }
}
