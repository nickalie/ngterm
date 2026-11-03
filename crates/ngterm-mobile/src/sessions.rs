//! A machine's sessions: listing them (`ngtermctl live tree`), New Session and End Session, run on the machine's
//! connection as the lines desktop's Attach Remote runs over ssh ([`ngterm_core::remote::ssh`]'s tails), and read with
//! the same decisions and strings ([`ngterm_core::remote::origin_replies`]).
//!
//! - The list is what desktop's session picker offers: a page companion has no row of its own, and parents come before
//!   their companions.
//! - New Session is desktop's three stages: the machine must run live sessions (`restore mode`), it creates a session
//!   of its own with `session new --no-select` (named, when the user gave a name), and it is listed every
//!   [`PAUSE`](ngterm_core::remote::origin_replies::PAUSE) until the session is attachable or the
//!   [`READY_BUDGET`](ngterm_core::remote::origin_replies::READY_BUDGET) runs out.
//! - End Session closes the session there (`session close --target`), with no undo.
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

use ngterm_core::remote::origin_replies::{
    ListingWait, NEW_SESSION_ARGUMENTS, PAUSE, RESTORE_MODE_ARGUMENTS, WaitStep, check_restore_mode, end_failure,
    end_session_arguments, pickable_sessions, read_remote_tree, session_created,
};
use ngterm_core::remote::ssh::{control_tail, tree_tail};
use ngterm_core::remote::tree::{REMOTE_CLI, RemoteCommandResult, TREE_DEADLINE, is_plain};
use ngterm_proto::{ControlRemoteSession, ControlRemoteTree};

use crate::transport::{ChannelEnd, ChannelOutput, Connection};

/// How long a control line (`restore mode`, `session new`, `session close`) may run: desktop's
/// `COMPANION_DEADLINE`.
pub const CONTROL_DEADLINE: Duration = Duration::from_secs(30);
/// What a line that did not finish within its deadline answers: desktop's runner's words.
pub const NO_ANSWER_IN_TIME: &str = "the remote did not answer in time";
/// Why End Session refuses a session id before running anything: desktop's words for a target that is not one plain
/// token.
pub const INVALID_SESSION: &str = "invalid remote session";
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
}

impl From<&ControlRemoteSession> for RemoteSession {
    fn from(session: &ControlRemoteSession) -> Self {
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
        }
    }
}

/// The sessions of `tree` the app lists: desktop's picker's (no page companions, parents first).
#[must_use]
pub fn listed_sessions(tree: &ControlRemoteTree) -> Vec<RemoteSession> {
    pickable_sessions(tree).into_iter().map(RemoteSession::from).collect()
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

/// Runs `host`'s `ngtermctl ARGUMENTS --json`.
async fn control<C: Connection, S: AsRef<str>>(connection: &C, arguments: &[S]) -> Result<RemoteCommandResult, String> {
    run(connection, &control_tail(arguments), CONTROL_DEADLINE).await
}

/// `host`'s attachable sessions, within `deadline`; `host` is the name the messages give the machine.
///
/// # Errors
///
/// Why there is no listing: desktop's reading of the far side's refusal or ssh's failure, or the transport's text.
pub async fn read_tree<C: Connection>(
    connection: &C,
    host: &str,
    deadline: Duration,
) -> Result<ControlRemoteTree, String> {
    let result = run(connection, &tree_tail(), deadline).await?;
    read_remote_tree(&result, host)
}

/// [`read_tree`] within desktop's [`TREE_DEADLINE`].
///
/// # Errors
///
/// As [`read_tree`].
pub async fn list<C: Connection>(connection: &C, host: &str) -> Result<ControlRemoteTree, String> {
    read_tree(connection, host, TREE_DEADLINE).await
}

/// New Session on `host`, named `name` when given: the created session once it is attachable, and the listing that
/// showed it.
///
/// # Errors
///
/// Why it was not created, or created but not attachable in time, in [`phone_wording`].
pub async fn create<C: Connection>(
    connection: &C,
    host: &str,
    name: Option<&str>,
) -> Result<(ControlRemoteSession, ControlRemoteTree), String> {
    let mode = control(connection, &RESTORE_MODE_ARGUMENTS).await?;
    check_restore_mode(&mode, host)?;
    let created = control(connection, &new_session_arguments(name)).await?;
    let created = session_created(&created, host).map_err(phone_wording)?;
    let mut wait = ListingWait::new(created, now());
    loop {
        match wait.next(now()) {
            WaitStep::Wait => tokio::time::sleep(PAUSE).await,
            WaitStep::GiveUp(reason) => return Err(phone_wording(wait.unattached(host, &reason))),
            WaitStep::List(within) => {
                // The connection is gone: no later listing could run either.
                let result = match run(connection, &tree_tail(), within).await {
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

/// End Session: `host` closes its session `session`, its programs and companions with it.
///
/// # Errors
///
/// [`INVALID_SESSION`] for an id that is not one plain token, otherwise why it did not end there (desktop's
/// `end_failure`) or the transport's text.
pub async fn end<C: Connection>(connection: &C, host: &str, session: &str) -> Result<(), String> {
    if !is_plain(session) {
        return Err(INVALID_SESSION.to_owned());
    }
    let result = control(connection, &end_session_arguments(session)).await?;
    end_failure(&result, host).map_or(Ok(()), Err)
}

/// `tree` without session `session` and its companions, as the far side's close took them.
pub fn forget(tree: &mut ControlRemoteTree, session: &str) {
    tree.sessions
        .retain(|listed| listed.id != session && listed.parent.as_deref() != Some(session));
}

/// The clock New Session's wait reads: tokio's, so a test's paused clock drives it.
fn now() -> std::time::Instant {
    tokio::time::Instant::now().into_std()
}
