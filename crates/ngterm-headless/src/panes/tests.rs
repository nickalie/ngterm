use ngterm_term::ExitStatus;

use super::*;

fn exit(held: bool) -> PaneEvent {
    PaneEvent::Exited(ExitStatus {
        code: 0,
        signal: None,
        held,
    })
}

const SHELL: Ran = Ran {
    command: false,
    wait: false,
};
const HELD_COMMAND: Ran = Ran {
    command: true,
    wait: true,
};

#[test]
fn titles_directories_and_notifications_reach_the_model() {
    assert_eq!(
        report(&PaneEvent::Title("vim".to_owned()), SHELL, "box"),
        Some(Report::Pane(PaneReport::Title("vim".to_owned())))
    );
    assert_eq!(
        report(&PaneEvent::Pwd("file://box/srv/app".to_owned()), SHELL, "box"),
        Some(Report::Pane(PaneReport::Pwd("/srv/app".to_owned())))
    );
    assert_eq!(
        report(&PaneEvent::Pwd("file://elsewhere/srv".to_owned()), SHELL, "box"),
        None,
        "another host's directory is not this pane's"
    );
    assert_eq!(
        report(
            &PaneEvent::Notification {
                title: "done".to_owned(),
                body: "build".to_owned()
            },
            SHELL,
            "box"
        ),
        Some(Report::Notification {
            title: "done".to_owned(),
            body: "build".to_owned()
        })
    );
    assert_eq!(report(&PaneEvent::Bell, SHELL, "box"), None);
}

#[test]
fn an_exit_closes_the_pane_unless_a_held_command_ended() {
    assert_eq!(report(&exit(false), SHELL, "box"), Some(Report::Exited));
    // no key will ever close a held exit here
    assert_eq!(report(&exit(true), SHELL, "box"), Some(Report::Exited));
    assert_eq!(report(&exit(true), HELD_COMMAND, "box"), Some(Report::Held));
    assert_eq!(report(&exit(false), HELD_COMMAND, "box"), Some(Report::Exited));
}

#[test]
fn a_model_without_windows_lays_out_no_pane() {
    let core = Core::new(
        ngterm_core::CoreConfig {
            app: crate::identity(),
            capabilities: Vec::new(),
        },
        ngterm_core::CoreState::default(),
    );
    assert!(layout(&core).is_empty());
}

/// A core over one window whose first session has a split, each pane holding a fake terminal; the session's id and
/// its primary's and split's tokens.
fn split_core() -> (Core, Uuid, String, String) {
    use ngterm_core::model::WindowLibrary;
    use ngterm_host::fakes::FakeSurface;

    let mut library = WindowLibrary::seeded("/home/u");
    let window = library.active_window_id().unwrap();
    let store = library.store_mut(window).unwrap();
    let session = store.sessions().next().unwrap().id();
    store.toggle_split(session, None);
    let found = store.session_mut(session).unwrap();
    let primary = pane_env::uuid_string(found.pane_identity);
    let split = pane_env::uuid_string(found.split_pane_identity.unwrap());
    drop(
        found
            .pane_slot_mut(StatusPane::Left)
            .replace(Box::new(FakeSurface::new(primary.clone()))),
    );
    drop(
        found
            .pane_slot_mut(StatusPane::Right)
            .replace(Box::new(FakeSurface::new(split.clone()))),
    );
    let core = Core::new(
        ngterm_core::CoreConfig {
            app: crate::identity(),
            capabilities: Vec::new(),
        },
        ngterm_core::CoreState {
            library,
            ..ngterm_core::CoreState::default()
        },
    );
    (core, session, primary, split)
}

/// What the panes start under, with the stock config and no login shell, resources or binaries.
fn environment() -> PaneEnvironment {
    let config = GhosttyConfig::default();
    PaneEnvironment {
        socket: "/tmp/a.sock".to_owned(),
        program_version: "1.2.3".to_owned(),
        host: Host::new(&config, None, None, None),
        config,
        grid: DEFAULT_GRID,
        hostname: "box".to_owned(),
    }
}

fn role(core: &Core, session: Uuid, token: &str) -> Option<StatusPane> {
    PaneSpec::of_token(core, session, token).map(|spec| spec.pane)
}

#[test]
fn a_token_keeps_its_role_until_the_model_moves_it() {
    let (core, session, primary, split) = split_core();
    assert_eq!(role(&core, session, &primary), Some(StatusPane::Left));
    assert_eq!(role(&core, session, &split), Some(StatusPane::Right));
    assert_eq!(role(&core, session, "no-such-token"), None);
    assert_eq!(role(&core, Uuid::new_v4(), &primary), None);
}

