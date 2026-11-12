use std::fmt::Write as _;

use ngterm_core::paths::MapEnv;

use super::*;

fn env(root: &Path) -> MapEnv {
    MapEnv::new()
        .with("HOME", root)
        .with("USERPROFILE", root)
        .with("XDG_CONFIG_HOME", root.join("config"))
        .with("APPDATA", root.join("config"))
        .with("NGTERM_STATE_DIR", root.join("state"))
}

#[test]
fn defaults_and_identity_describe_the_serving_build() {
    assert_eq!(Options::default().grid, (80, 24));
    assert!(!Options::default().skip_agent_setup);
    assert_eq!(Options::default().opencode_version, None);
    assert_eq!(identity().version, env!("CARGO_PKG_VERSION"));
    assert!(bin_dir().unwrap().is_dir());
    // a test binary has no staged layout; a system ghostty package may still provide the resources
    assert!(resources().is_none_or(|dir| dir.join("shell-integration").is_dir()));
}

#[test]
fn load_bootstraps_and_reloads_the_library_without_forcing_saved_settings() {
    let dir = tempfile::tempdir().unwrap();
    let env = env(dir.path());
    let mut state = load_state(&env);
    assert!(!state.library.open_ids().is_empty());
    // nothing saved: the default, not this host's live
    assert_eq!(state.settings.effective_restore_mode(), RestoreMode::None);
    let ids = state.library.open_ids();
    assert!(state.persistence.as_mut().unwrap().save_all_checked(&state.library));
    let settings = ngterm_core::persist::AppSettings {
        restore_mode: Some(RestoreMode::Rerun),
        ..Default::default()
    };
    SettingsStore::new(paths::state_dir(&env)).save(&settings).unwrap();
    let restored = load_state(&env);
    assert_eq!(restored.library.open_ids(), ids);
    assert_eq!(restored.settings.effective_restore_mode(), RestoreMode::Rerun);
}

/// A subscriber keeping each event's fields as text, for what a call logs.
#[derive(Clone, Default)]
struct Logged(Arc<std::sync::Mutex<Vec<String>>>);

impl tracing::Subscriber for Logged {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }

    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        let mut text = String::new();
        event.record(&mut |field: &tracing::field::Field, value: &dyn std::fmt::Debug| {
            write!(text, "{}={value:?} ", field.name()).unwrap();
        });
        self.0.lock().unwrap().push(text);
    }

    fn enter(&self, _: &tracing::span::Id) {}

    fn exit(&self, _: &tracing::span::Id) {}
}

#[test]
fn config_reads_a_private_file_and_reports_invalid_values_without_losing_valid_ones() {
    let dir = tempfile::tempdir().unwrap();
    let env = env(dir.path());
    let file = paths::ghostty_settings_config_path(&env);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "font-size = 18\nunknown-coverage-setting = yes\n").unwrap();
    let logged = Logged::default();
    let config = tracing::subscriber::with_default(logged.clone(), || terminal_config(&env, None));
    assert!((config.font.size - 18.0).abs() < f32::EPSILON);
    let messages = logged.0.lock().unwrap().clone();
    assert!(
        messages
            .iter()
            .any(|message| message.contains("unknown-coverage-setting")),
        "{messages:?}"
    );
    assert!((terminal_config(&env, Some(dir.path())).font.size - 18.0).abs() < f32::EPSILON);
}

#[test]
fn a_missing_session_executable_refuses_live_with_the_probe_reason() {
    let dir = tempfile::tempdir().unwrap();
    let env = env(dir.path()).with("NGTERM_SESSION_PATH", dir.path().join("absent"));
    let runtime = Runtime::capture(&env);
    let reason = runtime.live_probe(&env).unavailable_reason().unwrap();
    assert_eq!(
        live_decision(&runtime, &env),
        Err(Refused(format!("live sessions are unavailable here: {reason}")))
    );
}

#[cfg(unix)]
#[test]
fn a_failed_reset_restart_removes_the_armed_marker() {
    let dir = tempfile::tempdir().unwrap();
    let binary = dir.path().join("restart-test");
    ngterm_host::fakes::copy_executable(&std::env::current_exe().unwrap(), &binary).unwrap();
    let mut command = std::process::Command::new(&binary);
    command
        .args(["--exact", "tests::reset_restart_child", "--nocapture"])
        .env("HEADLESS_RESET_CHILD", dir.path());
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("NGTERM_") {
            command.env_remove(name);
        }
    }
    let output = command.output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(!dir.path().join(ngterm_core::live::reset::MARKER_FILE_NAME).exists());
}

