//! A pane overlay drawn above the session-wide program or page it opened over (#831), ported from upstream's
//! `AppStoreOverlayVisibilityTests` holder cases, and a pane ask leaving a shown session cover unshaded (401ff29), from
//! `SessionTests.paneAskLeavesAShownSessionCoverUnshaded`.

use ngterm_proto::AskButton;
use uuid::Uuid;

use super::overlay_visibility::{names, open_hud, page, program, session, split_store};
use crate::model::{AppStore, AskAlignment, AskStyle, OverlayPane, PendingAsk, VisibleCover};

const LEFT: OverlayPane = OverlayPane::Left;
const RIGHT: OverlayPane = OverlayPane::Right;

/// A shown split with both terminals and the left pane focused (upstream's `makeFixture`).
fn fixture() -> (AppStore, Uuid) {
    let (mut store, id) = split_store();
    store.set_split_focus(id, false);
    (store, id)
}

/// A session-wide cover: a page or the `revdiff` program (upstream's `openCover`).
fn open_cover(store: &mut AppStore, id: Uuid, is_page: bool) {
    if is_page {
        assert_eq!(
            store.open_html_overlay(id, None, page("/tmp/cover.html"), None, None),
            Ok(())
        );
    } else {
        assert!(store.open_overlay(id, program("revdiff")));
    }
}

/// A page or the `htop` program over `pane` (upstream's `openOnPane`).
fn open_on_pane(store: &mut AppStore, id: Uuid, pane: OverlayPane, is_page: bool) {
    if is_page {
        let path = format!("/tmp/{}.html", pane.wire_name());
        assert_eq!(store.open_html_overlay(id, Some(pane), page(&path), None, None), Ok(()));
    } else {
        assert_eq!(store.open_pane_overlay(id, pane, program("htop")), Ok(()));
    }
}

fn cover_visible(store: &AppStore, id: Uuid) -> bool {
    session(store, id).cover_overlay_visible()
}

// MARK: - the hold

#[test]
fn a_pane_opener_draws_above_the_session_cover_until_it_closes() {
    for (cover_is_page, opener_is_page) in [(false, false), (false, true), (true, false), (true, true)] {
        let (mut store, id) = fixture();
        open_cover(&mut store, id, cover_is_page);

        open_on_pane(&mut store, id, LEFT, opener_is_page);
        let found = session(&store, id);
        assert_eq!(found.hidden_overlay_names(), ["session"]);
        assert!(found.cover_overlay_active());
        assert!(!found.overlay.hidden, "the user's own hidden bit is untouched");
        assert!(!found.session_slot_shown());
        assert_eq!(found.visible_cover(), Some(VisibleCover::FocusedPane));
        if cover_is_page {
            assert!(!found.html_overlay_visible());
        } else {
            assert!(!found.program_overlay_visible());
        }
        assert!(!found.full_overlay_visible());

        assert!(store.close_pane_overlay(id, LEFT));
        assert!(names(&store, id).is_empty());
        assert_eq!(session(&store, id).visible_cover(), Some(VisibleCover::Session));
    }
}

#[test]
fn a_floating_cover_is_hidden_too() {
    let (mut store, id) = fixture();
    let floating = crate::model::OverlayProgram {
        size_percent: Some(60),
        ..program("revdiff")
    };
    assert!(store.open_overlay(id, floating));

    open_on_pane(&mut store, id, LEFT, true);

    assert_eq!(names(&store, id), ["session"]);
}

#[test]
fn a_hud_stays_up_over_a_pane_opener() {
    let (mut store, id) = fixture();
    let dir = tempfile::tempdir().unwrap();
    open_hud(&mut store, id, &dir);

    open_on_pane(&mut store, id, LEFT, true);
    let found = session(&store, id);
    assert!(found.overlay.hud_active());
    assert!(!found.overlay.hidden);
    assert_eq!(
        found.pane_overlay(LEFT).and_then(|overlay| overlay.holds_cover),
        None,
        "a HUD is no cover"
    );
    assert!(store.close_pane_overlay(id, LEFT));
    assert!(!session(&store, id).overlay.hidden);
}

#[test]
fn a_cover_hidden_by_hand_stays_hidden_after_the_pane_closes() {
    let (mut store, id) = fixture();
    open_cover(&mut store, id, false);
    assert_eq!(store.set_overlays_hidden(id, None, true), Ok(()));

    open_on_pane(&mut store, id, LEFT, true);
    assert!(store.close_pane_overlay(id, LEFT));

    assert_eq!(names(&store, id), ["session"]);
}

