//! Companions of a remote row (upstream's `ControlServer+RemoteCompanions.swift` and the remote half of
//! `AppActions+Companions.swift`): New Companion, Duplicate as Companion and a page companion on a row attached from
//! another machine are that machine's to create. Its `ngtermctl` is run once over ssh ([`super::ssh::control_command`],
//! never retried, never done here), the origin is polled until the new session is attachable, and it is attached here
//! under the row's group. A companion created there but not attached here is named with the line that attaches it.
//!
//! A page companion stays the other machine's: the row here shows one attachment of it (`session.companion.page.attach`
//! there), over its own page channel, and closes when the page ends there, its outcome copied in first. `live attach`
//! of a page row attaches it under the one row here showing its parent.
//!
//! Each run is a [`CompanionFlow`] stepped on the main loop: a control request waits on it ([`drive`]), the GUI's runs
//! from the remote tick with a progress and a failure HUD over the row ([`Core::begin_gui_remote_companion`]). Closing
//! a remote companion here only detaches it, as closing any remote row does.
//!
//! A headless origin shows no page itself, so it hands each page companion created under a presented row to this
//! machine (`companion.page`, this build's addition), which attaches it as `live attach` would
//! ([`Core::attach_handed_companion`]). One shown here already, by this machine's own create too, is never shown twice
//! by these two; `live attach` of a page shown here still shows it again, as upstream's does.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use ngterm_proto::command::SessionCmd;
use ngterm_proto::{Command, ControlRemoteSession, ControlRemoteTree, ControlResult};
use uuid::Uuid;

use super::attach::{ActionPanel, AttachRequest, ERROR_SECONDS, PROGRESS_SECONDS, Placement};
use super::frames::{self, PresentationCompanion, PresentationPage, PresentationPageSource};
use super::origin_replies::{
    COMPANION_DEADLINE, NOT_ATTACHABLE, PAUSE, READY_BUDGET, origin_created, read_remote_tree, reply_of, unattached,
};
use super::ssh::{control_command, page_serve_command};
use super::tree::{RemoteCommandResult, TREE_DEADLINE};
use super::viewer::{Answer, run_off_thread, take_answer};
use crate::Core;
use crate::dispatch::Dispatched;
use crate::error::ControlError;
use crate::html::{self, HtmlOverlay, HtmlPageFlags, HtmlSource};
use crate::model::{CompanionOptions, RemoteBinding, ReplicaPageRefusal, SessionContent, SidebarTint};
use crate::pane_env::uuid_string;
use crate::resolve::Resolver;

/// What a created companion is called in the messages about it.
const COMPANION_NOUN: &str = "companion";
/// How often a running ssh call is checked for its answer.
const POLL: Duration = Duration::from_millis(50);
const NOT_ATTACHED: &str = "the remote session is not attached";
const INVALID_HOST: &str = "invalid host";
const PARENT_CLOSED: &str = "its parent row here was closed";

/// A page companion the origin is asked to open: the page `session.companion.page` validated.
#[derive(Debug, Clone)]
pub(crate) struct RemotePage {
    pub(crate) source: HtmlSource,
    pub(crate) flags: HtmlPageFlags,
    pub(crate) background_color: Option<String>,
}

/// A companion the origin of a remote row is asked to create under its own session (upstream's
/// `RemoteCompanionRequest`).
#[derive(Debug, Clone)]
pub(crate) enum RemoteCompanionRequest {
    Terminal(CompanionOptions),
    Duplicate,
    Page(RemotePage),
}

impl RemoteCompanionRequest {
    fn tint(&self) -> Option<SidebarTint> {
        match self {
            Self::Terminal(options) => options.tint,
            Self::Duplicate | Self::Page(_) => None,
        }
    }

    fn command(&self) -> Command {
        Command::Session(match self {
            Self::Terminal(_) => SessionCmd::CompanionNew,
            Self::Duplicate => SessionCmd::CompanionDuplicate,
            Self::Page(_) => SessionCmd::CompanionPage,
        })
    }

