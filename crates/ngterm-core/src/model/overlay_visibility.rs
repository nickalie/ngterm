//! Hiding overlays without closing them (upstream's `Session+OverlayVisibility.swift` and
//! `AppStore+OverlayVisibility.swift`): a hidden program or page keeps running and stays addressable, while the
//! terminal under it takes the keyboard and the clicks.
//!
//! Each hideable slot carries its own hidden bit: the session-wide slot's ([`super::OverlaySlot::hidden`]) and each
//! pane overlay's ([`super::PaneOverlay::hidden`]), so a pane's bit follows its overlay across a swap or a promotion.
//! Closing a slot clears its bit, so a new occupant starts shown. A page shown on the presenting machine has no bit
//! here: a hide or show asks the presenter, and the page counts by its pending request, else by what the presenter
//! last reported.
//!
//! A pane overlay opened over a session-wide program or page draws above it (upstream's `holdsSessionCover`, #831):
//! while such a holder is shown on a laid-out pane, the session-wide slot is out of sight
//! ([`Session::session_slot_shown`]), and the visible predicates, the tree's `hiddenOverlays` and a local page's
//! `htmlOverlays[].hidden` follow. It is derived, never written into the session-wide bit, which stays the user's, so
//! a hide/show/toggle round trip keeps the opener on top; a replica page reports its own raw bit to its origin.
//!
//! Occupancy questions (`overlay`, `paneOverlays`, a result, the read commands, page lookup, the remote local hold)
//! keep asking the `*_active` predicates; what covers or owns input asks the `*_visible` ones here.

use uuid::Uuid;

use super::overlay::OverlayPane;
use super::remote_overlay::{RemoteOverlayChange, RemoteOverlaySlot};
use super::session::Session;
use super::store::AppStore;
use super::window_library::WindowLibrary;

/// The control error for an open into a slot whose overlay is hidden (upstream's `OverlayVisibilityError.hidden`):
/// the slot still holds its overlay, running; show or close it before opening another.
pub const OVERLAY_HIDDEN: &str = "overlay hidden";

/// What `session.overlay.hide|show|toggle` asks of the scoped overlays (upstream's `OverlayVisibilityMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayVisibilityMode {
    Hide,
    Show,
    Toggle,
}

/// What owns a session's keyboard instead of its terminal (upstream's `VisibleCover`): a session-wide program or
/// page, or one over the focused pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisibleCover {
    Session,
    FocusedPane,
}

/// Why a hide, show or toggle of a session's overlays refused (upstream's `OverlayVisibilityFailure`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayVisibilityFailure {
    UnknownSession,
    NoOverlay,
    /// The scope holds only a slot a viewer presents, which that viewer alone can hide.
    Remote,
}

impl OverlayVisibilityFailure {
    /// The control error for this refusal (upstream's `message`).
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::UnknownSession => "no such session",
            Self::NoOverlay => "no overlay",
            Self::Remote => "overlay is shown on another Mac",
        }
    }
}

impl Session {
    /// Whether the session-wide occupant is drawn (upstream's `sessionSlotShown`): not hidden by hand, and not under a
    /// pane overlay opened over it that is shown on a laid-out pane. The session-wide hidden bit stays the user's own.
    #[must_use]
    pub fn session_slot_shown(&self) -> bool {
        !self.overlay.hidden && !self.session_cover_held()
    }

    /// Whether a pane overlay opened over the session-wide program or page is shown on a laid-out pane, so it draws
    /// above that cover (upstream's `sessionCoverHeld`). Local presentation only: a replica page reports its own bit.
    fn session_cover_held(&self) -> bool {
        let Some(cover) = self.cover_hold() else {
            return false;
        };
        [OverlayPane::Left, OverlayPane::Right].into_iter().any(|pane| {
            self.pane_overlay(pane)
                .is_some_and(|overlay| overlay.holds_cover == Some(cover) && !overlay.hidden)
                && self.renders_overlay_pane(pane)
        })
    }

