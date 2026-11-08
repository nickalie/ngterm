//! The frontend half of the built-in actions: what `Core::perform_builtin` leaves to the frontend (upstream's
//! `AppActions` dialogs and the sidebar's inline rename), run from a keymap chord (port of
//! `agterm-gtk/src/keymap_actions.rs`).
//!
//! Each needs the window, the sidebar or a dialog: full screen and the delete confirmation are carried out here on the
//! registered window ([`crate::keymap_dispatch::register_window`]); the window close and Settings… go to the model;
//! the session close confirmation, the session and workspace renames, Delete Workspace and Open Directory… go to the
//! window's sidebar ([`WindowView::sidebar`]); Rename Window and Delete Window are alerts over their window
//! ([`crate::modal`]).

use gpui_kit::App;
use ngterm_core::app_actions::FrontendAction;
use ngterm_core::pane_env::uuid_string;
use ngterm_proto::command::WindowCmd;
use ngterm_proto::{Args, Command, Request};
use uuid::Uuid;

use crate::keymap_dispatch::{self, KeyOrigin};
use crate::sidebar::Sidebar;
use crate::state::{self, AppState, Poster};
use crate::window::WindowView;

/// Runs `action` in its window; `close_grace` is the soft-close setting a delete confirmation names.
pub fn perform(poster: &Poster, action: FrontendAction, close_grace: bool, cx: &mut App) {
    match action {
        FrontendAction::ToggleFullscreen(window) => {
            if let Some(handle) = keymap_dispatch::window_of(KeyOrigin::Window(window)) {
                let _ = handle.update(cx, |_, window, _| window.toggle_fullscreen());
            }
        }
        FrontendAction::DeleteWindow { window, name, sessions } => {
            delete_window(poster, window, &name, sessions, close_grace, cx);
        }
        FrontendAction::CloseWindow(window) => {
            state::post(poster, move |state: &mut AppState| state.close_requested(window));
        }
        FrontendAction::OpenSettings(window) => {
            state::post(poster, move |state: &mut AppState| state.open_settings(window));
        }
        FrontendAction::ConfirmCloseSession { window, session } => {
            with_sidebar(window, cx, |sidebar, root, cx| {
                sidebar.close_sessions(vec![session], root, cx);
            });
        }
        FrontendAction::RenameSession { window, session: id }
        | FrontendAction::RenameWorkspace { window, workspace: id } => {
            with_sidebar(window, cx, |sidebar, root, cx| sidebar.begin_rename(id, root, cx));
        }
        FrontendAction::DeleteWorkspace { window, workspace } => {
            with_sidebar(window, cx, |sidebar, root, cx| {
                sidebar.delete_workspace(workspace, root, cx);
            });
        }
        FrontendAction::OpenDirectory { window, workspace } => {
            with_sidebar(window, cx, |sidebar, root, cx| {
                sidebar.open_directory(workspace, root, cx);
            });
        }
        FrontendAction::RenameWindow { window, name } => rename_window(poster, window, &name, cx),
        // the alert sound over the window whose Edit item a hidden overlay refused
        FrontendAction::AlertBell(window) => {
            if let Some(handle) = keymap_dispatch::window_of(KeyOrigin::Window(window)) {
                let _ = handle.update(cx, |_, window, cx| crate::status_sound::alert(window, cx));
            }
        }
    }
}

/// Runs `run` on the sidebar of model window `window`, when that window is open.
fn with_sidebar(
    window: Uuid,
    cx: &mut App,
    run: impl FnOnce(&mut Sidebar, &mut gpui_kit::Window, &mut gpui_kit::Context<Sidebar>),
) {
    let Some(handle) =
        keymap_dispatch::window_of(KeyOrigin::Window(window)).and_then(|handle| handle.downcast::<WindowView>())
    else {
        return;
    };
    let ran = handle.update(cx, |view, root, cx| {
        view.sidebar().clone().update(cx, |sidebar, cx| run(sidebar, root, cx));
    });
    if ran.is_err() {
        tracing::debug!(%window, "the window closed before its sidebar action ran");
    }
}

