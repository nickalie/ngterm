//! `live.attach` and File ▸ Attach Remote (upstream's `attachRemoteSession` in `ControlServer+Zmx.swift` and
//! `AppActions+RemoteAttach.swift`): a local session attached to one of another machine's live sessions, each pane an
//! ssh into the origin's live session, marked remote. It stays out of the snapshot's tree; the snapshot lists it apart,
//! so the next launch attaches it again ([`super::relaunch`]).
//!
//! The remote is resolved again before anything is created, so a session gone since the list was taken fails rather
//! than handing back a fresh shell wearing its name; everything that can fail is checked before the model is touched.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use ngterm_proto::command::SessionCmd;
use ngterm_proto::{
    Args, Command, ControlLiveEndpoint, ControlPickOutcome, ControlRemoteSession, ControlRemoteTree, ControlResult,
    PickItem, Request, Response,
};
use uuid::Uuid;

use super::frames;
use super::origin_replies::{is_pickable, pickable_sessions, read_remote_tree};
use super::origin_sessions::{CreateRun, NEW_SESSION, NEW_SESSION_ID};
use super::remotes::{RemoteEntry, diagnostics_banner_body};
use super::ssh::{PaneLabel, attach_pane_command};
use super::tree::{self, RemoteCommandResult};
use super::viewer::{Answer, run_tree_off_thread, take_answer};
use crate::Core;
use crate::commands::Outcome;
use crate::dispatch::Dispatched;
use crate::error::ControlError;
use crate::live::lead::LeadAttachment;
use crate::modal::{MAX_PICK_ITEMS, PendingPick};
use crate::model::{AppStore, NewSession, RemoteBinding, RemoteOrigin, SplitAxis, WindowLibrary};
use crate::pane_env::uuid_string;
use crate::resolve::{Resolver, trimmed};

/// How often a waiting `live.attach` checks for the far side's answer.
const ATTACH_POLL: Duration = Duration::from_millis(50);
/// How long an attach's error HUD stays up (upstream's `remoteAttachErrorSeconds`).
pub(super) const ERROR_SECONDS: f64 = 5.0;
/// A progress panel is timed, so a soft-closed session drops it (upstream's `remoteAttachProgressSeconds`).
pub(super) const PROGRESS_SECONDS: f64 = 30.0;
/// The id of the session picker's Attach All row, which a far session's upper-case UUID never equals.
const ATTACH_ALL_ID: &str = "attach-all";
/// What the panels of an attach are headed by.
const ATTACH_REMOTE: &str = "Attach Remote";

/// Starts reading `host`'s attachable sessions off the main thread.
///
/// # Errors
///
/// `invalid host`: the host is not echoed, as it is refused for carrying control characters a terminal would print.
pub(crate) fn start_tree_read(core: &Core, host: &str) -> Result<Answer, ControlError> {
    start_tree_read_within(core, host, tree::TREE_DEADLINE)
}

/// [`start_tree_read`] given `deadline` to answer.
///
/// # Errors
///
/// As [`start_tree_read`].
pub(crate) fn start_tree_read_within(core: &Core, host: &str, deadline: Duration) -> Result<Answer, ControlError> {
    tree::validate(host).map_err(|_| ControlError::rejected("invalid host"))?;
    run_tree_off_thread(&core.remote.runner, host, deadline)
        .ok_or_else(|| ControlError::rejected(format!("the remote command failed on {host}")))
}

/// Starts `live.attach HOST SESSION [--window]` (upstream's dispatcher arm): no `active` default, as the target names a
/// session on another machine, and a target with control characters is refused before it can reach an error message.
pub(crate) fn begin_attach(core: &mut Core, request: &Request) -> Dispatched {
    let args = request.args.as_ref();
    let Some(host) = trimmed(args.and_then(|args| args.host.as_deref())).map(str::to_owned) else {
        return Dispatched::done(Err(ControlError::rejected("live.attach requires a host")));
    };
    let Some(session) = trimmed(request.target.as_deref()).map(str::to_owned) else {
        return Dispatched::done(Err(ControlError::rejected("live.attach requires a remote session")));
    };
    if !tree::is_plain(&session) {
        return Dispatched::done(Err(ControlError::rejected(tree::INVALID_REMOTE_SESSION)));
    }
    let window = trimmed(args.and_then(|args| args.window.as_deref())).map(str::to_owned);
    match start_tree_read(core, &host) {
        Ok(answer) => poll_attach(answer, AttachRequest { host, session, window }),
        Err(error) => Dispatched::done(Err(error)),
    }
}

/// One attach as asked.
#[derive(Debug, Clone)]
pub(crate) struct AttachRequest {
    pub(crate) host: String,
    pub(crate) session: String,
    pub(crate) window: Option<String>,
}

