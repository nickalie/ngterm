//! Pages handed to the viewer presenting a session, on the origin (upstream's `AppStore+RemotePage.swift` and the page
//! arms of `ControlServer+SessionActions.swift`).
//!
//! A page opened on a session another machine presents goes to that machine under the program overlay placement rule:
//! the slot is reserved here (a [`RemotePage`] in its [`RemoteOverlaySlot`]), the page's outcome is registered pending
//! and a `page.request` carries a single-use claim token. Nothing renders here. The presenter's `page.state` reports
//! are mirrored into the read-back; hide, show, reload, history and resize are sequence-numbered `page.command`s; a
//! close here, a rejection or close there, the presenter's loss and the session's removal end the page, keeping an
//! answer already submitted.

use std::time::{Duration, Instant};

use uuid::Uuid;

use super::frames::{
    self, Body, PresentationFrame, PresentationPage, PresentationPageAction, PresentationPageChange,
    PresentationPageCommand, PresentationPageSource, PresentationPane,
};
use super::hub::PAGE_KINDS;
use crate::Core;
use crate::commands::overlay_page::PageOpening;
use crate::html::{self, HtmlNavigation, HtmlOverlay, HtmlSource, errors};
use crate::model::{OverlayPane, RemoteOverlaySlot, RemotePage, Session};
use crate::pane_env::uuid_string;

/// How long a handed-over file page waits for its channel to be claimed (upstream's `pageClaimWindow`).
pub const PAGE_CLAIM_WINDOW: Duration = Duration::from_secs(30);
/// How long after the claim window the expiry runs, so a claim made at its deadline is not raced (upstream's `+ 0.1`).
const CLAIM_EXPIRY_SLACK: Duration = Duration::from_millis(100);

/// Upstream's `OverlayHtmlError.remotePage`: the file manager and the browser are refused for a file page shown on the
/// presenting machine.
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
    /// The presenter cannot show pages, so the page is refused.
    Unsupported,
    /// A URL on this machine's loopback names a server the presenter cannot reach.
    Loopback,
    /// The request would not fit one presentation frame.
    TooLarge,
    /// The open carries point sizes, which the presenting machine does not take.
    PointsUnsupported,
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