    /// The origin's command line: the origin keeps its own selection, as the companion is wanted here, not there.
    fn arguments(&self, origin: &str) -> Vec<String> {
        let words = |list: &[&str]| list.iter().map(|word| (*word).to_owned()).collect::<Vec<String>>();
        match self {
            Self::Duplicate => words(&["session", "duplicate", "--companion", "--target", origin, "--no-select"]),
            Self::Terminal(options) => {
                let mut arguments = words(&["session", "new", "--parent", origin, "--no-select"]);
                let flag = |name: &str, value: &Option<String>| {
                    value
                        .iter()
                        .flat_map(|value| [name.to_owned(), value.clone()])
                        .collect::<Vec<String>>()
                };
                arguments.extend(flag("--cwd", &options.cwd));
                arguments.extend(flag("--command", &options.command));
                if options.wait {
                    arguments.push("--wait".to_owned());
                }
                arguments.extend(flag("--name", &options.name));
                arguments
            }
            Self::Page(page) => page_arguments(page, origin),
        }
    }
}

/// A page companion's line: a file path is the origin's own, as the page is that machine's and is served from there.
fn page_arguments(page: &RemotePage, origin: &str) -> Vec<String> {
    let mut arguments: Vec<String> = ["session", "overlay", "open", "--companion", "--target", origin]
        .map(str::to_owned)
        .to_vec();
    match &page.source {
        HtmlSource::File { path, grant_root } => {
            arguments.extend(["--html".to_owned(), path.clone()]);
            if let Some(root) = grant_root {
                arguments.extend(["--cwd".to_owned(), root.clone()]);
            }
        }
        HtmlSource::Url(url) => arguments.extend(["--url".to_owned(), url.as_str().to_owned()]),
        HtmlSource::Replica { .. }
        | HtmlSource::Image { .. }
        | HtmlSource::Markdown { .. }
        | HtmlSource::Text { .. }
        | HtmlSource::Document { .. } => {}
    }
    let flags = &page.flags;
    let set = [
        (flags.navigation, "--navigation"),
        (flags.javascript, "--js"),
        (flags.chromeless, "--chromeless"),
        (flags.persistent, "--persistent"),
        (flags.browse, "--browse"),
    ];
    arguments.extend(set.iter().filter(|(on, _)| *on).map(|(_, flag)| (*flag).to_owned()));
    if let Some(color) = &page.background_color {
        arguments.extend(["--background-color".to_owned(), color.clone()]);
    }
    arguments
}

/// What a run answers: the reply's result, or why it failed.
pub(crate) type Answered = Result<ControlResult, String>;

/// One step of a run: not done yet (run it again after the delay), or its answer, boxed as a reply is large.
pub(crate) enum Step<T> {
    Pending(T, Duration),
    Done(Box<Answered>),
}

impl<T> Step<T> {
    fn done(answered: Answered) -> Self {
        Self::Done(Box::new(answered))
    }
}

/// Where a run is.
enum Stage {
    /// The origin is asked whether the parent is attachable there; the create runs once it is.
    Checking {
        answer: Answer,
        listed: String,
        create: Vec<String>,
    },
    Creating(Answer),
    /// The origin is listed until the created session is attachable.
    Waiting {
        created: String,
        deadline: Instant,
        reason: String,
        read: Option<Answer>,
    },
    /// A created page companion is being attached.
    Page {
        created: String,
        attach: PageAttach,
    },
}

/// One remote companion being created and attached (upstream's `createRemoteCompanion`).
pub(crate) struct CompanionFlow {
    request: RemoteCompanionRequest,
    host: String,
    /// The row the companion lands under: the remote row, or its parent for a companion row.
    parent: Uuid,
    select: bool,
    stage: Stage,
    /// A progress line not shown yet.
    progress: Option<String>,
}

/// One attachment of a page companion of another machine being shown here (upstream's `attachRemotePage`).
#[derive(Debug)]
pub(crate) struct PageAttach {
    origin_session: String,
    host: String,
    parent: Uuid,
    select: bool,
    /// The row already showing this companion answers the attach instead of a second one. Only the two attaches that
    /// can race for one new companion set it, a headless origin's handover and this machine's own create; a
    /// `live attach` of a shown page shows it again, as upstream's does.
    reuse_shown: bool,
    answer: Answer,
}

impl PageAttach {
    /// This attach answers with a row already showing its companion ([`PageAttach::reuse_shown`]).
    fn reusing_shown(self) -> Self {
        Self {
            reuse_shown: true,
            ..self
        }
    }
}

