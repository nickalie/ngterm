//! Hidden overlays, ported from upstream's `AppStoreOverlayVisibilityTests`, `SessionOverlayVisibilityTests` and the
//! hidden-overlay cases of `AppStorePaneSwapTests`, `AppStoreTreeProjectionTests`, `DashboardCoverTests` and
//! `TerminalZoomTests`. A `SpySurface` there is a [`FakeSurface`] here; a `tree.changed` is counted as upstream counts
//! the coalesced events after a flush.

use ngterm_host::fakes::FakeSurface;
use ngterm_proto::EventKind;
use uuid::Uuid;

use super::support::add;
use crate::dashboard::DashboardCover;
use crate::html::{HtmlLoadState, HtmlOverlay, HtmlPageFlags, HtmlSource};
use crate::hud::{HudPanelSize, HudSpec};
use crate::model::{
    AppStore, HudOpening, OverlayPane, OverlayProgram, OverlayVisibilityFailure, PaneOverlayOpenFailure,
    RemoteOverlaySlot, Session, StatusPane, VisibleCover, WindowLibrary,
};
use crate::projection;
use crate::zoom::{SurfaceId, ZoomSurface};

const LEFT: OverlayPane = OverlayPane::Left;
const RIGHT: OverlayPane = OverlayPane::Right;

/// A store with one selected session whose split shows, both panes with a terminal and the right one focused
/// (upstream's `makeFixture`), its outbox drained.
pub(super) fn split_store() -> (AppStore, Uuid) {
    let mut store = AppStore::new();
    let ws = store.add_workspace("work");
    let id = add(&mut store, ws, "/tmp");
    store.set_split_visibility(id, true, None);
    let session = store.session_mut(id).unwrap();
    session.surface.replace(Box::new(FakeSurface::new("L")));
    session.split_surface.replace(Box::new(FakeSurface::new("R")));
    store.take_outbox();
    (store, id)
}

pub(super) fn program(command: &str) -> OverlayProgram {
    OverlayProgram {
        command: command.to_owned(),
        ..OverlayProgram::default()
    }
}

pub(super) fn page(path: &str) -> HtmlOverlay {
    HtmlOverlay::new(
        HtmlSource::File {
            path: path.to_owned(),
            grant_root: None,
        },
        HtmlPageFlags::default(),
    )
}

/// Both pane overlays, then the session-wide program, which a pane overlay opened after it would hold (upstream's
/// `openAll`).
fn open_all(store: &mut AppStore, id: Uuid) {
    assert_eq!(store.open_pane_overlay(id, LEFT, program("htop")), Ok(()));
    assert_eq!(store.open_pane_overlay(id, RIGHT, program("top")), Ok(()));
    assert!(store.open_overlay(id, program("less")));
}

pub(super) fn session(store: &AppStore, id: Uuid) -> &Session {
    store.session(id).expect("the session exists")
}

pub(super) fn names(store: &AppStore, id: Uuid) -> Vec<&'static str> {
    session(store, id).hidden_overlay_names()
}

fn tree_changes(store: &mut AppStore) -> usize {
    usize::from(
        store
            .take_outbox()
            .events
            .iter()
            .any(|event| event.kind == EventKind::TreeChanged),
    )
}

/// A HUD in the session-wide slot.
pub(super) fn open_hud(store: &mut AppStore, id: Uuid, dir: &tempfile::TempDir) {
    let opening = HudOpening {
        file: dir.path().join("hud"),
        size: HudPanelSize::new(40, 10),
        pane_identity: None,
        font_size: None,
    };
    assert!(store.open_hud(id, HudSpec::new("deploying"), opening));
}

// MARK: - the store

#[test]
fn hiding_and_showing_the_session_acts_on_every_slot_and_keeps_occupants() {
    let (mut store, id) = split_store();
    open_all(&mut store, id);
    assert_eq!(store.set_overlays_hidden(id, None, true), Ok(()));
    assert_eq!(names(&store, id), ["session", "left", "right"]);
    let found = session(&store, id);
    assert!(found.overlay.active);
    assert_eq!(found.pane_overlays.open_panes(), [LEFT, RIGHT]);
    assert!(!found.cover_overlay_visible());

    assert_eq!(store.set_overlays_hidden(id, None, false), Ok(()));
    assert!(names(&store, id).is_empty());
    assert!(session(&store, id).cover_overlay_visible());
}

