//! Detach and reattach in a remote window (upstream's `RemoteCatalogMirror+DetachTests`, `ControlDispatcher+DetachTests`
//! and `ControlServer+Detach.swift`): a detached row or workspace is hidden here, left running on the server, and saved
//! in the window's view state.

use super::*;
use crate::catalog::fixture::companion;
use crate::remote::catalog::{CloseScope, DetachedItem, DetachedKind};

const W: Uuid = Uuid::from_u128(0x1111_0000_0000_4000_8000_0000_0000_0001);
const OTHER: Uuid = Uuid::from_u128(0x1111_0000_0000_4000_8000_0000_0000_0002);

impl Remote {
    /// A connected remote window showing `sessions` in workspace `W` and `other` in workspace `OTHER`.
    fn with_two(sessions: Vec<frames::CatalogSession>, other: Vec<frames::CatalogSession>) -> Self {
        let mut remote = Self::opened(&server_version());
        remote.snapshot(snapshot(vec![
            workspace(W, "w", sessions),
            workspace(OTHER, "other", other),
        ]));
        remote
    }

    /// The same, saving into a fresh state directory.
    fn persisted(dir: &std::path::Path) -> Self {
        let mut remote = Self::new(&server_version());
        remote.fixture.core.persistence = Some(LibraryPersistence::new(dir));
        remote.window = answered_id(&remote.open("box")).unwrap();
        remote.settle();
        remote
    }

    /// Sends `cmd` and commits, as the frontend does after every request.
    fn command(&mut self, cmd: Command, target: Option<&str>, args: Args) -> Response {
        let response = self.fixture.send(cmd, target, args);
        self.fixture.core.commit();
        response
    }

    fn detach(&mut self, ids: &[Uuid]) -> Response {
        let targets = ids.iter().copied().map(wire).collect();
        self.command(
            Command::Session(SessionCmd::Detach),
            None,
            args(|a| a.targets = Some(targets)),
        )
    }

    fn reattach(&mut self, id: Uuid) -> Response {
        let target = wire(id);
        self.command(Command::Session(SessionCmd::Reattach), Some(&target), Args::default())
    }

    fn detach_workspace(&mut self, id: Uuid) -> Response {
        let target = wire(id);
        self.command(Command::Workspace(WorkspaceCmd::Detach), Some(&target), Args::default())
    }

    fn reattach_workspace(&mut self, id: Uuid) -> Response {
        let target = wire(id);
        self.command(
            Command::Workspace(WorkspaceCmd::Reattach),
            Some(&target),
            Args::default(),
        )
    }

    fn detached(&self) -> Vec<Uuid> {
        self.fixture
            .core
            .detached_items(self.window)
            .iter()
            .map(|item| item.id)
            .collect()
    }

    fn set_font(&mut self, id: Uuid, size: f64) {
        self.fixture
            .core
            .library_mut()
            .store_mut(self.window)
            .and_then(|store| store.session_mut(id))
            .expect("the row")
            .font_size = Some(size);
    }

    fn view_path(&self) -> std::path::PathBuf {
        self.fixture
            .core
            .persistence
            .as_ref()
            .unwrap()
            .remote_view_path(self.window)
    }

    fn reopen(&mut self) {
        self.fixture.core.close_window(self.window);
        let _ = self.fixture.core.commit();
        assert_eq!(answered_id(&self.open("box")), Some(self.window));
        self.settle();
    }
}

fn ended(id: Uuid) -> frames::CatalogSessionFields {
    let mut fields = catalog::fields(id, "s", Uuid::new_v4(), false);
    fields.lifecycle = Lifecycle::Ended;
    fields
}

fn presentation(remote: &Remote, id: Uuid) -> Option<Link> {
    remote
        .links()
        .into_iter()
        .rev()
        .find(|link| link.0.borrow().key.is_some_and(|key| key.session == id))
}

#[test]
fn a_detached_row_leaves_this_machine_stays_running_and_is_saved() {
    let dir = tempfile::tempdir().unwrap();
    let mut remote = Remote::persisted(dir.path());
    let (a, b) = (session("a"), session("b"));
    remote.snapshot(snapshot(vec![workspace(W, "w", vec![a.clone(), b.clone()])]));
    let stream = presentation(&remote, a.id()).expect("a's stream");

    let response = remote.detach(&[a.id()]);

    assert!(response.ok, "{response:?}");
    assert_eq!(response.result.unwrap()["affected"], json!(1));
    assert_eq!(remote.rows(), vec![b.id()]);
    assert!(stream.0.borrow().stopped, "the hidden row's stream ends");
    assert!(remote.runner.operations().is_empty(), "nothing is sent to the server");
    assert_eq!(
        remote.window_node()["remote"]["detachedSessions"],
        json!([wire(a.id())])
    );
    assert_eq!(
        RemoteWindowViewState::load(&remote.view_path()).detached_sessions,
        vec![a.id()]
    );
}