/// A control request waiting on `flow`.
pub(crate) fn drive(flow: CompanionFlow, after: Duration) -> Dispatched {
    Dispatched::Pending {
        after,
        resume: Box::new(move |core: &mut Core| match core.step_companion(flow) {
            Step::Pending(flow, after) => drive(flow, after),
            Step::Done(outcome) => Dispatched::done((*outcome).map_err(ControlError::rejected)),
        }),
    }
}

fn drive_page(attach: PageAttach) -> Dispatched {
    Dispatched::Pending {
        after: POLL,
        resume: Box::new(move |core: &mut Core| match core.step_page_attach(attach) {
            Step::Pending(attach, _) => drive_page(attach),
            Step::Done(outcome) => Dispatched::done((*outcome).map_err(ControlError::rejected)),
        }),
    }
}

/// `live.attach HOST ID` of a page companion (upstream's `attachRemotePageRow`): attached under the one row here, in
/// the request's window, showing its parent from the same origin.
pub(crate) fn begin_page_row_attach(
    core: &mut Core,
    remote: &ControlRemoteSession,
    tree: &ControlRemoteTree,
    request: &AttachRequest,
) -> Dispatched {
    let host = request.host.as_str();
    let window = match Resolver::new(core.library()).open_placement_window(request.window.as_deref()) {
        Ok(window) => window,
        Err(error) => return Dispatched::done(Err(error)),
    };
    let rows = matching_parent_rows(core, window, host, remote.parent.as_deref(), &tree.endpoint);
    let [parent] = rows[..] else {
        let error = if rows.is_empty() {
            format!("attach the page's parent session from {host} first")
        } else {
            "more than one row here shows the page's parent".to_owned()
        };
        return Dispatched::done(Err(ControlError::rejected(error)));
    };
    match core.start_page_attach(&remote.id, host, parent, true) {
        Ok(attach) => drive_page(attach),
        Err(error) => Dispatched::done(Err(ControlError::rejected(error))),
    }
}

/// Top-level rows in this destination window showing exactly this origin parent.
pub(super) fn matching_parent_rows(
    core: &Core,
    window: Uuid,
    host: &str,
    parent: Option<&str>,
    endpoint: &ngterm_proto::ControlLiveEndpoint,
) -> Vec<Uuid> {
    core.library()
        .store(window)
        .map(|store| {
            store
                .sessions()
                .filter(|row| row.parent_id().is_none() && row.remote_host() == Some(host))
                .filter(|row| {
                    row.remote_presentation().is_some_and(|state| {
                        let binding = &state.binding;
                        Some(binding.remote_session_id.as_str()) == parent
                            && binding.origin.as_ref().map(|origin| &origin.endpoint) == Some(endpoint)
                    })
                })
                .map(crate::model::Session::id)
                .collect()
        })
        .unwrap_or_default()
}

/// What the origin's create said: the created session's id, or why there is none.
fn created(result: &RemoteCommandResult, host: &str, command: Command) -> Result<String, String> {
    let refused = reply_of(result).is_some_and(|reply| reply.refuses_unknown(command));
    if result.refused_arguments() || refused {
        return Err(format!("the ngterm on {host} is too old for companions"));
    }
    origin_created(result, host, COMPANION_NOUN)
}

/// What the origin handed over for one attachment of its page companion.
fn handed_page(result: &RemoteCommandResult, host: &str) -> Result<PresentationPage, String> {
    let too_old = || format!("the ngterm on {host} is too old for page companions");
    if result.refused_arguments() {
        return Err(too_old());
    }
    let Some(reply) = reply_of(result) else {
        let stderr = result.stderr.trim();
        return Err(if stderr.is_empty() {
            format!("{host} gave no answer")
        } else {
            stderr.to_owned()
        });
    };
    let page = reply
        .result
        .as_ref()
        .and_then(|result| result.companion_page.clone())
        .and_then(|page| serde_json::from_value::<PresentationPage>(page).ok());
    match page {
        Some(page) if result.status == 0 && reply.ok => Ok(page),
        _ => {
            if reply.refuses_unknown(Command::Session(SessionCmd::CompanionPageAttach)) {
                return Err(too_old());
            }
            Err(reply
                .error
                .unwrap_or_else(|| format!("{host} did not hand the page over")))
        }
    }
}

