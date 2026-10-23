//! `pane.focused` through commits of real model mutations and the window's debounce (upstream's `AppStoreEventTests`
//! pane-focus cases). Upstream settles by draining the observer's hop and flushing the debouncer; here a settle commits
//! (arming the debounce), flushes it and commits the emitted events.

use ngterm_host::fakes::FakeSurface;
use ngterm_proto::{ControlEvent, EventKind};
use uuid::Uuid;

use super::{Fixture, wire};
use crate::events::{EventCursor, ReadOptions};
use crate::model::{AppStore, NewSession, StatusPane};

/// Commits, fires the window's pending debounce and commits what it emitted.
fn settle(fixture: &mut Fixture) {
    let _ = fixture.core.commit();
    fixture.core.flush_pane_focus();
    let _ = fixture.core.commit();
}

fn store(fixture: &mut Fixture) -> &mut AppStore {
    fixture.store_mut().unwrap()
}

/// Installs a fake terminal carrying `token` in pane `pane` of session `id`, as upstream's tests set a `SpySurface`.
fn install(fixture: &mut Fixture, id: Uuid, pane: StatusPane, token: &str) {
    store(fixture)
        .session_mut(id)
        .unwrap()
        .pane_slot_mut(pane)
        .replace(Box::new(FakeSurface::new(token)));
}

/// The ring's tail, the anchor the events after it are read from.
fn anchor(fixture: &Fixture) -> u64 {
    fixture.core.events().ring().current_sequence()
}

/// The `pane.focused` events sequenced after `after`.
fn pane_focus_events(fixture: &Fixture, after: u64) -> Vec<ControlEvent> {
    let options = ReadOptions {
        cursor: Some(EventCursor {
            run: fixture.core.events().ring().run(),
            after,
        }),
        kinds: Some([EventKind::PaneFocused].into()),
        ..ReadOptions::default()
    };
    fixture.core.events().read(&options).unwrap().items
}

fn panes(events: &[ControlEvent]) -> Vec<Option<&str>> {
    events.iter().map(|event| event.payload.pane.as_deref()).collect()
}

fn tokens(events: &[ControlEvent]) -> Vec<Option<&str>> {
    events.iter().map(|event| event.payload.pane_id.as_deref()).collect()
}

/// A shown split with the right pane selected and both surfaces carrying tokens, its selection settled (upstream's
/// `splitSession`).
fn split_session() -> (Fixture, Uuid) {
    let mut fixture = Fixture::new(0);
    let id = fixture.sessions[0];
    install(&mut fixture, id, StatusPane::Left, "primary-token");
    store(&mut fixture).set_split_visibility(id, true, None);
    install(&mut fixture, id, StatusPane::Right, "split-token");
    settle(&mut fixture);
    (fixture, id)
}

#[test]
fn pane_selection_moves_emit_each_settled_pane_with_its_token() {
    let (mut fixture, id) = split_session();
    let after = anchor(&fixture);

    store(&mut fixture).set_split_focus(id, false);
    settle(&mut fixture);
    store(&mut fixture).set_split_focus(id, true);
    settle(&mut fixture);

    let events = pane_focus_events(&fixture, after);
    assert_eq!(panes(&events), [Some("left"), Some("right")]);
    assert_eq!(tokens(&events), [Some("primary-token"), Some("split-token")]);
    let name = fixture.store().session(id).unwrap().display_name();
    for event in &events {
        assert_eq!(event.window.as_deref(), Some(wire(fixture.window).as_str()));
        assert_eq!(event.workspace.as_deref(), Some(wire(fixture.workspace).as_str()));
        assert_eq!(event.session.as_deref(), Some(wire(id).as_str()));
        assert_eq!(event.payload.name.as_deref(), Some(name.as_str()));
        assert_eq!(event.payload.previous, None);
    }
}