#[test]
fn a_swap_moves_each_token_to_the_other_role() {
    let (mut core, session, primary, split) = split_core();
    let window = core.library().active_window_id().unwrap();
    core.library_mut()
        .store_mut(window)
        .unwrap()
        .swap_panes(session)
        .unwrap();
    assert_eq!(role(&core, session, &split), Some(StatusPane::Left));
    assert_eq!(role(&core, session, &primary), Some(StatusPane::Right));
}

#[test]
fn a_promoted_split_survivor_is_the_primary() {
    let (mut core, session, primary, split) = split_core();
    let window = core.library().active_window_id().unwrap();
    core.library_mut()
        .store_mut(window)
        .unwrap()
        .close_primary_pane(session);
    let spec = PaneSpec::of_token(&core, session, &split).unwrap();
    assert_eq!(spec.pane, StatusPane::Left);
    assert!(
        spec.env(&environment())
            .contains(&("NGTERM_PANE".to_owned(), "left".to_owned())),
        "the survivor's shell is told its new role"
    );
    assert_eq!(role(&core, session, &primary), None);
}

#[test]
fn a_respawned_primary_keeps_its_token_but_needs_a_fresh_terminal() {
    let (mut core, session, primary, split) = split_core();
    let window = core.library().active_window_id().unwrap();
    let store = core.library_mut().store_mut(window).unwrap();
    store
        .add_companion(session, ngterm_core::model::CompanionOptions::default(), "/home/u")
        .unwrap();
    store.close_split_pane(session);
    let spec = PaneSpec::of_token(&core, session, &primary).unwrap();
    assert!(carries(&core, &spec), "the terminal still runs");
    let store = core.library_mut().store_mut(window).unwrap();
    assert_eq!(
        store.close_primary_pane(session),
        ngterm_core::model::PrimaryPaneExit::Respawned
    );
    let spec = PaneSpec::of_token(&core, session, &primary).expect("the pane is still laid out");
    assert_eq!(spec.pane, StatusPane::Left);
    assert!(!carries(&core, &spec), "the respawn tore its terminal down");
    assert_eq!(role(&core, session, &split), None);
}

#[test]
fn an_unstarted_surface_refuses_io_and_reports_no_terminal_geometry() {
    let panes = Panes::new(environment(), Arc::new(|_, _| {}));
    let cell = panes.cell("waiting");
    let mut surface = HeadlessSurface(cell);
    assert_eq!(surface.pane_token(), "waiting");
    assert!(!surface.is_realized());
    assert!(!surface.is_live_session());
    assert!(!surface.type_text("hello\n"));
    assert!(!surface.binding_action("text:hello"));
    assert_eq!(surface.read_text(TextScope::Screen), None);
    assert_eq!(surface.read_text(TextScope::All), None);
    assert_eq!(surface.read_selection(), None);
    assert_eq!(surface.cursor_column(), None);
    assert_eq!(surface.foreground_pid(), None);
    assert_eq!(surface.font_size(), None);
    assert_eq!(surface.cell_size(), None);
    surface.promote_to_primary();
    Box::new(surface).teardown();
}

#[cfg(unix)]
fn shell_environment() -> PaneEnvironment {
    let mut env = environment();
    env.host = Host::new(&env.config, Some("/bin/sh".into()), None, None);
    env
}

/// The inherited names a test's shell must not see: this checkout's tests may run inside a live ngterm's pane.
#[cfg(unix)]
fn isolation() -> Vec<String> {
    std::env::vars_os()
        .map(|(name, _)| name.to_string_lossy().into_owned())
        .filter(|name| {
            let name = name.to_ascii_uppercase();
            name.starts_with("NGTERM_") || name.starts_with("AGTERM_")
        })
        .collect()
}

#[cfg(unix)]
fn wait_surface(surface: &HeadlessSurface, contains: &str, wake: &std::sync::mpsc::Receiver<()>) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !surface.read_text(TextScope::All).unwrap_or_default().contains(contains) {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        assert!(
            !remaining.is_zero(),
            "no {contains:?}: {:?}",
            surface.read_text(TextScope::All)
        );
        match wake.recv_timeout(remaining.min(std::time::Duration::from_millis(20))) {
            Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(error) => panic!("terminal wake channel closed: {error}"),
        }
    }
}

