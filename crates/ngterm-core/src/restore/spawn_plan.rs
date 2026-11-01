//! What a session pane spawns with on a relaunch (upstream's `Ghostty/LaunchSeed.swift`; its `LaunchSpawnPlan.swift`
//! paces a launch's spawns, which a lazy restore leaves nothing to pace, `commands::relaunch`).
//!
//! A pane's seed is decided at SPAWN time, not when its view is built: while a pane waits for its session to wake,
//! the captured argv and the pin stay on the session, so `restore.clear` still disarms them and a clean quit still
//! captures across that window. The seed takes the pane's live role, since `session.swap` moves a terminal and its
//! pending payloads to the other slot together.
//!
//! A live pane's command comes in two forms ([`crate::live::support::Creation`]): the pane's own creation command
//! (`session new --command`, `session split --command`) ends the pane when it exits, as outside Live, and holds on
//! `--wait` ([`resolve_live_seed`]); a captured argv's replay returns to the login shell, as `session restart
//! --command` does (`commands::restart_live`).

use std::collections::{BTreeSet, HashSet};

use crate::command_restore::{PaneInput, RestoreInputs, restore_plan, should_restore};
use crate::live::support::{Configuration, Creation, LiveAttach, attach_line, live_attach, settled};
use crate::model::{Session, StatusPane};

/// What a pane spawns with: the exec `command`, the `initial_input` typed into a login shell, and whether a command's
/// exit holds on the "press any key" prompt (upstream's `LaunchSeed`). `command` and `initial_input` exclude each
/// other.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LaunchSeed {
    pub command: Option<String>,
    /// Typed into the login shell, quoted for it where the frontend knows which shell it runs.
    pub initial_input: Option<PaneInput>,
    pub wait_after_command: bool,
    /// Variables the terminal runs with on top of the pane's own environment: a wrapped pane's attach contract.
    pub env: Vec<(String, String)>,
    /// The pane attaches to its live-session daemon (upstream's `backedByZmx`, `backedByLive` on the wire here), fixed
    /// at spawn as its disposition is.
    pub live: bool,
    /// A live pane's attach as data, which the pane runs in process in place of `command` (its shell line); `None`
    /// for any other pane, and behind a session host's client.
    pub attach: Option<LiveAttach>,
}

/// The launch-wide inputs a seed is decided against (upstream's `LaunchSeedPolicy`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LaunchSeedPolicy {
    /// This launch replays captured commands and pins (`rerun` is active).
    pub restore_enabled: bool,
    /// `live` was requested and fell back: neither pending slot is read, so both stay armed for the next launch.
    pub live_fell_back: bool,
    /// The program basenames never re-run (`restore-denylist.conf`).
    pub denylist: HashSet<String>,
    /// Daemons whose reset could not be confirmed at this launch: their panes attach with neither the captured replay
    /// nor the durable command, so a process that may still run is not started twice.
    pub suppressed_daemons: BTreeSet<String>,
}

impl LaunchSeedPolicy {
    /// The `initial_input` for a captured argv: its replay, or `None` outside rerun mode or when the argv may not be
    /// replayed (denylisted, a control character, an undecodable byte).
    fn restore_initial_input(&self, argv: Option<&[String]>) -> Option<PaneInput> {
        let argv = argv.filter(|_| self.restore_enabled)?;
        let denylist: Vec<&str> = self.denylist.iter().map(String::as_str).collect();
        should_restore(argv, &denylist).then(|| PaneInput::Replay(argv.to_vec()))
    }
}

/// Pane `pane`'s seed, consuming the slots the launch armed for it (upstream's `LaunchSeedProvider.seed`). The scratch
/// has none.
pub fn resolve_seed(session: &mut Session, pane: StatusPane, policy: &LaunchSeedPolicy) -> LaunchSeed {
    let Some(launch) = session.pane_launch(pane).cloned() else {
        return LaunchSeed::default();
    };
    let inputs = if policy.live_fell_back {
        RestoreInputs {
            was_restored: session.was_restored,
            restore_enabled: false,
            had_foreground: false,
            foreground_input: None,
            initial_command: launch.command,
            restore_override: None,
            requested_wait: launch.wait,
        }
    } else {
        let capture = session.take_pending_foreground(pane);
        RestoreInputs {
            was_restored: session.was_restored,
            restore_enabled: policy.restore_enabled,
            had_foreground: capture.is_some(),
            foreground_input: policy.restore_initial_input(capture.as_deref()),
            initial_command: launch.command,
            restore_override: session.take_pending_restore_override(pane),
            requested_wait: launch.wait,
        }
    };
    let plan = restore_plan(&inputs);
    LaunchSeed {
        command: plan.command,
        initial_input: plan.initial_input,
        wait_after_command: plan.wait_after_command,
        ..LaunchSeed::default()
    }
}

