//! The settle cases carried over from the GTK build (`agterm-gtk/src/state.rs`), plus the frontend work the GPUI
//! loop lands with the app context.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use ngterm_core::PaneRole;
use ngterm_core::events::{EventCursor, ReadOptions};
use ngterm_core::model::{NewSession, WindowLibrary};
use ngterm_core::persist::{LibraryPersistence, SAVE_DEBOUNCE};
use ngterm_core::{CoreConfig, CoreState};
use ngterm_host::fakes::FakeSurface;
use ngterm_host::{MainLoopClosed, MainTask};
use ngterm_proto::command::WindowCmd;
use ngterm_proto::{AppIdentity, Args, Command, EventKind, Request};

use super::*;

/// A loop that records what is posted, with its delay, for the test to run.
struct RecordingLoop<S> {
    posted: Mutex<Vec<(Option<Duration>, MainTask<S>)>>,
}

impl<S> RecordingLoop<S> {
    fn new() -> Self {
        Self {
            posted: Mutex::new(Vec::new()),
        }
    }

    fn take(&self) -> Vec<(Option<Duration>, MainTask<S>)> {
        std::mem::take(&mut *self.posted.lock().unwrap())
    }
}

impl<S> MainLoop<S> for RecordingLoop<S> {
    fn post(&self, task: MainTask<S>) -> Result<(), MainLoopClosed> {
        self.posted.lock().unwrap().push((None, task));
        Ok(())
    }

    fn post_after(&self, delay: Duration, task: MainTask<S>) -> Result<(), MainLoopClosed> {
        self.posted.lock().unwrap().push((Some(delay), task));
        Ok(())
    }
}

fn core(library: WindowLibrary, persistence: Option<LibraryPersistence>) -> Core {
    Core::new(
        CoreConfig {
            app: AppIdentity::from_recorded("1.2.3", None),
            capabilities: Vec::new(),
        },
        CoreState {
            library,
            persistence,
            ..CoreState::default()
        },
    )
}

/// The loop the state posts to, and the strong handle its weak poster needs kept alive.
type Recorded = (Arc<RecordingLoop<AppState>>, Arc<dyn MainLoop<AppState>>);

/// Points `state` at a fresh recording loop; the caller keeps both handles.
fn record(state: &mut AppState) -> Recorded {
    let main_loop: Arc<RecordingLoop<AppState>> = Arc::new(RecordingLoop::new());
    let as_dyn: Arc<dyn MainLoop<AppState>> = Arc::clone(&main_loop) as _;
    state.poster = Arc::downgrade(&as_dyn);
    (main_loop, as_dyn)
}

/// A state directory with a short path, so the control socket it holds fits a Unix socket address: under `/tmp`, and
/// in the temporary directory on Windows (whose socket is a named pipe).
fn short_tempdir() -> tempfile::TempDir {
    let base = if cfg!(unix) {
        std::path::PathBuf::from("/tmp")
    } else {
        std::env::temp_dir()
    };
    tempfile::Builder::new().prefix("agt").tempdir_in(base).unwrap()
}

#[test]
fn a_commit_arms_its_saves_grace_timers_and_tree_changed_with_their_delays() {
    let dir = short_tempdir();
    let mut persistence = LibraryPersistence::new(dir.path());
    let library = persistence.bootstrap("/home/u");
    let window = library.active_window_id().unwrap();
    let mut core = core(library, Some(persistence));
    let store = core.library_mut().store_mut(window).unwrap();
    let workspace = store.workspaces()[0].id();
    let first = store.sessions().next().unwrap().id();
    store.add_session(workspace, NewSession::at_cwd("/srv")).unwrap();
    let structural = core.commit();
    assert_eq!(
        structural.tree_changed.len(),
        1,
        "a structural change invalidates the tree"
    );
    core.library_mut()
        .store_mut(window)
        .unwrap()
        .select_session(Some(first));
    let mut timers = core.commit();
    assert_eq!(timers.debounced.len(), 1, "a selection saves debounced");
    timers.tree_changed = structural.tree_changed;
    core.library_mut()
        .store_mut(window)
        .unwrap()
        .set_split_visibility(first, true, None);
    timers.pane_focused = core.commit().pane_focused;
    assert_eq!(timers.pane_focused.len(), 1, "a new split moves the selected pane");
    let timer = PendingCloseTimer {
        window,
        record: Uuid::new_v4(),
        grace: Duration::from_secs(3),
    };
    timers.pending_closes.push(timer);

    let main_loop = RecordingLoop::<Vec<String>>::new();
    let handlers = TimerHandlers {
        save: |seen: &mut Vec<String>, save| seen.push(format!("save {}", save.window)),
        finalize: |seen: &mut Vec<String>, timer| seen.push(format!("finalize {}", timer.record)),
        tree_changed: |seen: &mut Vec<String>, timer| seen.push(format!("tree {}", timer.window)),
        pane_focused: |seen: &mut Vec<String>, timer| seen.push(format!("pane {}", timer.window)),
    };
    arm_commit(&main_loop, &timers, &handlers);
    let mut seen = Vec::new();
    let delays: Vec<Option<Duration>> = main_loop
        .take()
        .into_iter()
        .map(|(delay, task)| {
            task(&mut seen);
            delay
        })
        .collect();
    assert_eq!(
        delays,
        [
            Some(SAVE_DEBOUNCE),
            Some(Duration::from_secs(3)),
            Some(TREE_CHANGED_DEBOUNCE),
            Some(PANE_FOCUS_DEBOUNCE)
        ]
    );
    assert_eq!(
        seen,
        [
            format!("save {window}"),
            format!("finalize {}", timer.record),
            format!("tree {window}"),
            format!("pane {window}")
        ]
    );
}

#[test]
fn a_settled_split_pane_selection_lands_pane_focused_when_its_timer_fires() {
    let (mut state, window, session) = state_with_terminal();
    let (main_loop, _held) = record(&mut state);
    state.settle();
    // the window's sessions are new to its pane.focused baseline until its debounce checks them
    for (_, task) in main_loop.take() {
        task(&mut state);
    }
    let _ = main_loop.take();
    let tail = state.core.events().ring().current_sequence();

    let store = state.core.library_mut().store_mut(window).unwrap();
    store.set_split_visibility(session, true, None);
    state.settle();
    let posted = main_loop.take();
    assert_eq!(posted.len(), 1, "one debounce for the window");
    for (delay, task) in posted {
        assert_eq!(delay, Some(PANE_FOCUS_DEBOUNCE));
        task(&mut state);
    }
    let options = ReadOptions {
        cursor: Some(EventCursor {
            run: state.core.events().ring().run(),
            after: tail,
        }),
        kinds: Some([EventKind::PaneFocused].into()),
        ..ReadOptions::default()
    };
    let batch = state.core.events().read(&options).unwrap();
    assert_eq!(batch.items.len(), 1);
    assert_eq!(batch.items[0].payload.pane.as_deref(), Some("right"));
    assert_eq!(
        batch.items[0].session,
        Some(ngterm_core::pane_env::uuid_string(session))
    );
}

#[test]
fn settle_sequences_events_and_lands_tree_changed_when_its_timer_fires() {
    let (mut state, window, _) = state_with_terminal();
    let (main_loop, _held) = record(&mut state);
    state.settle();
    let _ = main_loop.take();
    let tail = state.core.events().ring().current_sequence();

    let store = state.core.library_mut().store_mut(window).unwrap();
    let workspace = store.workspaces()[0].id();
    store.add_session(workspace, NewSession::at_cwd("/srv")).unwrap();
    state.settle();
    let ring = state.core.events().ring();
    assert_eq!(ring.current_sequence(), tail + 2, "created and selected, at once");
    let posted = main_loop.take();
    assert_eq!(
        posted.len(),
        2,
        "the tree.changed debounce, and the pane.focused check settling the new session"
    );
    for (delay, task) in posted {
        assert_eq!(delay, Some(TREE_CHANGED_DEBOUNCE));
        task(&mut state);
    }
    let options = ReadOptions {
        cursor: Some(EventCursor {
            run: state.core.events().ring().run(),
            after: tail + 2,
        }),
        ..ReadOptions::default()
    };
    let batch = state.core.events().read(&options).unwrap();
    assert_eq!(batch.items.len(), 1);
    assert_eq!(batch.items[0].kind, EventKind::TreeChanged);
    assert_eq!(batch.items[0].window, Some(ngterm_core::pane_env::uuid_string(window)));
}

#[test]
fn settle_writes_a_debounced_save_when_its_timer_fires() {
    let dir = short_tempdir();
    let mut persistence = LibraryPersistence::new(dir.path());
    let library = persistence.bootstrap("/home/u");
    let window = library.active_window_id().unwrap();
    let path = persistence.window_path(window);
    let mut state = AppState::new(core(library, Some(persistence)));
    let (main_loop, _held) = record(&mut state);
    state.settle();
    let _ = main_loop.take();

    let store = state.core.library_mut().store_mut(window).unwrap();
    let workspace = store.workspaces()[0].id();
    let added = store.add_session(workspace, NewSession::at_cwd("/srv")).unwrap();
    state.settle();
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(
        saved.contains(&ngterm_core::pane_env::uuid_string(added)),
        "a structural change saves now"
    );
    let _ = main_loop.take();

    let first = state
        .core
        .library()
        .store(window)
        .unwrap()
        .sessions()
        .next()
        .unwrap()
        .id();
    state
        .core
        .library_mut()
        .store_mut(window)
        .unwrap()
        .select_session(Some(first));
    state.settle();
    let posted = main_loop.take();
    assert_eq!(posted.len(), 1);
    let before = std::fs::read_to_string(&path).unwrap();
    for (delay, task) in posted {
        assert_eq!(delay, Some(SAVE_DEBOUNCE));
        task(&mut state);
    }
    let after = std::fs::read_to_string(&path).unwrap();
    assert_ne!(before, after, "the fired save wrote the new selection");
    assert!(after.contains(&ngterm_core::pane_env::uuid_string(first)));
}

#[test]
fn nothing_saves_after_the_quit_save() {
    let dir = short_tempdir();
    let mut persistence = LibraryPersistence::new(dir.path());
    let library = persistence.bootstrap("/home/u");
    let window = library.active_window_id().unwrap();
    let path = persistence.window_path(window);
    let mut state = AppState::new(core(library, Some(persistence)));
    state.save_on_quit();
    assert!(state.is_quitting());
    let saved = std::fs::read_to_string(&path).unwrap();
    let session = state
        .core
        .library()
        .store(window)
        .unwrap()
        .sessions()
        .next()
        .unwrap()
        .id();
    let token = "T".to_owned();
    state
        .core
        .library_mut()
        .store_mut(window)
        .unwrap()
        .session_mut(session)
        .unwrap()
        .surface
        .replace(Box::new(FakeSurface::new(token.clone())));
    state.pane_exited(session, &token);
    assert!(state.core.library().store(window).unwrap().session(session).is_some());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), saved);
}

/// A seeded window whose first session's primary terminal is a fake with pane token `T`.
fn state_with_terminal() -> (AppState, Uuid, Uuid) {
    let mut library = WindowLibrary::seeded("/home/u");
    let window = library.active_window_id().unwrap();
    let store = library.store_mut(window).unwrap();
    let session = store.sessions().next().unwrap().id();
    store
        .session_mut(session)
        .unwrap()
        .surface
        .replace(Box::new(FakeSurface::new("T")));
    (AppState::new(core(library, None)), window, session)
}