impl Core {
    /// The clock the remote companion waits are timed by (tests advance it).
    pub(super) fn remote_now(&self) -> Instant {
        (self.remote.clock)()
    }

    /// Starts creating `request` under remote row `row` (upstream's `createRemoteCompanion` up to its first ssh
    /// call): the origin is first asked whether the parent is still attachable there, as a binding can outlive an
    /// origin relaunched into fresh shells, where nothing created could be attached.
    ///
    /// # Errors
    ///
    /// The row is not an attached remote row, or its host cannot be handed to ssh.
    pub(crate) fn start_remote_companion(
        &mut self,
        request: RemoteCompanionRequest,
        row: Uuid,
        select: bool,
    ) -> Result<CompanionFlow, String> {
        let store = self.library().store_for_session(row).ok_or(NOT_ATTACHED)?;
        let found = store.session(row).ok_or(NOT_ATTACHED)?;
        let (Some(host), Some(state)) = (found.remote_host(), found.remote_presentation()) else {
            return Err(NOT_ATTACHED.to_owned());
        };
        let origin = state.binding.remote_session_id.clone();
        let parent = store.parent_of(found).unwrap_or(found);
        // the companion lands under the parent, so the parent is what must still be attachable there
        let listed = parent
            .remote_presentation()
            .map_or_else(|| origin.clone(), |state| state.binding.remote_session_id.clone());
        let (host, parent) = (host.to_owned(), parent.id());
        let create = control_command(&host, &request.arguments(&origin)).map_err(|_| INVALID_HOST.to_owned())?;
        let answer = super::attach::start_tree_read(self, &host).map_err(|error| error.to_string())?;
        Ok(CompanionFlow {
            progress: Some(format!("creating a companion on {host}…")),
            request,
            host,
            parent,
            select,
            stage: Stage::Checking { answer, listed, create },
        })
    }

    /// Moves `flow` on by what has arrived.
    pub(crate) fn step_companion(&mut self, mut flow: CompanionFlow) -> Step<CompanionFlow> {
        let stage = std::mem::replace(&mut flow.stage, Stage::Creating(Answer::default()));
        match stage {
            Stage::Checking { answer, listed, create } => match take_answer(&answer) {
                None => {
                    flow.stage = Stage::Checking { answer, listed, create };
                    Step::Pending(flow, POLL)
                }
                Some(result) => self.checked(flow, &result, &listed, create),
            },
            Stage::Creating(answer) => match take_answer(&answer) {
                None => {
                    flow.stage = Stage::Creating(answer);
                    Step::Pending(flow, POLL)
                }
                Some(result) => self.created(flow, &result),
            },
            Stage::Waiting {
                created,
                deadline,
                reason,
                read,
            } => self.wait_attachable(flow, created, deadline, reason, read),
            Stage::Page { created, attach } => match self.step_page_attach(attach) {
                Step::Pending(attach, after) => {
                    flow.stage = Stage::Page { created, attach };
                    Step::Pending(flow, after)
                }
                Step::Done(done) => {
                    Step::done((*done).map_err(|reason| unattached(COMPANION_NOUN, &created, &flow.host, &reason)))
                }
            },
        }
    }

    fn checked(
        &mut self,
        mut flow: CompanionFlow,
        result: &RemoteCommandResult,
        listed: &str,
        create: Vec<String>,
    ) -> Step<CompanionFlow> {
        let host = flow.host.clone();
        let tree = match read_remote_tree(result, &host) {
            Ok(tree) => tree,
            Err(error) => return Step::done(Err(error)),
        };
        if !tree.sessions.iter().any(|session| session.id == listed) {
            return Step::done(Err(format!(
                "the session on {host} is not attachable now; nothing was created"
            )));
        }
        match run_off_thread(&self.remote.runner, create, COMPANION_DEADLINE) {
            Some(answer) => {
                flow.stage = Stage::Creating(answer);
                Step::Pending(flow, POLL)
            }
            None => Step::done(Err(format!("the remote command failed on {host}"))),
        }
    }

