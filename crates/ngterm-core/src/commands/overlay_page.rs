//! `session.overlay.*` against an HTML page (upstream's page arms of `ControlServer+SessionActions.swift`:
//! `openHtmlOverlay`, `submitSessionOverlay`, `reloadSessionOverlay`, `navigateSessionOverlay`, `htmlPageResult`), and
//! the core's side of the frontend's web views: what they report, the page bridge's entry, and the pages released
//! from every slot (upstream's `HtmlOverlayReleases`).
//!
//! A page's outcome lives here, keyed by its id ([`HtmlPageOutcomes`]): an open registers it (a page a snapshot
//! restore brought back is registered by the next sync), a submit records the value before the close, and a page
//! found gone from every slot of every open window (its session's undoable close included) is dismissed and handed to
//! the frontend to release its web view, whatever path emptied the slot.
//!
//! A page the placement rule hands to the presenting machine (`crate::remote`'s page half) is stored, rendered and
//! followed there: its open, submit, reload and history commands go to that machine, and its outcome is settled when
//! it ends there or here. On that machine the page is a replica (`crate::remote::replica_page`): a socket submit is
//! refused there, its release tells the origin, and its file is never handed to the browser or the file manager.

use std::collections::{HashMap, HashSet};
use std::fmt;

use ngterm_proto::{Args, ControlResult, Request};
use url::Url;
use uuid::Uuid;

use super::{Outcome, session_store, with_id};
use crate::Core;
use crate::app_actions::{ActionEffect, FrontendAction};
use crate::error::ControlError;
use crate::html::bridge::{self, HtmlBridgePage};
use crate::html::text_page::TextLog;
use crate::html::{
    HtmlHost, HtmlLoadState, HtmlNavigation, HtmlOverlay, HtmlPageFlags, HtmlPageInfo, HtmlPageOutcomes,
    HtmlReloadTarget, HtmlSource, errors, zoom,
};
use crate::model::{HtmlOverlayOpenFailure, HtmlPageSlot, OverlayPane, OverlayPointSize, OverlaySize, StatusPane};
use crate::pane_env::uuid_string;
use crate::persist::PageSnapshot;
use crate::remote::{REMOTE_LOOPBACK, REMOTE_TOO_LARGE, RemotePageCommand, RemotePageOpen};

/// A page that passed validation in a build with no web views (the `html` feature off).
pub(crate) const PAGES_UNAVAILABLE: &str = "session.overlay.open: html overlays are not available in this build";
/// A page that passed validation where the build's web engine cannot run (the host's
/// [`HtmlHost::web_views_unavailable`]), before the host's reason.
pub(crate) const PAGES_UNAVAILABLE_HERE: &str = "session.overlay.open: html overlays are not available here";
/// Upstream's `OverlayHtmlError.noResult`/`noRead`: a page runs no program, so there is no status and no text.
pub(crate) const PAGE_NO_RESULT: &str = "no overlay result: the slot holds an html page";
pub(crate) const PAGE_NO_READ: &str = "no overlay to read: the slot holds an html page";
const UNKNOWN_PAGE: &str = "no such page";
const ALREADY_OPEN: &str = "overlay already open";
const PANE_ALREADY_OPEN: &str = "pane overlay already open";
/// Upstream's `OverlayHtmlError.presenter`.
const PRESENTER: &str = "a viewer presents this session: an html overlay would open where nobody sees it";
/// Upstream's `OverlayHtmlError.logRemote`.
const LOG_REMOTE: &str = "session.overlay.open: a log page is not shown on another Mac";
/// The bridge's refusals of a page that left its slot (upstream's `handleBridgeRequest`).
const PAGE_CLOSED: &str = "page closed";

/// The pages' outcomes, the pages the frontend may still hold a web view for, and the frontend's web views.
#[derive(Default)]
pub(crate) struct PageRuntime {
    outcomes: HtmlPageOutcomes,
    /// Every page opened and not yet released.
    known: HashSet<Uuid>,
    /// The pages handed to the session's viewer (shown on a presenter or parked) whose outcome is registered: no web
    /// view of this machine's holds one, so they are never released, only forgotten once they leave their slot.
    viewer: HashSet<Uuid>,
    /// The pages a window's snapshot writes ([`HtmlOverlay::persisted`]), each by the window holding it and what the
    /// snapshot wrote for it at the last sync, so a page that comes, goes, moves or changes asks its windows for a
    /// save.
    persisted: HashMap<Uuid, (Uuid, PageSnapshot)>,
    /// Pages that left every slot since the frontend last took them.
    released: Vec<Uuid>,
    host: Option<Box<dyn HtmlHost>>,
    /// A headless origin has no local page surface, even when its terminal holds the lead.
    presenter_only: bool,
}

