//! A machine's sessions: listing them (`ngtermctl live tree`), New Session, Duplicate, Rename Session and End Session,
//! run on the machine's connection as the lines desktop's Attach Remote runs over ssh ([`ngterm_core::remote::ssh`]'s
//! tails), and read with the same decisions and strings ([`ngterm_core::remote::origin_replies`]).
//!
//! - The list is what desktop's session picker offers: a page companion has no row of its own, and parents come before
//!   their companions.
//! - New Session is desktop's three stages: the machine must run live sessions (`restore mode`), it creates a session
//!   of its own with `session new --no-select` (named, when the user gave a name), and it is listed every
//!   [`PAUSE`] until the session is attachable or the
//!   [`READY_BUDGET`](ngterm_core::remote::origin_replies::READY_BUDGET) runs out.
//! - Duplicate is New Session with `session duplicate --target` as its second line, desktop's Duplicate Session of a
//!   remote row: the machine copies its session's focused directory into a fresh shell after it, selected there. A
//!   companion's copy is a session of its own, and a parent's companions are not copied.
//! - Rename Session names the session there (`session rename --target ID -- NAME`).
//! - End Session closes the session there (`session close --target`), with no undo.
//!
//! A headless machine with its catalog up gets New Session, Rename Session and End Session under an operation id
//! instead ([`crate::catalog::operations`]).
//!
//! Each line runs once, never retried. A command that can't start (the connection is gone) fails with the transport's
//! own text; once it ran, what it answered decides.
//!
//! **Phone wording.** Two of desktop's strings end in CLI advice for a session created but not attached: "…;
//! `ngtermctl live attach HOST ID` attaches it" (the connection failed after `session new` answered, and the session
//! never became attachable in time). A phone has no `ngtermctl`, and the session shows in the machine's list once it is
//! attachable, so [`phone_wording`] replaces that advice with [`ATTACH_FROM_LIST`]. Every other reused string names no
//! CLI and stays as desktop words it.

#[cfg(test)]
mod tests;

use std::time::Duration;

use ngterm_core::link_policy;
use ngterm_core::pane_env::uuid_string;
use ngterm_core::remote::not_live;
use ngterm_core::remote::origin::{self, OriginForm, OriginShell};
use ngterm_core::remote::origin_replies::{
    COMPANION_DEADLINE, ListingWait, NEW_SESSION_ARGUMENTS, PAUSE, RESTORE_MODE_ARGUMENTS, WaitStep,
    check_restore_mode, duplicate_session_arguments, end_failure, end_session_arguments, no_answer, pickable_sessions,
    read_remote_tree, reply_of, session_created,
};
use ngterm_core::remote::ssh::{control_tail, control_tail_with_operands, tree_tail_for};
use ngterm_core::remote::tree::{
    INVALID_REMOTE_SESSION, NO_ANSWER_IN_TIME, REMOTE_CLI, RemoteCommandResult, TREE_DEADLINE, is_plain,
};
use ngterm_proto::{ControlRemoteSession, ControlRemoteTree, RemotePeer};

use crate::applog::tag;
use crate::catalog::operations::rename_arguments;
use crate::note;
use crate::transport::{ChannelEnd, ChannelOutput, Connection};

/// What replaces desktop's CLI advice in a created-but-unattached failure ([`phone_wording`]).
pub const ATTACH_FROM_LIST: &str = "attach it from the session list once it shows there";

/// A session a machine offers, as the app lists it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RemoteSession {
    /// The far side's session id (an upper-case UUID): what End Session and attaching name.
    pub id: String,
    pub name: String,
    /// Where it lives there.
    pub window_name: String,
    pub workspace_name: String,
    /// The far side's own note of what the session is for.
    pub context: Option<String>,
    pub cwd: String,
    /// The far agent's status text, color and shape; `None` for an idle session.
    pub status: Option<String>,
    pub status_color: Option<String>,
    pub status_shape: Option<String>,
    /// The parent session of a companion; `None` for a top-level session.
    pub parent: Option<String>,
    /// How many panes it shows (one, or two side by side).
    pub pane_count: u32,
    /// The divider's direction when it has a split.
    pub split_axis: Option<String>,
    /// The app that listed this session ([`crate::peer`]), which on a machine running both may differ from session to
    /// session; `None` reads as ngterm.
    #[uniffi(default = None)]
    pub peer: Option<crate::peer::PeerKind>,
    /// A session of a headless origin's catalog whose panes are gone without its shell exiting: it stays listed, marked
    /// ended, until it is closed. Always false on the plain listing.
    #[uniffi(default = false)]
    pub ended: bool,
    /// A page row of a headless origin's catalog: its page's kind (`file` or `url`). It shows no terminal, so it is
    /// never attached. `None` for a terminal session, and always on the plain listing, which lists no pages.
    #[uniffi(default = None)]
    pub page: Option<String>,
}