#[test]
fn a_command_waiting_on_its_terminal_answers_from_a_delayed_step() {
    let mut library = WindowLibrary::seeded("/home/u");
    let window = library.active_window_id().unwrap();
    let store = library.store_mut(window).unwrap();
    let session = store.sessions().next().unwrap().id();
    let surface = FakeSurface::new("T").with_realized(false);
    let probe = surface.probe();
    store.session_mut(session).unwrap().surface.replace(Box::new(surface));
    let mut state = AppState::new(core(library, None));
    let (main_loop, _held) = record(&mut state);

    let request = Request {
        cmd: Command::Session(SessionCmd::Type),
        target: None,
        args: Some(ngterm_proto::Args {
            text: Some("late".to_owned()),
            ..ngterm_proto::Args::default()
        }),
    };
    let reply = state.handle_request(&request);
    let posted = main_loop.take();
    let step = posted
        .into_iter()
        .find(|(delay, _)| *delay == Some(ngterm_core::POLL_INTERVAL));
    let (_, step) = step.expect("a delayed step");
    probe.set_realized(true);
    step(&mut state);
    let reply = reply.settled();
    assert!(reply.response.ok, "{:?}", reply.response);
    assert_eq!(probe.typed(), ["late"]);
}

#[test]
fn a_waiting_command_whose_step_is_dropped_answers_shutting_down() {
    let mut library = WindowLibrary::seeded("/home/u");
    let window = library.active_window_id().unwrap();
    let store = library.store_mut(window).unwrap();
    let session = store.sessions().next().unwrap().id();
    let surface = FakeSurface::new("T").with_realized(false);
    store.session_mut(session).unwrap().surface.replace(Box::new(surface));
    let mut state = AppState::new(core(library, None));
    let (main_loop, _held) = record(&mut state);
    let request = Request {
        cmd: Command::Session(SessionCmd::Type),
        target: None,
        args: Some(ngterm_proto::Args {
            text: Some("never".to_owned()),
            ..ngterm_proto::Args::default()
        }),
    };
    let reply = state.handle_request(&request);
    // the loop closed: its queued and delayed tasks are dropped unrun
    drop(main_loop.take());
    let reply = reply.settled();
    assert!(!reply.response.ok);
}

#[test]
fn the_primary_shell_exiting_closes_a_session_without_a_split() {
    let (mut state, window, session) = state_with_terminal();
    state.pane_exited(session, "other");
    assert!(
        state.core.library().store(window).unwrap().session(session).is_some(),
        "not its terminal"
    );
    state.pane_exited(session, "T");
    assert!(state.core.library().store(window).unwrap().session(session).is_none());
    state.pane_exited(session, "T");
}

#[test]
fn a_terminal_report_lands_on_its_session_in_any_window() {
    let (mut state, window, session) = state_with_terminal();
    state.core.library_mut().new_window(None);
    assert_ne!(state.core.library().active_window_id(), Some(window));
    state.report_pane(session, "T", &PaneReport::Pwd("/tmp/next".to_owned()));
    state.report_pane(session, "T", &PaneReport::Title("vim".to_owned()));
    state.report_pane(Uuid::new_v4(), "T", &PaneReport::Title("lost".to_owned()));
    state.report_pane(session, "other", &PaneReport::Title("stale".to_owned()));
    let reported = state.core.library().store(window).unwrap().session(session).unwrap();
    assert_eq!(reported.effective_cwd(), "/tmp/next");
    assert_eq!(reported.osc_title.as_deref(), Some("vim"));
    assert_eq!(reported.display_name(), "next", "the title names no session by default");
}

#[test]
fn a_terminal_report_lands_on_a_session_a_soft_close_holds() {
    let (mut state, window, session) = state_with_terminal();
    let store = state.core.library_mut().store_mut(window).unwrap();
    let workspace = store.workspaces()[0].id();
    store.add_session(workspace, NewSession::at_cwd("/srv")).unwrap();
    assert!(store.soft_close_session(session));
    assert!(
        store.session(session).is_none(),
        "out of the tree, held by the pending close"
    );
    state.report_pane(session, "T", &PaneReport::Pwd("/tmp/held".to_owned()));
    state.report_pane(session, "T", &PaneReport::Title("htop".to_owned()));
    let store = state.core.library_mut().store_mut(window).unwrap();
    assert!(store.undo_pending_close(None));
    let restored = store.session(session).unwrap();
    assert_eq!(
        restored.effective_cwd(),
        "/tmp/held",
        "the grace-period cwd survives the undo"
    );
    assert_eq!(restored.osc_title.as_deref(), Some("htop"));
}

/// [`state_with_terminal`] with a shown split whose pane is a fake with token `S`, the split focused.
fn state_with_split() -> (AppState, Uuid, Uuid) {
    let (mut state, window, session) = state_with_terminal();
    let store = state.core.library_mut().store_mut(window).unwrap();
    store.set_split_visibility(session, true, None);
    store
        .session_mut(session)
        .unwrap()
        .split_surface
        .replace(Box::new(FakeSurface::new("S")));
    (state, window, session)
}

#[test]
fn the_split_shell_exiting_collapses_the_split_and_the_primary_promotes_it() {
    let (mut state, window, session) = state_with_split();
    state.pane_exited(session, "S");
    let found = state.core.library().store(window).unwrap().session(session).unwrap();
    assert!(!found.split.exists && !found.split_surface.is_present());

    let (mut state, window, session) = state_with_split();
    state.pane_exited(session, "T");
    let found = state.core.library().store(window).unwrap().session(session).unwrap();
    assert_eq!(found.surface.get().map(ngterm_host::Surface::pane_token), Some("S"));
    assert!(!found.split.exists);
}

#[test]
fn a_split_pane_report_lands_on_the_split_fields() {
    let (mut state, window, session) = state_with_split();
    state.report_pane(session, "S", &PaneReport::Pwd("/srv/right".to_owned()));
    state.report_pane(session, "S", &PaneReport::Title("htop".to_owned()));
    let found = state.core.library().store(window).unwrap().session(session).unwrap();
    assert_eq!(found.split_cwd.as_deref(), Some("/srv/right"));
    assert_eq!(found.split_title.as_deref(), Some("htop"));
    assert_eq!(found.effective_cwd(), "/home/u", "the primary's stays");
}

#[test]
fn focusing_a_shown_split_pane_moves_the_split_focus_but_never_a_hidden_ones() {
    let (mut state, window, session) = state_with_split();
    let focused = |state: &AppState| {
        state
            .core
            .library()
            .store(window)
            .unwrap()
            .session(session)
            .unwrap()
            .split
            .focused
    };
    assert!(focused(&state));
    state.pane_focused(session, "T");
    assert!(!focused(&state));
    state.pane_focused(session, "S");
    assert!(focused(&state));
    let store = state.core.library_mut().store_mut(window).unwrap();
    store.set_split_visibility(session, false, None);
    state.pane_focused(session, "T");
    assert!(focused(&state), "a hidden split keeps its chosen pane");
}

#[test]
fn a_dragged_divider_stores_its_ratio_clamped() {
    let (mut state, window, session) = state_with_split();
    state.set_split_ratio(session, 0.3);
    let ratio = |state: &AppState| {
        state
            .core
            .library()
            .store(window)
            .unwrap()
            .session(session)
            .unwrap()
            .split
            .ratio
    };
    assert_eq!(ratio(&state), Some(0.3));
    state.set_split_ratio(session, 2.0);
    assert_eq!(ratio(&state), Some(ngterm_core::model::pane::SPLIT_RATIO_MAX));
    state.set_split_ratio(Uuid::new_v4(), 0.5);
}

#[test]
fn closing_a_window_saves_it_closes_its_store_and_tears_its_terminals_down() {
    let dir = short_tempdir();
    let mut persistence = LibraryPersistence::new(dir.path());
    let mut library = persistence.bootstrap("/home/u");
    let first = library.active_window_id().unwrap();
    let second = library.new_window(Some("second"));
    let path = persistence.window_path(second);
    let store = library.store_mut(second).unwrap();
    let session = store.sessions().next().unwrap().id();
    let terminal = FakeSurface::new("T");
    let probe = terminal.probe();
    store.session_mut(session).unwrap().surface.replace(Box::new(terminal));
    let mut state = AppState::new(core(library, Some(persistence)));
    // upstream's `windowShouldClose`: a window running sessions asks first, once
    state.close_requested(second);
    state.close_requested(second);
    assert!(state.core.library().is_open(second), "nothing closes before the answer");
    assert_eq!(probe.teardowns(), 0);
    assert_eq!(
        state.take_frontend_work().window_closes,
        [crate::state::WindowClose {
            window: second,
            name: "second".to_owned(),
            sessions: 1,
            mode: ngterm_core::persist::RestoreMode::None,
        }]
    );
    state.close_confirmed(second);
    assert!(!state.core.library().is_open(second));
    assert_eq!(state.core.library().windows().len(), 2, "its bundle stays");
    assert_eq!(state.core.library().active_window_id(), Some(first));
    assert_eq!(probe.teardowns(), 1, "its terminals go with it");
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(
        saved.contains(&ngterm_core::pane_env::uuid_string(session)),
        "written before it closed"
    );
}

/// Upstream's `clearFlags`: nothing while no session is flagged, else the frontend asks first and only the
/// confirmation unflags.
#[test]
fn clear_flagged_asks_first_and_only_the_confirmation_unflags() {
    let library = WindowLibrary::seeded("/home/u");
    let window = library.active_window_id().unwrap();
    let mut state = AppState::new(core(library, None));
    state.clear_flags_requested(window);
    assert_eq!(
        state.take_frontend_work().clear_flags,
        None,
        "nothing flagged: nothing to ask"
    );
    let flagged = |state: &AppState| state.core.library().store(window).unwrap().flagged_sessions().len();
    let store = state.core.library_mut().store_mut(window).unwrap();
    let session = store.sessions().next().unwrap().id();
    store.set_flag(true, session);
    state.clear_flags_requested(window);
    assert_eq!(state.take_frontend_work().clear_flags, Some((window, 1)));
    assert_eq!(flagged(&state), 1, "still flagged until confirmed");
    state.clear_flags_confirmed(window);
    assert_eq!(flagged(&state), 0);
}

#[test]
fn an_activated_window_becomes_frontmost_unless_quitting() {
    let mut library = WindowLibrary::seeded("/home/u");
    let first = library.active_window_id().unwrap();
    let second = library.new_window(None);
    let mut state = AppState::new(core(library, None));
    state.report_frontmost(first);
    assert_eq!(state.core.library().active_window_id(), Some(first));
    state.save_on_quit();
    state.report_frontmost(second);
    assert_eq!(state.core.library().active_window_id(), Some(first));
}

/// What a window built now hands its first activation.
fn built(state: &AppState) -> FirstFocus {
    FirstFocus::Built {
        frontmost_takes: state.core.frontmost_takes(),
    }
}

/// `window.<cmd>` with `args`, untargeted.
fn window_command(state: &mut AppState, cmd: WindowCmd, args: Args) {
    let response = state.core.dispatch(&Request {
        cmd: Command::Window(cmd),
        target: None,
        args: Some(args),
    });
    assert!(response.ok, "{:?}", response.error);
}

