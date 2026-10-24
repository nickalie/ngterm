//! Pages an origin handed to this machine, on the presenter (upstream's `AppStore+ReplicaPage.swift` and the app's
//! `ControlServer+RemotePage.swift`): a page the origin's placement rule sent here is shown in the mapped pane or the
//! session-wide slot under a page id of this machine's own, with `replica_of` naming the origin's.
//!
//! - A URL page loads directly, on this machine's browser store. A file page loads from
//!   `ngterm-page://<local id>/<origin path>` ([`crate::html::REPLICA_SCHEME`]): the frontend's scheme handler asks
//!   [`Core::replica_fetch`] for each file, and the page's bridge relays through [`Core::replica_bridge`], both over the
//!   page's channel ([`super::page_channel`]) to the origin, carried by a helper (`ssh … ngtermctl session overlay
//!   page-serve TOKEN`) the frontend's [`PageHelperLauncher`] starts.
//! - The origin's commands act on the page here (hide and show its slot only, reload, history, browser, resize), and
//!   what the page shows goes back as `page.state` with the last command's sequence number, once per commit at most.
//! - Every way a replica leaves its slot (the origin's close, a close here, the helper's end, the stream's loss, the
//!   row leaving) tells the origin `page.closed` once and closes its channel.
//!
//! The viewer's hello lists the page kinds only when this build can show a page (the web views are built in and can
//! run), where upstream's hello lists them always: an origin then keeps its own refusal instead of handing over a page
//! refused at once. Everything here is toolkit-neutral; the scheme handler is the frontend's. On Linux it is WPE
//! `WebKit`'s, answering each read whole (`ngterm_wpe::SchemeResponse::Bytes`), as `WebKit`'s `GLib` ports tell a scheme
//! handler nothing of a read stopped before its answer; a read for a load the page left is withdrawn here instead
//! ([`super::page_channel::ReplicaChannel::revision_moved`]). The macOS form is a `WKURLSchemeHandler` for
//! `ngterm-page` (upstream's `HtmlPageSchemeHandler`), the Windows form `WebView2`'s custom scheme registration with a
//! deferred `WebResourceRequested`; both belong to the web-view ports of those frontends.

use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;
use std::sync::Arc;

use uuid::Uuid;

use super::frames::{
    self, Body, PageShown, PresentationPage, PresentationPageAction, PresentationPageChange, PresentationPageCommand,
    PresentationPageSource, PresentationPageState,
};
use super::page_channel::{ChannelEvent, ChannelLink, Permits, Pull, ReplicaChannel, RequestId};
use crate::Core;
use crate::html::{HtmlNavigation, HtmlOverlay, HtmlPageFlags, HtmlReloadTarget, HtmlSource, errors};
use crate::model::{HtmlPageSlot, OverlayPane};
use crate::pane_env::uuid_string;

/// Why a file page cannot be shown: the session names no host, or the token cannot travel (upstream's refusal).
pub const ORIGIN_UNREACHABLE: &str = "the origin cannot be reached for this page";
/// The page was placed and is gone already (upstream's refusal).
const NOT_SHOWN: &str = "page not shown";
const NO_SESSION: &str = "no such session";
const INVALID_PAGE: &str = "invalid page id";
const INVALID_URL: &str = "invalid url";
const NO_HELPER: &str = "no page helper";
/// The MIME type of an answer that named none.
const OCTET_STREAM: &str = "application/octet-stream";

/// One launch of a page's helper, as its reports name it: a report from an earlier helper of the same page is dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HelperKey {
    pub page: Uuid,
    pub launch: u64,
}

/// Starts the helper that carries a file page's channel (upstream's `RemotePageLauncher`). The frontend reports each
/// line it reads through [`Core::page_helper_line`], after taking one of `permits` (and stops reading once it gets
/// none), and its end through [`Core::page_helper_closed`], both under `key`.
pub trait PageHelperLauncher {
    /// Starts `argv`.
    ///
    /// # Errors
    ///
    /// Why the helper did not start, which closes the page.
    fn launch(&self, argv: &[String], key: HelperKey, permits: Arc<Permits>) -> Result<Box<dyn ChannelLink>, String>;
}

/// A file the origin served.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageResource {
    pub mime_type: String,
    pub body: Vec<u8>,
}

/// Where a replica's file read is answered: called once, on the main loop.
pub type FetchAnswer = Box<dyn FnOnce(Result<PageResource, String>)>;
/// Where a replica's bridge request is answered: the result the page's promise resolves with, or its rejection.
pub type BridgeAnswer = Box<dyn FnOnce(Result<serde_json::Value, String>)>;