impl fmt::Debug for PageRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PageRuntime")
            .field("outcomes", &self.outcomes)
            .field("known", &self.known)
            .field("viewer", &self.viewer)
            .field("persisted", &self.persisted)
            .field("released", &self.released)
            .field("host", &self.host.is_some())
            .field("presenter_only", &self.presenter_only)
            .finish()
    }
}

/// How a page opens: what `session.overlay.open` validated, or a clicked link's browsing page.
pub(crate) struct PageOpening {
    pub source: HtmlSource,
    pub flags: HtmlPageFlags,
    pub pane: Option<OverlayPane>,
    pub size_percent: Option<i64>,
    /// The point axes overriding `size_percent`, which the dispatcher refuses without one.
    pub points: OverlayPointSize,
    pub background_color: Option<String>,
    pub follow: bool,
    /// A terminal click opens the page ([`HtmlOverlay::clicked`]); the dispatcher never sets it.
    pub clicked: bool,
    /// Makes a text page a log page ([`HtmlOverlay::text_log`]).
    pub text_log: Option<TextLog>,
}

impl PageOpening {
    /// The page `args` describe in the slot `pane`.
    pub(crate) fn from_args(
        source: HtmlSource,
        args: &Args,
        pane: Option<OverlayPane>,
        points: OverlayPointSize,
    ) -> Self {
        let set = |flag: Option<bool>| flag == Some(true);
        // only a text page takes a log's options; the dispatcher refused them on any other source
        let text_log = if matches!(source, HtmlSource::Text { .. }) {
            super::overlay::text_log(args)
        } else {
            None
        };
        Self {
            source,
            flags: HtmlPageFlags {
                navigation: set(args.navigation),
                javascript: set(args.javascript),
                chromeless: set(args.chromeless),
                persistent: set(args.persistent),
                browse: set(args.browse),
            },
            pane,
            size_percent: args.size_percent,
            points,
            background_color: args.color.clone(),
            follow: set(args.follow),
            clicked: false,
            text_log,
        }
    }
}

/// `session.overlay.open --html|--url|--image|--markdown` once validated (upstream's `openHtmlOverlay`): a local
/// document's descriptor is probed and an image's natural size resolved before the page takes its slot. It follows
/// a program's slot rules and answers with its id, the one `session.overlay.result --page` reads. A page the
/// placement rule hands to the presenting machine is reserved for it before any local check: nothing renders here,
/// so neither the web views, the saved store nor `--follow` are this machine's concern.
pub(crate) fn open(core: &mut Core, target: Option<&str>, window: Option<&str>, opening: PageOpening) -> Outcome {
    open_checked(core, target, window, opening, None, false)
}

