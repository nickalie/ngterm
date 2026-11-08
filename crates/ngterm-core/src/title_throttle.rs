//! The OS window title's writes, coalesced (upstream's `WindowAccessor.setTitle`, #817). Each write runs the
//! platform's title machinery, so an animated OSC title lands at most once per interval: the first change after a
//! quiet interval writes at once, later ones within it collapse into one trailing write that carries the latest
//! title, and a title equal to the one the window shows writes nothing. A write that found nothing to change does
//! not start an interval. The clock is the caller's, so the interval is testable without real time.

use std::time::{Duration, Instant};

/// The shortest time between two OS title writes (upstream's `titleWriteInterval`).
pub const TITLE_WRITE_INTERVAL: Duration = Duration::from_millis(500);

/// What the caller does with a title it offered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TitleStep {
    /// Write this title to the window now.
    Write(String),
    /// Call [`TitleThrottle::due`] at this instant: it carries the latest title then.
    Wait(Instant),
    /// Nothing to do: the window shows the title, or a trailing write already waits.
    Idle,
}

/// One window's title writes.
#[derive(Debug, Clone)]
pub struct TitleThrottle {
    interval: Duration,
    /// What the window shows.
    written: String,
    /// The title offered last.
    latest: String,
    /// When the last write that changed the title happened.
    last_write: Option<Instant>,
    /// A trailing write waits.
    pending: bool,
}

impl TitleThrottle {
    /// A window showing `shown`, writing at most once per `interval`.
    #[must_use]
    pub fn new(interval: Duration, shown: &str) -> Self {
        Self {
            interval,
            written: shown.to_owned(),
            latest: shown.to_owned(),
            last_write: None,
            pending: false,
        }
    }

    /// The title the window shows.
    #[must_use]
    pub fn written(&self) -> &str {
        &self.written
    }

    /// `title` is the window's title as of `now`.
    pub fn offer(&mut self, title: &str, now: Instant) -> TitleStep {
        if self.latest != title {
            title.clone_into(&mut self.latest);
        }
        if self.pending || self.written == self.latest {
            return TitleStep::Idle;
        }
        match self.last_write {
            Some(last) if now.saturating_duration_since(last) < self.interval => {
                self.pending = true;
                TitleStep::Wait(last + self.interval)
            }
            _ => self.write(now).map_or(TitleStep::Idle, TitleStep::Write),
        }
    }

    /// The trailing write fell due at `now`: the latest title to write, `None` when the window already shows it.
    pub fn due(&mut self, now: Instant) -> Option<String> {
        self.pending = false;
        self.write(now)
    }

    fn write(&mut self, now: Instant) -> Option<String> {
        if self.written == self.latest {
            return None;
        }
        self.last_write = Some(now);
        self.written.clone_from(&self.latest);
        Some(self.written.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    fn write(title: &str) -> TitleStep {
        TitleStep::Write(title.to_owned())
    }

    /// Upstream's `testRapidTitleChangesAreCoalescedIntoOneTrailingWrite`.
    #[test]
    fn rapid_title_changes_are_coalesced_into_one_trailing_write() {
        let mut throttle = TitleThrottle::new(ms(500), "");
        let t0 = Instant::now();
        assert_eq!(
            throttle.offer("a", t0),
            write("a"),
            "the first change after a quiet interval lands at once"
        );
        assert_eq!(throttle.offer("b", t0 + ms(10)), TitleStep::Wait(t0 + ms(500)));
        assert_eq!(throttle.offer("c", t0 + ms(20)), TitleStep::Idle, "one trailing write");
        assert_eq!(throttle.written(), "a", "changes within the interval wait");
        assert_eq!(
            throttle.due(t0 + ms(500)).as_deref(),
            Some("c"),
            "the trailing write carries the latest title"
        );
        assert_eq!(throttle.written(), "c");
    }

    #[test]
    fn an_unchanged_title_writes_nothing() {
        let mut throttle = TitleThrottle::new(ms(500), "shown");
        let t0 = Instant::now();
        assert_eq!(throttle.offer("shown", t0), TitleStep::Idle);
        assert_eq!(throttle.offer("next", t0 + ms(1)), write("next"));
        assert_eq!(throttle.offer("next", t0 + ms(2)), TitleStep::Idle);
        assert_eq!(throttle.offer("next", t0 + ms(900)), TitleStep::Idle);
    }

    #[test]
    fn a_trailing_write_back_to_the_shown_title_writes_nothing_and_does_not_settle_the_window() {
        let mut throttle = TitleThrottle::new(ms(500), "");
        let t0 = Instant::now();
        assert_eq!(throttle.offer("a", t0), write("a"));
        assert_eq!(throttle.offer("b", t0 + ms(100)), TitleStep::Wait(t0 + ms(500)));
        assert_eq!(throttle.offer("a", t0 + ms(200)), TitleStep::Idle);
        assert_eq!(throttle.due(t0 + ms(500)), None, "the window already shows it");
        // the interval still runs from the write at t0, so the next change lands at once
        assert_eq!(throttle.offer("d", t0 + ms(600)), write("d"));
    }

    #[test]
    fn a_change_after_a_quiet_interval_writes_at_once_again() {
        let mut throttle = TitleThrottle::new(ms(500), "");
        let t0 = Instant::now();
        assert_eq!(throttle.offer("a", t0), write("a"));
        assert_eq!(throttle.offer("b", t0 + ms(499)), TitleStep::Wait(t0 + ms(500)));
        assert_eq!(throttle.due(t0 + ms(500)).as_deref(), Some("b"));
        assert_eq!(
            throttle.offer("c", t0 + ms(700)),
            TitleStep::Wait(t0 + ms(1000)),
            "the interval runs from the trailing write"
        );
        assert_eq!(throttle.due(t0 + ms(1000)).as_deref(), Some("c"));
        assert_eq!(throttle.offer("d", t0 + ms(1500)), write("d"), "at the boundary");
    }
}