/// A read in flight.
struct Fetch {
    mime_type: Option<String>,
    body: Vec<u8>,
    answer: FetchAnswer,
}

/// A file page's channel and the calls waiting on it.
struct PageLink {
    key: HelperKey,
    channel: ReplicaChannel,
    /// The page load the channel's requests are made for.
    revision: u64,
    fetches: HashMap<RequestId, Fetch>,
    bridges: HashMap<RequestId, BridgeAnswer>,
}

/// A page shown here for an origin (upstream's `ReplicaPageRecord`).
struct ReplicaRecord {
    session: Uuid,
    origin: Uuid,
    link: Option<PageLink>,
    /// The last origin command applied.
    applied_seq: i64,
    /// The state the origin last got.
    reported: Option<PresentationPageState>,
}

/// The pages shown here for origins, by local page id, and how their helpers start.
#[derive(Default)]
pub(crate) struct ReplicaPages {
    records: HashMap<Uuid, ReplicaRecord>,
    launcher: Option<Rc<dyn PageHelperLauncher>>,
    launches: u64,
}

impl fmt::Debug for ReplicaPages {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReplicaPages")
            .field("records", &self.records.keys().collect::<Vec<_>>())
            .field("launcher", &self.launcher.is_some())
            .finish_non_exhaustive()
    }
}

/// What a channel's events leave to do once the record is let go: the answers to call and whether the page closes.
#[derive(Default)]
struct Settled {
    fetches: Vec<(FetchAnswer, Result<PageResource, String>)>,
    bridges: Vec<(BridgeAnswer, Result<serde_json::Value, String>)>,
    closed: bool,
}

impl Settled {
    fn run(self) {
        for (answer, result) in self.fetches {
            answer(result);
        }
        for (answer, result) in self.bridges {
            answer(result);
        }
    }
}

impl PageLink {
    /// Applies `events` to the calls waiting here.
    fn settle(&mut self, events: Vec<ChannelEvent>, settled: &mut Settled) {
        for event in events {
            match event {
                ChannelEvent::Head { id, mime_type } => {
                    if let Some(fetch) = self.fetches.get_mut(&id) {
                        fetch.mime_type = Some(mime_type);
                    }
                }
                ChannelEvent::Data { id } => self.drain(id, settled),
                ChannelEvent::Finished { id, result } => {
                    self.finish(id, result.map_err(|error| error.to_string()), settled);
                }
                ChannelEvent::Reply { id, result } => {
                    if let Some(answer) = self.bridges.remove(&id) {
                        let result = result
                            .map_err(|error| error.to_string())
                            .and_then(|raw| crate::html::bridge::shape_raw_reply(&raw));
                        settled.bridges.push((answer, result));
                    }
                }
                ChannelEvent::Closed { .. } => settled.closed = true,
            }
        }
    }

    /// Takes every chunk of read `id` that waits: the page consumes them at once into the answer it builds.
    fn drain(&mut self, id: RequestId, settled: &mut Settled) {
        loop {
            match self.channel.pull(id) {
                Pull::Chunk(chunk) => {
                    if let Some(fetch) = self.fetches.get_mut(&id) {
                        fetch.body.extend(chunk);
                    }
                }
                Pull::Done(result) => {
                    self.finish(id, result.map_err(|error| error.to_string()), settled);
                    return;
                }
                Pull::Wait => return,
            }
        }
    }

    fn finish(&mut self, id: RequestId, result: Result<(), String>, settled: &mut Settled) {
        let Some(fetch) = self.fetches.remove(&id) else {
            return;
        };
        let result = result.map(|()| PageResource {
            mime_type: fetch.mime_type.unwrap_or_else(|| OCTET_STREAM.to_owned()),
            body: fetch.body,
        });
        settled.fetches.push((fetch.answer, result));
    }
}

impl Core {
    /// Installs how a file page's helper starts (the app's child process; tests a fake).
    pub fn set_page_helper_launcher(&mut self, launcher: Rc<dyn PageHelperLauncher>) {
        self.remote.viewer.pages.launcher = Some(launcher);
    }

    /// Whether this machine shows handed-over pages: its web views are built in and can run. The viewer's hello lists
    /// the page kinds only then.
    pub(crate) fn shows_replica_pages(&self) -> bool {
        self.html_host()
            .is_some_and(|host| host.web_views_unavailable().is_none())
    }

