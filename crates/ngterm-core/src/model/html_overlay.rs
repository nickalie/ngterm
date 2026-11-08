//! Pages in the overlay slots (upstream's `AppStore+HtmlOverlay.swift`, `Session+HtmlOverlay.swift`): a page takes the
//! session-wide slot or one pane's slot with the slot rules of a program, answers a submit, reloads, and records what
//! the frontend's web view reports, wherever the page sits now, a session an undoable close holds included.
//!
//! A page has no terminal: it never counts as a program (`program_active`), only as a cover (`cover_overlay_active`,
//! and `cover_overlay_visible` while it is in sight: not hidden, and not held under a pane overlay opened over it,
//! [`Session::session_slot_shown`]).
//! Its outcome is the core's ([`crate::html::HtmlPageOutcomes`]): the core registers a page it opens or a restore
//! brought back, records a submit's answer, and settles a page that left every slot as dismissed at the next commit,
//! leaving an answered page alone; closing a slot records nothing.

use uuid::Uuid;

use super::overlay::OverlayPane;
use super::overlay_points::{OverlayPointSize, OverlaySize};
use super::page_snapshot::{park_local_pages, persisted_pages};
use super::pane::StatusPane;
use super::pane_overlay::PaneOverlay;
use super::session::Session;
use super::store::AppStore;
use super::window_library::WindowLibrary;
use crate::html::text_page::{TextChanges, TextLogReport, TextSearch};
use crate::html::{HtmlLoadState, HtmlOverlay, HtmlPageInfo, HtmlReloadTarget, HtmlSource, SearchCount, errors};
use crate::persist::PageSnapshot;

/// Why [`AppStore::open_html_overlay`] refused (upstream's `HtmlOverlayOpenFailure`; its `presenter` case is the
/// core's, which knows the presentation hub: `overlay_page::open` refuses before the store is asked).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HtmlOverlayOpenFailure {
    UnknownSession,
    AlreadyOpen,
    PaneNotVisible,
}

/// Why a command addressing a page refused (upstream's `HtmlOverlayCommandFailure`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HtmlOverlayCommandFailure {
    UnknownSession,
    NoOverlay,
    NotHtml,
}

impl HtmlOverlayCommandFailure {
    /// The control error for this refusal (upstream's `message`).
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::UnknownSession => "no such session",
            Self::NoOverlay => "no overlay",
            Self::NotHtml => "the overlay is not an html page",
        }
    }
}

/// Why [`AppStore::set_text_log_following`] refused (upstream's `TextLogFollowFailure`, with its page lookup's
/// `replica` case beside the others).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextLogFollowFailure {
    Page(HtmlOverlayCommandFailure),
    /// The page is shown here for another machine, which alone follows its file.
    Replica,
    NotLog,
}

impl TextLogFollowFailure {
    /// The control error for this refusal (upstream's `message`).
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::Page(failure) => failure.message(),
            Self::Replica => errors::REPLICA_SUBMIT,
            Self::NotLog => "no log page",
        }
    }
}

/// Why a `session.overlay.search` request was refused (upstream's `TextSearchFailure`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextSearchFailure {
    Page(HtmlOverlayCommandFailure),
    /// The slot holds a page that is not a local text page, a replica included.
    NotText,
    /// The slot is the presenting machine's, which alone shows the page.
    Remote,
    /// The page's renderer is not up to take a step.
    NotLoaded,
    /// A step with no query set.
    NoQuery,
}

impl TextSearchFailure {
    /// The control error for this refusal (upstream's `message`).
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::Page(failure) => failure.message(),
            Self::NotText => "no text page",
            Self::Remote => "remote page: search it on the Mac that shows it",
            Self::NotLoaded => "page not loaded",
            Self::NoQuery => "no search query: set one first",
        }
    }
}

/// Why a `session.overlay.changes` step was refused (upstream's `TextChangesFailure`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextChangesFailure {
    Page(HtmlOverlayCommandFailure),
    /// The slot holds a page that is not a plain local diff page.
    NotDiff,
    /// The slot is the presenting machine's, which alone shows the page.
    Remote,
    /// The page's renderer is not up to take a step.
    NotLoaded,
    /// The page has no change to step to.
    NoChanges,
}