    /// What a pane overlay opened now holds ([`super::PaneOverlay::holds_cover`]): the generation of the session-wide
    /// program or page, hidden or not; `None` with no such cover (a HUD is none).
    pub(crate) fn cover_hold(&self) -> Option<u64> {
        self.cover_overlay_active().then_some(self.overlay.generation)
    }

    /// [`Session::cover_overlay_active`] minus an occupant out of sight: the input-exclusion question (upstream's
    /// `coverOverlayVisible`).
    #[must_use]
    pub fn cover_overlay_visible(&self) -> bool {
        self.cover_overlay_active() && self.session_slot_shown()
    }

    /// [`super::OverlaySlot::program_active`] minus an occupant out of sight (upstream's `programOverlayVisible`).
    #[must_use]
    pub fn program_overlay_visible(&self) -> bool {
        self.overlay.program_active() && self.session_slot_shown()
    }

    /// [`Session::html_overlay_active`] minus an occupant out of sight (upstream's `htmlOverlayVisible`).
    #[must_use]
    pub fn html_overlay_visible(&self) -> bool {
        self.html_overlay_active() && self.session_slot_shown()
    }

    /// [`Session::full_overlay_active`] minus an occupant out of sight: whether the panes are hidden (upstream's
    /// `fullOverlayVisible`).
    #[must_use]
    pub fn full_overlay_visible(&self) -> bool {
        self.full_overlay_active() && self.session_slot_shown()
    }

    /// The cover owning the keyboard now, `None` for the terminal itself (upstream's `visibleCover`). It changes only
    /// when what the user types into changes: a hidden overlay closing or the other pane's overlay toggling leaves it
    /// alone, unless that overlay keeps the session-wide cover out of sight, and so does a pane overlay under the
    /// scratch, which keeps the keyboard.
    #[must_use]
    pub fn visible_cover(&self) -> Option<VisibleCover> {
        if self.cover_overlay_visible() {
            return Some(VisibleCover::Session);
        }
        if self.scratch_active {
            return None;
        }
        self.focused_overlay_pane().map(|_| VisibleCover::FocusedPane)
    }

    /// Whether `pane` holds an overlay that covers it now (upstream's `paneOverlayVisible`).
    #[must_use]
    pub fn pane_overlay_visible(&self, pane: OverlayPane) -> bool {
        self.pane_overlay(pane).is_some_and(|overlay| !overlay.hidden)
    }

    /// The hidden bit of the slot `pane` addresses, the session-wide one for `None` (upstream's `overlayHidden(_:)`):
    /// the user's own choice, which a pane overlay holding the session-wide cover leaves alone.
    #[must_use]
    pub fn overlay_hidden(&self, pane: Option<OverlayPane>) -> bool {
        match pane {
            None => self.overlay.hidden,
            Some(pane) => self.pane_overlay(pane).is_some_and(|overlay| overlay.hidden),
        }
    }

    /// The slots `pane` scopes that hold a program or page, `None` standing for the session-wide slot; every slot of
    /// the session for a `None` scope, ordered session, left, right (upstream's `hideableOverlaySlots`). A HUD, an ask
    /// and a slot reserved for a viewer hold nothing that can be hidden.
    #[must_use]
    pub fn hideable_overlay_slots(&self, pane: Option<OverlayPane>) -> Vec<Option<OverlayPane>> {
        if let Some(pane) = pane {
            return if self.pane_overlay(pane).is_some() {
                vec![Some(pane)]
            } else {
                Vec::new()
            };
        }
        // a page companion's page is the row: it never hides
        let session =
            (self.cover_overlay_active() && self.content() == super::SessionContent::Terminal).then_some(None);
        session
            .into_iter()
            .chain(
                [OverlayPane::Left, OverlayPane::Right]
                    .into_iter()
                    .filter(|pane| self.pane_overlay(*pane).is_some())
                    .map(Some),
            )
            .collect()
    }

    /// Whether a slot `pane` scopes holds a program or page that is not hidden: what a toggle hides (upstream's
    /// `anyOverlayShown`). A page shown on the presenting machine counts by its pending request, else by its report.
    #[must_use]
    pub fn any_overlay_shown(&self, pane: Option<OverlayPane>) -> bool {
        self.hideable_overlay_slots(pane)
            .into_iter()
            .any(|slot| !self.overlay_hidden(slot))
            || self
                .remote_page_slots(pane)
                .iter()
                .any(|slot| slot.page.as_ref().is_some_and(|page| !page.effective_hidden()))
    }