#[cfg(unix)]
#[test]
fn a_real_terminal_accepts_keys_bindings_and_reads_the_programs_output() {
    let (sender, wake) = std::sync::mpsc::channel();
    let panes = Panes::new(
        shell_environment(),
        Arc::new(move |_, _| {
            let _ = sender.send(());
        }),
    );
    let cell = panes.cell("input");
    let config = panes.pane_config(Uuid::new_v4(), &cell);
    // Each wake must be available to the waiter; drain normally clears the coalescing flag.
    let queued = Arc::clone(&cell.drain_queued);
    let wake_sender = config.wake.clone().unwrap();
    let mut config = config;
    config.wake = Some(Arc::new(move || {
        queued.store(false, Ordering::Release);
        wake_sender();
    }));
    let launch = Launch {
        command: Some(
            "stty -echo; printf 'READY\\n'; while IFS= read -r line; do printf 'GOT:%s\\n' \"$line\"; done".into(),
        ),
        env_remove: isolation(),
        ..Launch::default()
    };
    *cell.pane.borrow_mut() = Some(Rc::new(
        Pane::launch(&launch, &panes.environment.host, &config).unwrap(),
    ));
    let mut surface = HeadlessSurface(Rc::clone(&cell));
    wait_surface(&surface, "READY", &wake);
    assert!(surface.is_realized());
    assert!(!surface.is_live_session());
    cell.live.set(true);
    assert!(surface.is_live_session());
    assert_eq!(surface.font_size(), Some(f64::from(panes.environment.config.font.size)));
    assert_eq!(surface.cell_size(), Some((8.0, 16.0)));
    assert!(surface.cursor_column().is_some());
    assert!(surface.foreground_pid().is_some());
    assert_eq!(surface.read_selection(), None);
    assert!(surface.type_text("alpha\nbeta\n"));
    wait_surface(&surface, "GOT:alpha", &wake);
    wait_surface(&surface, "GOT:beta", &wake);
    assert!(surface.binding_action(r"text:gamma\r"));
    wait_surface(&surface, "GOT:gamma", &wake);
    assert!(surface.binding_action("copy_to_clipboard"));
    assert!(surface.binding_action("invalid-coverage-action"));
    assert!(surface.read_text(TextScope::Screen).unwrap().contains("GOT:gamma"));
    Box::new(surface).teardown();
    assert!(cell.pane.borrow().is_none());
}

#[test]
fn retaining_a_pending_close_keeps_its_surface_until_the_close_is_final() {
    let (mut core, session, primary, _) = split_core();
    let mut panes = Panes::new(environment(), Arc::new(|_, _| {}));
    let spec = PaneSpec::of_token(&core, session, &primary).unwrap();
    panes.mount(&mut core, &spec);
    assert_eq!(panes.len(), 1);
    assert!(panes.drain(session, &primary).is_empty());
    assert!(panes.drain(session, "missing").is_empty());
    panes.retain(&[], &std::collections::HashSet::from([session]));
    assert_eq!(panes.len(), 1);
    panes.retain(&[], &std::collections::HashSet::new());
    assert!(panes.is_empty());
    panes.launch_seeded(&session, &primary, &LaunchSeed::default());
    panes.teardown_all();
    assert!(panes.is_empty());
}

#[cfg(unix)]
#[test]
fn a_replacement_uses_the_current_role_cwd_and_environment_and_reports_exit() {
    let (mut core, session, primary, _) = split_core();
    let (sender, wake) = std::sync::mpsc::channel();
    let mut panes = Panes::new(
        shell_environment(),
        Arc::new(move |_, _| {
            let _ = sender.send(());
        }),
    );
    let spec = PaneSpec::of_token(&core, session, &primary).unwrap();
    panes.mount(&mut core, &spec);
    let dir = tempfile::tempdir().unwrap();
    let replacement = PaneReplacement {
        session,
        token: primary.clone(),
        command: "printf '\\033]2;replaced\\007'; printf '%s:%s\\n' \"$NGTERM_PANE\" \"$EXTRA\"; pwd".into(),
        attach: None,
        env: vec![
            ("NGTERM_PANE".into(), "overridden".into()),
            ("EXTRA".into(), "kept".into()),
        ],
        cwd: Some(dir.path().to_string_lossy().into_owned()),
        remote: true,
    };
    assert!(panes.replace_one(&mut core, &replacement));
    let cell = panes.cells.values().next().unwrap().clone();
    let surface = HeadlessSurface(cell);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut reports = Vec::new();
    loop {
        reports.extend(panes.drain(session, &primary));
        if reports.contains(&Report::Held) {
            break;
        }
        wake.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
            .unwrap();
    }
    assert!(reports.contains(&Report::Pane(PaneReport::Title("replaced".into()))));
    let output = surface.read_text(TextScope::All).unwrap();
    assert!(output.contains("overridden:kept"), "{output}");
    assert!(output.contains(dir.path().to_str().unwrap()), "{output}");
    assert!(!panes.replace_one(
        &mut core,
        &PaneReplacement {
            token: "missing".into(),
            ..replacement
        }
    ));
    panes.teardown_all();
    assert!(!surface.is_realized());
}