impl RemoteSession {
    /// `session`, as a listing of `peer` showed it.
    #[must_use]
    pub fn listed(session: &ControlRemoteSession, peer: RemotePeer) -> Self {
        Self {
            id: session.id.clone(),
            name: session.name.clone(),
            window_name: session.window_name.clone(),
            workspace_name: session.workspace_name.clone(),
            context: session.context.clone(),
            cwd: session.cwd.clone(),
            status: session.status.clone(),
            status_color: session.status_color.clone(),
            status_shape: session.status_shape.clone(),
            parent: session.parent.clone(),
            pane_count: u32::try_from(session.panes.len()).unwrap_or(u32::MAX),
            split_axis: session.split_axis.clone(),
            peer: Some(peer.into()),
            ended: false,
            page: None,
        }
    }
}

/// The sessions of `tree` the app lists: the desktop's terminal sessions (no pages, parents first), each naming the app
/// that listed it. On a machine that runs both apps, agterm's follow ngterm's.
#[must_use]
pub fn listed_sessions(tree: &ControlRemoteTree) -> Vec<RemoteSession> {
    tree.apps()
        .flat_map(|app| {
            let peer = app.endpoint.peer();
            pickable_sessions(app)
                .into_iter()
                .map(move |session| RemoteSession::listed(session, peer))
        })
        .collect()
}

/// The listed session a session link names, tapped in a pane of `from` (upstream's `WindowLibrary+SessionLink`): the
/// raw link must be exactly `ngterm://session/<id>`, or `agterm://session/<id>` when an agterm origin lists `from`. The
/// id resolves in the listing of the app that lists `from`, among the sessions the app lists. `None` for any other
/// link, an unknown or page session, or a `from` the listing does not hold.
#[must_use]
pub fn session_link_target(tree: &ControlRemoteTree, from: &str, raw: &str) -> Option<RemoteSession> {
    let app = tree
        .apps()
        .find(|app| app.sessions.iter().any(|listed| listed.id == from))?;
    let peer = app.endpoint.peer();
    let id = link_policy::session_link(raw, RemotePeer::Ngterm).or_else(|| match peer {
        RemotePeer::Agterm => link_policy::session_link(raw, peer),
        RemotePeer::Ngterm => None,
    })?;
    let id = uuid_string(id);
    pickable_sessions(app)
        .into_iter()
        .find(|session| session.id.eq_ignore_ascii_case(&id))
        .map(|session| RemoteSession::listed(session, peer))
}

/// Whether `raw` has the form of a session link, ngterm's or agterm's, whatever session it names.
#[must_use]
pub fn is_session_link(raw: &str) -> bool {
    [RemotePeer::Ngterm, RemotePeer::Agterm]
        .into_iter()
        .any(|peer| link_policy::session_link(raw, peer).is_some())
}

/// `fresh`, a listing of one app alone (New Session's and Duplicate's), as the machine's whole listing: with the
/// other app's sessions as `kept`, the machine's last listing, has them, when the machine runs both.
#[must_use]
pub fn beside_kept(mut fresh: ControlRemoteTree, kept: Option<&ControlRemoteTree>) -> ControlRemoteTree {
    let Some(kept) = kept.filter(|kept| kept.beside.is_some() && fresh.beside.is_none()) else {
        return fresh;
    };
    if kept.endpoint.peer() == fresh.endpoint.peer() {
        fresh.beside.clone_from(&kept.beside);
        return fresh;
    }
    ControlRemoteTree {
        beside: Some(Box::new(fresh)),
        ..kept.clone()
    }
}

