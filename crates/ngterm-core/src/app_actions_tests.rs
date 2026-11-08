//! Built-in and menu actions at the model and fake-terminal boundaries.

use ngterm_host::fakes::{FakeSurface, FakeSurfaceProbe};
use ngterm_proto::AppIdentity;

use super::*;
use crate::model::{StatusPane, WindowLibrary};
use crate::paths::{ConfigLocator, MapEnv};
use crate::persist::AppSettings;
use crate::{CoreConfig, CoreState};

struct Fixture {
    core: Core,
    window: Uuid,
    workspace: Uuid,
    session: Uuid,
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let library = WindowLibrary::seeded("/tmp");
        let window = library.active_window_id().unwrap();
        let store = library.store(window).unwrap();
        let workspace = store.current_workspace_id().unwrap();
        let session = store.selected_session_id().unwrap();
        let mut core = Core::new(
            CoreConfig {
                app: AppIdentity::from_recorded("0.1.0", None),
                capabilities: Vec::new(),
            },
            CoreState {
                library,
                settings: AppSettings::default(),
                persistence: None,
            },
        );
        let dir = tempfile::tempdir().unwrap();
        core.set_config_locator(ConfigLocator::capture(
            &MapEnv::new().with("NGTERM_STATE_DIR", dir.path()),
        ));
        Self {
            core,
            window,
            workspace,
            session,
            dir,
        }
    }

    fn store(&self) -> &AppStore {
        self.core.library().store(self.window).unwrap()
    }

    fn store_mut(&mut self) -> &mut AppStore {
        self.core.library_mut().store_mut(self.window).unwrap()
    }

    fn run(&mut self, action: BuiltinAction) -> ActionEffect {
        self.core.perform_builtin(
            ActionOrigin {
                window: self.window,
                quick: false,
            },
            action,
        )
    }

    fn menu(&mut self, command: PaletteCommand) -> ActionEffect {
        self.core.perform_menu_command(self.window, command)
    }

    fn install(&mut self, pane: StatusPane) -> FakeSurfaceProbe {
        let surface = FakeSurface::new(format!("{pane:?}"));
        let probe = surface.probe();
        let session = self.session;
        self.store_mut()
            .session_mut(session)
            .unwrap()
            .pane_slot_mut(pane)
            .replace(Box::new(surface));
        probe
    }
}

#[test]
fn window_dialogs_and_navigation_use_the_named_window() {
    let mut f = Fixture::new();
    let window = f.window;
    let name = f.core.library().window_name(Some(window));
    assert_eq!(
        f.run(BuiltinAction::RenameWindow),
        frontend(FrontendAction::RenameWindow {
            window,
            name: name.clone()
        })
    );
    let other = f.core.library_mut().new_window(Some("other"));
    assert_eq!(
        f.run(BuiltinAction::DeleteWindow),
        frontend(FrontendAction::DeleteWindow {
            window,
            name,
            sessions: 1
        })
    );
    f.core.report_frontmost(window);
    f.run(BuiltinAction::NextWindow);
    assert_eq!(f.core.library().active_window_id(), Some(other));
    f.run(BuiltinAction::PreviousWindow);
    assert_eq!(f.core.library().active_window_id(), Some(window));
}