impl TextChangesFailure {
    /// The control error for this refusal (upstream's `message`).
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::Page(failure) => failure.message(),
            Self::NotDiff => "no diff page",
            Self::Remote => "remote page: step it on the Mac that shows it",
            Self::NotLoaded => "page not loaded",
            Self::NoChanges => "no changes",
        }
    }
}

/// Where a page sits: its window, its session, and the pane slot holding it (`None` for the session-wide one).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HtmlPageSlot {
    pub window: Uuid,
    pub session: Uuid,
    pub pane: Option<OverlayPane>,
}

impl Session {
    /// Whether a page covers the session (upstream's `htmlOverlayActive`).
    #[must_use]
    pub fn html_overlay_active(&self) -> bool {
        self.overlay.html_active()
    }

    /// Whether `pane`'s own overlay is a page (upstream's `paneOverlayIsHtml`).
    #[must_use]
    pub fn pane_overlay_is_html(&self, pane: OverlayPane) -> bool {
        self.pane_overlay(pane).is_some_and(|overlay| overlay.html.is_some())
    }

    /// Whether a page holds the slot a `--pane` command addresses, the session-wide one for `None` (upstream's
    /// `htmlCovers`).
    #[must_use]
    pub fn html_covers(&self, pane: Option<OverlayPane>) -> bool {
        pane.map_or_else(|| self.html_overlay_active(), |pane| self.pane_overlay_is_html(pane))
    }

    /// Whether a page covers the terminal a `--pane` font command addresses (upstream's `htmlHidesTerminal`): the
    /// session-wide page covers both panes and a shown scratch, a pane page only its own pane, a hidden page nothing.
    #[must_use]
    pub fn html_hides_terminal(&self, pane: Option<StatusPane>) -> bool {
        let pane_page = |pane: OverlayPane| self.pane_overlay_visible(pane) && self.pane_overlay_is_html(pane);
        match pane {
            Some(StatusPane::Scratch) => self.html_overlay_visible() && self.scratch_active,
            None | Some(StatusPane::Left) => self.html_overlay_visible() || pane_page(OverlayPane::Left),
            Some(StatusPane::Right) => self.html_overlay_visible() || pane_page(OverlayPane::Right),
        }
    }

    /// The page that takes the keyboard when focus returns to this session, in the order covers resolve; `None` when a
    /// terminal is on top (upstream's `topmostHtmlOverlay`). A hidden page never does.
    #[must_use]
    pub fn topmost_html_overlay(&self) -> Option<&HtmlOverlay> {
        if self.html_overlay_visible() {
            return self.overlay.page();
        }
        if self.program_overlay_visible() || self.scratch_active {
            return None;
        }
        self.focused_overlay_pane()
            .and_then(|pane| self.pane_overlay(pane))
            .and_then(|overlay| overlay.html.as_ref())
    }

    /// The pages in this session's slots, session-wide first, then left and right, each with its slot's pane.
    #[must_use]
    pub fn html_overlays(&self) -> Vec<(Option<OverlayPane>, &HtmlOverlay)> {
        let mut pages: Vec<(Option<OverlayPane>, &HtmlOverlay)> =
            self.overlay.page().map(|page| (None, page)).into_iter().collect();
        for pane in [OverlayPane::Left, OverlayPane::Right] {
            if let Some(page) = self.pane_overlay(pane).and_then(|overlay| overlay.html.as_ref()) {
                pages.push((Some(pane), page));
            }
        }
        pages
    }

    /// The slot holding page `id` here: `Some(None)` for the session-wide one.
    #[must_use]
    pub fn html_overlay_pane(&self, id: Uuid) -> Option<Option<OverlayPane>> {
        self.html_overlays()
            .into_iter()
            .find(|(_, page)| page.id == id)
            .map(|(pane, _)| pane)
    }

    /// The page in the slot `pane` addresses (upstream's private `htmlOverlay(_:pane:)`).
    ///
    /// # Errors
    ///
    /// No program or page holds the slot, or a program does.
    pub fn html_overlay_in(&self, pane: Option<OverlayPane>) -> Result<&HtmlOverlay, HtmlOverlayCommandFailure> {
        let occupied = match pane {
            Some(pane) => self.pane_overlay(pane).is_some(),
            None => self.overlay.active && !self.overlay.hud_active(),
        };
        if !occupied {
            return Err(HtmlOverlayCommandFailure::NoOverlay);
        }
        let shown = match pane {
            Some(pane) => self.pane_overlay(pane).and_then(|overlay| overlay.html.as_ref()),
            None => self.overlay.html.as_ref(),
        };
        shown.ok_or(HtmlOverlayCommandFailure::NotHtml)
    }