    /// Shows `page`, which session `session`'s origin handed over (upstream's `showReplicaPage`): a file page needs the
    /// session's host to start its helper, a persistent URL page the saved store.
    ///
    /// # Errors
    ///
    /// The reason the origin hears in `page.rejected`.
    pub(crate) fn show_replica_page(&mut self, page: &PresentationPage, session: Uuid) -> Result<(), String> {
        let Some(host) = self.html_host().filter(|host| host.web_views_unavailable().is_none()) else {
            return Err(super::client::PAGES_UNSUPPORTED.to_owned());
        };
        let found = self.presented_session(session).ok_or(NO_SESSION)?;
        let argv = match &page.source {
            PresentationPageSource::File { .. } => Some(
                found
                    .remote_host()
                    .and_then(|host| super::ssh::page_serve_command(host, &page.token).ok())
                    .ok_or(ORIGIN_UNREACHABLE)?,
            ),
            PresentationPageSource::Url { .. } => {
                if page.persistent
                    && let Some(failure) = host.persistent_store_failure()
                {
                    return Err(failure);
                }
                None
            }
        };
        let local = self.present_replica_page(page, session)?;
        let origin = self
            .replica_page_slot(local)
            .and_then(|(_, overlay)| overlay.replica_of)
            .ok_or(NOT_SHOWN)?;
        self.track_page(local);
        self.remote.viewer.pages.records.insert(
            local,
            ReplicaRecord {
                session,
                origin,
                link: None,
                applied_seq: 0,
                reported: None,
            },
        );
        // the state the page starts in is what the origin assumed: only a change is reported
        let initial = self.replica_page_state(local, 0);
        if let Some(record) = self.remote.viewer.pages.records.get_mut(&local) {
            record.reported = initial;
        }
        if let Some(argv) = argv {
            self.start_page_helper(local, &argv);
        }
        Ok(())
    }

    /// Places `page` in the mapped pane or the session-wide slot under a new id (upstream's `presentReplicaPage`): no
    /// outcome is registered, since the origin's ledger alone takes the page's answer. The page's local id.
    fn present_replica_page(&mut self, request: &PresentationPage, session: Uuid) -> Result<Uuid, String> {
        let store = self.library_mut().store_for_session_mut(session).ok_or(NO_SESSION)?;
        let found = store
            .session(session)
            .filter(|found| found.remote_presentation().is_some())
            .ok_or(NO_SESSION)?;
        let origin = frames::parse_uuid(&request.page).ok_or(INVALID_PAGE)?;
        let source = match &request.source {
            PresentationPageSource::File { path, .. } => HtmlSource::Replica { path: path.clone() },
            PresentationPageSource::Url { address } => {
                HtmlSource::Url(crate::html::web_url(address).ok_or(INVALID_URL)?)
            }
        };
        let pane = match &request.pane {
            None => None,
            Some(remote) => Some(
                found
                    .local_pane(Some(remote))
                    .filter(|role| found.renders_overlay_pane(*role))
                    .ok_or(crate::commands::hud::PANE_NOT_VISIBLE)?,
            ),
        };
        let flags = HtmlPageFlags {
            navigation: request.navigation,
            javascript: request.javascript,
            chromeless: request.chromeless,
            persistent: request.persistent,
            browse: request.browse,
        };
        let mut overlay = HtmlOverlay::new(source, flags);
        overlay.replica_of = Some(origin);
        let local = overlay.id;
        store
            .open_html_overlay(
                session,
                pane,
                overlay,
                request.size_percent,
                request.background_color.clone(),
            )
            .map_err(|failure| crate::commands::overlay_page::open_failure(failure, pane))?;
        if request.follow {
            store.select_session(Some(session));
        }
        Ok(local)
    }

    /// Starts the helper of file page `local`; one that cannot start closes the page at once.
    fn start_page_helper(&mut self, local: Uuid, argv: &[String]) {
        let pages = &mut self.remote.viewer.pages;
        pages.launches += 1;
        let key = HelperKey {
            page: local,
            launch: pages.launches,
        };
        let permits = ReplicaChannel::permits();
        let launched = match pages.launcher.clone() {
            Some(launcher) => launcher.launch(argv, key, Arc::clone(&permits)),
            None => Err(NO_HELPER.to_owned()),
        };
        match launched {
            Ok(link) => {
                let revision = self
                    .replica_page_slot(local)
                    .map_or(0, |(_, page)| page.reload_revision);
                if let Some(record) = self.remote.viewer.pages.records.get_mut(&local) {
                    record.link = Some(PageLink {
                        key,
                        channel: ReplicaChannel::new(link, permits),
                        revision,
                        fetches: HashMap::new(),
                        bridges: HashMap::new(),
                    });
                }
            }
            Err(reason) => {
                tracing::warn!(%reason, "a page's helper did not start; closing the page");
                self.library_mut().close_html_overlay(local);
            }
        }
    }