#[test]
fn detaching_ends_the_scratch_attach_and_reattaching_attaches_it_again() {
    let mut a = session("a");
    let scratch = pane();
    a.fields.scratch = Some(CatalogScratch {
        pane: scratch.clone(),
        shown: true,
    });
    let mut remote = Remote::with_two(vec![a.clone(), session("b")], vec![]);

    remote.detach(&[a.id()]);
    assert!(remote.store().session(a.id()).is_none());
    let response = remote.reattach(a.id());

    assert_eq!(answered_id(&response), Some(a.id()));
    let row = remote.row(a.id());
    assert!(row.scratch_active);
    assert!(readable(row.scratch_command.as_deref().unwrap()).contains(&scratch.daemon));
    assert!(presentation(&remote, a.id()).is_some_and(|link| !link.0.borrow().stopped));
}

#[test]
fn a_detached_parent_hides_its_companions_and_ones_made_later() {
    let parent = session("p");
    let child = companion("c", parent.id());
    let mut remote = Remote::with_two(vec![parent.clone(), child], vec![]);

    remote.detach(&[parent.id()]);
    remote.delta(
        1,
        CatalogChange::SessionAdded {
            session: companion("later", parent.id()),
            workspace: W,
            index: 2,
        },
    );

    assert!(remote.rows().is_empty());
    assert_eq!(remote.detached(), vec![parent.id()]);
}

#[test]
fn a_detached_parent_respawned_on_the_server_stays_hidden_through_every_frame() {
    let parent = session("p");
    let child = companion("c", parent.id());
    let mut remote = Remote::with_two(vec![parent.clone(), child.clone()], vec![]);
    remote.detach(&[parent.id()]);
    let links = remote.links().len();
    let before = snapshot(vec![
        workspace(W, "w", vec![parent.clone(), child.clone()]),
        workspace(OTHER, "other", vec![]),
    ]);
    let mut after = before.clone();
    after.workspaces[0].sessions[0].fields.cwd = "/home/ngterm".to_owned();
    after.workspaces[0].sessions[0].panes = vec![pane()];
    let changes = before.changes(&after);
    assert!(!changes.is_empty());

    let mut shown = Vec::new();
    for (index, change) in changes.iter().enumerate() {
        remote.delta(i64::try_from(index).unwrap() + 1, change.clone());
        shown.push(remote.rows());
    }
    after.rev = i64::try_from(changes.len()).unwrap() + 1;
    remote.snapshot(after);
    shown.push(remote.rows());

    assert!(shown.iter().all(Vec::is_empty), "{shown:?}");
    assert_eq!(remote.links().len(), links, "no hidden row opens a stream");
    assert_eq!(
        remote.fixture.core.detached_items(remote.window),
        vec![DetachedItem {
            kind: DetachedKind::Session,
            id: parent.id(),
            name: "p".to_owned(),
            ended: false,
        }]
    );
}

#[test]
fn a_detached_split_promoted_on_the_server_stays_hidden_through_every_frame() {
    let (left, right) = (pane(), pane());
    let entry = session_with(Uuid::new_v4(), "s", vec![left.clone(), right.clone()]);
    let mut remote = Remote::with_two(vec![entry.clone(), session("b")], vec![]);
    remote.detach(&[entry.id()]);
    let mut promoted = entry.fields.clone();
    promoted.primary = right.identity;
    promoted.axis = None;
    promoted.shown = false;

    remote.delta(
        1,
        CatalogChange::PaneRemoved {
            identity: left.identity,
            session: entry.id(),
        },
    );
    let after_pane = remote.rows().contains(&entry.id());
    remote.delta(2, CatalogChange::SessionChanged(promoted));

    assert!(!after_pane);
    assert!(!remote.rows().contains(&entry.id()));
    assert_eq!(remote.detached(), vec![entry.id()]);
}

