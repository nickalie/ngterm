//! Rows attached from other machines come back after a relaunch, an extension beyond upstream, whose relaunch drops
//! them. Each window's snapshot lists them and its store holds them as saved rows until they attach again
//! (`model::saved_remote_rows`); here every machine is listed once for each app its saved rows came from, and each
//! saved row it still offers attaches where it stood, a companion under its parent's row, nothing selected. A session
//! the machine no longer offers, or one that cannot be attached, is dropped and named in a banner, as is a saved row
//! whose workspace was closed meanwhile, silently. A machine that cannot be listed (asleep, offline, ssh refused) is
//! listed again on the reconnect schedule ([`super::reconnect::retry_delay`]) and its rows stay saved, so a quit before
//! it answers keeps them for the next launch, until they have waited [`GIVE_UP_AFTER`] through failures in a row (and
//! [`GIVE_UP_IN_RUN`] of them in one run). Attaching the session by hand meanwhile takes its saved row over, and closing
//! an attached row is what forgets it.

use std::collections::HashMap;
use std::time::Instant;

use ngterm_proto::ControlRemoteTree;
use uuid::Uuid;

use super::attach::Placement;
use super::frames;
use super::origin_replies::read_peer_tree;
use super::peer::RemotePeer;
use super::reconnect::retry_delay;
use super::tree;
use super::viewer::{Answer, run_tree_off_thread, take_answer};
use crate::Core;
use crate::model::AppStore;
use crate::persist::RemoteSessionSnapshot;

/// What a banner about a machine's dropped rows is headed by.
const BANNER_TITLE: &str = "Attach Remote";
/// How long a machine's saved rows wait through failed listings in a row before they are dropped: a machine gone for
/// good is not listed on every launch forever, and a laptop away for a weekend keeps its rows.
pub(crate) const GIVE_UP_AFTER: std::time::Duration = std::time::Duration::from_hours(7 * 24);
/// How long the listings must also keep failing in this run before [`GIVE_UP_AFTER`] drops anything: the days ngterm
/// was not running count toward the week, but a launch that fails once before the network is up drops nothing.
pub(crate) const GIVE_UP_IN_RUN: std::time::Duration = std::time::Duration::from_hours(1);

/// What became of one saved row once its machine answered.
enum Reattached {
    /// Attached, or a row here already shows its session.
    Attached,
    /// The machine no longer offers it; its name.
    Gone(String),
    /// The attach failed; its name and why.
    Failed(String),
    /// Its workspace was closed while it waited.
    Left,
}

/// One machine's app whose saved rows wait for its listing: a machine running both apps is listed once for each, and
/// each listing is pinned to its app, so a row is never matched against the other app's sessions and dropped as gone.
#[derive(Debug)]
pub(crate) struct RelaunchRead {
    host: String,
    peer: RemotePeer,
    /// The listing in flight; `None` while it waits for `retry_at`.
    answer: Option<Answer>,
    /// The listings in a row that failed.
    failures: u32,
    /// When the first of those failed, in this run.
    failing_since: Option<Instant>,
    retry_at: Instant,
}

impl RelaunchRead {
    fn key(&self) -> (String, RemotePeer) {
        (self.host.clone(), self.peer)
    }
}

impl Core {
    /// Starts attaching again the rows the open windows saved: one listing per machine and app the saved rows name, none
    /// for one already being listed. The launch runs it once its windows are restored, and a window reopened from its file runs it
    /// again. Commit afterwards.
    pub fn reattach_saved_remotes(&mut self) {
        let now = Instant::now();
        for (host, peer) in self.saved_remote_hosts() {
            let reads = &mut self.remote.viewer.relaunch_reads;
            if reads.iter().all(|read| read.host != host || read.peer != peer) {
                reads.push(RelaunchRead {
                    host,
                    peer,
                    answer: None,
                    failures: 0,
                    failing_since: None,
                    retry_at: now,
                });
            }
        }
        self.tick_relaunch(now);
    }

    /// The machines the open windows' saved rows name, each with the app they were attached from, in their order.
    fn saved_remote_hosts(&self) -> Vec<(String, RemotePeer)> {
        let library = self.library();
        let mut hosts: Vec<(String, RemotePeer)> = Vec::new();
        let saved = library
            .open_ids()
            .into_iter()
            .filter_map(|window| library.store(window))
            .flat_map(AppStore::saved_remote_rows);
        for row in saved {
            let key = (row.host.clone(), row.peer());
            if !hosts.contains(&key) {
                hosts.push(key);
            }
        }
        hosts
    }

    /// Whether a listing is in flight (`Some(true)`), only waiting for its retry (`Some(false)`), or none is left.
    pub(super) fn relaunch_pending(&self) -> Option<bool> {
        let reads = &self.remote.viewer.relaunch_reads;
        (!reads.is_empty()).then(|| reads.iter().any(|read| read.answer.is_some()))
    }

    /// Every machine waiting for its retry is listed now.
    pub(super) fn retry_relaunch_now(&mut self, now: Instant) {
        for read in &mut self.remote.viewer.relaunch_reads {
            read.retry_at = read.retry_at.min(now);
        }
    }

