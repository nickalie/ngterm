//! Pages handed to the viewer presenting a session, on the origin (upstream's `AppStore+RemotePage.swift` and the page
//! arms of `ControlServer+SessionActions.swift`).
//!
//! A page opened on a session another machine presents goes to that machine under the program overlay placement rule
//! (always on a headless origin, which has no local page surface):
//! the slot is reserved here (a [`RemotePage`] in its [`RemoteOverlaySlot`]), the page's outcome is registered pending
//! and a `page.request` carries a single-use claim token. Nothing renders here. The presenter's `page.state` reports
//! are mirrored into the read-back; hide, show, reload, history and resize are sequence-numbered `page.command`s; a
//! close here, a rejection or close there and the session's removal end the page, keeping an answer already submitted.
//!
//! An image, Markdown, text or clicked HTML page (#891, #917) goes only to a presenter whose hello lists the kind
//! ([`super::hub::PAGE_CONTENT_KIND`], [`super::hub::PAGE_DOCUMENT_KIND`]), read here first as an open reads it, a
//! picture with its probed pixels, which that presenter fits to its own screen (`presenter_sized`).
//!
//! The presenter's loss, its page channel's helper going, an unclaimed channel page's claim window passing and the
//! presenter's `page.detached` (its own helper for the page ended) park the page instead (an extension beyond upstream,
//! where they end it, the last one being a `page.closed` there): the slot stays reserved, the outcome pending, and the
//! presenter, when it stays and still holds a replica, is told to drop it. The next presenter granted the session that speaks pages
//! is handed every parked page again as a fresh `page.request` ([`Core::hand_on_parked_pages`]), in the order they were
//! opened, a pane page once its pane is laid out; the placement rule is not run again, as a parked page belongs to the
//! session's viewer. A parked page closes here like any page, telling no presenter; its visibility and size are
//! recorded for the hand-on, and what needs a page shown is refused.

use std::time::{Duration, Instant};

use uuid::Uuid;

use super::frames::{
    self, Body, PresentationFrame, PresentationPage, PresentationPageAction, PresentationPageChange,
    PresentationPageCommand, PresentationPageSource, PresentationPane,
};
use super::hub::{PAGE_CONTENT_KIND, PAGE_DOCUMENT_KIND, PAGE_KINDS};
use crate::Core;
use crate::commands::overlay_page::PageOpening;
use crate::html::image_page::{BASE_PERCENT, ImagePixels};
use crate::html::{self, HtmlNavigation, HtmlOverlay, HtmlSource, errors};
use crate::model::{HandedOn, OverlayPane, OverlayPointSize, OverlaySize, RemoteOverlaySlot, RemotePage, Session};
use crate::pane_env::uuid_string;

/// How long a handed-over page that fetches over a channel waits for it to be claimed (upstream's `pageClaimWindow`).
pub const PAGE_CLAIM_WINDOW: Duration = Duration::from_secs(30);
/// How long after the claim window the expiry runs, so a claim made at its deadline is not raced (upstream's `+ 0.1`).
const CLAIM_EXPIRY_SLACK: Duration = Duration::from_millis(100);

/// Upstream's `OverlayHtmlError.remotePage`: the file manager and the browser are refused for a page shown on the
/// presenting machine that it fetches from here (every page but a URL page).
pub(crate) const REMOTE_PAGE: &str = "page is shown on another Mac";
/// Upstream's `OverlayHtmlError.remoteLoopback`.
pub(crate) const REMOTE_LOOPBACK: &str =
    "a loopback url names this Mac, so the Mac presenting the session cannot load it";
/// Upstream's `OverlayHtmlError.remoteTooLarge`.
pub(crate) const REMOTE_TOO_LARGE: &str = "page too large to show on another Mac";
/// Upstream's `OverlayResultError.viewerGone`.
pub(crate) const VIEWER_GONE: &str = "the viewer showing this overlay is gone";

/// What [`open_remote_page`] did with a page (upstream's `RemotePageOpen`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RemotePageOpen {
    /// No presenter, or the placement rule keeps the page here.
    Local,
    /// The slot holds an overlay, local or remote.
    SlotTaken,
    /// The session has no pane in the requested role.
    PaneMissing,
    /// The presenter cannot show pages, or not this kind of page, so the page is refused.
    Unsupported,
    /// A URL on this machine's loopback names a server the presenter cannot reach.
    Loopback,
    /// The request would not fit one presentation frame.
    TooLarge,
    Opened,
}

