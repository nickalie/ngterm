//! One remote window's connection to its server (upstream's `RemoteCatalogClient` and the window half of
//! `ControlServer+RemoteCatalog.swift`): the `version` probe, the catalog stream, its retries and its watchdog, and
//! the rows the stream's catalog becomes.
//!
//! Event-driven, as the presentation client is: the remote tick moves the probe, the retry and the watchdog on, and
//! the stream's lines and end reach [`Core::viewer_line`] and [`Core::viewer_closed`] under the window's link key.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ngterm_proto::{ControlRemoteWindow, ControlResult, Request, Response, VersionResult};
use uuid::Uuid;

use super::actions::RemoteActions;
use super::stream::{CatalogState, CatalogStream};
use super::view_state::RemoteWindowViewState;
use super::{PROBE_DEADLINE, SILENCE_LIMIT, retry_delay};
use crate::Core;
use crate::catalog::frames::{self, CatalogFrame, CatalogHello, CatalogPane, CatalogSnapshot};
use crate::dispatch::Dispatched;
use crate::error::ControlError;
use crate::live::lead::LeadAttachment;
use crate::model::{AppStore, CatalogBinding, CatalogRowCommands, CatalogRowContext, Session};
use crate::pane_env::uuid_string;
use crate::remote::client::{LinkKey, PresentationLink, PresentationTransport};
use crate::remote::peer::RemotePeer;
use crate::remote::ssh::{self, PaneLabel};
use crate::remote::tree::{self, RemoteCommandResult, RemoteCommandRunner};
use crate::remote::viewer::{Answer, run_off_thread, take_answer};
use crate::remote::viewer_run::ViewerRun;
use crate::resolve::trimmed;
use crate::windows::WindowRequest;

/// How often a waiting `window.remote.open` looks whether its window settled.
const OPEN_POLL: Duration = Duration::from_millis(50);
/// How many looks it takes before answering anyway: 20 s, the probe's deadline with room to spare.
const OPEN_POLLS: u32 = 400;

const OPEN_REQUIRES_HOST: &str = "window.remote.open requires a host";
const NOT_A_HOST: &str = "window.remote.open: not a host ssh can be given";

/// One remote window's connection.
pub(crate) struct CatalogRun {
    window: Uuid,
    client: u64,
    pub(crate) stream: CatalogStream,
    view: RemoteWindowViewState,
    launch: u64,
    link: Option<Box<dyn PresentationLink>>,
    attempt: usize,
    retry_at: Option<Instant>,
    probe: Option<Answer>,
    last_frame: Instant,
    pub(crate) actions: RemoteActions,
    /// A created row to select once the catalog brings it, unless the selection moved from the baseline.
    pub(crate) pending_selection: Option<(Uuid, Option<Uuid>)>,
}

impl std::fmt::Debug for CatalogRun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CatalogRun")
            .field("window", &self.window)
            .field("state", self.stream.state())
            .field("launch", &self.launch)
            .finish_non_exhaustive()
    }
}

impl CatalogRun {
    fn new(window: Uuid, client: u64, host: &str, view: RemoteWindowViewState, now: Instant) -> Self {
        Self {
            window,
            client,
            stream: CatalogStream::new(host),
            view,
            launch: 0,
            link: None,
            attempt: 0,
            retry_at: None,
            probe: None,
            last_frame: now,
            actions: RemoteActions::default(),
            pending_selection: None,
        }
    }

    /// Whether the `version` probe is out.
    pub(crate) const fn probing(&self) -> bool {
        self.probe.is_some()
    }

    /// Asks the server's `version`, unless a probe or a stream is already up or the window was refused.
    fn connect(&mut self, runner: &Arc<dyn RemoteCommandRunner>, now: Instant) {
        self.retry_at = None;
        if self.probe.is_some() || self.link.is_some() || self.stream.state().is_refused() {
            return;
        }
        let Ok(argv) = ssh::control_command(self.stream.host(), RemotePeer::Ngterm, &["version"]) else {
            self.stream.refuse(NOT_A_HOST.to_owned());
            return;
        };
        match run_off_thread(runner, argv, PROBE_DEADLINE) {
            Some(answer) => self.probe = Some(answer),
            None => self.lost(self.no_answer(), now),
        }
    }

    fn no_answer(&self) -> String {
        format!("{} did not answer", self.stream.host())
    }

