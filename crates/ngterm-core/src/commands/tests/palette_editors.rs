//! The palette's membership editors (upstream's #803: the five editor cases of `AppActionsPaletteTests`, the
//! enablement of `PaletteCatalogTests` through a real window, and the stay-open run of `PaletteUITests`): Edit Flagged
//! Sessions lists every session with a live flag mark, Edit Focused Workspaces every workspace with a live focus mark,
//! and a run toggles the row in place with the palette left open.

use super::{Fixture, wire};
use crate::app_actions::{ActionEffect, ActionOrigin};
use crate::keymap::BuiltinAction;
use crate::model::SidebarMode;
use crate::palette::{EDITOR_HINT, PaletteAction, PaletteEntry, PaletteMark, PaletteMode, filter_entries};
use crate::palette_catalog::PaletteCommand;

fn row(fixture: &Fixture, mode: PaletteMode, id: uuid::Uuid) -> PaletteEntry {
    fixture
        .core
        .palette_entries(mode)
        .into_iter()
        .find(|entry| entry.id == wire(id))
        .unwrap_or_else(|| panic!("no row {id}"))
}

fn origin(fixture: &Fixture) -> ActionOrigin {
    ActionOrigin {
        window: fixture.window,
        quick: false,
    }
}

/// Upstream's `testFlaggedEditorListsEverySessionInTreeOrderIncludingFocusHiddenOnes`.
#[test]
fn the_flagged_editor_lists_every_session_in_tree_order_including_focus_hidden_ones() {
    let mut fixture = Fixture::new(1);
    let first = fixture.workspace;
    let (other, theirs) = fixture.add_workspace("other", 2);
    let store = fixture.store_mut().unwrap();
    store.set_focus_membership(first, true);
    store.set_focus_enabled(true);
    assert!(
        !fixture
            .store()
            .visible_workspaces()
            .iter()
            .any(|workspace| workspace.id() == other),
        "focus hides it"
    );

    let entries = fixture.core.palette_entries(PaletteMode::FlaggedEditor);
    let expected: Vec<String> = fixture.sessions.iter().chain(&theirs).map(|id| wire(*id)).collect();
    let ids: Vec<&str> = entries.iter().map(|entry| entry.id.as_str()).collect();
    assert_eq!(ids, expected);
    let name = fixture.store().workspaces()[0].name.clone();
    let subtitles: Vec<Option<&str>> = entries.iter().map(|entry| entry.subtitle.as_deref()).collect();
    assert_eq!(
        subtitles,
        [Some(name.as_str()), Some(name.as_str()), Some("other"), Some("other")]
    );
    assert!(entries.iter().all(|entry| entry.mark == Some(PaletteMark::Flag(false))));
    assert!(
        entries.iter().all(|entry| entry.indicator.is_some()),
        "a session row carries its status"
    );
    assert_eq!(
        filter_entries(&entries, "", PaletteMode::FlaggedEditor),
        [0, 1, 2, 3],
        "a blank query keeps the tree order"
    );
}

/// Upstream's `testFlaggedEditorRowTogglesTheFlagAndItsMarkFollows`, with the palette left open on the same
/// generation, as `testFlaggedEditorTogglesInPlaceAndStaysOpen` checks on screen.
#[test]
fn a_flagged_editor_row_toggles_the_flag_in_place_and_its_mark_follows() {
    let mut fixture = Fixture::new(1);
    let window = fixture.window;
    let session = fixture.sessions[1];
    fixture.core.open_palette(window, PaletteMode::FlaggedEditor);
    let generation = fixture.core.palette_generation();
    let action = row(&fixture, PaletteMode::FlaggedEditor, session).action;
    assert_eq!(action, PaletteAction::ToggleFlag(session));

    assert_eq!(
        fixture.core.run_palette_entry(window, &action),
        Some(ActionEffect::Done)
    );
    assert!(fixture.store().session(session).unwrap().flagged);
    assert_eq!(
        row(&fixture, PaletteMode::FlaggedEditor, session).mark,
        Some(PaletteMark::Flag(true))
    );
    assert_eq!(
        fixture.core.palette_mode(),
        Some(PaletteMode::FlaggedEditor),
        "stays open"
    );
    assert_eq!(
        fixture.core.palette_generation(),
        generation,
        "the same palette, query and all"
    );

    fixture.core.run_palette_entry(window, &action);
    assert!(!fixture.store().session(session).unwrap().flagged);
    assert_eq!(
        row(&fixture, PaletteMode::FlaggedEditor, session).mark,
        Some(PaletteMark::Flag(false))
    );
    assert_eq!(fixture.core.palette_mode(), Some(PaletteMode::FlaggedEditor));
}