fn open_checked(
    core: &mut Core,
    target: Option<&str>,
    window: Option<&str>,
    mut opening: PageOpening,
    image: Option<crate::html::image_page::ImageData>,
    checked: bool,
) -> Outcome {
    let resolved = crate::resolve::Resolver::new(core.library()).session(target, window)?;
    super::overlay::refuse_hidden_slot(core, resolved.window, resolved.session, opening.pane)?;
    let already_open = if opening.pane.is_none() {
        ALREADY_OPEN
    } else {
        PANE_ALREADY_OPEN
    };
    let page = HtmlOverlay {
        clicked: opening.clicked,
        ..HtmlOverlay::new(opening.source.clone(), opening.flags)
    }
    .with_text_log(opening.text_log);
    let page_id = page.id;
    let refusal = match crate::remote::open_remote_page(core, resolved.window, resolved.session, &page, &opening) {
        RemotePageOpen::Local => None,
        RemotePageOpen::Opened => {
            return Ok(ControlResult {
                page_id: Some(uuid_string(page_id)),
                ..with_id(resolved.session)
            });
        }
        RemotePageOpen::SlotTaken => Some(already_open),
        RemotePageOpen::PaneMissing => Some(super::hud::PANE_NOT_VISIBLE),
        RemotePageOpen::Unsupported => Some(PRESENTER),
        RemotePageOpen::Loopback => Some(REMOTE_LOOPBACK),
        RemotePageOpen::TooLarge => Some(REMOTE_TOO_LARGE),
    };
    if let Some(refusal) = refusal {
        return Err(ControlError::rejected(refusal));
    }
    let (store, id) = session_store(core, target, window)?;
    if store
        .session(id)
        .is_some_and(|session| session.remote_overlays().slot(opening.pane).is_some())
    {
        return Err(ControlError::rejected(already_open));
    }
    let Some(host) = core.pages.host.as_ref() else {
        return Err(ControlError::rejected(PAGES_UNAVAILABLE));
    };
    if let Some(reason) = host.web_views_unavailable() {
        return Err(ControlError::rejected(format!("{PAGES_UNAVAILABLE_HERE}: {reason}")));
    }
    if opening.flags.persistent
        && let Some(failure) = host.persistent_store_failure()
    {
        return Err(ControlError::rejected(format!("session.overlay.open: {failure}")));
    }
    prepare_document(core, host.as_ref(), resolved, &mut opening, image, checked)?;
    // a presenter would get the text page without its log state, so it is refused rather than changed (upstream's
    // `textLogRefusal`, after the probe); a presentable text page must meet this before its hand-off
    if opening.text_log.is_some() && core.page_belongs_remote(id, opening.pane) {
        return Err(ControlError::rejected(LOG_REMOTE));
    }
    // the store's own guard (upstream's `openHtmlOverlay`): a page placed on the presenting machine never renders here
    if core.page_belongs_remote(id, opening.pane) {
        return Err(ControlError::rejected(PRESENTER));
    }
    let (store, _) = session_store(core, target, window)?;
    store
        .open_html_overlay_sized(
            id,
            opening.pane,
            page,
            OverlaySize {
                percent: opening.size_percent,
                points: opening.points,
            },
            opening.background_color,
        )
        .map_err(|failure| ControlError::rejected(open_failure(failure, opening.pane)))?;
    if opening.follow {
        store.select_session(Some(id));
    }
    core.pages.outcomes.register(page_id);
    core.pages.known.insert(page_id);
    Ok(ControlResult {
        page_id: Some(uuid_string(page_id)),
        ..with_id(id)
    })
}

/// Checks the local descriptor and resolves a natural image size before the slot is taken.
fn prepare_document(
    core: &Core,
    host: &dyn HtmlHost,
    target: crate::resolve::SessionTarget,
    opening: &mut PageOpening,
    image: Option<crate::html::image_page::ImageData>,
    checked: bool,
) -> Result<(), ControlError> {
    match &opening.source {
        HtmlSource::Image { path } => {
            let probe = image
                .map_or_else(|| host.probe_image(path), Ok)
                .map_err(|error| ControlError::rejected(format!("session.overlay.open: {error}")))?;
            if opening.pane.is_none() && opening.size_percent.is_none() && opening.points.is_empty() {
                let bounds = core
                    .library()
                    .store(target.window)
                    .and_then(|store| store.session(target.session))
                    .and_then(|session| session.hud_frames.detail);
                opening.points = crate::html::image_page::points(
                    probe.natural(host.display_scale(target.window)),
                    host.page_zoom(),
                    bounds,
                );
                opening.size_percent = Some(crate::html::image_page::BASE_PERCENT);
            }
        }
        HtmlSource::Markdown { path } if !checked => host
            .check_markdown(path)
            .map_err(|error| ControlError::rejected(format!("session.overlay.open: {error}")))?,
        HtmlSource::Text { path, .. } if !checked => match opening.text_log {
            // a log reads only its end, so its size is never refused
            Some(log) => host.check_log(path, log.tail),
            None => host.check_text(path),
        }
        .map_err(|error| ControlError::rejected(format!("session.overlay.open: {error}")))?,
        _ => {}
    }
    Ok(())
}

