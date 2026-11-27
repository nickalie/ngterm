//! A remote window's actions on the core (upstream's `ControlServer+RemoteActions.swift` and
//! `AppActions+RemoteCatalog.swift`): a control request or a gesture that makes, changes or ends a row goes to the
//! window's server, its mark shows on the row until the catalog settles it, and its reply goes back to the asker.

use std::time::Duration;

use ngterm_proto::tree::ControlTree;
use ngterm_proto::{ControlRemoteOperation, ControlResult, Response};
use uuid::Uuid;

use super::actions::{Asker, Finished, lost_reply};
use super::operation::{RemoteOperation, RemoteOperationTarget};
use super::run::CatalogRun;
use crate::Core;
use crate::dispatch::Dispatched;
use crate::error::ControlError;
use crate::model::AppStore;
use crate::remote::frames;

/// How often a control request whose action is out looks for its reply.
const REPLY_POLL: Duration = Duration::from_millis(20);

impl Core {
    /// Sends `operation` to remote window `window`'s server for the window's own controls; false for a local window,
    /// whose change is the caller's. A refusal shows over the row it names, else over the selected one. `select`
    /// selects the row it creates once the window shows it.
    pub(crate) fn perform_remote(&mut self, window: Uuid, operation: RemoteOperation, select: bool) -> bool {
        if !self.library.is_remote_window(window) {
            return false;
        }
        let label = operation.label();
        let target = operation.target();
        if let Err(error) = self.send_remote(window, operation, Asker::Gui, select) {
            self.report_remote_refusal(window, label, target, &error);
        }
        true
    }

    /// Sends `operation` for a control request and answers with the server's reply once it comes.
    pub(crate) fn begin_remote_operation(
        &mut self,
        window: Uuid,
        operation: RemoteOperation,
        select: bool,
    ) -> Dispatched {
        let lost = lost_reply(&self.remote_host(window), operation.label());
        match self.send_remote(window, operation, Asker::Control, select) {
            Ok(op_id) => {
                let client = self.remote.viewer.catalogs.get(&window).map(CatalogRun::client);
                await_reply(window, client, op_id, lost)
            }
            Err(error) => Dispatched::done(Err(ControlError::rejected(error))),
        }
    }

    fn send_remote(
        &mut self,
        window: Uuid,
        operation: RemoteOperation,
        asker: Asker,
        select: bool,
    ) -> Result<String, String> {
        let selection = self.library.store(window).and_then(AppStore::selected_session_id);
        let host = self.remote_host(window);
        let Some(run) = self.remote.viewer.catalogs.get_mut(&window) else {
            return Err(super::actions::not_connected(&host));
        };
        let link = super::actions::link(&run.stream);
        let prepared = run.actions.prepare(link, operation, selection)?;
        let sent = run.actions.send(link, &self.remote.runner, prepared, asker, select);
        self.remote_actions_changed(window);
        sent
    }

    fn remote_host(&self, window: Uuid) -> String {
        self.library
            .remote_source(window)
            .map(|source| source.host.clone())
            .unwrap_or_default()
    }

    /// Takes the answers of window `window`'s actions that came.
    pub(crate) fn poll_remote_actions(&mut self, window: Uuid) {
        let Some(run) = self.remote.viewer.catalogs.get_mut(&window) else {
            return;
        };
        let link = super::actions::link(&run.stream);
        let finished = run.actions.poll(link, &self.remote.runner);
        for done in finished {
            self.remote_action_finished(window, &done);
        }
        self.remote_actions_changed(window);
    }

    /// Window `window`'s catalog moved; `snapshot` for a whole one.
    pub(crate) fn remote_catalog_moved(&mut self, window: Uuid, snapshot: bool) {
        let Some(run) = self.remote.viewer.catalogs.get_mut(&window) else {
            return;
        };
        let link = super::actions::link(&run.stream);
        run.actions.catalog_moved(link, &self.remote.runner, snapshot);
        self.remote_actions_changed(window);
    }

    fn remote_action_finished(&mut self, window: Uuid, done: &Finished) {
        if let Some((created, baseline)) = done.created {
            self.select_when_shown(window, created, baseline);
        }
        if done.asker == Asker::Gui && !done.response.ok {
            let error = done
                .response
                .error
                .clone()
                .unwrap_or_else(|| format!("{} was not done", done.operation.label()));
            self.report_remote_refusal(window, done.operation.label(), done.operation.target(), &error);
        }
        tracing::debug!(%window, op_id = %done.op_id, ok = done.response.ok, "a remote window's action answered");
    }