#[test]
fn a_new_windows_first_focus_leaves_frontmost_to_a_later_command() {
    let mut library = WindowLibrary::seeded("/home/u");
    let first = library.active_window_id().unwrap();
    // `window.new` made the second window frontmost
    let second = library.new_window(None);
    let mut state = AppState::new(core(library, None));
    let first_focus = Some(built(&state));
    // `window.select` took frontmost back before the window manager focused the newly mapped second window
    let select = crate::keymap_actions::window_request(WindowCmd::Select, first, None);
    assert!(state.core.dispatch(&select).ok);
    state.window_activated(second, first_focus);
    assert_eq!(state.key_window(), Some(second), "it holds the keyboard");
    assert_eq!(
        state.core.library().frontmost_window_id,
        Some(first),
        "the later command keeps frontmost"
    );
    // any later activation is the user's
    state.window_activated(second, None);
    assert_eq!(state.core.library().frontmost_window_id, Some(second));
}

#[test]
fn the_answer_to_a_raise_leaves_frontmost_to_a_later_command() {
    let mut library = WindowLibrary::seeded("/home/u");
    let first = library.active_window_id().unwrap();
    let second = library.new_window(None);
    let mut state = AppState::new(core(library, None));
    // `window.select` raises the first window, then `window.go` takes the second before the window manager answered
    let select = crate::keymap_actions::window_request(WindowCmd::Select, first, None);
    assert!(state.core.dispatch(&select).ok);
    let raised = FirstFocus::Raised {
        frontmost_takes: state.core.frontmost_takes(),
    };
    let go = crate::keymap_actions::window_request(WindowCmd::Select, second, None);
    assert!(state.core.dispatch(&go).ok);
    state.window_activated(first, Some(raised));
    assert_eq!(state.key_window(), Some(first), "it holds the keyboard");
    assert_eq!(
        state.core.library().frontmost_window_id,
        Some(second),
        "the later command keeps frontmost"
    );
    // with no command since the raise, its answer counts
    let raised = FirstFocus::Raised {
        frontmost_takes: state.core.frontmost_takes(),
    };
    state.window_activated(second, Some(raised));
    assert_eq!(state.core.library().frontmost_window_id, Some(second));
    let select = crate::keymap_actions::window_request(WindowCmd::Select, first, None);
    assert!(state.core.dispatch(&select).ok);
    let raised = FirstFocus::Raised {
        frontmost_takes: state.core.frontmost_takes(),
    };
    state.window_activated(first, Some(raised));
    assert_eq!(state.core.library().frontmost_window_id, Some(first));
    // the user's own activation always counts
    state.window_activated(second, None);
    assert_eq!(state.core.library().frontmost_window_id, Some(second));
}

#[test]
fn a_new_windows_first_focus_leaves_frontmost_to_a_later_new_window() {
    let library = WindowLibrary::seeded("/home/u");
    let first = library.active_window_id().unwrap();
    let mut state = AppState::new(core(library, None));
    let new_window = |state: &mut AppState| {
        window_command(state, WindowCmd::New, Args::default());
        let library = state.core.library();
        library.frontmost_window_id.unwrap()
    };
    let earlier = new_window(&mut state);
    let earlier_focus = Some(built(&state));
    let later = new_window(&mut state);
    assert!(![first, earlier].contains(&later));
    // the window manager focuses the earlier window after the later `window.new`
    state.window_activated(earlier, earlier_focus);
    assert_eq!(state.core.library().frontmost_window_id, Some(later));
}

#[test]
fn a_new_windows_first_focus_counts_without_a_command_since() {
    let mut library = WindowLibrary::seeded("/home/u");
    let first = library.active_window_id().unwrap();
    let second = library.new_window(None);
    let mut state = AppState::new(core(library, None));
    let first_focus = Some(built(&state));
    // the user went to the first window, then into the second one the window manager left unfocused
    state.window_activated(first, None);
    state.window_activated(second, first_focus);
    assert_eq!(state.core.library().frontmost_window_id, Some(second));
}

#[test]
fn a_reopened_windows_first_focus_counts() {
    let mut library = WindowLibrary::seeded("/home/u");
    let first = library.active_window_id().unwrap();
    let second = library.new_window(None);
    library.close_window(second);
    library.set_frontmost(Some(first));
    let mut state = AppState::new(core(library, None));
    // a notification's click reopens its closed window, its session gone: no command took frontmost
    assert!(state.core.reveal_notification(second, Uuid::new_v4(), PaneRole::Main));
    assert!(state.core.library().is_open(second));
    assert_eq!(state.core.library().frontmost_window_id, Some(first));
    state.window_activated(second, Some(built(&state)));
    assert_eq!(state.core.library().frontmost_window_id, Some(second));
}

#[test]
fn a_clicked_banner_raises_its_window_with_the_servers_token() {
    let (mut state, window, session) = state_with_terminal();
    let identity = ngterm_core::notification_identity(window, session, PaneRole::Main);
    state.reveal_notification(&identity, Some("token".to_owned()));
    assert_eq!(
        state.take_frontend_work().activations,
        [Activation {
            token: Some("token".to_owned()),
            startup_id: None,
        }],
        "the server's token raises the window"
    );
    state.reveal_notification(&identity, None);
    assert!(
        state.take_frontend_work().activations.is_empty(),
        "without a token the window request raises it"
    );
    state.reveal_notification("not an identity", Some("token".to_owned()));
    assert!(state.take_frontend_work().activations.is_empty(), "nothing to reveal");
    // a `notify` banner's control slot reveals the main pane, raising the window the same way
    let control = ngterm_core::control_notification_identity(window, session);
    state.reveal_notification(&control, Some("control".to_owned()));
    assert_eq!(
        state.take_frontend_work().activations,
        [Activation {
            token: Some("control".to_owned()),
            startup_id: None,
        }],
        "the control slot's click reveals its session"
    );
}

#[test]
fn a_restored_windows_first_focus_counts_over_the_saved_frontmost() {
    let mut library = WindowLibrary::seeded("/home/u");
    let first = library.active_window_id().unwrap();
    let saved = library.new_window(None);
    let mut state = AppState::new(core(library, None));
    assert_eq!(state.core.library().frontmost_window_id, Some(saved));
    // the window manager kept the keyboard on the first window mapped
    state.window_activated(first, Some(built(&state)));
    assert_eq!(state.core.library().frontmost_window_id, Some(first));
}

#[test]
fn a_parked_new_windows_first_focus_leaves_frontmost_where_its_command_handed_it() {
    let library = WindowLibrary::seeded("/home/u");
    let first = library.active_window_id().unwrap();
    let mut state = AppState::new(core(library, None));
    let args = Args {
        minimized: Some(true),
        ..Args::default()
    };
    window_command(&mut state, WindowCmd::New, args);
    let library = state.core.library();
    let second = library.open_ids().into_iter().find(|id| *id != first).unwrap();
    assert_eq!(library.frontmost_window_id, Some(first), "handed off");
    state.window_activated(second, Some(FirstFocus::Parked));
    assert_eq!(state.core.library().frontmost_window_id, Some(first));
}

#[test]
fn sidebar_actions_select_sessions_and_fold_workspaces() {
    let (mut state, window, first) = state_with_terminal();
    let store = state.core.library_mut().store_mut(window).unwrap();
    let workspace = store.workspaces()[0].id();
    let second = store.add_session(workspace, NewSession::at_cwd("/srv")).unwrap();
    assert_eq!(store.selected_session_id(), Some(second));
    state.sidebar_action(
        window,
        SidebarAction::SelectSessions {
            active: first,
            selection: vec![first],
        },
    );
    let store = state.core.library().store(window).unwrap();
    assert_eq!(store.selected_session_id(), Some(first));
    state.sidebar_toggle_workspace(window, workspace, false);
    let store = state.core.library().store(window).unwrap();
    assert!(!store.workspace(workspace).unwrap().is_expanded);
    state.sidebar_toggle_workspace(window, workspace, true);
    let store = state.core.library().store(window).unwrap();
    assert!(store.workspace(workspace).unwrap().is_expanded);
}

/// Runs the armed zero-delay fires of `main_loop` on `state`; how many there were.
fn fire_zero_delays(main_loop: &RecordingLoop<AppState>, state: &mut AppState) -> usize {
    let fires: Vec<_> = main_loop
        .take()
        .into_iter()
        .filter(|(delay, _)| *delay == Some(Duration::ZERO))
        .collect();
    let count = fires.len();
    for (_, task) in fires {
        task(state);
    }
    count
}

#[test]
fn a_sidebar_rename_holds_auto_follow_until_it_ends() {
    use ngterm_core::auto_follow::FollowConfig;
    use ngterm_core::model::{AgentIndicator, AgentStatus};

    let (mut state, window, first) = state_with_terminal();
    let (main_loop, _held) = record(&mut state);
    state.settle();
    let _ = main_loop.take();
    let store = state.core.library_mut().store_mut(window).unwrap();
    let workspace = store.workspaces()[0].id();
    let second = store.add_session(workspace, NewSession::at_cwd("/srv")).unwrap();
    let blocked = AgentIndicator {
        status: AgentStatus::Blocked,
        ..AgentIndicator::default()
    };
    store.set_agent_indicator(blocked, first);
    let selected = |state: &AppState| state.core.library().store(window).unwrap().selected_session_id();

    state.sidebar_rename_began(window);
    state.core.set_auto_follow(Some(FollowConfig {
        timeout: Duration::ZERO,
        stay_on_active: false,
    }));
    state.settle();
    assert_eq!(
        fire_zero_delays(&main_loop, &mut state),
        1,
        "a follow is armed mid-rename"
    );
    assert_eq!(
        selected(&state),
        Some(second),
        "but does not fire while the field is open"
    );

    state.sidebar_rename_ended(
        window,
        Some(SidebarAction::RenameSession {
            session: second,
            name: "build".into(),
        }),
    );
    let store = state.core.library().store(window).unwrap();
    assert_eq!(store.session(second).unwrap().display_name(), "build");
    // the suppression was lifted: the next armed follow jumps to the blocked session
    state.sidebar_action(
        window,
        SidebarAction::SelectSessions {
            active: second,
            selection: vec![second],
        },
    );
    assert_eq!(fire_zero_delays(&main_loop, &mut state), 1);
    assert_eq!(selected(&state), Some(first));
}

#[test]
fn a_sidebar_click_is_activity_and_an_armed_fire_follows_to_the_blocked_session() {
    use ngterm_core::auto_follow::FollowConfig;
    use ngterm_core::model::{AgentIndicator, AgentStatus};

    let (mut state, window, first) = state_with_terminal();
    state.set_key_window(Some(window));
    let (main_loop, _held) = record(&mut state);
    state.settle();
    let _ = main_loop.take();
    assert_eq!(state.core.tree(None).unwrap().idle_ms, None);
    let store = state.core.library_mut().store_mut(window).unwrap();
    let workspace = store.workspaces()[0].id();
    let second = store.add_session(workspace, NewSession::at_cwd("/srv")).unwrap();
    let blocked = AgentIndicator {
        status: AgentStatus::Blocked,
        ..AgentIndicator::default()
    };
    store.set_agent_indicator(blocked, first);
    state.core.set_auto_follow(Some(FollowConfig {
        timeout: Duration::ZERO,
        stay_on_active: false,
    }));
    state.sidebar_action(
        window,
        SidebarAction::SelectSessions {
            active: second,
            selection: vec![second],
        },
    );
    assert!(state.core.tree(None).unwrap().idle_ms.is_some());
    assert_eq!(fire_zero_delays(&main_loop, &mut state), 1);
    let store = state.core.library().store(window).unwrap();
    assert_eq!(store.selected_session_id(), Some(first));
}

