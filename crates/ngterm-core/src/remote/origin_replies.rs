//! What another machine's `ngtermctl` answers to the lines a viewer runs there over ssh, read with no [`crate::Core`]:
//! its attachable sessions (`live tree`), the three stages of New Session (`restore mode`, `session new --no-select`,
//! then listing it until the new session is attachable), End Session (`session close --target`) and a companion's
//! create. The desktop flows (`origin_sessions`, `companions`, `attach`) and the mobile app share these decisions and
//! their strings.

use std::time::{Duration, Instant};

use ngterm_proto::{ControlRemoteSession, ControlRemoteTree, ControlResult, Response};

use super::frames;
use super::tree::{self, RemoteCommandResult, TREE_DEADLINE};

/// How long a line run on the origin (`restore mode`, `session new`, `session close`, a companion's create) may take
/// (upstream's `remoteCompanionDeadline`).
pub const COMPANION_DEADLINE: Duration = Duration::from_secs(30);
/// How long a created session may take to become attachable: every pane's daemon must run first (upstream's
/// `remoteCompanionReadyBudget`).
pub const READY_BUDGET: Duration = Duration::from_secs(20);
/// The pause between two listings while waiting for it (upstream's `remoteCompanionPause`).
pub const PAUSE: Duration = Duration::from_secs(1);
/// Why a created session is not attached yet, until a listing says otherwise.
pub const NOT_ATTACHABLE: &str = "it did not become attachable";
/// What a created session is called in the messages about it.
pub const SESSION_NOUN: &str = "session";
/// The restore mode `restore.mode` reports while each pane runs in a live session.
pub const LIVE_MODE: &str = "live";
/// New Session's first line: whether the origin runs live sessions.
pub const RESTORE_MODE_ARGUMENTS: [&str; 2] = ["restore", "mode"];
/// New Session's second line: the origin keeps its own selection, as the session is wanted here, not there.
pub const NEW_SESSION_ARGUMENTS: [&str; 3] = ["session", "new", "--no-select"];

/// Duplicate Session's second line: the origin duplicates `origin` in its current focused directory and selects the
/// copy, as its own Duplicate does.
#[must_use]
pub fn duplicate_session_arguments(origin: &str) -> [&str; 4] {
    ["session", "duplicate", "--target", origin]
}

/// End Session's line for the origin's session `origin`.
#[must_use]
pub fn end_session_arguments(origin: &str) -> [&str; 4] {
    ["session", "close", "--target", origin]
}

/// The receipt the origin's `ngtermctl --json` printed.
#[must_use]
pub fn reply_of(result: &RemoteCommandResult) -> Option<Response<ControlResult>> {
    serde_json::from_str(result.stdout.trim()).ok()
}

/// The text after "gave no answer": what ssh said, if anything.
#[must_use]
pub fn no_answer(result: &RemoteCommandResult, host: &str, rest: &str) -> String {
    let stderr = result.stderr.trim();
    if stderr.is_empty() {
        format!("{host} gave no answer; {rest}")
    } else {
        format!("{host} gave no answer ({stderr}); {rest}")
    }
}

/// The far CLI refused its command line, before sending anything.
#[must_use]
pub fn too_old(host: &str) -> String {
    format!("the ngterm on {host} is too old for this; nothing was created")
}

/// Why `host`'s answer to `restore.mode` rules a new session out; `None` when it runs live sessions.
#[must_use]
pub fn not_live(result: &RemoteCommandResult, host: &str) -> Option<String> {
    if result.refused_arguments() {
        return Some(too_old(host));
    }
    let Some(reply) = reply_of(result) else {
        return Some(no_answer(result, host, "nothing was created"));
    };
    if !reply.ok {
        let reason = reply
            .error
            .unwrap_or_else(|| format!("the remote command failed on {host}"));
        return Some(format!("{reason}; nothing was created"));
    }
    let active = reply
        .result
        .and_then(|result| result.restore)
        .map(|restore| restore.active);
    match active.as_deref() {
        Some(LIVE_MODE) => None,
        Some(mode) => Some(format!(
            "{host} does not run live sessions (restore mode {mode}), so a session made there could not attach; \
             nothing was created"
        )),
        None => Some(format!("{host} did not name its restore mode; nothing was created")),
    }
}

/// What the origin's close said: `None` when the session ended there, else why it did not. A full receipt counts
/// whatever the transport did afterwards, since it names work the origin did.
#[must_use]
pub fn end_failure(result: &RemoteCommandResult, host: &str) -> Option<String> {
    let Some(reply) = reply_of(result) else {
        return Some(no_answer(result, host, "whether the session ended there is unknown"));
    };
    (!reply.ok).then(|| {
        reply
            .error
            .unwrap_or_else(|| format!("the remote command failed on {host}"))
    })
}

/// The id a reply names, only when it is a well-formed one: it reaches an error message.
fn created_id(reply: &Response<ControlResult>) -> Option<String> {
    reply
        .result
        .as_ref()
        .and_then(|result| result.id.clone())
        .filter(|id| frames::parse_uuid(id).is_some())
}

/// What an origin's create of a `noun` said: the created session's id.
///
/// # Errors
///
/// Why there is none: no answer, a refusal, a receipt the transport failed after, or a receipt naming no id.
pub fn origin_created(result: &RemoteCommandResult, host: &str, noun: &str) -> Result<String, String> {
    let Some(reply) = reply_of(result) else {
        return Err(no_answer(
            result,
            host,
            &format!("whether the {noun} was created there is unknown"),
        ));
    };
    // a full receipt with a failed transport still names work the origin did, so it is reported, never redone
    if result.status != 0
        && reply.ok
        && let Some(id) = created_id(&reply)
    {
        return Err(format!(
            "created {noun} {id} on {host} but the connection failed before attaching it; `ngtermctl live attach \
             {host} {id}` attaches it"
        ));
    }
    if result.status != 0 || !reply.ok {
        return Err(reply
            .error
            .unwrap_or_else(|| format!("the remote command failed on {host}")));
    }
    created_id(&reply).ok_or_else(|| format!("{host} created a {noun} but did not name it"))
}

