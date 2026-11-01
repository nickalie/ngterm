//! The cross-window attention step, ported from upstream's `WindowLibraryAttentionTests.swift`.
//!
//! Upstream's `attentionIncludesActive` lives on every store; here it is one app-wide setting, so
//! `appliesEachWindowsIncludesActiveRule` passes it to the step. `theStoreStepStaysInItsWindowWithTheFlagOn` is a
//! `session.go` test here (`commands/tests/session.rs`), since the store holds no copy of the setting.

use uuid::Uuid;

use super::support::add_background;
use crate::model::{AgentIndicator, AgentStatus, AppStore, SessionNavigation, WindowLibrary};

const HOME: &str = "/home/tester";

fn store_mut(library: &mut WindowLibrary, id: Uuid) -> &mut AppStore {
    library.store_mut(id).expect("the window is open")
}

/// Adds an unselected session with `status` to `workspace`, else the window's first workspace.
fn add_session(library: &mut WindowLibrary, window: Uuid, status: AgentStatus, workspace: Option<Uuid>) -> Uuid {
    let store = store_mut(library, window);
    let workspace = workspace.unwrap_or_else(|| store.workspaces()[0].id());
    let session = add_background(store, workspace, "/tmp");
    store.set_agent_indicator(
        AgentIndicator {
            status,
            ..AgentIndicator::default()
        },
        session,
    );
    session
}

fn select(library: &mut WindowLibrary, window: Uuid, session: Option<Uuid>) {
    store_mut(library, window).select_session(session);
}

/// The session a step from `source` lands on, every window eligible and `active` not a stop.
fn step(library: &WindowLibrary, source: Uuid, forward: bool) -> Option<Uuid> {
    library
        .attention_step(source, forward, false, |_| true)
        .map(|(_, session)| session)
}

/// A seeded library: its first window, which a new window does not change.
fn library() -> (WindowLibrary, Uuid) {
    let library = WindowLibrary::seeded(HOME);
    let first = library.windows()[0].id;
    (library, first)
}

#[test]
fn steps_through_windows_in_library_order_then_sidebar_order() {
    let (mut library, first) = library();
    let origin = add_session(&mut library, first, AgentStatus::Idle, None);
    let local = add_session(&mut library, first, AgentStatus::Completed, None);
    let second = library.new_window(Some("second"));
    let remote = add_session(&mut library, second, AgentStatus::Blocked, None);
    select(&mut library, first, Some(origin));
    assert_eq!(
        library.attention_step(first, true, false, |_| true),
        Some((first, local))
    );
    select(&mut library, first, Some(local));
    assert_eq!(
        library.attention_step(first, true, false, |_| true),
        Some((second, remote))
    );
}

#[test]
fn wraps_from_the_last_window_back_to_the_first() {
    let (mut library, first) = library();
    let early = add_session(&mut library, first, AgentStatus::Completed, None);
    let second = library.new_window(Some("second"));
    let origin = add_session(&mut library, second, AgentStatus::Idle, None);
    select(&mut library, second, Some(origin));
    assert_eq!(step(&library, second, true), Some(early));
}

#[test]
fn stepping_backward_from_the_first_window_wraps_to_the_last() {
    let (mut library, first) = library();
    let origin = add_session(&mut library, first, AgentStatus::Idle, None);
    let second = library.new_window(Some("second"));
    let late = add_session(&mut library, second, AgentStatus::Blocked, None);
    select(&mut library, first, Some(origin));
    assert_eq!(step(&library, first, false), Some(late));
}

#[test]
fn excludes_the_current_session() {
    let (mut library, first) = library();
    let only = add_session(&mut library, first, AgentStatus::Idle, None);
    select(&mut library, first, Some(only));
    store_mut(&mut library, first).set_agent_indicator(
        AgentIndicator {
            status: AgentStatus::Blocked,
            ..AgentIndicator::default()
        },
        only,
    );
    library.new_window(Some("second"));
    assert_eq!(step(&library, first, true), None);
}

