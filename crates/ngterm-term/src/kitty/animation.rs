//! Ghostty `src/renderer/generic.zig` animation clock and wake policy at `LIBGHOSTTY_VT_REV`.
//! One monotonic epoch belongs to the terminal, shared by every view. The mailbox consumes each wake once;
//! the next capture ticks and rearms it. Without a capture there is no repeated wake or idle polling.

use std::time::{Duration, Instant};

use ngterm_ghostty_vt as vt;

const DRAW_INTERVAL: Duration = Duration::from_millis(8);

#[derive(Debug)]
pub(crate) struct Animation {
    epoch: Instant,
    next: Option<(u64, Instant)>,
}

impl Animation {
    pub(crate) fn new(epoch: Instant) -> Self {
        Self { epoch, next: None }
    }

    /// Called under the VT lock before `begin_update`; a held frame is never ticked.
    pub(crate) fn tick(
        &mut self,
        terminal: &mut vt::Terminal,
        now: Instant,
        held: bool,
    ) -> Result<Option<Instant>, vt::Error> {
        if held {
            self.next = None;
            return Ok(None);
        }
        let now_ms = u64::try_from(now.duration_since(self.epoch).as_millis()).unwrap_or(u64::MAX);
        let next_ms = terminal
            .kitty_graphics()
            .map(|mut graphics| graphics.animation_tick(now_ms))
            .transpose()?
            .flatten();
        Ok(self.schedule(now, next_ms))
    }

    fn schedule(&mut self, now: Instant, next_ms: Option<u64>) -> Option<Instant> {
        let Some(next_ms) = next_ms else {
            self.next = None;
            return None;
        };
        // Repainting another view cannot push the same deadline into the future, including an overdue one.
        if let Some((previous, deadline)) = self.next
            && previous == next_ms
        {
            return Some(deadline);
        }
        let deadline = self
            .epoch
            .checked_add(Duration::from_millis(next_ms))
            .unwrap_or(now + DRAW_INTERVAL)
            .max(now + DRAW_INTERVAL);
        self.next = Some((next_ms, deadline));
        Some(deadline)
    }
}

#[cfg(test)]
mod tests;