fn poll_attach(answer: Answer, request: AttachRequest) -> Dispatched {
    Dispatched::Pending {
        after: ATTACH_POLL,
        resume: Box::new(move |core: &mut Core| match take_answer(&answer) {
            Some(result) => core.attach_answered(&result, &request),
            None => poll_attach(answer, request),
        }),
    }
}

/// Where an attached row goes (upstream's `RemotePlacement`): a workspace of a window, or under `parent`, a remote row
/// of the same host, at `at`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Placement {
    pub(crate) window: Uuid,
    pub(crate) workspace: Uuid,
    pub(crate) at: Option<usize>,
    pub(crate) parent: Option<Uuid>,
    pub(crate) select: bool,
}

impl Placement {
    /// At the end of `parent`'s group, in whichever window holds it; `None` when no row can take a companion there.
    pub(crate) fn under(library: &WindowLibrary, parent: Uuid, select: bool) -> Option<Self> {
        let window = library.window_holding_session(parent)?;
        let place = library.store(window)?.remote_companion_place(parent)?;
        Some(Self {
            window,
            workspace: place.workspace,
            at: Some(place.index),
            parent: Some(parent),
            select,
        })
    }
}

impl Core {
    /// `live.attach` once its host's list arrived: a page companion attaches under the one row here showing its
    /// parent ([`super::companions`]); a terminal companion joins its unique matching parent in this window.
    fn attach_answered(&mut self, result: &RemoteCommandResult, request: &AttachRequest) -> Dispatched {
        let remote_tree = match read_remote_tree(result, &request.host) {
            Ok(tree) => tree,
            Err(error) => return Dispatched::done(Err(ControlError::rejected(error))),
        };
        let page = remote_tree
            .sessions
            .iter()
            .find(|candidate| candidate.id == request.session && candidate.content.as_deref() == Some("page"));
        match page {
            Some(page) => super::companions::begin_page_row_attach(self, page, &remote_tree, request),
            None => Dispatched::done(self.attach_from(&remote_tree, request)),
        }
    }

    /// Creates the local session attached to `request`'s remote session once its host's list arrived (upstream's
    /// `attachRemoteSession`).
    pub(crate) fn finish_attach(&mut self, result: &RemoteCommandResult, request: &AttachRequest) -> Outcome {
        let remote_tree = read_remote_tree(result, &request.host).map_err(ControlError::rejected)?;
        self.attach_from(&remote_tree, request)
    }

    fn attach_from(&mut self, remote_tree: &ControlRemoteTree, request: &AttachRequest) -> Outcome {
        let host = request.host.as_str();
        // by id only: names are mutable and not unique, and `live tree` prints the id for exactly this hand-off
        let remote = remote_tree
            .sessions
            .iter()
            .find(|candidate| candidate.id == request.session)
            .ok_or_else(|| ControlError::rejected(format!("no attachable session {} on {host}", request.session)))?;
        let window = Resolver::new(self.library()).open_placement_window(request.window.as_deref())?;
        let rows = super::companions::matching_parent_rows(
            self,
            window,
            host,
            remote.parent.as_deref(),
            &remote_tree.endpoint,
        );
        let under = match rows.as_slice() {
            [parent] => Placement::under(self.library(), *parent, true),
            _ => None,
        };
        let placement = under
            .or_else(|| self.workspace_placement(window, true))
            .ok_or_else(|| ControlError::rejected("no window to attach into"))?;
        self.attach_remote_session(remote, remote_tree, host, placement)
    }

