//! Timeouts for named-pipe calls, which Windows gives no per-call timeout: one process-wide timer thread cancels a
//! call still pending past its deadline (`CancelIoEx`), and the call then fails with [`io::ErrorKind::TimedOut`].
//!
//! interprocess issues every pipe read and write overlapped, waiting alertably for its completion, so a cancel from
//! another thread ends it. The timer cancels everything pending on the pipe, not one call; it keeps cancelling every
//! [`RETRY`] until the call returns, since a cancel that lands just before the call is issued finds nothing to cancel.

use std::io;
use std::ptr;
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::IO::CancelIoEx;

/// How often a call past its deadline is cancelled again until it returns.
const RETRY: Duration = Duration::from_millis(10);

/// Runs `call`, a read or write on the pipe `handle`, cancelling it once `timeout` passes; `None` waits for as long as
/// it takes. A call that fails after its deadline fails with [`io::ErrorKind::TimedOut`], whatever the cancel made of
/// it; one that finishes anyway keeps its result.
pub(super) fn timed<T>(
    handle: HANDLE,
    timeout: Option<Duration>,
    call: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    let Some(timeout) = timeout else {
        return call();
    };
    let timer = timer()?;
    let id = timer.arm(handle, Instant::now() + timeout);
    let result = call();
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
    due: Instant,
    fired: bool,
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

    fn arm(&self, handle: HANDLE, due: Instant) -> u64 {
        let mut armed = self.lock();
        let id = armed.next_id;
        armed.next_id += 1;
        armed.calls.push(Call {
            id,
            handle: handle.addr(),
            due,
            fired: false,
        });
        self.wake.notify_one();
        id
    }

    /// Forgets the call `id`; whether its deadline passed.
    fn disarm(&self, id: u64) -> bool {
        let mut armed = self.lock();
        let at = armed.calls.iter().position(|call| call.id == id);
        at.is_some_and(|at| armed.calls.swap_remove(at).fired)
    }

    fn run(&self) {
        let mut armed = self.lock();
        loop {
            let now = Instant::now();
            for call in armed.calls.iter_mut().filter(|call| call.due <= now) {
                // SAFETY: the handle is open: the call's thread borrows its pipe until `disarm`, which takes this
                // lock, so it cannot close while the cancel runs. A null overlapped pointer cancels everything
                // pending on it; nothing pending is no error worth reporting.
                unsafe { CancelIoEx(ptr::without_provenance_mut(call.handle), ptr::null()) };
                call.fired = true;
                call.due = now + RETRY;
            }
            let next = armed.calls.iter().map(|call| call.due).min();
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