/// `fresh`, a listing that went on without an app the machine has ([`ControlRemoteTree::skipped`]), as the machine's
/// whole listing: with that app's sessions as `kept`, the machine's last listing, has them, since an app that did not
/// answer has not ended its sessions. A listing that skipped nothing, or an app `kept` never listed, is as it came.
#[must_use]
pub fn skipped_kept(mut fresh: ControlRemoteTree, kept: Option<&ControlRemoteTree>) -> ControlRemoteTree {
    let last = fresh
        .skipped
        .and_then(|skipped| kept?.apps().find(|app| app.endpoint.peer() == skipped));
    let Some(last) = last else {
        return fresh;
    };
    fresh.skipped = None;
    let last = ControlRemoteTree {
        beside: None,
        skipped: None,
        ..last.clone()
    };
    if last.endpoint.peer() == RemotePeer::Ngterm {
        fresh.beside = None;
        return ControlRemoteTree {
            beside: Some(Box::new(fresh)),
            ..last
        };
    }
    fresh.beside = Some(Box::new(last));
    fresh
}

/// `message` with desktop's `ngtermctl live attach` advice replaced by [`ATTACH_FROM_LIST`]; any other message as it
/// is.
#[must_use]
pub fn phone_wording(message: String) -> String {
    let advice = format!("; `{REMOTE_CLI} live attach ");
    match message.find(&advice) {
        Some(at) if message.ends_with("` attaches it") => format!("{}; {ATTACH_FROM_LIST}", &message[..at]),
        _ => message,
    }
}

/// New Session's second line: desktop's, with `--name=NAME` when the user named the session (a blank name is none).
/// The `=` form keeps a name that starts with `-` a value.
#[must_use]
pub fn new_session_arguments(name: Option<&str>) -> Vec<String> {
    let mut arguments: Vec<String> = NEW_SESSION_ARGUMENTS.iter().map(|&word| word.to_owned()).collect();
    if let Some(name) = name.map(str::trim).filter(|name| !name.is_empty()) {
        arguments.push(format!("--name={name}"));
    }
    arguments
}

/// Runs `command` on a new exec channel of `connection` with no pty, to its end or `deadline`: what it printed and how
/// it ended, as desktop's runner reports an ssh run (status -1 when it was lost, killed by a signal or out of time).
///
/// # Errors
///
/// The transport's text when the channel could not open: the command never ran.
pub async fn run<C: Connection>(
    connection: &C,
    command: &str,
    deadline: Duration,
) -> Result<RemoteCommandResult, String> {
    let until = tokio::time::Instant::now() + deadline;
    let mut channel = match tokio::time::timeout_at(until, connection.exec(command, None)).await {
        Ok(Ok(channel)) => channel,
        Ok(Err(error)) => return Err(error.to_string()),
        Err(_) => return Ok(out_of_time()),
    };
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let end = loop {
        match tokio::time::timeout_at(until, channel.reader.read()).await {
            Ok(ChannelOutput::Data(bytes)) => stdout.extend_from_slice(&bytes),
            Ok(ChannelOutput::Stderr(bytes)) => stderr.extend_from_slice(&bytes),
            Ok(ChannelOutput::End(end)) => break end,
            Err(_) => {
                // A channel already gone has nothing to close.
                let _ = channel.writer.close();
                return Ok(out_of_time());
            }
        }
    };
    let mut stderr = String::from_utf8_lossy(&stderr).into_owned();
    let status = match end {
        ChannelEnd::Exit { code: Some(code), .. } => i32::try_from(code).unwrap_or(-1),
        ChannelEnd::Exit { code: None, .. } => -1,
        ChannelEnd::Lost(reason) => {
            if stderr.trim().is_empty() {
                stderr = reason;
            }
            -1
        }
    };
    Ok(RemoteCommandResult {
        status,
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr,
    })
}

fn out_of_time() -> RemoteCommandResult {
    RemoteCommandResult {
        status: -1,
        stdout: String::new(),
        stderr: NO_ANSWER_IN_TIME.to_owned(),
    }
}