    fn created(&mut self, mut flow: CompanionFlow, result: &RemoteCommandResult) -> Step<CompanionFlow> {
        let host = flow.host.clone();
        let created = match created(result, &host, flow.request.command()) {
            Ok(created) => created,
            Err(error) => return Step::done(Err(error)),
        };
        flow.progress = Some("attaching the companion…".to_owned());
        if matches!(flow.request, RemoteCompanionRequest::Page(_)) {
            // a page is attachable as soon as it exists, so there is nothing to wait for
            return match self.start_page_attach(&created, &host, flow.parent, flow.select) {
                // a headless origin's handover of the same companion may attach it first
                Ok(attach) => {
                    let attach = attach.reusing_shown();
                    flow.stage = Stage::Page { created, attach };
                    Step::Pending(flow, POLL)
                }
                Err(reason) => Step::done(Err(unattached(COMPANION_NOUN, &created, &host, &reason))),
            };
        }
        flow.stage = Stage::Waiting {
            created,
            deadline: self.remote_now() + READY_BUDGET,
            reason: NOT_ATTACHABLE.to_owned(),
            read: None,
        };
        Step::Pending(flow, Duration::ZERO)
    }

    /// Lists the origin until the created session shows up attachable, then attaches it under the parent (upstream's
    /// `attachCreatedCompanion`). The parent's window and group are read again after every wait, as the row may have
    /// moved or closed meanwhile.
    fn wait_attachable(
        &mut self,
        mut flow: CompanionFlow,
        created: String,
        deadline: Instant,
        mut reason: String,
        read: Option<Answer>,
    ) -> Step<CompanionFlow> {
        let host = flow.host.clone();
        let fail = |reason: &str| Step::done(Err(unattached(COMPANION_NOUN, &created, &host, reason)));
        let Some(answer) = read else {
            let remaining = deadline.saturating_duration_since(self.remote_now());
            if remaining.is_zero() {
                return fail(&reason);
            }
            let answer = match super::attach::start_tree_read_within(self, &host, remaining.min(TREE_DEADLINE)) {
                Ok(answer) => answer,
                Err(error) => return fail(&error.to_string()),
            };
            flow.stage = Stage::Waiting {
                created,
                deadline,
                reason,
                read: Some(answer),
            };
            return Step::Pending(flow, POLL);
        };
        let Some(result) = take_answer(&answer) else {
            flow.stage = Stage::Waiting {
                created,
                deadline,
                reason,
                read: Some(answer),
            };
            return Step::Pending(flow, POLL);
        };
        let listed = read_remote_tree(&result, &host);
        let remote = match &listed {
            Ok(tree) => tree.sessions.iter().find(|session| session.id == created),
            Err(error) => {
                reason.clone_from(error);
                None
            }
        };
        let (Ok(tree), Some(remote)) = (&listed, remote) else {
            flow.stage = Stage::Waiting {
                created,
                deadline,
                reason,
                read: None,
            };
            return Step::Pending(flow, PAUSE);
        };
        Step::done(
            self.attach_created(&flow, remote, tree)
                .map_err(|reason| unattached(COMPANION_NOUN, &created, &host, &reason)),
        )
    }

    fn attach_created(
        &mut self,
        flow: &CompanionFlow,
        remote: &ControlRemoteSession,
        tree: &ControlRemoteTree,
    ) -> Result<ControlResult, String> {
        let placement = Placement::under(self.library(), flow.parent, flow.select).ok_or(PARENT_CLOSED)?;
        let attached = self
            .attach_remote_session(remote, tree, &flow.host, placement)
            .map_err(|error| error.to_string())?;
        if let Some(tint) = flow.request.tint()
            && let Some(id) = attached.id.as_deref().and_then(frames::parse_uuid)
            && let Some(store) = self.library_mut().store_for_session_mut(id)
        {
            store.set_session_tint(Some(tint), id);
        }
        Ok(attached)
    }

    // MARK: - a page companion's attachment