    /// The reserved slots `pane` scopes that hold a page shown on the presenting machine; every such slot for a `None`
    /// scope (upstream's `remotePageSlots`).
    #[must_use]
    pub fn remote_page_slots(&self, pane: Option<OverlayPane>) -> Vec<&RemoteOverlaySlot> {
        self.remote_overlays
            .slots()
            .iter()
            .filter(|slot| slot.page.is_some() && (pane.is_none() || slot.pane == pane))
            .collect()
    }

    /// Whether the slot `pane` addresses holds an occupant that is not drawn now (upstream's `overlayOutOfSight`): by
    /// hand or, for the session-wide slot, under a pane overlay opened over it. What the local read-back reports as
    /// hidden.
    #[must_use]
    pub fn overlay_out_of_sight(&self, pane: Option<OverlayPane>) -> bool {
        match pane {
            None => !self.session_slot_shown(),
            Some(_) => self.overlay_hidden(pane),
        }
    }

    /// The tree's `hiddenOverlays`: the slots out of sight as `session`, `left`, `right`, a page shown on the presenting
    /// machine counted as that machine reports it (upstream's `hiddenOverlayNames`).
    #[must_use]
    pub fn hidden_overlay_names(&self) -> Vec<&'static str> {
        let mut hidden: Vec<Option<OverlayPane>> = self
            .hideable_overlay_slots(None)
            .into_iter()
            .filter(|slot| self.overlay_out_of_sight(*slot))
            .collect();
        hidden.extend(
            self.remote_page_slots(None)
                .into_iter()
                .filter(|slot| slot.page.as_ref().is_some_and(|page| page.hidden))
                .map(|slot| slot.pane),
        );
        [None, Some(OverlayPane::Left), Some(OverlayPane::Right)]
            .into_iter()
            .filter(|slot| hidden.contains(slot))
            .map(|slot| slot.map_or("session", OverlayPane::wire_name))
            .collect()
    }

    /// Sets the hidden bit of the slot `pane` addresses (upstream's `setOverlayHidden(_:slot:)`); a pane slot without
    /// an overlay is left alone.
    pub(crate) fn set_overlay_hidden(&mut self, hidden: bool, pane: Option<OverlayPane>) {
        match pane {
            None => self.overlay.hidden = hidden,
            Some(pane) => {
                if let Some(overlay) = self.pane_overlays.slot_mut(pane).overlay.as_mut() {
                    overlay.hidden = hidden;
                }
            }
        }
    }

    /// Why `pane`'s scope holds nothing to hide (upstream's `overlayVisibilityRefusal`): a slot reserved there for a
    /// viewer is shown on another machine, which alone can hide it.
    #[must_use]
    pub fn overlay_visibility_refusal(&self, pane: Option<OverlayPane>) -> OverlayVisibilityFailure {
        let reserved = match pane {
            Some(pane) => self.remote_overlays.slot(Some(pane)).is_some(),
            None => !self.remote_overlays.slots().is_empty(),
        };
        if reserved {
            OverlayVisibilityFailure::Remote
        } else {
            OverlayVisibilityFailure::NoOverlay
        }
    }
}