#[test]
fn workspace_dialogs_collapse_and_navigation_change_the_current_workspace() {
    let mut f = Fixture::new();
    let window = f.window;
    let workspace = f.workspace;
    assert_eq!(
        f.run(BuiltinAction::RenameWorkspace),
        frontend(FrontendAction::RenameWorkspace { window, workspace })
    );
    let middle = f.store_mut().add_workspace_with(
        "other",
        crate::model::WorkspaceOptions {
            reveal: false,
            ..crate::model::WorkspaceOptions::default()
        },
    );
    assert_eq!(
        f.run(BuiltinAction::DeleteWorkspace),
        frontend(FrontendAction::DeleteWorkspace { window, workspace })
    );
    f.store_mut().note_sidebar_expansion([workspace].into());
    f.run(BuiltinAction::ToggleWorkspaceCollapse);
    assert!(!f.store().workspace(workspace).unwrap().is_expanded);
    assert_eq!(
        f.store_mut().take_sidebar_expansion_requests().get(&workspace),
        Some(&false)
    );
    f.store_mut().note_sidebar_expansion([].into());
    f.run(BuiltinAction::ToggleWorkspaceCollapse);
    assert!(f.store().workspace(workspace).unwrap().is_expanded);
    assert_eq!(
        f.store_mut().take_sidebar_expansion_requests().get(&workspace),
        Some(&true)
    );
    f.run(BuiltinAction::NewWorkspace);
    let other = f.store().current_workspace_id().unwrap();
    assert_ne!(other, workspace);
    f.run(BuiltinAction::PreviousWorkspace);
    assert_eq!(f.store().current_workspace_id(), Some(middle));
    f.run(BuiltinAction::PreviousWorkspace);
    assert_eq!(f.store().current_workspace_id(), Some(workspace));
    f.run(BuiltinAction::NextWorkspace);
    assert_eq!(f.store().current_workspace_id(), Some(middle));
    f.run(BuiltinAction::NextWorkspace);
    assert_eq!(f.store().current_workspace_id(), Some(other));
}

#[test]
fn focus_menu_marks_without_enabling_then_clears_membership() {
    let mut f = Fixture::new();
    let workspace = f.workspace;
    f.menu(PaletteCommand::AddWorkspaceToFocus);
    assert!(f.store().focus().ids().contains(&workspace));
    assert!(!f.store().focus().enabled());
    f.menu(PaletteCommand::ClearFocus);
    assert!(f.store().focus().ids().is_empty());
}

#[test]
fn swapping_and_closing_a_split_changes_panes_and_restores_focus() {
    let mut f = Fixture::new();
    let session = f.session;
    f.run(BuiltinAction::ToggleHorizontalSplit);
    assert_eq!(
        f.store().session(session).unwrap().split.axis,
        crate::model::SplitAxis::TopBottom
    );
    f.install(StatusPane::Left);
    f.install(StatusPane::Right);
    f.menu(PaletteCommand::SwapPanes);
    let state = f.store().session(session).unwrap();
    assert_eq!(state.surface.get().unwrap().pane_token(), "Right");
    assert_eq!(state.split_surface.get().unwrap().pane_token(), "Left");
    assert!(f.core.pane_focus_requested(session));
    f.menu(PaletteCommand::CloseSplit);
    assert!(!f.store().session(session).unwrap().split.exists);
}

#[test]
fn find_starts_on_screen_and_ends_on_the_search_owner() {
    let mut f = Fixture::new();
    let session = f.session;
    let primary = f.install(StatusPane::Left);
    let scratch = f.install(StatusPane::Scratch);
    f.run(BuiltinAction::ToggleScratch);
    f.run(BuiltinAction::ToggleSearch);
    assert_eq!(scratch.actions(), ["start_search"]);
    assert!(primary.actions().is_empty());
    let state = f.store_mut().session_mut(session).unwrap();
    state.search.active = true;
    state.search.owner = Some(StatusPane::Scratch);
    state.scratch_active = false;
    f.run(BuiltinAction::ToggleSearch);
    assert_eq!(scratch.actions(), ["start_search", "end_search"]);
    assert!(primary.actions().is_empty());
}

#[test]
fn a_confirmed_close_returns_the_session_to_confirm_without_removing_it() {
    let mut f = Fixture::new();
    f.core.settings_unsaved().confirm_close_session = Some(true);
    let window = f.window;
    let session = f.session;
    assert_eq!(
        f.run(BuiltinAction::CloseSession),
        frontend(FrontendAction::ConfirmCloseSession { window, session })
    );
    assert!(f.store().session(session).is_some());
}