    /// The probe answered: a server that can serve the window gets its stream, one that cannot is refused for good,
    /// and one that did not answer is retried.
    fn probed(&mut self, result: &RemoteCommandResult, transport: Option<&dyn PresentationTransport>, now: Instant) {
        let reply = (result.status == 0)
            .then(|| serde_json::from_str::<Response<VersionResult>>(result.stdout.trim()).ok())
            .flatten()
            .filter(|reply| reply.ok);
        let Some(reply) = reply else {
            self.lost(self.no_answer(), now);
            return;
        };
        let version = reply.result;
        let versions = version.as_ref().and_then(|version| version.versions.as_ref());
        let app = version.as_ref().map(|version| &version.app);
        if self.stream.check_versions(versions, app) {
            self.open(transport, now);
        }
    }

    /// Subscribes: the stream opens with this window's hello, which asks for no page rows.
    fn open(&mut self, transport: Option<&dyn PresentationTransport>, now: Instant) {
        let (Some(transport), Ok(argv)) = (transport, ssh::catalog_command(self.stream.host())) else {
            return;
        };
        self.stream.subscribing();
        self.launch += 1;
        self.last_frame = now;
        let link = transport.open(
            &argv,
            LinkKey {
                session: self.window,
                client: self.client,
                launch: self.launch,
            },
        );
        if let Ok(hello) = frames::encode(&CatalogFrame::Hello(CatalogHello {
            version: frames::VERSION,
            ..CatalogHello::default()
        })) {
            link.send(&hello);
        }
        self.link = Some(link);
    }

    fn resubscribe(&mut self, transport: Option<&dyn PresentationTransport>, now: Instant) {
        self.drop_link();
        self.open(transport, now);
    }

    fn drop_link(&mut self) {
        if let Some(link) = self.link.take() {
            link.stop();
        }
    }

    /// The stream was lost for `reason`: Disconnected, and a retry after the backoff.
    fn lost(&mut self, reason: String, now: Instant) {
        if self.stream.state().is_refused() {
            return;
        }
        tracing::info!(window = %self.window, %reason, "remote window disconnected");
        self.stream.connection_lost(reason);
        self.retry_at = Some(now + retry_delay(self.attempt));
        self.attempt += 1;
    }

    /// Moves the probe, the retry and the watchdog on.
    fn tick(
        &mut self,
        runner: &Arc<dyn RemoteCommandRunner>,
        transport: Option<&dyn PresentationTransport>,
        now: Instant,
    ) {
        if let Some(result) = self.probe.as_ref().and_then(take_answer) {
            self.probe = None;
            self.probed(&result, transport, now);
        }
        if self.retry_at.is_some_and(|at| now >= at) {
            self.connect(runner, now);
        }
        if self.link.is_some() && now.duration_since(self.last_frame) > SILENCE_LIMIT {
            self.drop_link();
            let reason = format!(
                "no word from {} for {} seconds",
                self.stream.host(),
                SILENCE_LIMIT.as_secs()
            );
            self.lost(reason, now);
        }
    }

    /// One line of launch `launch`; a late line of a replaced launch is dropped.
    fn receive(
        &mut self,
        launch: u64,
        line: &[u8],
        transport: Option<&dyn PresentationTransport>,
        now: Instant,
        bind: &mut dyn FnMut(Uuid) -> bool,
    ) -> Option<bool> {
        if launch != self.launch || self.link.is_none() {
            return None;
        }
        self.last_frame = now;
        let Ok(frame) = frames::decode(line) else {
            self.resubscribe(transport, now);
            return None;
        };
        let received = self.stream.receive(frame, bind);
        if let (Some(reply), Some(link)) = (&received.reply, &self.link)
            && let Ok(line) = frames::encode(reply)
        {
            link.send(&line);
        }
        match self.stream.state() {
            CatalogState::Connected => self.attempt = 0,
            CatalogState::Refused(_) => {
                self.drop_link();
                return received.moved;
            }
            CatalogState::Connecting | CatalogState::Disconnected(_) => {}
        }
        if self.stream.needs_snapshot() {
            self.resubscribe(transport, now);
        }
        received.moved
    }

    /// Launch `launch` ended for `reason`; the end of a replaced launch changes nothing.
    fn closed(&mut self, launch: u64, reason: &str, now: Instant) {
        if launch != self.launch || self.link.is_none() {
            return;
        }
        self.link = None;
        let reason = format!("the catalog stream from {} ended: {reason}", self.stream.host());
        self.lost(reason, now);
    }

    /// Ends the stream and every retry; the window keeps what it shows.
    fn stop(&mut self) {
        self.probe = None;
        self.retry_at = None;
        self.drop_link();
        self.actions.stop();
    }
}

/// The commands a remote window's panes run, and the leads their attaches register.
struct RowCommands<'a> {
    host: &'a str,
    leads: Vec<(Uuid, LeadAttachment)>,
}