    /// Starts the due listings and attaches the rows of each one that answered.
    pub(super) fn tick_relaunch(&mut self, now: Instant) {
        let reads = std::mem::take(&mut self.remote.viewer.relaunch_reads);
        let mut kept = Vec::with_capacity(reads.len());
        for read in reads {
            if let Some(read) = self.step_relaunch(read, now) {
                kept.push(read);
            }
        }
        // a read another call added meanwhile stays after the ones that were already waiting
        kept.append(&mut self.remote.viewer.relaunch_reads);
        self.remote.viewer.relaunch_reads = kept;
    }

    fn step_relaunch(&mut self, mut read: RelaunchRead, now: Instant) -> Option<RelaunchRead> {
        // the windows holding its rows closed meanwhile: a reopen lists it again
        if !self.saved_remote_hosts().contains(&read.key()) {
            return None;
        }
        let Some(answer) = read.answer.as_ref() else {
            if read.retry_at > now {
                return Some(read);
            }
            if tree::validate(&read.host).is_err() {
                // a host ssh is never handed cannot become attachable later
                let names = self.forget_saved_rows_of(&read.host);
                self.report_dropped_rows(&read, "invalid", &names, "an invalid host");
                return None;
            }
            let listing = super::peer::pinned(&read.host, read.peer);
            match run_tree_off_thread(&self.remote.runner, &listing, tree::TREE_DEADLINE) {
                Some(answer) => read.answer = Some(answer),
                None => return self.relaunch_failed(read, now, "the listing could not start"),
            }
            return Some(read);
        };
        let Some(result) = take_answer(answer) else {
            return Some(read);
        };
        match read_peer_tree(&result, &read.host) {
            Ok(tree) => {
                self.reattach_from(&tree, &read);
                None
            }
            Err(error) => self.relaunch_failed(read, now, &error),
        }
    }

    /// `read`'s machine could not be listed: it is listed again on the reconnect schedule, and once this run's listings
    /// have failed for [`GIVE_UP_IN_RUN`], the saved rows that have waited longer than [`GIVE_UP_AFTER`] are dropped.
    /// `None` once none of its rows is left.
    fn relaunch_failed(&mut self, mut read: RelaunchRead, now: Instant, error: &str) -> Option<RelaunchRead> {
        read.failures += 1;
        let failing_since = *read.failing_since.get_or_insert(now);
        let limit =
            (now.saturating_duration_since(failing_since) >= GIVE_UP_IN_RUN).then(|| GIVE_UP_AFTER.as_secs_f64());
        read.retry_at = now + retry_delay(read.failures);
        read.answer = None;
        tracing::info!(
            host = read.host,
            failures = read.failures,
            error,
            "saved remote rows wait"
        );
        let epoch = crate::projection::epoch_seconds(std::time::SystemTime::now());
        let mut expired = Vec::new();
        for window in self.library().open_ids() {
            let Some(store) = self.library_mut().store_mut(window) else {
                continue;
            };
            if store
                .saved_remote_rows()
                .iter()
                .all(|row| row.host != read.host || row.peer() != read.peer)
            {
                continue;
            }
            expired.extend(store.wait_saved_remote_rows((&read.host, read.peer), epoch, limit));
            // the start of the wait is written, so the limit counts across launches
            self.request_window_save(window);
        }
        self.report_dropped_rows(&read, "expired", &expired, "no answer for a week");
        self.saved_remote_hosts().contains(&read.key()).then_some(read)
    }

    /// Attaches every saved row of `read`'s machine and app that `tree` still offers, parents first so each companion
    /// finds its parent's row, and drops the rest.
    fn reattach_from(&mut self, tree: &ControlRemoteTree, read: &RelaunchRead) {
        let host = read.host.as_str();
        let mut rows = self.rows_attached_from(host, &tree.endpoint);
        let mut gone: Vec<String> = Vec::new();
        let mut failed: Vec<String> = Vec::new();
        for window in self.library().open_ids() {
            let Some(store) = self.library().store(window) else {
                continue;
            };
            let mut saved: Vec<RemoteSessionSnapshot> = store
                .saved_remote_rows()
                .iter()
                .filter(|row| row.host == host && row.peer() == read.peer)
                .cloned()
                .collect();
            if saved.is_empty() {
                continue;
            }
            saved.sort_by_key(|row| (row.parent_session_id.is_some(), row.order));
            for row in saved {
                match self.reattach_saved_row(window, row, tree, &mut rows) {
                    Reattached::Attached | Reattached::Left => {}
                    Reattached::Gone(name) => gone.push(name),
                    Reattached::Failed(reason) => failed.push(reason),
                }
            }
            self.request_window_save(window);
        }
        self.report_dropped_rows(read, "gone", &gone, "gone there");
        if !failed.is_empty() {
            tracing::warn!(host, failures = failed.join("; "), "saved remote rows did not attach");
            self.report_dropped_rows(read, "failed", &failed, "could not be attached");
        }
    }