#[test]
fn a_pane_scope_touches_only_that_pane() {
    for pane in [LEFT, RIGHT] {
        let (mut store, id) = split_store();
        open_all(&mut store, id);
        assert_eq!(store.set_overlays_hidden(id, Some(pane), true), Ok(()));
        assert_eq!(names(&store, id), [pane.wire_name()]);
    }
}

#[test]
fn tree_changed_fires_only_on_an_actual_transition() {
    let (mut store, id) = split_store();
    open_all(&mut store, id);
    store.take_outbox();
    store.set_overlays_hidden(id, Some(LEFT), true).unwrap();
    assert_eq!(tree_changes(&mut store), 1);
    store.set_overlays_hidden(id, Some(LEFT), true).unwrap();
    assert_eq!(tree_changes(&mut store), 0);
    store.set_overlays_hidden(id, Some(LEFT), false).unwrap();
    assert_eq!(tree_changes(&mut store), 1);
    assert_eq!(store.set_overlays_hidden(id, Some(LEFT), false), Ok(()));
    assert_eq!(tree_changes(&mut store), 0);
}

#[test]
fn toggle_hides_everything_while_any_is_shown_then_shows_all() {
    let (mut store, id) = split_store();
    open_all(&mut store, id);
    store.set_overlays_hidden(id, Some(LEFT), true).unwrap();
    assert_eq!(store.toggle_overlays_hidden(id, None), Ok(()));
    assert_eq!(names(&store, id), ["session", "left", "right"]);
    assert_eq!(store.toggle_overlays_hidden(id, None), Ok(()));
    assert!(names(&store, id).is_empty());
}

#[test]
fn toggle_with_a_pane_flips_only_that_pane() {
    let (mut store, id) = split_store();
    open_all(&mut store, id);
    assert_eq!(store.toggle_overlays_hidden(id, Some(RIGHT)), Ok(()));
    assert_eq!(names(&store, id), ["right"]);
    assert_eq!(store.toggle_overlays_hidden(id, Some(RIGHT)), Ok(()));
    assert!(names(&store, id).is_empty());
}

#[test]
fn refuses_an_unknown_session_and_an_empty_scope() {
    let (mut store, id) = split_store();
    let unknown = Uuid::new_v4();
    assert_eq!(
        store.set_overlays_hidden(unknown, None, true),
        Err(OverlayVisibilityFailure::UnknownSession)
    );
    assert_eq!(
        store.toggle_overlays_hidden(unknown, None),
        Err(OverlayVisibilityFailure::UnknownSession)
    );
    assert_eq!(
        store.set_overlays_hidden(id, None, true),
        Err(OverlayVisibilityFailure::NoOverlay)
    );
    assert_eq!(
        store.toggle_overlays_hidden(id, Some(LEFT)),
        Err(OverlayVisibilityFailure::NoOverlay)
    );
    assert_eq!(tree_changes(&mut store), 0);
}

#[test]
fn a_hud_alone_has_nothing_to_hide() {
    let (mut store, id) = split_store();
    let dir = tempfile::tempdir().unwrap();
    open_hud(&mut store, id, &dir);
    assert_eq!(
        store.toggle_overlays_hidden(id, None),
        Err(OverlayVisibilityFailure::NoOverlay)
    );
    assert!(!session(&store, id).overlay.hidden);
}

fn reservation(pane: Option<OverlayPane>) -> RemoteOverlaySlot {
    RemoteOverlaySlot {
        job: "job".to_owned(),
        pane,
        owner: 1,
        size_percent: None,
        wait: false,
        ended: false,
        page: None,
    }
}

#[test]
fn the_session_scope_skips_a_remote_reservation_and_acts_on_local_slots() {
    let (mut store, id) = split_store();
    assert!(store.reserve_remote_overlay(id, reservation(Some(RIGHT))));
    assert_eq!(
        store.toggle_overlays_hidden(id, None),
        Err(OverlayVisibilityFailure::Remote)
    );
    assert_eq!(
        store.toggle_overlays_hidden(id, Some(RIGHT)),
        Err(OverlayVisibilityFailure::Remote)
    );
    assert_eq!(store.open_pane_overlay(id, LEFT, program("htop")), Ok(()));
    assert_eq!(store.toggle_overlays_hidden(id, None), Ok(()));
    assert_eq!(names(&store, id), ["left"]);
}