#[test]
fn a_companion_that_exits_under_a_detached_parent_leaves_the_rest_of_the_group_hidden() {
    let parent = session("p");
    let (exiting, staying) = (companion("x", parent.id()), companion("y", parent.id()));
    let other = session("o");
    let mut remote = Remote::with_two(
        vec![parent.clone(), exiting.clone(), staying.clone(), other.clone()],
        vec![],
    );
    remote.detach(&[parent.id()]);

    remote.delta(1, CatalogChange::SessionRemoved(exiting.id()));

    assert_eq!(remote.rows(), vec![other.id()]);
    assert_eq!(remote.detached(), vec![parent.id()]);
    let scope = remote.fixture.core.remote_close_scope(remote.window, parent.id());
    assert_eq!(scope.map(|scope| scope.sessions), Some(vec![parent.id(), staying.id()]));
}

#[test]
fn a_detached_row_stays_hidden_once_ended_and_refuses_reattach() {
    let a = session("a");
    let mut remote = Remote::with_two(vec![a.clone(), session("b")], vec![]);
    remote.detach(&[a.id()]);

    remote.delta(1, CatalogChange::SessionChanged(ended(a.id())));

    assert!(!remote.rows().contains(&a.id()));
    assert_eq!(
        remote.fixture.core.detached_items(remote.window),
        vec![DetachedItem {
            kind: DetachedKind::Session,
            id: a.id(),
            name: "s".to_owned(),
            ended: true,
        }]
    );
    let response = remote.reattach(a.id());
    assert_eq!(
        response.error.as_deref(),
        Some("the session has ended; session.close removes its record")
    );
    assert!(remote.store().catalog_detached_sessions.contains(&a.id()));
}

#[test]
fn a_detached_id_is_forgotten_only_when_its_record_goes_and_that_is_saved() {
    let dir = tempfile::tempdir().unwrap();
    let mut remote = Remote::persisted(dir.path());
    let a = session("a");
    remote.snapshot(snapshot(vec![workspace(W, "w", vec![a.clone(), session("b")])]));
    remote.detach(&[a.id()]);
    remote.delta(1, CatalogChange::SessionChanged(ended(a.id())));
    assert_eq!(remote.store().catalog_detached_sessions.len(), 1);

    remote.delta(2, CatalogChange::SessionRemoved(a.id()));

    assert!(remote.store().catalog_detached_sessions.is_empty());
    assert!(
        RemoteWindowViewState::load(&remote.view_path())
            .detached_sessions
            .is_empty()
    );
}

#[test]
fn a_reattached_row_comes_back_with_its_view() {
    let a = session("a");
    let mut remote = Remote::with_two(vec![a.clone(), session("b")], vec![]);
    remote.set_font(a.id(), 17.0);
    remote.detach(&[a.id()]);

    assert_eq!(answered_id(&remote.reattach(a.id())), Some(a.id()));

    assert_eq!(remote.row(a.id()).font_size, Some(17.0));
    assert_eq!(remote.rows()[0], a.id(), "it comes back in its catalog place");
}

#[test]
fn a_detached_workspace_hides_every_row_and_an_empty_one_too() {
    let inside = session("in");
    let mut remote = Remote::with_two(vec![session("a")], vec![inside.clone()]);

    assert!(remote.detach_workspace(OTHER).ok);
    let hidden: Vec<Uuid> = remote
        .store()
        .workspaces()
        .iter()
        .map(crate::model::Workspace::id)
        .collect();
    assert!(remote.detach_workspace(W).ok);

    assert_eq!(hidden, vec![W]);
    assert!(remote.store().workspaces().is_empty());
    assert_eq!(
        remote
            .fixture
            .core
            .detached_items(remote.window)
            .iter()
            .map(|item| item.kind)
            .collect::<Vec<_>>(),
        vec![DetachedKind::Workspace, DetachedKind::Workspace]
    );
    assert_eq!(
        remote.window_node()["remote"]["detachedWorkspaces"],
        json!([wire(W), wire(OTHER)])
    );
    assert_eq!(answered_id(&remote.reattach_workspace(OTHER)), Some(OTHER));
    assert_eq!(remote.rows(), vec![inside.id()]);
}

#[test]
fn a_child_detached_before_its_parent_comes_back_on_its_own() {
    let parent = session("p");
    let child = companion("c", parent.id());
    let mut remote = Remote::with_two(vec![parent.clone(), child.clone()], vec![]);

    remote.detach(&[child.id()]);
    remote.detach(&[parent.id()]);
    let listed = remote.detached();
    let refused = remote.reattach(child.id());
    remote.reattach(parent.id());

    assert_eq!(listed, vec![parent.id()]);
    assert_eq!(
        refused.error.as_deref(),
        Some("its parent is detached; reattach the parent")
    );
    assert_eq!(remote.rows(), vec![parent.id()]);
    assert_eq!(remote.detached(), vec![child.id()]);
}