    /// Puts `overlay` in the slot `pane` names, which must be free: a pane slot gets a fresh generation and holds the
    /// session-wide cover open now; the session-wide slot takes the size clamped into 1...100, its points only beside
    /// a percent. The open's checks are the caller's.
    pub(crate) fn install_html_overlay(
        &mut self,
        pane: Option<OverlayPane>,
        overlay: HtmlOverlay,
        size: OverlaySize,
        background_color: Option<String>,
    ) {
        if let Some(pane) = pane {
            let generation = self.pane_overlays.next_generation();
            let holds_cover = self.cover_hold();
            let slot = self.pane_overlays.slot_mut(pane);
            slot.exit_code = None;
            slot.overlay = Some(PaneOverlay {
                command: String::new(),
                cwd: None,
                background_color,
                wait: false,
                generation,
                code_file: None,
                html: Some(overlay),
                hidden: false,
                holds_cover,
            });
            self.remote_overlays.clear_failure(Some(pane));
            return;
        }
        let slot = &mut self.overlay;
        slot.generation += 1;
        slot.exit_code = None;
        slot.code_file = None;
        slot.command = None;
        slot.cwd = None;
        slot.wait = false;
        slot.size_percent = size.percent.map(|percent| percent.clamp(1, 100));
        slot.points = if slot.size_percent.is_some() {
            size.points
        } else {
            OverlayPointSize::default()
        };
        slot.background_color = background_color;
        slot.html = Some(overlay);
        slot.active = true;
        self.remote_overlays.clear_failure(None);
    }

    /// Applies `change` to page `id` in whichever slot of this session holds it; false when none does.
    pub fn update_html_overlay(&mut self, id: Uuid, change: impl FnOnce(&mut HtmlOverlay)) -> bool {
        if let Some(page) = self.overlay.html.as_mut().filter(|page| page.id == id) {
            change(page);
            return true;
        }
        for pane in [OverlayPane::Left, OverlayPane::Right] {
            let page = self
                .pane_overlays
                .slot_mut(pane)
                .overlay
                .as_mut()
                .and_then(|overlay| overlay.html.as_mut())
                .filter(|page| page.id == id);
            if let Some(page) = page {
                change(page);
                return true;
            }
        }
        false
    }
}

impl AppStore {
    /// Opens a page in the session-wide slot (`pane` `None`) or one pane's slot, with the slot rules of
    /// [`AppStore::open_overlay`]/[`AppStore::open_pane_overlay`] (upstream's `openHtmlOverlay`): a HUD yields the
    /// session-wide slot, a program or page refuses; a pane slot needs its pane laid out. The size is clamped into
    /// 1...100. The caller registers the page's outcome.
    ///
    /// # Errors
    ///
    /// Why it refused, with nothing changed.
    pub fn open_html_overlay(
        &mut self,
        id: Uuid,
        pane: Option<OverlayPane>,
        overlay: HtmlOverlay,
        size_percent: Option<i64>,
        background_color: Option<String>,
    ) -> Result<(), HtmlOverlayOpenFailure> {
        self.open_html_overlay_sized(id, pane, overlay, OverlaySize::percent(size_percent), background_color)
    }

    /// [`AppStore::open_html_overlay`] with the session-wide slot's point sizes beside its percent: kept only beside
    /// one, never in a pane slot (upstream's `openHtmlOverlay` `points`).
    ///
    /// # Errors
    ///
    /// Why it refused, with nothing changed.
    pub fn open_html_overlay_sized(
        &mut self,
        id: Uuid,
        pane: Option<OverlayPane>,
        overlay: HtmlOverlay,
        size: OverlaySize,
        background_color: Option<String>,
    ) -> Result<(), HtmlOverlayOpenFailure> {
        let session = self.session_mut(id).ok_or(HtmlOverlayOpenFailure::UnknownSession)?;
        if let Some(pane) = pane {
            if session.pane_overlay(pane).is_some() {
                return Err(HtmlOverlayOpenFailure::AlreadyOpen);
            }
            if !session.renders_overlay_pane(pane) {
                return Err(HtmlOverlayOpenFailure::PaneNotVisible);
            }
        } else {
            if session.overlay.hud_active() {
                session.overlay.close();
            }
            if session.overlay.active {
                return Err(HtmlOverlayOpenFailure::AlreadyOpen);
            }
        }
        session.install_html_overlay(pane, overlay, size, background_color);
        self.schedule_tree_changed();
        Ok(())
    }