#[test]
fn closing_the_session_slot_clears_its_bit_and_a_new_occupant_starts_shown() {
    let (mut store, id) = split_store();
    assert!(store.open_overlay(id, program("less")));
    store.set_overlays_hidden(id, None, true).unwrap();
    assert!(!store.open_overlay(id, program("vim")));
    assert!(store.close_overlay(id));
    assert!(!session(&store, id).overlay.hidden);
    assert!(store.open_overlay(id, program("vim")));
    assert!(session(&store, id).cover_overlay_visible());
}

#[test]
fn closing_a_pane_slot_clears_its_bit_and_a_reopen_starts_shown() {
    let (mut store, id) = split_store();
    assert_eq!(store.open_pane_overlay(id, LEFT, program("htop")), Ok(()));
    store.set_overlays_hidden(id, Some(LEFT), true).unwrap();
    assert_eq!(
        store.open_pane_overlay(id, LEFT, program("top")),
        Err(PaneOverlayOpenFailure::AlreadyOpen)
    );
    assert!(store.close_pane_overlay(id, LEFT));
    assert_eq!(store.open_pane_overlay(id, LEFT, program("top")), Ok(()));
    assert!(session(&store, id).pane_overlay_visible(LEFT));
}

/// Upstream's `aWaitProgramExitKeepsTheSlotHidden`: here a held program's exit leaves the slot as it is (its wrapper
/// writes the status, the frontend keeps the terminal), so the slot stays occupied and hidden.
#[test]
fn a_wait_program_exit_keeps_the_slot_hidden() {
    let (mut store, id) = split_store();
    let held = OverlayProgram {
        wait: true,
        ..program("make")
    };
    assert!(store.open_overlay(id, held));
    store.set_overlays_hidden(id, None, true).unwrap();
    let file = session(&store, id).overlay.code_file.clone().unwrap();
    std::fs::write(&file, "2\n").unwrap();
    let found = session(&store, id);
    assert!(found.overlay.hidden && found.overlay.active);
    // an in-place change of the occupant keeps the bit too
    assert!(store.resize_overlay(id, Some(40)));
    assert!(session(&store, id).overlay.hidden);
    assert!(store.close_overlay(id));
    assert_eq!(session(&store, id).overlay.exit_code, Some(2));
}

#[test]
fn a_page_update_keeps_its_slot_hidden() {
    for pane in [None, Some(LEFT)] {
        let (mut store, id) = split_store();
        let shown = page("/tmp/p.html");
        let page_id = shown.id;
        assert_eq!(store.open_html_overlay(id, pane, shown, None, None), Ok(()));
        store.set_overlays_hidden(id, pane, true).unwrap();
        let updated = store
            .session_mut(id)
            .unwrap()
            .update_html_overlay(page_id, |page| page.load_state = HtmlLoadState::Loaded);
        assert!(updated);
        let found = session(&store, id);
        assert!(found.overlay_hidden(pane), "{pane:?}");
        let node = projection::session_node(found, true);
        let page_node = &node.html_overlays.unwrap()[0];
        assert_eq!(page_node.hidden, Some(true), "{pane:?}");
        assert_eq!(page_node.state, "loaded");
    }
}

#[test]
fn the_visible_cover_changes_only_with_what_owns_the_keyboard() {
    let (mut store, id) = split_store();
    assert_eq!(session(&store, id).visible_cover(), None);
    assert!(store.open_overlay(id, program("less")));
    assert_eq!(session(&store, id).visible_cover(), Some(VisibleCover::Session));
    store.set_overlays_hidden(id, None, true).unwrap();
    assert_eq!(session(&store, id).visible_cover(), None);
    store.set_overlays_hidden(id, None, false).unwrap();
    assert_eq!(session(&store, id).visible_cover(), Some(VisibleCover::Session));
    store.set_overlays_hidden(id, None, true).unwrap();
    assert!(store.close_overlay(id));
    assert_eq!(
        session(&store, id).visible_cover(),
        None,
        "closing a hidden overlay leaves the terminal in charge"
    );
}