#[test]
fn a_session_under_a_detached_workspace_is_not_listed_and_refuses_reattach() {
    let inside = session("in");
    let mut remote = Remote::with_two(vec![session("a")], vec![inside.clone()]);

    remote.detach(&[inside.id()]);
    remote.detach_workspace(OTHER);

    assert_eq!(remote.detached(), vec![OTHER]);
    assert_eq!(
        remote.reattach(inside.id()).error.as_deref(),
        Some("its workspace is detached; reattach the workspace")
    );
}

#[test]
fn detaching_while_disconnected_hides_the_row() {
    let a = session("a");
    let mut remote = Remote::with_two(vec![a.clone(), session("b")], vec![]);
    let link = remote.link();
    remote.close(&link, "gone");
    assert!(matches!(remote.state(), CatalogState::Disconnected(_)));

    assert!(remote.detach(&[a.id()]).ok);

    assert!(!remote.rows().contains(&a.id()));
}

#[test]
fn detached_state_and_hidden_views_outlive_the_window() {
    let dir = tempfile::tempdir().unwrap();
    let mut remote = Remote::persisted(dir.path());
    let (a, b) = (session("a"), session("b"));
    let catalog = snapshot(vec![workspace(W, "w", vec![a.clone(), b.clone()])]);
    remote.snapshot(catalog.clone());
    remote.set_font(a.id(), 19.0);
    remote.detach(&[a.id()]);

    remote.reopen();
    remote.snapshot(catalog);
    let hidden_after_reopen = remote.store().session(a.id()).is_none();
    let saved = RemoteWindowViewState::load(&remote.view_path());
    remote.reattach(a.id());

    assert_eq!(saved.detached_sessions, vec![a.id()]);
    assert_eq!(saved.sessions.get(&a.id()).and_then(|view| view.font_size), Some(19.0));
    assert!(hidden_after_reopen);
    assert_eq!(remote.row(a.id()).font_size, Some(19.0));
}

#[test]
fn closing_a_remote_window_detaches_it_and_its_view_is_saved() {
    let dir = tempfile::tempdir().unwrap();
    let mut remote = Remote::persisted(dir.path());
    let (a, b) = (session("a"), session("b"));
    remote.snapshot(snapshot(vec![workspace(W, "w", vec![a.clone(), b.clone()])]));
    remote.set_font(b.id(), 15.0);

    remote.fixture.core.close_window(remote.window);
    let _ = remote.fixture.core.commit();

    assert!(remote.runner.operations().is_empty(), "nothing ends on the server");
    let saved = RemoteWindowViewState::load(&remote.view_path());
    assert_eq!(saved.sessions.get(&b.id()).and_then(|view| view.font_size), Some(15.0));
}

#[test]
fn an_ended_row_with_a_running_scratch_counts_as_live() {
    let mut ended_row = session("e");
    ended_row.fields.lifecycle = Lifecycle::Ended;
    ended_row.panes.clear();
    ended_row.fields.scratch = Some(CatalogScratch {
        pane: pane(),
        shown: false,
    });
    let remote = Remote::with_two(vec![ended_row.clone()], vec![]);

    assert_eq!(
        remote.fixture.core.remote_close_scope(remote.window, ended_row.id()),
        Some(CloseScope {
            sessions: vec![ended_row.id()],
            live: true,
        })
    );
}

#[test]
fn closing_a_parent_reaches_its_companions_and_counts_them_as_live() {
    let mut parent = session("p");
    parent.fields.lifecycle = Lifecycle::Ended;
    parent.panes.clear();
    let child = companion("c", parent.id());
    let mut inert = session("i");
    inert.fields.lifecycle = Lifecycle::Ended;
    inert.panes.clear();
    let remote = Remote::with_two(vec![parent.clone(), child.clone(), inert.clone()], vec![]);
    let scope = |id| remote.fixture.core.remote_close_scope(remote.window, id);

    assert_eq!(
        scope(parent.id()),
        Some(CloseScope {
            sessions: vec![parent.id(), child.id()],
            live: true,
        })
    );
    assert_eq!(
        scope(inert.id()),
        Some(CloseScope {
            sessions: vec![inert.id()],
            live: false,
        })
    );
    assert_eq!(
        scope(child.id()),
        Some(CloseScope {
            sessions: vec![child.id()],
            live: true,
        })
    );
    assert_eq!(scope(Uuid::new_v4()), None);
}