    /// Creates the local row for `remote`, one session of `remote_tree`, where `placement` says, and binds its panes
    /// (upstream's `attach(_:from:host:placement:)`). Everything that can fail is checked before the model is touched;
    /// `{id, parent}`.
    pub(crate) fn attach_remote_session(
        &mut self,
        remote: &ControlRemoteSession,
        remote_tree: &ControlRemoteTree,
        host: &str,
        placement: Placement,
    ) -> Outcome {
        let (left, right) = panes_by_role(remote)
            .ok_or_else(|| ControlError::rejected(format!("{host} reported panes ngterm cannot address")))?;
        // attaching is the user asking for the session HERE, so every pane claims the lead at once
        let leads = (LeadAttachment::new(true), LeadAttachment::new(true));
        let command = |daemon: &str, pane: &'static str, lead: &LeadAttachment| {
            let label = PaneLabel {
                session: &remote.name,
                pane,
            };
            attach_pane_command(host, &remote_tree.endpoint, daemon, label, Some(lead))
        };
        let unaddressable = || ControlError::rejected(format!("{host} reported a session ngterm cannot address"));
        let primary = command(&left, "left", &leads.0).map_err(|_| unaddressable())?;
        let split = right
            .as_deref()
            .map(|daemon| command(daemon, "right", &leads.1))
            .transpose()
            .map_err(|_| unaddressable())?;
        // the LOCAL working directory, not the remote one: a path on the far side may not exist here
        let home = self.library().home().to_owned();
        let store = self
            .library_mut()
            .store_mut(placement.window)
            .ok_or(ControlError::NoOpenWindow)?;
        let created = store
            .add_session(
                placement.workspace,
                NewSession {
                    command: Some(primary),
                    name: Some(remote.name.clone()),
                    wait: true,
                    remote_host: Some(host.to_owned()),
                    at: placement.at,
                    select: placement.select,
                    parent: placement.parent,
                    ..NewSession::at_cwd(home)
                },
            )
            .ok_or_else(|| ControlError::rejected("could not create the session"))?;
        let daemons = install_panes(store, created, &left, right.as_deref(), split, remote);
        let origin = RemoteOrigin {
            host: host.to_owned(),
            endpoint: remote_tree.endpoint.clone(),
            session_name: remote.name.clone(),
        };
        let identities: Vec<Uuid> = daemons.keys().copied().collect();
        store.bind_remote(
            RemoteBinding::new(remote.id.clone(), daemons, remote_tree.presentation, Some(origin)),
            created,
        );
        let primary_identity = store.session(created).map(|session| session.pane_identity);
        for identity in identities {
            let lead = if Some(identity) == primary_identity {
                leads.0.clone()
            } else {
                leads.1.clone()
            };
            self.live.leads.begin(identity, lead, false);
        }
        // the row's created event fired before the binding existed
        self.start_remote_presentation(created);
        // a session attached by any path takes over the row a relaunch saved for it, so closing it forgets it
        self.forget_saved_remote_session(host, &remote.id);
        if placement.select {
            self.request_pane_focus(created);
        }
        Ok(ControlResult {
            parent: placement.parent.map(uuid_string),
            ..ControlResult::id(uuid_string(created))
        })
    }
}

/// The left and the optional right daemon, by role and never by position: a payload with two lefts or no left fails
/// rather than quietly becoming one pane, or the wrong one.
fn panes_by_role(remote: &ControlRemoteSession) -> Option<(String, Option<String>)> {
    let mut roles: HashMap<&str, &str> = HashMap::new();
    for pane in &remote.panes {
        if !matches!(pane.pane.as_str(), "left" | "right") || roles.insert(&pane.pane, &pane.daemon).is_some() {
            return None;
        }
    }
    let left = roles.get("left")?;
    Some(((*left).to_owned(), roles.get("right").map(|right| (*right).to_owned())))
}

/// Gives the new session its split when the remote has one, and returns the daemon each local pane attaches to.
fn install_panes(
    store: &mut AppStore,
    created: Uuid,
    left: &str,
    right: Option<&str>,
    split: Option<String>,
    remote: &ControlRemoteSession,
) -> HashMap<Uuid, String> {
    if let Some(split) = split
        && let Some(session) = store.session_mut(created)
    {
        session.split_launch.command = Some(split);
        session.split_launch.wait = true;
        let axis = remote
            .split_axis
            .as_deref()
            .and_then(SplitAxis::from_wire)
            .unwrap_or_default();
        store.set_split_visibility(created, true, Some(axis));
    }
    let Some(session) = store.session(created) else {
        return HashMap::new();
    };
    let mut daemons = HashMap::from([(session.pane_identity, left.to_owned())]);
    if let (Some(right), Some(local)) = (right, session.split_pane_identity) {
        daemons.insert(local, right.to_owned());
    }
    daemons
}

// MARK: - File ▸ Attach Remote

/// Where one Attach Remote is.
#[derive(Debug)]
pub(super) enum Step {
    /// The window's picker asks which configured machine to attach from.
    ChoosingHost { pick: String },
    /// The machine's sessions are being listed.
    Listing { host: String, answer: Answer },
    /// The window's picker asks which session.
    ChoosingSession {
        host: String,
        pick: String,
        parents: HashSet<String>,
    },
    /// The chosen session is being attached; its label names it in the progress panel.
    Attaching { request: AttachRequest, answer: Answer },
    /// A selected parent attaches its freshly discovered companion group.
    AttachingGroup(super::group_attach::GroupRun),
    /// The machine is being listed again for Attach All, so what attaches is what it offers now.
    AttachingAll { host: String, answer: Answer },
    /// The machine creates a session, which attaches here once it is attachable ([`super::origin_sessions`]).
    CreatingSession(CreateRun),
}

/// Where an action's HUD goes: over `session` in `window`, its message headed by `action`.
pub(super) struct ActionPanel<'a> {
    pub(super) action: &'a str,
    pub(super) window: Uuid,
    pub(super) session: Uuid,
}

