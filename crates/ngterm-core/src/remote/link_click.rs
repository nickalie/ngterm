//! Paths clicked in a pane attached from another machine, opened by that machine (upstream's #923:
//! `RemoteLinkRefusal.swift`, `AppStore+LinkClickRequest.swift`, `AppStore+ForwardedClick.swift`,
//! `ControlServer+RemoteLinkClick.swift` and `ControlServer+ForwardedClick.swift`).
//!
//! **Viewer**: the link policy turns a bare, relative or `file://` path clicked in a remote pane into
//! [`Route::ForwardRemote`]. While this machine presents the session, the origin agreed [`LINK_CLICK_KIND`], nothing
//! here would hide the page it opens and the clicked pane has an origin twin, the path goes to the origin as typed in a
//! `link.click` frame with the origin's pane identity ([`Core::forward_remote_link_click`]); this machine never reads or
//! reveals it. Any other outcome, and the origin's `link.refused` answer, shows a short self-hiding HUD worded here
//! from the refusal's reason and the session's host, or plays the alert sound when a HUD, a cover, a zoomed terminal or
//! this machine's own hidden page is in the way.
//!
//! **Origin**: a `link.click` from the session's presenter ([`super::hub::PresentationHub::receive`]) is resolved with
//! that pane's directory as a click in that pane here would be, on the main thread as the control path reads a page it
//! hands over, and only a page route opens, handed to the presenter or not at all
//! ([`crate::commands::overlay_page`]'s forwarded open). It reveals nothing and opens no browser here. Every click it
//! cannot open answers the presenter with one `link.refused`. A forwarded page replaces a clicked page the presenter
//! hides, once every other check passed ([`super::page::open_remote_page`]).

use std::path::Path;

use uuid::Uuid;

use super::frames::{Body, PresentationLinkClick, PresentationLinkRefusal, PresentationMode};
use super::hub::LINK_CLICK_KIND;
use crate::Core;
use crate::html::HtmlSource;
use crate::html::text_page::{self, TextLog};
use crate::hud::HudSpec;
use crate::link_policy::{self, ClickOrigin, Route};
use crate::model::StatusPane;
use crate::pane_env::uuid_string;
use crate::persist::settings::LinkOpenMode;

/// A path clicked in a pane attached from another machine (upstream's `RemoteLinkClick`). `pane` is this machine's
/// identity of the clicked pane, not the origin's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteLinkClick {
    pub raw: String,
    pub session: Uuid,
    pub pane: Option<Uuid>,
    pub mode: LinkOpenMode,
}

/// Why a path clicked in a pane attached from another machine was not opened (upstream's `RemoteLinkRefusal`). It
/// travels by its raw value, and the presenting machine builds the text itself, so no wording from the other machine
/// reaches the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteLinkRefusal {
    RevealIntent,
    NotPresenter,
    OriginTooOld,
    PaneNotAttached,
    /// A HUD, a zoomed terminal or a cover on this machine would hide the page.
    Hidden,
    NotFound,
    /// The file exists on the other machine but is not a kind a page can show.
    NotAPage,
    /// The other machine could not open the page, for a busy slot or a pane led from there.
    NotOpened,
}

impl RemoteLinkRefusal {
    pub const ALL: [Self; 8] = [
        Self::RevealIntent,
        Self::NotPresenter,
        Self::OriginTooOld,
        Self::PaneNotAttached,
        Self::Hidden,
        Self::NotFound,
        Self::NotAPage,
        Self::NotOpened,
    ];

    /// The wire value.
    #[must_use]
    pub fn raw(self) -> &'static str {
        match self {
            Self::RevealIntent => "revealIntent",
            Self::NotPresenter => "notPresenter",
            Self::OriginTooOld => "originTooOld",
            Self::PaneNotAttached => "paneNotAttached",
            Self::Hidden => "hidden",
            Self::NotFound => "notFound",
            Self::NotAPage => "notAPage",
            Self::NotOpened => "notOpened",
        }
    }

    #[must_use]
    pub fn from_raw(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|reason| reason.raw() == raw)
    }

    /// The refusal's text for a session attached from `host` (upstream's `message(host:)`, ngterm for agterm).
    #[must_use]
    pub fn message(self, host: &str) -> String {
        match self {
            Self::RevealIntent => format!("the file is on {host}; only a file on this Mac can be revealed in Finder"),
            Self::NotPresenter => format!("another Mac presents this session, so {host} cannot show the file here"),
            Self::OriginTooOld => format!("the ngterm on {host} is too old to open clicked files"),
            Self::PaneNotAttached => format!("this pane is not attached to {host}"),
            Self::Hidden => format!("the page from {host} would be hidden here"),
            Self::NotFound => format!("no such file on {host}"),
            Self::NotAPage => format!("{host} cannot show this file as a page"),
            Self::NotOpened => format!("{host} could not open the file here"),
        }
    }
}

