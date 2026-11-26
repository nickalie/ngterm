//! The catalogs of headless machines on the actor ([`crate::catalog`]): a machine whose listing says it is headless is
//! followed from then on, its stream a local task on the machine's connection. The rows it reports replace the
//! listing's ([`MobileEvent::Tree`]) and its state arrives as [`MobileEvent::Catalog`].
//!
//! - The first attempt, and each after a loss, asks `version` first: a machine that advertises no usable catalog is
//!   refused for good, so its plain listing stays; one that does not answer is retried. The stream then opens with the
//!   hello; a gap or a bad frame subscribes again at once, for a fresh snapshot, and a lost stream or
//!   [`SILENCE_LIMIT`] of silence waits out [`retry_delay`] first.
//! - A lost connection stops the stream and keeps the rows (Disconnected); the reconnect starts it again. A suspend does
//!   the same until resume. A disconnect, or a connect of the machine as edited, forgets the catalog.

use std::rc::{Rc, Weak};
use std::time::Duration;

use ngterm_core::catalog::frames;
use ngterm_core::remote::not_live;
use ngterm_core::remote::origin::OriginForm;
use ngterm_core::remote::origin_replies::COMPANION_DEADLINE;
use ngterm_core::remote::ssh::{catalog_tail, control_tail};
use ngterm_proto::{ControlRemoteTree, RemotePeer, Response, VersionResult};
use tokio::sync::Notify;
use tokio::task::AbortHandle;

use super::State;
use crate::api::MobileEvent;
use crate::applog::tag;
use crate::catalog::operations::Position;
use crate::catalog::{self, CatalogState, Mirror, SILENCE_LIMIT, retry_delay};
use crate::note;
use crate::sessions::{self, RemoteSession};
use crate::transport::{ChannelEnd, ChannelOutput, ChannelWriter, Connection, Transport};

/// A machine's catalog as the actor follows it.
pub(super) struct Follow {
    pub(super) mirror: Mirror,
    task: Option<AbortHandle>,
    /// Woken each time the catalog moves or its state changes, for the actions waiting on a revision.
    pub(super) moved: Rc<Notify>,
}