#[test]
fn a_workspace_terminates_only_with_another_on_the_server_detached_ones_counted() {
    let mut remote = Remote::with_two(vec![session("a")], vec![session("b")]);
    remote.detach_workspace(OTHER);
    assert!(remote.fixture.core.can_terminate_remote_workspace(remote.window));

    remote.snapshot(snapshot(vec![workspace(W, "w", vec![session("a")])]));
    assert!(!remote.fixture.core.can_terminate_remote_workspace(remote.window));
}

#[test]
fn a_row_reset_to_defaults_before_detach_comes_back_at_the_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let mut remote = Remote::persisted(dir.path());
    let a = session("a");
    let mut saved = RemoteWindowViewState::default();
    saved.sessions.insert(
        a.id(),
        SessionView {
            font_size: Some(21.0),
            ..SessionView::default()
        },
    );
    remote.fixture.core.close_window(remote.window);
    let _ = remote.fixture.core.commit();
    saved.save(&remote.view_path()).unwrap();
    assert_eq!(answered_id(&remote.open("box")), Some(remote.window));
    remote.settle();
    remote.snapshot(snapshot(vec![workspace(W, "w", vec![a.clone(), session("b")])]));
    let restored = remote.row(a.id()).font_size;
    remote.store_mut_row(a.id()).font_size = None;

    remote.detach(&[a.id()]);
    remote.reattach(a.id());

    assert_eq!(restored, Some(21.0));
    assert_eq!(remote.row(a.id()).font_size, None);
}

#[test]
fn a_detached_workspace_keeps_its_collapse_through_a_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let mut remote = Remote::persisted(dir.path());
    let catalog = snapshot(vec![
        workspace(W, "w", vec![session("a")]),
        workspace(OTHER, "o", vec![session("b")]),
    ]);
    remote.snapshot(catalog.clone());
    remote
        .fixture
        .core
        .library_mut()
        .store_mut(remote.window)
        .unwrap()
        .set_workspace_expanded(OTHER, false);

    remote.detach_workspace(OTHER);
    remote.reopen();
    remote.snapshot(catalog);
    let saved = RemoteWindowViewState::load(&remote.view_path());
    remote.reattach_workspace(OTHER);

    assert_eq!(saved.collapsed_workspaces, vec![OTHER]);
    assert!(!remote.store().workspace(OTHER).unwrap().is_expanded);
}

#[test]
fn a_workspace_expanded_before_detach_comes_back_expanded() {
    let dir = tempfile::tempdir().unwrap();
    let mut remote = Remote::persisted(dir.path());
    remote.fixture.core.close_window(remote.window);
    let _ = remote.fixture.core.commit();
    let saved = RemoteWindowViewState {
        collapsed_workspaces: vec![OTHER],
        ..RemoteWindowViewState::default()
    };
    saved.save(&remote.view_path()).unwrap();
    assert_eq!(answered_id(&remote.open("box")), Some(remote.window));
    remote.settle();
    remote.snapshot(snapshot(vec![
        workspace(W, "w", vec![session("a")]),
        workspace(OTHER, "o", vec![session("b")]),
    ]));
    remote
        .fixture
        .core
        .library_mut()
        .store_mut(remote.window)
        .unwrap()
        .set_workspace_expanded(OTHER, true);

    remote.detach_workspace(OTHER);
    remote.reattach_workspace(OTHER);

    assert!(remote.store().workspace(OTHER).unwrap().is_expanded);
}

#[test]
fn a_close_by_id_reaches_a_detached_row_on_the_server() {
    let (a, b) = (session("a"), session("b"));
    let mut remote = Remote::with_two(vec![a.clone(), b.clone()], vec![]);
    remote.detach(&[a.id()]);

    let closed = remote.command(
        Command::Session(SessionCmd::Close),
        Some(&wire(a.id())),
        Args::default(),
    );

    assert!(closed.ok, "{closed:?}");
    let operations = remote.runner.operations();
    assert_eq!(operations.len(), 1, "{operations:?}");
    assert!(
        operations[0].contains(&format!("session close --target {} --op-id", wire(a.id()))),
        "{operations:?}"
    );
    let missing = remote.command(Command::Session(SessionCmd::Close), Some("zzz"), Args::default());
    assert_eq!(missing.error.as_deref(), Some("no such session: zzz"));
}

