//! The server's scratch (upstream's `ServerActions+Scratch.swift` and `ServerPanes.swift` scratch parts): on
//! `ngterm-headless` a session's scratch is a live session of its own name shape ([`scratch_daemon_name`]), which
//! nothing that adopts, maps or prunes pane daemons claims. It outlives a hide and a viewer's detach, and ends with
//! its own exit, its session's close, a `session scratch --command` that replaces it, or the next server start.

use ngterm_host::KillOutcome;
use uuid::Uuid;

use crate::Core;
use crate::live::support::{Creation, attach_line, live_attach, settled};
use crate::live::{is_scratch_daemon_name, scratch_daemon_name};
use crate::restore::spawn_plan::LaunchSeed;

impl Core {
    /// Whether a shown scratch runs as a live session of its own: on a server, which serves the catalog, where this
    /// instance wraps panes.
    #[must_use]
    pub(crate) fn scratch_runs_live(&self) -> bool {
        self.serves_catalog() && self.live.host().is_some() && self.live.wrap.is_some()
    }

    /// The seed of the server scratch `identity`: an attach that creates its live session, running `command` once in
    /// place of the login shell. `None` (logged) when this instance cannot wrap a pane, so the scratch runs as an
    /// ordinary terminal.
    #[must_use]
    pub fn scratch_seed(&self, identity: Uuid, command: Option<&str>) -> Option<LaunchSeed> {
        let mut configuration = self.wrap_configuration(identity, None)?;
        configuration.daemon_name = scratch_daemon_name(identity);
        let attach = live_attach(&configuration, None, command.map(Creation::EndingPane), &[] as &[&str]);
        Some(LaunchSeed {
            command: Some(settled(&attach_line(&configuration, &attach))),
            env: configuration.environment.clone().into_iter().collect(),
            live: true,
            attach: configuration.in_process(attach),
            ..LaunchSeed::default()
        })
    }

    /// Ends the live session of the server scratch `identity`, which its session's close or a replacement dropped.
    /// Whether the session host ended it; a failed kill is logged, and the next server start ends the daemon.
    pub fn end_scratch(&self, identity: Uuid) -> bool {
        let Some(host) = self.live.host() else {
            return false;
        };
        let name = scratch_daemon_name(identity);
        match host.kill(&name, true) {
            KillOutcome::Killed | KillOutcome::StaleSocket => true,
            outcome @ KillOutcome::Failed(_) => {
                tracing::warn!(name, ?outcome, "could not end the scratch's live session");
                false
            }
        }
    }

    /// Ends every listed scratch daemon (upstream's `endUnownedScratches` at a start): a scratch never outlives the
    /// server that started it, so at a start every one is a previous run's. The names it ended; none when the listing
    /// failed.
    pub fn end_previous_scratches(&self) -> Vec<String> {
        let Some(host) = self.live.host() else {
            return Vec::new();
        };
        let records = match host.list() {
            Ok(records) => records,
            Err(error) => {
                tracing::warn!(%error, "could not list the live sessions to end the scratches of a previous run");
                return Vec::new();
            }
        };
        records
            .into_iter()
            .map(|record| record.name)
            .filter(|name| is_scratch_daemon_name(name))
            .filter(|name| match host.kill(name, true) {
                KillOutcome::Killed | KillOutcome::StaleSocket => true,
                outcome @ KillOutcome::Failed(_) => {
                    tracing::warn!(name, ?outcome, "could not end a previous run's scratch");
                    false
                }
            })
            .collect()
    }
}