    /// The page in the slot `pane` of session `id` addresses (upstream's `htmlOverlayCommandFailure` when it fails).
    ///
    /// # Errors
    ///
    /// An unknown session, an empty slot, or a slot holding a program.
    pub fn html_overlay(&self, id: Uuid, pane: Option<OverlayPane>) -> Result<&HtmlOverlay, HtmlOverlayCommandFailure> {
        self.session(id)
            .ok_or(HtmlOverlayCommandFailure::UnknownSession)?
            .html_overlay_in(pane)
    }

    /// Asks the frontend to reload the page, keeping the slot, its identity and the web view (upstream's
    /// `reloadHtmlOverlay`): the revision moves, the page is loading again, and a reload of the original forgets
    /// where the user had navigated.
    ///
    /// # Errors
    ///
    /// As [`AppStore::html_overlay`].
    pub fn reload_html_overlay(
        &mut self,
        id: Uuid,
        pane: Option<OverlayPane>,
        target: HtmlReloadTarget,
    ) -> Result<(), HtmlOverlayCommandFailure> {
        let page_id = self.html_overlay(id, pane)?.id;
        if let Some(session) = self.session_mut(id) {
            session.update_html_overlay(page_id, |page| reload(page, target));
        }
        Ok(())
    }

    /// Sets whether the log page in the slot `pane` of session `id` addresses follows its file, to what `following`
    /// makes of its current state, and answers the state left (upstream's `setTextLogFollowing(_:pane:mode:)`). A change
    /// emits `tree.changed`.
    ///
    /// # Errors
    ///
    /// The page lookup's refusal, a replica, or a page that is not a log page; nothing changes.
    pub fn set_text_log_following(
        &mut self,
        id: Uuid,
        pane: Option<OverlayPane>,
        following: impl FnOnce(bool) -> bool,
    ) -> Result<bool, TextLogFollowFailure> {
        let shown = self.html_overlay(id, pane).map_err(TextLogFollowFailure::Page)?;
        if shown.replica_of.is_some() {
            return Err(TextLogFollowFailure::Replica);
        }
        let state = shown.log_state.ok_or(TextLogFollowFailure::NotLog)?;
        let (page_id, wanted) = (shown.id, following(state.following));
        if wanted != state.following {
            if let Some(session) = self.session_mut(id) {
                session.update_html_overlay(page_id, |page| {
                    set_following(page, wanted);
                });
            }
            self.schedule_tree_changed();
        }
        Ok(wanted)
    }
}

/// Records `following` on a log page; false when it is no log page or already says so.
fn set_following(page: &mut HtmlOverlay, following: bool) -> bool {
    match page.log_state.as_mut() {
        Some(state) if state.following != following => {
            state.following = following;
            true
        }
        _ => false,
    }
}

/// One reload of `page` (upstream's `reloadHtmlOverlay` change).
fn reload(page: &mut HtmlOverlay, target: HtmlReloadTarget) {
    page.reload_revision += 1;
    page.reload_target = target;
    page.load_state = HtmlLoadState::Loading;
    page.load_error = None;
    if target == HtmlReloadTarget::Original {
        page.current = None;
    }
}

impl WindowLibrary {
    /// Where page `id` sits in a visible session of an open window (upstream's `htmlOverlaySlot`); `None` once the
    /// page left its slot, and while its session sits in an undoable close.
    #[must_use]
    pub fn html_overlay_slot(&self, id: Uuid) -> Option<HtmlPageSlot> {
        self.open_ids().into_iter().find_map(|window| {
            self.store(window)?.sessions().find_map(|session| {
                session.html_overlay_pane(id).map(|pane| HtmlPageSlot {
                    window,
                    session: session.id(),
                    pane,
                })
            })
        })
    }