#[test]
fn opening_a_new_split_emits_the_new_pane() {
    let mut fixture = Fixture::new(0);
    let id = fixture.sessions[0];
    install(&mut fixture, id, StatusPane::Left, "primary-token");
    settle(&mut fixture);
    let after = anchor(&fixture);

    store(&mut fixture).set_split_visibility(id, true, None);
    settle(&mut fixture);

    let events = pane_focus_events(&fixture, after);
    assert_eq!(panes(&events), [Some("right")]);
    assert_eq!(tokens(&events), [None], "the new pane has no surface yet");
}

#[test]
fn a_burst_returning_to_the_settled_pane_emits_nothing() {
    let (mut fixture, id) = split_session();
    let after = anchor(&fixture);

    store(&mut fixture).set_split_focus(id, false);
    let first = fixture.core.commit().pane_focused;
    store(&mut fixture).set_split_focus(id, true);
    let second = fixture.core.commit().pane_focused;
    assert_eq!(
        (first.len(), second.len()),
        (1, 1),
        "each move re-arms the window's debounce"
    );
    for timer in first.into_iter().chain(second) {
        fixture.core.fire_pane_focus(timer);
    }
    let _ = fixture.core.commit();

    assert!(pane_focus_events(&fixture, after).is_empty());
    assert!(fixture.store().session(id).unwrap().focuses_split());
}

/// A timer a later move re-armed past is stale: firing it while the selection is still moved emits nothing, and only
/// the live one reports the settled pane.
#[test]
fn a_stale_timer_emits_nothing_while_the_selection_is_still_moved() {
    let (mut fixture, id) = split_session();
    let after = anchor(&fixture);

    let mut timers = Vec::new();
    for focused in [false, true, false] {
        store(&mut fixture).set_split_focus(id, focused);
        timers.extend(fixture.core.commit().pane_focused);
    }
    let [first, second, live] = timers[..] else {
        panic!("each move re-arms the window's debounce: {timers:?}");
    };
    fixture.core.fire_pane_focus(first);
    fixture.core.fire_pane_focus(second);
    let _ = fixture.core.commit();
    assert!(
        pane_focus_events(&fixture, after).is_empty(),
        "stale timers check nothing"
    );

    fixture.core.fire_pane_focus(live);
    let _ = fixture.core.commit();
    assert_eq!(panes(&pane_focus_events(&fixture, after)), [Some("left")]);
}

#[test]
fn a_commit_that_moves_nothing_arms_nothing() {
    let (mut fixture, id) = split_session();
    store(&mut fixture).rename_session(id, "renamed");
    assert!(fixture.core.commit().pane_focused.is_empty());
}

#[test]
fn swaps_same_value_writes_and_a_window_coming_forward_emit_nothing() {
    let (mut fixture, id) = split_session();
    let after = anchor(&fixture);

    store(&mut fixture).swap_panes(id).unwrap();
    settle(&mut fixture);
    let focused = fixture.store().session(id).unwrap().split.focused;
    store(&mut fixture).set_split_focus(id, focused);
    settle(&mut fixture);
    store(&mut fixture).set_split_visibility(id, false, None);
    store(&mut fixture).set_split_visibility(id, true, None);
    settle(&mut fixture);
    let window = fixture.window;
    fixture.core.library_mut().set_frontmost(Some(window));
    settle(&mut fixture);

    assert!(pane_focus_events(&fixture, after).is_empty());
}

#[test]
fn closing_the_selected_split_emits_the_primary() {
    let (mut fixture, id) = split_session();
    let after = anchor(&fixture);

    store(&mut fixture).close_split(id);
    settle(&mut fixture);

    let events = pane_focus_events(&fixture, after);
    assert_eq!(panes(&events), [Some("left")]);
    assert_eq!(tokens(&events), [Some("primary-token")]);
}

#[test]
fn promoting_the_selected_survivor_emits_nothing() {
    let (mut fixture, id) = split_session();
    let after = anchor(&fixture);

    store(&mut fixture).close_primary_pane(id);
    settle(&mut fixture);

    assert!(pane_focus_events(&fixture, after).is_empty());
}