/// Whether `path` names something on disk.
fn exists(path: &str) -> bool {
    Path::new(path).try_exists().is_ok_and(|found| found)
}

impl Core {
    // MARK: - viewer

    /// Sends a path clicked in an attached pane to its origin, or reports why it cannot go (upstream's
    /// `LinkOpener.carryOut` `.forwardRemote` arm with `forwardRemoteLinkClick`): `reveal` (the reveal chord) is
    /// refused, as the file is on the other machine.
    pub fn forward_remote_link_click(&mut self, click: &RemoteLinkClick, reveal: bool) {
        let sent = if reveal {
            Err(RemoteLinkRefusal::RevealIntent)
        } else {
            self.send_remote_link_click(click)
        };
        if let Err(reason) = sent {
            self.refuse_remote_link_click(click.session, reason);
        }
    }

    fn send_remote_link_click(&mut self, click: &RemoteLinkClick) -> Result<(), RemoteLinkRefusal> {
        let supported = self.origin_link_speaks(click.session, LINK_CLICK_KIND);
        let hidden = self.remote_link_page_hidden(click.session);
        let frame = self.link_click_request(click, supported, hidden)?;
        self.answer_origin(click.session, Body::LinkClick(frame));
        Ok(())
    }

    /// The frame that forwards `click` to its session's origin, or why it cannot go (upstream's `linkClickRequest`):
    /// this machine must present the session, the origin must speak `link.click`, nothing here may hide the page it
    /// opens, and the clicked pane must have an origin twin.
    pub(crate) fn link_click_request(
        &self,
        click: &RemoteLinkClick,
        origin_supports_click: bool,
        hidden: bool,
    ) -> Result<PresentationLinkClick, RemoteLinkRefusal> {
        let presentation = self
            .library()
            .store_for_session(click.session)
            .and_then(|store| store.session(click.session))
            .and_then(crate::model::Session::remote_presentation)
            .filter(|presentation| presentation.mode == PresentationMode::Presenter)
            .ok_or(RemoteLinkRefusal::NotPresenter)?;
        if !origin_supports_click {
            return Err(RemoteLinkRefusal::OriginTooOld);
        }
        if hidden {
            return Err(RemoteLinkRefusal::Hidden);
        }
        let pane = click
            .pane
            .and_then(|local| presentation.binding.remote_pane(local))
            .ok_or(RemoteLinkRefusal::PaneNotAttached)?;
        Ok(PresentationLinkClick {
            raw: click.raw.clone(),
            pane,
            mode: click.mode,
        })
    }

    /// Reports a click that was not carried out with a panel that hides itself, as a failed attach does, or with the
    /// alert sound when that panel would replace another HUD or not be seen (upstream's `refuseRemoteLinkClick`).
    pub(crate) fn refuse_remote_link_click(&mut self, session: Uuid, reason: RemoteLinkRefusal) {
        if reason != RemoteLinkRefusal::Hidden
            && !self.remote_link_page_hidden(session)
            && self.open_link_refusal_hud(session, reason)
        {
            return;
        }
        self.play_status_sound("default".to_owned());
    }

    fn open_link_refusal_hud(&mut self, session: Uuid, reason: RemoteLinkRefusal) -> bool {
        let Some(host) = self
            .library()
            .store_for_session(session)
            .and_then(|store| store.session(session))
            .and_then(|found| found.remote_host().map(str::to_owned))
        else {
            return false;
        };
        let spec = HudSpec {
            hide_after: Some(super::attach::ERROR_SECONDS),
            ..HudSpec::new(reason.message(&host))
        };
        let placement = crate::commands::hud::Placement::default();
        if crate::commands::hud::open(self, Some(&uuid_string(session)), None, spec, &placement).is_err() {
            return false;
        }
        let generation = self
            .library()
            .store_for_session(session)
            .and_then(|store| store.session(session))
            .map(|found| found.overlay.generation);
        if let Some(generation) = generation {
            self.remote.viewer.link_refusal_huds.insert(session, generation);
        }
        true
    }