    /// The ids of every page an open window holds, in its sessions and in its undoable closes, whose web views go on.
    #[must_use]
    pub fn live_html_overlays(&self) -> Vec<Uuid> {
        self.live_html_pages().into_iter().map(|(_, page)| page.id).collect()
    }

    /// [`WindowLibrary::live_html_overlays`]' pages themselves, each beside the window holding it.
    #[must_use]
    pub fn live_html_pages(&self) -> Vec<(Uuid, &HtmlOverlay)> {
        let mut pages = Vec::new();
        for window in self.open_ids() {
            let Some(store) = self.store(window) else { continue };
            let pending = store.pending_close_members();
            let sessions = store.sessions().chain(pending.iter().map(|member| member.session));
            pages.extend(
                sessions.flat_map(|session| session.html_overlays().into_iter().map(move |(_, page)| (window, page))),
            );
        }
        pages
    }

    /// The pages of [`WindowLibrary::live_html_pages`] a window's snapshot writes ([`HtmlOverlay::persisted`]), each
    /// as (window, page, what the snapshot writes for it).
    #[must_use]
    pub(crate) fn live_persisted_pages(&self) -> Vec<(Uuid, Uuid, PageSnapshot)> {
        let mut pages = Vec::new();
        for window in self.open_ids() {
            let Some(store) = self.store(window) else { continue };
            let pending = store.pending_close_members();
            let sessions = store.sessions().chain(pending.iter().map(|member| member.session));
            pages.extend(sessions.flat_map(|session| {
                persisted_pages(session)
                    .into_iter()
                    .map(move |(page, snapshot)| (window, page, snapshot))
            }));
        }
        pages
    }

    /// Parks every page a presenter can show that sits in an open window's own slots ([`park_local_pages`]): a
    /// presenter-only origin's sweep over what a restore opened, closing what no presenter can show. A window that
    /// parked or closed one asks for a save, so its file writes the page as the viewer's or no more. The pages parked.
    pub(crate) fn park_local_pages(&mut self) -> Vec<Uuid> {
        let mut parked = Vec::new();
        for window in self.open_ids() {
            let Some(store) = self.store_mut(window) else { continue };
            let sessions: Vec<Uuid> = store.sessions().map(Session::id).collect();
            let mut changed = false;
            for id in sessions {
                if let Some(session) = store.session_mut(id) {
                    let swept = park_local_pages(session);
                    changed |= swept.dropped || !swept.parked.is_empty();
                    parked.extend(swept.parked);
                }
            }
            if changed {
                store.schedule_tree_changed();
                store.schedule_save();
            }
        }
        parked
    }

    /// Applies `change` to page `id` wherever its slot now is, a session an undoable close holds included (upstream's
    /// `updateHtmlOverlay`); false once the page left its slot.
    pub fn update_html_overlay(&mut self, id: Uuid, change: impl FnOnce(&mut HtmlOverlay)) -> bool {
        let Some(session) = self.session_holding_page(id) else {
            return false;
        };
        self.held_session_mut(session)
            .is_some_and(|session| session.update_html_overlay(id, change))
    }

    /// Records what the web view reported for page `id` (upstream's `setHtmlLoadState`); a no-op once it left its slot.
    pub fn set_html_load_state(&mut self, id: Uuid, state: HtmlLoadState, error: Option<String>) -> bool {
        self.update_html_overlay(id, |page| {
            page.load_state = state;
            page.load_error = error;
        })
    }

    /// Records what the web view shows now (upstream's `setHtmlPage`).
    pub fn set_html_page(&mut self, id: Uuid, info: Option<HtmlPageInfo>) -> bool {
        self.update_html_overlay(id, |page| page.current = info)
    }

    /// Records whether log page `id` follows its file, as the page reported it after a scroll, wherever its slot now is
    /// (upstream's `setTextLogFollowing(_:following:)`). A change emits `tree.changed`; a page that is not a log page is
    /// left alone. Whether it changed.
    pub fn set_text_log_following(&mut self, id: Uuid, following: bool) -> bool {
        let mut changed = false;
        self.update_html_overlay(id, |page| changed = set_following(page, following));
        if changed && let Some(session) = self.session_holding_page(id) {
            self.schedule_tree_changed_for(session);
        }
        changed
    }