/// One File ▸ Attach Remote (upstream's `attachRemote` task with its `RemoteAttachHud`) or a remote row's New Session
/// or Duplicate Session in progress.
#[derive(Debug)]
pub(crate) struct AttachFlow {
    pub(super) window: Uuid,
    /// What its panels are headed by.
    pub(super) action: &'static str,
    /// The session the progress and error panels go over: the window's selection when File ▸ Attach Remote started, the
    /// clicked row for a row's New Session or Duplicate Session.
    hud_session: Option<Uuid>,
    /// The slot generation of the panel this flow opened; one another caller posted meanwhile is not its to close.
    owned_hud: Option<u64>,
    pub(super) step: Step,
}

impl AttachFlow {
    /// A flow in `window` with its panels over `hud_session`, its first step yet to be set.
    pub(super) fn new(window: Uuid, hud_session: Option<Uuid>) -> Self {
        Self {
            window,
            action: ATTACH_REMOTE,
            hud_session,
            owned_hud: None,
            step: Step::ChoosingHost { pick: String::new() },
        }
    }
}

/// What an Attach All did: the rows it made, and why the others were not made.
#[derive(Debug, Default)]
struct AttachAllReport {
    attached: usize,
    failures: Vec<String>,
}

/// The sessions Attach All attaches, parents before their companions so each companion finds its parent's row: what
/// `tree` offers that `rows` does not show yet. A page companion attaches only under a parent the picker cannot name,
/// so it stays out, as it stays out of the picker's rows.
fn attach_all_sessions<'a>(tree: &'a ControlRemoteTree, rows: &HashMap<String, Uuid>) -> Vec<&'a ControlRemoteSession> {
    pickable_sessions(tree)
        .into_iter()
        .filter(|session| !rows.contains_key(&session.id))
        .collect()
}

/// One remote session as a picker row: its name, then where it lives, its context, its cwd and what runs in it, and
/// the far node's status for the row's trailing glyph (upstream's `remotePickItem`).
fn pick_item(session: &ControlRemoteSession) -> PickItem {
    let programs: Vec<String> = session
        .panes
        .iter()
        .filter_map(|pane| pane.foreground.as_ref()?.first())
        .map(|program| program.rsplit('/').next().unwrap_or(program).to_owned())
        .collect();
    let parts = [
        format!("{}/{}", session.window_name, session.workspace_name),
        session.context.clone().unwrap_or_default(),
        session.cwd.clone(),
        programs.join(" | "),
    ];
    let subtitle: Vec<&str> = parts
        .iter()
        .map(String::as_str)
        .filter(|part| !part.is_empty())
        .collect();
    PickItem {
        id: session.id.clone(),
        label: session.name.clone(),
        subtitle: Some(subtitle.join("  ·  ")),
        status: session.status.clone(),
        status_color: session.status_color.clone(),
        status_shape: session.status_shape.clone(),
    }
}

impl Core {
    /// The `remotes.conf` of the config directory, beside `keymap.conf` (upstream's `remotesURL`).
    #[must_use]
    pub fn remotes_path(&self) -> PathBuf {
        self.config_dir().join("remotes.conf")
    }

    /// The file the remotes state reads, which the frontend watches; it moves with the config directory.
    #[must_use]
    pub fn watched_remotes_path(&self) -> &Path {
        self.remote.remotes.path()
    }

    /// The hosts `remotes.conf` listed at its last read; an unreadable file reads as none.
    #[must_use]
    pub fn configured_remotes(&self) -> &[RemoteEntry] {
        self.remote.remotes.entries()
    }

    /// The launch step: reads `remotes.conf` and posts its problems banner when the file is already broken, as
    /// upstream's scene task does. Commit afterwards.
    pub fn load_remotes_at_launch(&mut self) {
        self.repoint_remotes();
        let count = self.remote.remotes.issue_count();
        if count > 0 {
            self.post_remotes_banner(count);
        }
    }

    /// Reads `remotes.conf` again, as the frontend's watch asks on every change of the file or its directory. With
    /// `reporting`, an edit that turned a clean file into one with problems posts the banner, once per such episode.
    /// Settle afterwards: File ▸ Attach Remote follows the entries.
    pub fn refresh_remotes(&mut self, reporting: bool) {
        if let Some(count) = self.remote.remotes.refresh(reporting) {
            self.post_remotes_banner(count);
        }
    }

    /// Points the remotes state at the config directory's file, without a report (upstream's `remotes.watch`).
    pub(crate) fn repoint_remotes(&mut self) {
        let path = self.remotes_path();
        self.remote.remotes.watch(path);
    }

    fn post_remotes_banner(&mut self, count: usize) {
        self.post_app_banner("remotes-diagnostics", "Remotes", diagnostics_banner_body(count));
    }