    /// Asks `host` for one attachment of its page companion `origin_session`, to show under row `parent` here.
    ///
    /// # Errors
    ///
    /// The host cannot be handed to ssh, or no thread could run the call.
    pub(crate) fn start_page_attach(
        &mut self,
        origin_session: &str,
        host: &str,
        parent: Uuid,
        select: bool,
    ) -> Result<PageAttach, String> {
        let argv = control_command(host, &["session", "overlay", "page-attach", "--target", origin_session])
            .map_err(|_| INVALID_HOST.to_owned())?;
        let answer = run_off_thread(&self.remote.runner, argv, COMPANION_DEADLINE)
            .ok_or_else(|| format!("the remote command failed on {host}"))?;
        Ok(PageAttach {
            origin_session: origin_session.to_owned(),
            host: host.to_owned(),
            parent,
            select,
            reuse_shown: false,
            answer,
        })
    }

    pub(crate) fn step_page_attach(&mut self, attach: PageAttach) -> Step<PageAttach> {
        match take_answer(&attach.answer) {
            None => Step::Pending(attach, POLL),
            Some(result) => {
                Step::done(handed_page(&result, &attach.host).and_then(|page| self.show_companion_page(&attach, &page)))
            }
        }
    }

    /// Shows the page `host` handed over as a page companion row under `parent` (the rest of upstream's
    /// `attachRemotePage`); the row carries the origin's page session, so New Companion on it makes a sibling there.
    fn show_companion_page(&mut self, attach: &PageAttach, page: &PresentationPage) -> Result<ControlResult, String> {
        let host = attach.host.as_str();
        if let PresentationPageSource::Url { address } = &page.source
            && html::is_loopback(address)
        {
            return Err(format!("a page on {host}'s loopback cannot be shown on this Mac"));
        }
        let web = self.html_host().filter(|web| web.web_views_unavailable().is_none());
        let Some(web) = web else {
            return Err(super::client::PAGES_UNSUPPORTED.to_owned());
        };
        // the other machine's browser profile says nothing about this one's
        if page.persistent
            && let Some(failure) = web.persistent_store_failure()
        {
            return Err(failure);
        }
        if self.library().store_for_session(attach.parent).is_none() {
            return Err(PARENT_CLOSED.to_owned());
        }
        // the origin's handover and this machine's own create can both attach the same companion
        if attach.reuse_shown
            && let Some(shown) = self.shown_page_companion(attach.parent, &attach.origin_session)
        {
            return Ok(self.reuse_page_companion(attach, shown));
        }
        let cannot_show = || "the page cannot be shown here".to_owned();
        let origin = frames::parse_uuid(&page.page).ok_or_else(cannot_show)?;
        let source = super::replica_page::replica_source(&page.source).ok_or_else(cannot_show)?;
        let serve =
            page_serve_command(host, &page.token).map_err(|_| format!("the page cannot be reached on {host}"))?;
        let flags = HtmlPageFlags {
            navigation: page.navigation,
            javascript: page.javascript,
            chromeless: page.chromeless,
            persistent: page.persistent,
            browse: page.browse,
        };
        let mut overlay = HtmlOverlay::new(source, flags);
        overlay.replica_of = Some(origin);
        let local = overlay.id;
        let home = self.library().home().to_owned();
        let store = self
            .library_mut()
            .store_for_session_mut(attach.parent)
            .ok_or(PARENT_CLOSED)?;
        let row = store
            .add_replica_page_companion(
                attach.parent,
                overlay,
                page.background_color.clone(),
                attach.select,
                &home,
            )
            .map_err(|refusal| match refusal {
                ReplicaPageRefusal::ParentGone => "the parent row here is gone".to_owned(),
                ReplicaPageRefusal::NotOpened(failure) => {
                    crate::commands::overlay_page::open_failure(failure, None).to_owned()
                }
            })?;
        store.bind_remote(
            RemoteBinding::new(attach.origin_session.clone(), HashMap::new(), None, None),
            row,
        );
        // its outcome is the other machine's, copied in when that machine ends the page; a close here dismisses it
        self.register_page_outcome(local);
        self.follow_companion_replica(local, row, origin, host, &serve);
        Ok(ControlResult {
            page_id: Some(uuid_string(local)),
            parent: Some(uuid_string(attach.parent)),
            ..ControlResult::id(uuid_string(row))
        })
    }