impl Drop for Follow {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

/// `tree`, a listing of `machine_id` kept on its current connection: a headless ngterm that is not followed yet is from
/// now on.
pub(super) fn listed<T: Transport>(state: &Rc<State<T>>, machine_id: &str, host: &str, tree: &ControlRemoteTree) {
    if tree.headless != Some(true) || tree.endpoint.peer() != RemotePeer::Ngterm {
        return;
    }
    if state.catalogs.borrow().contains_key(machine_id) {
        return;
    }
    note!(INFO, tag::SESSIONS, "catalog followed", machine = machine_id);
    state.catalogs.borrow_mut().insert(
        machine_id.to_owned(),
        Follow {
            mirror: Mirror::new(host),
            task: None,
            moved: Rc::default(),
        },
    );
    emit_state(state, machine_id);
    start(state, machine_id);
}

/// Starts `machine_id`'s stream on its current connection, the last one replaced; nothing for a machine not followed,
/// refused, or without a connection.
pub(super) fn start<T: Transport>(state: &Rc<State<T>>, machine_id: &str) {
    let link = state
        .connections
        .borrow()
        .get(machine_id)
        .map(|link| (Rc::clone(&link.connection), super::shown_name(&link.machine)));
    let Some((connection, host)) = link else { return };
    let mut catalogs = state.catalogs.borrow_mut();
    let Some(follow) = catalogs
        .get_mut(machine_id)
        .filter(|follow| !follow.mirror.is_refused())
    else {
        return;
    };
    if let Some(task) = follow.task.take() {
        task.abort();
    }
    let run = follow_stream(Rc::downgrade(state), machine_id.to_owned(), connection, host);
    follow.task = Some(tokio::task::spawn_local(run).abort_handle());
}

/// `machine_id`'s connection went for `reason` (lost or suspended): the stream stops, the rows stay, Disconnected.
pub(super) fn stop<T: Transport>(state: &State<T>, machine_id: &str, reason: &str) {
    let changed = {
        let mut catalogs = state.catalogs.borrow_mut();
        let Some(follow) = catalogs.get_mut(machine_id) else {
            return;
        };
        if let Some(task) = follow.task.take() {
            task.abort();
        }
        let before = follow.mirror.state().clone();
        follow.mirror.connection_lost(reason.to_owned());
        follow.moved.notify_waiters();
        before != *follow.mirror.state()
    };
    if changed {
        emit_state(state, machine_id);
    }
}

/// `machine_id` was disconnected or edited: its catalog goes.
pub(super) fn forget<T: Transport>(state: &State<T>, machine_id: &str) {
    let gone = state.catalogs.borrow_mut().remove(machine_id);
    if let Some(follow) = gone {
        follow.moved.notify_waiters();
    }
}

/// The rows of `machine_id`'s catalog, with what `tree` adds, then the sessions `tree` lists of the machine's other
/// app; `None` while it has no catalog to list from.
pub(super) fn rows<T: Transport>(
    state: &State<T>,
    machine_id: &str,
    tree: Option<&ControlRemoteTree>,
) -> Option<Vec<RemoteSession>> {
    let catalogs = state.catalogs.borrow();
    let catalog = catalogs.get(machine_id)?.mirror.catalog()?;
    let mut rows = catalog::rows(catalog, tree);
    // the other app of a machine that runs both is not in the catalog: its sessions stay as listed
    rows.extend(
        tree.map(sessions::listed_sessions)
            .unwrap_or_default()
            .into_iter()
            .filter(|listed| listed.peer != Some(RemotePeer::Ngterm.into())),
    );
    Some(rows)
}

/// Reports `machine_id`'s sessions: its catalog's rows, with its kept listing's statuses and the other app's sessions,
/// or without a catalog to list from, its kept listing as it is; the listing's hint either way; nothing when it has
/// neither.
pub(super) fn report<T: Transport>(state: &State<T>, machine_id: &str) {
    let listed = {
        let listings = state.listings.borrow();
        let tree = listings.get(machine_id).and_then(|kept| kept.tree.as_ref());
        let hint = tree.and_then(not_live::hint);
        rows(state, machine_id, tree)
            .or_else(|| tree.map(sessions::listed_sessions))
            .map(|rows| (rows, hint))
    };
    if let Some((sessions, hint)) = listed {
        state.emit(MobileEvent::Tree {
            machine_id: machine_id.to_owned(),
            sessions,
            hint,
        });
    }
}

fn emit_state<T: Transport>(state: &State<T>, machine_id: &str) {
    let current = state
        .catalogs
        .borrow()
        .get(machine_id)
        .map(|follow| follow.mirror.state().clone());
    if let Some(catalog_state) = current {
        state.emit(MobileEvent::Catalog {
            machine_id: machine_id.to_owned(),
            state: catalog_state,
        });
    }
}

/// Runs `act` on `machine_id`'s mirror, then reports what changed: its state, and its rows when they moved.
fn with_mirror<T: Transport, R>(
    state: &State<T>,
    machine_id: &str,
    act: impl FnOnce(&mut Mirror) -> R,
    moved: impl FnOnce(&R) -> bool,
) -> Option<R> {
    let (answer, state_changed, rows_moved) = {
        let mut catalogs = state.catalogs.borrow_mut();
        let follow = catalogs.get_mut(machine_id)?;
        let before = follow.mirror.state().clone();
        let answer = act(&mut follow.mirror);
        let state_changed = before != *follow.mirror.state();
        let rows_moved = moved(&answer) || (state_changed && follow.mirror.is_refused());
        if state_changed || rows_moved {
            follow.moved.notify_waiters();
        }
        (answer, state_changed, rows_moved)
    };
    if state_changed {
        emit_state(state, machine_id);
    }
    if rows_moved {
        report(state, machine_id);
    }
    Some(answer)
}

/// How one subscription ended.
enum Ended {
    /// Subscribe again at once, for a fresh snapshot.
    Resubscribe,
    /// The stream was lost: retry after the next delay.
    Lost(String),
    /// The machine cannot serve a catalog, or it is no longer followed.
    Done,
}

/// The stream of `machine_id` on `connection`, until the machine cannot serve one or the task is aborted.
async fn follow_stream<T: Transport>(
    state: Weak<State<T>>,
    machine_id: String,
    connection: Rc<T::Connection>,
    host: String,
) {
    let form = OriginForm {
        peer: RemotePeer::Ngterm,
        ..connection.origin_form()
    };
    let mut attempt = 0;
    let mut probed = false;
    loop {
        let refusal = if probed {
            Ok(None)
        } else {
            probe(&*connection, form, &host).await
        };
        let ended = match refusal {
            Err(reason) => Ended::Lost(reason),
            Ok(Some(refusal)) => {
                let Some(state) = state.upgrade() else { return };
                with_mirror(&state, &machine_id, |mirror| mirror.refuse(refusal), |()| false);
                return;
            }
            Ok(None) => {
                probed = true;
                subscribe(&state, &machine_id, &connection, &host, &mut attempt).await
            }
        };
        match ended {
            Ended::Done => return,
            Ended::Resubscribe => {}
            Ended::Lost(reason) => {
                probed = false;
                {
                    let Some(state) = state.upgrade() else { return };
                    note!(
                        WARN,
                        tag::SESSIONS,
                        "catalog lost",
                        machine = machine_id.as_str(),
                        attempt = attempt
                    );
                    with_mirror(&state, &machine_id, |mirror| mirror.connection_lost(reason), |()| false);
                }
                tokio::time::sleep(retry_delay(attempt)).await;
                attempt += 1;
            }
        }
    }
}

/// `host`'s `version`: `Some` refusal when it advertises no usable catalog.
///
/// # Errors
///
/// It did not answer.
async fn probe<C: Connection>(connection: &C, form: OriginForm, host: &str) -> Result<Option<String>, String> {
    let no_answer = || format!("{host} did not answer");
    let result = sessions::run(connection, &control_tail(form, &["version"]), COMPANION_DEADLINE)
        .await
        .map_err(|_| no_answer())?;
    let reply: Response<VersionResult> = (result.status == 0)
        .then(|| serde_json::from_str(result.stdout.trim()).ok())
        .flatten()
        .filter(|reply: &Response<VersionResult>| reply.ok)
        .ok_or_else(no_answer)?;
    let versions = reply.result.and_then(|result| result.versions);
    Ok(catalog::version_refusal(host, versions.as_ref()))
}

/// One subscription: the hello, then each frame into the mirror until the stream ends or asks for a fresh snapshot.
async fn subscribe<T: Transport>(
    state: &Weak<State<T>>,
    machine_id: &str,
    connection: &Rc<T::Connection>,
    host: &str,
    attempt: &mut usize,
) -> Ended {
    {
        let Some(state) = state.upgrade() else {
            return Ended::Done;
        };
        if with_mirror(&state, machine_id, Mirror::subscribing, |()| false).is_none() {
            return Ended::Done;
        }
    }
    let mut channel = match connection.exec(&catalog_tail(connection.origin_shell()), None).await {
        Ok(channel) => channel,
        Err(error) => return Ended::Lost(error.to_string()),
    };
    if !channel.writer.write(&catalog::hello_line()) {
        return Ended::Lost(format!("the catalog stream from {host} ended"));
    }
    let mut pending = Vec::new();
    let mut stderr = Vec::new();
    let ended = loop {
        let Ok(output) = tokio::time::timeout(SILENCE_LIMIT, channel.reader.read()).await else {
            break Ended::Lost(format!("no word from {host} for {} seconds", SILENCE_LIMIT.as_secs()));
        };
        match output {
            ChannelOutput::Data(bytes) => {
                pending.extend_from_slice(&bytes);
                if let Some(ended) = take_lines(state, machine_id, &mut pending, &channel.writer, attempt) {
                    break ended;
                }
            }
            ChannelOutput::Stderr(bytes) => stderr.extend_from_slice(&bytes),
            ChannelOutput::End(end) => {
                let said = String::from_utf8_lossy(&stderr).trim().to_owned();
                let reason = match end {
                    _ if !said.is_empty() => said,
                    ChannelEnd::Lost(reason) => reason,
                    ChannelEnd::Exit { code, .. } => {
                        code.map_or_else(|| "it was stopped".to_owned(), |code| format!("exit status {code}"))
                    }
                };
                break Ended::Lost(format!("the catalog stream from {host} ended: {reason}"));
            }
        }
    };
    // a channel already gone has nothing to close
    let _ = channel.writer.close();
    ended
}

/// Feeds every whole line of `pending` to the mirror, answering on `writer`; how the subscription ends, if a line ended
/// it.
fn take_lines<T: Transport>(
    state: &Weak<State<T>>,
    machine_id: &str,
    pending: &mut Vec<u8>,
    writer: &ChannelWriter,
    attempt: &mut usize,
) -> Option<Ended> {
    while let Some(at) = pending.iter().position(|&byte| byte == b'\n') {
        let line: Vec<u8> = pending.drain(..=at).collect();
        let Some(state) = state.upgrade() else {
            return Some(Ended::Done);
        };
        let Ok(frame) = frames::decode(&line[..line.len() - 1]) else {
            note!(WARN, tag::SESSIONS, "catalog frame unreadable", machine = machine_id);
            return Some(Ended::Resubscribe);
        };
        let received = with_mirror(
            &state,
            machine_id,
            |mirror| (mirror.receive(frame), mirror.state().clone(), mirror.needs_snapshot()),
            |(received, ..)| received.moved.is_some(),
        );
        let Some((received, catalog_state, needs_snapshot)) = received else {
            return Some(Ended::Done);
        };
        if let Some(reply) = received.reply.as_ref().and_then(|reply| frames::encode(reply).ok()) {
            // a stream gone under the ack ends on its own
            let _ = writer.write(&reply);
        }
        match catalog_state {
            CatalogState::Connected => *attempt = 0,
            CatalogState::Refused { .. } => return Some(Ended::Done),
            CatalogState::Connecting | CatalogState::Disconnected { .. } => {}
        }
        if needs_snapshot {
            return Some(Ended::Resubscribe);
        }
    }
    None
}

/// Where `machine_id`'s catalog is, for an action decided now; `None` unless its stream is up.
pub(super) fn position<T: Transport>(state: &State<T>, machine_id: &str) -> Option<Position> {
    let catalogs = state.catalogs.borrow();
    let mirror = &catalogs.get(machine_id)?.mirror;
    let catalog = mirror
        .catalog()
        .filter(|_| *mirror.state() == CatalogState::Connected)?;
    Some(Position {
        epoch: catalog.epoch.clone(),
        rev: catalog.rev,
    })
}

/// What `check` finds in `machine_id`'s mirror, as soon as it does, within `budget`; `None` when it runs out or the
/// catalog is forgotten.
pub(super) async fn wait_for<T: Transport, R>(
    state: &State<T>,
    machine_id: &str,
    budget: Duration,
    check: impl Fn(&Mirror) -> Option<R>,
) -> Option<R> {
    let until = tokio::time::Instant::now() + budget;
    loop {
        let moved = Rc::clone(&state.catalogs.borrow().get(machine_id)?.moved);
        // made before the check, so a move between the check and the wait still wakes it
        let notified = moved.notified();
        if let Some(found) = check(&state.catalogs.borrow().get(machine_id)?.mirror) {
            return Some(found);
        }
        tokio::time::timeout_at(until, notified).await.ok()?;
    }
}

/// `machine_id`'s catalog without session `id` and its companions, which a plain End Session closed there; the stream
/// brings the same change later.
pub(super) fn forget_session<T: Transport>(state: &State<T>, machine_id: &str, id: &str) {
    if let Some(follow) = state.catalogs.borrow_mut().get_mut(machine_id) {
        follow.mirror.forget(id);
    }
}

/// Why `session_id`, a row of `machine_id`'s catalog, is not attached: a page shows no terminal, and an ended session
/// with no scratch has nothing to show (upstream's `the session has ended on HOST`). `None` for any other row.
pub(super) fn attach_refusal<T: Transport>(state: &State<T>, machine_id: &str, session_id: &str) -> Option<String> {
    let catalogs = state.catalogs.borrow();
    let mirror = &catalogs.get(machine_id)?.mirror;
    let session = catalog::session(mirror.catalog()?, session_id)?;
    if catalog::is_page(session) {
        return Some(format!(
            "{} is a page on {}; this app shows terminal sessions only",
            session.fields.name,
            mirror.host()
        ));
    }
    catalog::has_ended(session).then(|| format!("the session has ended on {}", mirror.host()))
}

/// The row of session `id` once `machine_id`'s catalog shows it attachable (with its panes) or ended, within `budget`.
pub(super) async fn shown<T: Transport>(
    state: &State<T>,
    machine_id: &str,
    id: &str,
    budget: Duration,
) -> Option<RemoteSession> {
    wait_for(state, machine_id, budget, |mirror| {
        let catalog = mirror.catalog()?;
        catalog::session(catalog, id)
            .filter(|session| !session.panes.is_empty() || session.fields.lifecycle == frames::Lifecycle::Ended)?;
        let listings = state.listings.borrow();
        let tree = listings.get(machine_id).and_then(|kept| kept.tree.as_ref());
        catalog::rows(catalog, tree)
            .into_iter()
            .find(|row| row.id.eq_ignore_ascii_case(id))
    })
    .await
}

/// Whether `machine_id`'s catalog holds session `id`.
pub(super) fn holds<T: Transport>(state: &State<T>, machine_id: &str, id: &str) -> bool {
    let catalogs = state.catalogs.borrow();
    catalogs
        .get(machine_id)
        .and_then(|follow| follow.mirror.catalog())
        .is_some_and(|catalog| catalog::session(catalog, id).is_some())
}