impl CatalogRowCommands for RowCommands<'_> {
    /// Never claims the lead: a remote window attaches again on every reconnect, and a claim each time would take the
    /// pane from another machine at every network blip.
    fn attach(&mut self, pane: &CatalogPane) -> String {
        let lead = LeadAttachment::new(false);
        let label = PaneLabel {
            session: &pane.daemon,
            pane: "left",
        };
        let Ok(command) = ssh::attach_pane_command(self.host, &pane.endpoint, &pane.daemon, label, Some(&lead)) else {
            tracing::error!(host = %self.host, pane = %pane.identity, "a remote window's server listed a pane ngterm cannot address");
            return unaddressable(&format!("ngterm: {} listed a pane ngterm cannot address", self.host));
        };
        self.leads.push((pane.identity, lead));
        command
    }

    fn scratch(&mut self, pane: &CatalogPane) -> String {
        let label = PaneLabel {
            session: "scratch",
            pane: "left",
        };
        ssh::attach_pane_command(self.host, &pane.endpoint, &pane.daemon, label, None)
            .unwrap_or_else(|_| unaddressable(&format!("ngterm: {} listed a scratch ngterm cannot address", self.host)))
    }

    fn ended(&mut self) -> String {
        ssh::ended_command(self.host)
    }
}

/// A pane's command that says `line` and runs nothing else.
fn unaddressable(line: &str) -> String {
    if cfg!(windows) {
        return ViewerRun::Held {
            text: format!("{line}\r\n"),
        }
        .line();
    }
    let script = format!(
        "printf '%s\\n' {}; exit 1",
        crate::command_restore::shell_quoted_line(&[line])
    );
    crate::command_restore::shell_quoted_line(&["/usr/bin/env", "/bin/sh", "-c", script.as_str()])
}

/// The first row of `store`'s tree.
fn first_row(store: &AppStore) -> Option<Uuid> {
    store.sessions().next().map(Session::id)
}

impl Core {
    /// Starts a connection for every open remote window that has none, and ends those of windows no longer open:
    /// closing a remote window detaches it, and its server keeps every session.
    pub(crate) fn sync_remote_windows(&mut self) {
        let open = self.library.open_remote_windows();
        let gone: Vec<Uuid> = self
            .remote
            .viewer
            .catalogs
            .keys()
            .filter(|window| !open.contains(window))
            .copied()
            .collect();
        for window in gone {
            if let Some(mut run) = self.remote.viewer.catalogs.remove(&window) {
                run.stop();
            }
        }
        for window in open {
            if !self.remote.viewer.catalogs.contains_key(&window) {
                self.start_catalog(window);
            }
        }
    }

    fn start_catalog(&mut self, window: Uuid) {
        let Some(host) = self.library.remote_source(window).map(|source| source.host.clone()) else {
            return;
        };
        let view = self
            .persistence
            .as_ref()
            .map(|persistence| RemoteWindowViewState::load(&persistence.remote_view_path(window)))
            .unwrap_or_default();
        let now = (self.remote.clock)();
        self.remote.viewer.last_client += 1;
        let mut run = CatalogRun::new(window, self.remote.viewer.last_client, &host, view, now);
        run.connect(&self.remote.runner, now);
        self.remote.viewer.catalogs.insert(window, run);
        self.catalog_state_changed(window);
    }

    /// A line of a remote window's stream; false when `key` names no remote window's current client.
    pub(crate) fn remote_catalog_line(&mut self, key: LinkKey, line: &[u8]) -> bool {
        let transport = self.remote.viewer.transport.clone();
        let now = (self.remote.clock)();
        let window = key.session;
        let Some(run) = self
            .remote
            .viewer
            .catalogs
            .get_mut(&window)
            .filter(|run| run.client == key.client)
        else {
            return false;
        };
        let before = run.stream.state().clone();
        let library = &mut self.library;
        let mut duplicate = None;
        let mut bind = |catalog: Uuid| match library.bind_catalog(catalog, window) {
            CatalogBinding::Duplicate(other) => {
                duplicate = Some(other);
                false
            }
            CatalogBinding::Bound | CatalogBinding::Rebound { .. } => true,
        };
        let moved = run.receive(key.launch, line, transport.as_deref(), now, &mut bind);
        let changed = run.stream.state() != &before;
        if let Some(snapshot) = moved {
            self.reconcile_catalog(window);
            self.remote_catalog_moved(window, snapshot);
        }
        if changed {
            self.catalog_state_changed(window);
        }
        if let Some(other) = duplicate {
            self.settle_duplicate_window(window, other);
        }
        true
    }