    /// File ▸ Attach Remote in window `window` (upstream's `attachRemote()`): with one configured machine it lists that
    /// machine's sessions at once, with several the window's picker asks which first. Commit afterwards.
    pub fn attach_remote(&mut self, window: Uuid) {
        if !self.library().is_open(window) || self.modal_pending(window) {
            return;
        }
        let remotes = self.configured_remotes().to_vec();
        let Some(first) = remotes.first() else {
            return;
        };
        let hud_session = self.library().store(window).and_then(AppStore::selected_session_id);
        if remotes.len() == 1 {
            let host = first.destination.clone();
            self.begin_listing(window, hud_session, host);
            return;
        }
        let items = remotes
            .iter()
            .map(|remote| PickItem {
                id: remote.destination.clone(),
                label: remote.label.clone(),
                subtitle: (remote.label != remote.destination).then(|| remote.destination.clone()),
                ..PickItem::default()
            })
            .collect();
        let Some(pick) = self.open_internal_pick(window, items, "Attach from which Mac?") else {
            return;
        };
        let mut flow = AttachFlow::new(window, hud_session);
        flow.step = Step::ChoosingHost { pick };
        self.remote.viewer.attach_flows.push(flow);
    }

    fn open_internal_pick(&mut self, window: Uuid, items: Vec<PickItem>, prompt: &str) -> Option<String> {
        let id = uuid_string(Uuid::new_v4());
        let pick = PendingPick {
            id: id.clone(),
            items,
            prompt: Some(prompt.to_owned()),
            query: None,
            allow_custom: false,
            selection: None,
        };
        self.modals.open_pick(window, pick).ok()?;
        self.sync_modals();
        Some(id)
    }

    fn begin_listing(&mut self, window: Uuid, hud_session: Option<Uuid>, host: String) {
        let mut flow = AttachFlow::new(window, hud_session);
        self.attach_progress(&mut flow, &format!("listing sessions on {host}…"));
        match start_tree_read(self, &host) {
            Ok(answer) => {
                flow.step = Step::Listing { host, answer };
                self.remote.viewer.attach_flows.push(flow);
            }
            Err(error) => self.attach_failed(&mut flow, &format!("{host}: {error}")),
        }
    }

    /// Moves every Attach Remote on: a picker answered, a listing or an attach arrived.
    pub(crate) fn advance_attach_flows(&mut self) {
        let flows = std::mem::take(&mut self.remote.viewer.attach_flows);
        for flow in flows {
            if let Some(flow) = self.advance_attach_flow(flow) {
                self.remote.viewer.attach_flows.push(flow);
            }
        }
    }

    /// One flow's next step; `None` once it ended.
    fn advance_attach_flow(&mut self, mut flow: AttachFlow) -> Option<AttachFlow> {
        match std::mem::replace(&mut flow.step, Step::ChoosingHost { pick: String::new() }) {
            Step::ChoosingHost { pick } => match self.picked(flow.window, &pick) {
                Picked::Waiting => {
                    flow.step = Step::ChoosingHost { pick };
                    Some(flow)
                }
                Picked::Chosen(host, _) => {
                    self.begin_listing(flow.window, flow.hud_session, host);
                    None
                }
                Picked::Ended => None,
            },
            Step::Listing { host, answer } => {
                let Some(result) = take_answer(&answer) else {
                    flow.step = Step::Listing { host, answer };
                    return Some(flow);
                };
                self.listed(flow, &host, &result)
            }
            Step::ChoosingSession { host, pick, parents } => match self.picked(flow.window, &pick) {
                Picked::Waiting => {
                    flow.step = Step::ChoosingSession { host, pick, parents };
                    Some(flow)
                }
                Picked::Chosen(session, _) if session == ATTACH_ALL_ID => self.start_attach_all(flow, host),
                Picked::Chosen(session, _) if session == NEW_SESSION_ID => self.start_origin_session(flow, host, None),
                Picked::Chosen(session, label) => {
                    let group = parents.contains(&session);
                    self.start_picked_attach(flow, host, session, &label, group)
                }
                Picked::Ended => None,
            },
            Step::Attaching { request, answer } => {
                let Some(result) = take_answer(&answer) else {
                    flow.step = Step::Attaching { request, answer };
                    return Some(flow);
                };
                match self.finish_attach(&result, &request) {
                    Ok(_) => self.attach_close_hud(&mut flow),
                    Err(error) => self.attach_failed(&mut flow, &error.to_string()),
                }
                None
            }
            Step::AttachingGroup(run) => match super::group_attach::step(self, run) {
                super::group_attach::GroupStep::Pending(run, _) => {
                    flow.step = Step::AttachingGroup(run);
                    Some(flow)
                }
                super::group_attach::GroupStep::Done(reply) => {
                    self.finish_picked_group(&mut flow, &reply);
                    None
                }
            },
            Step::AttachingAll { host, answer } => {
                let Some(result) = take_answer(&answer) else {
                    flow.step = Step::AttachingAll { host, answer };
                    return Some(flow);
                };
                self.finish_attach_all(&mut flow, &host, &result);
                None
            }
            Step::CreatingSession(run) => self.advance_origin_session(flow, run),
        }
    }

