//! A HUD panel's IO: the painter ([`ngterm_core::hud::painter`], upstream's `hud.sh`) run in process on a thread of
//! the pane, in place of a PTY and a child, on every OS alike. The thread re-reads the body file every tick, hands
//! each changed frame to the pane's mailbox as a PTY's output, and waits out the tick's interval; the same IO worker
//! feeds it to the VT.
//!
//! - **End**: the body file gone (how every teardown path stops the painter, upstream's contract) ends the pane as a
//!   program's exit with status 0 would, after the script's exit sequence (the cursor shown again, the attributes
//!   reset). Dropping the pane stops the thread without an exit report.
//! - **Resize**: the terminal reflows what is on screen, so the next frame is painted even when it did not change (the
//!   script's `WINCH` trap); the painter is woken for it at once. The grid itself is never measured here: it rides
//!   the body's header, where the app put the size it gave the panel.
//! - **Input and replies**: nothing reads them. The pane's writes are dropped and its terminal's replies are closed
//!   off, as a painter that never reads its input loses them.

use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use ngterm_core::hud::painter::{END, Painter, START};

use crate::pane::{Shared, lock};

/// What the frontend asks of the painter's thread.
#[derive(Debug, Default, Clone, Copy)]
struct Signal {
    /// The pane goes away.
    stop: bool,
    /// The terminal was resized since the last tick.
    resized: bool,
}

/// The painter's side of a painted pane: its file and the signals its thread waits on.
#[derive(Debug)]
pub(crate) struct Painted {
    file: PathBuf,
    signal: Mutex<Signal>,
    changed: Condvar,
}

impl Painted {
    pub(crate) fn new(file: PathBuf) -> Self {
        Self {
            file,
            signal: Mutex::new(Signal::default()),
            changed: Condvar::new(),
        }
    }

    fn post(&self, change: impl FnOnce(&mut Signal)) {
        change(&mut lock(&self.signal));
        self.changed.notify_all();
    }

    /// The pane goes away: the thread stops at once, reporting nothing.
    pub(crate) fn stop(&self) {
        self.post(|signal| signal.stop = true);
    }

    /// The terminal was resized: the thread paints again now.
    pub(crate) fn resized(&self) {
        self.post(|signal| signal.resized = true);
    }

    /// Waits out `interval` or the next signal, and takes the signals.
    fn wait(&self, interval: Duration) -> Signal {
        let guard = lock(&self.signal);
        let (mut guard, _) = self
            .changed
            .wait_timeout_while(guard, interval, |signal| !signal.stop && !signal.resized)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let taken = *guard;
        guard.resized = false;
        taken
    }

    /// The painter's thread: a tick per interval while the body file is there, then the pane's end. A file that is
    /// there but cannot be read paints the built-in box, as the script's failed read did.
    pub(crate) fn run(&self, shared: &Shared) {
        let mailbox = shared.mailbox();
        mailbox.deliver(START.as_bytes());
        let mut painter = Painter::new();
        while is_file(&self.file) {
            let body = match std::fs::read(&self.file) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                read => read.unwrap_or_default(),
            };
            let tick = painter.tick(&String::from_utf8_lossy(&body));
            if let Some(output) = tick.output {
                mailbox.deliver(output.as_bytes());
            }
            let signal = self.wait(tick.interval);
            if signal.stop {
                return;
            }
            if signal.resized {
                painter.forget();
            }
        }
        mailbox.deliver(END.as_bytes());
        mailbox.end_of_output();
        mailbox.child_exited();
        crate::pane::report_exit(shared, Some(0), None);
    }
}

fn is_file(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file())
}

#[cfg(test)]
mod tests;