/// How a page command addressed to a slot ended (upstream's `RemotePageCommand`): the slot holds no page shown on the
/// presenting machine, the command was queued for that machine (not proof it applied it), or it was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RemotePageCommand {
    Local,
    Sent,
    Refused(&'static str),
}

/// A page's full source on the wire (upstream's `presentationSource`): a replica, itself shown for another machine, is
/// never handed on, and an image only with its probed size, `image_pixels`.
pub(crate) fn presentation_source(
    source: &HtmlSource,
    image_pixels: Option<ImagePixels>,
) -> Option<PresentationPageSource> {
    match source {
        HtmlSource::File { path, grant_root } => Some(PresentationPageSource::File {
            path: path.clone(),
            grant_root: grant_root.clone(),
        }),
        HtmlSource::Url(url) => Some(PresentationPageSource::Url {
            address: url.as_str().to_owned(),
        }),
        HtmlSource::Image { path } => image_pixels.map(|pixels| PresentationPageSource::Image {
            path: path.clone(),
            width: pixels.width,
            height: pixels.height,
        }),
        HtmlSource::Markdown { path } => Some(PresentationPageSource::Markdown { path: path.clone() }),
        HtmlSource::Text { path, line } => Some(PresentationPageSource::Text {
            path: path.clone(),
            line: *line,
        }),
        HtmlSource::Document { path } => Some(PresentationPageSource::Document { path: path.clone() }),
        HtmlSource::Replica { .. } => None,
    }
}

/// Whether a page of `source` can be handed to a presenter at all: every page this machine opened, never a replica (an
/// image once its size is probed).
pub(crate) fn travels(source: &HtmlSource) -> bool {
    !matches!(source, HtmlSource::Replica { .. })
}

/// Whether `session`'s presenter builds a page of `source`: a content page needs [`PAGE_CONTENT_KIND`] in its hello, a
/// clicked document [`PAGE_DOCUMENT_KIND`] too, since an older presenter fails its whole stream on a source it cannot
/// decode (upstream's guards in `openRemotePage`).
fn presenter_builds(core: &Core, session: Uuid, source: &PresentationPageSource) -> bool {
    let hub = &core.remote.hub;
    if source.is_content() && !hub.presenter_supports(PAGE_CONTENT_KIND, session) {
        return false;
    }
    !matches!(source, PresentationPageSource::Document { .. }) || hub.presenter_supports(PAGE_DOCUMENT_KIND, session)
}

/// The identity of the pane `pane` addresses, `None` for the session-wide slot.
///
/// # Errors
///
/// [`RemotePageOpen::PaneMissing`] for a split pane the session does not have.
fn slot_identity(session: &Session, pane: Option<OverlayPane>) -> Result<Option<Uuid>, RemotePageOpen> {
    match pane {
        None => Ok(None),
        Some(OverlayPane::Left) => Ok(Some(session.pane_identity)),
        Some(OverlayPane::Right) => session.split_pane_identity.map(Some).ok_or(RemotePageOpen::PaneMissing),
    }
}

/// The `page.request` for `overlay` opened as `opening` asks, in the slot of the pane `identity` names.
fn page_request(
    overlay: &HtmlOverlay,
    opening: &PageOpening,
    source: PresentationPageSource,
    identity: Option<Uuid>,
    token: String,
    size: OverlaySize,
) -> PresentationPage {
    let flags = overlay.flags;
    PresentationPage {
        width: size.points.width,
        height: size.points.height,
        page: uuid_string(overlay.id),
        token,
        source,
        identity: overlay.identity(),
        chromeless: flags.chromeless,
        javascript: flags.javascript,
        navigation: flags.navigation,
        browse: flags.browse,
        persistent: flags.persistent,
        background_color: opening.background_color.clone(),
        pane: identity.map(PresentationPane::Identity),
        size_percent: size.percent,
        follow: opening.follow,
    }
}