    /// The listing arrived: the window's picker asks which session to attach.
    fn listed(&mut self, mut flow: AttachFlow, host: &str, result: &RemoteCommandResult) -> Option<AttachFlow> {
        // a page companion attaches only under its parent's row, which the picker has no place for
        let (endpoint, sessions) = match read_remote_tree(result, host) {
            Ok(remote) => {
                let sessions: Vec<ControlRemoteSession> = remote.sessions.into_iter().filter(is_pickable).collect();
                (remote.endpoint, sessions)
            }
            Err(error) => {
                self.attach_failed(&mut flow, &format!("{host}: {error}"));
                return None;
            }
        };
        if sessions.len() > MAX_PICK_ITEMS {
            self.attach_failed(&mut flow, &format!("{host} offers too many sessions"));
            return None;
        }
        self.attach_close_hud(&mut flow);
        // the window may have closed during the ssh wait, and a picker no window renders would never resolve
        if !self.library().is_open(flow.window) {
            return None;
        }
        let items = self.session_items(&sessions, host, &endpoint);
        let pick = self.open_internal_pick(flow.window, items, &format!("Attach from {host}"))?;
        flow.step = Step::ChoosingSession {
            host: host.to_owned(),
            pick,
            parents: sessions
                .iter()
                .filter(|row| row.parent.is_none() && row.content.is_none())
                .map(|row| row.id.clone())
                .collect(),
        };
        Some(flow)
    }

    /// The session picker's rows: each listed session, then an Attach All row when it would attach more than one and a
    /// New Session row while the picker has room. Last, as the picker's selection goes to the first row on opening and on
    /// every filter edit, and Enter takes it; with nothing to attach, New Session is the only row.
    fn session_items(
        &self,
        sessions: &[ControlRemoteSession],
        host: &str,
        endpoint: &ControlLiveEndpoint,
    ) -> Vec<PickItem> {
        let rows = self.rows_attached_from(host, endpoint);
        let missing = sessions
            .iter()
            .filter(|session| !rows.contains_key(&session.id))
            .count();
        let names: HashMap<&str, &str> = sessions
            .iter()
            .map(|row| (row.id.as_str(), row.name.as_str()))
            .collect();
        let mut items: Vec<PickItem> = sessions
            .iter()
            .map(|session| {
                let mut item = pick_item(session);
                if let Some(parent) = session.parent.as_deref().and_then(|id| names.get(id)) {
                    item.label = format!("{parent} › {}", session.name);
                }
                item
            })
            .collect();
        if missing > 1 && items.len() < MAX_PICK_ITEMS {
            items.push(PickItem {
                id: ATTACH_ALL_ID.to_owned(),
                label: "Attach All".to_owned(),
                subtitle: Some(format!("{missing} sessions not attached here")),
                ..PickItem::default()
            });
        }
        if items.len() < MAX_PICK_ITEMS {
            items.push(PickItem {
                id: NEW_SESSION_ID.to_owned(),
                label: NEW_SESSION.to_owned(),
                subtitle: Some(format!("create a session on {host} and attach it")),
                ..PickItem::default()
            });
        }
        items
    }

    /// The rows of every open window attached from `host`'s ngterm at `endpoint`, by the origin's session id.
    pub(super) fn rows_attached_from(&self, host: &str, endpoint: &ControlLiveEndpoint) -> HashMap<String, Uuid> {
        let library = self.library();
        library
            .open_ids()
            .into_iter()
            .filter_map(|window| library.store(window))
            .flat_map(AppStore::sessions)
            .filter(|row| row.remote_host() == Some(host))
            .filter_map(|row| {
                let binding = &row.remote_presentation()?.binding;
                let origin = binding.origin.as_ref()?;
                (origin.endpoint == *endpoint).then(|| (binding.remote_session_id.clone(), row.id()))
            })
            .collect()
    }

    fn start_attach_all(&mut self, mut flow: AttachFlow, host: String) -> Option<AttachFlow> {
        self.attach_progress(&mut flow, "attaching all sessions…");
        match start_tree_read(self, &host) {
            Ok(answer) => {
                flow.step = Step::AttachingAll { host, answer };
                Some(flow)
            }
            Err(error) => {
                self.attach_failed(&mut flow, &error.to_string());
                None
            }
        }
    }