    /// Where page `local` sits and the page, in a visible session of an open window.
    fn replica_page_slot(&self, local: Uuid) -> Option<(HtmlPageSlot, &HtmlOverlay)> {
        let slot = self.library().html_overlay_slot(local)?;
        let session = self.library().store(slot.window)?.session(slot.session)?;
        let page = match slot.pane {
            Some(pane) => session.pane_overlay(pane)?.html.as_ref()?,
            None => session.overlay.page()?,
        };
        Some((slot, page))
    }

    /// The local id of the page session `session` shows for origin page `origin` (upstream's `replicaPage`).
    fn replica_local(&self, origin: &str, session: Uuid) -> Option<(Option<OverlayPane>, Uuid)> {
        let origin = frames::parse_uuid(origin)?;
        self.presented_session(session)?
            .html_overlays()
            .into_iter()
            .find(|(_, page)| page.replica_of == Some(origin))
            .map(|(pane, page)| (pane, page.id))
    }

    /// Takes down the page shown for `origin`, once its origin closed it (upstream's `closeReplicaPage`); the origin
    /// still hears `page.closed`, as for every way a replica leaves.
    pub(crate) fn close_replica_page(&mut self, origin: &str, session: Uuid) {
        let Some((pane, _)) = self.replica_local(origin, session) else {
            return;
        };
        if let Some(store) = self.library_mut().store_for_session_mut(session) {
            match pane {
                Some(pane) => store.close_pane_overlay(session, pane),
                None => store.close_overlay(session),
            };
        }
    }

    /// Takes down every page `session` shows for its origin (upstream's `closeReplicaPages`): with the stream gone the
    /// origin has ended them, and nothing could reach it from them. A session an undoable close holds included.
    pub(crate) fn close_replica_pages(&mut self, session: Uuid) {
        let Some(found) = self.library_mut().held_session_mut(session) else {
            return;
        };
        let pages: Vec<Option<OverlayPane>> = found
            .html_overlays()
            .into_iter()
            .filter(|(_, page)| page.replica_of.is_some())
            .map(|(pane, _)| pane)
            .collect();
        if pages.is_empty() {
            return;
        }
        for pane in pages {
            match pane {
                Some(pane) => {
                    found.pane_overlays.slot_mut(pane).close();
                }
                None => {
                    found.overlay.close();
                }
            }
        }
        self.library_mut().schedule_tree_changed_for(session);
    }

    /// Runs one origin request on the page shown for it (upstream's `applyReplicaPageCommand`): a refusal is logged,
    /// and the next state report carries the request's sequence number.
    pub(crate) fn apply_replica_page_command(&mut self, command: &PresentationPageCommand, session: Uuid) {
        let Some((pane, local)) = self.replica_local(&command.page, session) else {
            return;
        };
        let refusal = match command.action {
            PresentationPageAction::Hide | PresentationPageAction::Show => {
                self.set_replica_page_hidden(local, command.action == PresentationPageAction::Hide);
                None
            }
            PresentationPageAction::ReloadOriginal | PresentationPageAction::ReloadCurrent => {
                let target = if command.action == PresentationPageAction::ReloadCurrent {
                    HtmlReloadTarget::Current
                } else {
                    HtmlReloadTarget::Original
                };
                (!self.library_mut().reload_html_overlay_by_id(local, target)).then(|| errors::NOT_REALIZED.to_owned())
            }
            PresentationPageAction::Back => self.navigate_replica(local, HtmlNavigation::Back).err(),
            PresentationPageAction::Forward => self.navigate_replica(local, HtmlNavigation::Forward).err(),
            PresentationPageAction::Browser => self.navigate_replica(local, HtmlNavigation::Browser).err(),
            PresentationPageAction::Resize => {
                // a pane page is always full-pane
                if pane.is_none()
                    && let Some(store) = self.library_mut().store_for_session_mut(session)
                {
                    store.resize_overlay(session, command.size_percent);
                }
                None
            }
        };
        if let Some(refusal) = refusal {
            tracing::info!(action = ?command.action, %refusal, "an origin request was not applied");
        }
        if let Some(record) = self.remote.viewer.pages.records.get_mut(&local) {
            record.applied_seq = record.applied_seq.max(command.seq);
        }
    }