/// A pane page is full; point axes only apply to a floating session-wide page.
fn page_size(opening: &PageOpening) -> OverlaySize {
    let percent = opening
        .size_percent
        .filter(|_| opening.pane.is_none())
        .map(|percent| percent.clamp(1, 100));
    let points = if percent.is_some() {
        opening.points
    } else {
        OverlayPointSize::default()
    };
    OverlaySize { percent, points }
}

/// Hands `overlay` to the viewer presenting `session` under the program overlay placement rule (upstream's
/// `openRemotePage`): it reserves the slot, registers the page's outcome pending and asks the presenter to show it.
/// Nothing is mounted here. The request is encoded before anything changes, as a frame over the cap would fail the
/// offer and drop the presenter with every page it shows.
pub(crate) fn open_remote_page(
    core: &mut Core,
    window: Uuid,
    session: Uuid,
    overlay: &HtmlOverlay,
    opening: &PageOpening,
) -> RemotePageOpen {
    let slot = opening.pane;
    if !core.remote.hub.has_presenter(session) {
        return RemotePageOpen::Local;
    }
    let Some(source) = presentation_source(&overlay.source, opening.image_pixels) else {
        return RemotePageOpen::Local;
    };
    let Some(found) = core.library().store(window).and_then(|store| store.session(session)) else {
        return RemotePageOpen::Local;
    };
    if found.remote_overlays().slot(slot).is_some() || found.local_overlay_holds(slot) {
        return RemotePageOpen::SlotTaken;
    }
    let identity = match slot_identity(found, slot) {
        Ok(identity) => identity,
        Err(refusal) => return refusal,
    };
    if !core.pages_presenter_only() && !core.follows_remotely(found, identity) {
        return RemotePageOpen::Local;
    }
    if !PAGE_KINDS
        .iter()
        .all(|kind| core.remote.hub.presenter_supports(kind, session))
    {
        return RemotePageOpen::Unsupported;
    }
    if !presenter_builds(core, session, &source) {
        return RemotePageOpen::Unsupported;
    }
    if unreachable_loopback(core, session, &overlay.source) {
        return RemotePageOpen::Loopback;
    }
    let token = uuid_string(Uuid::new_v4());
    let size = page_size(opening);
    // the presenter sizes an unsized picture itself, as a floating panel at the base percent, so the slot records that
    // percent: the tree then reads it floating and a point resize has a percent to keep
    let presenter_sized = presenter_fits(&source, slot, size.percent);
    let reserved_percent = if presenter_sized {
        Some(BASE_PERCENT)
    } else {
        size.percent
    };
    let channel = source.uses_channel();
    let sent = OverlaySize {
        points: core.presentable_points(session, size.points),
        ..size
    };
    let request = Body::PageRequest(page_request(overlay, opening, source, identity, token.clone(), sent));
    if !fits_one_frame(&request) {
        return RemotePageOpen::TooLarge;
    }
    let hud = found.overlay.hud_active();
    let owner = core.remote.hub.presenter_generation(session);
    let Some(store) = core.library_mut().store_mut(window) else {
        return RemotePageOpen::Local;
    };
    if slot.is_none() && hud {
        store.close_hud(session);
    }
    let mut reserved = RemotePage::new(overlay.clone(), token, Instant::now() + PAGE_CLAIM_WINDOW);
    reserved.background_color.clone_from(&opening.background_color);
    reserved.image_pixels = opening.image_pixels;
    store.reserve_remote_overlay(
        session,
        RemoteOverlaySlot {
            points: size.points,
            job: uuid_string(overlay.id),
            pane: slot,
            owner,
            size_percent: reserved_percent,
            wait: false,
            ended: false,
            page: Some(reserved),
            presenter_sized,
        },
    );
    core.register_viewer_page_outcome(overlay.id);
    core.remote.hub.send_to_presenter(request, session);
    core.arm_page_claim_expiry(channel);
    RemotePageOpen::Opened
}

/// Whether the presenter fits a page of `source` in `slot` itself: a session-wide picture the caller gave no size.
fn presenter_fits(source: &PresentationPageSource, slot: Option<OverlayPane>, percent: Option<i64>) -> bool {
    matches!(source, PresentationPageSource::Image { .. }) && slot.is_none() && percent.is_none()
}