// regression upstream: hiding or closing the unfocused pane's overlay refocused the active pane and stole Find's keys
#[test]
fn the_other_panes_overlay_never_changes_the_visible_cover() {
    let (mut store, id) = split_store();
    store.set_split_focus(id, false);
    assert_eq!(store.open_pane_overlay(id, RIGHT, program("htop")), Ok(()));
    assert_eq!(session(&store, id).visible_cover(), None);
    store.set_overlays_hidden(id, Some(RIGHT), true).unwrap();
    assert_eq!(session(&store, id).visible_cover(), None);
    store.set_overlays_hidden(id, Some(RIGHT), false).unwrap();
    assert_eq!(session(&store, id).visible_cover(), None);
    assert!(store.close_pane_overlay(id, RIGHT));
    assert_eq!(session(&store, id).visible_cover(), None);

    assert_eq!(store.open_pane_overlay(id, LEFT, program("htop")), Ok(()));
    assert_eq!(session(&store, id).visible_cover(), Some(VisibleCover::FocusedPane));
    store.set_overlays_hidden(id, Some(LEFT), true).unwrap();
    assert_eq!(session(&store, id).visible_cover(), None);
}

// regression upstream: a pane overlay changing under the scratch refocused the scratch and stole Find's keys
#[test]
fn a_pane_overlay_under_the_scratch_never_changes_the_visible_cover() {
    let (mut store, id) = split_store();
    store.set_split_focus(id, false);
    store.session_mut(id).unwrap().scratch_active = true;
    assert_eq!(store.open_pane_overlay(id, LEFT, program("htop")), Ok(()));
    assert_eq!(session(&store, id).visible_cover(), None);
    store.set_overlays_hidden(id, Some(LEFT), true).unwrap();
    assert_eq!(session(&store, id).visible_cover(), None);
    assert!(store.close_pane_overlay(id, LEFT));
    assert_eq!(session(&store, id).visible_cover(), None);
    assert!(store.open_overlay(id, program("less")));
    assert_eq!(
        session(&store, id).visible_cover(),
        Some(VisibleCover::Session),
        "a session-wide overlay draws above the scratch"
    );
}

/// The seeded library's window store and its session, split as [`split_store`] splits.
fn library() -> (WindowLibrary, Uuid, Uuid) {
    let mut library = WindowLibrary::seeded("/home/u");
    let window = library.active_window_id().unwrap();
    let store = library.store_mut(window).unwrap();
    let id = store.active_session().unwrap().id();
    store.set_split_visibility(id, true, None);
    let found = store.session_mut(id).unwrap();
    found.surface.replace(Box::new(FakeSurface::new("L")));
    found.split_surface.replace(Box::new(FakeSurface::new("R")));
    (library, window, id)
}

#[test]
fn a_pages_own_hide_uses_the_page_scope() {
    let (mut library, window, id) = library();
    let pane_page = page("/tmp/pane.html");
    let page_id = pane_page.id;
    let store = library.store_mut(window).unwrap();
    assert_eq!(store.open_html_overlay(id, Some(RIGHT), pane_page, None, None), Ok(()));
    assert!(store.open_overlay(id, program("less")));
    assert!(library.hide_html_overlay(page_id));
    let store = library.store(window).unwrap();
    assert_eq!(names(store, id), ["right"], "a pane page hides its own pane only");
    assert!(library.store_mut(window).unwrap().close_pane_overlay(id, RIGHT));
    assert!(
        !library.hide_html_overlay(page_id),
        "a page that left its slot has nothing to hide"
    );
}

#[test]
fn a_session_wide_pages_own_hide_hides_the_session() {
    let (mut library, window, id) = library();
    let shown = page("/tmp/session.html");
    let page_id = shown.id;
    let store = library.store_mut(window).unwrap();
    assert_eq!(store.open_pane_overlay(id, LEFT, program("htop")), Ok(()));
    assert_eq!(store.open_html_overlay(id, None, shown, None, None), Ok(()));
    assert!(library.hide_html_overlay(page_id));
    assert_eq!(names(library.store(window).unwrap(), id), ["session", "left"]);
}

// MARK: - the session's predicates

/// What the session-wide slot holds in [`the_visible_predicates_subtract_the_hidden_bit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Occupant {
    Empty,
    FullProgram,
    FloatingProgram,
    Page,
    Hud,
}