#[test]
fn close_unzooms_the_quick_terminal_before_dismissing_it() {
    let mut f = Fixture::new();
    f.core.toggle_quick_terminal();
    f.core.install_quick_surface(Box::new(FakeSurface::new("quick")));
    f.core.run_request(
        Command::Surface(SurfaceCmd::Zoom),
        Some("quick".to_owned()),
        Args {
            mode: Some("show".to_owned()),
            ..Args::default()
        },
    );
    assert!(f.core.quick_zoomed());
    let origin = ActionOrigin {
        window: f.window,
        quick: true,
    };
    assert_eq!(
        f.core.perform_builtin(origin, BuiltinAction::CloseSession),
        ActionEffect::Done
    );
    assert!(!f.core.quick_zoomed());
    assert!(f.core.quick_visible());
    f.core.perform_builtin(origin, BuiltinAction::CloseSession);
    assert!(!f.core.quick_visible());
    assert!(f.store().session(f.session).is_some());
}

#[test]
fn menu_reload_reads_config_and_editor_exit_reloads_only_its_generation() {
    let mut f = Fixture::new();
    let config = f.dir.path().join("config");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(config.join("keymap.conf"), "command \"Build\" echo build\n").unwrap();
    f.menu(PaletteCommand::ReloadKeymap);
    assert_eq!(f.core.keymap().commands()[0].name, "Build");
    f.menu(PaletteCommand::ReloadHooks);
    assert!(f.core.hooks_diagnostics().is_empty());
    f.menu(PaletteCommand::ReloadConfig);
    assert!(f.core.take_config_reload());
    for (command, file) in [
        (PaletteCommand::EditHooks, ConfigFile::Hooks),
        (PaletteCommand::EditGhosttyConfig, ConfigFile::Ghostty),
    ] {
        f.menu(command);
        let state = f.store().session(f.session).unwrap();
        assert!(state.overlay.program_active());
        let generation = state.overlay.generation;
        assert_eq!(state.overlay.size_percent, Some(95));
        assert_eq!(f.core.config_edit_ended(f.session, generation + 1), None);
        assert_eq!(f.core.config_edit_ended(f.session, generation), Some(file));
        assert_eq!(f.core.config_edit_ended(f.session, generation), None);
        let session = f.session;
        f.store_mut().close_overlay(session);
    }
    assert!(f.core.take_config_reload());
}

#[test]
fn missing_and_closed_windows_ignore_actions_and_menu_commands() {
    let mut f = Fixture::new();
    let missing = Uuid::new_v4();
    assert!(!f.core.palette_context(missing).has(PaletteFacts::HAS_ACTIVE_SESSION));
    for window in [missing, f.window] {
        if window == f.window {
            f.core.library_mut().close_window(window);
        }
        assert_eq!(
            f.core
                .perform_builtin(ActionOrigin { window, quick: false }, BuiltinAction::NewSession),
            ActionEffect::Done
        );
        assert_eq!(
            f.core.perform_menu_command(window, PaletteCommand::ReloadConfig),
            ActionEffect::Done
        );
    }
    assert!(!f.core.take_config_reload());
    assert!(f.core.take_window_requests().is_empty());
}

#[test]
fn reopen_recent_restores_a_closed_session_in_the_origin_window() {
    let mut f = Fixture::new();
    let session = f.session;
    f.store_mut().close_session(session);
    assert!(f.store().session(session).is_none());
    f.run(BuiltinAction::ReopenRecent);
    assert!(f.store().session(session).is_some());
    assert_eq!(f.core.library().active_window_id(), Some(f.window));
    assert!(f.core.library().recent_closed_items().is_empty());
}

#[test]
fn pane_focus_actions_and_previous_session_follow_the_selected_session() {
    let mut f = Fixture::new();
    let first = f.session;
    f.run(BuiltinAction::ToggleSplit);
    f.run(BuiltinAction::FocusLeftPane);
    assert!(!f.store().session(first).unwrap().split.focused);
    f.run(BuiltinAction::FocusRightPane);
    assert!(f.store().session(first).unwrap().split.focused);
    f.run(BuiltinAction::NewSession);
    assert_ne!(f.store().selected_session_id(), Some(first));
    f.run(BuiltinAction::PreviousSession);
    assert_eq!(f.store().selected_session_id(), Some(first));
}