/// Whether `body` encodes into one presentation frame, at the largest generation and revision it could carry.
fn fits_one_frame(body: &Body) -> bool {
    let probe = PresentationFrame {
        generation: i64::MAX,
        rev: i64::MAX,
        body: body.clone(),
    };
    frames::encode(&probe).is_ok()
}

/// Whether `source` is a URL on this machine's loopback that `session`'s presenter cannot reach: the address it would
/// load, as the URL standard spells it, so a loopback alias counts too, unless the presenter forwards this machine's
/// loopback to its own ([`super::page_tunnel`]).
fn unreachable_loopback(core: &Core, session: Uuid, source: &HtmlSource) -> bool {
    matches!(source, HtmlSource::Url(url) if html::is_loopback(url.as_str()))
        && !core
            .remote
            .hub
            .presenter_supports(super::page_tunnel::TUNNEL_KIND, session)
}

/// Where a page handed to the presenter is reserved now (upstream's `remotePagePlace`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RemotePagePlace {
    pub window: Uuid,
    pub session: Uuid,
    pub pane: Option<OverlayPane>,
}

impl Core {
    /// Whether the placement rule hands a page in `pane`'s slot of `session` to the presenting machine (upstream's
    /// `pageBelongsRemote`): a presenter exists and the target pane, or every pane for the session-wide slot, follows
    /// a remote lead, or this is a headless origin. A local open is refused then: it would render where nobody looks.
    pub(crate) fn page_belongs_remote(&self, session: Uuid, pane: Option<OverlayPane>) -> bool {
        if !self.remote.hub.has_presenter(session) {
            return false;
        }
        let Some(found) = self.presented_session(session) else {
            return false;
        };
        slot_identity(found, pane)
            .is_ok_and(|identity| self.pages_presenter_only() || self.follows_remotely(found, identity))
    }

    /// The page in `pane`'s slot of `session` shown on the presenting machine, if one is.
    fn remote_page(&self, session: Uuid, pane: Option<OverlayPane>) -> Option<&RemotePage> {
        self.presented_session(session)?
            .remote_overlays()
            .slot(pane)?
            .page
            .as_ref()
    }

    /// Whether `slot` holds a page shown on the stream it was handed to: not parked, and that stream still presents.
    fn page_shown_by_owner(&self, session: Uuid, slot: &RemoteOverlaySlot) -> bool {
        slot.page.as_ref().is_some_and(|page| !page.parked)
            && self.remote.hub.has_presenter(session)
            && self.remote.hub.presenter_generation(session) == slot.owner
    }

    /// Asks the presenter to act on the page in `pane`'s slot (upstream's `sendRemotePageCommand`); false when no remote
    /// page sits there, it is parked, or the stream it was handed to is gone. A hide or show is recorded as the pending
    /// visibility.
    pub(crate) fn send_remote_page_command(
        &mut self,
        session: Uuid,
        pane: Option<OverlayPane>,
        action: PresentationPageAction,
        size: OverlaySize,
        merge: Option<bool>,
    ) -> bool {
        let owned = self
            .presented_session(session)
            .and_then(|found| found.remote_overlays().slot(pane))
            .is_some_and(|slot| self.page_shown_by_owner(session, slot));
        if !owned {
            return false;
        }
        let Some((job, seq)) = self
            .library_mut()
            .store_for_session_mut(session)
            .and_then(|store| store.number_remote_page_command(session, pane, action))
        else {
            return false;
        };
        let command = PresentationPageCommand {
            width: size.points.width,
            height: size.points.height,
            page: job,
            seq,
            action,
            size_percent: size.percent,
            merge,
        };
        self.remote.hub.send_to_presenter(Body::PageCommand(command), session)
    }