/// Upstream's `testFlaggedEditorRowGoesInertWhenItsSessionCloses`.
#[test]
fn a_flagged_editor_row_goes_inert_when_its_session_closes() {
    let mut fixture = Fixture::new(1);
    let window = fixture.window;
    let session = fixture.sessions[1];
    fixture.core.open_palette(window, PaletteMode::FlaggedEditor);
    let action = row(&fixture, PaletteMode::FlaggedEditor, session).action;
    fixture.store_mut().unwrap().close_session(session);

    assert!(!fixture.core.palette_entry_enabled(window, &action));
    assert_eq!(fixture.core.run_palette_entry(window, &action), None);
    assert_eq!(fixture.core.palette_mode(), Some(PaletteMode::FlaggedEditor));
}

/// Upstream's `testUnflaggingTheActiveSessionKeepsTheEditorWorking`: the selection moves to the flagged survivor and
/// the editor stays open, its next run acting on the row again.
#[test]
fn unflagging_the_active_session_in_the_flagged_view_keeps_the_editor_open() {
    let mut fixture = Fixture::new(1);
    let window = fixture.window;
    let [first, active] = [fixture.sessions[0], fixture.sessions[1]];
    let store = fixture.store_mut().unwrap();
    store.set_flags(true, &[first, active]);
    store.select_session(Some(active));
    store.set_sidebar_mode(SidebarMode::Flagged);
    fixture.core.open_palette(window, PaletteMode::FlaggedEditor);

    fixture
        .core
        .run_palette_entry(window, &PaletteAction::ToggleFlag(active));
    assert!(!fixture.store().session(active).unwrap().flagged);
    assert_eq!(
        fixture.selected(),
        Some(first),
        "the selection moves to the flagged survivor"
    );
    assert_eq!(fixture.core.palette_mode(), Some(PaletteMode::FlaggedEditor));

    fixture
        .core
        .run_palette_entry(window, &PaletteAction::ToggleFlag(active));
    assert!(fixture.store().session(active).unwrap().flagged);
}

/// Upstream's `testFocusEditorTogglesMembershipWithoutEverEnablingTheFilter`.
#[test]
fn the_focus_editor_toggles_membership_without_ever_enabling_the_filter() {
    let mut fixture = Fixture::new(0);
    let window = fixture.window;
    let first = fixture.workspace;
    let (other, _) = fixture.add_workspace("other", 0);
    let entries = fixture.core.palette_entries(PaletteMode::FocusEditor);
    let expected: Vec<String> = fixture
        .store()
        .workspaces()
        .iter()
        .map(|workspace| wire(workspace.id()))
        .collect();
    assert_eq!(
        entries.iter().map(|entry| entry.id.clone()).collect::<Vec<_>>(),
        expected
    );
    let subtitles: Vec<Option<&str>> = entries.iter().map(|entry| entry.subtitle.as_deref()).collect();
    assert_eq!(subtitles, [Some("1 session"), Some("0 sessions")]);
    fixture.core.open_palette(window, PaletteMode::FocusEditor);
    let toggle = |fixture: &mut Fixture, id| {
        fixture
            .core
            .run_palette_entry(window, &PaletteAction::ToggleFocusMember(id))
    };
    let focus = |fixture: &Fixture| fixture.store().focus().clone();

    assert_eq!(toggle(&mut fixture, first), Some(ActionEffect::Done));
    assert_eq!(focus(&fixture).ids().iter().copied().collect::<Vec<_>>(), [first]);
    assert_eq!(
        row(&fixture, PaletteMode::FocusEditor, first).mark,
        Some(PaletteMark::Focus(true))
    );
    assert!(!focus(&fixture).enabled(), "marking never turns the filter on");

    toggle(&mut fixture, first);
    toggle(&mut fixture, first);
    assert_eq!(focus(&fixture).ids().iter().copied().collect::<Vec<_>>(), [first]);
    assert!(
        !focus(&fixture).enabled(),
        "re-adding the last member leaves it off too"
    );

    fixture.store_mut().unwrap().set_focus_enabled(true);
    toggle(&mut fixture, other);
    toggle(&mut fixture, first);
    assert_eq!(focus(&fixture).ids().iter().copied().collect::<Vec<_>>(), [other]);
    assert!(
        focus(&fixture).enabled(),
        "an applied filter stays on while members remain"
    );
    assert_eq!(
        row(&fixture, PaletteMode::FocusEditor, first).mark,
        Some(PaletteMark::Focus(false))
    );
    assert_eq!(
        fixture.core.palette_mode(),
        Some(PaletteMode::FocusEditor),
        "stays open"
    );
}