#[test]
fn close_dismisses_a_terminal_zoom_before_closing_its_session() {
    let mut f = Fixture::new();
    f.run(BuiltinAction::ToggleTerminalZoom);
    assert!(f.core.zoom_target(f.window).is_some());
    f.run(BuiltinAction::CloseSession);
    assert_eq!(f.core.zoom_target(f.window), None);
    assert!(f.store().session(f.session).is_some());
}

#[test]
fn close_dismisses_the_dashboard_before_closing_its_session() {
    let mut f = Fixture::new();
    f.run(BuiltinAction::NewSession);
    f.run(BuiltinAction::Dashboard);
    assert!(f.core.dashboard(f.window).is_some());
    f.run(BuiltinAction::CloseSession);
    assert!(f.core.dashboard(f.window).is_none());
    assert_eq!(f.store().sessions().count(), 2);
}

#[test]
fn close_cancels_a_window_picker_and_retains_the_selected_session() {
    use ngterm_proto::PickItem;
    use ngterm_proto::command::PickCmd;

    let mut f = Fixture::new();
    let response = f.core.dispatch(&Request {
        cmd: Command::Pick(PickCmd::Open),
        target: None,
        args: Some(Args {
            window: Some(uuid_string(f.window)),
            items: Some(vec![PickItem {
                id: "one".to_owned(),
                label: "One".to_owned(),
                ..PickItem::default()
            }]),
            ..Args::default()
        }),
    });
    assert!(response.ok, "{response:?}");
    let id = response.result.unwrap()["id"].as_str().unwrap().to_owned();
    f.run(BuiltinAction::CloseSession);
    let result = f.core.dispatch(&Request {
        cmd: Command::Pick(PickCmd::Result),
        target: Some(id),
        args: None,
    });
    assert!(result.ok, "{result:?}");
    assert_eq!(result.result.unwrap()["pick"]["result"], "cancelled");
    assert_eq!(f.store().selected_session_id(), Some(f.session));
}

#[test]
fn close_escapes_a_terminal_ask_without_closing_its_session() {
    use ngterm_proto::AskButton;
    use ngterm_proto::command::AskCmd;

    let mut f = Fixture::new();
    let response = f.core.dispatch(&Request {
        cmd: Command::Ask(AskCmd::Open),
        target: Some(uuid_string(f.session)),
        args: Some(Args {
            title: Some("Continue?".to_owned()),
            buttons: Some(vec![AskButton {
                id: "ok".to_owned(),
                label: "OK".to_owned(),
                hotkey: None,
            }]),
            ..Args::default()
        }),
    });
    assert!(response.ok, "{response:?}");
    let id = response.result.unwrap()["id"].as_str().unwrap().to_owned();
    f.run(BuiltinAction::CloseSession);
    let result = f.core.dispatch(&Request {
        cmd: Command::Ask(AskCmd::Result),
        target: Some(id),
        args: None,
    });
    assert!(result.ok, "{result:?}");
    assert_eq!(result.result.unwrap()["ask"]["result"], "escaped");
    assert_eq!(f.store().selected_session_id(), Some(f.session));
}

#[test]
fn workspace_fold_menu_opens_all_then_keeps_only_the_current_workspace_open() {
    let mut f = Fixture::new();
    let workspace = f.workspace;
    let other = f.store_mut().add_workspace("other");
    f.menu(PaletteCommand::CollapseWorkspaces);
    assert!(!f.store().workspace(workspace).unwrap().is_expanded);
    assert!(f.store().workspace(other).unwrap().is_expanded);
    f.menu(PaletteCommand::ExpandWorkspaces);
    assert!(f.store().workspaces().iter().all(|workspace| workspace.is_expanded));
}

#[test]
fn undo_close_restores_the_pending_session() {
    let mut f = Fixture::new();
    f.core.settings_unsaved().close_grace_undo_enabled = Some(true);
    f.run(BuiltinAction::CloseSession);
    assert!(f.store().session(f.session).is_none());
    assert!(f.store().pending_close_summary().is_some());
    f.run(BuiltinAction::UndoClose);
    assert!(f.store().session(f.session).is_some());
    assert!(f.store().pending_close_summary().is_none());
}