    /// The end of a remote window's stream; false when `key` names no remote window's current client.
    pub(crate) fn remote_catalog_closed(&mut self, key: LinkKey, reason: &str) -> bool {
        let now = (self.remote.clock)();
        let Some(run) = self
            .remote
            .viewer
            .catalogs
            .get_mut(&key.session)
            .filter(|run| run.client == key.client)
        else {
            return false;
        };
        let before = run.stream.state().clone();
        run.closed(key.launch, reason, now);
        if *run.stream.state() != before {
            self.catalog_state_changed(key.session);
        }
        true
    }

    /// Moves every remote window's probe, retry and watchdog on.
    pub(crate) fn tick_catalogs(&mut self, now: Instant) {
        let transport = self.remote.viewer.transport.clone();
        let windows: Vec<Uuid> = self.remote.viewer.catalogs.keys().copied().collect();
        for window in windows {
            let Some(run) = self.remote.viewer.catalogs.get_mut(&window) else {
                continue;
            };
            let before = run.stream.state().clone();
            run.tick(&self.remote.runner, transport.as_deref(), now);
            if run.stream.state() != &before {
                self.catalog_state_changed(window);
            }
            self.poll_remote_actions(window);
        }
    }

    /// Whether a remote window waits on its probe, or holds a connection at all: `(probing, any)`.
    pub(crate) fn catalog_wake(&self) -> (bool, bool) {
        let catalogs = &self.remote.viewer.catalogs;
        let busy = |run: &CatalogRun| run.probing() || run.actions.busy();
        (catalogs.values().any(busy), !catalogs.is_empty())
    }

    /// Ends every remote window's stream (the quit path).
    pub(crate) fn shutdown_catalogs(&mut self) {
        for run in self.remote.viewer.catalogs.values_mut() {
            run.stop();
        }
    }

    fn catalog_state_changed(&mut self, window: Uuid) {
        if let Some(run) = self.remote.viewer.catalogs.get(&window) {
            tracing::info!(%window, state = ?run.stream.state(), "remote window connection");
        }
        if let Some(store) = self.library.store_mut(window) {
            store.schedule_tree_changed();
        }
    }

    /// Window `window` turned out to show the catalog window `other` shows: `other` is raised in its place, and
    /// `window` goes.
    fn settle_duplicate_window(&mut self, window: Uuid, other: Uuid) {
        if let Some(mut run) = self.remote.viewer.catalogs.remove(&window) {
            run.stop();
        }
        self.open_window(other);
        self.request_window(WindowRequest::Raise(other));
        self.take_frontmost(other);
        match self.persistence.as_mut() {
            Some(persistence) => persistence.remove_window(&mut self.library, window),
            None => self.library.remove_window(window),
        }
    }

    /// Brings window `window`'s rows to its catalog (upstream's `RemoteCatalogMirror.reconcile`), starting and
    /// stopping the presentation streams of rows whose session ended or whose scratch came or went.
    fn reconcile_catalog(&mut self, window: Uuid) {
        let Some(run) = self.remote.viewer.catalogs.get(&window) else {
            return;
        };
        let Some(catalog) = run.stream.catalog().cloned() else {
            return;
        };
        let host = run.stream.host().to_owned();
        let view = run.view.clone();
        let Some(store) = self.library.store_mut(window) else {
            return;
        };
        let presented: HashMap<Uuid, bool> = store
            .sessions()
            .map(|session| (session.id(), session.presents_catalog()))
            .collect();
        let mut commands = RowCommands {
            host: &host,
            leads: Vec::new(),
        };
        mirror_rows(store, &catalog, &view, &mut commands);
        if let Some(run) = self.remote.viewer.catalogs.get_mut(&window)
            && let Some((created, baseline)) = run.pending_selection
            && store.session(created).is_some()
        {
            run.pending_selection = None;
            if store.selected_session_id() == baseline {
                store.select_session(Some(created));
            }
        }
        let streams: Vec<(Uuid, bool)> = store
            .sessions()
            .filter_map(|session| {
                let now = session.presents_catalog();
                presented
                    .get(&session.id())
                    .filter(|before| **before != now)
                    .map(|_| (session.id(), now))
            })
            .collect();
        for (identity, lead) in commands.leads {
            self.live.leads.begin(identity, lead, false);
        }
        self.follow_remote_rows(streams);
    }