/// Runs the machine's `live tree` within `deadline`, learning a Windows origin from a refusal of the POSIX form as
/// desktop does ([`ngterm_core::remote::origin`]), kept on the connection for the commands that follow. `pin` lists
/// that app alone, as a flow that has chosen it reads the machine.
async fn run_tree<C: Connection>(
    connection: &C,
    deadline: Duration,
    pin: Option<RemotePeer>,
) -> Result<RemoteCommandResult, String> {
    let shell = connection.origin_shell();
    note!(
        INFO,
        tag::SESSIONS,
        "listing asked",
        cli = asked_cli(shell, pin),
        shell = shell_class(shell)
    );
    let first = run(connection, &tree_tail_for(shell, pin), deadline).await?;
    if shell == OriginShell::Windows || !origin::rejected_by_windows_shell(&first) {
        return Ok(first);
    }
    let probe = run(connection, origin::WINDOWS_PROBE, origin::WINDOWS_PROBE_DEADLINE).await?;
    if !origin::is_windows_answer(&probe) {
        return Ok(first);
    }
    connection.set_origin_shell(OriginShell::Windows);
    note!(INFO, tag::SESSIONS, "listing asked again: the machine runs Windows");
    run(connection, &tree_tail_for(OriginShell::Windows, pin), deadline).await
}

/// Which CLI a listing asks, for the log: both apps' when no app is pinned.
fn asked_cli(shell: OriginShell, pin: Option<RemotePeer>) -> &'static str {
    match (shell, pin) {
        (OriginShell::Windows, _) | (OriginShell::Posix, Some(RemotePeer::Ngterm)) => "ngtermctl",
        (OriginShell::Posix, Some(RemotePeer::Agterm)) => "agtermctl",
        (OriginShell::Posix, None) => "ngtermctl-and-agtermctl",
    }
}

fn shell_class(shell: OriginShell) -> &'static str {
    match shell {
        OriginShell::Posix => "posix",
        OriginShell::Windows => "windows",
    }
}

/// The app's name, for the log.
pub(crate) fn peer_class(peer: RemotePeer) -> &'static str {
    match peer {
        RemotePeer::Ngterm => "ngterm",
        RemotePeer::Agterm => "agterm",
    }
}

/// Why a listing's answer gave no sessions, for the log: never what the machine printed.
fn failure_class(result: &RemoteCommandResult) -> &'static str {
    if result.status == 0 {
        "unreadable-answer"
    } else if result.stderr == NO_ANSWER_IN_TIME {
        "no-answer-in-time"
    } else if result.status == -1 {
        "lost"
    } else {
        "exit-status"
    }
}

/// The form of the commands for `app` on the connection's machine, the app whose listing shows the session they are
/// about. With none named it is the machine's own, once a listing has told it: a connection no listing has told yet
/// lists first, so a first line to an agterm Mac is never ngterm's.
async fn known_form<C: Connection>(connection: &C, host: &str, app: Option<RemotePeer>) -> Result<OriginForm, String> {
    if let Some(peer) = app {
        return Ok(OriginForm {
            peer,
            ..connection.origin_form()
        });
    }
    if !connection.learned().peer_known {
        read_tree(connection, host, TREE_DEADLINE)
            .await
            .map_err(phone_wording)?;
    }
    Ok(connection.origin_form())
}

/// Runs `ARGUMENTS --json` in `form`.
async fn control_in<C: Connection, S: AsRef<str>>(
    connection: &C,
    form: OriginForm,
    arguments: &[S],
) -> Result<RemoteCommandResult, String> {
    run(connection, &control_tail(form, arguments), COMPANION_DEADLINE).await
}

/// `host`'s attachable sessions, within `deadline`; `host` is the name the messages give the machine. The plain
/// listing a flow reads before it acts there, as desktop's flows read `tree::run_tree`; [`list`] is the one the
/// sessions screen shows.
///
/// # Errors
///
/// Why there is no listing: desktop's reading of the far side's refusal or ssh's failure, or the transport's text.
pub async fn read_tree<C: Connection>(
    connection: &C,
    host: &str,
    deadline: Duration,
) -> Result<ControlRemoteTree, String> {
    let result = tree_answer(connection, host, deadline).await?;
    learned_tree(connection, host, &result)
}