#[cfg(unix)]
#[test]
fn reset_restart_child() {
    let Some(root) = std::env::var_os("HEADLESS_RESET_CHILD").map(PathBuf::from) else {
        return;
    };
    let config = GhosttyConfig::default();
    let mut core = Core::new(
        CoreConfig {
            app: identity(),
            capabilities: Vec::new(),
        },
        CoreState::default(),
    );
    core.install_live_sessions(LiveInstall {
        host: Arc::new(ngterm_host::fakes::FakeSessionHost::new()),
        endpoint: ControlLiveEndpoint {
            attach_bridge: None,
            executable: "/bin/false".into(),
            socket_directory: root.join("live").to_string_lossy().into_owned(),
        },
        wrap: Inputs::default(),
        state_dir: root.clone(),
        outdated_before: None,
    });
    let main_loop: Arc<dyn MainLoop<HeadlessState>> = Arc::new(ThreadMainLoop::default());
    let panes = Panes::new(
        PaneEnvironment {
            socket: "unused".into(),
            program_version: "test".into(),
            host: Host::new(&config, None, None, None),
            config,
            grid: (80, 24),
            hostname: "box".into(),
        },
        Arc::new(|_, _| {}),
    );
    let state = HeadlessState::new(core, Arc::downgrade(&main_loop), panes);
    let markers = ngterm_core::live::reset::MarkerStore::new(&root);
    markers
        .write(&ngterm_core::live::reset::Marker::new(Vec::new(), SystemTime::now()))
        .unwrap();
    assert!(root.join(ngterm_core::live::reset::MARKER_FILE_NAME).is_file());
    std::fs::remove_file(std::env::current_exe().unwrap()).unwrap();
    restart_for_reset(&state, &Options::default());
    assert!(!root.join(ngterm_core::live::reset::MARKER_FILE_NAME).exists());
}

#[test]
fn process_completions_return_to_the_main_loop_and_settle_their_core_changes() {
    let main_loop = Arc::new(ThreadMainLoop::<HeadlessState>::default());
    let posting: Arc<dyn MainLoop<HeadlessState>> = main_loop.clone();
    let poster = Arc::downgrade(&posting);
    let process = process_host(&poster, "private-control.sock".into());
    assert_eq!(process.socket, "private-control.sock");
    assert_eq!(process.cli_dir, bin_dir());
    let config = GhosttyConfig::default();
    let core = Core::new(
        CoreConfig {
            app: identity(),
            capabilities: Vec::new(),
        },
        CoreState {
            library: ngterm_core::model::WindowLibrary::seeded("/"),
            ..CoreState::default()
        },
    );
    let panes = Panes::new(
        PaneEnvironment {
            socket: process.socket,
            program_version: "test".into(),
            host: Host::new(&config, None, None, None),
            config,
            grid: (80, 24),
            hostname: "box".into(),
        },
        Arc::new(|_, _| {}),
    );
    let mut state = HeadlessState::new(core, poster, panes);
    (process.poster)(Box::new(|core| {
        let response = core.dispatch(
            &serde_json::from_value(
                serde_json::json!({"cmd":"session.rename","target":"active","args":{"name":"finished"}}),
            )
            .unwrap(),
        );
        assert!(response.ok, "{response:?}");
    }));
    main_loop.run(&mut state, |_| main_loop.close());
    let response = state
        .core
        .dispatch(&ngterm_proto::Request::new(ngterm_proto::Command::Tree));
    assert!(serde_json::to_string(&response).unwrap().contains("finished"));
}

#[cfg(unix)]
#[test]
fn a_staged_session_build_is_installed_without_starting_an_absent_server() {
    let dir = tempfile::tempdir().unwrap();
    let binary = dir.path().join("bin/ngterm-session");
    std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
    // `false` is `/bin/false` on Linux, `/usr/bin/false` alone on macOS
    let inert = ["/bin/false", "/usr/bin/false"]
        .into_iter()
        .find(|tool| Path::new(tool).exists())
        .unwrap();
    ngterm_host::fakes::copy_executable(Path::new(inert), &binary).unwrap();
    ngterm_session::build_id::Staged::write_beside(&binary, "coverage-build").unwrap();
    let env = env(dir.path()).with("NGTERM_SESSION_PATH", &binary);
    let mut core = Core::new(
        CoreConfig {
            app: identity(),
            capabilities: Vec::new(),
        },
        CoreState::default(),
    );
    install_live(&mut core, &Runtime::capture(&env));
    let response = core.dispatch(&ngterm_proto::Request::new(ngterm_proto::Command::Live(
        ngterm_proto::command::LiveCmd::Tree,
    )));
    assert!(response.ok, "{response:?}");
    assert_eq!(
        response.result.unwrap()["remote"]["endpoint"]["executable"],
        binary.to_string_lossy().as_ref()
    );
    let directory = ngterm_core::live::socket_directory(&paths::state_dir(&env));
    assert!(Client::new(directory).api().unwrap().is_none());
}

#[test]
fn a_saved_live_mode_survives_bootstrap_without_a_home_directory() {
    let dir = tempfile::tempdir().unwrap();
    let env = MapEnv::new().with("NGTERM_STATE_DIR", dir.path());
    let settings = ngterm_core::persist::AppSettings {
        restore_mode: Some(RestoreMode::Live),
        ..Default::default()
    };
    SettingsStore::new(dir.path()).save(&settings).unwrap();
    let state = load_state(&env);
    assert_eq!(state.settings.effective_restore_mode(), RestoreMode::Live);
    let window = state.library.active_window_id().unwrap();
    assert_eq!(
        state
            .library
            .store(window)
            .unwrap()
            .sessions()
            .next()
            .unwrap()
            .effective_cwd(),
        "/"
    );
}
