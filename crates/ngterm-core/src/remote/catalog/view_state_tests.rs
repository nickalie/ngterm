//! A remote window's view state (upstream's `RemoteWindowViewState` cases).

use uuid::Uuid;

use super::{RemoteWindowViewState, SessionView};
use crate::model::{AppStore, NewSession};

fn split_store() -> (AppStore, Uuid) {
    let mut store = AppStore::new();
    let workspace = store.add_workspace("w");
    let id = store.add_session(workspace, NewSession::at_cwd("/tmp")).unwrap();
    store.set_split_visibility(id, true, None);
    (store, id)
}

#[test]
fn a_rows_focus_font_and_divider_come_back_on_the_row() {
    let (mut store, id) = split_store();
    let session = store.session_mut(id).unwrap();
    session.split.focused = true;
    session.split.ratio = Some(0.3);
    session.font_size = Some(16.0);
    let view = SessionView::of(session);
    assert_eq!(view.focused_pane, session.split_pane_identity);
    session.split.focused = false;
    session.split.ratio = None;
    session.font_size = None;
    view.apply(session);
    assert!(session.split.focused);
    assert_eq!(session.split.ratio, Some(0.3));
    assert_eq!(session.font_size, Some(16.0));
}

#[test]
fn a_focused_pane_the_row_no_longer_has_is_ignored() {
    let (mut store, id) = split_store();
    let session = store.session_mut(id).unwrap();
    session.split.focused = true;
    let view = SessionView {
        focused_pane: Some(Uuid::new_v4()),
        ..SessionView::default()
    };
    view.apply(session);
    assert!(!session.split.focused);
}

#[test]
fn the_capture_leaves_out_default_rows_and_round_trips_through_its_file() {
    let (mut store, id) = split_store();
    let workspace = store.workspaces()[0].id();
    let plain = store.add_session(workspace, NewSession::at_cwd("/tmp")).unwrap();
    store.set_font_size(id, 15.0);
    store.set_workspace_expanded(workspace, false);
    let state = RemoteWindowViewState::capture(&store);
    assert_eq!(state.selected_session_id, Some(plain));
    assert_eq!(state.collapsed_workspaces, vec![workspace]);
    assert!(state.sessions.contains_key(&id));
    assert!(!state.sessions.contains_key(&plain));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("remote").join("w.view.json");
    state.save(&path).unwrap();
    assert_eq!(RemoteWindowViewState::load(&path), state);
    assert_eq!(
        RemoteWindowViewState::load(&dir.path().join("missing")),
        RemoteWindowViewState::default()
    );
}

#[test]
fn the_file_names_every_id_in_upper_case() {
    let (mut store, id) = split_store();
    let workspace = store.workspaces()[0].id();
    store.session_mut(id).unwrap().split.focused = true;
    store.set_font_size(id, 15.0);
    store.set_workspace_expanded(workspace, false);
    let state = RemoteWindowViewState::capture(&store);
    let focused = state.sessions[&id].focused_pane.unwrap();
    let text = serde_json::to_string(&state).unwrap();
    for uuid in [id, workspace, focused] {
        let upper = uuid.to_string().to_uppercase();
        assert!(text.contains(&upper), "{uuid} in {text}");
        assert!(!text.contains(&uuid.to_string()), "{uuid} in {text}");
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("w.view.json");
    state.save(&path).unwrap();
    assert_eq!(RemoteWindowViewState::load(&path), state);
}

#[test]
fn the_sessions_are_one_array_of_alternating_ids_and_views_as_upstream_writes_them() {
    let id = Uuid::new_v4();
    let view = SessionView {
        font_size: Some(15.0),
        ..SessionView::default()
    };
    let state = RemoteWindowViewState {
        sessions: [(id, view.clone())].into(),
        ..RemoteWindowViewState::default()
    };
    let upper = id.to_string().to_uppercase();
    let written = serde_json::to_value(&state).unwrap();
    assert_eq!(written["sessions"], serde_json::json!([upper, {"fontSize": 15.0}]));
    let upstream = serde_json::json!({"sessions": [upper, {"fontSize": 15.0}]});
    assert_eq!(
        serde_json::from_value::<RemoteWindowViewState>(upstream).unwrap(),
        state
    );
    let keyed = serde_json::json!({"sessions": {upper.clone(): {"fontSize": 15.0}}});
    assert_eq!(serde_json::from_value::<RemoteWindowViewState>(keyed).unwrap(), state);
    let odd = serde_json::json!({"sessions": [upper]});
    assert!(serde_json::from_value::<RemoteWindowViewState>(odd).is_err());
}

#[test]
fn an_older_view_state_file_decodes_with_nothing_detached() {
    let state: RemoteWindowViewState = serde_json::from_str(r#"{"collapsedWorkspaces":[],"sessions":[]}"#).unwrap();
    assert!(state.detached_sessions.is_empty());
    assert!(state.detached_workspaces.is_empty());
}

#[test]
fn the_capture_keeps_what_a_detach_hides_in_upper_case() {
    let mut store = AppStore::new();
    let shown = store.add_workspace("w");
    let (hidden_row, hidden_workspace) = (Uuid::new_v4(), Uuid::new_v4());
    store.catalog_detached_sessions.insert(hidden_row);
    store.catalog_detached_workspaces.insert(hidden_workspace);
    store.catalog_hidden_views.insert(
        hidden_row,
        SessionView {
            font_size: Some(19.0),
            ..SessionView::default()
        },
    );
    store.catalog_hidden_collapsed.insert(hidden_workspace, true);
    store.catalog_hidden_collapsed.insert(shown, false);

    let state = RemoteWindowViewState::capture(&store);

    assert_eq!(state.detached_sessions, vec![hidden_row]);
    assert_eq!(state.detached_workspaces, vec![hidden_workspace]);
    assert_eq!(state.collapsed_workspaces, vec![hidden_workspace]);
    assert_eq!(
        state.sessions.get(&hidden_row).and_then(|view| view.font_size),
        Some(19.0)
    );
    let json = serde_json::to_value(&state).unwrap();
    let upper = hidden_row.to_string().to_uppercase();
    assert_eq!(json["detachedSessions"], serde_json::json!([upper]));
}