#[test]
fn the_visible_predicates_subtract_the_hidden_bit() {
    use Occupant as O;
    for occupant in [O::Empty, O::FullProgram, O::FloatingProgram, O::Page, O::Hud] {
        for hidden in [false, true] {
            let (mut store, id) = split_store();
            let dir = tempfile::tempdir().unwrap();
            match occupant {
                O::Empty => {}
                O::FullProgram => assert!(store.open_overlay(id, program("less"))),
                O::FloatingProgram => assert!(store.open_overlay(
                    id,
                    OverlayProgram {
                        size_percent: Some(60),
                        ..program("less")
                    }
                )),
                O::Page => assert_eq!(
                    store.open_html_overlay(id, None, page("/tmp/p.html"), None, None),
                    Ok(())
                ),
                O::Hud => open_hud(&mut store, id, &dir),
            }
            store.session_mut(id).unwrap().overlay.hidden = hidden;
            let found = session(&store, id);
            let case = format!("{occupant:?} hidden={hidden}");
            assert_eq!(
                found.cover_overlay_visible(),
                found.cover_overlay_active() && !hidden,
                "{case}"
            );
            assert_eq!(
                found.program_overlay_visible(),
                found.overlay.program_active() && !hidden,
                "{case}"
            );
            assert_eq!(
                found.html_overlay_visible(),
                found.html_overlay_active() && !hidden,
                "{case}"
            );
            assert_eq!(
                found.full_overlay_visible(),
                found.full_overlay_active() && !hidden,
                "{case}"
            );
            assert_eq!(
                found.cover_overlay_active(),
                matches!(occupant, O::FullProgram | O::FloatingProgram | O::Page),
                "{case}"
            );
            assert_eq!(found.overlay_hidden(None), hidden, "{case}");
        }
    }
}

#[test]
fn pane_visibility_follows_its_overlay_hidden_bit() {
    for pane in [LEFT, RIGHT] {
        let (mut store, id) = split_store();
        assert!(!session(&store, id).pane_overlay_visible(pane));
        assert!(!session(&store, id).overlay_hidden(Some(pane)));
        assert_eq!(store.open_pane_overlay(id, pane, program("htop")), Ok(()));
        assert!(session(&store, id).pane_overlay_visible(pane));
        store.session_mut(id).unwrap().set_overlay_hidden(true, Some(pane));
        let found = session(&store, id);
        assert!(!found.pane_overlay_visible(pane));
        assert!(found.overlay_hidden(Some(pane)));
        assert_eq!(
            found.pane_overlay(pane).map(|overlay| overlay.command.as_str()),
            Some("htop")
        );
        assert!(!found.overlay_hidden(None));
    }
}

#[test]
fn any_overlay_shown_counts_only_unhidden_slots_in_scope() {
    let (mut store, id) = split_store();
    assert!(!session(&store, id).any_overlay_shown(None));
    assert!(store.open_overlay(id, program("less")));
    assert_eq!(store.open_pane_overlay(id, RIGHT, program("htop")), Ok(()));
    assert!(session(&store, id).any_overlay_shown(None));

    store.session_mut(id).unwrap().set_overlay_hidden(true, None);
    let found = session(&store, id);
    assert!(found.any_overlay_shown(None));
    assert!(found.any_overlay_shown(Some(RIGHT)));
    assert!(!found.any_overlay_shown(Some(LEFT)));

    store.session_mut(id).unwrap().set_overlay_hidden(true, Some(RIGHT));
    let found = session(&store, id);
    assert!(!found.any_overlay_shown(None));
    assert!(!found.any_overlay_shown(Some(RIGHT)));
}

#[test]
fn hideable_slots_skip_a_hud_and_empty_panes() {
    let (mut store, id) = split_store();
    let dir = tempfile::tempdir().unwrap();
    open_hud(&mut store, id, &dir);
    assert_eq!(store.open_pane_overlay(id, RIGHT, program("htop")), Ok(()));
    let found = session(&store, id);
    assert_eq!(found.hideable_overlay_slots(None), [Some(RIGHT)]);
    assert!(found.hideable_overlay_slots(Some(LEFT)).is_empty());
    assert_eq!(found.hideable_overlay_slots(Some(RIGHT)), [Some(RIGHT)]);
}

#[test]
fn hideable_slots_list_session_then_left_then_right() {
    let (mut store, id) = split_store();
    assert!(store.open_overlay(id, program("less")));
    assert_eq!(store.open_pane_overlay(id, RIGHT, program("a")), Ok(()));
    assert_eq!(store.open_pane_overlay(id, LEFT, program("b")), Ok(()));
    assert_eq!(
        session(&store, id).hideable_overlay_slots(None),
        [None, Some(LEFT), Some(RIGHT)]
    );
}