#[test]
fn the_cover_returns_only_after_the_last_pane_opener_closes() {
    let (mut store, id) = fixture();
    open_cover(&mut store, id, false);
    open_on_pane(&mut store, id, LEFT, true);
    open_on_pane(&mut store, id, RIGHT, false);

    assert!(store.close_pane_overlay(id, LEFT));
    assert!(!cover_visible(&store, id));
    assert!(store.close_pane_overlay(id, RIGHT));
    assert!(cover_visible(&store, id));
}

#[test]
fn a_toggle_round_trip_keeps_the_pane_opener_on_top() {
    let (mut store, id) = fixture();
    open_cover(&mut store, id, false);
    open_on_pane(&mut store, id, LEFT, true);

    assert_eq!(store.toggle_overlays_hidden(id, None), Ok(()));
    assert_eq!(names(&store, id), ["session", "left"]);
    assert_eq!(store.toggle_overlays_hidden(id, None), Ok(()));

    let found = session(&store, id);
    assert!(found.pane_overlay_visible(LEFT));
    assert!(!found.cover_overlay_visible());
    assert_eq!(found.visible_cover(), Some(VisibleCover::FocusedPane));
}

#[test]
fn hiding_the_pane_opener_shows_the_cover_and_a_toggle_restores_the_order() {
    let (mut store, id) = fixture();
    open_cover(&mut store, id, false);
    open_on_pane(&mut store, id, LEFT, true);

    assert_eq!(store.set_overlays_hidden(id, Some(LEFT), true), Ok(()));
    assert!(cover_visible(&store, id));
    assert_eq!(names(&store, id), ["left"]);

    assert_eq!(store.toggle_overlays_hidden(id, None), Ok(()));
    assert_eq!(store.toggle_overlays_hidden(id, None), Ok(()));
    assert!(session(&store, id).pane_overlay_visible(LEFT));
    assert!(!cover_visible(&store, id));
}

#[test]
fn a_pane_opener_under_a_hand_hidden_cover_still_draws_above_it_when_shown() {
    let (mut store, id) = fixture();
    open_cover(&mut store, id, false);
    assert_eq!(store.set_overlays_hidden(id, None, true), Ok(()));
    open_on_pane(&mut store, id, LEFT, true);

    assert_eq!(store.set_overlays_hidden(id, None, false), Ok(()));

    assert!(!cover_visible(&store, id));
    assert!(store.close_pane_overlay(id, LEFT));
    assert!(cover_visible(&store, id));
}

#[test]
fn a_cover_opened_after_the_pane_overlay_stays_on_top() {
    let (mut store, id) = fixture();
    open_on_pane(&mut store, id, LEFT, true);

    open_cover(&mut store, id, false);

    assert!(cover_visible(&store, id));
    assert!(names(&store, id).is_empty());
}

#[test]
fn a_holder_on_a_pane_the_session_does_not_lay_out_leaves_the_cover_shown() {
    let (mut store, id) = fixture();
    open_cover(&mut store, id, false);
    open_on_pane(&mut store, id, RIGHT, true);
    assert!(!cover_visible(&store, id));

    let found = store.session_mut(id).unwrap();
    found.split.focused = false;
    found.split.shown = false;

    assert!(cover_visible(&store, id));
    store.session_mut(id).unwrap().split.shown = true;
    assert!(!cover_visible(&store, id));
}

#[test]
fn a_replacement_cover_is_not_held_by_an_opener_of_the_old_one() {
    let (mut store, id) = fixture();
    open_cover(&mut store, id, false);
    open_on_pane(&mut store, id, LEFT, true);
    assert!(store.close_overlay(id));
    open_cover(&mut store, id, false);
    assert_eq!(session(&store, id).visible_cover(), Some(VisibleCover::Session));

    open_on_pane(&mut store, id, RIGHT, false);
    assert!(!cover_visible(&store, id));
    assert!(store.close_pane_overlay(id, RIGHT));

    assert!(cover_visible(&store, id));
}

/// The same for a cover torn down with its slot rather than closed: the session's terminals all go.
#[test]
fn a_torn_down_cover_leaves_no_hold_for_the_next_one() {
    let (mut store, id) = fixture();
    open_cover(&mut store, id, true);
    open_on_pane(&mut store, id, LEFT, true);
    store.session_mut(id).unwrap().overlay.close();
    open_cover(&mut store, id, false);

    assert!(cover_visible(&store, id));
    assert!(names(&store, id).is_empty());
}

#[test]
fn closing_the_holding_split_brings_the_cover_back() {
    let (mut store, id) = fixture();
    open_cover(&mut store, id, false);
    open_on_pane(&mut store, id, RIGHT, true);

    store.close_split(id);

    assert!(session(&store, id).pane_overlays.open_panes().is_empty());
    assert!(cover_visible(&store, id));
}