    /// Runs a toolbar navigation on the page in `pane`'s slot when the presenting machine shows it (upstream's
    /// `navigateRemotePage`): history goes over; the browser only for a URL page, every page it fetches from here (a
    /// file or content page) being refused like a file page; the file manager never, a URL page keeping the answer it
    /// gets here.
    pub(crate) fn navigate_remote_page(
        &mut self,
        session: Uuid,
        pane: Option<OverlayPane>,
        navigation: HtmlNavigation,
    ) -> RemotePageCommand {
        let Some(shown) = self.remote_page(session, pane) else {
            return RemotePageCommand::Local;
        };
        let file = !matches!(shown.overlay.source, HtmlSource::Url(_));
        match navigation {
            HtmlNavigation::Back => self.deliver_remote_page_command(session, pane, PresentationPageAction::Back),
            HtmlNavigation::Forward => self.deliver_remote_page_command(session, pane, PresentationPageAction::Forward),
            HtmlNavigation::Browser | HtmlNavigation::Finder if file => RemotePageCommand::Refused(REMOTE_PAGE),
            HtmlNavigation::Browser => self.deliver_remote_page_command(session, pane, PresentationPageAction::Browser),
            HtmlNavigation::Finder => RemotePageCommand::Refused(errors::FINDER_REQUIRES_FILE),
        }
    }

    /// Asks the presenting machine to reload the page in `pane`'s slot, its original source or what it shows now
    /// (upstream's `reloadRemotePage`).
    pub(crate) fn reload_remote_page(
        &mut self,
        session: Uuid,
        pane: Option<OverlayPane>,
        current: bool,
    ) -> RemotePageCommand {
        if self.remote_page(session, pane).is_none() {
            return RemotePageCommand::Local;
        }
        let action = if current {
            PresentationPageAction::ReloadCurrent
        } else {
            PresentationPageAction::ReloadOriginal
        };
        self.deliver_remote_page_command(session, pane, action)
    }

    fn deliver_remote_page_command(
        &mut self,
        session: Uuid,
        pane: Option<OverlayPane>,
        action: PresentationPageAction,
    ) -> RemotePageCommand {
        if self.send_remote_page_command(session, pane, action, OverlaySize::default(), None) {
            RemotePageCommand::Sent
        } else {
            RemotePageCommand::Refused(VIEWER_GONE)
        }
    }

    /// Applies what the presenter of `session` reported about one of its pages (upstream's `receiveRemotePageFrame`):
    /// a state is mirrored, a rejection or close ends the page. A parked page takes none of them: the presenter
    /// answering the `page.close` that parked it only confirms it dropped its replica.
    pub(super) fn receive_remote_page_frame(&mut self, session: Uuid, body: &Body) {
        let page = match body {
            Body::PageState(state) => &state.page,
            Body::PageRejected(change) | Body::PageClosed(change) => &change.page,
            _ => return,
        };
        let parked = self
            .presented_session(session)
            .and_then(|found| found.remote_overlays().slot_of_job(page))
            .and_then(|slot| slot.page.as_ref())
            .is_some_and(|page| page.parked);
        if parked {
            return;
        }
        let Some(store) = self.library_mut().store_for_session_mut(session) else {
            return;
        };
        let change = match body {
            Body::PageState(state) => {
                store.apply_remote_page_state(session, state);
                return;
            }
            Body::PageRejected(change) => {
                tracing::info!(
                    reason = change.reason.as_deref().unwrap_or(""),
                    "the presenter refused a page"
                );
                change
            }
            Body::PageClosed(change) => change,
            _ => return,
        };
        if let Some(page) = store.end_remote_page(session, &change.page) {
            self.remote_page_ended(page);
        }
    }

    /// A page closed here: the presenter takes it down when it was `shown` there, and it ends (upstream's
    /// `closeRemoteOverlay` page arm).
    pub(super) fn close_remote_page(&mut self, session: Uuid, page: Uuid, shown: bool) {
        if shown {
            let close = Body::PageClose(PresentationPageChange::page(uuid_string(page)));
            self.remote.hub.send_to_presenter(close, session);
        }
        self.remote_page_ended(page);
    }

    /// Sends a visibility request the model numbered, while the page is still held for the stream it was handed to.
    pub(super) fn deliver_page_command(&mut self, session: Uuid, command: PresentationPageCommand) {
        let owned = self
            .presented_session(session)
            .and_then(|found| found.remote_overlays().slot_of_job(&command.page))
            .is_some_and(|slot| self.page_shown_by_owner(session, slot));
        if owned {
            self.remote.hub.send_to_presenter(Body::PageCommand(command), session);
        }
    }

    /// A page handed to the presenting machine ended, by close, rejection or session removal: its
    /// outcome is dismissed unless it was answered (upstream's `endRemotePage` and `RemotePageEnds`).
    pub(super) fn remote_page_ended(&mut self, page: Uuid) {
        self.dismiss_page_outcome(page);
        self.shut_page_serve_channel(page);
    }

