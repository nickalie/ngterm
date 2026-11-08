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
    let action = PaletteAction::MoveSession {
        session,
        workspace: destination,
    };
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
    core.run_palette_entry(
        window,
        &PaletteAction::MoveSession {
            session,
            workspace: destination,
        },
    );
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