    /// The `remote` node of window `window`, `None` for a local one: its server, catalog and connection, the
    /// connection absent while the window is closed.
    pub(crate) fn remote_window_node(&self, window: Uuid) -> Option<ControlRemoteWindow> {
        let source = self.library.remote_source(window)?;
        let state = self.library.is_open(window).then(|| {
            self.remote
                .viewer
                .catalogs
                .get(&window)
                .map_or(CatalogState::Connecting, |run| run.stream.state().clone())
        });
        let (state, reason) = state.as_ref().map(CatalogState::wire).unzip();
        Some(ControlRemoteWindow {
            host: source.host.clone(),
            catalog_id: source.catalog_id.map(uuid_string),
            state: state.map(str::to_owned),
            reason: reason.flatten().map(str::to_owned),
            detached_sessions: None,
            detached_workspaces: None,
            operations: self.remote_operations(window),
        })
    }

    /// The connection of remote window `window`; `None` for a local or closed one.
    #[must_use]
    pub fn remote_window_state(&self, window: Uuid) -> Option<&CatalogState> {
        self.remote.viewer.catalogs.get(&window).map(|run| run.stream.state())
    }

    /// The window remote window `window` turned out to be: the one already showing its server's catalog, `window`
    /// itself once its connection left Connecting or it closed, and `None` while it still connects.
    fn settled_remote_window(&self, window: Uuid) -> Option<Uuid> {
        if let Some(other) = self.library.remote_duplicate_of(window) {
            return Some(other);
        }
        match self.remote_window_state(window) {
            Some(CatalogState::Connecting) => None,
            _ => Some(window),
        }
    }
}

/// Brings `store`'s rows to `catalog`, restoring `view` on each row it creates and keeping a selection that still
/// names a row, else taking the saved one, else the first row.
fn mirror_rows(
    store: &mut AppStore,
    catalog: &CatalogSnapshot,
    view: &RemoteWindowViewState,
    commands: &mut RowCommands<'_>,
) {
    let collapsed: HashSet<Uuid> = view.collapsed_workspaces.iter().copied().collect();
    store.prepare_catalog_workspaces(&catalog.workspaces, &collapsed);
    let mut context = CatalogRowContext {
        host: commands.host,
        protocol: catalog.versions.protocol,
        commands,
    };
    for workspace in &catalog.workspaces {
        for (index, entry) in workspace.sessions.iter().enumerate() {
            if store.session(entry.id()).is_some() {
                store.move_catalog_session(entry.id(), workspace.id, index);
                store.update_catalog_session(entry.id(), entry, &mut context);
                continue;
            }
            let Some(id) = store.insert_catalog_session(entry, workspace.id, index, &mut context) else {
                continue;
            };
            if let Some(session) = store.session_mut(id) {
                view.apply(session);
            }
        }
    }
    store.finish_catalog_workspaces(&catalog.workspaces);
    store.catalog_mirrored = true;
    let selected = store.selected_session_id().filter(|id| store.session(*id).is_some());
    if selected.is_none() {
        let saved = view.selected_session_id.filter(|id| store.session(*id).is_some());
        if let Some(target) = saved.or_else(|| first_row(store)) {
            store.select_session(Some(target));
        }
    }
}

/// `window.remote.open HOST` (upstream's `windowRemoteOpen`): opens `HOST`'s window, or raises the one already showing
/// it, and answers once it knows which window shows that server's catalog: a host that names a server another window
/// already shows settles on that window.
pub(crate) fn begin_open(core: &mut Core, request: &Request) -> Dispatched {
    let host = trimmed(request.args.as_ref().and_then(|args| args.host.as_deref())).map(str::to_owned);
    let Some(host) = host else {
        return Dispatched::done(Err(ControlError::rejected(OPEN_REQUIRES_HOST)));
    };
    if tree::validate(&host).is_err() {
        return Dispatched::done(Err(ControlError::rejected(NOT_A_HOST)));
    }
    let (window, created) = core.library.new_remote_window(&host);
    if !created {
        core.open_window(window);
        core.request_window(WindowRequest::Raise(window));
    }
    core.take_frontmost(window);
    core.sync_remote_windows();
    poll_open(window, OPEN_POLLS)
}

fn poll_open(window: Uuid, polls: u32) -> Dispatched {
    Dispatched::Pending {
        after: OPEN_POLL,
        resume: Box::new(move |core: &mut Core| match core.settled_remote_window(window) {
            Some(settled) => answer_open(core, settled),
            None if polls > 1 => poll_open(window, polls - 1),
            None => answer_open(core, window),
        }),
    }
}

fn answer_open(core: &mut Core, settled: Uuid) -> Dispatched {
    // a caller in the background gets no focus handoff, so frontmost is taken here
    core.take_frontmost(settled);
    Dispatched::done(Ok(ControlResult::id(uuid_string(settled))))
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;