    /// Where page `page` is reserved now, `None` once it ended, parked or its presenter changed (upstream's
    /// `remotePagePlace`): a swap moves the slot, and a new presenter generation means the page belongs to a stream
    /// that is gone.
    pub(crate) fn remote_page_place(&self, page: Uuid) -> Option<RemotePagePlace> {
        let job = uuid_string(page);
        let library = self.library();
        library.open_ids().into_iter().find_map(|window| {
            library.store(window)?.sessions().find_map(|session| {
                let slot = session.remote_overlays().slot_of_job(&job)?;
                self.page_shown_by_owner(session.id(), slot).then_some(RemotePagePlace {
                    window,
                    session: session.id(),
                    pane: slot.pane,
                })
            })
        })
    }

    /// Ends page `page` wherever its reservation sits, which also tells the presenter (upstream's server-side
    /// `endRemotePage`): the slot is closed as `session.overlay.close` closes it.
    pub(super) fn end_remote_page_anywhere(&mut self, page: Uuid) {
        let job = uuid_string(page);
        let library = self.library();
        let found = library.open_ids().into_iter().find_map(|window| {
            library.store(window)?.sessions().find_map(|session| {
                let slot = session.remote_overlays().slot_of_job(&job)?;
                Some((window, session.id(), slot.pane))
            })
        });
        if let Some((window, session, pane)) = found
            && let Some(store) = self.library_mut().store_mut(window)
        {
            store.close_remote_overlay(session, pane);
            self.settle_remote_overlays();
        }
    }

    /// Parks page `page` wherever its reservation sits, its channel's helper gone or its claim expired, and tells the
    /// presenter showing it to drop its replica; a page already parked is left as it is.
    pub(super) fn park_remote_page_anywhere(&mut self, page: Uuid) {
        let job = uuid_string(page);
        let Some(session) = self.library().open_ids().into_iter().find_map(|window| {
            self.library()
                .store(window)?
                .sessions()
                .find(|session| session.remote_overlays().slot_of_job(&job).is_some())
                .map(Session::id)
        }) else {
            return;
        };
        let parked = self
            .library_mut()
            .store_for_session_mut(session)
            .is_some_and(|store| store.park_remote_page(session, &job));
        if parked {
            let close = Body::PageClose(PresentationPageChange::page(job));
            self.remote.hub.send_to_presenter(close, session);
            self.shut_page_serve_channel(page);
        }
    }

    /// The presenter of `session` let page `job` go without closing it (`page.detached`: its helper ended there): a page
    /// it shows is parked as for a helper lost here and its channel shut, telling the presenter nothing, as it holds no
    /// replica any more. It waits for the next presenter; a page already parked, or handed to another presenter, is
    /// left as it is.
    pub(super) fn detach_remote_page(&mut self, session: Uuid, job: &str) {
        let Some(page) = self
            .presented_session(session)
            .and_then(|found| found.remote_overlays().slot_of_job(job))
            .filter(|slot| self.page_shown_by_owner(session, slot))
            .and_then(|slot| slot.page.as_ref())
            .map(|page| page.overlay.id)
        else {
            return;
        };
        let parked = self
            .library_mut()
            .store_for_session_mut(session)
            .is_some_and(|store| store.park_remote_page(session, job));
        if parked {
            self.shut_page_serve_channel(page);
        }
    }

    /// Arms the claim expiry of a page handed over: one that fetches over a `channel` (every page but a URL page) has
    /// to claim it in time, a URL page loads with none.
    fn arm_page_claim_expiry(&mut self, channel: bool) {
        if channel {
            self.remote
                .page_serve
                .expiries
                .push(PAGE_CLAIM_WINDOW + CLAIM_EXPIRY_SLACK);
        }
    }