#[test]
fn quit_and_activation_requests_wait_for_the_frontend_in_order() {
    let (mut state, _, _) = state_with_terminal();
    assert_eq!(state.take_frontend_work(), FrontendWork::default());
    let first = Activation {
        token: Some("a".to_owned()),
        startup_id: None,
    };
    let second = Activation {
        token: None,
        startup_id: Some("b_TIME7".to_owned()),
    };
    state.activation_requested(first.clone());
    state.activation_requested(second.clone());
    state.request_quit();
    assert_eq!(
        state.take_frontend_work(),
        FrontendWork {
            activations: vec![first, second],
            quit: true,
            ..FrontendWork::default()
        }
    );
    assert_eq!(state.take_frontend_work(), FrontendWork::default(), "taken once");
}

#[test]
fn no_window_is_raised_once_the_quit_save_ran() {
    let (mut state, _, _) = state_with_terminal();
    state.save_on_quit();
    state.activation_requested(Activation::default());
    state.request_quit();
    assert_eq!(
        state.take_frontend_work(),
        FrontendWork {
            activations: Vec::new(),
            quit: true,
            ..FrontendWork::default()
        }
    );
}

#[test]
fn attaching_settles_and_arms_on_the_loop() {
    let dir = short_tempdir();
    let mut persistence = LibraryPersistence::new(dir.path());
    let library = persistence.bootstrap("/home/u");
    let window = library.active_window_id().unwrap();
    let mut state = AppState::new(core(library, Some(persistence)));
    let main_loop: Arc<RecordingLoop<AppState>> = Arc::new(RecordingLoop::new());
    let as_dyn: Arc<dyn MainLoop<AppState>> = Arc::clone(&main_loop) as _;
    state.attach(Arc::downgrade(&as_dyn));
    assert!(state.poster().upgrade().is_some());
    let store = state.core.library_mut().store_mut(window).unwrap();
    let workspace = store.workspaces()[0].id();
    store.add_session(workspace, NewSession::at_cwd("/srv")).unwrap();
    state.settle();
    assert!(
        main_loop
            .take()
            .iter()
            .any(|(delay, _)| *delay == Some(TREE_CHANGED_DEBOUNCE)),
        "the attached loop gets the commit's timers"
    );
    drop((as_dyn, main_loop));
    assert!(state.poster().upgrade().is_none(), "held weakly");
}

#[test]
fn a_notify_lands_its_banner_and_its_sound_with_the_frontend() {
    let (mut state, window, session) = state_with_terminal();
    state
        .core
        .change_settings(ngterm_core::settings_model::SettingsChange::Notifications(
            ngterm_core::settings_model::NotificationsChange::Sound(Some("Glass".to_owned())),
        ));
    let response = state.core.dispatch(&Request {
        cmd: Command::Notify,
        target: Some(ngterm_core::pane_env::uuid_string(session)),
        args: Some(Args {
            title: Some("t".to_owned()),
            body: Some("done".to_owned()),
            ..Args::default()
        }),
    });
    assert!(response.ok, "{response:?}");
    state.settle();
    let work = state.take_frontend_work();
    assert_eq!(work.banners.len(), 1, "{:?}", work.banners);
    assert!(matches!(
        &work.banners[0],
        ngterm_core::NotificationEffect::Post(post) if post.window == window && post.body == "done"
    ));
    assert_eq!(work.sounds, ["Glass"], "a posted batch sounds the notification sound");
    state.settle();
    assert_eq!(state.take_frontend_work(), FrontendWork::default(), "landed once");
}

#[test]
fn pane_relative_links_use_their_own_reported_local_directory() {
    let (mut state, _, session) = state_with_terminal();
    let directory = tempfile::tempdir().unwrap();
    // Windows does not allow `?` in a filename; its other literal path characters are still exercised.
    let name = if cfg!(windows) {
        "name%20# with spaces.txt"
    } else {
        "name%20?# with spaces.txt"
    };
    let path = directory.path().join(name);
    std::fs::write(&path, "local file").unwrap();
    state.report_pane(
        session,
        "T",
        &PaneReport::Pwd(directory.path().to_string_lossy().into_owned()),
    );
    state.follow_pane_link(
        &format!("./{name}"),
        session,
        "T",
        false,
        crate::terminal::LinkKind::Url,
    );
    assert_eq!(state.frontend.reveals, vec![path]);
    state.follow_pane_link("./stale.txt", session, "gone", false, crate::terminal::LinkKind::Url);
    assert_eq!(state.frontend.reveals.len(), 1, "a replaced pane cannot route a link");
}

#[test]
fn remote_primary_paths_stay_remote_while_scratch_paths_stay_local() {
    let mut library = WindowLibrary::seeded("/local/home");
    let store = library.active_store_mut().unwrap();
    let workspace = store.workspaces()[0].id();
    let session = store
        .add_session(
            workspace,
            NewSession {
                remote_host: Some("remote".to_owned()),
                ..NewSession::at_cwd("/local/home")
            },
        )
        .unwrap();
    let found = store.session_mut(session).unwrap();
    found.surface.replace(Box::new(FakeSurface::new("R")));
    found.scratch_surface.replace(Box::new(FakeSurface::new("S")));
    let mut state = AppState::new(core(library, None));
    let remote_path = std::env::temp_dir().join("remote-origin").join("a.png");
    state.follow_pane_link(
        &remote_path.to_string_lossy(),
        session,
        "R",
        false,
        crate::terminal::LinkKind::Url,
    );
    state.follow_pane_link("relative.txt", session, "R", false, crate::terminal::LinkKind::Url);
    assert!(state.frontend.reveals.is_empty());
    let local = std::env::temp_dir().join("remote.png");
    let file_url = url::Url::from_file_path(&local).unwrap();
    state.follow_pane_link(file_url.as_str(), session, "R", false, crate::terminal::LinkKind::Url);
    assert_eq!(state.frontend.reveals, vec![local]);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("local.txt");
    std::fs::write(&path, "scratch file").unwrap();
    state.report_pane(
        session,
        "S",
        &PaneReport::Pwd(directory.path().to_string_lossy().into_owned()),
    );
    state.follow_pane_link("./local.txt", session, "S", false, crate::terminal::LinkKind::Url);
    assert_eq!(state.frontend.reveals.last(), Some(&path));
    let _ = state
        .core
        .library_mut()
        .store_for_session_mut(session)
        .unwrap()
        .session_mut(session)
        .unwrap()
        .scratch_surface
        .take();
    state.settle();
    assert!(!state.link_directories.contains_key(&(session, "S".to_owned())));
}

#[test]
fn only_detected_existing_relative_links_with_an_osc7_directory_resolve() {
    let (mut state, _, session) = state_with_terminal();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("notes.txt");
    std::fs::write(&path, "notes").unwrap();
    state
        .core
        .library_mut()
        .store_for_session_mut(session)
        .unwrap()
        .session_mut(session)
        .unwrap()
        .set_current_cwd(&directory.path().to_string_lossy());
    // Even an existing model cwd is not an OSC 7 report and cannot authorize relative resolution.
    state.follow_pane_link("notes.txt", session, "T", false, crate::terminal::LinkKind::Url);
    assert!(state.frontend.reveals.is_empty());
    state.report_pane(
        session,
        "T",
        &PaneReport::Pwd(directory.path().to_string_lossy().into_owned()),
    );
    // OSC 8 links stay literal, even when their relative spelling names an existing local file.
    state.follow_pane_link("notes.txt", session, "T", false, crate::terminal::LinkKind::Osc8);
    state.follow_pane_link("missing.txt", session, "T", false, crate::terminal::LinkKind::Url);
    assert!(state.frontend.reveals.is_empty());
    state.follow_pane_link("notes.txt", session, "T", false, crate::terminal::LinkKind::Url);
    assert_eq!(state.frontend.reveals, [path]);
}

#[test]
fn detected_relative_resolution_refuses_network_and_automount_targets_before_existence_checks() {
    for cwd in ["/net/server", "/misc/share"] {
        assert_eq!(detected_relative_path("notes.md", cwd), None);
    }
    for raw in [
        r"\\server\share\notes.md",
        r"\\?\UNC\server\share\notes.md",
        "/\\server/share/notes.md",
    ] {
        assert_eq!(
            detected_relative_path(raw, &std::env::temp_dir().to_string_lossy()),
            None
        );
    }
}

fn send(state: &mut AppState, cmd: Command, target: Option<String>, args: Args) -> ngterm_proto::Response {
    state
        .handle_request(&Request {
            cmd,
            target,
            args: Some(args),
        })
        .settled()
        .response
}

#[test]
fn the_keyboard_owner_survives_a_late_blur_from_another_window() {
    let (mut state, first, _) = state_with_terminal();
    let second = state.core.library_mut().new_window(None);
    state.set_key_window(Some(first));
    state.set_key_window(Some(second));
    state.key_window_lost(first);
    assert_eq!(state.key_window(), Some(second));
    state.key_window_lost(second);
    assert_eq!(state.key_window(), None);
}

#[test]
fn sidebar_width_rounds_clamps_and_ignores_missing_windows() {
    let (mut state, window, _) = state_with_terminal();
    state.sidebar_resized(Uuid::new_v4(), 300.0);
    state.sidebar_resized(window, 300.4);
    assert!((state.core.library().store(window).unwrap().sidebar().width - 300.0).abs() < f64::EPSILON);
    state.sidebar_resized(window, 300.49);
    assert!((state.core.library().store(window).unwrap().sidebar().width - 300.0).abs() < f64::EPSILON);
    // to the nearest pixel, not down
    state.sidebar_resized(window, 300.6);
    assert!((state.core.library().store(window).unwrap().sidebar().width - 301.0).abs() < f64::EPSILON);
    state.sidebar_resized(window, 0.0);
    assert!(
        (state.core.library().store(window).unwrap().sidebar().width - ngterm_core::model::SIDEBAR_WIDTH_MIN).abs()
            < f64::EPSILON
    );
    state.sidebar_resized(window, 1e6);
    assert!(
        (state.core.library().store(window).unwrap().sidebar().width - ngterm_core::model::SIDEBAR_WIDTH_MAX).abs()
            < f64::EPSILON
    );
}

#[test]
fn frontend_requests_are_drained_once_and_settings_stop_at_quit() {
    let (mut state, window, _) = state_with_terminal();
    state.show_welcome();
    state.ask_quit("End the running sessions?".into());
    state.open_settings(window);
    let work = state.take_frontend_work();
    assert!(work.welcome);
    assert_eq!(work.quit_prompt.as_deref(), Some("End the running sessions?"));
    assert_eq!(work.settings, Some(window));
    assert_eq!(state.take_frontend_work(), FrontendWork::default());
    state.save_on_quit();
    state.open_settings(window);
    assert_eq!(state.take_frontend_work(), FrontendWork::default());
}

#[test]
fn a_deferred_step_can_wait_again_and_panics_answer_the_socket() {
    let (mut state, _, _) = state_with_terminal();
    let (main_loop, _held) = record(&mut state);
    let (reply, sender) = Reply::deferred();
    state.resume(
        Command::Tree,
        Box::new(|_| Dispatched::Pending {
            after: Duration::from_millis(9),
            resume: Box::new(|_| {
                Dispatched::Done(ngterm_proto::Response::with_result(serde_json::json!({"ready": true})))
            }),
        }),
        Waiter::Socket(sender),
    );
    for (delay, task) in main_loop.take() {
        if delay == Some(Duration::from_millis(9)) {
            task(&mut state);
        }
    }
    assert_eq!(
        reply.settled().response.result,
        Some(serde_json::json!({"ready": true}))
    );
    let (reply, sender) = Reply::deferred();
    state.resume(
        Command::Tree,
        Box::new(|_| panic!("host boundary failed")),
        Waiter::Socket(sender),
    );
    let response = reply.settled().response;
    assert!(!response.ok);
    assert_eq!(
        response,
        ngterm_proto::Response::from(ControlError::HandlerPanicked(Command::Tree))
    );
}