    /// Attaches saved `row` of `window` from `tree` and records its row in `rows`; the row is no longer saved either
    /// way. A session a row here already shows (attached by hand meanwhile, or saved in another window too) is not
    /// attached twice.
    fn reattach_saved_row(
        &mut self,
        window: Uuid,
        row: RemoteSessionSnapshot,
        tree: &ControlRemoteTree,
        rows: &mut HashMap<String, Uuid>,
    ) -> Reattached {
        let placement = self.saved_row_placement(window, &row, rows);
        if let Some(store) = self.library_mut().store_mut(window) {
            store.forget_saved_remote_row(&row.host, &row.session_id);
        }
        if rows.contains_key(&row.session_id) {
            return Reattached::Attached;
        }
        let Some(placement) = placement else {
            tracing::info!(
                host = row.host,
                session = row.name,
                "a saved remote row's workspace was closed"
            );
            return Reattached::Left;
        };
        let remote = tree
            .sessions
            .iter()
            .find(|candidate| candidate.id == row.session_id && candidate.content.is_none());
        let Some(remote) = remote else {
            return Reattached::Gone(row.name);
        };
        let result = match self.attach_remote_session(remote, tree, &row.host, placement) {
            Ok(result) => result,
            Err(error) => return Reattached::Failed(format!("{}: {error}", row.name)),
        };
        if let Some(id) = result.id.as_deref().and_then(frames::parse_uuid) {
            if let Some(store) = self.library_mut().store_mut(placement.window) {
                if let Some(name) = row.local_name.as_deref() {
                    store.rename_session(id, name);
                }
                if placement.parent.is_none() {
                    store.record_reattached_place(id, (row.after_session_id, row.order));
                }
            }
            rows.insert(row.session_id, id);
        }
        Reattached::Attached
    }

    /// Where saved `row` of `window` goes: `None` once its workspace is closed, else under its parent's row when that
    /// is attached, else where it stood in its workspace.
    fn saved_row_placement(
        &self,
        window: Uuid,
        row: &RemoteSessionSnapshot,
        rows: &HashMap<String, Uuid>,
    ) -> Option<Placement> {
        // a closed workspace's rows come back nowhere, a companion under a parent attached elsewhere included
        let at = self.library().store(window)?.saved_remote_row_index(row)?;
        let parent = row
            .parent_session_id
            .as_ref()
            .and_then(|parent| rows.get(parent).copied());
        if let Some(placement) = parent.and_then(|parent| Placement::under(self.library(), parent, false)) {
            return Some(placement);
        }
        Some(Placement {
            window,
            workspace: row.workspace_id,
            at: Some(at),
            parent: None,
            select: false,
        })
    }

    /// Drops the saved row of `host`'s session `session` in every open window: a row now shows that session.
    pub(super) fn forget_saved_remote_session(&mut self, host: &str, session: &str) {
        for window in self.library().open_ids() {
            let forgot = self
                .library_mut()
                .store_mut(window)
                .is_some_and(|store| store.forget_saved_remote_row(host, session));
            if forgot {
                self.request_window_save(window);
            }
        }
    }

    /// Drops every saved row of `host` in the open windows; the names of the rows dropped.
    fn forget_saved_rows_of(&mut self, host: &str) -> Vec<String> {
        let mut names = Vec::new();
        for window in self.library().open_ids() {
            let Some(store) = self.library_mut().store_mut(window) else {
                continue;
            };
            let rows: Vec<RemoteSessionSnapshot> = store
                .saved_remote_rows()
                .iter()
                .filter(|row| row.host == host)
                .cloned()
                .collect();
            if rows.is_empty() {
                continue;
            }
            for row in rows {
                store.forget_saved_remote_row(host, &row.session_id);
                names.push(row.name);
            }
            self.request_window_save(window);
        }
        names
    }

    /// The banner naming the saved rows of `read`'s machine that did not come back, and why; `kind`, and the app for an
    /// app other than ngterm, keep one reason's or one app's banner from replacing another's.
    fn report_dropped_rows(&mut self, read: &RelaunchRead, kind: &str, names: &[String], why: &str) {
        if names.is_empty() {
            return;
        }
        let host = &read.host;
        let what = if names.len() == 1 { "session" } else { "sessions" };
        let body = format!(
            "{} {what} from {host} not attached again ({why}): {}",
            names.len(),
            names.join(", ")
        );
        let app = match read.peer {
            RemotePeer::Ngterm => String::new(),
            RemotePeer::Agterm => format!("-{}", RemotePeer::Agterm.name()),
        };
        self.post_app_banner(format!("remote-relaunch-{kind}-{host}{app}"), BANNER_TITLE, body);
    }

    /// Asks for `window`'s snapshot to be written now, so a dropped saved row stays dropped.
    fn request_window_save(&mut self, window: Uuid) {
        if let Some(store) = self.library_mut().store_mut(window) {
            store.request_save(crate::model::SaveRequest::Now);
        }
    }
}

#[cfg(test)]
#[path = "relaunch_tests.rs"]
mod tests;