#[test]
fn promoting_after_the_primary_exits_emits_the_survivor() {
    let (mut fixture, id) = split_session();
    store(&mut fixture).set_split_focus(id, false);
    settle(&mut fixture);
    let after = anchor(&fixture);

    store(&mut fixture).close_primary_pane(id);
    settle(&mut fixture);

    let events = pane_focus_events(&fixture, after);
    assert_eq!(panes(&events), [Some("left")]);
    assert_eq!(tokens(&events), [Some("split-token")]);
}

#[test]
fn a_reload_is_not_a_selection_but_a_later_flip_is() {
    let (mut fixture, id) = split_session();
    store(&mut fixture).set_split_focus(id, false);
    let left_selected = fixture.store().snapshot();
    store(&mut fixture).set_split_focus(id, true);
    settle(&mut fixture);
    let after = anchor(&fixture);

    store(&mut fixture).restore(&left_selected, false);
    settle(&mut fixture);
    assert!(!fixture.store().session(id).unwrap().focuses_split());
    store(&mut fixture).set_split_focus(id, true);
    settle(&mut fixture);

    let events = pane_focus_events(&fixture, after);
    assert_eq!(panes(&events), [Some("right")]);
    assert_eq!(events[0].session.as_deref(), Some(wire(id).as_str()));
}

#[test]
fn a_session_new_to_the_store_only_joins_the_baseline() {
    let (mut fixture, _) = split_session();
    let after = anchor(&fixture);

    let workspace = fixture.workspace;
    let added = store(&mut fixture)
        .add_session(workspace, NewSession::at_cwd("/tmp"))
        .unwrap();
    settle(&mut fixture);

    assert!(pane_focus_events(&fixture, after).is_empty());
    let identity = fixture.store().session(added).unwrap().pane_identity;
    assert_eq!(fixture.store().pane_focus.baseline.get(&added), Some(&identity));
}

/// A new session joins the baseline at the debounce's check, not at its create's commit (upstream's
/// `emitPaneFocusChanges`): a split before that check is where the session joins, unreported.
#[test]
fn a_split_right_after_the_create_only_joins_the_baseline() {
    let (mut fixture, _) = split_session();
    let workspace = fixture.workspace;
    let added = store(&mut fixture)
        .add_session(workspace, NewSession::at_cwd("/tmp"))
        .unwrap();
    let after = anchor(&fixture);
    let _ = fixture.core.commit();

    store(&mut fixture).set_split_visibility(added, true, None);
    settle(&mut fixture);

    assert!(pane_focus_events(&fixture, after).is_empty());
    let split = fixture.store().session(added).unwrap().split_pane_identity;
    assert_eq!(fixture.store().pane_focus.baseline.get(&added), split.as_ref());
}

/// A new window's store observes from its creation, as upstream's `newWindow` builds an empty store and adds the
/// default session after: that session is new to the baseline, so a split before the first debounce check is where
/// it joins, unreported.
#[test]
fn a_split_right_after_window_new_only_joins_the_baseline() {
    let mut fixture = Fixture::new(0);
    settle(&mut fixture);
    let window = fixture.core.library_mut().new_window(Some("second"));
    let store = fixture.core.library_mut().store_mut(window).unwrap();
    let id = store.sessions().next().unwrap().id();
    let after = anchor(&fixture);
    assert_eq!(
        fixture.core.commit().pane_focused.len(),
        1,
        "the window's default session is new to its baseline"
    );

    let store = fixture.core.library_mut().store_mut(window).unwrap();
    store.set_split_visibility(id, true, None);
    let _ = fixture.core.commit();
    fixture.core.flush_pane_focus();
    let _ = fixture.core.commit();

    assert!(pane_focus_events(&fixture, after).is_empty());
    let store = fixture.core.library().store(window).unwrap();
    let split = store.session(id).unwrap().split_pane_identity;
    assert!(split.is_some());
    assert_eq!(store.pane_focus.baseline.get(&id), split.as_ref());
}