    /// The presenter of `session` was just granted, or a pane of the session came into view: when the presenter speaks
    /// pages, every page parked in the session and not handed to it yet is handed to it (an extension beyond upstream),
    /// each as a fresh `page.request` under a new claim token and the new generation, never raising the session
    /// (`follow` off), at the size the slot keeps, then a hide when it was hidden. A page parked while this presenter
    /// held the session (its helper gone or its claim expired) waits for the next one. A loopback URL page waits for a
    /// presenter that forwards this machine's loopback, a pane page for its pane to be laid out (a presenter refuses a
    /// page for a pane it cannot show, which would end it), and one whose request no longer fits a frame stays parked.
    ///
    /// Pages go in the order they were reserved, as the viewer stacks a pane page over the session-wide page open when
    /// it arrives and `page.request` carries no stacking of its own. So while a pane page waits for its pane, the
    /// session-wide page reserved after it waits too, and so does every page reserved after that one; they follow it
    /// once the pane is laid out.
    pub(crate) fn hand_on_parked_pages(&mut self, session: Uuid) {
        if !PAGE_KINDS
            .iter()
            .all(|kind| self.remote.hub.presenter_supports(kind, session))
        {
            return;
        }
        let generation = self.remote.hub.presenter_generation(session);
        let parked: Vec<(RemoteOverlaySlot, bool)> = self
            .presented_session(session)
            .map(|found| {
                found
                    .remote_overlays()
                    .slots()
                    .iter()
                    .filter(|slot| slot.owner != generation && slot.page.as_ref().is_some_and(|page| page.parked))
                    .map(|slot| {
                        let unlaid = slot.pane.is_some_and(|pane| !found.renders_overlay_pane(pane));
                        (slot.clone(), unlaid)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut pane_waits = false;
        for (slot, unlaid) in &parked {
            if *unlaid {
                pane_waits = true;
                continue;
            }
            if pane_waits && slot.pane.is_none() {
                // the session-wide page and every page after it keep their place above the waiting pane page
                return;
            }
            self.hand_on_parked_page(session, slot);
        }
    }

    fn hand_on_parked_page(&mut self, session: Uuid, slot: &RemoteOverlaySlot) {
        let Some(page) = &slot.page else {
            return;
        };
        let overlay = &page.overlay;
        // a picture a restore parked again has no probed size: it is read now, as an open reads it
        let pixels = page.image_pixels.or_else(|| match &overlay.source {
            HtmlSource::Image { path } => self.probe_image_pixels(path),
            _ => None,
        });
        let Some(source) = presentation_source(&overlay.source, pixels) else {
            return;
        };
        // a content page waits for a presenter that builds it
        if !presenter_builds(self, session, &source) {
            return;
        }
        let Some(identity) = self
            .presented_session(session)
            .and_then(|found| slot_identity(found, slot.pane).ok())
        else {
            return;
        };
        if unreachable_loopback(self, session, &overlay.source) {
            return;
        }
        // a picture the last presenter fitted itself is fitted again by the next one
        let fitted = slot.presenter_sized;
        let opening = PageOpening {
            source: overlay.source.clone(),
            flags: overlay.flags,
            pane: slot.pane,
            size_percent: slot.size_percent.filter(|_| !fitted),
            points: slot.points,
            background_color: page.background_color.clone(),
            follow: false,
            clicked: overlay.clicked,
            text_log: overlay.text_log,
            image_pixels: pixels,
        };
        let size = page_size(&opening);
        let sent = OverlaySize {
            points: self.presentable_points(session, size.points),
            ..size
        };
        let token = uuid_string(Uuid::new_v4());
        let channel = source.uses_channel();
        let request = Body::PageRequest(page_request(overlay, &opening, source, identity, token.clone(), sent));
        if !fits_one_frame(&request) {
            return;
        }
        let owner = self.remote.hub.presenter_generation(session);
        let deadline = Instant::now() + PAGE_CLAIM_WINDOW;
        let Some(handed) = self
            .library_mut()
            .store_for_session_mut(session)
            .and_then(|store| store.hand_on_remote_page(session, &slot.job, owner, token, deadline))
        else {
            return;
        };
        self.remote.hub.send_to_presenter(request, session);
        if let HandedOn::Hidden(seq) = handed {
            let command = PresentationPageCommand {
                width: None,
                height: None,
                page: slot.job.clone(),
                seq,
                action: PresentationPageAction::Hide,
                size_percent: None,
                merge: None,
            };
            self.remote.hub.send_to_presenter(Body::PageCommand(command), session);
        }
        self.arm_page_claim_expiry(channel);
    }

    /// Admits the page channel `token` names (upstream's `claimRemotePage`): unclaimed, inside its deadline at `now`,
    /// and still shown by the presenter it was handed to. A token claims once. The page's id and the source its
    /// channel serves.
    pub(crate) fn claim_remote_page(&mut self, token: &str, now: Instant) -> Option<(Uuid, HtmlSource)> {
        let library = self.library();
        let found = library.open_ids().into_iter().find_map(|window| {
            library.store(window)?.sessions().find_map(|session| {
                let id = session.id();
                let slot = session
                    .remote_overlays()
                    .slots()
                    .iter()
                    .find(|slot| slot.page.as_ref().is_some_and(|page| page.token == token))?;
                let page = slot.page.as_ref()?;
                let admissible = !page.claimed && now <= page.claim_deadline && self.page_shown_by_owner(id, slot);
                admissible.then(|| {
                    (
                        window,
                        id,
                        slot.job.clone(),
                        page.overlay.id,
                        page.overlay.source.clone(),
                    )
                })
            })
        });
        let (window, session, job, page, source) = found?;
        self.library_mut().store_mut(window)?.claim_remote_page(session, &job);
        Some((page, source))
    }

    /// Parks every channel page shown on a presenter whose channel was not claimed by `now` (upstream's
    /// `expireRemotePageClaims`, which ends it); a URL page loads on the presenter with no channel and never expires.
    pub(crate) fn expire_remote_page_claims_at(&mut self, now: Instant) {
        let library = self.library();
        let expired: Vec<Uuid> = library
            .open_ids()
            .into_iter()
            .filter_map(|window| library.store(window))
            .flat_map(|store| {
                store.sessions().flat_map(move |session| {
                    session.remote_overlays().slots().iter().filter_map(move |slot| {
                        let page = slot.page.as_ref()?;
                        let unclaimed = !page.claimed
                            && !page.parked
                            && now > page.claim_deadline
                            && !matches!(page.overlay.source, HtmlSource::Url(_));
                        unclaimed.then_some(page.overlay.id)
                    })
                })
            })
            .collect();
        for page in expired {
            self.park_remote_page_anywhere(page);
        }
    }

    /// Runs the claim expiry, at a deadline the frontend armed from [`Core::take_page_claim_expiries`].
    pub fn expire_remote_page_claims(&mut self) {
        self.expire_remote_page_claims_at(Instant::now());
    }
}

#[cfg(test)]
mod document_tests {
    use super::*;
    use crate::html::ReplicaContent;

    /// Upstream's `everyLocalPageKindTravelsToAnotherMacButAReplica` and `anImageWithoutItsProbedSizeDoesNotTravel`.
    #[test]
    fn every_local_page_kind_travels_but_a_replica_and_an_unprobed_image() {
        let pixels = Some(ImagePixels {
            width: 800,
            height: 600,
        });
        let image = HtmlSource::Image {
            path: "/a.png".to_owned(),
        };
        assert_eq!(
            presentation_source(&image, pixels),
            Some(PresentationPageSource::Image {
                path: "/a.png".to_owned(),
                width: 800,
                height: 600
            })
        );
        assert_eq!(presentation_source(&image, None), None);
        assert!(travels(&image));
        for (source, sent) in [
            (
                HtmlSource::Markdown {
                    path: "/a.md".to_owned(),
                },
                PresentationPageSource::Markdown {
                    path: "/a.md".to_owned(),
                },
            ),
            (
                HtmlSource::Text {
                    path: "/a.go".to_owned(),
                    line: Some(3),
                },
                PresentationPageSource::Text {
                    path: "/a.go".to_owned(),
                    line: Some(3),
                },
            ),
            (
                HtmlSource::Document {
                    path: "/pr.html".to_owned(),
                },
                PresentationPageSource::Document {
                    path: "/pr.html".to_owned(),
                },
            ),
        ] {
            assert_eq!(presentation_source(&source, None), Some(sent));
        }
        for content in [ReplicaContent::Html, ReplicaContent::Image { width: 1, height: 1 }] {
            let replica = HtmlSource::Replica {
                path: "/a.html".to_owned(),
                content,
            };
            assert_eq!(presentation_source(&replica, pixels), None);
            assert!(!travels(&replica));
        }
    }
}