#[test]
fn a_detached_loop_drops_tasks_and_deferred_replies_answer_shutting_down() {
    let (state, _, _) = state_with_terminal();
    let held = Arc::new(());
    let in_task = Arc::clone(&held);
    post(state.poster(), move |_| drop(in_task));
    assert_eq!(Arc::strong_count(&held), 1, "the task is dropped, not kept");
    let (reply, sender) = Reply::deferred();
    state.schedule(
        Command::Tree,
        Duration::ZERO,
        Box::new(|_| panic!("must not run")),
        Waiter::Socket(sender),
    );
    assert_eq!(
        reply.settled().response,
        ngterm_proto::Response::from(ControlError::ShuttingDown)
    );
}

#[test]
fn a_notification_is_suppressed_only_when_its_pane_and_window_hold_the_keyboard() {
    let (mut state, _, session) = state_with_terminal();
    state.terminal_notification(
        session,
        "T",
        "build",
        "done",
        NotifyFocus {
            pane: true,
            active: true,
        },
    );
    assert!(state.take_frontend_work().banners.is_empty());
    state.terminal_notification(
        session,
        "T",
        "build",
        "done",
        NotifyFocus {
            pane: true,
            active: false,
        },
    );
    assert!(!state.take_frontend_work().banners.is_empty());
    state.save_on_quit();
    let _ = state.take_frontend_work();
    state.terminal_notification(session, "T", "build", "done", NotifyFocus::default());
    assert!(state.take_frontend_work().banners.is_empty());
}

#[test]
fn info_and_hud_expiries_ignore_stale_timers_and_hide_the_current_message() {
    for cmd in [SessionCmd::Info, SessionCmd::HudOpen] {
        let (mut state, window, session) = state_with_terminal();
        let dir = tempfile::tempdir().unwrap();
        state.core.set_hud_body_dir(dir.path().to_path_buf());
        let (main_loop, _held) = record(&mut state);
        let mut batches = Vec::new();
        for text in ["old", "new"] {
            let response = send(
                &mut state,
                Command::Session(cmd),
                Some(session.to_string()),
                Args {
                    mode: (cmd == SessionCmd::Info).then(|| "set".into()),
                    text: (cmd == SessionCmd::Info).then(|| text.into()),
                    message: (cmd == SessionCmd::HudOpen).then(|| text.into()),
                    hide_after: Some(0.01),
                    ..Args::default()
                },
            );
            assert!(response.ok, "{response:?}");
            batches.push(main_loop.take());
        }
        let mut batches = batches.into_iter();
        for (delay, task) in batches.next().unwrap() {
            if delay == Some(Duration::from_millis(10)) {
                task(&mut state);
            }
        }
        let found = state.core.library().store(window).unwrap().session(session).unwrap();
        let message = if cmd == SessionCmd::Info {
            found.effective_info().unwrap().source.clone()
        } else {
            found.overlay.effective_hud_spec().unwrap().message
        };
        assert_eq!(message, "new", "the earlier timer cannot hide its replacement");
        for (delay, task) in batches.next().unwrap() {
            if delay == Some(Duration::from_millis(10)) {
                task(&mut state);
            }
        }
        let found = state.core.library().store(window).unwrap().session(session).unwrap();
        assert!(found.effective_info().is_none());
        assert!(found.overlay.hud.is_none());
    }
}

#[test]
fn builtin_frontend_actions_keep_their_window_and_soft_close_setting() {
    use ngterm_core::keymap::BuiltinAction;
    let (mut state, window, _) = state_with_terminal();
    state.keymap_fired(
        KeyOrigin::Window(window),
        Fired::Builtin(BuiltinAction::ToggleFullscreen),
        None,
    );
    assert_eq!(
        state.take_frontend_work().actions,
        [(FrontendAction::ToggleFullscreen(window), true)]
    );
    state.keymap_fired(KeyOrigin::Quick, Fired::Builtin(BuiltinAction::ToggleFullscreen), None);
    assert_eq!(
        state.take_frontend_work().actions,
        [(FrontendAction::ToggleFullscreen(window), true)]
    );
    state
        .core
        .update_settings(|settings| settings.close_grace_undo_enabled = Some(false));
    state.keymap_fired(
        KeyOrigin::Window(window),
        Fired::Builtin(BuiltinAction::ToggleFullscreen),
        None,
    );
    assert_eq!(
        state.take_frontend_work().actions,
        [(FrontendAction::ToggleFullscreen(window), false)]
    );
    state.save_on_quit();
    state.keymap_fired(
        KeyOrigin::Window(window),
        Fired::Builtin(BuiltinAction::ToggleFullscreen),
        None,
    );
    assert!(state.take_frontend_work().actions.is_empty());
}

#[test]
fn malformed_or_unavailable_links_leave_only_allowed_desktop_work() {
    let (mut state, _, session) = state_with_terminal();
    for raw in ["", "javascript:alert(1)", "https://example.com/\n"] {
        state.follow_link(raw, ClickOrigin::Pane(session), false);
    }
    assert!(state.take_frontend_work().open_urls.is_empty());
    state.follow_link("https://example.com/docs", ClickOrigin::Pane(Uuid::new_v4()), false);
    assert_eq!(state.take_frontend_work().open_urls, ["https://example.com/docs"]);
    state.follow_link("https://example.com/browser", ClickOrigin::Pane(session), true);
    assert_eq!(state.take_frontend_work().open_urls, ["https://example.com/browser"]);
    let path = std::env::temp_dir().join("missing-document.md");
    state.open_document_link(path.clone(), Uuid::new_v4(), None, false);
    assert_eq!(state.take_frontend_work().reveals, [path]);
}

fn open_input_ask(state: &mut AppState, session: Uuid, gui: bool) -> String {
    let response = send(
        state,
        Command::Ask(ngterm_proto::command::AskCmd::Open),
        Some(session.to_string()),
        Args {
            title: Some("Name".into()),
            input: Some("line".into()),
            style: gui.then(|| "gui".into()),
            ..Args::default()
        },
    );
    assert!(response.ok, "{response:?}");
    result_id(&response).unwrap()
}

fn ask_result(state: &mut AppState, id: &str) -> serde_json::Value {
    let response = send(
        state,
        Command::Ask(ngterm_proto::command::AskCmd::Result),
        Some(id.into()),
        Args::default(),
    );
    assert!(response.ok, "{response:?}");
    response.result.unwrap()["ask"].clone()
}

#[test]
fn session_and_window_asks_answer_with_their_edited_drafts_and_ignore_stale_ids() {
    use crate::ask_dialog::form::DraftEdit;
    let (mut state, window, session) = state_with_split();
    for gui in [false, true] {
        let id = open_input_ask(&mut state, session, gui);
        let edit = DraftEdit::Text("typed name".into());
        if gui {
            state.edit_window_ask_draft(window, "stale", &edit);
            state.edit_window_ask_draft(window, &id, &edit);
            state.answer_window_ask_button(window, "stale", 0);
            state.answer_window_ask_button(window, &id, 0);
        } else {
            state.select_ask_pane(Uuid::new_v4());
            state.select_ask_pane(session);
            state.edit_session_ask_draft(session, "stale", &edit);
            state.edit_session_ask_draft(session, &id, &edit);
            state.answer_session_ask_button(session, "stale", 0);
            state.answer_session_ask_button(session, &id, 0);
        }
        let answer = ask_result(&mut state, &id);
        assert_eq!(answer["result"], "answered");
        assert_eq!(answer["text"], "typed name");
    }
}

#[test]
fn ask_dismissals_and_picker_answers_reach_the_wire() {
    use ngterm_proto::{ControlAskOutcome, ControlAskResult, ControlPickOutcome, ControlPickResult};
    let (mut state, window, session) = state_with_terminal();
    for gui in [false, true] {
        let id = open_input_ask(&mut state, session, gui);
        let escaped = ControlAskResult::outcome(ControlAskOutcome::Escaped);
        if gui {
            state.answer_window_ask(window, &id, escaped);
        } else {
            state.answer_session_ask(session, &id, escaped);
        }
        assert_eq!(ask_result(&mut state, &id)["result"], "escaped");
    }
    let response = send(
        &mut state,
        Command::Pick(ngterm_proto::command::PickCmd::Open),
        None,
        Args {
            items: Some(vec![ngterm_proto::PickItem {
                id: "one".into(),
                label: "One".into(),
                ..Default::default()
            }]),
            ..Args::default()
        },
    );
    assert!(response.ok, "{response:?}");
    let id = result_id(&response).unwrap();
    state.resolve_pick(
        window,
        "stale",
        ControlPickResult::outcome(ControlPickOutcome::Cancelled),
    );
    assert!(state.core.modal_pending(window));
    state.resolve_pick(window, &id, ControlPickResult::outcome(ControlPickOutcome::Cancelled));
    let result = send(
        &mut state,
        Command::Pick(ngterm_proto::command::PickCmd::Result),
        Some(id),
        Args::default(),
    );
    assert_eq!(result.result.unwrap()["pick"]["result"], "cancelled");
    assert!(!state.core.modal_pending(window));
}

/// A gone loop and a loop refusing new work have the same user-visible shutdown behavior.
struct ClosedLoop;

impl MainLoop<AppState> for ClosedLoop {
    fn post(&self, _: MainTask<AppState>) -> Result<(), MainLoopClosed> {
        Err(MainLoopClosed)
    }
    fn post_after(&self, _: Duration, _: MainTask<AppState>) -> Result<(), MainLoopClosed> {
        Err(MainLoopClosed)
    }
}

#[test]
fn a_closed_loop_releases_deferred_replies_and_inbound_stream_capacity() {
    let (mut state, _, _) = state_with_terminal();
    let main_loop: Arc<dyn MainLoop<AppState>> = Arc::new(ClosedLoop);
    state.poster = Arc::downgrade(&main_loop);
    let (reply, sender) = Reply::deferred();
    state.schedule(
        Command::Tree,
        Duration::ZERO,
        Box::new(|_| panic!("closed")),
        Waiter::Socket(sender),
    );
    assert_eq!(
        reply.settled().response,
        ngterm_proto::Response::from(ControlError::ShuttingDown)
    );
    // a line the loop refused frees its place at once: one more than the limit gets through without blocking
    let mut reader = presentation_reader(state.poster.clone(), 1);
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for _ in 0..=ngterm_core::remote::INBOUND_LIMIT {
            reader(b"ignored".to_vec());
        }
        done.send(()).unwrap();
    });
    finished
        .recv_timeout(Duration::from_secs(5))
        .expect("the reader waits on capacity a closed loop never gives back");
}