#[test]
fn promotion_and_swap_keep_the_hold() {
    let (mut store, id) = fixture();
    open_cover(&mut store, id, false);
    open_on_pane(&mut store, id, RIGHT, true);

    assert_eq!(store.swap_panes(id), Ok(()));
    assert_eq!(session(&store, id).pane_overlays.open_panes(), [LEFT]);
    assert!(!cover_visible(&store, id));

    assert_eq!(store.swap_panes(id), Ok(()));
    store.close_primary_pane(id);
    assert_eq!(session(&store, id).pane_overlays.open_panes(), [LEFT]);
    assert!(!cover_visible(&store, id));

    assert!(store.close_pane_overlay(id, LEFT));
    assert!(cover_visible(&store, id));
}

/// A local page's `htmlOverlays[].hidden` reads the session-wide page back as out of sight while it is held, and the
/// pane page over it as shown.
#[test]
fn a_held_session_page_reads_back_hidden() {
    let (mut store, id) = fixture();
    open_cover(&mut store, id, true);
    open_on_pane(&mut store, id, LEFT, true);

    let node = crate::projection::session_node(session(&store, id), true);
    let pages = node.html_overlays.unwrap();
    assert_eq!(pages[0].pane, None);
    assert_eq!(pages[0].hidden, Some(true));
    assert_eq!(pages[1].pane.as_deref(), Some("left"));
    assert_eq!(pages[1].hidden, None);
    assert_eq!(node.hidden_overlays, Some(vec!["session".to_owned()]));
}

// MARK: - the ask shade

/// What the session holds under the ask in [`a_pane_ask_leaves_a_shown_session_cover_unshaded`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShadeCase {
    SessionAskOverProgram,
    PaneAsk,
    PaneAskOverProgram,
    PaneAskOverPage,
    PaneAskOverHiddenProgram,
    PaneAskOverHud,
    PaneAskOverHeldProgram,
}

fn pending_ask() -> PendingAsk {
    PendingAsk {
        id: "a".to_owned(),
        title: "Continue?".to_owned(),
        message: None,
        buttons: vec![AskButton {
            id: "yes".to_owned(),
            label: "Yes".to_owned(),
            hotkey: None,
        }],
        default_id: None,
        destructive_id: None,
        style: AskStyle::Terminal,
        align: AskAlignment::Center,
        width: None,
        anchor: None,
        form: None,
        cancel_id: None,
    }
}

fn set_up_shade_case(store: &mut AppStore, id: Uuid, case: ShadeCase, dir: &tempfile::TempDir) {
    match case {
        ShadeCase::SessionAskOverProgram | ShadeCase::PaneAskOverProgram => open_cover(store, id, false),
        ShadeCase::PaneAsk => {}
        ShadeCase::PaneAskOverPage => open_cover(store, id, true),
        ShadeCase::PaneAskOverHiddenProgram => {
            open_cover(store, id, false);
            assert_eq!(store.set_overlays_hidden(id, None, true), Ok(()));
        }
        ShadeCase::PaneAskOverHud => open_hud(store, id, dir),
        ShadeCase::PaneAskOverHeldProgram => {
            open_cover(store, id, false);
            open_on_pane(store, id, LEFT, true);
        }
    }
}

/// Regression: a pane ask's pane-sized shade split a session-wide cover into two tones at the divider. The held case is
/// this build's own: a cover out of sight under a pane overlay is shaded as a hidden one is.
#[test]
fn a_pane_ask_leaves_a_shown_session_cover_unshaded() {
    use ShadeCase as C;
    let cases = [
        C::SessionAskOverProgram,
        C::PaneAsk,
        C::PaneAskOverProgram,
        C::PaneAskOverPage,
        C::PaneAskOverHiddenProgram,
        C::PaneAskOverHud,
        C::PaneAskOverHeldProgram,
    ];
    for case in cases {
        let (mut store, id) = fixture();
        let dir = tempfile::tempdir().unwrap();
        set_up_shade_case(&mut store, id, case, &dir);
        let split = session(&store, id).split_pane_identity;
        assert!(split.is_some());
        let pane = if case == C::SessionAskOverProgram { None } else { split };
        assert!(store.open_session_ask(id, pending_ask(), pane));

        let unshaded = [C::PaneAskOverProgram, C::PaneAskOverPage].contains(&case);
        assert_eq!(session(&store, id).ask_shades_frame(), !unshaded, "{case:?}");
    }
}
