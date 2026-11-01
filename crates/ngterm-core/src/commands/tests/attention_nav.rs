//! Previous/Next Attention Session as a chord, the menu or the action palette runs it ([`crate::app_actions`]), with
//! and without the `attentionNavigationAcrossWindows` setting (upstream's `AppActionsNavigationTests.swift`).
//!
//! Upstream raises another window's target one main-queue tick later, so a palette dismissal goes first; here the
//! palette row has closed the palette before the step runs, so the raise is immediate and the tests check the
//! outcome only (the `WindowRequest::Raise` the frontend performs, the frontmost window, the selection).

use uuid::Uuid;

use super::Fixture;
use crate::WindowRequest;
use crate::app_actions::ActionOrigin;
use crate::keymap::BuiltinAction;
use crate::model::{AgentIndicator, AgentStatus, AppStore, NewSession, StatusPane};
use crate::palette::{PaletteAction, PaletteMode};
use crate::palette_catalog::PaletteCommand;
use crate::persist::AppSettings;
use crate::zoom::{SurfaceId, ZoomSurface};

/// Two open windows, `front` frontmost with an idle `origin` selected, and `back` behind it.
struct Pair {
    fixture: Fixture,
    front: Uuid,
    origin: Uuid,
    back: Uuid,
}

fn pair(across_windows: bool) -> Pair {
    let settings = AppSettings {
        attention_navigation_across_windows: across_windows.then_some(true),
        ..AppSettings::default()
    };
    let mut fixture = Fixture::with_settings(0, settings);
    let front = fixture.window;
    let origin = add(&mut fixture, front, AgentIndicator::default());
    store_mut(&mut fixture, front).select_session(Some(origin));
    let back = fixture.core.library_mut().new_window(Some("back"));
    fixture.core.report_frontmost(front);
    let _ = fixture.core.commit();
    let _ = fixture.core.take_window_requests();
    Pair {
        fixture,
        front,
        origin,
        back,
    }
}

fn store_mut(fixture: &mut Fixture, window: Uuid) -> &mut AppStore {
    fixture
        .core
        .library_mut()
        .store_mut(window)
        .expect("the window is open")
}

fn store(fixture: &Fixture, window: Uuid) -> &AppStore {
    fixture.core.library().store(window).expect("the window is open")
}

/// An unselected session in `window`'s first workspace carrying `indicator`.
fn add(fixture: &mut Fixture, window: Uuid, indicator: AgentIndicator) -> Uuid {
    let store = store_mut(fixture, window);
    let workspace = store.workspaces()[0].id();
    let spec = NewSession {
        select: false,
        ..NewSession::at_cwd("/tmp")
    };
    let session = store.add_session(workspace, spec).expect("the workspace exists");
    store.set_agent_indicator(indicator, session);
    session
}

fn status(status: AgentStatus) -> AgentIndicator {
    AgentIndicator {
        status,
        ..AgentIndicator::default()
    }
}

fn step(pair: &mut Pair, action: BuiltinAction) {
    let origin = ActionOrigin {
        window: pair.front,
        quick: false,
    };
    pair.fixture.core.perform_builtin(origin, action);
    let _ = pair.fixture.core.commit();
}

fn selected(pair: &Pair, window: Uuid) -> Option<Uuid> {
    store(&pair.fixture, window).selected_session_id()
}

fn frontmost(pair: &Pair) -> Option<Uuid> {
    pair.fixture.core.library().active_window_id()
}

#[test]
fn the_step_stays_in_the_window_with_the_setting_off() {
    let mut pair = pair(false);
    let before = selected(&pair, pair.back);
    add(&mut pair.fixture, pair.back, status(AgentStatus::Blocked));
    step(&mut pair, BuiltinAction::NextAttentionSession);
    assert_eq!(frontmost(&pair), Some(pair.front));
    assert_eq!(selected(&pair, pair.front), Some(pair.origin));
    assert_eq!(selected(&pair, pair.back), before);
    assert!(pair.fixture.core.take_window_requests().is_empty());
}