#[test]
fn skips_an_ineligible_window_and_keeps_scanning() {
    let (mut library, first) = library();
    let origin = add_session(&mut library, first, AgentStatus::Idle, None);
    let covered = library.new_window(Some("covered"));
    add_session(&mut library, covered, AgentStatus::Blocked, None);
    let open = library.new_window(Some("open"));
    let reachable = add_session(&mut library, open, AgentStatus::Blocked, None);
    select(&mut library, first, Some(origin));
    let found = library.attention_step(first, true, false, |id| id != covered);
    assert_eq!(found, Some((open, reachable)));
}

#[test]
fn keeps_the_source_position_when_the_source_is_not_eligible() {
    let (mut library, first) = library();
    let origin = add_session(&mut library, first, AgentStatus::Idle, None);
    let local = add_session(&mut library, first, AgentStatus::Blocked, None);
    library.new_window(Some("second"));
    select(&mut library, first, Some(origin));
    let found = library.attention_step(first, true, false, |id| id != first);
    assert_eq!(found, Some((first, local)));
}

#[test]
fn honors_each_windows_focus_filter() {
    let (mut library, first) = library();
    let origin = add_session(&mut library, first, AgentStatus::Idle, None);
    let second = library.new_window(Some("second"));
    add_session(&mut library, second, AgentStatus::Blocked, None);
    let marked = store_mut(&mut library, second).add_workspace("marked");
    let shown = add_session(&mut library, second, AgentStatus::Completed, Some(marked));
    let store = store_mut(&mut library, second);
    store.set_focus_membership(marked, true);
    store.set_focus_enabled(true);
    select(&mut library, first, Some(origin));
    assert_eq!(step(&library, first, true), Some(shown));
}

/// Upstream's `appliesEachWindowsIncludesActiveRule`, with the app-wide setting passed to the step.
#[test]
fn applies_the_includes_active_rule() {
    for included in [false, true] {
        let (mut library, first) = library();
        let origin = add_session(&mut library, first, AgentStatus::Idle, None);
        let second = library.new_window(Some("second"));
        let running = add_session(&mut library, second, AgentStatus::Active, None);
        select(&mut library, first, Some(origin));
        let found = library
            .attention_step(first, true, included, |_| true)
            .map(|(_, session)| session);
        assert_eq!(found, included.then_some(running), "{included}");
    }
}

#[test]
fn matches_the_in_window_step_with_one_window_open() {
    for forward in [true, false] {
        let (mut library, first) = library();
        add_session(&mut library, first, AgentStatus::Completed, None);
        let origin = add_session(&mut library, first, AgentStatus::Idle, None);
        add_session(&mut library, first, AgentStatus::Blocked, None);
        select(&mut library, first, Some(origin));
        let crossed = step(&library, first, forward);
        let direction = if forward {
            SessionNavigation::NextAttention
        } else {
            SessionNavigation::PreviousAttention
        };
        let store = store_mut(&mut library, first);
        store.navigate_session(direction);
        assert!(crossed.is_some(), "{forward}");
        assert_eq!(crossed, store.selected_session_id(), "{forward}");
    }
}

#[test]
fn enters_at_the_ends_with_no_selection() {
    for forward in [true, false] {
        let (mut library, first) = library();
        let early = add_session(&mut library, first, AgentStatus::Completed, None);
        let second = library.new_window(Some("second"));
        let late = add_session(&mut library, second, AgentStatus::Blocked, None);
        select(&mut library, first, None);
        let expected = if forward { early } else { late };
        assert_eq!(step(&library, first, forward), Some(expected), "{forward}");
    }
}

#[test]
fn a_closed_window_is_no_candidate() {
    let (mut library, first) = library();
    let origin = add_session(&mut library, first, AgentStatus::Idle, None);
    let closed = library.new_window(Some("closed"));
    add_session(&mut library, closed, AgentStatus::Blocked, None);
    library.close_window(closed);
    select(&mut library, first, Some(origin));
    assert_eq!(step(&library, first, true), None);
}