// MARK: - the commands

#[test]
fn every_detach_command_on_a_local_window_is_refused_after_its_target_resolves() {
    let mut remote = Remote::opened(&server_version());
    let local = wire(remote.fixture.window);
    for cmd in [
        Command::Session(SessionCmd::Detach),
        Command::Workspace(WorkspaceCmd::Detach),
    ] {
        let response = remote
            .fixture
            .send(cmd, Some("active"), args(|a| a.window = Some(local.clone())));
        assert_eq!(
            response.error.as_deref(),
            Some(format!("{cmd} acts only on a remote window").as_str())
        );
    }
    let missing = remote.fixture.send(
        Command::Session(SessionCmd::Detach),
        Some("zzz"),
        args(|a| a.window = Some(local.clone())),
    );
    assert_eq!(missing.error.as_deref(), Some("no such session: zzz"));
}

#[test]
fn a_reattach_without_an_id_is_refused_before_any_window() {
    let mut remote = Remote::opened(&server_version());
    for (cmd, error) in [
        (
            Command::Session(SessionCmd::Reattach),
            "session.reattach requires a detached session's id",
        ),
        (
            Command::Workspace(WorkspaceCmd::Reattach),
            "workspace.reattach requires a detached workspace's id",
        ),
    ] {
        let missing = remote.fixture.send(cmd, None, Args::default());
        let active = remote.fixture.send(cmd, Some("Active"), Args::default());
        assert_eq!(missing.error.as_deref(), Some(error));
        assert_eq!(active.error.as_deref(), Some(error));
    }
}

#[test]
fn an_empty_detach_batch_is_refused() {
    let mut remote = Remote::opened(&server_version());
    let response = remote.fixture.send(
        Command::Session(SessionCmd::Detach),
        None,
        args(|a| a.targets = Some(Vec::new())),
    );
    assert_eq!(
        response.error.as_deref(),
        Some("session.detach requires at least one --target")
    );
}

#[test]
fn a_detach_batch_takes_rows_of_one_window() {
    let a = session("a");
    let mut remote = Remote::with_two(vec![a.clone()], vec![]);
    let local = remote
        .fixture
        .core
        .library()
        .store(remote.fixture.window)
        .unwrap()
        .sessions()
        .next()
        .unwrap()
        .id();

    let response = remote.detach(&[a.id(), local]);

    assert_eq!(
        response.error.as_deref(),
        Some("session.detach takes sessions of one window")
    );
    assert_eq!(remote.rows(), vec![a.id()]);
}

#[test]
fn a_reattach_names_what_is_not_detached_and_an_ambiguous_prefix() {
    let (a, b) = (
        session_with(
            Uuid::from_u128(0xAB00_0000_0000_4000_8000_0000_0000_0001),
            "a",
            vec![pane()],
        ),
        session_with(
            Uuid::from_u128(0xAB00_0000_0000_4000_8000_0000_0000_0002),
            "b",
            vec![pane()],
        ),
    );
    let mut remote = Remote::with_two(vec![a.clone(), b.clone(), session("c")], vec![]);
    assert_eq!(
        remote.reattach(a.id()).error.as_deref(),
        Some(format!("no such detached session: {}", wire(a.id())).as_str())
    );
    assert_eq!(
        remote.reattach_workspace(W).error.as_deref(),
        Some(format!("no such detached workspace: {}", wire(W)).as_str())
    );
    remote.detach(&[a.id(), b.id()]);

    let ambiguous = remote
        .fixture
        .send(Command::Session(SessionCmd::Reattach), Some("ab"), Args::default());

    assert!(
        ambiguous
            .error
            .as_deref()
            .is_some_and(|error| error.starts_with("ambiguous detached session prefix 'ab'")),
        "{ambiguous:?}"
    );
    let local = wire(remote.fixture.window);
    let elsewhere = remote.fixture.send(
        Command::Session(SessionCmd::Reattach),
        Some(&wire(a.id())),
        args(|args| args.window = Some(local)),
    );
    assert!(
        elsewhere
            .error
            .is_some_and(|error| error.starts_with("no such detached session"))
    );
}

impl Remote {
    fn store_mut_row(&mut self, id: Uuid) -> &mut Session {
        self.fixture
            .core
            .library_mut()
            .store_mut(self.window)
            .and_then(|store| store.session_mut(id))
            .expect("the row")
    }
}