#[test]
fn presentation_lines_and_closes_are_main_loop_work_and_stand_down_after_quit() {
    let (mut state, _, session) = state_with_live_terminal();
    let (main_loop, _held) = record(&mut state);
    let hello = br#"{"kind":"hello","gen":0,"rev":0,"hello":{"version":1,"kinds":["status"],"mode":"mirror"}}"#;
    let viewer = Presenter::default();
    let serial = state
        .core
        .adopt_presentation_stream(&session.to_string(), Box::new(viewer.clone()));
    presentation_reader(state.poster.clone(), serial)(hello.to_vec());
    assert!(viewer.0.borrow().is_empty(), "the line waits for the loop");
    let posted = main_loop.take();
    assert_eq!(posted.len(), 1);
    for (_, task) in posted {
        task(&mut state);
    }
    assert_eq!(viewer.last("hello")["kind"], "hello", "the hello subscribed the viewer");
    presentation_closer(state.poster.clone(), serial)();
    assert!(state.core.has_presentation_streams(), "the close waits for the loop");
    for (_, task) in main_loop.take() {
        task(&mut state);
    }
    assert!(!state.core.has_presentation_streams());

    let late = Presenter::default();
    let serial = state
        .core
        .adopt_presentation_stream(&session.to_string(), Box::new(late.clone()));
    state.save_on_quit();
    // every line is answered by the loop's release, so more than the limit never blocks the reader
    let mut reader = presentation_reader(state.poster.clone(), serial);
    for _ in 0..=ngterm_core::remote::INBOUND_LIMIT {
        reader(hello.to_vec());
        for (_, task) in main_loop.take() {
            task(&mut state);
        }
    }
    assert!(late.0.borrow().is_empty(), "lines after quit are not read");
    presentation_closer(state.poster.clone(), serial)();
    for (_, task) in main_loop.take() {
        task(&mut state);
    }
    assert!(
        state.core.has_presentation_streams(),
        "a close after quit is left to the exit"
    );
}

fn state_with_live_terminal() -> (AppState, Uuid, Uuid) {
    let (mut state, window, session) = state_with_terminal();
    state
        .core
        .library_mut()
        .store_mut(window)
        .unwrap()
        .session_mut(session)
        .unwrap()
        .surface
        .replace(Box::new(FakeSurface::new("T").with_live_session(true)));
    (state, window, session)
}

#[test]
fn a_presentation_handoff_streams_hello_and_snapshot_then_releases_on_disconnect() {
    use std::io::{BufRead as _, Write as _};
    let (mut state, _, session) = state_with_live_terminal();
    let (main_loop, _held) = record(&mut state);
    let request = Request {
        target: Some(session.to_string()),
        ..Request::new(Command::Live(LiveCmd::Present))
    };
    let mut reply = state.handle_request(&request);
    assert!(reply.response.ok, "{:?}", reply.response);
    let owner = reply.stream.take().expect("presentation owns the connection");
    let (server, mut peer) = ngterm_server::endpoint::pair().unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let worker = std::thread::spawn(move || owner.adopt(server, Vec::new()));
    peer.write_all(b"{\"kind\":\"hello\",\"gen\":0,\"rev\":0,\"hello\":{\"version\":1,\"kinds\":[\"status\",\"hud\"],\"mode\":\"mirror\"}}\n").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let posted = main_loop.take();
        let received = posted.iter().any(|(delay, _)| delay.is_none());
        for (delay, task) in posted {
            if delay.is_none() {
                task(&mut state);
            }
        }
        if received {
            break;
        }
        assert!(Instant::now() < deadline, "presentation did not post its hello");
        std::thread::yield_now();
    }
    let mut reader = std::io::BufReader::new(peer.try_clone().unwrap());
    let mut kinds = Vec::new();
    for _ in 0..2 {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let frame: serde_json::Value = serde_json::from_str(&line).unwrap();
        kinds.push(frame["kind"].as_str().unwrap().to_owned());
    }
    // the viewer hangs up by closing its end: a Windows pipe client's shutdown ends its server's read only once its
    // last handle closes
    drop((reader, peer));
    worker.join().unwrap();
    for (_, task) in main_loop.take() {
        task(&mut state);
    }
    assert_eq!(kinds, ["hello", "snapshot"]);
    assert!(!state.core.has_presentation_streams());
}

/// Presents `session` to a viewer: the stream's owner, and the hello deadline and heartbeat it armed, in that order
/// (both wait the same 10 seconds, so the order is what tells them apart).
fn present(
    state: &mut AppState,
    main_loop: &RecordingLoop<AppState>,
    session: Uuid,
) -> (
    Box<dyn ngterm_server::StreamOwner>,
    MainTask<AppState>,
    MainTask<AppState>,
) {
    let mut reply = state.handle_request(&Request {
        target: Some(session.to_string()),
        ..Request::new(Command::Live(LiveCmd::Present))
    });
    assert!(reply.response.ok, "{:?}", reply.response);
    let timers: Vec<_> = main_loop
        .take()
        .into_iter()
        .filter(|(delay, _)| *delay == Some(ngterm_core::remote::HELLO_DEADLINE))
        .map(|(_, task)| task)
        .collect();
    let Ok([deadline, heartbeat]) = <[_; 2]>::try_from(timers) else {
        panic!("a presentation arms its hello deadline and the heartbeat");
    };
    (reply.stream.take().unwrap(), deadline, heartbeat)
}

/// Whether `posted` holds a re-armed heartbeat.
fn heartbeat_posted(posted: &[(Option<Duration>, MainTask<AppState>)]) -> bool {
    posted
        .iter()
        .any(|(delay, _)| *delay == Some(ngterm_core::remote::HEARTBEAT_INTERVAL))
}

#[test]
fn presentation_deadlines_and_heartbeats_stop_after_the_last_stream_or_quit() {
    use std::io::Read as _;
    let (mut state, _, session) = state_with_live_terminal();
    let (main_loop, _held) = record(&mut state);
    let (owner, deadline, heartbeat) = present(&mut state, &main_loop, session);
    assert!(state.core.has_presentation_streams());
    state.arm_presentation_heartbeat();
    assert!(main_loop.take().is_empty(), "the existing heartbeat is not duplicated");
    heartbeat(&mut state);
    let rearmed = main_loop.take();
    assert!(heartbeat_posted(&rearmed), "the heartbeat re-arms while a stream is up");
    assert!(state.heartbeat_armed);

    // a viewer that sends no hello is closed at its deadline
    let (server, mut peer) = ngterm_server::endpoint::pair().unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let worker = std::thread::spawn(move || owner.adopt(server, Vec::new()));
    deadline(&mut state);
    assert_eq!(peer.read(&mut [0; 64]).unwrap(), 0, "the deadline closed the stream");
    worker.join().unwrap();
    for (_, task) in main_loop.take() {
        task(&mut state);
    }
    assert!(!state.core.has_presentation_streams());
    for (_, task) in rearmed {
        task(&mut state);
    }
    assert!(
        !heartbeat_posted(&main_loop.take()),
        "no stream is left to keep the heartbeat"
    );
    assert!(!state.heartbeat_armed);

    // the quit stops the heartbeat while the stream it closed waits for its close
    let (owner, _, heartbeat) = present(&mut state, &main_loop, session);
    state.save_on_quit();
    assert!(state.core.has_presentation_streams());
    heartbeat(&mut state);
    assert!(main_loop.take().is_empty());
    assert!(!state.heartbeat_armed);
    owner.abandon();
}

#[test]
fn a_ready_pane_gets_one_spawn_grant_and_quit_prevents_new_grants() {
    let (mut state, _, session) = state_with_terminal();
    state.pane_spawn_ready(session, "T");
    let spawns = state.take_spawns();
    assert_eq!(spawns.len(), 1);
    assert_eq!(spawns[0].session, session);
    assert_eq!(spawns[0].token, "T");
    assert!(state.take_spawns().is_empty());
    state.save_on_quit();
    state.pane_spawn_ready(session, "T");
    assert!(state.take_spawns().is_empty());
}

#[test]
fn stale_remote_reports_are_ignored_before_and_after_quit() {
    let (mut state, _, session) = state_with_terminal();
    let key = ngterm_core::remote::LinkKey {
        session,
        client: 99,
        launch: 1,
    };
    let helper = ngterm_core::remote::replica_page::HelperKey {
        page: Uuid::new_v4(),
        launch: 1,
    };
    for quitting in [false, true] {
        if quitting {
            state.save_on_quit();
        }
        state.viewer_line(key, b"{}");
        state.viewer_closed(key, "closed");
        state.page_helper_line(helper, b"{}");
        state.page_helper_closed(helper, "closed");
        state.retry_remote_links();
        assert_eq!(state.take_frontend_work(), FrontendWork::default());
    }
}

/// The origin's presentation transport, with frames read as the remote presenter sees them.
#[derive(Clone, Default)]
struct Presenter(Rc<RefCell<Vec<serde_json::Value>>>);

impl ngterm_core::LineChannel for Presenter {
    fn send(&self, line: Vec<u8>) -> bool {
        self.0.borrow_mut().push(serde_json::from_slice(&line).unwrap());
        true
    }
    fn shutdown(&self) {}
}

impl Presenter {
    fn last(&self, kind: &str) -> serde_json::Value {
        self.0
            .borrow()
            .iter()
            .rev()
            .find(|frame| frame["kind"] == kind)
            .unwrap()
            .clone()
    }
}

/// A local live pane reported as a follower, with a presenter ready to receive its overlays.
fn presented_state() -> (AppState, Uuid, Presenter, tempfile::TempDir) {
    let (mut state, window, session) = state_with_terminal();
    let dir = tempfile::tempdir().unwrap();
    let integration = dir.path().join("shell-integration/zsh");
    std::fs::create_dir_all(&integration).unwrap();
    std::fs::write(integration.join(".zshenv"), "").unwrap();
    state
        .core
        .set_restore_launch(ngterm_core::restore::mode::RestoreLaunchDecision::new(
            ngterm_core::persist::RestoreMode::Live,
            None,
        ));
    state.core.install_live_sessions(ngterm_core::LiveInstall {
        host: Arc::new(ngterm_host::fakes::FakeSessionHost::new()),
        endpoint: ngterm_proto::ControlLiveEndpoint {
            executable: "ngterm-session".into(),
            socket_directory: dir.path().to_string_lossy().into_owned(),
        },
        wrap: ngterm_core::live::support::Inputs {
            executable: std::env::current_exe().unwrap(),
            login_shell: Some("/bin/zsh".into()),
            resources_dir: Some(dir.path().to_owned()),
            state_dir: dir.path().join("state"),
            ..Default::default()
        },
        state_dir: dir.path().to_owned(),
        outdated_before: None,
    });
    let found = state
        .core
        .library_mut()
        .store_mut(window)
        .unwrap()
        .session_mut(session)
        .unwrap();
    let token = found.pane_identity.to_string().to_uppercase();
    found
        .surface
        .replace(Box::new(FakeSurface::new(&token).with_live_session(true)));
    let seed = state.core.request_pane_spawn(session, &token).unwrap();
    let nonce = seed
        .env
        .iter()
        .find(|(key, _)| key == "NGTERM_LIVE_MANAGED")
        .expect("the live attachment nonce")
        .1
        .clone();
    state.report_pane(
        session,
        &token,
        &PaneReport::Title(format!("ngterm-live-role;{nonce}:follower:1")),
    );
    let presenter = Presenter::default();
    let serial = state
        .core
        .adopt_presentation_stream(&session.to_string(), Box::new(presenter.clone()));
    state.core.presentation_line(serial, &serde_json::to_vec(&serde_json::json!({"kind":"hello", "gen":0, "rev":0, "hello":{"version":1, "kinds":ngterm_core::remote::hub::SUPPORTED_KINDS, "mode":"presenter"}})).unwrap());
    let generation = presenter.last("hello")["gen"].clone();
    state.core.presentation_line(
        serial,
        &serde_json::to_vec(&serde_json::json!({"kind":"presenter.acquire", "gen":generation, "rev":1})).unwrap(),
    );
    assert_eq!(presenter.last("presenter.granted")["kind"], "presenter.granted");
    (state, session, presenter, dir)
}