/// Delete Window: confirmed while it holds sessions (upstream's `deleteActiveWindow`), then `window.delete`.
fn delete_window(poster: &Poster, window: Uuid, name: &str, sessions: usize, close_grace: bool, cx: &mut App) {
    let poster = poster.clone();
    let delete = move || {
        state::post(&poster, move |state: &mut AppState| {
            state.window_command(WindowCmd::Delete, window, None);
        });
    };
    if sessions == 0 {
        delete();
        return;
    }
    let Some(handle) = keymap_dispatch::window_of(KeyOrigin::Window(window)) else {
        return;
    };
    let spec = crate::modal::AlertSpec::confirm(
        format!("Delete “{name}”?"),
        delete_body(sessions, close_grace),
        "Delete",
    );
    let shown = handle.update(cx, |_, window, cx| {
        crate::modal::open_alert(
            window,
            spec,
            move |answer, _, _| {
                if answer.button == 1 {
                    delete();
                }
            },
            cx,
        );
    });
    if shown.is_err() {
        tracing::debug!(%window, "the window closed before its delete confirmation");
    }
}

/// Rename Window's alert (upstream's `renameActiveWindow`, the GTK build's dialog): a field holding the name, all of it
/// selected; OK (Return) commits it through `window.rename`, Cancel (Escape) keeps the name.
#[must_use]
pub fn rename_alert(name: &str) -> crate::modal::AlertSpec {
    crate::modal::AlertSpec {
        heading: "Rename Window".to_owned(),
        body: "Enter a new name for this window.".to_owned(),
        buttons: vec![
            crate::modal::AlertButton::cancel("Cancel"),
            crate::modal::AlertButton::plain("OK").as_default().suggested(),
        ],
        checks: Vec::new(),
        field: Some(name.to_owned()),
    }
}

/// Rename Window: [`rename_alert`] over the window, committed through `window.rename`.
fn rename_window(poster: &Poster, window: Uuid, name: &str, cx: &mut App) {
    let Some(handle) = keymap_dispatch::window_of(KeyOrigin::Window(window)) else {
        return;
    };
    let poster = poster.clone();
    let spec = rename_alert(name);
    let shown = handle.update(cx, |_, root, cx| {
        crate::modal::open_alert(
            root,
            spec,
            move |answer, _, _| {
                let Some(name) = answer.text.filter(|_| answer.button == 1) else {
                    return;
                };
                state::post(&poster, move |state: &mut AppState| {
                    state.window_command(WindowCmd::Rename, window, Some(name));
                });
            },
            cx,
        );
    });
    if shown.is_err() {
        tracing::debug!(%window, "the window closed before its rename");
    }
}

/// Upstream's `confirmDelete` text.
#[must_use]
pub fn delete_body(sessions: usize, close_grace: bool) -> String {
    let suffix = if close_grace {
        "after a short undo window."
    } else {
        "immediately. You can reopen it from File > Open Recent."
    };
    if sessions == 1 {
        format!("This closes its session {suffix}")
    } else {
        format!("This closes {sessions} sessions {suffix}")
    }
}