#[cfg(unix)]
#[test]
fn sync_mounts_all_split_panes_once_and_follows_the_models_roles_and_closes() {
    let (mut core, session, primary, split) = split_core();
    let window = core.library().active_window_id().unwrap();
    let dir = tempfile::tempdir().unwrap();
    {
        let found = core
            .library_mut()
            .store_mut(window)
            .unwrap()
            .session_mut(session)
            .unwrap();
        found.current_cwd = Some(dir.path().to_string_lossy().into_owned());
        found.split_cwd = found.current_cwd.clone();
        found.launch.command = Some("printf 'PRIMARY\\n'".into());
        found.launch.wait = true;
        found.split_launch.command = Some("printf 'SPLIT\\n'".into());
        found.split_launch.wait = true;
    }
    let (sender, wake) = std::sync::mpsc::channel();
    let mut panes = Panes::new(
        shell_environment(),
        Arc::new(move |_, _| {
            let _ = sender.send(());
        }),
    );
    assert!(!panes.sync(&mut core));
    assert_eq!(panes.len(), 2);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut reports = Vec::new();
    loop {
        reports.extend(panes.drain(session, &primary));
        reports.extend(panes.drain(session, &split));
        if reports.iter().filter(|report| **report == Report::Held).count() == 2 {
            break;
        }
        wake.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
            .unwrap();
    }
    assert!(!panes.sync(&mut core));
    core.library_mut()
        .store_mut(window)
        .unwrap()
        .swap_panes(session)
        .unwrap();
    assert!(!panes.sync(&mut core));
    let spec = panes.specs.values().find(|spec| spec.token == primary).unwrap();
    assert_eq!(spec.pane, StatusPane::Right);
    let surfaces: Vec<_> = panes.cells.values().map(|cell| HeadlessSurface(cell.clone())).collect();
    core.library_mut().store_mut(window).unwrap().close_session(session);
    assert!(!panes.sync(&mut core));
    assert!(panes.is_empty());
    assert!(surfaces.iter().all(|surface| !surface.is_realized()));
}

#[cfg(unix)]
#[test]
fn a_programs_clipboard_read_is_denied_so_its_output_keeps_flowing() {
    clipboard_unblocks(r"\033]52;c;?\007");
}

#[cfg(unix)]
#[test]
fn a_programs_clipboard_write_is_unsupported_so_its_output_keeps_flowing() {
    clipboard_unblocks(r"\033]5522;type=write\007\033]5522;type=wdata\007");
}

#[cfg(unix)]
fn clipboard_unblocks(sequence: &str) {
    let (mut core, session, primary, _) = split_core();
    let (sender, wake) = std::sync::mpsc::channel();
    let mut panes = Panes::new(
        shell_environment(),
        Arc::new(move |_, _| {
            let _ = sender.send(());
        }),
    );
    let mut spec = PaneSpec::of_token(&core, session, &primary).unwrap();
    spec.cwd = "/".into();
    panes.mount(&mut core, &spec);
    // the program stays until told to go: its exit would deny the read by itself
    panes.launch_seeded(
        &session,
        &primary,
        &LaunchSeed {
            command: Some(format!("stty -echo; printf '{sequence}after-clipboard\\n'; read -r _")),
            wait_after_command: true,
            ..LaunchSeed::default()
        },
    );
    let mut surface = HeadlessSurface(panes.cells.values().next().unwrap().clone());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut reports = Vec::new();
    while !surface
        .read_text(TextScope::All)
        .unwrap_or_default()
        .contains("after-clipboard")
    {
        reports.extend(panes.drain(session, &primary));
        wake.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
            .expect("the output after the clipboard read never came");
    }
    assert!(
        !reports.contains(&Report::Held),
        "the program ended before its read was answered"
    );
    assert!(surface.type_text("\n"));
    loop {
        reports.extend(panes.drain(session, &primary));
        if reports.contains(&Report::Held) {
            break;
        }
        wake.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
            .unwrap();
    }
    panes.teardown_all();
}