    /// The new listing arrived: reports what Attach All left unattached. A partial failure's panel goes over the row
    /// the attach selected, since the one the progress panel went over is out of view by then.
    fn finish_attach_all(&mut self, flow: &mut AttachFlow, host: &str, result: &RemoteCommandResult) {
        let report = match read_remote_tree(result, host) {
            Ok(tree) => self.attach_all(&tree, host, flow.window),
            Err(error) => {
                self.attach_failed(flow, &format!("{host}: {error}"));
                return;
            }
        };
        if report.failures.is_empty() {
            if report.attached == 0 {
                self.attach_failed(flow, &format!("nothing to attach on {host}"));
            } else {
                self.attach_close_hud(flow);
            }
            return;
        }
        let total = report.attached + report.failures.len();
        let reasons = report.failures.join("; ");
        let text = format!("attached {} of {total} from {host}; {reasons}", report.attached);
        if report.attached == 0 {
            self.attach_failed(flow, &text);
            return;
        }
        tracing::info!(reason = text, "attach all partly failed");
        self.attach_close_hud(flow);
        let selected = self
            .library()
            .store(flow.window)
            .and_then(AppStore::selected_session_id);
        self.action_failed(flow.action, flow.window, selected, &text);
    }

    /// Attaches every session `tree` offers that no row here shows: each companion under its parent's row wherever
    /// that row is, the rest in `window`'s current workspace, the first of those selected.
    fn attach_all(&mut self, tree: &ControlRemoteTree, host: &str, window: Uuid) -> AttachAllReport {
        let mut report = AttachAllReport::default();
        let mut rows = self.rows_attached_from(host, &tree.endpoint);
        let mut selected = false;
        for remote in attach_all_sessions(tree, &rows) {
            let parent = remote.parent.as_ref().and_then(|parent| rows.get(parent).copied());
            let under = parent.and_then(|parent| Placement::under(self.library(), parent, false));
            let placement = under.or_else(|| self.workspace_placement(window, !selected));
            let attached = placement
                .ok_or(ControlError::NoOpenWindow)
                .and_then(|placement| self.attach_remote_session(remote, tree, host, placement));
            match attached {
                Ok(result) => {
                    selected |= under.is_none();
                    report.attached += 1;
                    if let Some(row) = result.id.as_deref().and_then(frames::parse_uuid) {
                        rows.insert(remote.id.clone(), row);
                    }
                }
                Err(error) => report.failures.push(format!("{}: {error}", remote.name)),
            }
        }
        report
    }

    /// A row of its own in `window`'s current workspace; `None` once the window has none.
    pub(super) fn workspace_placement(&self, window: Uuid, select: bool) -> Option<Placement> {
        let workspace = self.library().store(window)?.current_workspace_id()?;
        Some(Placement {
            window,
            workspace,
            at: None,
            parent: None,
            select,
        })
    }

    fn start_picked_attach(
        &mut self,
        mut flow: AttachFlow,
        host: String,
        session: String,
        label: &str,
        group: bool,
    ) -> Option<AttachFlow> {
        if !group {
            return self.start_attach(flow, host, session, label);
        }
        self.attach_progress(&mut flow, &format!("attaching {label}…"));
        let request = AttachRequest {
            host,
            session,
            window: Some(uuid_string(flow.window)),
        };
        match super::group_attach::start(self, request) {
            Ok(run) => {
                flow.step = Step::AttachingGroup(run);
                Some(flow)
            }
            Err(error) => {
                self.attach_failed(&mut flow, &error.to_string());
                None
            }
        }
    }

    fn finish_picked_group(&mut self, flow: &mut AttachFlow, reply: &Response) {
        if reply.ok {
            self.attach_close_hud(flow);
            return;
        }
        let parent = reply
            .result
            .as_ref()
            .and_then(|result| result.get("id"))
            .and_then(serde_json::Value::as_str)
            .and_then(frames::parse_uuid);
        let error = reply.error.as_deref().unwrap_or("attach failed");
        if let Some(parent) = parent
            && let Some(window) = self.library().window_holding_session(parent)
        {
            self.attach_close_hud(flow);
            self.action_failed(flow.action, window, Some(parent), error);
        } else {
            self.attach_failed(flow, error);
        }
    }

    fn start_attach(&mut self, mut flow: AttachFlow, host: String, session: String, label: &str) -> Option<AttachFlow> {
        self.attach_progress(&mut flow, &format!("attaching {label}…"));
        match start_tree_read(self, &host) {
            Ok(answer) => {
                flow.step = Step::Attaching {
                    request: AttachRequest {
                        host,
                        session,
                        window: Some(uuid_string(flow.window)),
                    },
                    answer,
                };
                Some(flow)
            }
            Err(error) => {
                self.attach_failed(&mut flow, &error.to_string());
                None
            }
        }
    }