impl AppStore {
    /// Hides or shows session `id`'s overlays: one pane's, or every program and page of the session for a `None`
    /// `pane` (upstream's `setOverlaysHidden`). A hidden overlay keeps running and stays addressable. Success includes
    /// a call that changed nothing; the tree is invalidated only by a change. A page shown on the presenting machine is
    /// asked to change, and its reported state changes when that machine answers.
    ///
    /// # Errors
    ///
    /// An unknown session, or a scope holding nothing to hide.
    pub fn set_overlays_hidden(
        &mut self,
        id: Uuid,
        pane: Option<OverlayPane>,
        hidden: bool,
    ) -> Result<(), OverlayVisibilityFailure> {
        let session = self.session_mut(id).ok_or(OverlayVisibilityFailure::UnknownSession)?;
        let slots = session.hideable_overlay_slots(pane);
        let pages: Vec<Option<OverlayPane>> = session
            .remote_page_slots(pane)
            .into_iter()
            .map(|slot| slot.pane)
            .collect();
        if slots.is_empty() && pages.is_empty() {
            return Err(session.overlay_visibility_refusal(pane));
        }
        self.request_remote_page_visibility(id, &pages, hidden);
        let Some(session) = self.session_mut(id) else {
            return Ok(());
        };
        let changing: Vec<Option<OverlayPane>> = slots
            .into_iter()
            .filter(|slot| session.overlay_hidden(*slot) != hidden)
            .collect();
        for slot in &changing {
            session.set_overlay_hidden(hidden, *slot);
        }
        if !changing.is_empty() {
            self.schedule_tree_changed();
        }
        Ok(())
    }

    /// Asks the presenter for `hidden` on each page in `panes`' slots whose pending or reported visibility differs.
    fn request_remote_page_visibility(&mut self, id: Uuid, panes: &[Option<OverlayPane>], hidden: bool) {
        use crate::remote::frames::{PresentationPageAction, PresentationPageCommand};
        let action = if hidden {
            PresentationPageAction::Hide
        } else {
            PresentationPageAction::Show
        };
        for pane in panes {
            let differs = self
                .session(id)
                .and_then(|found| found.remote_overlays.slot(*pane))
                .and_then(|slot| slot.page.as_ref())
                .is_some_and(|page| page.effective_hidden() != hidden);
            if !differs {
                continue;
            }
            if let Some((page, seq)) = self.number_remote_page_command(id, *pane, action) {
                let command = PresentationPageCommand {
                    page,
                    seq,
                    action,
                    size_percent: None,
                };
                self.remote_overlay_changes
                    .push(RemoteOverlayChange::PageCommand { session: id, command });
            }
        }
    }

    /// Hides the scoped overlays while any of them is shown, otherwise shows them all (upstream's
    /// `toggleOverlaysHidden`).
    ///
    /// # Errors
    ///
    /// As [`AppStore::set_overlays_hidden`].
    pub fn toggle_overlays_hidden(
        &mut self,
        id: Uuid,
        pane: Option<OverlayPane>,
    ) -> Result<(), OverlayVisibilityFailure> {
        let shown = self
            .session(id)
            .ok_or(OverlayVisibilityFailure::UnknownSession)?
            .any_overlay_shown(pane);
        self.set_overlays_hidden(id, pane, shown)
    }

    /// Runs a control `mode` against the scoped overlays (upstream's `applyOverlayVisibility`).
    ///
    /// # Errors
    ///
    /// As [`AppStore::set_overlays_hidden`].
    pub fn apply_overlay_visibility(
        &mut self,
        id: Uuid,
        pane: Option<OverlayPane>,
        mode: OverlayVisibilityMode,
    ) -> Result<(), OverlayVisibilityFailure> {
        // a page companion's page is never hidden, so showing it is already done
        if mode == OverlayVisibilityMode::Show
            && pane.is_none()
            && self
                .session(id)
                .is_some_and(|session| session.content() == super::SessionContent::Page)
        {
            return Ok(());
        }
        match mode {
            OverlayVisibilityMode::Hide => self.set_overlays_hidden(id, pane, true),
            OverlayVisibilityMode::Show => self.set_overlays_hidden(id, pane, false),
            OverlayVisibilityMode::Toggle => self.toggle_overlays_hidden(id, pane),
        }
    }
}

impl WindowLibrary {
    /// Hides page `id` from its own strip with the page's scope: a pane page hides its pane, a session-wide page every
    /// overlay of its session (upstream's `hideHtmlOverlay`). False once the page left its slot, and while its session
    /// sits in an undoable close.
    pub fn hide_html_overlay(&mut self, id: Uuid) -> bool {
        let Some(slot) = self.html_overlay_slot(id) else {
            return false;
        };
        self.store_mut(slot.window)
            .is_some_and(|store| store.set_overlays_hidden(slot.session, slot.pane, true).is_ok())
    }
}
