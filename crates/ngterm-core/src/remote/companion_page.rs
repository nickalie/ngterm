//! Serving this machine's page companions to the machines viewing their parents (upstream's
//! `ControlServer+CompanionPage.swift`).
//!
//! `session.companion.page.attach` hands out one attachment of a page companion: the page and a single-use token its
//! `session.overlay.page.serve` claims within [`PAGE_CLAIM_WINDOW`]. Each claim is its own channel
//! ([`super::page_serve`]), and ending one ends nothing else: the page belongs to this machine until it closes here,
//! which ends every channel and token serving it. A soft-closed page still serves, so undo brings it back with its
//! viewers attached.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use uuid::Uuid;

use super::frames::PresentationPage;
use super::page::{PAGE_CLAIM_WINDOW, RemotePagePlace, presentation_source};
use super::page_serve::PageServeStream;
use crate::Core;
use crate::html::{self, HtmlOverlay, HtmlSource};
use crate::model::SessionContent;
use crate::pane_env::uuid_string;

/// Upstream's refusal of an attach of anything but a page companion.
const NOT_A_PAGE_COMPANION: &str = "not a page companion";
/// Upstream's refusal of a page on this machine's loopback, which the other machine would never reach.
const LOOPBACK_PAGE: &str = "a page on this Mac's loopback cannot be shown on another Mac";

/// One attachment of a page companion handed out and not claimed yet (upstream's `CompanionPageToken`).
#[derive(Debug, Clone, Copy)]
struct CompanionPageToken {
    session: Uuid,
    page: Uuid,
    deadline: Instant,
}

/// The page companion one claimed attachment serves; `claim` keys its channel (upstream's `CompanionPageServing`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompanionPageServing {
    pub claim: Uuid,
    pub session: Uuid,
    pub page: Uuid,
}

/// The attachments handed out, claimed and adopted (upstream's `companionPageTokens`, `pendingCompanionClaims` and
/// `companionPageStreams`).
#[derive(Debug, Default)]
pub(crate) struct CompanionPageRuntime {
    tokens: HashMap<String, CompanionPageToken>,
    claims: HashMap<Uuid, CompanionPageServing>,
    pub(super) streams: HashMap<Uuid, Arc<PageServeStream>>,
}

impl CompanionPageRuntime {
    /// Takes the claim answered `ok` under id `claim`, for its adoption.
    pub(super) fn take_claim(&mut self, claim: Uuid) -> Option<CompanionPageServing> {
        self.claims.remove(&claim)
    }
}

impl Core {
    /// `session.companion.page.attach` on session `session` (upstream's `attachCompanionPage`): a token for one claim
    /// of the page's channel, and the page as the other machine shows it.
    ///
    /// # Errors
    ///
    /// The refusal: `session` is not a page companion, or its page is on this machine's loopback.
    pub(crate) fn attach_companion_page(
        &mut self,
        session: Uuid,
        now: Instant,
    ) -> Result<PresentationPage, &'static str> {
        let found = self
            .library()
            .store_for_session(session)
            .and_then(|store| store.session(session))
            .filter(|row| row.content() == SessionContent::Page);
        let (overlay, background) = found
            .and_then(|row| Some((row.overlay.page()?.clone(), row.overlay.background_color.clone())))
            .ok_or(NOT_A_PAGE_COMPANION)?;
        let source = presentation_source(&overlay.source).ok_or(NOT_A_PAGE_COMPANION)?;
        if let HtmlSource::Url(url) = &overlay.source
            && html::is_loopback(url.as_str())
        {
            return Err(LOOPBACK_PAGE);
        }
        let page = PresentationPage {
            page: uuid_string(overlay.id),
            token: uuid_string(Uuid::new_v4()),
            source,
            identity: overlay.identity(),
            chromeless: overlay.flags.chromeless,
            javascript: overlay.flags.javascript,
            navigation: overlay.flags.navigation,
            browse: overlay.flags.browse,
            persistent: overlay.flags.persistent,
            background_color: background,
            pane: None,
            size_percent: None,
            follow: false,
        };
        let companions = &mut self.remote.page_serve.companions;
        companions.tokens.retain(|_, token| token.deadline >= now);
        companions.tokens.insert(
            page.token.clone(),
            CompanionPageToken {
                session,
                page: overlay.id,
                deadline: now + PAGE_CLAIM_WINDOW,
            },
        );
        Ok(page)
    }

    /// Admits a page companion's token once (upstream's `claimCompanionPage`): `None` for a token that is not one,
    /// else the claim's id, or `Err` for a token past its deadline or whose page is gone.
    pub(crate) fn claim_companion_page(&mut self, token: &str, now: Instant) -> Option<Result<Uuid, ()>> {
        let entry = self.remote.page_serve.companions.tokens.remove(token)?;
        if now > entry.deadline || self.served_companion_page(entry.session, entry.page).is_none() {
            return Some(Err(()));
        }
        let claim = Uuid::new_v4();
        self.remote.page_serve.companions.claims.insert(
            claim,
            CompanionPageServing {
                claim,
                session: entry.session,
                page: entry.page,
            },
        );
        Some(Ok(claim))
    }

    /// Where a served page companion sits now, `None` once it is gone (upstream's `companionPagePlace`). A soft-closed
    /// page still serves, from the window whose undoable close holds it.
    pub(crate) fn companion_page_place(&self, serving: &CompanionPageServing) -> Option<RemotePagePlace> {
        self.served_companion_page(serving.session, serving.page)?;
        Some(RemotePagePlace {
            window: self.library().window_holding_session(serving.session)?,
            session: serving.session,
            pane: None,
        })
    }

    /// Ends every attachment of page `page`, which left its slot here (upstream's `endCompanionPageServing`): its
    /// unclaimed tokens, its claims not adopted yet and its channels.
    pub(crate) fn end_companion_page_serving(&mut self, page: Uuid) {
        let companions = &mut self.remote.page_serve.companions;
        companions.tokens.retain(|_, token| token.page != page);
        companions.claims.retain(|_, serving| serving.page != page);
        let ended: Vec<Uuid> = companions
            .streams
            .iter()
            .filter(|(_, stream)| stream.page() == page)
            .map(|(claim, _)| *claim)
            .collect();
        for claim in ended {
            if let Some(stream) = companions.streams.remove(&claim) {
                stream.shutdown();
            }
        }
    }

    /// The page companion `session` showing page `page`, wherever an open window holds it, its undoable close included.
    pub(super) fn served_companion_page(&self, session: Uuid, page: Uuid) -> Option<&HtmlOverlay> {
        let row = self.library().held_session(session)?;
        if row.content() != SessionContent::Page {
            return None;
        }
        row.overlay.page().filter(|overlay| overlay.id == page)
    }

    /// How many page companion channels are adopted now.
    #[must_use]
    pub fn companion_page_streams(&self) -> usize {
        self.remote.page_serve.companions.streams.len()
    }

    /// Whether a page companion token is still waiting for its claim.
    #[must_use]
    pub fn companion_page_token_pending(&self, token: &str) -> bool {
        self.remote.page_serve.companions.tokens.contains_key(token)
    }
}