    /// Picker `pick`'s answer in window `window`.
    fn picked(&self, window: Uuid, pick: &str) -> Picked {
        let result = self
            .modals
            .controller(window)
            .and_then(|controller| controller.result(pick))
            .or_else(|| self.modals.retained_pick(pick).map(|(_, result)| result));
        match result {
            Some(result) if result.result == ControlPickOutcome::Pending => Picked::Waiting,
            Some(result) if result.result == ControlPickOutcome::Picked => match result.id {
                Some(id) => {
                    let label = result.label.unwrap_or_else(|| id.clone());
                    Picked::Chosen(id, label)
                }
                None => Picked::Ended,
            },
            _ => Picked::Ended,
        }
    }

    // MARK: - the attach's panels

    /// Opens the attach's panel over the session it started from; whether it opened.
    fn attach_open_hud(&mut self, flow: &mut AttachFlow, text: &str, hide_after: f64, spinner: bool) -> bool {
        let Some(session) = flow.hud_session else {
            return false;
        };
        let panel = ActionPanel {
            action: flow.action,
            window: flow.window,
            session,
        };
        flow.owned_hud = self.open_action_hud(&panel, text, hide_after, spinner);
        flow.owned_hud.is_some()
    }

    /// Opens `panel`'s HUD saying `text` (upstream's `RemoteAttachHud`); the slot generation of the panel it opened,
    /// `None` when none opened.
    pub(super) fn open_action_hud(
        &mut self,
        panel: &ActionPanel<'_>,
        text: &str,
        hide_after: f64,
        spinner: bool,
    ) -> Option<u64> {
        let args = Args {
            message: Some(crate::keymap::failure::message(panel.action, text)),
            spinner: spinner.then(|| "bar".to_owned()),
            hide_after: Some(hide_after),
            window: Some(uuid_string(panel.window)),
            ..Args::default()
        };
        let response: Response = self.dispatch(&Request {
            cmd: Command::Session(SessionCmd::HudOpen),
            target: Some(uuid_string(panel.session)),
            args: Some(args),
        });
        if !response.ok {
            return None;
        }
        self.library()
            .store_for_session(panel.session)
            .and_then(|store| store.session(panel.session))
            .filter(|found| found.overlay.hud_active())
            .map(|found| found.overlay.generation)
    }

    /// `action` failed: a timed error panel over `session` in `window`, else a banner (upstream's
    /// `RemoteAttachHud.fail` and its `notifyCommandFailure` fallback).
    pub(crate) fn action_failed(&mut self, action: &str, window: Uuid, session: Option<Uuid>, text: &str) {
        let opened = session.and_then(|session| {
            let panel = ActionPanel {
                action,
                window,
                session,
            };
            self.open_action_hud(&panel, text, ERROR_SECONDS, false)
        });
        if opened.is_none() {
            self.post_app_banner(
                format!("command-failure:{action}"),
                "Command failed",
                format!("{action} ({text})"),
            );
        }
    }

    pub(super) fn attach_progress(&mut self, flow: &mut AttachFlow, text: &str) {
        self.attach_open_hud(flow, text, PROGRESS_SECONDS, true);
    }

    /// Closes the panel this flow opened, never one another caller posted meanwhile.
    pub(super) fn attach_close_hud(&mut self, flow: &mut AttachFlow) {
        if let (Some(session), Some(owned)) = (flow.hud_session, flow.owned_hud.take()) {
            self.close_owned_hud(session, owned);
        }
    }

    /// Closes `session`'s panel when it is still the one posted as generation `owned`, never one another caller
    /// posted meanwhile.
    pub(super) fn close_owned_hud(&mut self, session: Uuid, owned: u64) {
        let Some(store) = self.library_mut().store_for_session_mut(session) else {
            return;
        };
        let current = store
            .session(session)
            .filter(|found| found.overlay.hud_active())
            .map(|found| found.overlay.generation);
        if current == Some(owned) {
            store.close_hud(session);
        }
    }

    /// The attach failed: an error panel over its session, else a banner (upstream's `RemoteAttachHud.fail`).
    pub(super) fn attach_failed(&mut self, flow: &mut AttachFlow, text: &str) {
        tracing::info!(reason = text, "attach remote failed");
        if self.attach_open_hud(flow, text, ERROR_SECONDS, false) {
            return;
        }
        self.attach_close_hud(flow);
        self.action_failed(flow.action, flow.window, None, text);
    }
}

/// A picker's answer as a flow reads it.
enum Picked {
    Waiting,
    /// The picked row's id and label.
    Chosen(String, String),
    /// Dismissed, or gone with its window.
    Ended,
}