    /// The row here already showing the origin's page companion `origin_session` under `parent`, and its page.
    fn shown_page_companion(&self, parent: Uuid, origin_session: &str) -> Option<(Uuid, Uuid)> {
        let store = self.library().store_for_session(parent)?;
        store.companions(parent).into_iter().find_map(|row| {
            let bound = row
                .remote_presentation()
                .is_some_and(|state| state.binding.remote_session_id == origin_session);
            let page = row
                .overlay
                .page()
                .filter(|_| bound && row.content() == SessionContent::Page)?;
            Some((row.id(), page.id))
        })
    }

    /// Answers an attach of a page companion already shown here with that row, selected when the attach asked to be.
    fn reuse_page_companion(&mut self, attach: &PageAttach, (row, page): (Uuid, Uuid)) -> ControlResult {
        if attach.select
            && let Some(store) = self.library_mut().store_for_session_mut(row)
        {
            store.select_session(Some(row));
        }
        ControlResult {
            page_id: Some(uuid_string(page)),
            parent: Some(uuid_string(attach.parent)),
            ..ControlResult::id(uuid_string(row))
        }
    }

    // MARK: - a headless origin's handover

    /// Attaches under row `row` the page companion its origin created and handed over (this build's
    /// `companion.page`), unless it is shown here or on its way already. A failure is reported, never retried: the
    /// group's `live attach` attaches it later.
    pub(crate) fn attach_handed_companion(&mut self, row: Uuid, companion: &PresentationCompanion) {
        let origin_session = companion.session.as_str();
        let pending = self
            .remote
            .viewer
            .handed_companions
            .iter()
            .any(|attach| attach.parent == row && attach.origin_session == origin_session);
        if pending || self.shown_page_companion(row, origin_session).is_some() {
            return;
        }
        let host = self
            .library()
            .held_session(row)
            .filter(|found| found.parent_id().is_none())
            .and_then(|found| found.remote_host())
            .map(str::to_owned);
        let Some(host) = host else {
            return;
        };
        match self.start_page_attach(origin_session, &host, row, false) {
            Ok(attach) => self.remote.viewer.handed_companions.push(attach.reusing_shown()),
            Err(reason) => self.handed_companion_failed(row, &host, &reason),
        }
    }

    /// Moves every handed-over page companion's attach on.
    pub(crate) fn advance_handed_companions(&mut self) {
        for attach in std::mem::take(&mut self.remote.viewer.handed_companions) {
            let (row, host) = (attach.parent, attach.host.clone());
            match self.step_page_attach(attach) {
                Step::Pending(attach, _) => self.remote.viewer.handed_companions.push(attach),
                Step::Done(done) => {
                    if let Err(reason) = *done {
                        self.handed_companion_failed(row, &host, &reason);
                    }
                }
            }
        }
    }

    fn handed_companion_failed(&mut self, row: Uuid, host: &str, reason: &str) {
        tracing::info!(%row, host, reason, "a page companion from the origin was not shown");
        self.post_app_banner(
            format!("remote-page-companion-{row}"),
            "Page Companion",
            format!("Could not show a page companion from {host}: {reason}"),
        );
    }

    /// The page of row `row` ended on `host` (the channel's end): its outcome there becomes the view's, then the row
    /// closes; a row parked by a soft close is made final instead, as Undo would bring back a page that ended
    /// (upstream's `channel.onClose` and `settleRemotePageOutcome`). An unreadable answer leaves the close's dismissal.
    pub(crate) fn page_companion_ended(&mut self, local: Uuid, row: Uuid, origin: Uuid, host: &str) {
        let argv = self
            .page_outcome_pending(local)
            .then(|| control_command(host, &["session", "overlay", "result", "--page", &uuid_string(origin)]).ok())
            .flatten();
        let answer = argv.and_then(|argv| run_off_thread(&self.remote.runner, argv, TREE_DEADLINE));
        match answer {
            Some(answer) => self.remote.viewer.page_endings.push(PageEnding { local, row, answer }),
            None => self.close_ended_page_companion(local, row),
        }
    }

    /// Settles the page endings whose outcome read arrived.
    pub(crate) fn settle_page_endings(&mut self) {
        let endings = std::mem::take(&mut self.remote.viewer.page_endings);
        for ending in endings {
            let Some(result) = take_answer(&ending.answer) else {
                self.remote.viewer.page_endings.push(ending);
                continue;
            };
            let outcome = reply_of(&result).and_then(|reply| reply.result?.page_outcome);
            self.settle_page_outcome(ending.local, outcome.as_ref());
            self.close_ended_page_companion(ending.local, ending.row);
        }
    }