/// A page's full source on the wire; a replica, itself shown for another machine, is never handed on.
pub(super) fn presentation_source(source: &HtmlSource) -> Option<PresentationPageSource> {
    match source {
        HtmlSource::File { path, grant_root } => Some(PresentationPageSource::File {
            path: path.clone(),
            grant_root: grant_root.clone(),
        }),
        HtmlSource::Url(url) => Some(PresentationPageSource::Url {
            address: url.as_str().to_owned(),
        }),
        HtmlSource::Replica { .. } => None,
    }
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
    size_percent: Option<i64>,
) -> PresentationPage {
    let flags = overlay.flags;
    PresentationPage {
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
        size_percent,
        follow: opening.follow,
    }
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
    let Some(source) = presentation_source(&overlay.source) else {
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
    if !core.follows_remotely(found, identity) {
        return RemotePageOpen::Local;
    }
    // the presentation wire carries only a percent, so nothing is reserved or sent
    if !opening.points.is_empty() {
        return RemotePageOpen::PointsUnsupported;
    }
    if !PAGE_KINDS
        .iter()
        .all(|kind| core.remote.hub.presenter_supports(kind, session))
    {
        return RemotePageOpen::Unsupported;
    }
    // the address the presenter would load, as the URL standard spells it, so a loopback alias is refused too
    if let HtmlSource::Url(url) = &overlay.source
        && html::is_loopback(url.as_str())
    {
        return RemotePageOpen::Loopback;
    }
    let token = uuid_string(Uuid::new_v4());
    let size = if slot.is_none() {
        opening.size_percent.map(|percent| percent.clamp(1, 100))
    } else {
        None
    };
    let request = Body::PageRequest(page_request(overlay, opening, source, identity, token.clone(), size));
    let probe = PresentationFrame {
        generation: i64::MAX,
        rev: i64::MAX,
        body: request.clone(),
    };
    if frames::encode(&probe).is_err() {
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
    let reserved = RemotePage::new(overlay.clone(), token, Instant::now() + PAGE_CLAIM_WINDOW);
    store.reserve_remote_overlay(
        session,
        RemoteOverlaySlot {
            job: uuid_string(overlay.id),
            pane: slot,
            owner,
            size_percent: size,
            wait: false,
            ended: false,
            page: Some(reserved),
        },
    );
    core.register_page_outcome(overlay.id);
    core.remote.hub.send_to_presenter(request, session);
    if matches!(overlay.source, HtmlSource::File { .. }) {
        core.remote
            .page_serve
            .expiries
            .push(PAGE_CLAIM_WINDOW + CLAIM_EXPIRY_SLACK);
    }
    RemotePageOpen::Opened
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
    /// a remote lead. A local open is refused then: it would render where nobody looks.
    pub(crate) fn page_belongs_remote(&self, session: Uuid, pane: Option<OverlayPane>) -> bool {
        if !self.remote.hub.has_presenter(session) {
            return false;
        }
        let Some(found) = self.presented_session(session) else {
            return false;
        };
        slot_identity(found, pane).is_ok_and(|identity| self.follows_remotely(found, identity))
    }

    /// The page in `pane`'s slot of `session` shown on the presenting machine, if one is.
    fn remote_page(&self, session: Uuid, pane: Option<OverlayPane>) -> Option<&RemotePage> {
        self.presented_session(session)?
            .remote_overlays()
            .slot(pane)?
            .page
            .as_ref()
    }

    /// Asks the presenter to act on the page in `pane`'s slot (upstream's `sendRemotePageCommand`); false when no remote
    /// page sits there or the stream it was handed to is gone. A hide or show is recorded as the pending visibility.
    pub(crate) fn send_remote_page_command(
        &mut self,
        session: Uuid,
        pane: Option<OverlayPane>,
        action: PresentationPageAction,
        size_percent: Option<i64>,
    ) -> bool {
        let owned = self
            .presented_session(session)
            .and_then(|found| found.remote_overlays().slot(pane))
            .is_some_and(|slot| slot.page.is_some() && self.remote.hub.presenter_generation(session) == slot.owner);
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
            page: job,
            seq,
            action,
            size_percent,
        };
        self.remote.hub.send_to_presenter(Body::PageCommand(command), session)
    }

    /// Runs a toolbar navigation on the page in `pane`'s slot when the presenting machine shows it (upstream's
    /// `navigateRemotePage`): history goes over; the browser only for a URL page; the file manager never, a URL page
    /// keeping the answer it gets here.
    pub(crate) fn navigate_remote_page(
        &mut self,
        session: Uuid,
        pane: Option<OverlayPane>,
        navigation: HtmlNavigation,
    ) -> RemotePageCommand {
        let Some(shown) = self.remote_page(session, pane) else {
            return RemotePageCommand::Local;
        };
        let file = matches!(shown.overlay.source, HtmlSource::File { .. });
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
        if self.send_remote_page_command(session, pane, action, None) {
            RemotePageCommand::Sent
        } else {
            RemotePageCommand::Refused(VIEWER_GONE)
        }
    }

    /// Applies what the presenter of `session` reported about one of its pages (upstream's `receiveRemotePageFrame`):
    /// a state is mirrored, a rejection or close ends the page.
    pub(super) fn receive_remote_page_frame(&mut self, session: Uuid, body: &Body) {
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

    /// A page closed here: the presenter takes it down, and it ends (upstream's `closeRemoteOverlay` page arm).
    pub(super) fn close_remote_page(&mut self, session: Uuid, page: Uuid) {
        let close = Body::PageClose(PresentationPageChange::page(uuid_string(page)));
        self.remote.hub.send_to_presenter(close, session);
        self.remote_page_ended(page);
    }

    /// Sends a visibility request the model numbered, while the page is still held for the stream it was handed to.
    pub(super) fn deliver_page_command(&mut self, session: Uuid, command: PresentationPageCommand) {
        let owned = self
            .presented_session(session)
            .and_then(|found| found.remote_overlays().slot_of_job(&command.page))
            .is_some_and(|slot| slot.page.is_some() && self.remote.hub.presenter_generation(session) == slot.owner);
        if owned {
            self.remote.hub.send_to_presenter(Body::PageCommand(command), session);
        }
    }

    /// A page handed to the presenting machine ended, by close, rejection, presenter loss or session removal: its
    /// outcome is dismissed unless it was answered (upstream's `endRemotePage` and `RemotePageEnds`).
    pub(super) fn remote_page_ended(&mut self, page: Uuid) {
        self.dismiss_page_outcome(page);
        self.shut_page_serve_channel(page);
    }

    /// Where page `page` is reserved now, `None` once it ended or its presenter changed (upstream's
    /// `remotePagePlace`): a swap moves the slot, and a new presenter generation means the page belongs to a stream
    /// that is gone.
    pub(crate) fn remote_page_place(&self, page: Uuid) -> Option<RemotePagePlace> {
        let job = uuid_string(page);
        let library = self.library();
        library.open_ids().into_iter().find_map(|window| {
            library.store(window)?.sessions().find_map(|session| {
                let slot = session.remote_overlays().slot_of_job(&job)?;
                let hub = &self.remote.hub;
                let held = slot.page.is_some()
                    && hub.has_presenter(session.id())
                    && hub.presenter_generation(session.id()) == slot.owner;
                held.then_some(RemotePagePlace {
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

    /// Admits the page channel `token` names (upstream's `claimRemotePage`): unclaimed, inside its deadline at `now`,
    /// and still held for the presenter it was handed to. A token claims once. The page's id and the source its
    /// channel serves.
    pub(crate) fn claim_remote_page(&mut self, token: &str, now: Instant) -> Option<(Uuid, HtmlSource)> {
        let library = self.library();
        let hub = &self.remote.hub;
        let found = library.open_ids().into_iter().find_map(|window| {
            library.store(window)?.sessions().find_map(|session| {
                let id = session.id();
                let slot = session
                    .remote_overlays()
                    .slots()
                    .iter()
                    .find(|slot| slot.page.as_ref().is_some_and(|page| page.token == token))?;
                let page = slot.page.as_ref()?;
                let admissible = !page.claimed
                    && now <= page.claim_deadline
                    && hub.has_presenter(id)
                    && hub.presenter_generation(id) == slot.owner;
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

    /// Ends every file page whose channel was not claimed by `now` (upstream's `expireRemotePageClaims`); a URL page
    /// loads on the presenter with no channel and never expires.
    pub(crate) fn expire_remote_page_claims_at(&mut self, now: Instant) {
        let library = self.library();
        let expired: Vec<(Uuid, Uuid, Option<OverlayPane>)> = library
            .open_ids()
            .into_iter()
            .filter_map(|window| library.store(window).map(|store| (window, store)))
            .flat_map(|(window, store)| {
                store.sessions().flat_map(move |session| {
                    session.remote_overlays().slots().iter().filter_map(move |slot| {
                        let page = slot.page.as_ref()?;
                        let unclaimed = !page.claimed
                            && now > page.claim_deadline
                            && matches!(page.overlay.source, HtmlSource::File { .. });
                        unclaimed.then_some((window, session.id(), slot.pane))
                    })
                })
            })
            .collect();
        for (window, session, pane) in expired {
            if let Some(store) = self.library_mut().store_mut(window) {
                store.close_remote_overlay(session, pane);
            }
        }
        self.settle_remote_overlays();
    }

    /// Runs the claim expiry, at a deadline the frontend armed from [`Core::take_page_claim_expiries`].
    pub fn expire_remote_page_claims(&mut self) {
        self.expire_remote_page_claims_at(Instant::now());
    }
}