/// Runs posted socket callbacks until `done` observes their effect, without advancing timers.
fn run_until(state: &mut AppState, main_loop: &RecordingLoop<AppState>, done: impl Fn(&AppState) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done(state) {
        for (delay, task) in main_loop.take() {
            if delay.is_none() {
                task(state);
            }
        }
        assert!(Instant::now() < deadline, "the main-loop effect did not arrive");
        std::thread::yield_now();
    }
}

#[test]
fn a_job_handoff_reports_the_helpers_exit_code_to_the_overlay_result() {
    use std::io::{BufRead as _, Write as _};
    let (mut state, session, presenter, _dir) = presented_state();
    let (main_loop, _held) = record(&mut state);
    assert!(
        send(
            &mut state,
            Command::Session(SessionCmd::OverlayOpen),
            Some(session.to_string()),
            Args {
                command: Some("printf done".into()),
                follow: Some(true),
                ..Args::default()
            }
        )
        .ok
    );
    let job = presenter.last("overlay.request")["overlay"]["job"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut reply = state.handle_request(&Request {
        target: Some(job),
        ..Request::new(Command::Session(SessionCmd::OverlayJobRun))
    });
    assert!(reply.response.ok, "{:?}", reply.response);
    let owner = reply.stream.take().unwrap();
    let (server, mut peer) = ngterm_server::endpoint::pair().unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let worker = std::thread::spawn(move || owner.adopt(server, Vec::new()));
    let mut line = String::new();
    std::io::BufReader::new(peer.try_clone().unwrap())
        .read_line(&mut line)
        .unwrap();
    let frame: serde_json::Value = serde_json::from_str(&line).unwrap();
    peer.write_all(b"{\"kind\":\"started\"}\n{\"kind\":\"exited\",\"code\":7}\n")
        .unwrap();
    // the helper hangs up by closing its end: a Windows pipe has no half-close
    drop(peer);
    worker.join().unwrap();
    for (_, task) in main_loop.take() {
        task(&mut state);
    }
    assert_eq!(frame["kind"], "context");
    let result = send(
        &mut state,
        Command::Session(SessionCmd::OverlayResult),
        Some(session.to_string()),
        Args::default(),
    );
    assert!(result.ok, "{result:?}");
    assert_eq!(result.result.unwrap()["exitCode"], 7);
}

fn open_served_page(
    state: &mut AppState,
    session: Uuid,
    presenter: &Presenter,
    dir: &std::path::Path,
) -> (String, String) {
    let file = dir.join("index.html");
    std::fs::write(&file, "<p>served</p>").unwrap();
    let response = send(
        state,
        Command::Session(SessionCmd::OverlayOpen),
        Some(session.to_string()),
        Args {
            html: Some(file.to_string_lossy().into_owned()),
            cwd: Some(dir.to_string_lossy().into_owned()),
            javascript: Some(true),
            follow: Some(true),
            ..Args::default()
        },
    );
    assert!(response.ok, "{response:?}");
    let page = response.result.unwrap()["pageID"].as_str().unwrap().to_owned();
    let token = presenter.last("page.request")["page"]["token"]
        .as_str()
        .unwrap()
        .to_owned();
    (page, token)
}

#[test]
fn a_page_channel_runs_bridge_requests_and_returns_their_wire_reply() {
    use std::io::{BufRead as _, Write as _};
    let (mut state, session, presenter, dir) = presented_state();
    let (main_loop, _held) = record(&mut state);
    let (_, token) = open_served_page(&mut state, session, &presenter, dir.path());
    let mut reply = state.handle_request(&Request {
        target: Some(token),
        ..Request::new(Command::Session(SessionCmd::OverlayPageServe))
    });
    assert!(reply.response.ok, "{:?}", reply.response);
    let (server, mut peer) = ngterm_server::endpoint::pair().unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let owner = reply.stream.take().unwrap();
    let worker = std::thread::spawn(move || owner.adopt(server, Vec::new()));
    let frame = serde_json::json!({"kind":"bridge", "id":1, "request":r#"{"cmd":"session.rename","args":{"name":"from page"}}"#});
    writeln!(peer, "{frame}").unwrap();
    run_until(&mut state, &main_loop, |state| {
        state.core.library().held_session(session).unwrap().display_name() == "from page"
    });
    let mut line = String::new();
    std::io::BufReader::new(peer.try_clone().unwrap())
        .read_line(&mut line)
        .unwrap();
    // the helper hangs up by closing its end: a Windows pipe client's shutdown ends its server's read only once its
    // last handle closes
    drop(peer);
    worker.join().unwrap();
    for (_, task) in main_loop.take() {
        task(&mut state);
    }
    let frame: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(frame["kind"], "reply");
    assert_eq!(frame["id"], 1);
    let response: ngterm_proto::Response = serde_json::from_str(frame["response"].as_str().unwrap()).unwrap();
    assert!(response.ok, "{response:?}");
}

#[test]
fn abandoning_a_page_claim_ends_its_page_and_a_late_adoption_stays_closed() {
    let (mut state, session, presenter, dir) = presented_state();
    let (main_loop, _held) = record(&mut state);
    let (page, token) = open_served_page(&mut state, session, &presenter, dir.path());
    let mut reply = state.handle_request(&Request {
        target: Some(token),
        ..Request::new(Command::Session(SessionCmd::OverlayPageServe))
    });
    assert!(reply.response.ok, "{:?}", reply.response);
    reply.stream.take().unwrap().abandon();
    for (_, task) in main_loop.take() {
        task(&mut state);
    }
    assert_eq!(presenter.last("page.close")["page"]["page"], page);
    let closes = presenter
        .0
        .borrow()
        .iter()
        .filter(|frame| frame["kind"] == "page.close")
        .count();
    let mut late = state.page_serve_stream(reply.response);
    late.stream.take().unwrap().abandon();
    for (_, task) in main_loop.take() {
        task(&mut state);
    }
    assert_eq!(
        presenter
            .0
            .borrow()
            .iter()
            .filter(|frame| frame["kind"] == "page.close")
            .count(),
        closes
    );
}

/// The HTML engine boundary: an available engine that accepts navigation.
struct HtmlEngine;

impl ngterm_core::html::HtmlHost for HtmlEngine {
    fn persistent_store_failure(&self) -> Option<String> {
        None
    }
    fn navigate(&self, _: Uuid, _: ngterm_core::html::HtmlNavigation) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn document_probes_open_supported_documents_and_reveal_failed_probes() {
    use image::ImageEncoder as _;
    for (name, bytes, image) in [
        ("notes.md", b"# Notes".to_vec(), false),
        ("broken.png", b"not an image".to_vec(), true),
        (
            "pixel.png",
            {
                let mut bytes = Vec::new();
                image::codecs::png::PngEncoder::new(&mut bytes)
                    .write_image(&[255, 0, 0, 255], 1, 1, image::ExtendedColorType::Rgba8)
                    .unwrap();
                bytes
            },
            true,
        ),
    ] {
        let (mut state, _, session) = state_with_terminal();
        let (main_loop, _held) = record(&mut state);
        state.core.set_html_host(Box::new(HtmlEngine));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        state.open_document_link(path.clone(), session, Some(60), image);
        run_until(&mut state, &main_loop, |state| {
            !state.frontend.reveals.is_empty()
                || state
                    .core
                    .library()
                    .held_session(session)
                    .unwrap()
                    .overlay
                    .page()
                    .is_some()
        });
        if name == "broken.png" {
            assert_eq!(state.take_frontend_work().reveals, [path]);
        } else {
            assert!(state.take_frontend_work().reveals.is_empty());
        }
    }
}

#[test]
fn web_links_open_overlays_and_quit_discards_an_inflight_document_probe() {
    let (mut state, _, session) = state_with_terminal();
    state.core.set_html_host(Box::new(HtmlEngine));
    state.follow_link("https://example.com", ClickOrigin::Pane(session), false);
    // Browser is the default.
    assert_eq!(state.take_frontend_work().open_urls, ["https://example.com"]);
    state
        .core
        .change_settings(ngterm_core::settings_model::SettingsChange::General(
            ngterm_core::settings_model::GeneralChange::LinkOpenMode(LinkOpenMode::Overlay),
        ));
    state.follow_link("https://example.com/overlay", ClickOrigin::Pane(session), false);
    assert!(
        state
            .core
            .library()
            .held_session(session)
            .unwrap()
            .overlay
            .page()
            .is_some()
    );
    assert!(state.take_frontend_work().open_urls.is_empty());
    let (main_loop, _held) = record(&mut state);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.md");
    std::fs::write(&path, "# Notes").unwrap();
    state.open_document_link(path, session, None, false);
    state.save_on_quit();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let tasks = main_loop.take();
        if !tasks.is_empty() {
            for (_, task) in tasks {
                task(&mut state);
            }
            break;
        }
        assert!(Instant::now() < deadline, "the document probe never completed");
        std::thread::yield_now();
    }
    assert_eq!(state.take_frontend_work(), FrontendWork::default());
}

#[test]
fn the_remotes_watch_refreshes_the_model_and_stops_on_drop() {
    let (mut state, _, _) = state_with_terminal();
    let dir = tempfile::tempdir().unwrap();
    state
        .core
        .set_config_locator(ngterm_core::paths::ConfigLocator::capture(
            &ngterm_core::paths::MapEnv::new().with("NGTERM_STATE_DIR", dir.path()),
        ));
    let path = dir.path().join("config/remotes.conf");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "studio\n").unwrap();
    state.core.load_remotes_at_launch();
    let (main_loop, _held) = record(&mut state);
    state.watch_remotes();
    assert!(state.remotes_watcher.is_some());
    // The watch's first callback is sent after its OS watch has been armed.
    let next_callback = |state: &mut AppState, what: &str| {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let tasks = main_loop.take();
            if !tasks.is_empty() {
                for (_, task) in tasks {
                    task(state);
                }
                return;
            }
            assert!(Instant::now() < deadline, "{what}");
            std::thread::yield_now();
        }
    };
    next_callback(&mut state, "watch never armed");
    std::fs::write(&path, "studio\nlaptop\n").unwrap();
    run_until(&mut state, &main_loop, |state| {
        state.core.configured_remotes().len() == 2
    });
    assert_eq!(state.core.watched_remotes_path(), path);
    state.save_on_quit();
    std::fs::write(&path, "studio\n").unwrap();
    next_callback(&mut state, "the write after quit was not seen");
    assert_eq!(
        state.core.configured_remotes().len(),
        2,
        "quit ignores late watch callbacks"
    );
    // the callbacks of that burst stop with the watch: the drop joins its thread
    drop(state.remotes_watcher.take());
    drop(main_loop.take());
    std::fs::write(&path, "studio\nlaptop\nbuild\n").unwrap();
    std::thread::sleep(crate::remotes_watcher::DEBOUNCE * 6);
    assert!(main_loop.take().is_empty(), "a dropped watch sends nothing");
}

#[test]
fn page_bridge_requests_type_immediately_or_resume_when_the_terminal_is_ready() {
    let (mut state, window, session) = state_with_terminal();
    let surface = FakeSurface::new("T").with_realized(false);
    let probe = surface.probe();
    state
        .core
        .library_mut()
        .store_mut(window)
        .unwrap()
        .session_mut(session)
        .unwrap()
        .surface
        .replace(Box::new(surface));
    let (main_loop, _held) = record(&mut state);
    let request = Request {
        cmd: Command::Session(SessionCmd::Type),
        target: Some(session.to_string()),
        args: Some(Args {
            text: Some("from bridge".into()),
            ..Args::default()
        }),
    };
    state.handle_page_request(&request, u64::MAX);
    let posted = main_loop.take();
    probe.set_realized(true);
    for (_, task) in posted {
        task(&mut state);
    }
    assert_eq!(probe.typed(), ["from bridge"]);
    state.handle_page_request(&request, u64::MAX);
    assert_eq!(probe.typed(), ["from bridge", "from bridge"]);
}

#[test]
fn window_state_zoom_and_user_activity_reach_the_model() {
    let (mut state, window, session) = state_with_terminal();
    let changed = ngterm_core::WindowState {
        fullscreen: true,
        ..Default::default()
    };
    state.window_state_changed(window, changed);
    assert_eq!(state.core.window_state(window), Some(changed));
    state.core.set_zoom_target(
        window,
        Some(ngterm_core::zoom::SurfaceId {
            session,
            surface: ngterm_core::zoom::ZoomSurface::Primary,
        }),
    );
    state.exit_zoom(window);
    assert!(state.core.zoom_target(window).is_none());
    state.overlay_keystroke(session);
    assert!(state.core.tree(None).unwrap().idle_ms.is_some());
    let (mut quick, _, _) = state_with_terminal();
    assert!(quick.core.tree(None).unwrap().idle_ms.is_none());
    quick.quick_keystroke();
    assert!(quick.core.tree(None).unwrap().idle_ms.is_some());
    state.save_on_quit();
    state.window_state_changed(window, ngterm_core::WindowState::default());
    assert_eq!(state.core.window_state(window), Some(changed));
}

fn configure_remote_choices(state: &mut AppState, dir: &std::path::Path) {
    state
        .core
        .set_config_locator(ngterm_core::paths::ConfigLocator::capture(
            &ngterm_core::paths::MapEnv::new().with("NGTERM_STATE_DIR", dir),
        ));
    let path = dir.join("config/remotes.conf");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, "studio\nlaptop\n").unwrap();
    state.core.load_remotes_at_launch();
}