/// The sessions screen's listing of `host`, within desktop's [`TREE_DEADLINE`]. An agterm there that lists none is
/// then asked its restore mode, within [`not_live::RESTORE_DEADLINE`] more, as desktop's listings ask it
/// ([`not_live::run_listing`]), so the listing says why.
///
/// # Errors
///
/// As [`read_tree`].
pub async fn list<C: Connection>(connection: &C, host: &str) -> Result<ControlRemoteTree, String> {
    let mut result = tree_answer(connection, host, TREE_DEADLINE).await?;
    if not_live::asks_restore_mode(&result) {
        // the mode only says why agterm listed nothing, so a failed ask leaves the listing as it is
        if let Ok(restore) = run(connection, &not_live::restore_tail(), not_live::RESTORE_DEADLINE).await {
            result = not_live::with_restore(result, &restore);
        }
    }
    learned_tree(connection, host, &result)
}

/// `host`'s listing answer within `deadline`, a channel that never opened noted.
async fn tree_answer<C: Connection>(
    connection: &C,
    host: &str,
    deadline: Duration,
) -> Result<RemoteCommandResult, String> {
    run_tree(connection, deadline, None).await.inspect_err(|_| {
        note!(WARN, tag::SESSIONS, "listing failed", class = "no-channel", host = host);
    })
}

/// `result` read as `host`'s listing, its app kept on the connection for the commands that follow.
fn learned_tree<C: Connection>(
    connection: &C,
    host: &str,
    result: &RemoteCommandResult,
) -> Result<ControlRemoteTree, String> {
    let tree = read_remote_tree(result, host).inspect_err(|_| {
        note!(
            WARN,
            tag::SESSIONS,
            "listing failed",
            class = failure_class(result),
            status = result.status,
            stdout_bytes = result.stdout.len(),
            stderr_bytes = result.stderr.len(),
            host = host
        );
    })?;
    // an ngterm that did not answer this once still has the machine, once a listing has told it
    if tree.skipped != Some(RemotePeer::Ngterm) || !connection.learned().peer_known {
        connection.set_origin_peer(tree.endpoint.peer());
    }
    let sessions: usize = tree.apps().map(|app| app.sessions.len()).sum();
    note!(
        INFO,
        tag::SESSIONS,
        "listing answered",
        answer = if sessions == 0 { "empty" } else { "sessions" },
        peer = peer_class(tree.endpoint.peer()),
        beside = tree
            .beside
            .as_deref()
            .map_or("none", |beside| peer_class(beside.endpoint.peer())),
        skipped = tree.skipped.map_or("none", peer_class),
        sessions = sessions,
        pickable = listed_sessions(&tree).len(),
        host = host
    );
    Ok(tree)
}

/// New Session on `host`, named `name` when given: the created session once it is attachable, and the listing that
/// showed it. `listing_starts` is called as each listing begins, so the caller knows how recent the last one is.
///
/// # Errors
///
/// Why it was not created, or created but not attachable in time, in [`phone_wording`].
pub async fn create<C: Connection>(
    connection: &C,
    host: &str,
    name: Option<&str>,
    listing_starts: impl FnMut(),
) -> Result<(ControlRemoteSession, ControlRemoteTree), String> {
    make(connection, (host, None), &new_session_arguments(name), listing_starts).await
}

/// Duplicate on `host` of its session `source`, a session of `app` (`None`: the machine's own), as [`create`] answers.
///
/// # Errors
///
/// [`INVALID_REMOTE_SESSION`] for an id that is not one plain token, otherwise as [`create`].
pub async fn duplicate<C: Connection>(
    connection: &C,
    host: &str,
    (source, app): (&str, Option<RemotePeer>),
    listing_starts: impl FnMut(),
) -> Result<(ControlRemoteSession, ControlRemoteTree), String> {
    if !is_plain(source) {
        return Err(INVALID_REMOTE_SESSION.to_owned());
    }
    make(
        connection,
        (host, app),
        &duplicate_session_arguments(source),
        listing_starts,
    )
    .await
}