/// With the setting off the chord is the in-window `session.go` step plus the reveal of the pane that set the status.
#[test]
fn the_in_window_step_reveals_the_pane_that_set_the_status() {
    let mut pair = pair(false);
    let target = add(
        &mut pair.fixture,
        pair.front,
        AgentIndicator {
            status_pane: Some(StatusPane::Scratch),
            ..status(AgentStatus::Blocked)
        },
    );
    step(&mut pair, BuiltinAction::NextAttentionSession);
    assert_eq!(selected(&pair, pair.front), Some(target));
    assert!(store(&pair.fixture, pair.front).session(target).unwrap().scratch_active);
}

#[test]
fn the_step_raises_the_other_window_and_selects_there_with_the_setting_on() {
    let mut pair = pair(true);
    let target = add(&mut pair.fixture, pair.back, status(AgentStatus::Blocked));
    step(&mut pair, BuiltinAction::NextAttentionSession);
    assert_eq!(frontmost(&pair), Some(pair.back));
    assert_eq!(
        pair.fixture.core.take_window_requests(),
        [WindowRequest::Raise(pair.back)]
    );
    assert_eq!(selected(&pair, pair.back), Some(target));
    assert_eq!(selected(&pair, pair.front), Some(pair.origin));
}

#[test]
fn the_step_prefers_the_next_target_in_the_same_window_and_selects_it_at_once() {
    let mut pair = pair(true);
    let local = add(&mut pair.fixture, pair.front, status(AgentStatus::Completed));
    add(&mut pair.fixture, pair.back, status(AgentStatus::Blocked));
    step(&mut pair, BuiltinAction::NextAttentionSession);
    assert_eq!(selected(&pair, pair.front), Some(local));
    assert_eq!(frontmost(&pair), Some(pair.front));
    assert!(pair.fixture.core.take_window_requests().is_empty());
}

#[test]
fn the_previous_step_wraps_backward_into_the_other_window() {
    let mut pair = pair(true);
    let target = add(&mut pair.fixture, pair.back, status(AgentStatus::Blocked));
    step(&mut pair, BuiltinAction::PreviousAttentionSession);
    assert_eq!(frontmost(&pair), Some(pair.back));
    assert_eq!(selected(&pair, pair.back), Some(target));
}

#[test]
fn the_step_skips_a_window_under_terminal_zoom() {
    let mut pair = pair(true);
    let before = selected(&pair, pair.back);
    let session = add(&mut pair.fixture, pair.back, status(AgentStatus::Blocked));
    let zoomed = SurfaceId {
        session,
        surface: ZoomSurface::Primary,
    };
    pair.fixture.core.set_zoom_target(pair.back, Some(zoomed));
    step(&mut pair, BuiltinAction::NextAttentionSession);
    assert_eq!(frontmost(&pair), Some(pair.front));
    assert_eq!(selected(&pair, pair.front), Some(pair.origin));
    assert_eq!(selected(&pair, pair.back), before);
}

/// The covered window is skipped, not a dead end: the scan carries on to a target in a later window. Pins that the
/// step passes the cover check to the scan rather than leaving it to the selection, which would swallow the step.
#[test]
fn the_step_skips_a_zoomed_window_and_carries_on_to_the_next() {
    let mut pair = pair(true);
    let before = selected(&pair, pair.back);
    let covered = add(&mut pair.fixture, pair.back, status(AgentStatus::Blocked));
    let third = pair.fixture.core.library_mut().new_window(Some("third"));
    let target = add(&mut pair.fixture, third, status(AgentStatus::Blocked));
    let zoomed = SurfaceId {
        session: covered,
        surface: ZoomSurface::Primary,
    };
    pair.fixture.core.set_zoom_target(pair.back, Some(zoomed));
    pair.fixture.core.report_frontmost(pair.front);
    let _ = pair.fixture.core.commit();
    let _ = pair.fixture.core.take_window_requests();
    step(&mut pair, BuiltinAction::NextAttentionSession);
    assert_eq!(frontmost(&pair), Some(third));
    assert_eq!(pair.fixture.core.take_window_requests(), [WindowRequest::Raise(third)]);
    assert_eq!(selected(&pair, third), Some(target));
    assert_eq!(selected(&pair, pair.back), before);
    assert_eq!(selected(&pair, pair.front), Some(pair.origin));
}

