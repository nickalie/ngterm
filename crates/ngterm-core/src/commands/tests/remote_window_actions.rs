//! A remote window's actions (upstream's `ControlServerRemoteActionsTests` and `AppActions+RemoteCatalogTests`): what
//! makes, changes or ends a row goes to the server with an operation id and changes nothing here until the catalog
//! says so; what would make a shell or a row on this machine is refused.

use super::*;

impl Remote {
    /// A connected remote window showing `sessions` in one workspace, `w`.
    fn showing(w: Uuid, sessions: Vec<frames::CatalogSession>) -> Self {
        let mut remote = Self::opened(&server_version());
        remote.snapshot(snapshot(vec![workspace(w, "w", sessions)]));
        remote
    }

    /// Ticks until no ssh run of the window is out.
    fn settle_actions(&mut self) {
        for _ in 0..2000 {
            self.fixture.core.remote_tick(Instant::now());
            self.fixture.core.commit();
            if !self.fixture.core.catalog_wake().0 {
                return;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("the window's actions never settled");
    }

    fn row_node(&mut self, id: Uuid) -> Value {
        let tree = self.tree();
        let sessions = tree["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|workspace| workspace["sessions"].as_array().unwrap().clone())
            .collect::<Vec<Value>>();
        sessions
            .into_iter()
            .find(|node| node["id"] == json!(wire(id)))
            .expect("the row")
    }

    fn store_mut(&mut self) -> &mut AppStore {
        self.fixture
            .core
            .library_mut()
            .store_mut(self.window)
            .expect("the remote window")
    }

    fn dismiss(&mut self) -> Response {
        let window = wire(self.window);
        self.fixture.send(
            Command::Window(WindowCmd::RemoteDismiss),
            Some(&window),
            Args::default(),
        )
    }
}

fn session_cmd(cmd: SessionCmd) -> Command {
    Command::Session(cmd)
}

#[test]
fn a_command_that_makes_a_local_shell_or_row_is_refused_and_sends_nothing() {
    let a = session("a");
    let mut remote = Remote::showing(Uuid::new_v4(), vec![a.clone()]);
    let before = remote.rows();
    let window = wire(remote.window);
    let row = wire(a.id());
    let cases = [
        (
            session_cmd(SessionCmd::CompanionConvert),
            Some(row.clone()),
            "session.companion.convert",
        ),
        (
            session_cmd(SessionCmd::CompanionDuplicate),
            Some(row.clone()),
            "session.companion.duplicate",
        ),
        (session_cmd(SessionCmd::Page), Some(row.clone()), "session.page"),
        (
            session_cmd(SessionCmd::CompanionPage),
            Some(row),
            "session.companion.page",
        ),
        (Command::Live(LiveCmd::Attach), None, "live.attach"),
        (Command::Live(LiveCmd::AttachGroup), None, "live.attach.group"),
        (Command::Recent(RecentCmd::Open), None, "recent.open"),
    ];
    for (cmd, target, name) in cases {
        for scope in [None, Some(window.clone())] {
            let response = remote
                .fixture
                .send(cmd, target.as_deref(), args(|a| a.window.clone_from(&scope)));
            assert_eq!(
                response.error,
                Some(format!("{name} is not available in a remote window")),
                "{scope:?}"
            );
        }
    }
    assert_eq!(remote.rows(), before);
    assert!(remote.runner.operations().is_empty());
}

#[test]
fn split_rename_close_and_move_of_a_row_go_to_the_server_and_change_nothing_here() {
    let (a, b, w, other) = (
        session_with(Uuid::new_v4(), "a", vec![pane(), pane()]),
        session("b"),
        Uuid::new_v4(),
        Uuid::new_v4(),
    );
    let mut remote = Remote::opened(&server_version());
    remote.snapshot(snapshot(vec![
        workspace(w, "w", vec![a.clone(), b.clone()]),
        workspace(other, "x", vec![]),
    ]));
    let name = remote.row(a.id()).display_name();
    let (row, b_row) = (wire(a.id()), wire(b.id()));
    let requests: Vec<(Command, Option<String>, Args)> = vec![
        (
            session_cmd(SessionCmd::Split),
            Some(row.clone()),
            args(|a| a.mode = Some("off".to_owned())),
        ),
        (
            session_cmd(SessionCmd::Rename),
            Some(row.clone()),
            args(|a| a.name = Some("-renamed".to_owned())),
        ),
        (
            session_cmd(SessionCmd::Move),
            Some(row.clone()),
            args(|a| a.workspace = Some(wire(other))),
        ),
        (
            session_cmd(SessionCmd::Close),
            None,
            args(|a| a.targets = Some(vec![b_row.clone()])),
        ),
    ];
    for (cmd, target, args) in requests {
        let response = remote.fixture.send(cmd, target.as_deref(), args);
        assert!(response.ok, "{cmd}: {response:?}");
    }
    let operations = remote.runner.operations();
    let expected = [
        format!("session split visibility off --target {row} --expect-panes"),
        format!("session rename --target {row} --op-id"),
        format!("session move --target {row} --op-id"),
        format!("session close --target {b_row} --op-id"),
    ];
    assert_eq!(operations.len(), expected.len(), "{operations:?}");
    for (line, expected) in operations.iter().zip(&expected) {
        assert!(line.contains(expected.as_str()), "{line} lacks {expected}");
        assert!(line.contains(" --epoch e1 --json"), "{line}");
    }
    assert!(operations[1].ends_with("--json -- -renamed"), "{}", operations[1]);
    assert!(
        operations[2].ends_with(&format!("--json -- {}", wire(other))),
        "{}",
        operations[2]
    );
    assert_eq!(remote.rows(), vec![a.id(), b.id()]);
    assert_eq!(remote.row(a.id()).display_name(), name);
    assert!(
        remote.row(a.id()).split.shown,
        "the split stays shown until the catalog hides it"
    );
    assert_eq!(remote.store().workspace(other).map(|w| w.sessions.len()), Some(0));
}

#[test]
fn workspace_commands_go_to_the_server() {
    let w = Uuid::new_v4();
    let mut remote = Remote::showing(w, vec![session("a")]);
    let target = wire(w);
    for (cmd, args) in [
        (WorkspaceCmd::New, args(|a| a.name = Some("dev".to_owned()))),
        (WorkspaceCmd::Rename, args(|a| a.name = Some("ops".to_owned()))),
        (WorkspaceCmd::Move, args(|a| a.to = Some("top".to_owned()))),
        (WorkspaceCmd::Delete, Args::default()),
    ] {
        let target = (cmd != WorkspaceCmd::New).then_some(target.as_str());
        let response = remote.fixture.send(Command::Workspace(cmd), target, args);
        assert!(response.ok, "{cmd:?}: {response:?}");
    }
    let operations = remote.runner.operations();
    assert!(operations[0].contains("workspace new --op-id"), "{}", operations[0]);
    assert!(operations[1].contains(&format!("workspace rename --target {target} --op-id")));
    assert!(operations[2].contains(&format!("workspace move --target {target} --to top --op-id")));
    assert!(operations[3].contains(&format!("workspace delete --target {target} --op-id")));
    assert_eq!(remote.store().workspaces().len(), 1);
}

#[test]
fn a_local_row_command_runs_here_and_sends_nothing() {
    let mut remote = Remote::showing(Uuid::new_v4(), vec![session("a")]);
    let local = wire(remote.fixture.window);
    let created = remote.fixture.send(
        session_cmd(SessionCmd::New),
        None,
        args(|a| a.window = Some(local.clone())),
    );
    let id = answered_id(&created).expect("a local session");
    let closed = remote
        .fixture
        .send(session_cmd(SessionCmd::Close), Some(&wire(id)), Args::default());
    assert!(closed.ok, "{closed:?}");
    assert!(remote.runner.operations().is_empty());
}

#[test]
fn session_new_in_a_background_remote_window_answers_the_servers_row_and_selects_it_when_it_arrives() {
    let (a, w) = (session("a"), Uuid::new_v4());
    let mut remote = Remote::showing(w, vec![a.clone()]);
    let local = remote.fixture.window;
    remote.fixture.core.take_frontmost(local);
    let created = session("new");
    *remote.runner.created.lock().unwrap() = Some(created.id());
    let window = wire(remote.window);
    let response = remote
        .fixture
        .send(session_cmd(SessionCmd::New), None, args(|a| a.window = Some(window)));
    assert_eq!(answered_id(&response), Some(created.id()), "{response:?}");
    assert!(remote.runner.operations()[0].contains(&format!("session new --workspace {}", wire(w))));
    assert_eq!(remote.store().selected_session_id(), Some(a.id()));
    remote.delta(
        1,
        CatalogChange::SessionAdded {
            session: created.clone(),
            workspace: w,
            index: 1,
        },
    );
    assert_eq!(remote.store().selected_session_id(), Some(created.id()));
}

#[test]
fn a_created_row_is_not_selected_when_the_user_moved_during_the_send() {
    let (a, b, w) = (session("a"), session("b"), Uuid::new_v4());
    let mut remote = Remote::showing(w, vec![a.clone(), b.clone()]);
    let created = session("new");
    *remote.runner.created.lock().unwrap() = Some(created.id());
    let window = remote.window;
    remote.store_mut().select_session(Some(a.id()));
    assert!(
        remote
            .fixture
            .core
            .sidebar_action(window, SidebarAction::NewSessionInCurrent)
    );
    remote.store_mut().select_session(Some(b.id()));
    remote.settle_actions();
    remote.delta(
        1,
        CatalogChange::SessionAdded {
            session: created,
            workspace: w,
            index: 2,
        },
    );
    assert_eq!(remote.store().selected_session_id(), Some(b.id()));
}

#[test]
fn the_tree_reads_a_row_pending_until_the_server_reaches_its_receipt() {
    let (a, w) = (session("a"), Uuid::new_v4());
    let mut remote = Remote::showing(w, vec![a.clone()]);
    let rename = remote.fixture.send(
        session_cmd(SessionCmd::Rename),
        Some(&wire(a.id())),
        args(|a| a.name = Some("renamed".to_owned())),
    );
    assert!(rename.ok, "{rename:?}");
    let pending = remote.row_node(a.id());
    let operations = remote.tree()["remote"]["operations"].clone();
    let mut renamed = a.fields.clone();
    renamed.custom_name = Some("renamed".to_owned());
    remote.delta(1, CatalogChange::SessionChanged(renamed));
    let settled = remote.row_node(a.id());
    assert_eq!(pending["pending"], json!("pending"));
    assert_eq!(operations[0]["action"], json!("Rename Session"));
    assert_eq!(operations[0]["session"], json!(wire(a.id())));
    assert_eq!(operations[0]["state"], json!("pending"));
    assert_eq!(settled.get("pending"), None);
    assert_eq!(settled["name"], json!("renamed"));
    assert_eq!(remote.tree()["remote"].get("operations"), None);
}

#[test]
fn a_lost_reply_is_unknown_never_resent_and_dismissed_by_window_remote_dismiss() {
    let (a, w) = (session("a"), Uuid::new_v4());
    let mut remote = Remote::showing(w, vec![a.clone()]);
    remote.runner.lose.store(true, Ordering::SeqCst);
    let rename = remote.fixture.send(
        session_cmd(SessionCmd::Rename),
        Some(&wire(a.id())),
        args(|a| a.name = Some("renamed".to_owned())),
    );
    remote.settle_actions();
    assert_eq!(
        rename.error.as_deref(),
        Some("box did not answer Rename Session; its outcome is unknown until box can be asked")
    );
    assert_eq!(remote.row_node(a.id())["pending"], json!("unknown"));
    assert_eq!(
        remote.window_node()["remote"]["operations"][0]["state"],
        json!("unknown")
    );
    assert_eq!(remote.runner.operations().len(), 1, "the command is never sent again");
    let lookups = remote.runner.lookups();
    assert_eq!(lookups.len(), 1, "{lookups:?}");
    assert!(lookups[0].contains(" catalog op "), "{}", lookups[0]);
    let dismissed = remote.dismiss();
    assert_eq!(dismissed.result, Some(json!({"text": "1 dismissed"})));
    assert_eq!(remote.row_node(a.id()).get("pending"), None);
    assert_eq!(remote.tree()["remote"].get("operations"), None);
    assert_eq!(remote.dismiss().result, Some(json!({"text": "0 dismissed"})));
}

#[test]
fn window_remote_dismiss_refuses_a_local_window() {
    let mut remote = Remote::opened(&server_version());
    let local = wire(remote.fixture.window);
    let response = remote
        .fixture
        .send(Command::Window(WindowCmd::RemoteDismiss), Some(&local), Args::default());
    assert_eq!(
        response.error.as_deref(),
        Some("window.remote.dismiss acts only on a remote window")
    );
}

#[test]
fn while_disconnected_structural_commands_are_refused_and_nothing_is_queued() {
    let (a, w) = (session("a"), Uuid::new_v4());
    let mut remote = Remote::showing(w, vec![a.clone()]);
    let link = remote.link();
    remote.close(&link, "ssh exited");
    assert!(matches!(remote.state(), CatalogState::Disconnected(_)));
    let rename = remote.fixture.send(
        session_cmd(SessionCmd::Rename),
        Some(&wire(a.id())),
        args(|a| a.name = Some("renamed".to_owned())),
    );
    let window = remote.window;
    assert!(!remote.fixture.core.structural_actions_enabled(window));
    assert!(remote.fixture.core.structural_actions_enabled(remote.fixture.window));
    remote.fixture.core.sidebar_action(window, SidebarAction::NewWorkspace);
    assert_eq!(rename.error.as_deref(), Some("box is not connected"));
    assert!(remote.runner.operations().is_empty());
    assert_eq!(remote.tree()["remote"].get("operations"), None);
}

#[test]
fn the_sidebar_sends_its_row_changes_to_the_server() {
    let (a, b, w, other) = (session("a"), session("b"), Uuid::new_v4(), Uuid::new_v4());
    let mut remote = Remote::opened(&server_version());
    remote.snapshot(snapshot(vec![
        workspace(w, "w", vec![a.clone(), b.clone()]),
        workspace(other, "x", vec![]),
    ]));
    let window = remote.window;
    let name = remote.row(a.id()).display_name();
    let actions = [
        SidebarAction::NewSessionInCurrent,
        SidebarAction::NewSession { workspace: other },
        SidebarAction::NewWorkspace,
        SidebarAction::NewCompanion(a.id()),
        SidebarAction::RenameSession {
            session: a.id(),
            name: "renamed".to_owned(),
        },
        SidebarAction::RenameWorkspace {
            workspace: w,
            name: " ops ".to_owned(),
        },
        SidebarAction::MoveSessions {
            sessions: vec![b.id()],
            workspace: other,
        },
        SidebarAction::DropSessions {
            sessions: vec![a.id()],
            workspace: w,
            destination: 1,
        },
        SidebarAction::DropWorkspace {
            workspace: other,
            destination: 0,
        },
        SidebarAction::CloseSessions(vec![b.id()]),
        SidebarAction::DeleteWorkspace(other),
    ];
    for action in actions {
        assert!(remote.fixture.core.sidebar_action(window, action.clone()), "{action:?}");
        remote.settle_actions();
    }
    let lines = remote.runner.operations();
    let (a_row, b_row, w_row, x_row) = (wire(a.id()), wire(b.id()), wire(w), wire(other));
    let expected = [
        format!("session new --workspace {w_row} --op-id"),
        format!("session new --workspace {x_row} --op-id"),
        "workspace new --op-id".to_owned(),
        format!("session new --parent {a_row} --op-id"),
        format!("session rename --target {a_row} --op-id"),
        format!("workspace rename --target {w_row} --op-id"),
        format!("session move --target {b_row} --op-id"),
        format!("session move --target {a_row} --after {b_row} --op-id"),
        format!("workspace move --target {x_row} --to top --op-id"),
        format!("session close --target {b_row} --op-id"),
        format!("workspace delete --target {x_row} --op-id"),
    ];
    assert_eq!(lines.len(), expected.len(), "{lines:#?}");
    for (line, expected) in lines.iter().zip(&expected) {
        assert!(line.contains(expected.as_str()), "{line} lacks {expected}");
    }
    assert!(lines[5].ends_with("--json -- ops"), "{}", lines[5]);
    assert_eq!(remote.rows(), vec![a.id(), b.id()]);
    assert_eq!(remote.row(a.id()).display_name(), name);
    assert_eq!(remote.store().workspaces().len(), 2);
}

#[test]
fn duplicate_is_a_fresh_server_shell_after_the_row_and_a_repeat_waits_for_the_first() {
    let (a, w) = (session("a"), Uuid::new_v4());
    let mut remote = Remote::showing(w, vec![a.clone()]);
    let window = remote.window;
    assert!(
        remote
            .fixture
            .core
            .sidebar_action(window, SidebarAction::Duplicate(a.id()))
    );
    remote.settle_actions();
    remote
        .fixture
        .core
        .sidebar_action(window, SidebarAction::Duplicate(a.id()));
    remote.settle_actions();
    let lines = remote.runner.operations();
    assert_eq!(lines.len(), 1, "{lines:?}");
    let expected = format!("session new --after {} --cwd=/home --op-id", wire(a.id()));
    assert!(lines[0].contains(&expected), "{}", lines[0]);
    assert_eq!(remote.rows(), vec![a.id()]);
}

#[test]
fn a_sidebar_gesture_that_makes_a_local_shell_sends_nothing() {
    let (a, w) = (session("a"), Uuid::new_v4());
    let mut remote = Remote::showing(w, vec![a.clone()]);
    let window = remote.window;
    for action in [
        SidebarAction::DuplicateAsCompanion(a.id()),
        SidebarAction::ConvertToCompanion(a.id()),
        SidebarAction::OpenDirectories {
            workspace: w,
            directories: vec!["/tmp".to_owned()],
        },
        SidebarAction::DropSessions {
            sessions: vec![a.id(), a.id()],
            workspace: w,
            destination: 0,
        },
    ] {
        assert!(
            !remote.fixture.core.sidebar_action(window, action.clone()),
            "{action:?}"
        );
    }
    assert!(remote.runner.operations().is_empty());
    assert_eq!(remote.rows(), vec![a.id()]);
    assert!(remote.fixture.core.sidebar_action(
        window,
        SidebarAction::SelectSessions {
            active: a.id(),
            selection: vec![a.id()],
        }
    ));
}

#[test]
fn a_refused_gesture_shows_the_servers_reason_over_its_row() {
    let (a, w) = (session("a"), Uuid::new_v4());
    let mut remote = Remote::showing(w, vec![a.clone()]);
    let dir = tempfile::tempdir().unwrap();
    remote.fixture.core.set_hud_body_dir(dir.path().join("bodies"));
    let link = remote.link();
    remote.close(&link, "ssh exited");
    let window = remote.window;
    remote.fixture.core.sidebar_action(
        window,
        SidebarAction::RenameSession {
            session: a.id(),
            name: "x".to_owned(),
        },
    );
    let spec = remote
        .row(a.id())
        .overlay
        .effective_hud_spec()
        .expect("an error panel over the row");
    assert_eq!(spec.message, "Rename Session: box is not connected");
    assert!(remote.runner.operations().is_empty());
}

#[test]
fn the_menu_and_palette_send_pane_and_move_actions_to_the_server() {
    let (a, other) = (session_with(Uuid::new_v4(), "a", vec![pane(), pane()]), Uuid::new_v4());
    let mut remote = Remote::opened(&server_version());
    remote.snapshot(snapshot(vec![
        workspace(Uuid::new_v4(), "w", vec![a.clone()]),
        workspace(other, "x", vec![]),
    ]));
    let window = remote.window;
    remote.store_mut().select_session(Some(a.id()));
    let (primary, split) = {
        let row = remote.row(a.id());
        (row.pane_identity, row.split_pane_identity)
    };
    for command in [PaletteCommand::SwapPanes, PaletteCommand::CloseSplit] {
        remote.fixture.core.perform_menu_command(window, command);
        remote.settle_actions();
    }
    remote.fixture.core.toggle_palette(window, PaletteMode::Actions);
    let row = PaletteAction::MoveSession { workspace: other };
    assert!(remote.fixture.core.palette_entry_enabled(window, &row));
    remote.fixture.core.run_palette_entry(window, &row);
    remote.settle_actions();
    let lines = remote.runner.operations();
    let a_row = wire(a.id());
    assert!(
        lines[0].contains(&format!("session swap --target {a_row} --expect-panes")),
        "{}",
        lines[0]
    );
    assert!(lines[1].contains(&format!("session split close --target {a_row} --expect-panes")));
    assert!(
        lines[2].contains(&format!("session move --target {a_row} --op-id")),
        "{}",
        lines[2]
    );
    let row = remote.row(a.id());
    assert!(row.split.exists);
    assert_eq!((row.pane_identity, row.split_pane_identity), (primary, split));
    assert_eq!(remote.store().workspace(other).map(|w| w.sessions.len()), Some(0));
}

#[test]
fn the_palette_move_row_is_off_while_disconnected() {
    let (a, other) = (session("a"), Uuid::new_v4());
    let mut remote = Remote::opened(&server_version());
    remote.snapshot(snapshot(vec![
        workspace(Uuid::new_v4(), "w", vec![a.clone()]),
        workspace(other, "x", vec![]),
    ]));
    let window = remote.window;
    remote.store_mut().select_session(Some(a.id()));
    let link = remote.link();
    remote.close(&link, "ssh exited");
    remote.fixture.core.toggle_palette(window, PaletteMode::Actions);
    let row = PaletteAction::MoveSession { workspace: other };
    assert!(!remote.fixture.core.palette_entry_enabled(window, &row));
}

#[test]
fn the_gui_hides_a_shown_server_scratch_on_the_server() {
    let mut a = session_with(Uuid::new_v4(), "a", vec![pane(), pane()]);
    a.fields.scratch = Some(CatalogScratch {
        pane: pane(),
        shown: true,
    });
    let mut remote = Remote::showing(Uuid::new_v4(), vec![a.clone()]);
    let window = remote.window;
    remote.store_mut().select_session(Some(a.id()));
    let origin = ActionOrigin { window, quick: false };
    remote.fixture.core.perform_builtin(origin, BuiltinAction::ToggleSplit);
    remote.settle_actions();
    assert!(remote.row(a.id()).scratch_active, "shown until the catalog hides it");
    let indicator = AgentIndicator {
        status: AgentStatus::Blocked,
        status_pane: Some(StatusPane::Left),
        ..AgentIndicator::default()
    };
    remote.fixture.core.reveal_blocked_pane(a.id(), Some(&indicator));
    assert!(remote.row(a.id()).scratch_active, "reveal");
    let lines = remote.runner.operations();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains(&format!("session scratch off --target {} --op-id", wire(a.id()))));
    assert_eq!(remote.rows(), vec![a.id()]);
}

#[test]
fn a_request_whose_window_closed_before_the_reply_answers_its_outcome_unknown() {
    let (a, w) = (session("a"), Uuid::new_v4());
    let mut remote = Remote::showing(w, vec![a.clone()]);
    let request = Request {
        cmd: session_cmd(SessionCmd::Rename),
        target: Some(wire(a.id())),
        args: Some(args(|a| a.name = Some("renamed".to_owned()))),
    };
    let Dispatched::Pending { resume, .. } = remote.fixture.core.begin(&request) else {
        panic!("the rename waits for the server's reply");
    };
    remote.fixture.core.close_window(remote.window);
    remote.fixture.core.commit();
    let response = resume(&mut remote.fixture.core).finish_now(&mut remote.fixture.core);
    assert_eq!(
        response.error.as_deref(),
        Some("box did not answer Rename Session; its outcome is unknown until box can be asked")
    );
}

#[test]
fn a_request_the_window_cannot_send_is_refused_with_its_reason() {
    let (a, b, w) = (session("a"), session("b"), Uuid::new_v4());
    let mut remote = Remote::showing(w, vec![a.clone(), b.clone()]);
    let response = remote.fixture.send(
        session_cmd(SessionCmd::Close),
        None,
        args(|args| args.targets = Some(vec![wire(a.id()), wire(b.id())])),
    );
    assert_eq!(
        response.error.as_deref(),
        Some("a remote window closes one session at a time")
    );
    assert!(remote.runner.operations().is_empty());
}

#[test]
fn a_gesture_the_server_refuses_shows_its_reason_and_a_created_row_already_shown_is_selected() {
    let (a, b, w) = (session("a"), session("b"), Uuid::new_v4());
    let mut remote = Remote::showing(w, vec![a.clone(), b.clone()]);
    let dir = tempfile::tempdir().unwrap();
    remote.fixture.core.set_hud_body_dir(dir.path().join("bodies"));
    let window = remote.window;
    remote.store_mut().select_session(Some(a.id()));
    *remote.runner.created.lock().unwrap() = Some(b.id());
    assert!(
        remote
            .fixture
            .core
            .sidebar_action(window, SidebarAction::NewSessionInCurrent)
    );
    remote.settle_actions();
    assert_eq!(remote.store().selected_session_id(), Some(b.id()));
    remote.runner.lose.store(true, Ordering::SeqCst);
    remote.fixture.core.sidebar_action(
        window,
        SidebarAction::RenameSession {
            session: a.id(),
            name: "x".to_owned(),
        },
    );
    remote.settle_actions();
    let spec = remote.row(a.id()).overlay.effective_hud_spec().expect("an error panel");
    assert_eq!(
        spec.message,
        "Rename Session: box did not answer Rename Session; its outcome is unknown until box can be asked"
    );
}

#[test]
fn close_split_over_a_shown_scratch_hides_the_scratch_and_a_row_without_a_split_sends_nothing() {
    let mut a = session_with(Uuid::new_v4(), "a", vec![pane(), pane()]);
    a.fields.scratch = Some(CatalogScratch {
        pane: pane(),
        shown: true,
    });
    let b = session("b");
    let w = Uuid::new_v4();
    let mut remote = Remote::showing(w, vec![a.clone(), b.clone()]);
    let window = remote.window;
    remote.store_mut().select_session(Some(a.id()));
    remote
        .fixture
        .core
        .perform_menu_command(window, PaletteCommand::CloseSplit);
    remote.store_mut().select_session(Some(b.id()));
    remote
        .fixture
        .core
        .perform_menu_command(window, PaletteCommand::CloseSplit);
    remote.settle_actions();
    assert!(!remote.fixture.core.sidebar_action(
        window,
        SidebarAction::RenameWorkspace {
            workspace: w,
            name: "  ".to_owned(),
        }
    ));
    let lines = remote.runner.operations();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains(&format!("session scratch off --target {} --op-id", wire(a.id()))));
}

#[test]
fn a_gesture_in_a_closed_remote_window_sends_nothing() {
    let (a, w) = (session("a"), Uuid::new_v4());
    let mut remote = Remote::showing(w, vec![a.clone()]);
    let window = remote.window;
    remote.fixture.core.close_window(window);
    remote.fixture.core.commit();
    assert!(!remote.fixture.core.new_remote_session(window, w));
    assert!(
        !remote
            .fixture
            .core
            .remote_sidebar_action(window, SidebarAction::NewWorkspace)
    );
    assert!(remote.runner.operations().is_empty());
}