async fn make<C: Connection, S: AsRef<str>>(
    connection: &C,
    (host, app): (&str, Option<RemotePeer>),
    arguments: &[S],
    mut listing_starts: impl FnMut(),
) -> Result<(ControlRemoteSession, ControlRemoteTree), String> {
    // the app asked to create is the one whose listing shows the session, whichever app answers the machine meanwhile
    let form = known_form(connection, host, app).await?;
    let mode = control_in(connection, form, &RESTORE_MODE_ARGUMENTS).await?;
    check_restore_mode(&mode, host)?;
    let created = control_in(connection, form, arguments).await?;
    let created = session_created(&created, host).map_err(phone_wording)?;
    let mut wait = ListingWait::new(created, now());
    loop {
        match wait.next(now()) {
            WaitStep::Wait => tokio::time::sleep(PAUSE).await,
            WaitStep::GiveUp(reason) => return Err(phone_wording(wait.unattached(host, &reason))),
            WaitStep::List(within) => {
                listing_starts();
                // The connection is gone: no later listing could run either.
                let result = match run_tree(connection, within, Some(form.peer)).await {
                    Ok(result) => result,
                    Err(reason) => return Err(phone_wording(wait.unattached(host, &reason))),
                };
                if let Some(listed) = wait.listed(&result, host, now()) {
                    return Ok(listed);
                }
            }
        }
    }
}

/// End Session: `host` closes its session `session`, a session of `app` (`None`: the machine's own), its programs and
/// companions with it.
///
/// # Errors
///
/// [`INVALID_REMOTE_SESSION`] for an id that is not one plain token, otherwise why it did not end there (desktop's
/// `end_failure`) or the transport's text.
pub async fn end<C: Connection>(
    connection: &C,
    host: &str,
    session: &str,
    app: Option<RemotePeer>,
) -> Result<(), String> {
    if !is_plain(session) {
        return Err(INVALID_REMOTE_SESSION.to_owned());
    }
    let form = known_form(connection, host, app).await?;
    let result = control_in(connection, form, &end_session_arguments(session)).await?;
    end_failure(&result, host).map_or(Ok(()), Err)
}

/// Rename Session: `host` gives its session `session`, a session of `app` (`None`: the machine's own), the name `name`.
///
/// # Errors
///
/// [`INVALID_REMOTE_SESSION`] for an id that is not one plain token, otherwise why it was not renamed there
/// ([`rename_failure`]) or the transport's text.
pub async fn rename<C: Connection>(
    connection: &C,
    host: &str,
    (session, app): (&str, Option<RemotePeer>),
    name: &str,
) -> Result<(), String> {
    if !is_plain(session) {
        return Err(INVALID_REMOTE_SESSION.to_owned());
    }
    let form = known_form(connection, host, app).await?;
    let line = control_tail_with_operands(form, &rename_arguments(session), &[name]);
    let result = run(connection, &line, COMPANION_DEADLINE).await?;
    rename_failure(&result, host).map_or(Ok(()), Err)
}

/// Why `host`'s answer to `session rename` says the session was not renamed; `None` when it was.
#[must_use]
pub fn rename_failure(result: &RemoteCommandResult, host: &str) -> Option<String> {
    let Some(reply) = reply_of(result) else {
        return Some(no_answer(
            result,
            host,
            "whether the session was renamed there is unknown",
        ));
    };
    (!reply.ok).then(|| {
        reply
            .error
            .unwrap_or_else(|| format!("the remote command failed on {host}"))
    })
}

/// `tree` with session `session` named `name`, whichever app listed it.
pub fn renamed(tree: &mut ControlRemoteTree, session: &str, name: &str) {
    if let Some(listed) = tree.sessions.iter_mut().find(|listed| listed.id == session) {
        name.clone_into(&mut listed.name);
    }
    if let Some(beside) = tree.beside.as_deref_mut() {
        renamed(beside, session, name);
    }
}

/// `tree` without session `session` and its companions, as the far side's close took them, whichever app listed it.
pub fn forget(tree: &mut ControlRemoteTree, session: &str) {
    tree.sessions
        .retain(|listed| listed.id != session && listed.parent.as_deref() != Some(session));
    if let Some(beside) = tree.beside.as_deref_mut() {
        forget(beside, session);
    }
}

/// The clock New Session's wait and the presentation streams read: tokio's, so a test's paused clock drives it.
pub(crate) fn now() -> std::time::Instant {
    tokio::time::Instant::now().into_std()
}