#[test]
fn hidden_overlay_names_list_only_hidden_slots_in_order() {
    let (mut store, id) = split_store();
    open_all(&mut store, id);
    assert!(names(&store, id).is_empty());
    let found = store.session_mut(id).unwrap();
    found.set_overlay_hidden(true, Some(RIGHT));
    found.set_overlay_hidden(true, None);
    assert_eq!(names(&store, id), ["session", "right"]);
}

#[test]
fn a_promoted_pane_overlay_keeps_its_hidden_bit() {
    let (mut store, id) = split_store();
    assert_eq!(store.open_pane_overlay(id, RIGHT, program("htop")), Ok(()));
    store.set_overlays_hidden(id, Some(RIGHT), true).unwrap();
    // the primary's shell exits: the survivor and its overlay move left
    store.close_primary_pane(id);
    let found = session(&store, id);
    assert!(found.overlay_hidden(Some(LEFT)));
    assert_eq!(
        found.pane_overlay(LEFT).map(|overlay| overlay.command.as_str()),
        Some("htop")
    );
    assert!(found.pane_overlay(RIGHT).is_none());
}

/// Upstream's `aHiddenPaneOverlayStaysHiddenOnItsNewPane`.
#[test]
fn a_hidden_pane_overlay_stays_hidden_on_its_new_pane() {
    let (mut store, id) = split_store();
    assert_eq!(store.open_pane_overlay(id, LEFT, program("left overlay")), Ok(()));
    store.set_overlays_hidden(id, Some(LEFT), true).unwrap();
    assert_eq!(store.swap_panes(id), Ok(()));
    let found = session(&store, id);
    assert_eq!(
        found.pane_overlay(RIGHT).map(|overlay| overlay.command.as_str()),
        Some("left overlay")
    );
    assert!(found.overlay_hidden(Some(RIGHT)));
    assert!(!found.overlay_hidden(Some(LEFT)));
}

#[test]
fn the_refusal_names_a_remote_reservation_in_scope() {
    for pane in [None, Some(LEFT), Some(RIGHT)] {
        let (mut store, id) = split_store();
        assert_eq!(
            session(&store, id).overlay_visibility_refusal(pane),
            OverlayVisibilityFailure::NoOverlay
        );
        assert!(store.reserve_remote_overlay(id, reservation(Some(RIGHT))));
        let expected = if pane == Some(LEFT) {
            OverlayVisibilityFailure::NoOverlay
        } else {
            OverlayVisibilityFailure::Remote
        };
        assert_eq!(
            session(&store, id).overlay_visibility_refusal(pane),
            expected,
            "{pane:?}"
        );
    }
}

/// Upstream's `focusRoutingSkipsHiddenCoversAndLandsOnThePane`: this build routes the keyboard by the active surface
/// and the focused overlay pane, upstream's `topmostSurface` and `focusTarget`.
#[test]
fn focus_routing_skips_hidden_covers_and_lands_on_the_pane() {
    let (mut store, id) = split_store();
    store.set_split_focus(id, false);
    store.select_session(Some(id));
    assert_eq!(store.open_pane_overlay(id, LEFT, program("htop")), Ok(()));
    assert!(store.open_overlay(id, program("less")));
    let active = |store: &AppStore| SurfaceId::active_in(store).map(|surface| surface.surface);
    assert_eq!(active(&store), Some(ZoomSurface::Overlay));

    store.session_mut(id).unwrap().set_overlay_hidden(true, None);
    assert_eq!(active(&store), Some(ZoomSurface::OverlayLeft));
    assert_eq!(session(&store, id).focused_overlay_pane(), Some(LEFT));

    store.session_mut(id).unwrap().set_overlay_hidden(true, Some(LEFT));
    assert_eq!(active(&store), Some(ZoomSurface::Primary));
    assert_eq!(session(&store, id).focused_overlay_pane(), None);
}