/// The error of a refused page open (upstream's `HtmlOverlayOpenFailure.message(pane:)`).
pub(crate) fn open_failure(failure: HtmlOverlayOpenFailure, pane: Option<OverlayPane>) -> &'static str {
    match failure {
        HtmlOverlayOpenFailure::UnknownSession => "no such session",
        HtmlOverlayOpenFailure::AlreadyOpen if pane.is_none() => ALREADY_OPEN,
        HtmlOverlayOpenFailure::AlreadyOpen => PANE_ALREADY_OPEN,
        HtmlOverlayOpenFailure::PaneNotVisible => super::hud::PANE_NOT_VISIBLE,
    }
}

/// `session.overlay.submit` (upstream's `submitHtmlOverlay`): the page in the addressed slot is closed and answered
/// with `value`. The close only empties the slot and records nothing; the page is settled as dismissed later, at the
/// next commit's `Core::sync_pages` (or a `result --page` read), which leaves a page that already has an answer alone,
/// so the answer recorded here stands.
pub(crate) fn submit(
    core: &mut Core,
    target: Option<&str>,
    window: Option<&str>,
    pane: Option<OverlayPane>,
    value: &str,
) -> Outcome {
    let (store, id) = session_store(core, target, window)?;
    // a page the presenting machine shows is answered here, then closed there (upstream's `submitHtmlOverlay`)
    let remote = store
        .session(id)
        .and_then(|session| session.remote_overlays().slot(pane))
        .and_then(|slot| slot.page.as_ref())
        .map(|page| page.overlay.id);
    if let Some(page_id) = remote {
        core.pages.outcomes.submit(page_id, value);
        if let Some(store) = core.library_mut().store_for_session_mut(id) {
            store.close_remote_overlay(id, pane);
        }
        core.settle_remote_overlays();
        return Ok(with_id(id));
    }
    let shown = store
        .html_overlay(id, pane)
        .map_err(|failure| ControlError::rejected(failure.message()))?;
    // a page shown here for another machine is answered there, through the page itself
    if shown.replica_of.is_some() {
        return Err(ControlError::rejected(errors::REPLICA_SUBMIT));
    }
    let page_id = shown.id;
    match pane {
        Some(pane) => store.close_pane_overlay(id, pane),
        None => store.close_overlay(id),
    };
    core.pages.outcomes.submit(page_id, value);
    Ok(with_id(id))
}

/// `session.overlay.reload` (upstream's `reloadSessionOverlay`): the original source, or with `--current` the page
/// the user navigated to; the frontend reloads at its next render, the web view kept.
pub(crate) fn reload(
    core: &mut Core,
    target: Option<&str>,
    window: Option<&str>,
    pane: Option<OverlayPane>,
    current: bool,
) -> Outcome {
    let id = session_store(core, target, window)?.1;
    if let Some(answer) = remote_answer(core.reload_remote_page(id, pane, current), id) {
        return answer;
    }
    let (store, id) = session_store(core, target, window)?;
    let reload = if current {
        HtmlReloadTarget::Current
    } else {
        HtmlReloadTarget::Original
    };
    store
        .reload_html_overlay(id, pane, reload)
        .map_err(|failure| ControlError::rejected(failure.message()))?;
    Ok(with_id(id))
}

/// `session.overlay.navigate` (upstream's `navigateSessionOverlay`): a history step or hand-off on the page's web
/// view, refused while it has none.
pub(crate) fn navigate(
    core: &mut Core,
    target: Option<&str>,
    window: Option<&str>,
    pane: Option<OverlayPane>,
    navigation: HtmlNavigation,
) -> Outcome {
    let id = session_store(core, target, window)?.1;
    if let Some(answer) = remote_answer(core.navigate_remote_page(id, pane, navigation), id) {
        return answer;
    }
    let (store, id) = session_store(core, target, window)?;
    let shown = store
        .html_overlay(id, pane)
        .map_err(|failure| ControlError::rejected(failure.message()))?;
    if let Some(refusal) = crate::remote::replica_page::replica_navigation_refusal(shown, navigation) {
        return Err(ControlError::rejected(refusal));
    }
    if navigation == HtmlNavigation::Browser {
        match shown.source {
            HtmlSource::Markdown { .. } => return Err(ControlError::rejected(errors::MARKDOWN_IN_OVERLAY)),
            HtmlSource::Text { .. } => return Err(ControlError::rejected(errors::TEXT_IN_OVERLAY)),
            HtmlSource::Document { .. } => return Err(ControlError::rejected(errors::DOCUMENT_IN_OVERLAY)),
            _ => {}
        }
    }
    let page_id = shown.id;
    let host = core
        .pages
        .host
        .as_ref()
        .ok_or_else(|| ControlError::rejected(errors::NOT_REALIZED))?;
    host.navigate(page_id, navigation).map_err(ControlError::rejected)?;
    Ok(with_id(id))
}