    fn close_ended_page_companion(&mut self, local: Uuid, row: Uuid) {
        if self.library_mut().close_html_overlay(local) {
            return;
        }
        if let Some(window) = self.library().window_holding_session(row)
            && let Some(store) = self.library_mut().store_mut(window)
        {
            store.finalize_pending_close_of_session(row);
        }
    }

    // MARK: - the GUI

    /// New Companion or Duplicate as Companion (`action`) on remote row `session` of window `window` (upstream's
    /// `AppActions.createRemoteCompanion`): the origin creates it, the two ssh waits and a failure show in a HUD over
    /// the row, and the remote tick moves it on. Whether anything changed.
    pub(crate) fn begin_gui_remote_companion(
        &mut self,
        request: RemoteCompanionRequest,
        window: Uuid,
        session: Uuid,
        action: &'static str,
    ) -> bool {
        let mut gui = GuiCompanion {
            action,
            window,
            session,
            owned_hud: None,
            due: Instant::now(),
            flow: None,
        };
        match self.start_remote_companion(request, session, true) {
            Ok(flow) => {
                gui.flow = Some(flow);
                self.show_companion_progress(&mut gui);
                self.remote.viewer.companion_flows.push(gui);
            }
            Err(reason) => self.gui_companion_failed(&mut gui, &reason),
        }
        true
    }

    /// Moves every GUI companion run on whose wait is over.
    pub(crate) fn advance_companion_flows(&mut self, now: Instant) {
        let runs = std::mem::take(&mut self.remote.viewer.companion_flows);
        for mut gui in runs {
            if gui.due > now {
                self.remote.viewer.companion_flows.push(gui);
                continue;
            }
            let Some(flow) = gui.flow.take() else {
                continue;
            };
            match self.step_companion(flow) {
                Step::Pending(flow, after) => {
                    gui.flow = Some(flow);
                    gui.due = now + after;
                    self.show_companion_progress(&mut gui);
                    self.remote.viewer.companion_flows.push(gui);
                }
                Step::Done(done) => match *done {
                    Ok(_) => {
                        self.close_companion_hud(&mut gui);
                        self.note_user_activity(gui.session);
                    }
                    Err(reason) => self.gui_companion_failed(&mut gui, &reason),
                },
            }
        }
    }

    fn show_companion_progress(&mut self, gui: &mut GuiCompanion) {
        let Some(text) = gui.flow.as_mut().and_then(|flow| flow.progress.take()) else {
            return;
        };
        let panel = ActionPanel {
            action: gui.action,
            window: gui.window,
            session: gui.session,
        };
        gui.owned_hud = self.open_action_hud(&panel, &text, PROGRESS_SECONDS, true);
    }

    /// The panel this run opened closes, never one another caller posted meanwhile.
    fn close_companion_hud(&mut self, gui: &mut GuiCompanion) {
        if let Some(owned) = gui.owned_hud.take() {
            self.close_owned_hud(gui.session, owned);
        }
    }

    /// A timed error panel over the row, else a banner (upstream's `RemoteAttachHud.fail`).
    fn gui_companion_failed(&mut self, gui: &mut GuiCompanion, reason: &str) {
        tracing::info!(action = gui.action, reason, "a remote companion was not created");
        let panel = ActionPanel {
            action: gui.action,
            window: gui.window,
            session: gui.session,
        };
        if self.open_action_hud(&panel, reason, ERROR_SECONDS, false).is_some() {
            return;
        }
        self.close_companion_hud(gui);
        self.action_failed(gui.action, gui.window, None, reason);
    }
}

/// A page companion's page that ended on its origin, waiting for that machine's outcome before its row closes.
pub(crate) struct PageEnding {
    local: Uuid,
    row: Uuid,
    answer: Answer,
}

/// A GUI companion run and its panel.
pub(crate) struct GuiCompanion {
    action: &'static str,
    window: Uuid,
    session: Uuid,
    /// The slot generation of the panel this run opened.
    owned_hud: Option<u64>,
    /// When the run is moved on next.
    due: Instant,
    flow: Option<CompanionFlow>,
}
