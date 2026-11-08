//! Dynamic palette rows retain their targets and recheck live state before running.

use ngterm_proto::AppIdentity;

use super::*;
use crate::model::{NewSession, WindowLibrary};
use crate::persist::AppSettings;
use crate::{CoreConfig, CoreState};

fn core() -> (Core, Uuid) {
    let library = WindowLibrary::seeded("/tmp");
    let window = library.active_window_id().unwrap();
    let core = Core::new(
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
    (core, window)
}

#[test]
fn a_builtin_row_closes_the_palette_then_returns_its_frontend_effect() {
    let (mut core, window) = core();
    core.open_palette(window, PaletteMode::Actions);
    assert_eq!(
        core.run_palette_entry(
            window,
            &PaletteAction::Builtin(crate::keymap::BuiltinAction::ToggleFullscreen)
        ),
        Some(ActionEffect::Frontend(FrontendAction::ToggleFullscreen(window)))
    );
    assert_eq!(core.palette_mode(), None);
}

#[test]
fn a_move_row_transfers_its_session_to_the_chosen_workspace() {
    let (mut core, window) = core();
    let store = core.library_mut().store_mut(window).unwrap();
    let session = store.selected_session_id().unwrap();
    let source = store.current_workspace_id().unwrap();
    let destination = store.add_workspace("destination");
    core.open_palette(window, PaletteMode::Sessions);
    let action = PaletteAction::MoveSession(destination);
    assert_eq!(core.run_palette_entry(window, &action), Some(ActionEffect::Done));
    let store = core.library().store(window).unwrap();
    assert!(store.workspace(source).unwrap().sessions.is_empty());
    assert_eq!(store.workspace_for_session(session).unwrap().id(), destination);
    assert_eq!(core.palette_mode(), None);
}

#[test]
fn a_covered_window_refuses_dynamic_rows_without_moving_the_session() {
    use ngterm_proto::command::AskCmd;
    use ngterm_proto::{AskButton, Request};

    let (mut core, window) = core();
    let store = core.library_mut().store_mut(window).unwrap();
    let session = store.selected_session_id().unwrap();
    let source = store.current_workspace_id().unwrap();
    let destination = store.add_workspace("destination");
    core.open_palette(window, PaletteMode::Sessions);
    let response = core.dispatch(&Request {
        cmd: Command::Ask(AskCmd::Open),
        target: None,
        args: Some(Args {
            window: Some(uuid_string(window)),
            title: Some("Continue?".to_owned()),
            style: Some("gui".to_owned()),
            buttons: Some(vec![AskButton {
                id: "ok".to_owned(),
                label: "OK".to_owned(),
                hotkey: None,
            }]),
            ..Args::default()
        }),
    });
    assert!(response.ok, "{response:?}");
    core.run_palette_entry(window, &PaletteAction::MoveSession(destination));
    assert_eq!(
        core.library()
            .store(window)
            .unwrap()
            .workspace_for_session(session)
            .unwrap()
            .id(),
        source
    );
}

#[test]
fn a_stale_session_row_does_not_change_selection() {
    let (mut core, window) = core();
    let store = core.library_mut().store_mut(window).unwrap();
    let selected = store.selected_session_id();
    let workspace = store.current_workspace_id().unwrap();
    let stale = store
        .add_session(
            workspace,
            NewSession {
                select: false,
                ..NewSession::at_cwd("/tmp")
            },
        )
        .unwrap();
    store.close_session(stale);
    core.open_palette(window, PaletteMode::Sessions);
    assert_eq!(
        core.run_palette_entry(window, &PaletteAction::SelectSession(stale)),
        Some(ActionEffect::Done)
    );
    assert_eq!(core.library().store(window).unwrap().selected_session_id(), selected);
    assert_eq!(core.palette_mode(), None);
}

#[test]
fn custom_rows_run_sessionless_and_ignore_commands_removed_since_listing() {
    use crate::paths::{ConfigLocator, MapEnv};
    use crate::{CoreTask, ProcessHost};
    use ngterm_host::fakes::FakeProcessLauncher;
    use std::sync::{Arc, mpsc};

    let (mut core, window) = core();
    let dir = tempfile::tempdir().unwrap();
    core.set_config_locator(ConfigLocator::capture(
        &MapEnv::new().with("NGTERM_STATE_DIR", dir.path()),
    ));
    std::fs::create_dir_all(dir.path().join("config")).unwrap();
    std::fs::write(core.keymap_path(), "command \"Build\" echo build\n").unwrap();
    core.reload_keymap();
    let command = core.keymap().commands()[0].id;
    let launcher = Arc::new(FakeProcessLauncher::new());
    let (sender, receiver) = mpsc::channel::<CoreTask>();
    core.set_process_host(ProcessHost {
        launcher: launcher.clone(),
        poster: Arc::new(move |task| {
            sender.send(task).unwrap();
        }),
        socket: "unused-test.sock".to_owned(),
        cli_dir: None,
        path: None,
    });
    let store = core.library_mut().store_mut(window).unwrap();
    let session = store.selected_session_id().unwrap();
    store.close_session(session);
    core.open_palette(window, PaletteMode::CustomCommands);
    assert_eq!(
        core.run_palette_entry(window, &PaletteAction::Custom(command)),
        Some(ActionEffect::Done)
    );
    let specs = launcher.launches();
    assert_eq!(specs.len(), 1);
    assert!(
        specs[0]
            .env
            .iter()
            .any(|(key, value)| key == "AGT_SESSION_ID" && value.is_empty())
    );
    assert!(launcher.finish(FakeProcessLauncher::FIRST_PID, ngterm_host::ProcessExit::default()));
    assert!(receiver.try_recv().is_err());
    assert!(launcher.running().is_empty());
    std::fs::write(core.keymap_path(), "").unwrap();
    core.reload_keymap();
    core.open_palette(window, PaletteMode::CustomCommands);
    core.run_palette_entry(window, &PaletteAction::Custom(command));
    assert_eq!(launcher.launches().len(), 1);
}

/// The action palette's "Move Session to" row for workspace `target`, as the palette lists it now.
fn move_row(core: &Core, target: Uuid) -> PaletteAction {
    let id = format!("move-{}", uuid_string(target));
    core.palette_entries(PaletteMode::Actions)
        .into_iter()
        .find(|entry| entry.id == id)
        .unwrap()
        .action
}

/// `testMoveRowsMoveTheSessionSelectedWhenTheyRun`: a row listed while one session was selected moves the session
/// selected when it runs.
#[test]
fn move_rows_move_the_session_selected_when_they_run() {
    let (mut core, window) = core();
    let store = core.library_mut().store_mut(window).unwrap();
    let owner = store.current_workspace_id().unwrap();
    let first = store.selected_session_id().unwrap();
    let second = store.add_session(owner, NewSession::at_cwd("/tmp")).unwrap();
    let target = store.add_workspace("target");
    store.select_session(Some(first));
    let action = move_row(&core, target);

    let store = core.library_mut().store_mut(window).unwrap();
    store.select_session(Some(second));
    core.open_palette(window, PaletteMode::Actions);
    assert!(core.palette_entry_enabled(window, &action));
    assert_eq!(core.run_palette_entry(window, &action), Some(ActionEffect::Done));

    let store = core.library().store(window).unwrap();
    assert_eq!(store.workspace_for_session(first).unwrap().id(), owner);
    assert_eq!(store.workspace_for_session(second).unwrap().id(), target);
}

/// `testAMoveRowGoesInertWhenTheSessionSelectedUnderItAlreadyLivesInItsTarget`: the row goes inert, and the palette
/// stays open, once the selection already lives in its target or nothing is selected.
#[test]
fn a_move_row_goes_inert_when_the_session_selected_under_it_already_lives_in_its_target() {
    let (mut core, window) = core();
    let store = core.library_mut().store_mut(window).unwrap();
    let owner = store.current_workspace_id().unwrap();
    let first = store.selected_session_id().unwrap();
    let target = store.add_workspace("target");
    let resident = store.add_session(target, NewSession::at_cwd("/tmp")).unwrap();
    store.select_session(Some(first));
    let action = move_row(&core, target);

    core.open_palette(window, PaletteMode::Actions);
    let store = core.library_mut().store_mut(window).unwrap();
    store.select_session(Some(resident));
    assert!(!core.palette_entry_enabled(window, &action));
    let store = core.library_mut().store_mut(window).unwrap();
    store.select_session(None);
    assert!(!core.palette_entry_enabled(window, &action));
    assert_eq!(core.run_palette_entry(window, &action), None);
    assert_eq!(core.palette_mode(), Some(PaletteMode::Actions));

    let store = core.library().store(window).unwrap();
    assert_eq!(store.workspace_for_session(first).unwrap().id(), owner);
    assert_eq!(store.workspace_for_session(resident).unwrap().id(), target);
}

/// `testMoveRowsAreListedButInertForACompanion`: a companion only reorders inside its group, so its move rows are
/// listed but never run.
#[test]
fn move_rows_are_listed_but_inert_for_a_companion() {
    use crate::model::CompanionOptions;

    let (mut core, window) = core();
    let store = core.library_mut().store_mut(window).unwrap();
    let owner = store.current_workspace_id().unwrap();
    let parent = store.selected_session_id().unwrap();
    let child = store
        .add_companion(
            parent,
            CompanionOptions {
                name: Some("child".to_owned()),
                ..CompanionOptions::default()
            },
            "/tmp",
        )
        .unwrap();
    let target = store.add_workspace("target");
    store.select_session(Some(child));
    let action = move_row(&core, target);

    core.open_palette(window, PaletteMode::Actions);
    assert!(!core.palette_entry_enabled(window, &action));
    assert_eq!(core.run_palette_entry(window, &action), None);
    let store = core.library().store(window).unwrap();
    assert_eq!(store.workspace_for_session(child).unwrap().id(), owner);
}

/// `testAMoveRowGoesInertWhenItsTargetIsRemovedUnderIt`.
#[test]
fn a_move_row_goes_inert_when_its_target_is_removed_under_it() {
    let (mut core, window) = core();
    let store = core.library_mut().store_mut(window).unwrap();
    let session = store.selected_session_id().unwrap();
    let owner = store.current_workspace_id().unwrap();
    let target = store.add_workspace("target");
    store.select_session(Some(session));
    let action = move_row(&core, target);

    let store = core.library_mut().store_mut(window).unwrap();
    assert!(store.remove_workspace(target).is_some());
    core.open_palette(window, PaletteMode::Actions);
    assert!(!core.palette_entry_enabled(window, &action));
    assert_eq!(core.run_palette_entry(window, &action), None);
    let store = core.library().store(window).unwrap();
    assert_eq!(store.workspace_for_session(session).unwrap().id(), owner);
}
