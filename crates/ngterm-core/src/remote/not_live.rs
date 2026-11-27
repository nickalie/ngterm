//! Why an agterm lists no session: only a session started in its Live sessions mode runs in a zmx daemon a viewer can
//! attach, and an agterm in another restore mode answers its listing with none and no reason. A listing in which
//! agterm listed nothing asks it, read-only, for its restore mode (`agtermctl restore mode`, one more line run there)
//! and keeps the answer on agterm's part of the listing ([`ngterm_proto::ControlRemoteTree::restore`]); every place
//! that lists a host's sessions then says why with [`hint`]. The setting's names are upstream's Settings ▸ General ▸
//! Sessions picker (`agterm/Views/SettingsView.swift`: "Restore sessions", its "Live sessions" mode from
//! `RestoreMode.displayName`, and "Changes apply after restarting agterm."). Nothing here changes agterm's setting.

#[cfg(test)]
mod tests;

use std::time::Duration;

use ngterm_proto::{ControlRemoteTree, ControlRestoreStatus, ControlResult, RemotePeer, Response};

use super::origin::{OriginForm, OriginShell};
use super::origin_replies::{LIVE_MODE, RESTORE_MODE_ARGUMENTS};
use super::tree::{self, RemoteCommandResult, RemoteCommandRunner};

/// The line [`with_restore`] puts ahead of agterm's answer to `restore mode`, after the listing it follows.
pub const AGTERM_RESTORE: &str = "ngterm-peer: agterm-restore";
/// How long agterm may take to name its restore mode once connected; a later answer leaves the listing without a reason.
pub const RESTORE_DEADLINE: Duration = Duration::from_secs(5);

/// The remote command asking an agterm origin for its restore mode: `agtermctl restore mode --json`.
#[must_use]
pub fn restore_tail() -> String {
    let agterm = OriginForm {
        shell: OriginShell::Posix,
        peer: RemotePeer::Agterm,
    };
    super::ssh::control_tail(agterm, &RESTORE_MODE_ARGUMENTS)
}

/// Whether `listing`, a finished listing, has an agterm that listed no session, which is then asked its restore mode.
#[must_use]
pub fn asks_restore_mode(listing: &RemoteCommandResult) -> bool {
    listing.status == 0 && tree::decode(&listing.stdout).is_ok_and(|tree| unlisted_agterm(&tree).is_some())
}

/// `listing` with agterm's answer to `restore mode` after the [`AGTERM_RESTORE`] line, where [`tree::decode`] reads
/// it; unchanged when that command failed.
#[must_use]
pub fn with_restore(mut listing: RemoteCommandResult, restore: &RemoteCommandResult) -> RemoteCommandResult {
    if restore.status == 0 {
        if !listing.stdout.is_empty() && !listing.stdout.ends_with('\n') {
            listing.stdout.push('\n');
        }
        listing.stdout.push_str(AGTERM_RESTORE);
        listing.stdout.push('\n');
        listing.stdout.push_str(&restore.stdout);
    }
    listing
}

/// `host`'s listing ([`tree::run_tree`]), and when agterm listed nothing there, its restore mode asked in a second
/// ssh call. For the places that show a host's sessions; a flow waiting for one session reads [`tree::run_tree`].
#[must_use]
pub fn run_listing(runner: &dyn RemoteCommandRunner, host: &str, deadline: Duration) -> RemoteCommandResult {
    let listing = tree::run_tree(runner, host, deadline);
    if !asks_restore_mode(&listing) {
        return listing;
    }
    let mut argv = tree::ssh_arguments(host, false);
    argv.push(restore_tail());
    let restore = runner.run(&argv, super::connect_timeout::after_connect(host, RESTORE_DEADLINE));
    with_restore(listing, &restore)
}

/// A listing's output split at its [`AGTERM_RESTORE`] line: the listing, and the restore status agterm answered with.
pub(super) fn split(stdout: &str) -> (&str, Option<ControlRestoreStatus>) {
    let mut start = 0;
    for line in stdout.split_inclusive('\n') {
        if line.trim_end_matches(['\r', '\n']) == AGTERM_RESTORE {
            let answer = &stdout[start + line.len()..];
            let status = serde_json::from_str::<Response<ControlResult>>(answer.trim())
                .ok()
                .filter(|reply| reply.ok)
                .and_then(|reply| reply.result?.restore);
            return (&stdout[..start], status);
        }
        start += line.len();
    }
    (stdout, None)
}

/// `tree` with `status` on its agterm that listed nothing; unchanged when it has none.
pub(super) fn keep(mut tree: ControlRemoteTree, status: Option<ControlRestoreStatus>) -> ControlRemoteTree {
    let Some(status) = status else {
        return tree;
    };
    let mut app = Some(&mut tree);
    while let Some(current) = app {
        if is_unlisted_agterm(current) {
            current.restore = Some(status);
            break;
        }
        app = current.beside.as_deref_mut();
    }
    tree
}

fn is_unlisted_agterm(app: &ControlRemoteTree) -> bool {
    app.endpoint.peer() == RemotePeer::Agterm && app.sessions.is_empty()
}

fn unlisted_agterm(tree: &ControlRemoteTree) -> Option<&ControlRemoteTree> {
    tree.apps().find(|app| is_unlisted_agterm(app))
}

/// Why `tree`'s agterm lists no session: it is not in Live sessions mode, and what to change there. `None` for an
/// agterm in that mode, one that lists sessions, and a host with no agterm: their listings say nothing more.
#[must_use]
pub fn hint(tree: &ControlRemoteTree) -> Option<String> {
    let app = unlisted_agterm(tree)?;
    let status = app.restore.as_ref().filter(|status| status.active != LIVE_MODE)?;
    let place = app
        .host
        .as_deref()
        .map(|host| format!(" on {}", super::peer::ssh_destination(host)))
        .unwrap_or_default();
    let attachable = "only sessions started after that can be attached";
    Some(match (&status.unavailable_reason, status.configured == LIVE_MODE) {
        (Some(reason), _) => format!("agterm{place} could not start Live sessions: {reason}"),
        (None, true) => {
            format!("agterm{place} is not in Live sessions mode until it restarts: restart agterm; {attachable}")
        }
        (None, false) => format!(
            "agterm{place} is not in Live sessions mode: set Settings > General > Restore sessions to Live sessions \
             and restart agterm; {attachable}"
        ),
    })
}