    /// Records what the app last read for log page `id` (upstream's `setTextLogReport`): written only on a change and
    /// never emitted, so a follower's ticks wake no hook; a page that is not a log page is left alone.
    pub fn set_text_log_report(&mut self, id: Uuid, report: TextLogReport) {
        self.update_html_overlay(id, |page| {
            if let Some(state) = page.log_state.as_mut() {
                state.report = report;
            }
        });
    }

    /// Sets text page `id`'s query wherever its slot now is, an empty one closing the search, from control or the page's
    /// own field (upstream's `setTextSearch`). A change moves the search revision, clears the report and emits
    /// `tree.changed`; the new revision, `None` for no change or a page that is not a text page.
    pub fn set_text_search(&mut self, id: Uuid, query: &str) -> Option<u64> {
        let mut revision = None;
        self.update_html_overlay(id, |page| {
            let current = page.text_search.as_ref().map_or("", |search| search.query.as_str());
            if !matches!(page.source, HtmlSource::Text { .. }) || current == query {
                return;
            }
            page.search_revision += 1;
            page.text_search = (!query.is_empty()).then(|| TextSearch::new(query));
            revision = Some(page.search_revision);
        });
        if revision.is_some()
            && let Some(session) = self.session_holding_page(id)
        {
            self.schedule_tree_changed_for(session);
        }
        revision
    }

    /// Records text page `id`'s matches and current match for `query` at `revision` (upstream's
    /// `setTextSearchReport`), dropped when either is not the model's, so counts never land under another query. Never
    /// emitted, like [`WindowLibrary::set_text_log_report`].
    pub fn set_text_search_report(&mut self, id: Uuid, revision: u64, query: &str, count: SearchCount) {
        self.update_html_overlay(id, |page| {
            if page.search_revision != revision {
                return;
            }
            if let Some(search) = page.text_search.as_mut().filter(|search| search.query == query) {
                search.matches = count.matches;
                search.current = count.current;
            }
        });
    }

    /// Records diff page `id`'s count and current change, `None` clearing it while a fresh read is under way (upstream's
    /// `setTextChangesReport`). Only a text page takes it; never emitted, like [`WindowLibrary::set_text_search_report`].
    pub fn set_text_changes_report(&mut self, id: Uuid, changes: Option<TextChanges>) {
        self.update_html_overlay(id, |page| {
            if matches!(page.source, HtmlSource::Text { .. }) {
                page.text_changes = changes;
            }
        });
    }

    /// Reloads page `id` wherever it sits (the toolbar's reload; upstream's `HtmlOverlayRegistry.reload(_:target:)`).
    pub fn reload_html_overlay_by_id(&mut self, id: Uuid, target: HtmlReloadTarget) -> bool {
        let Some(slot) = self.html_overlay_slot(id) else {
            return false;
        };
        self.store_mut(slot.window)
            .is_some_and(|store| store.reload_html_overlay(slot.session, slot.pane, target).is_ok())
    }

    /// Closes page `id` wherever its slot now is (upstream's `closeHtmlOverlay`): the toolbar's close button and a
    /// page's own `window.close()`, whose view cannot trust the pane it was built for after a swap. False while its
    /// session sits in an undoable close or once it left its slot.
    pub fn close_html_overlay(&mut self, id: Uuid) -> bool {
        let Some(slot) = self.html_overlay_slot(id) else {
            return false;
        };
        let Some(store) = self.store_mut(slot.window) else {
            return false;
        };
        match slot.pane {
            Some(pane) => store.close_pane_overlay(slot.session, pane),
            None => store.close_overlay(slot.session),
        }
    }

    /// The session holding page `id` in an open window, visible or in an undoable close.
    fn session_holding_page(&self, id: Uuid) -> Option<Uuid> {
        self.open_ids().into_iter().find_map(|window| {
            let store = self.store(window)?;
            let pending = store.pending_close_members();
            let mut sessions = store.sessions().chain(pending.iter().map(|member| member.session));
            sessions
                .find(|session| session.html_overlay_pane(id).is_some())
                .map(Session::id)
        })
    }
}