    /// Hides or shows only the slot holding page `local`, never its neighbours (upstream's `setReplicaPageHidden`).
    fn set_replica_page_hidden(&mut self, local: Uuid, hidden: bool) {
        let Some(slot) = self.library().html_overlay_slot(local) else {
            return;
        };
        let Some(store) = self.library_mut().store_mut(slot.window) else {
            return;
        };
        let Some(found) = store.session_mut(slot.session) else {
            return;
        };
        if found.overlay_hidden(slot.pane) == hidden {
            return;
        }
        found.set_overlay_hidden(hidden, slot.pane);
        store.schedule_tree_changed();
    }

    /// A history step or hand-off on replica `local`: a file page's file is on the origin, so neither the browser nor
    /// the file manager reaches it from here (upstream's `navigate` on a `.replica` source).
    fn navigate_replica(&self, local: Uuid, navigation: HtmlNavigation) -> Result<(), String> {
        if let Some(refusal) = self
            .replica_page_slot(local)
            .and_then(|(_, page)| replica_navigation_refusal(page, navigation))
        {
            return Err(refusal.to_owned());
        }
        let host = self.html_host().ok_or_else(|| errors::NOT_REALIZED.to_owned())?;
        host.navigate(local, navigation)
    }

    /// What page `local` shows now, as its origin reads it back (upstream's `replicaPageState`); `None` once it left
    /// its slot or for a page of this machine's own.
    fn replica_page_state(&self, local: Uuid, applied_seq: i64) -> Option<PresentationPageState> {
        let (slot, page) = self.replica_page_slot(local)?;
        let origin = page.replica_of?;
        let hidden = self
            .library()
            .store(slot.window)?
            .session(slot.session)?
            .overlay_hidden(slot.pane);
        let current = page.current.as_ref();
        let shown = PageShown {
            title: current.and_then(|current| current.title.clone()),
            address: current.map(|current| current.page.clone()),
            load: page.load_state,
            error: page.load_error.clone(),
            can_go_back: current.is_some_and(|current| current.can_go_back),
            can_go_forward: current.is_some_and(|current| current.can_go_forward),
        };
        Some(PresentationPageState::new(
            uuid_string(origin),
            shown,
            hidden,
            applied_seq,
        ))
    }

    /// Page `local` left every slot (upstream's `replicaPageReleased`): its origin hears `page.closed` once and its
    /// channel closes, failing what still waits on it. Runs from the commit's page sweep, whatever emptied the slot.
    pub(crate) fn replica_page_released(&mut self, local: Uuid) {
        let Some(record) = self.remote.viewer.pages.records.remove(&local) else {
            return;
        };
        self.answer_origin(
            record.session,
            Body::PageClosed(PresentationPageChange::page(uuid_string(record.origin))),
        );
        if let Some(mut link) = record.link {
            let events = link.channel.close("page closed");
            let mut settled = Settled::default();
            link.settle(events, &mut settled);
            settled.run();
        }
    }

    /// Brings every replica in line after a commit: requests made for a load the page left are withdrawn, and each
    /// page whose state changed since its origin last heard reports it, once per commit at most (upstream's
    /// `replicaPageChanged` coalescing).
    pub(crate) fn sync_replica_pages(&mut self) {
        let locals: Vec<Uuid> = self.remote.viewer.pages.records.keys().copied().collect();
        for local in locals {
            let revision = self.replica_page_slot(local).map(|(_, page)| page.reload_revision);
            let Some(record) = self.remote.viewer.pages.records.get_mut(&local) else {
                continue;
            };
            let applied_seq = record.applied_seq;
            if let (Some(revision), Some(link)) = (revision, record.link.as_mut())
                && revision > link.revision
            {
                link.revision = revision;
                let events = link.channel.revision_moved(revision);
                self.settle_channel(local, events);
            }
            let Some(state) = self.replica_page_state(local, applied_seq) else {
                continue;
            };
            let Some(record) = self.remote.viewer.pages.records.get_mut(&local) else {
                continue;
            };
            if record.reported.as_ref() == Some(&state) {
                continue;
            }
            record.reported = Some(state.clone());
            let session = record.session;
            self.answer_origin(session, Body::PageState(state));
        }
    }