    /// Selects row `created` once the catalog brings it, unless the selection moved from `baseline` (upstream's
    /// `selectWhenShown`).
    fn select_when_shown(&mut self, window: Uuid, created: Uuid, baseline: Option<Uuid>) {
        let Some(store) = self.library.store_mut(window) else {
            return;
        };
        if store.selected_session_id() != baseline {
            return;
        }
        if store.session(created).is_some() {
            store.select_session(Some(created));
            return;
        }
        if let Some(run) = self.remote.viewer.catalogs.get_mut(&window) {
            run.pending_selection = Some((created, baseline));
        }
    }

    /// Shows why the server did not take an action, over the row it named or the selected one.
    fn report_remote_refusal(&mut self, window: Uuid, label: &str, target: Option<RemoteOperationTarget>, error: &str) {
        let store = self.library.store(window);
        let named = match target {
            Some(RemoteOperationTarget::Session(id)) => store.and_then(|store| store.session(id)).map(|_| id),
            _ => None,
        };
        let row = named.or_else(|| store.and_then(AppStore::selected_session_id));
        self.action_failed(label, window, row, error);
    }

    fn remote_actions_changed(&mut self, window: Uuid) {
        let changed = self
            .remote
            .viewer
            .catalogs
            .get_mut(&window)
            .is_some_and(|run| run.actions.take_changed());
        if changed && let Some(store) = self.library.store_mut(window) {
            store.schedule_tree_changed();
        }
    }

    /// The `remote` node's operations of window `window`: still pending or with an unknown outcome; `None` when there
    /// are none.
    pub(crate) fn remote_operations(&self, window: Uuid) -> Option<Vec<ControlRemoteOperation>> {
        let marks = self.remote.viewer.catalogs.get(&window)?.actions.marks();
        (!marks.is_empty()).then(|| marks.iter().map(ControlRemoteOperation::from).collect())
    }

    /// Each row of remote window `window`'s tree reads back its operations' state (`tree.session.pending`).
    pub(crate) fn project_remote_pending(&self, window: Option<Uuid>, tree: &mut ControlTree) {
        let Some(run) = window.and_then(|window| self.remote.viewer.catalogs.get(&window)) else {
            return;
        };
        for node in tree
            .workspaces
            .iter_mut()
            .flat_map(|workspace| workspace.sessions.iter_mut())
        {
            node.pending = frames::parse_uuid(&node.id)
                .and_then(|id| run.actions.pending_state(id))
                .map(str::to_owned);
        }
    }

    /// `window.remote.dismiss`: drops remote window `window`'s marks whose outcome this machine could not learn.
    pub(crate) fn dismiss_remote_unknown(&mut self, window: Uuid) -> Result<ControlResult, ControlError> {
        let Some(run) = self
            .remote
            .viewer
            .catalogs
            .get_mut(&window)
            .filter(|_| self.library.is_remote_window(window))
        else {
            return Err(ControlError::rejected(super::actions::remote_window_only(
                "window.remote.dismiss",
            )));
        };
        let dismissed = run.actions.dismiss_unknown();
        self.remote_actions_changed(window);
        Ok(ControlResult::text(format!("{dismissed} dismissed")))
    }
}

/// Waits for the reply of `op_id` from the run of `client`; `lost` answers when that run ended first (the window
/// closed, or closed and opened again) and its reply can no longer come.
fn await_reply(window: Uuid, client: Option<u64>, op_id: String, lost: String) -> Dispatched {
    Dispatched::Pending {
        after: REPLY_POLL,
        resume: Box::new(move |core: &mut Core| {
            core.poll_remote_actions(window);
            let Some(run) = core
                .remote
                .viewer
                .catalogs
                .get_mut(&window)
                .filter(|run| Some(run.client()) == client)
            else {
                return Dispatched::Done(Response::failure(lost));
            };
            match run.actions.take_reply(&op_id) {
                Some(reply) => Dispatched::Done(reply),
                None => await_reply(window, client, op_id, lost),
            }
        }),
    }
}