#[test]
fn the_step_from_a_window_with_no_selection_reaches_the_other_window() {
    let mut pair = pair(true);
    let target = add(&mut pair.fixture, pair.back, status(AgentStatus::Blocked));
    store_mut(&mut pair.fixture, pair.front).select_session(None);
    step(&mut pair, BuiltinAction::NextAttentionSession);
    assert_eq!(frontmost(&pair), Some(pair.back));
    assert_eq!(selected(&pair, pair.back), Some(target));
}

#[test]
fn the_step_reveals_the_captured_pane_after_an_auto_reset_clears() {
    let mut pair = pair(true);
    let indicator = AgentIndicator {
        auto_reset: true,
        status_pane: Some(StatusPane::Scratch),
        ..status(AgentStatus::Completed)
    };
    let target = add(&mut pair.fixture, pair.back, indicator);
    step(&mut pair, BuiltinAction::NextAttentionSession);
    let state = store(&pair.fixture, pair.back).session(target).unwrap();
    assert_eq!(state.agent_indicator.status, AgentStatus::Idle);
    assert!(state.scratch_active);
}

#[test]
fn the_action_palette_row_closes_the_palette_then_raises_the_other_window() {
    let mut pair = pair(true);
    let target = add(&mut pair.fixture, pair.back, status(AgentStatus::Blocked));
    let front = pair.front;
    pair.fixture.core.toggle_palette(front, PaletteMode::Actions);
    let entries = pair.fixture.core.palette_entries(PaletteMode::Actions);
    let row = entries
        .iter()
        .find(|entry| entry.title == "Next Attention Session")
        .expect("the row is listed");
    let ran = pair.fixture.core.run_palette_entry(front, &row.action.clone());
    assert!(ran.is_some(), "the row ran");
    let _ = pair.fixture.core.commit();
    assert_eq!(pair.fixture.core.palette_mode(), None);
    assert_eq!(frontmost(&pair), Some(pair.back));
    assert_eq!(selected(&pair, pair.back), Some(target));
}

#[test]
fn an_empty_window_enables_the_attention_steps_only_with_the_setting() {
    let mut pair = pair(false);
    let front = pair.front;
    store_mut(&mut pair.fixture, front).select_session(None);
    let enabled = |pair: &Pair, command: PaletteCommand| command.is_enabled(&pair.fixture.core.palette_context(front));
    assert!(!enabled(&pair, PaletteCommand::NextAttentionSession));
    pair.fixture
        .core
        .update_settings(|settings| settings.attention_navigation_across_windows = Some(true));
    assert!(enabled(&pair, PaletteCommand::NextAttentionSession));
    assert!(enabled(&pair, PaletteCommand::PreviousAttentionSession));
    let action = PaletteAction::Command(PaletteCommand::NextAttentionSession);
    assert!(pair.fixture.core.palette_entry_enabled(front, &action));
}

#[test]
fn the_step_with_no_other_target_reveals_the_current_sessions_tagged_pane() {
    let mut pair = pair(true);
    let (front, origin) = (pair.front, pair.origin);
    let indicator = AgentIndicator {
        status_pane: Some(StatusPane::Scratch),
        ..status(AgentStatus::Blocked)
    };
    store_mut(&mut pair.fixture, front).set_agent_indicator(indicator, origin);
    step(&mut pair, BuiltinAction::NextAttentionSession);
    assert_eq!(frontmost(&pair), Some(front));
    assert!(store(&pair.fixture, front).session(origin).unwrap().scratch_active);
}