/// The answer to a page command on a page the presenting machine shows; `None` sends it on to this machine's own slot
/// (upstream's `remotePageResponse`).
fn remote_answer(command: RemotePageCommand, session: Uuid) -> Option<Outcome> {
    match command {
        RemotePageCommand::Local => None,
        RemotePageCommand::Sent => Some(Ok(with_id(session))),
        RemotePageCommand::Refused(error) => Some(Err(ControlError::rejected(error))),
    }
}

/// `session.overlay.result --page` (upstream's `htmlPageResult`): where the page stands, by its id alone.
pub(crate) fn page_result(core: &mut Core, page: Uuid) -> Outcome {
    core.sync_pages();
    let outcome = core
        .pages
        .outcomes
        .outcome(page)
        .ok_or_else(|| ControlError::rejected(UNKNOWN_PAGE))?;
    Ok(ControlResult {
        page_outcome: Some(outcome),
        ..ControlResult::default()
    })
}

// MARK: - the frontend's hooks

impl Core {
    /// Routes pages to the session's presenter regardless of terminal leadership. A headless origin's newly
    /// created split leads locally until a viewer attaches it, but its page still belongs on that viewer.
    pub fn set_page_presenter_only(&mut self) {
        self.pages.presenter_only = true;
    }

    pub(crate) fn pages_presenter_only(&self) -> bool {
        self.pages.presenter_only
    }

    /// Installs the frontend's web views; without them a page that passed validation is refused.
    pub fn set_html_host(&mut self, host: Box<dyn HtmlHost>) {
        self.pages.host = Some(host);
    }

    /// The frontend's web views, if it installed them.
    pub(crate) fn html_host(&self) -> Option<&dyn HtmlHost> {
        self.pages.host.as_deref()
    }

    /// Settles the pages that left every slot: each is dismissed (a no-op for one already answered) and queued for
    /// the frontend to release its web view. Then follows the pages a snapshot writes ([`Core::sync_persisted_pages`]).
    /// Runs at every commit and before an outcome is read. A presenter-only origin first parks what a restore opened
    /// in its own slots, as it renders no page itself.
    pub(crate) fn sync_pages(&mut self) {
        if self.pages.presenter_only {
            for page in self.library_mut().park_local_pages() {
                self.pages.known.remove(&page);
            }
        }
        let live: HashSet<Uuid> = self.library().live_html_overlays().into_iter().collect();
        let gone: Vec<Uuid> = self.pages.known.difference(&live).copied().collect();
        for page in gone {
            self.pages.known.remove(&page);
            self.pages.outcomes.dismiss(page);
            // only a frontend with web views takes the released pages; a headless origin's would pile up
            if self.pages.host.is_some() {
                self.pages.released.push(page);
            }
            self.replica_page_released(page);
            self.end_companion_page_serving(page);
        }
        let persisted = self.library().live_persisted_pages();
        self.sync_persisted_pages(persisted);
    }

