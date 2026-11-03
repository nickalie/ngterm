//! Keeping a machine connected: what the actor does when a connection goes away by itself, and around the app's
//! suspend and resume.
//!
//! - **A lost connection** ([`crate::transport::Connection::lost`] while it is still the machine's) is retried on the
//!   schedule every remote link shares ([`retry_delay`]): one second after the loss, doubling to thirty seconds and
//!   later five minutes, each wait reported as [`crate::MachineState::Reconnecting`]. A retry that fails waits the
//!   next delay; one that succeeds reports `Connected`. The machine's attached sessions stay attached meanwhile:
//!   their streams fail on their own backoff (a stream launched while the machine is down closes at once), and once
//!   it is back each gets `retry_now` and reopens.
//! - **`network_changed`** ends every reconnect's wait: the retry runs at once and its backoff starts over. A connect
//!   of a machine being reconnected does the same and waits for that retry.
//! - **`suspend`** closes every machine's connection and stops its streams without reporting an error state, and
//!   cancels the connects in flight; **`resume`** reconnects each machine it set aside, at once and on the reconnect
//!   schedule, and starts its streams again once it is back.
//!
//! A disconnect, or a connect of the machine as edited, ends all of it: its streams and its retries go.
//!
//! [`retry_delay`]: ngterm_core::remote::reconnect::retry_delay

use std::time::Duration;

use ngterm_core::remote::reconnect::retry_delay;

/// Where a reconnect is in its schedule: how many tries failed in a row before the one it waits for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Retry {
    failures: u32,
}

impl Retry {
    /// The first retry after a lost connection, which counts as the first failure.
    pub(crate) fn after_loss() -> Self {
        Self { failures: 1 }
    }

    /// A retry with its backoff over: it runs at once, and a failure starts the schedule over.
    pub(crate) fn at_once() -> Self {
        Self { failures: 0 }
    }

    /// The retry after this one failed.
    #[must_use]
    pub(crate) fn failed(self) -> Self {
        Self {
            failures: self.failures.saturating_add(1),
        }
    }

    /// How long this retry waits before it runs.
    pub(crate) fn delay(self) -> Duration {
        if self.failures == 0 {
            Duration::ZERO
        } else {
            retry_delay(self.failures)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lost_connection_is_retried_after_a_second_and_the_wait_doubles_with_each_failure() {
        let mut retry = Retry::after_loss();
        let mut delays = Vec::new();
        for _ in 0..7 {
            delays.push(retry.delay().as_secs());
            retry = retry.failed();
        }
        assert_eq!(delays, [1, 2, 4, 8, 16, 30, 30]);
    }

    #[test]
    fn the_wait_follows_the_shared_schedule_to_five_minutes() {
        let mut retry = Retry::after_loss();
        for failures in 1..=12 {
            assert_eq!(retry.delay(), retry_delay(failures));
            retry = retry.failed();
        }
        assert_eq!(retry.delay(), Duration::from_secs(300));
    }

    #[test]
    fn a_retry_at_once_waits_nothing_and_its_failure_starts_the_schedule_over() {
        assert_eq!(Retry::at_once().delay(), Duration::ZERO);
        assert_eq!(Retry::at_once().failed().delay(), Duration::from_secs(1));
    }
}
