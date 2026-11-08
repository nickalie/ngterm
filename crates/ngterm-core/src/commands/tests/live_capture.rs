//! The quit capture of live panes (upstream's `AppDelegateCaptureTests` over `ZmxClient`'s capture listing, #918): it
//! lists the daemons' leaders once, within [`EXIT_LISTING_TIMEOUT`], and reads every live pane through that listing.

use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use ngterm_host::fakes::{FakeProcessTable, FakeSessionHost, FakeSurface};
use ngterm_host::{HostError, KillOutcome, LiveScreen, LiveSessionRecord, ProcessStat, SessionHost};
use uuid::Uuid;

use super::Fixture;
use super::live::{install, resources};
use crate::live::daemon_name;
use crate::model::StatusPane;
use crate::pane_env::uuid_string;
use crate::persist::RestoreMode;
use crate::{EXIT_CAPTURE_BUDGET, EXIT_LISTING_TIMEOUT};

/// The daemon's shell, the leader its listing reports.
const SHELL: i32 = 100;
/// The program that shell runs in the foreground.
const JOB: i32 = 300;

/// A host whose listings take `delay` and that records each: `None` for an unbounded [`SessionHost::list`], the
/// timeout for a [`SessionHost::list_within`].
struct SlowListHost {
    inner: Arc<FakeSessionHost>,
    delay: Duration,
    listings: Mutex<Vec<Option<Duration>>>,
}

impl SlowListHost {
    fn new(delay: Duration) -> Self {
        Self {
            inner: Arc::new(FakeSessionHost::new()),
            delay,
            listings: Mutex::new(Vec::new()),
        }
    }

    fn listings(&self) -> Vec<Option<Duration>> {
        self.listings.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn listed(&self, timeout: Option<Duration>) -> Result<Vec<LiveSessionRecord>, HostError> {
        self.listings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(timeout);
        thread::sleep(timeout.map_or(self.delay, |timeout| timeout.min(self.delay)));
        if timeout.is_some_and(|timeout| timeout < self.delay) {
            return Err(HostError("the listing outran its timeout".to_owned()));
        }
        self.inner.list()
    }
}

impl SessionHost for SlowListHost {
    fn is_available(&self) -> bool {
        self.inner.is_available()
    }

    fn list(&self) -> Result<Vec<LiveSessionRecord>, HostError> {
        self.listed(None)
    }

    fn list_within(&self, timeout: Duration) -> Result<Vec<LiveSessionRecord>, HostError> {
        self.listed(Some(timeout))
    }

    fn kill(&self, name: &str, force: bool) -> KillOutcome {
        self.inner.kill(name, force)
    }

    fn screen(&self, name: &str, all: bool) -> Result<LiveScreen, HostError> {
        self.inner.screen(name, all)
    }

    fn type_input(&self, name: &str, bytes: &[u8]) -> Result<(), HostError> {
        self.inner.type_input(name, bytes)
    }
}

/// A live launch whose first session's primary pane is a terminal on `host` (live when `live`), its daemon's shell
/// running `claude 41`, with `live` configured for the next launch so the quit captures.
fn quitting(host: &Arc<SlowListHost>, live: bool) -> (Fixture, Uuid, tempfile::TempDir) {
    let mut fixture = Fixture::new(0);
    let resources = resources();
    install(&mut fixture.core, host, resources.path(), RestoreMode::Live);
    fixture.core.force_restore_mode(RestoreMode::Live);
    let session = fixture.sessions[0];
    let left = fixture.store().session(session).unwrap().pane_identity;
    host.inner.set_record(&daemon_name(left), Some(1), Some(SHELL), None);
    let member = |pid, ppid| ProcessStat {
        pid,
        ppid,
        pgrp: pid,
        session: SHELL,
    };
    let table = FakeProcessTable::default();
    table.insert(member(SHELL, 1), Some(&["-zsh"]));
    table.insert(member(JOB, SHELL), Some(&["claude", "41"]));
    table.set_terminal_foreground(SHELL, JOB);
    fixture.core.set_process_table(Box::new(table), None);
    fixture
        .store_mut()
        .and_then(|store| store.session_mut(session))
        .unwrap()
        .pane_slot_mut(StatusPane::Left)
        .replace(Box::new(FakeSurface::new(uuid_string(left)).with_live_session(live)));
    (fixture, session, resources)
}

fn captured(fixture: &Fixture, session: Uuid) -> Option<Vec<String>> {
    fixture.store().session(session).unwrap().restore.foreground.clone()
}

fn claude() -> Vec<String> {
    vec!["claude".to_owned(), "41".to_owned()]
}

// #915: a listing slower than upstream's 100 ms timed out and left every reattached pane with no command
#[test]
fn an_exit_capture_reads_a_live_listing_slower_than_a_hundred_milliseconds() {
    let host = Arc::new(SlowListHost::new(Duration::from_millis(150)));
    let (mut fixture, session, _resources) = quitting(&host, true);
    fixture.core.capture_on_exit();
    assert_eq!(captured(&fixture, session), Some(claude()));
    assert_eq!(host.listings(), [Some(EXIT_LISTING_TIMEOUT)]);
}

#[test]
fn the_exit_capture_lists_once_within_its_budget_and_keeps_the_cached_leaders_past_it() {
    // a listing as slow as the client's own 3 s timeout
    let host = Arc::new(SlowListHost::new(Duration::from_secs(3)));
    let (mut fixture, session, _resources) = quitting(&host, true);
    let left = fixture.store().session(session).unwrap().pane_identity;
    let cached = [(daemon_name(left), SHELL)].into_iter().collect();
    let mut leaders = fixture.core.live.leaders.borrow_mut();
    leaders.accept(Some(cached));
    // a daemon came or went just before the quit: the next read would list again
    leaders.note_lifecycle_change();
    drop(leaders);
    fixture.core.capture_on_exit();
    assert_eq!(
        captured(&fixture, session),
        Some(claude()),
        "the leaders listed before the quit answer"
    );
    assert_eq!(
        host.listings(),
        [Some(EXIT_LISTING_TIMEOUT)],
        "no listing past the bounded one"
    );
    assert!(EXIT_LISTING_TIMEOUT < EXIT_CAPTURE_BUDGET);
}

#[test]
fn an_exit_capture_without_a_live_pane_lists_nothing() {
    let host = Arc::new(SlowListHost::new(Duration::ZERO));
    let (mut fixture, _session, _resources) = quitting(&host, false);
    fixture.core.capture_on_exit();
    assert_eq!(host.listings(), []);
}