    /// Follows the pages a window's snapshot writes, `pages` being each as (window, page, what the snapshot writes for
    /// it). One in a slot that no open of this run put there is one a restore brought back (with a fresh id, so never
    /// a page answered before): its outcome starts pending, as an open's does; a page the session's viewer holds is
    /// followed apart from the local ones, as no web view here holds it. A persisted page that came, went, moved or
    /// changed what its snapshot writes (its slot, visibility, size, background, cover hold or parking) asks the windows
    /// it touched for a debounced save, so a crash neither loses an open page nor revives a closed one or its old state.
    fn sync_persisted_pages(&mut self, pages: Vec<(Uuid, Uuid, PageSnapshot)>) {
        let mut before = std::mem::take(&mut self.pages.persisted);
        let mut dirty: HashSet<Uuid> = HashSet::new();
        for (window, page, snapshot) in pages {
            let restored = if snapshot.viewer {
                self.pages.viewer.insert(page)
            } else {
                self.pages.known.insert(page)
            };
            if restored {
                self.pages.outcomes.register(page);
            } else {
                match before.remove(&page) {
                    Some((previous, written)) if previous == window && written == snapshot => {}
                    // a page that changed in its window touches that window alone, the set keeping it once
                    Some((previous, _)) => {
                        dirty.extend([previous, window]);
                    }
                    None => {
                        dirty.insert(window);
                    }
                }
            }
            self.pages.persisted.insert(page, (window, snapshot));
        }
        let persisted = &self.pages.persisted;
        self.pages.viewer.retain(|page| persisted.contains_key(page));
        dirty.extend(before.into_values().map(|(window, _)| window));
        for window in dirty {
            if let Some(store) = self.library_mut().store_mut(window) {
                store.schedule_save();
            }
        }
    }

    /// Starts following page `page`, which a web view of this machine's shows but whose outcome is another machine's
    /// (a page shown here for its origin): its release reaches the frontend, and an outcome is never registered.
    pub(crate) fn track_page(&mut self, page: Uuid) {
        self.pages.known.insert(page);
    }

    /// Starts tracking a page handed to the presenting machine as pending; no web view of this machine's holds it.
    pub(crate) fn register_page_outcome(&mut self, page: Uuid) {
        self.pages.outcomes.register(page);
    }

    /// [`Core::register_page_outcome`] for a page held in a session's slot for its viewer, which its snapshot writes:
    /// the page sync then knows it was opened in this run, not restored.
    pub(crate) fn register_viewer_page_outcome(&mut self, page: Uuid) {
        self.pages.viewer.insert(page);
        self.register_page_outcome(page);
    }

    /// Whether page `page` is still open and unanswered.
    pub(crate) fn page_outcome_pending(&self, page: Uuid) -> bool {
        self.pages.outcomes.is_pending(page)
    }

    /// Where page `page`'s outcome stands: the tests' read.
    #[cfg(test)]
    pub(crate) fn page_outcome(&self, page: Uuid) -> Option<ngterm_proto::ControlHtmlPageOutcome> {
        self.pages.outcomes.outcome(page)
    }

    /// Answers pending page `page` the way another machine answered the page it shows.
    pub(crate) fn settle_page_outcome(&mut self, page: Uuid, like: Option<&ngterm_proto::ControlHtmlPageOutcome>) {
        self.pages.outcomes.settle(page, like);
    }

    /// Records page `page` closed unanswered; a no-op for one already answered.
    pub(crate) fn dismiss_page_outcome(&mut self, page: Uuid) {
        self.pages.outcomes.dismiss(page);
    }

    /// Takes the pages released since the last take: their web views go.
    pub fn take_released_pages(&mut self) -> Vec<Uuid> {
        std::mem::take(&mut self.pages.released)
    }

    /// Where page `id` sits in a visible session of an open window.
    #[must_use]
    pub fn html_overlay_slot(&self, id: Uuid) -> Option<HtmlPageSlot> {
        self.library().html_overlay_slot(id)
    }

    /// The page zoom every page shows at (upstream's `HtmlOverlayRegistry.zoom`, from `htmlOverlayZoom`).
    #[must_use]
    pub fn html_zoom(&self) -> f64 {
        zoom::resolve(self.settings().html_overlay_zoom)
    }

    /// The web view of page `id` reported its load state; whether the page still sits in a slot. Commit afterwards.
    pub fn set_html_load_state(&mut self, id: Uuid, state: HtmlLoadState, error: Option<String>) -> bool {
        self.library_mut().set_html_load_state(id, state, error)
    }

    /// The web view of page `id` reported what it shows; whether the page still sits in a slot.
    pub fn set_html_page(&mut self, id: Uuid, info: Option<HtmlPageInfo>) -> bool {
        self.library_mut().set_html_page(id, info)
    }

    /// The toolbar's reload of page `id`, wherever it sits.
    pub fn reload_html_overlay(&mut self, id: Uuid, target: HtmlReloadTarget) -> bool {
        self.library_mut().reload_html_overlay_by_id(id, target)
    }