#[test]
fn a_hidden_page_neither_takes_the_keys_nor_hides_the_terminal_from_font_commands() {
    let (mut store, id) = split_store();
    assert_eq!(
        store.open_html_overlay(id, Some(RIGHT), page("/tmp/q.html"), None, None),
        Ok(())
    );
    assert_eq!(
        store.open_html_overlay(id, None, page("/tmp/p.html"), None, None),
        Ok(())
    );
    let found = session(&store, id);
    assert!(found.topmost_html_overlay().is_some());
    assert!(found.html_hides_terminal(Some(StatusPane::Left)));

    let found = store.session_mut(id).unwrap();
    found.set_overlay_hidden(true, None);
    found.set_overlay_hidden(true, Some(RIGHT));
    let found = session(&store, id);
    assert!(found.topmost_html_overlay().is_none());
    assert!(!found.html_hides_terminal(Some(StatusPane::Left)));
    assert!(!found.html_hides_terminal(Some(StatusPane::Right)));
    assert!(found.html_covers(None), "the page still holds its slot");
}

#[test]
fn an_unrealized_hidden_pane_overlay_on_a_rendered_pane_is_kept() {
    let (mut store, id) = split_store();
    assert_eq!(store.open_pane_overlay(id, LEFT, program("htop")), Ok(()));
    store.set_overlays_hidden(id, Some(LEFT), true).unwrap();
    let found = store.session_mut(id).unwrap();
    assert!(!found.drop_unrealized_pane_overlays(|_| false));
    assert_eq!(found.pane_overlay(LEFT).map(|overlay| overlay.hidden), Some(true));
}

// MARK: - the read-backs and covers

/// Upstream's `hiddenOverlaysReportHiddenSlotsWhileOccupancyStays`.
#[test]
fn hidden_overlays_report_hidden_slots_while_occupancy_stays() {
    let (mut store, id) = split_store();
    assert_eq!(store.open_pane_overlay(id, RIGHT, program("htop")), Ok(()));
    assert_eq!(
        store.open_html_overlay(id, None, page("/tmp/a/p.html"), None, None),
        Ok(())
    );
    assert_eq!(
        projection::session_node(session(&store, id), true).hidden_overlays,
        None
    );

    store.set_overlays_hidden(id, None, true).unwrap();
    let node = projection::session_node(session(&store, id), true);
    assert_eq!(
        node.hidden_overlays,
        Some(vec!["session".to_owned(), "right".to_owned()])
    );
    assert!(node.overlay);
    assert_eq!(node.pane_overlays, Some(vec!["right".to_owned()]));
    assert_eq!(node.html_overlays.as_ref().unwrap()[0].hidden, Some(true));
    let decoded: ngterm_proto::tree::ControlSessionNode =
        serde_json::from_value(serde_json::to_value(&node).unwrap()).unwrap();
    assert_eq!(decoded, node);
}

/// Upstream's `aHiddenOverlayCoversNothing`.
#[test]
fn a_hidden_overlay_covers_nothing_on_the_dashboard() {
    let (mut store, id) = split_store();
    assert_eq!(
        store.open_html_overlay(id, Some(RIGHT), page("/tmp/p.html"), None, None),
        Ok(())
    );
    assert!(store.open_overlay(id, program("less")));
    assert_eq!(
        session(&store, id).dashboard_cover(LEFT),
        Some(DashboardCover::Program {
            command: Some("less".to_owned())
        })
    );
    store.set_overlays_hidden(id, None, true).unwrap();
    assert_eq!(session(&store, id).dashboard_cover(LEFT), None);
    assert_eq!(session(&store, id).dashboard_cover(RIGHT), None);
}

/// Upstream's `aHiddenOverlayIsNeitherTheTargetNorAvailable`.
#[test]
fn a_hidden_overlay_is_neither_the_zoom_target_nor_available() {
    let (mut store, id) = split_store();
    store.select_session(Some(id));
    store.set_split_focus(id, true);
    assert_eq!(store.open_pane_overlay(id, RIGHT, program("htop")), Ok(()));
    assert!(store.open_overlay(id, program("less")));
    let target = |surface| SurfaceId { session: id, surface };
    assert_eq!(SurfaceId::active_in(&store), Some(target(ZoomSurface::Overlay)));

    store.session_mut(id).unwrap().set_overlay_hidden(true, None);
    assert_eq!(SurfaceId::active_in(&store), Some(target(ZoomSurface::OverlayRight)));
    assert!(!target(ZoomSurface::Overlay).is_valid_in(&store));

    store.set_overlays_hidden(id, Some(RIGHT), true).unwrap();
    assert_eq!(SurfaceId::active_in(&store), Some(target(ZoomSurface::Split)));
    assert!(!target(ZoomSurface::OverlayRight).is_valid_in(&store));
    assert!(ZoomSurface::Split.is_visible(session(&store, id)));
}