/// The failure of a `noun` created on `host` but not attached here.
#[must_use]
pub fn unattached(noun: &str, created: &str, host: &str, reason: &str) -> String {
    format!(
        "created {noun} {created} on {host} but could not attach it: {reason}; `ngtermctl live attach {host} \
         {created}` attaches it"
    )
}

/// What a finished remote read says: the far side's own refusal from stdout first, since its ngtermctl prints it there
/// and exits nonzero; ssh's failures reach stderr. An ok-looking payload from a nonzero process is never accepted
/// (upstream's `remoteTree(host:)` remote form).
///
/// # Errors
///
/// The refusal or failure as the caller reads it.
pub fn read_remote_tree(result: &RemoteCommandResult, host: &str) -> Result<ControlRemoteTree, String> {
    if result.status != 0 {
        let stderr = result.stderr.trim();
        let detail = tree::remote_error(&result.stdout).or_else(|| (!stderr.is_empty()).then(|| stderr.to_owned()));
        return Err(detail.unwrap_or_else(|| format!("the remote command failed on {host}")));
    }
    let remote = tree::decode(&result.stdout).map_err(|error| error.message())?;
    super::peer::learn(host, remote.endpoint.peer());
    Ok(remote.stamped(host))
}

/// Whether a listed session is one a picker offers: a page companion attaches only under its parent's row, which a
/// picker has no place for.
#[must_use]
pub fn is_pickable(session: &ControlRemoteSession) -> bool {
    session.content.is_none()
}

/// The sessions of `tree` a picker offers, parents before their companions so each companion finds its parent's row.
#[must_use]
pub fn pickable_sessions(tree: &ControlRemoteTree) -> Vec<&ControlRemoteSession> {
    let mut sessions: Vec<&ControlRemoteSession> =
        tree.sessions.iter().filter(|session| is_pickable(session)).collect();
    sessions.sort_by_key(|session| session.parent.is_some());
    sessions
}

/// New Session's first stage: `host`'s answer to `restore mode`.
///
/// # Errors
///
/// Why no session is created there ([`not_live`]).
pub fn check_restore_mode(result: &RemoteCommandResult, host: &str) -> Result<(), String> {
    not_live(result, host).map_or(Ok(()), Err)
}

/// New Session's second stage: `host`'s answer to `session new --no-select`, the created session's id.
///
/// # Errors
///
/// Why there is none ([`too_old`], [`origin_created`]).
pub fn session_created(result: &RemoteCommandResult, host: &str) -> Result<String, String> {
    if result.refused_arguments() {
        return Err(too_old(host));
    }
    origin_created(result, host, SESSION_NOUN)
}

/// What New Session's last stage does next while no listing runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaitStep {
    /// Nothing until the pause after the last listing is over.
    Wait,
    /// Start a listing, given this long to answer.
    List(Duration),
    /// The budget is spent: the reason the created session is not attached.
    GiveUp(String),
}

/// New Session's last stage: the origin is listed every [`PAUSE`] within [`READY_BUDGET`] until the created session
/// is attachable, as every pane's daemon must run first.
#[derive(Debug, Clone)]
pub struct ListingWait {
    created: String,
    deadline: Instant,
    /// When the next listing starts.
    due: Instant,
    /// Why it is not attached yet, for the failure if it never is.
    reason: String,
}

impl ListingWait {
    /// The wait for session `created`, starting at `now` with a listing at once.
    #[must_use]
    pub fn new(created: String, now: Instant) -> Self {
        Self {
            created,
            deadline: now + READY_BUDGET,
            due: now,
            reason: NOT_ATTACHABLE.to_owned(),
        }
    }

    /// The created session's id.
    #[must_use]
    pub fn created(&self) -> &str {
        &self.created
    }

    /// What to do at `now`, with no listing running.
    #[must_use]
    pub fn next(&self, now: Instant) -> WaitStep {
        if now < self.due {
            return WaitStep::Wait;
        }
        let remaining = self.deadline.saturating_duration_since(now);
        if remaining.is_zero() {
            return WaitStep::GiveUp(self.reason.clone());
        }
        WaitStep::List(remaining.min(TREE_DEADLINE))
    }

    /// A listing of `host` finished at `now`: the created session and the listing once it is there; otherwise the
    /// next listing is due after a [`PAUSE`], and a failed one is the reason a give-up names.
    pub fn listed(
        &mut self,
        result: &RemoteCommandResult,
        host: &str,
        now: Instant,
    ) -> Option<(ControlRemoteSession, ControlRemoteTree)> {
        match read_remote_tree(result, host) {
            Ok(tree) => {
                if let Some(session) = tree.sessions.iter().find(|session| session.id == self.created) {
                    return Some((session.clone(), tree));
                }
            }
            Err(error) => self.reason = error,
        }
        self.due = now + PAUSE;
        None
    }

    /// The failure of the created session not attached here, for `reason`.
    #[must_use]
    pub fn unattached(&self, host: &str, reason: &str) -> String {
        unattached(SESSION_NOUN, &self.created, host, reason)
    }
}

#[cfg(test)]
#[path = "origin_replies_tests.rs"]
mod tests;