    /// Closes page `id` wherever it sits (the strip's close button, the page's own `window.close()`); false while its
    /// session sits in an undoable close.
    pub fn close_html_overlay(&mut self, id: Uuid) -> bool {
        self.library_mut().close_html_overlay(id)
    }

    /// The strip's Close of page `id` (upstream's `onClose`): a page companion's page is its row, so it takes the session
    /// close as Close Session does, confirmed first when the setting asks (the frontend's half) and with its undo
    /// grace; any other page leaves its slot as [`Core::close_html_overlay`] has it. `None` when nothing changed.
    pub fn close_html_overlay_from_strip(&mut self, id: Uuid) -> Option<ActionEffect> {
        let row = self.html_overlay_slot(id).filter(|slot| {
            slot.pane.is_none()
                && self
                    .library()
                    .store(slot.window)
                    .and_then(|store| store.session(slot.session))
                    .is_some_and(|session| session.content() == crate::model::SessionContent::Page)
        });
        let closed = match row {
            Some(HtmlPageSlot { window, session, .. }) if self.settings().confirm_close_session == Some(true) => {
                return Some(ActionEffect::Frontend(FrontendAction::ConfirmCloseSession {
                    window,
                    session,
                }));
            }
            Some(slot) => self.sidebar_action(
                slot.window,
                crate::sidebar_actions::SidebarAction::CloseSessions(vec![slot.session]),
            ),
            None => self.close_html_overlay(id),
        };
        closed.then_some(ActionEffect::Done)
    }

    /// Hides page `id` from its own strip with the page's scope: its pane's overlay, or every overlay of its session for
    /// a session-wide page (upstream's `hideHtmlOverlay`); false once it left its slot or while its session sits in an
    /// undoable close.
    pub fn hide_html_overlay(&mut self, id: Uuid) -> bool {
        self.library_mut().hide_html_overlay(id)
    }

    /// The user focused pane page `id`: the split's focus follows the pane it covers (upstream's `onFocus`).
    pub fn html_page_focused(&mut self, id: Uuid) -> bool {
        let Some(HtmlPageSlot {
            window,
            session,
            pane: Some(pane),
        }) = self.html_overlay_slot(id)
        else {
            return false;
        };
        self.library_mut()
            .store_mut(window)
            .is_some_and(|store| store.set_split_focus(session, pane == OverlayPane::Right))
    }

    /// The user clicked session-wide page `id` while a pane ask is up beside it: the split's focus moves to the pane
    /// the ask leaves uncovered, or the next refocus hands the keys back to the ask (upstream's `onClick`).
    pub fn html_page_clicked(&mut self, id: Uuid) -> bool {
        let Some(HtmlPageSlot {
            window,
            session,
            pane: None,
        }) = self.html_overlay_slot(id)
        else {
            return false;
        };
        let Some(store) = self.library_mut().store_mut(window) else {
            return false;
        };
        let Some(ask) = store.session(session).and_then(crate::model::Session::ask_target_pane) else {
            return false;
        };
        store.set_split_focus(session, ask == OverlayPane::Left)
    }

    /// The request page `id`'s message makes (upstream's `handleBridgeRequest`): resolved where the page sits now, so
    /// a request after a swap or a move acts from its new place, and a page that left its slot is refused.
    ///
    /// # Errors
    ///
    /// The message the page's promise rejects with.
    pub fn html_bridge_request(&self, id: Uuid, body: &[u8]) -> Result<Request, String> {
        let slot = self.html_overlay_slot(id).ok_or_else(|| PAGE_CLOSED.to_owned())?;
        let page = HtmlBridgePage {
            window: Some(uuid_string(slot.window)),
            session: slot.session,
            pane: slot.pane,
        };
        bridge::request(body, &page).map_err(|error| error.message)
    }

    /// The clicked-link eligibility check: a live session with neither a HUD nor a zoomed window.
    #[must_use]
    pub fn can_show_link_overlay(&self, session: Uuid) -> bool {
        let Some(found) = self
            .library()
            .store_for_session(session)
            .and_then(|store| store.session(session))
        else {
            return false;
        };
        if found.overlay.hud_active() {
            return false;
        }
        if self
            .library()
            .window_id_for_session(session)
            .is_some_and(|window| self.zoom_target(window).is_some())
        {
            return false;
        }
        true
    }