    /// Whether `session`'s live HUD is still the refusal panel a forwarded click opened, which a mirrored HUD replaces
    /// rather than waiting behind (upstream's `showsRemoteLinkRefusal`).
    pub(crate) fn shows_remote_link_refusal(&self, session: Uuid) -> bool {
        let Some(generation) = self.remote.viewer.link_refusal_huds.get(&session) else {
            return false;
        };
        self.library()
            .store_for_session(session)
            .and_then(|store| store.session(session))
            .is_some_and(|found| found.overlay.hud_active() && found.overlay.generation == *generation)
    }

    /// Whether this machine would hide a page or panel over `session` (upstream's `remoteLinkPageHidden`): a HUD
    /// already up other than a refusal panel, a shown cover, a hidden one of this machine's own (a hidden replica page
    /// the origin replaces does not count), or a zoomed terminal in its window.
    fn remote_link_page_hidden(&self, session: Uuid) -> bool {
        let Some(found) = self
            .library()
            .store_for_session(session)
            .and_then(|store| store.session(session))
        else {
            return true;
        };
        if found.overlay.hud_active() && !self.shows_remote_link_refusal(session) {
            return true;
        }
        if found.cover_overlay_visible() {
            return true;
        }
        let replica = found.overlay.page().is_some_and(|page| page.replica_of.is_some());
        if found.cover_overlay_active() && !replica {
            return true;
        }
        self.library()
            .window_id_for_session(session)
            .is_some_and(|window| self.zoom_target(window).is_some())
    }

    // MARK: - origin

    /// Opens the page a path clicked on the presenting machine names, the way a click in that pane here would, and
    /// never on this machine (upstream's `receiveForwardedClick`); a click it cannot open is answered with one refusal
    /// to the presenter that sent it.
    pub(super) fn receive_forwarded_click(&mut self, session: Uuid, click: &PresentationLinkClick) {
        if let Err(reason) = self.open_forwarded_click(session, click) {
            let refusal = Body::LinkRefused(PresentationLinkRefusal { reason });
            self.remote.hub.send_to_presenter(refusal, session);
        }
    }

    fn open_forwarded_click(&mut self, session: Uuid, click: &PresentationLinkClick) -> Result<(), RemoteLinkRefusal> {
        let cwd = self
            .forwarded_click_cwd(session, click.pane)
            .ok_or(RemoteLinkRefusal::PaneNotAttached)?;
        if link_policy::names_foreign_absolute_path(&click.raw) {
            return Err(RemoteLinkRefusal::NotFound);
        }
        let origin = ClickOrigin::Pane(session);
        let candidates = link_policy::locations(&click.raw, origin, Some(&cwd));
        let route = link_policy::resolved_route(&click.raw, &candidates, click.mode, origin, exists);
        // a location nobody found routes as typed, and a relative one then has nothing to open
        if route == Route::Ignore && !candidates.is_empty() {
            return Err(RemoteLinkRefusal::NotFound);
        }
        if route
            .local_file()
            .is_some_and(|file| !file.try_exists().is_ok_and(|found| found))
        {
            return Err(RemoteLinkRefusal::NotFound);
        }
        let path = |path: std::path::PathBuf| path.to_string_lossy().into_owned();
        // a presenter without live text pages is handed the plain page it can show
        let log = (route.opens_as_log() && self.click_opens_log(session)).then_some(TextLog {
            tail: text_page::DEFAULT_TAIL,
            follow: true,
        });
        let (source, size_percent) = match route {
            Route::Image { path: file, .. } => (HtmlSource::Image { path: path(file) }, None),
            Route::Markdown {
                path: file,
                size_percent,
                ..
            } => (HtmlSource::Markdown { path: path(file) }, size_percent),
            Route::Document {
                path: file,
                size_percent,
                ..
            } => (HtmlSource::Document { path: path(file) }, size_percent),
            Route::Text {
                path: file,
                line,
                size_percent,
                ..
            } => (HtmlSource::Text { path: path(file), line }, size_percent),
            _ => return Err(RemoteLinkRefusal::NotAPage),
        };
        if self.open_forwarded_link(source, session, size_percent, log) {
            Ok(())
        } else {
            Err(RemoteLinkRefusal::NotOpened)
        }
    }

    /// The directory of the pane a forwarded click names, by its identity (upstream's `forwardedClickPane`); `None`
    /// for one that is neither the session's primary nor its split.
    fn forwarded_click_cwd(&self, session: Uuid, pane: Uuid) -> Option<String> {
        let found = self.presented_session(session)?;
        let role = if pane == found.pane_identity {
            StatusPane::Left
        } else if Some(pane) == found.split_pane_identity {
            StatusPane::Right
        } else {
            return None;
        };
        Some(found.cwd_for(role).to_owned())
    }
}

#[cfg(test)]
#[path = "link_click_tests.rs"]
mod tests;