#[test]
fn the_remote_tick_rearms_while_a_host_picker_waits_and_stops_after_cancel() {
    let (mut state, window, _) = state_with_terminal();
    let dir = tempfile::tempdir().unwrap();
    configure_remote_choices(&mut state, dir.path());
    let (main_loop, _held) = record(&mut state);
    state.core.attach_remote(window);
    state.settle();
    let id = state.core.tree(None).unwrap().pick_pending.unwrap();
    let first = main_loop.take();
    assert!(state.remote_tick_armed);
    state.arm_remote_tick();
    assert!(main_loop.take().is_empty(), "a waiting picker has one tick");
    for (_, task) in first {
        task(&mut state);
    }
    assert!(state.core.modal_pending(window));
    let second = main_loop.take();
    assert!(second.iter().any(|(delay, _)| delay.is_some()), "the tick re-armed");
    assert!(state.remote_tick_armed);
    state.resolve_pick(
        window,
        &id,
        ngterm_proto::ControlPickResult::outcome(ngterm_proto::ControlPickOutcome::Cancelled),
    );
    assert!(!state.core.modal_pending(window));
    for (_, task) in second.into_iter().chain(main_loop.take()) {
        task(&mut state);
    }
    assert_eq!(state.core.remote_wake(), None);
    assert!(main_loop.take().is_empty(), "nothing remote waits: the tick stops");
    assert!(!state.remote_tick_armed);
}

#[test]
fn a_remote_tick_armed_before_the_quit_does_not_rearm_after_it() {
    let (mut state, window, _) = state_with_terminal();
    let dir = tempfile::tempdir().unwrap();
    configure_remote_choices(&mut state, dir.path());
    let (main_loop, _held) = record(&mut state);
    state.core.attach_remote(window);
    state.settle();
    let armed = main_loop.take();
    assert!(state.remote_tick_armed);
    state.save_on_quit();
    for (_, task) in armed {
        task(&mut state);
    }
    assert!(main_loop.take().is_empty());
    assert!(!state.remote_tick_armed);
}

#[test]
fn a_closed_loop_leaves_remote_timers_unarmed_and_panels_usable() {
    let (mut state, window, _) = state_with_terminal();
    let dir = tempfile::tempdir().unwrap();
    configure_remote_choices(&mut state, dir.path());
    let main_loop: Arc<dyn MainLoop<AppState>> = Arc::new(ClosedLoop);
    state.poster = Arc::downgrade(&main_loop);
    state.core.attach_remote(window);
    state.settle();
    assert!(!state.remote_tick_armed);
    assert!(state.core.modal_pending(window));
    let id = state.core.tree(None).unwrap().pick_pending.unwrap();
    state.resolve_pick(
        window,
        &id,
        ngterm_proto::ControlPickResult::outcome(ngterm_proto::ControlPickOutcome::Cancelled),
    );
    assert!(!state.core.modal_pending(window));
}

#[test]
fn quitting_ignores_late_ask_edits_answers_and_window_confirmations() {
    use crate::ask_dialog::form::DraftEdit;
    use ngterm_proto::{ControlAskOutcome, ControlAskResult, ControlPickOutcome, ControlPickResult};
    let (mut state, window, session) = state_with_terminal();
    let id = open_input_ask(&mut state, session, true);
    state.save_on_quit();
    let before = serde_json::to_value(state.core.tree(None).unwrap()).unwrap();
    let answer = ControlAskResult::outcome(ControlAskOutcome::Escaped);
    state.edit_window_ask_draft(window, &id, &DraftEdit::Text("late".into()));
    state.edit_session_ask_draft(session, &id, &DraftEdit::Text("late".into()));
    state.answer_window_ask_button(window, &id, 0);
    state.answer_session_ask_button(session, &id, 0);
    state.answer_window_ask(window, &id, answer.clone());
    state.answer_session_ask(session, &id, answer);
    state.resolve_pick(window, &id, ControlPickResult::outcome(ControlPickOutcome::Cancelled));
    state.close_requested(window);
    state.close_confirmed(window);
    state.clear_flags_requested(window);
    state.clear_flags_confirmed(window);
    state.report_pane(session, "T", &PaneReport::Title("late".into()));
    state.window_command(WindowCmd::Rename, window, Some("late".into()));
    state.pane_focused(session, "T");
    state.save_on_quit();
    assert_eq!(serde_json::to_value(state.core.tree(None).unwrap()).unwrap(), before);
    assert_eq!(state.take_frontend_work(), FrontendWork::default());
}

#[test]
fn a_soft_close_timer_finalizes_its_held_session_and_tears_down_its_terminal() {
    let (mut state, window, session) = state_with_terminal();
    let store = state.core.library_mut().store_mut(window).unwrap();
    let workspace = store.workspaces()[0].id();
    let survivor = store.add_session(workspace, NewSession::at_cwd("/srv")).unwrap();
    let surface = FakeSurface::new("T");
    let probe = surface.probe();
    store.session_mut(session).unwrap().surface.replace(Box::new(surface));
    assert!(store.soft_close_session(session));
    let (main_loop, _held) = record(&mut state);
    state.settle();
    assert!(state.core.library().held_session(session).is_some());
    for (_, task) in main_loop.take() {
        task(&mut state);
    }
    assert!(state.core.library().held_session(session).is_none());
    assert!(state.core.library().held_session(survivor).is_some());
    assert_eq!(probe.teardowns(), 1);
}

#[test]
fn a_pane_keystroke_clears_only_the_status_owned_by_that_pane() {
    use ngterm_core::model::{AgentIndicator, AgentStatus};
    let (mut state, window, session) = state_with_terminal();
    state.core.library_mut().store_mut(window).unwrap().set_agent_indicator(
        AgentIndicator {
            status: AgentStatus::Blocked,
            ..Default::default()
        },
        session,
    );
    state.pane_keystroke(session, "stale", StatusKeystroke::Other);
    assert_eq!(
        state
            .core
            .library()
            .held_session(session)
            .unwrap()
            .agent_indicator
            .status,
        AgentStatus::Blocked
    );
    state.pane_keystroke(session, "T", StatusKeystroke::Other);
    assert_eq!(
        state
            .core
            .library()
            .held_session(session)
            .unwrap()
            .agent_indicator
            .status,
        AgentStatus::Idle
    );
}

#[test]
fn a_live_reset_quit_is_deferred_until_the_reply_has_time_to_leave() {
    let (mut state, _, _) = state_with_terminal();
    let (main_loop, _held) = record(&mut state);
    state.schedule_live_quit(ngterm_core::LiveQuit { relaunch: false });
    assert!(!state.take_frontend_work().quit);
    for (delay, task) in main_loop.take() {
        assert_eq!(delay, Some(LIVE_QUIT_DELAY));
        task(&mut state);
    }
    assert!(state.take_frontend_work().quit);
    state.ask_live_reset(crate::live_runtime::ResetAlert::Refused(
        ngterm_core::ResetRefusal::NothingToReset,
    ));
    assert_eq!(
        state.take_frontend_work().live_reset,
        Some(crate::live_runtime::ResetAlert::Refused(
            ngterm_core::ResetRefusal::NothingToReset
        ))
    );
}

#[test]
fn a_custom_keybind_launches_with_its_typing_sessions_context() {
    let (mut state, window, session) = state_with_terminal();
    let launcher = Arc::new(ngterm_host::fakes::FakeProcessLauncher::new());
    state.core.set_process_host(ngterm_core::ProcessHost {
        launcher: launcher.clone(),
        poster: Arc::new(|_| {}),
        socket: "/private/control.sock".into(),
        cli_dir: None,
        path: None,
    });
    let command = ngterm_core::keymap::CustomCommand::new("mark", "echo {AGT_SESSION_ID}", "");
    state.keymap_fired(
        KeyOrigin::Window(window),
        Fired::Command(command),
        Some(TypingSurface {
            owner: ngterm_core::keymap::SurfaceOwner::Pane {
                session,
                token: "T".into(),
            },
            selection: None,
        }),
    );
    let attempts = launcher.launches();
    assert_eq!(attempts.len(), 1);
    let expected = ngterm_host::ShellCommand::new(&format!("echo {}", session.to_string().to_uppercase()));
    assert_eq!(attempts[0].program, expected.program);
    assert_eq!(attempts[0].args, expected.args);
    assert_eq!(attempts[0].raw_tail, expected.raw_tail);
    assert!(launcher.finish(
        ngterm_host::fakes::FakeProcessLauncher::FIRST_PID,
        ngterm_host::ProcessExit::default()
    ));
}

#[test]
fn hud_frame_reports_update_the_live_hud_but_quit_keeps_the_last_geometry() {
    let (mut state, _, session) = state_with_terminal();
    let dir = tempfile::tempdir().unwrap();
    state.core.set_hud_body_dir(dir.path().to_path_buf());
    let response = send(
        &mut state,
        Command::Session(SessionCmd::HudOpen),
        Some(session.to_string()),
        Args {
            message: Some("Building".into()),
            ..Args::default()
        },
    );
    assert!(response.ok, "{response:?}");
    let frames = ngterm_core::hud::HudPaneFrames {
        detail: Some(ngterm_core::hud::HudPaneFrame {
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 500.0,
        }),
        ..Default::default()
    };
    state.report_hud_frames(session, &frames);
    assert_eq!(state.core.library().held_session(session).unwrap().hud_frames, frames);
    state.report_hud_frames(session, &frames);
    state.save_on_quit();
    let changed = ngterm_core::hud::HudPaneFrames {
        detail: Some(ngterm_core::hud::HudPaneFrame {
            width: 100.0,
            height: 100.0,
            ..Default::default()
        }),
        ..Default::default()
    };
    state.report_hud_frames(session, &changed);
    assert_eq!(state.core.library().held_session(session).unwrap().hud_frames, frames);
}