    /// Closes the session-wide page a terminal click opened while it is hidden by hand (upstream's
    /// `releaseHiddenClickedPage`), so the next click page can take the slot; the page is dismissed. Any other occupant,
    /// a shown page, one only out of sight under a newer pane overlay, and a slot the placement rule hands to the
    /// presenting machine are left alone, and the answer is false. A click path calls it after every check that could
    /// still refuse its own open, so a refused click never costs the old page.
    pub fn release_hidden_clicked_page(&mut self, session: Uuid) -> bool {
        let Some(page) = self
            .library()
            .store_for_session(session)
            .and_then(|store| store.session(session))
            .filter(|found| found.overlay_hidden(None))
            .and_then(|found| found.overlay.page())
            .filter(|page| page.clicked)
            .map(|page| page.id)
        else {
            return false;
        };
        if self.page_belongs_remote(session, None) {
            return false;
        }
        let closed = self
            .library_mut()
            .store_for_session_mut(session)
            .is_some_and(|store| store.close_overlay(session));
        if closed {
            self.pages.outcomes.dismiss(page);
        }
        closed
    }

    /// Releases a hidden clicked page for the click page about to open in `session` (upstream's
    /// `ControlServer.releaseHiddenClickedPage`), only once nothing this machine checks could still refuse that open:
    /// web views that can run, for a `persistent` page a usable saved store, and for a picture a probe that read it,
    /// which `image` then carries on to the open.
    fn release_for_click(
        &mut self,
        session: Uuid,
        persistent: bool,
        source: &HtmlSource,
        image: &mut Option<crate::html::image_page::ImageData>,
    ) {
        let Some(host) = self.pages.host.as_ref() else {
            return;
        };
        if host.web_views_unavailable().is_some() || (persistent && host.persistent_store_failure().is_some()) {
            return;
        }
        if let (HtmlSource::Image { path }, None) = (source, image.as_ref()) {
            let Ok(probe) = host.probe_image(path) else {
                return;
            };
            *image = Some(probe);
        }
        self.release_hidden_clicked_page(session);
    }

    /// Opens a local document a terminal click named after an off-main probe; slot eligibility is checked again here.
    /// The page is marked clicked, and replaces a hidden page an earlier click opened once nothing else can refuse it.
    pub fn open_document_link(
        &mut self,
        source: HtmlSource,
        session: Uuid,
        size_percent: Option<i64>,
        mut image: Option<crate::html::image_page::ImageData>,
    ) -> bool {
        if !self.can_show_link_overlay(session) {
            return false;
        }
        self.release_for_click(session, false, &source, &mut image);
        let opening = PageOpening {
            source,
            flags: HtmlPageFlags::default(),
            pane: None,
            size_percent,
            points: OverlayPointSize::default(),
            background_color: None,
            follow: false,
            clicked: true,
            text_log: None,
        };
        open_checked(self, Some(&uuid_string(session)), None, opening, image, true).is_ok()
    }

    /// Opens a clicked web link as a browsing overlay without selecting its session; false uses the browser. The page
    /// is marked clicked, and replaces a hidden page an earlier click opened once the saved store is known usable.
    pub fn open_link_overlay(&mut self, url: Url, session: Uuid, size_percent: Option<i64>) -> bool {
        if !self.can_show_link_overlay(session) {
            return false;
        }
        let source = HtmlSource::Url(url);
        self.release_for_click(session, true, &source, &mut None);
        let opening = PageOpening {
            source,
            flags: HtmlPageFlags {
                navigation: true,
                javascript: true,
                chromeless: false,
                persistent: true,
                browse: true,
            },
            pane: None,
            size_percent,
            points: OverlayPointSize::default(),
            background_color: None,
            follow: false,
            clicked: true,
            text_log: None,
        };
        open(self, Some(&uuid_string(session)), None, opening).is_ok()
    }

    /// Whether a page covers the terminal a font command addresses in session `session` (upstream's
    /// `htmlHidesTerminal`): the command then walks the page zoom instead.
    #[must_use]
    pub fn html_hides_terminal(&self, session: Uuid, pane: Option<StatusPane>) -> bool {
        self.library()
            .store_for_session(session)
            .and_then(|store| store.session(session))
            .is_some_and(|found| found.html_hides_terminal(pane))
    }
}