    /// Closes every replica's channel and stops its helper (the quit path); what still waits on one fails.
    pub(crate) fn shutdown_replica_channels(&mut self) {
        let locals: Vec<Uuid> = self.remote.viewer.pages.records.keys().copied().collect();
        for local in locals {
            let Some(mut link) = self
                .remote
                .viewer
                .pages
                .records
                .get_mut(&local)
                .and_then(|record| record.link.take())
            else {
                continue;
            };
            let events = link.channel.close("the app is quitting");
            let mut settled = Settled::default();
            link.settle(events, &mut settled);
            settled.run();
        }
    }

    /// Whether page `id` is a file page shown here for another machine, whose files and bridge requests go to that
    /// machine over its channel: the tests' read of the link.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn linked_replica(&self, id: Uuid) -> bool {
        self.remote
            .viewer
            .pages
            .records
            .get(&id)
            .is_some_and(|record| record.link.is_some())
    }

    // MARK: - the frontend's side

    /// Asks the origin of replica `id` for its file at `path` (its scheme URL's path, which is the origin path);
    /// `answer` runs once with the file or the failure, on the main loop.
    pub fn replica_fetch(&mut self, id: Uuid, path: &str, answer: FetchAnswer) {
        let revision = self.replica_page_slot(id).map_or(0, |(_, page)| page.reload_revision);
        let Some(link) = self.page_link(id) else {
            answer(Err(errors::REMOTE_FILE.to_owned()));
            return;
        };
        match link.channel.fetch(path, revision) {
            Ok((request, events)) => {
                link.fetches.insert(
                    request,
                    Fetch {
                        mime_type: None,
                        body: Vec::new(),
                        answer,
                    },
                );
                self.settle_channel(id, events);
            }
            Err(failure) => answer(Err(failure.to_string())),
        }
    }

    /// Relays the request replica `id`'s page sent, as the raw JSON it sent, to its origin, which fills the defaults
    /// and acts there (upstream's linked `handleBridgeRequest`); `answer` runs once with the shaped reply.
    pub fn replica_bridge(&mut self, id: Uuid, body: &str, answer: BridgeAnswer) {
        let Some(link) = self.page_link(id) else {
            answer(Err("page closed".to_owned()));
            return;
        };
        match link.channel.bridge(body) {
            Ok((request, events)) => {
                link.bridges.insert(request, answer);
                self.settle_channel(id, events);
            }
            Err(failure) => answer(Err(failure.to_string())),
        }
    }

    /// One line the helper `key` read, holding the permit its reader took; commit afterwards.
    pub fn page_helper_line(&mut self, key: HelperKey, line: &[u8]) {
        let Some(helper) = self.page_link(key.page).filter(|helper| helper.key == key) else {
            return;
        };
        let events = helper.channel.receive(line);
        self.settle_channel(key.page, events);
    }

    /// The helper `key` ended with `reason`: the channel closes, and the page with it; commit afterwards.
    pub fn page_helper_closed(&mut self, key: HelperKey, reason: &str) {
        let Some(link) = self.page_link(key.page).filter(|link| link.key == key) else {
            return;
        };
        let events = link.channel.close(reason);
        self.settle_channel(key.page, events);
    }

    fn page_link(&mut self, id: Uuid) -> Option<&mut PageLink> {
        self.remote
            .viewer
            .pages
            .records
            .get_mut(&id)
            .and_then(|record| record.link.as_mut())
    }

    /// Answers what channel `id`'s events settled; a channel that ended closes its page, whose release tells the origin.
    fn settle_channel(&mut self, id: Uuid, events: Vec<ChannelEvent>) {
        if events.is_empty() {
            return;
        }
        let mut settled = Settled::default();
        if let Some(link) = self.page_link(id) {
            link.settle(events, &mut settled);
        }
        let closed = settled.closed;
        settled.run();
        if closed {
            self.library_mut().close_html_overlay(id);
        }
    }
}

/// Why `navigation` cannot run on `page`, `None` when it can: a file page shown for another machine has no file here
/// to open in the browser or reveal (upstream's `OverlayHtmlError.remoteFile`).
#[must_use]
pub fn replica_navigation_refusal(page: &HtmlOverlay, navigation: HtmlNavigation) -> Option<&'static str> {
    let file = matches!(page.source, HtmlSource::Replica { .. });
    (file && matches!(navigation, HtmlNavigation::Browser | HtmlNavigation::Finder)).then_some(errors::REMOTE_FILE)
}