/// A `window.*` request for window `window`, as the dialogs commit them.
#[must_use]
pub fn window_request(cmd: WindowCmd, window: Uuid, name: Option<String>) -> Request {
    Request {
        cmd: Command::Window(cmd),
        target: Some(uuid_string(window)),
        args: Some(Args {
            name,
            ..Args::default()
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delete_confirmation_uses_upstreams_words() {
        assert_eq!(
            delete_body(1, true),
            "This closes its session after a short undo window."
        );
        assert_eq!(
            delete_body(3, false),
            "This closes 3 sessions immediately. You can reopen it from File > Open Recent."
        );
    }

    #[test]
    fn a_window_request_targets_the_window_by_id() {
        let window = Uuid::from_u128(7);
        let request = window_request(WindowCmd::Rename, window, Some("w".into()));
        assert_eq!(request.cmd, Command::Window(WindowCmd::Rename));
        assert_eq!(request.target, Some(uuid_string(window)));
        assert_eq!(request.args.and_then(|args| args.name).as_deref(), Some("w"));
    }

    /// A loop at the frontend boundary: dialogs enqueue model work for the test to run.
    #[derive(Default)]
    struct Posted(std::sync::Mutex<Vec<ngterm_host::MainTask<AppState>>>);

    impl ngterm_host::MainLoop<AppState> for Posted {
        fn post(&self, task: ngterm_host::MainTask<AppState>) -> Result<(), ngterm_host::MainLoopClosed> {
            self.0.lock().unwrap().push(task);
            Ok(())
        }

        fn post_after(
            &self,
            _: std::time::Duration,
            _: ngterm_host::MainTask<AppState>,
        ) -> Result<(), ngterm_host::MainLoopClosed> {
            Ok(())
        }
    }

    impl Posted {
        fn drain(&self, state: &mut AppState) {
            for task in std::mem::take(&mut *self.0.lock().unwrap()) {
                task(state);
            }
        }
    }

    fn model() -> (AppState, Uuid) {
        let library = ngterm_core::model::WindowLibrary::seeded("/home/u");
        let window = library.active_window_id().unwrap();
        let core = ngterm_core::Core::new(
            ngterm_core::CoreConfig {
                app: ngterm_proto::AppIdentity::from_recorded("1.2.3", None),
                capabilities: Vec::new(),
            },
            ngterm_core::CoreState {
                library,
                settings: ngterm_core::persist::AppSettings {
                    confirm_close_session: Some(true),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        (AppState::new(core), window)
    }

    struct Blank;

    impl gpui_kit::Render for Blank {
        fn render(&mut self, _: &mut gpui_kit::Window, _: &mut gpui_kit::Context<Self>) -> impl gpui_kit::IntoElement {
            gpui_kit::div()
        }
    }

    #[gpui_kit::test]
    fn window_dialogs_cancel_or_commit_through_the_model(cx: &mut gpui_kit::TestAppContext) {
        cx.update(gpui_kit::init);
        let (_, cx) = cx.add_window_view(|_, _| Blank);
        let (mut state, window) = model();
        let queue = std::sync::Arc::new(Posted::default());
        let poster: Poster = std::sync::Arc::downgrade(&queue) as Poster;
        let handle = cx.update(|root, _| root.window_handle());
        keymap_dispatch::register_window(handle, KeyOrigin::Window(window));
        for (answer, expected) in [("Cancel", "window 1"), ("OK", "renamed")] {
            cx.cx.update(|cx| {
                perform(
                    &poster,
                    FrontendAction::RenameWindow {
                        window,
                        name: "renamed".into(),
                    },
                    false,
                    cx,
                );
            });
            cx.update(|root, cx| assert!(crate::modal::answer_front_alert(root, answer, cx)));
            queue.drain(&mut state);
            assert_eq!(state.core.library().windows()[0].name, expected);
        }
        state.core.library_mut().new_window(None);
        for answer in ["Cancel", "Delete"] {
            cx.cx.update(|cx| {
                perform(
                    &poster,
                    FrontendAction::DeleteWindow {
                        window,
                        name: "renamed".into(),
                        sessions: 1,
                    },
                    true,
                    cx,
                );
            });
            cx.update(|root, cx| assert!(crate::modal::answer_front_alert(root, answer, cx)));
            queue.drain(&mut state);
            assert_eq!(state.core.library().is_open(window), answer == "Cancel");
        }
        keymap_dispatch::unregister_window(handle);
    }

    #[gpui_kit::test]
    fn actions_without_dialogs_post_their_frontend_or_model_effect(cx: &mut gpui_kit::TestAppContext) {
        let (mut state, window) = model();
        let queue = std::sync::Arc::new(Posted::default());
        let poster: Poster = std::sync::Arc::downgrade(&queue) as Poster;
        cx.update(|cx| {
            perform(&poster, FrontendAction::OpenSettings(window), false, cx);
            perform(&poster, FrontendAction::CloseWindow(window), false, cx);
        });
        queue.drain(&mut state);
        let work = state.take_frontend_work();
        assert_eq!(work.settings, Some(window));
        assert_eq!(work.window_closes.len(), 1);
        let empty = state.core.library_mut().new_window(None);
        cx.update(|cx| {
            perform(
                &poster,
                FrontendAction::DeleteWindow {
                    window: empty,
                    name: "empty".into(),
                    sessions: 0,
                },
                false,
                cx,
            );
        });
        queue.drain(&mut state);
        assert!(!state.core.library().is_open(empty));
    }

    #[gpui_kit::test]
    fn fullscreen_and_bell_reach_a_registered_window(cx: &mut gpui_kit::TestAppContext) {
        cx.update(gpui_kit::init);
        let (_, cx) = cx.add_window_view(|_, _| Blank);
        let window = Uuid::new_v4();
        let handle = cx.update(|root, _| root.window_handle());
        keymap_dispatch::register_window(handle, KeyOrigin::Window(window));
        let poster = state::detached_poster();
        cx.cx
            .update(|cx| perform(&poster, FrontendAction::ToggleFullscreen(window), false, cx));
        assert!(cx.update(|root, _| root.is_fullscreen()));
        cx.cx
            .update(|cx| perform(&poster, FrontendAction::ToggleFullscreen(window), false, cx));
        assert!(!cx.update(|root, _| root.is_fullscreen()));
        cx.cx
            .update(|cx| perform(&poster, FrontendAction::AlertBell(window), false, cx));
        keymap_dispatch::unregister_window(handle);
    }

    #[gpui_kit::test]
    fn actions_for_a_window_that_has_gone_do_not_post_model_changes(cx: &mut gpui_kit::TestAppContext) {
        let window = Uuid::new_v4();
        let id = Uuid::new_v4();
        let queue = std::sync::Arc::new(Posted::default());
        let poster: Poster = std::sync::Arc::downgrade(&queue) as Poster;
        let actions = [
            FrontendAction::ToggleFullscreen(window),
            FrontendAction::AlertBell(window),
            FrontendAction::RenameWindow {
                window,
                name: "gone".into(),
            },
            FrontendAction::DeleteWindow {
                window,
                name: "gone".into(),
                sessions: 1,
            },
            FrontendAction::ConfirmCloseSession { window, session: id },
            FrontendAction::RenameSession { window, session: id },
            FrontendAction::RenameWorkspace { window, workspace: id },
            FrontendAction::DeleteWorkspace { window, workspace: id },
            FrontendAction::OpenDirectory { window, workspace: id },
        ];
        cx.update(|cx| {
            for action in actions {
                perform(&poster, action, false, cx);
            }
        });
        assert!(queue.0.lock().unwrap().is_empty());
    }

    #[gpui_kit::test]
    fn session_and_workspace_actions_reach_the_registered_sidebar(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;
        use std::rc::Rc;
        cx.update(gpui_kit::init);
        let (mut state, window) = model();
        let store = state.core.library().store(window).unwrap();
        let session = store.sessions().next().unwrap().id();
        let workspace = store.workspaces()[0].id();
        let queue = std::sync::Arc::new(Posted::default());
        let poster: Poster = std::sync::Arc::downgrade(&queue) as Poster;
        let dir = tempfile::tempdir().unwrap();
        let launcher: crate::pane::Launcher = Rc::new(|_| Box::new(|_| Err(ngterm_term::PaneError::Closed)));
        let env = Rc::new(crate::deck::DeckEnv::new(
            crate::pane::PaneContext {
                socket: dir.path().join("control.sock").to_string_lossy().into_owned(),
                program_version: "1.2.3".into(),
            },
            launcher,
            crate::terminal::TerminalConfig::default(),
            poster.clone(),
            crate::deck::Watermarks {
                state_dir: dir.path().to_owned(),
                config_with: Rc::new(|_| crate::terminal::TerminalConfig::default()),
            },
        ));
        let keys = poster.clone();
        let (view, cx) = cx.add_window_view(move |root, cx| {
            let deck = cx.new(|_| crate::deck::Deck::new(window, env));
            WindowView::new(window, deck, poster, 0, root, cx)
        });
        let sidebar = view.read_with(cx, |view, _| view.sidebar().clone());
        sidebar.update(cx, |sidebar, cx| sidebar.sync(&mut state.core, cx));
        let actions = [
            FrontendAction::ConfirmCloseSession { window, session },
            FrontendAction::DeleteWorkspace { window, workspace },
        ];
        for action in actions {
            cx.cx.update(|cx| perform(&keys, action, false, cx));
            cx.update(|root, cx| {
                if crate::modal::alert_open(root.window_handle(), cx) {
                    assert!(crate::modal::answer_front_alert(root, "Cancel", cx));
                }
            });
        }
        for action in [
            FrontendAction::RenameSession { window, session },
            FrontendAction::RenameWorkspace { window, workspace },
        ] {
            let id = match action {
                FrontendAction::RenameSession { session, .. } => session,
                _ => workspace,
            };
            cx.cx.update(|cx| perform(&keys, action, false, cx));
            assert_eq!(sidebar.read_with(cx, |sidebar, _| sidebar.editing()), Some(id));
            cx.simulate_keystrokes("escape");
        }
        view.update_in(cx, |view, root, cx| view.close(root, cx));
        queue.drain(&mut state);
        assert!(state.core.library().store(window).unwrap().session(session).is_some());
    }
}