/// Pane `pane`'s seed under the wrapped live disposition `configuration` (upstream's `ZmxLaunch.surfaceSeed` and
/// `LaunchSeed.swift`): the attach, carrying the pane's captured argv as the payload a new daemon runs, which returns
/// to the login shell, else the pane's own creation command, which the pane ends with ([`Creation::EndingPane`]). The
/// capture is consumed either way; a suppressed daemon attaches with neither. The pending `session.restore` pin is
/// never read here: a daemon that survived runs its own program. The attach's exit closes the pane, unless the pane's
/// launch asked `--wait`: the hold stays with the pane across reattachment and a captured replay, as upstream's
/// `requestedWait` (the left pane's own, the right one's own; the scratch has none).
pub fn resolve_live_seed(
    session: &mut Session,
    pane: StatusPane,
    policy: &LaunchSeedPolicy,
    configuration: &Configuration,
) -> LaunchSeed {
    let suppressed = policy.suppressed_daemons.contains(&configuration.daemon_name);
    let captured = session.take_pending_foreground(pane);
    let replay = captured.filter(|_| !suppressed);
    let launch = session.pane_launch(pane);
    let pane_command = if replay.is_none() && !suppressed {
        launch.and_then(|launch| launch.command.as_deref())
    } else {
        None
    };
    let denylist: Vec<&str> = policy.denylist.iter().map(String::as_str).collect();
    let attach = live_attach(
        configuration,
        replay.as_deref(),
        pane_command.map(Creation::EndingPane),
        &denylist,
    );
    LaunchSeed {
        command: Some(settled(&attach_line(configuration, &attach))),
        initial_input: None,
        wait_after_command: launch.is_some_and(|launch| launch.wait),
        env: configuration.environment.clone().into_iter().collect(),
        live: true,
        attach: configuration.in_process(attach),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SessionSeed;

    fn policy(restore_enabled: bool, denylist: &[&str]) -> LaunchSeedPolicy {
        LaunchSeedPolicy {
            restore_enabled,
            live_fell_back: false,
            denylist: denylist.iter().map(|name| (*name).to_owned()).collect(),
            ..LaunchSeedPolicy::default()
        }
    }

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| (*part).to_owned()).collect()
    }

    fn session(command: Option<&str>, wait: bool, restored: bool) -> Session {
        let mut session = Session::new(SessionSeed {
            initial_cwd: "/tmp".to_owned(),
            ..SessionSeed::default()
        });
        session.launch.command = command.map(str::to_owned);
        session.launch.wait = wait;
        session.was_restored = restored;
        session
    }

    #[test]
    fn a_fresh_command_session_always_runs_its_command_and_holds_when_asked() {
        let mut fresh = session(Some("cat"), true, false);
        let seed = resolve_seed(&mut fresh, StatusPane::Left, &policy(false, &[]));
        assert_eq!(
            seed,
            LaunchSeed {
                command: Some("cat".to_owned()),
                initial_input: None,
                wait_after_command: true,
                ..LaunchSeed::default()
            }
        );
    }

    #[test]
    fn a_restored_command_session_reruns_only_in_rerun_mode() {
        let mut restored = session(Some("cat"), true, true);
        assert_eq!(
            resolve_seed(&mut restored, StatusPane::Left, &policy(false, &[])),
            LaunchSeed::default()
        );
        let seed = resolve_seed(&mut restored, StatusPane::Left, &policy(true, &[]));
        assert_eq!(seed.command.as_deref(), Some("cat"));
        assert!(seed.wait_after_command);
    }

    #[test]
    fn a_captured_job_is_typed_once_and_preempts_the_creation_command() {
        let mut restored = session(Some("/bin/sh"), false, true);
        restored.restore.pending_foreground = Some(argv(&["sleep", "600"]));
        let rerun = policy(true, &[]);
        let seed = resolve_seed(&mut restored, StatusPane::Left, &rerun);
        assert_eq!(seed.command, None);
        assert_eq!(
            seed.initial_input,
            Some(PaneInput::Replay(vec!["sleep".to_owned(), "600".to_owned()]))
        );
        assert_eq!(restored.restore.pending_foreground, None, "consumed once");
        assert_eq!(
            resolve_seed(&mut restored, StatusPane::Left, &rerun).command.as_deref(),
            Some("/bin/sh")
        );
    }

    #[test]
    fn a_denylisted_capture_still_preempts_and_leaves_a_plain_shell() {
        let mut restored = session(Some("tmux"), false, true);
        restored.restore.pending_foreground = Some(argv(&["tmux", "attach"]));
        let rerun = policy(true, &["tmux"]);
        assert_eq!(
            resolve_seed(&mut restored, StatusPane::Left, &rerun),
            LaunchSeed::default()
        );
    }

    #[test]
    fn a_pin_is_typed_verbatim_in_rerun_and_ignored_in_fresh_shells() {
        let mut restored = session(None, false, true);
        restored.restore.pending_command = Some("cd x && claude --resume y".to_owned());
        let mut fresh = session(None, false, true);
        fresh.restore.pending_command = Some("echo pinned".to_owned());
        assert_eq!(
            resolve_seed(&mut fresh, StatusPane::Left, &policy(false, &[])),
            LaunchSeed::default()
        );
        let seed = resolve_seed(&mut restored, StatusPane::Left, &policy(true, &[]));
        assert_eq!(
            seed.initial_input,
            Some(PaneInput::Line("cd x && claude --resume y".to_owned()))
        );
        assert_eq!(seed.command, None);
    }

    fn wrapped() -> Configuration {
        Configuration {
            executable: "/bin/ngterm-session".to_owned(),
            session_host_executable: None,
            environment: [("SHELL", "/bin/zsh"), ("ZDOTDIR", "/bundle/zsh")]
                .into_iter()
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
                .collect(),
            daemon_name: "ngterm-pane".to_owned(),
            socket_directory: "/tmp/ngterm-live-x".to_owned(),
            pane_id: "pane".to_owned(),
        }
    }

    /// The seed command of a live pane whose attach carries `replaying` or `creation`.
    fn live_command(replaying: Option<&[String]>, creation: Option<Creation<'_>>) -> String {
        let configuration = wrapped();
        let attach = live_attach(&configuration, replaying, creation, &[] as &[&str]);
        settled(&attach_line(&configuration, &attach))
    }

    /// upstream's `testWrappedPaneCommandEndsThePaneAndCarriesItsWait`.
    #[test]
    fn a_wrapped_pane_command_ends_the_pane_and_carries_its_wait() {
        let mut fresh = session(Some("lazygit"), true, false);
        fresh.split_launch.command = Some("htop".to_owned());
        let rerun = policy(true, &[]);
        let primary = resolve_live_seed(&mut fresh, StatusPane::Left, &rerun, &wrapped());
        assert_eq!(
            primary.command,
            Some(live_command(None, Some(Creation::EndingPane("lazygit"))))
        );
        assert!(primary.wait_after_command && primary.live);
        let split = resolve_live_seed(&mut fresh, StatusPane::Right, &rerun, &wrapped());
        assert_eq!(
            split.command,
            Some(live_command(None, Some(Creation::EndingPane("htop"))))
        );
        assert!(!split.wait_after_command);
    }

    /// upstream's `testWrappedSplitCarriesItsOwnWait`; the scratch never holds.
    #[test]
    fn a_wrapped_split_carries_its_own_wait() {
        let mut fresh = session(None, false, false);
        fresh.split_launch.command = Some("htop".to_owned());
        fresh.split_launch.wait = true;
        let rerun = policy(true, &[]);
        assert!(resolve_live_seed(&mut fresh, StatusPane::Right, &rerun, &wrapped()).wait_after_command);
        assert!(!resolve_live_seed(&mut fresh, StatusPane::Left, &rerun, &wrapped()).wait_after_command);
        assert!(!resolve_live_seed(&mut fresh, StatusPane::Scratch, &rerun, &wrapped()).wait_after_command);
    }

    /// A captured argv replays (and returns to the shell) in place of the pane command, and a suppressed daemon
    /// attaches with neither; the hold stays with the pane either way.
    #[test]
    fn the_wait_stays_with_the_pane_across_a_replay_and_a_suppressed_daemon() {
        let mut restored = session(Some("lazygit"), true, true);
        restored.restore.pending_foreground = Some(argv(&["top"]));
        let rerun = policy(true, &[]);
        let replayed = resolve_live_seed(&mut restored, StatusPane::Left, &rerun, &wrapped());
        assert_eq!(replayed.command, Some(live_command(Some(&argv(&["top"])), None)));
        assert!(replayed.wait_after_command);
        restored.restore.pending_foreground = Some(argv(&["top"]));
        let suppressed = LaunchSeedPolicy {
            suppressed_daemons: BTreeSet::from(["ngterm-pane".to_owned()]),
            ..rerun
        };
        let bare = resolve_live_seed(&mut restored, StatusPane::Left, &suppressed, &wrapped());
        assert_eq!(bare.command, Some(live_command(None, None)));
        assert!(bare.wait_after_command);
        assert_eq!(restored.restore.pending_foreground, None, "consumed either way");
    }

    #[test]
    fn a_live_fallback_reads_no_slot_and_holds_the_creation_command() {
        let mut restored = session(Some("cat"), false, true);
        restored.restore.pending_foreground = Some(argv(&["vim"]));
        restored.restore.pending_command = Some("echo pinned".to_owned());
        let fallback = LaunchSeedPolicy {
            restore_enabled: false,
            live_fell_back: true,
            ..LaunchSeedPolicy::default()
        };
        assert_eq!(
            resolve_seed(&mut restored, StatusPane::Left, &fallback),
            LaunchSeed::default()
        );
        assert!(
            restored.restore.pending_foreground.is_some(),
            "kept for the next launch"
        );
        assert!(restored.restore.pending_command.is_some());
    }
}
