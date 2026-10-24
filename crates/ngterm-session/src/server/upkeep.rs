//! The server's upkeep of its namespace's files: at its start and once per [`UPKEEP_INTERVAL`] after it, the
//! coordinator moves the lock file's times to now ([`crate::endpoint::Listener::keep_current`]), so a temp-dir cleaner
//! that deletes files untouched for days does not take the lock from a server that runs that long.
//!
//! ngterm's addition, after upstream agterm's #733 (`Host.maintain` and `Host.timeIncludingSleep` in
//! `agtermCore/Sources/SessionHostRuntime/Host.swift` at v0.38.1): the schedule runs on a clock that keeps counting
//! while the machine sleeps ([`time_including_sleep`]), since the cleaners age a file by wall time, and a machine that
//! sleeps most of the day would otherwise skip the upkeep for longer than they wait. `std`'s `Instant` does not count
//! sleep on Linux or macOS, and neither do the timed waits `std` offers (a condvar's), so the coordinator never waits
//! longer than [`UPKEEP_POLL`] before it reads the clock again ([`Upkeep::wait`]): an upkeep a sleep made due runs
//! within that slice of the machine waking, where a wait for the whole interval would still have up to an interval of
//! awake time to run. Upstream's host polls every 100 ms and checks on each pass to the same end.

use std::time::Duration;

/// How often the server keeps its namespace's files current: well inside the shortest cleaner's age (macOS, three
/// days).
pub const UPKEEP_INTERVAL: Duration = Duration::from_hours(1);

/// The longest the coordinator waits before it reads [`time_including_sleep`] again, whatever the next upkeep's due
/// time: the bound on how late an upkeep runs after the machine wakes from a sleep that made it due.
pub const UPKEEP_POLL: Duration = Duration::from_mins(1);

/// When the next upkeep is due, on the clock of [`time_including_sleep`] (or a test's), and how long a wait for it may
/// last before that clock is read again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upkeep {
    interval: Duration,
    poll: Duration,
    next: Option<Duration>,
}

impl Upkeep {
    /// A schedule whose first upkeep is due at once, then once per `interval`, waited for in slices of at most `poll`.
    #[must_use]
    pub const fn new(interval: Duration, poll: Duration) -> Self {
        Self {
            interval,
            poll,
            next: None,
        }
    }

    /// Whether an upkeep is due at `now`; when it is, the next one is scheduled one interval from `now`.
    pub fn due(&mut self, now: Duration) -> bool {
        if self.next.is_some_and(|next| now < next) {
            return false;
        }
        self.next = Some(now.saturating_add(self.interval));
        true
    }

    /// How long from `now` until the next upkeep is due: zero when it is due already.
    #[must_use]
    pub fn remaining(&self, now: Duration) -> Duration {
        self.next.map_or(Duration::ZERO, |next| next.saturating_sub(now))
    }

    /// How long a wait from `now` may last before the clock is read again: until the next upkeep, but no longer than
    /// the poll slice, since the wait itself runs on a clock that stops while the machine sleeps.
    #[must_use]
    pub fn wait(&self, now: Duration) -> Duration {
        self.remaining(now).min(self.poll)
    }
}

/// The time on a clock that keeps counting while the machine sleeps, from an unspecified start: `CLOCK_BOOTTIME` on
/// Linux and Android, `CLOCK_MONOTONIC` elsewhere on Unix (it counts sleep on macOS, and is `CLOCK_BOOTTIME`'s alias
/// on FreeBSD). On Windows `Instant` (`QueryPerformanceCounter`), which paces only a no-op there
/// ([`crate::endpoint::Listener::keep_current`]).
#[must_use]
pub fn time_including_sleep() -> Duration {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    let clock = rustix::time::ClockId::Boottime;
    #[cfg(all(unix, not(any(target_os = "linux", target_os = "android"))))]
    let clock = rustix::time::ClockId::Monotonic;
    #[cfg(unix)]
    {
        let time = rustix::time::clock_gettime(clock);
        Duration::new(
            u64::try_from(time.tv_sec).unwrap_or(0),
            u32::try_from(time.tv_nsec).unwrap_or(0),
        )
    }
    #[cfg(windows)]
    {
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        START.get_or_init(std::time::Instant::now).elapsed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_upkeep_is_due_at_once_then_once_per_interval() {
        let mut upkeep = Upkeep::new(UPKEEP_INTERVAL, UPKEEP_POLL);
        let start = Duration::from_secs(1_000);
        assert_eq!(upkeep.remaining(start), Duration::ZERO);
        assert!(upkeep.due(start));
        assert_eq!(upkeep.remaining(start), UPKEEP_INTERVAL);

        let inside = start + UPKEEP_INTERVAL.saturating_sub(Duration::from_secs(1));
        assert!(!upkeep.due(inside));
        assert_eq!(upkeep.remaining(inside), Duration::from_secs(1));

        let past = start + UPKEEP_INTERVAL;
        assert_eq!(upkeep.remaining(past), Duration::ZERO);
        assert!(upkeep.due(past));
        assert!(!upkeep.due(past));
        assert_eq!(upkeep.remaining(past), UPKEEP_INTERVAL);
    }

    /// A machine that slept through several intervals gets one upkeep on waking, and the next one an interval later.
    #[test]
    fn a_long_gap_gives_one_upkeep_and_a_fresh_interval() {
        let mut upkeep = Upkeep::new(UPKEEP_INTERVAL, UPKEEP_POLL);
        assert!(upkeep.due(Duration::ZERO));
        let woken = UPKEEP_INTERVAL * 30;
        assert!(upkeep.due(woken));
        assert!(!upkeep.due(woken + UPKEEP_INTERVAL / 2));
        assert!(upkeep.due(woken + UPKEEP_INTERVAL));
    }

    /// A wait never outlasts the poll slice, so a sleep that made the upkeep due is seen within a slice of waking; it
    /// ends sooner when the upkeep falls due sooner.
    #[test]
    fn a_wait_is_capped_at_the_poll_slice() {
        let mut upkeep = Upkeep::new(UPKEEP_INTERVAL, UPKEEP_POLL);
        let start = Duration::from_secs(1_000);
        assert_eq!(upkeep.wait(start), Duration::ZERO, "due at once");
        assert!(upkeep.due(start));
        assert_eq!(upkeep.wait(start), UPKEEP_POLL);
        let close = start + UPKEEP_INTERVAL.saturating_sub(Duration::from_secs(5));
        assert_eq!(upkeep.wait(close), Duration::from_secs(5));
    }

    #[test]
    fn the_clock_moves_forward() {
        let first = time_including_sleep();
        std::thread::sleep(Duration::from_millis(5));
        assert!(time_including_sleep() > first);
    }
}