/// Upstream's `testEditorBuiltinsOpenTheirModeOnTheNextRunloopTick`, without the tick: the action palette's row has
/// closed that palette before the built-in runs ([`crate::Core::run_palette_entry`]), so the editor opens at once. The
/// chord opens and never toggles, and the selection need not be a session for the flagged editor.
#[test]
fn the_editor_builtins_and_rows_open_their_palette() {
    let mut fixture = Fixture::new(0);
    let window = fixture.window;
    fixture.store_mut().unwrap().select_session(None);
    for (action, command, mode) in [
        (
            BuiltinAction::EditFlaggedSessions,
            PaletteCommand::EditFlaggedSessions,
            PaletteMode::FlaggedEditor,
        ),
        (
            BuiltinAction::EditFocusedWorkspaces,
            PaletteCommand::EditFocusedWorkspaces,
            PaletteMode::FocusEditor,
        ),
    ] {
        assert_eq!(
            fixture.core.perform_builtin(origin(&fixture), action),
            ActionEffect::Done
        );
        assert_eq!(fixture.core.palette_mode(), Some(mode), "{action:?}");
        fixture.core.perform_builtin(origin(&fixture), action);
        assert_eq!(
            fixture.core.palette_mode(),
            Some(mode),
            "{action:?} opens, never toggles"
        );
        fixture.core.close_palette();

        fixture.core.toggle_palette(window, PaletteMode::Actions);
        let entries = fixture.core.palette_entries(PaletteMode::Actions);
        assert!(
            entries
                .iter()
                .any(|entry| entry.action == PaletteAction::Command(command))
        );
        assert!(
            fixture
                .core
                .palette_entry_enabled(window, &PaletteAction::Command(command))
        );
        fixture.core.run_palette_entry(window, &PaletteAction::Command(command));
        assert_eq!(fixture.core.palette_mode(), Some(mode), "{command:?}");
        fixture.core.close_palette();
    }
}

/// The flagged editor needs a session somewhere in the window, not a selected one (upstream's `hasSessions`).
#[test]
fn the_flagged_editor_needs_a_session_in_the_window() {
    let mut fixture = Fixture::new(0);
    let window = fixture.window;
    let session = fixture.sessions[0];
    let enabled =
        |fixture: &Fixture| PaletteCommand::EditFlaggedSessions.is_enabled(&fixture.core.palette_context(window));
    fixture.store_mut().unwrap().select_session(None);
    assert!(enabled(&fixture), "an unselected session still counts");
    fixture.store_mut().unwrap().close_session(session);
    assert!(!enabled(&fixture), "no session to edit");
    fixture
        .core
        .perform_builtin(origin(&fixture), BuiltinAction::EditFlaggedSessions);
    assert_eq!(fixture.core.palette_mode(), None);
    assert!(
        PaletteCommand::EditFocusedWorkspaces.is_enabled(&fixture.core.palette_context(window)),
        "the window keeps its workspace"
    );
}

/// The footer each editor states (upstream's `editorHint`, pinned by `testViewMenuOpensTheFlaggedEditor` and
/// `testViewMenuOpensTheFocusEditor`).
#[test]
fn the_editors_state_their_footer_and_the_focus_editor_the_filter() {
    let mut fixture = Fixture::new(0);
    let window = fixture.window;
    let workspace = fixture.workspace;
    assert_eq!(fixture.core.palette_editor_hint(window), None, "closed");
    fixture.core.open_palette(window, PaletteMode::Actions);
    assert_eq!(fixture.core.palette_editor_hint(window), None, "not an editor");

    fixture.core.open_palette(window, PaletteMode::FlaggedEditor);
    assert_eq!(
        fixture.core.palette_editor_hint(window).as_deref(),
        Some("Return toggles · Esc closes · Changes apply immediately")
    );
    assert_eq!(fixture.core.palette_editor_hint(window).as_deref(), Some(EDITOR_HINT));

    fixture.core.open_palette(window, PaletteMode::FocusEditor);
    let hint = |fixture: &Fixture| fixture.core.palette_editor_hint(window).unwrap();
    assert_eq!(hint(&fixture), format!("{EDITOR_HINT}\nFilter off"));
    let store = fixture.store_mut().unwrap();
    store.set_focus_membership(workspace, true);
    store.set_focus_enabled(true);
    assert_eq!(hint(&fixture), format!("{EDITOR_HINT}\nFilter on"));
    fixture.store_mut().unwrap().set_sidebar_mode(SidebarMode::Flagged);
    assert_eq!(
        hint(&fixture),
        format!("{EDITOR_HINT}\nFilter on, applies in tree view")
    );
}
