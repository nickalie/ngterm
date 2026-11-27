//! How long every ssh this app starts may take to reach a machine (ssh's `ConnectTimeout`), as `remotes.conf` sets it:
//! a file-wide `connect-timeout = SECONDS` line and `connect-timeout DESTINATION = SECONDS` lines for one machine.
//! The table is process-wide, like [`super::origin`]'s, since the ssh lines are built away from the `Core`; each read of
//! `remotes.conf` replaces it, so an edit reaches the next ssh call.
//!
//! A wall-clock deadline around an ssh call is the connect timeout plus the far side's own time ([`after_connect`]),
//! so a slow handshake is never read as a far side that did not answer.

use std::collections::BTreeMap;
use std::time::Duration;

/// The connect timeout of a machine `remotes.conf` sets none for, in seconds.
pub const DEFAULT_SECONDS: u32 = 10;
/// The longest connect timeout `remotes.conf` may set, in seconds.
pub const MAX_SECONDS: u32 = 300;
/// The `remotes.conf` setting's name.
pub const SETTING: &str = "connect-timeout";

/// The connect timeouts `remotes.conf` sets, in seconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectTimeouts {
    default: u32,
    /// By ssh destination: a peer pin names an app there, not another machine.
    machines: BTreeMap<String, u32>,
}

impl Default for ConnectTimeouts {
    fn default() -> Self {
        Self {
            default: DEFAULT_SECONDS,
            machines: BTreeMap::new(),
        }
    }
}

impl ConnectTimeouts {
    /// Sets the file-wide value.
    pub fn set_default(&mut self, seconds: u32) {
        self.default = seconds;
    }

    /// Sets `destination`'s own value.
    pub fn set_for(&mut self, destination: &str, seconds: u32) {
        self.machines
            .insert(super::peer::ssh_destination(destination).to_owned(), seconds);
    }

    /// Whether `destination` has its own value.
    #[must_use]
    pub fn has_own(&self, destination: &str) -> bool {
        self.machines.contains_key(super::peer::ssh_destination(destination))
    }

    /// The seconds an ssh to `host` may take to connect: its own value, else the file-wide one.
    #[must_use]
    pub fn seconds_for(&self, host: &str) -> u32 {
        self.machines
            .get(super::peer::ssh_destination(host))
            .copied()
            .unwrap_or(self.default)
    }
}

/// Reads a setting's value: whole seconds from 1 to [`MAX_SECONDS`].
///
/// # Errors
///
/// The value's diagnostic, which names it.
pub fn parse_seconds(value: &str) -> Result<u32, String> {
    match value.parse::<u32>() {
        Ok(seconds) if (1..=MAX_SECONDS).contains(&seconds) => Ok(seconds),
        _ => Err(format!(
            "invalid {SETTING} '{value}' (whole seconds, 1 to {MAX_SECONDS}); the setting is skipped"
        )),
    }
}

#[cfg(not(test))]
static INSTALLED: std::sync::LazyLock<std::sync::RwLock<ConnectTimeouts>> =
    std::sync::LazyLock::new(std::sync::RwLock::default);

/// Replaces the table every ssh line reads from.
#[cfg(not(test))]
pub fn install(timeouts: ConnectTimeouts) {
    *INSTALLED.write().unwrap_or_else(std::sync::PoisonError::into_inner) = timeouts;
}

/// The seconds an ssh to `host` may take to connect, as `remotes.conf` last set them.
#[cfg(not(test))]
#[must_use]
pub fn seconds(host: &str) -> u32 {
    INSTALLED
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .seconds_for(host)
}

// Unit tests run in parallel in one process: each test thread keeps its own table, so one test's file never reaches
// another test's ssh lines.
#[cfg(test)]
thread_local! {
    static INSTALLED: std::cell::RefCell<ConnectTimeouts> = std::cell::RefCell::default();
}

/// Replaces the table every ssh line reads from.
#[cfg(test)]
pub fn install(timeouts: ConnectTimeouts) {
    INSTALLED.with(|installed| *installed.borrow_mut() = timeouts);
}

/// The seconds an ssh to `host` may take to connect, as `remotes.conf` last set them.
#[cfg(test)]
#[must_use]
pub fn seconds(host: &str) -> u32 {
    INSTALLED.with(|installed| installed.borrow().seconds_for(host))
}

/// The table a thread that builds ssh lines for the calling thread reads: the process-wide one, or in unit tests the
/// calling thread's.
pub(crate) struct Inherited(#[cfg(test)] ConnectTimeouts);

/// The calling thread's [`Inherited`] table, for a thread it starts.
pub(crate) fn inherited() -> Inherited {
    #[cfg(test)]
    return Inherited(INSTALLED.with(|installed| installed.borrow().clone()));
    #[cfg(not(test))]
    Inherited()
}

impl Inherited {
    /// Takes the table on the started thread.
    #[cfg_attr(
        not(test),
        expect(
            clippy::unused_self,
            reason = "outside unit tests every thread reads the one process-wide table"
        )
    )]
    pub(crate) fn adopt(self) {
        #[cfg(test)]
        install(self.0);
    }
}

/// A deadline for an ssh call to `host` whose far side needs up to `work` once connected: the connect timeout plus
/// `work`.
#[must_use]
pub fn after_connect(host: &str, work: Duration) -> Duration {
    Duration::from_secs(seconds(host).into()) + work
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_machine_value_wins_over_the_file_value_and_a_pin_shares_its_machine() {
        let mut timeouts = ConnectTimeouts::default();
        assert_eq!(timeouts.seconds_for("studio"), DEFAULT_SECONDS);
        timeouts.set_default(20);
        timeouts.set_for("agterm:studio", 45);
        assert_eq!(timeouts.seconds_for("studio"), 45);
        assert_eq!(timeouts.seconds_for("ngterm:studio"), 45);
        assert_eq!(timeouts.seconds_for("mini"), 20);
        assert!(timeouts.has_own("studio"));
        assert!(!timeouts.has_own("mini"));
    }

    #[test]
    fn a_value_is_whole_seconds_within_the_bounds() {
        assert_eq!(parse_seconds("1"), Ok(1));
        assert_eq!(parse_seconds("300"), Ok(300));
        for bad in ["0", "301", "-5", "2.5", "ten", ""] {
            let error = parse_seconds(bad).unwrap_err();
            assert!(error.contains(&format!("'{bad}'")), "{error}");
        }
    }

    #[test]
    fn the_installed_table_sets_the_deadlines() {
        install(ConnectTimeouts::default());
        assert_eq!(after_connect("studio", Duration::from_secs(5)), Duration::from_secs(15));
        let mut timeouts = ConnectTimeouts::default();
        timeouts.set_for("studio", 30);
        install(timeouts);
        assert_eq!(seconds("studio"), 30);
        assert_eq!(after_connect("studio", Duration::from_secs(5)), Duration::from_secs(35));
        assert_eq!(seconds("mini"), DEFAULT_SECONDS);
        install(ConnectTimeouts::default());
    }
}
