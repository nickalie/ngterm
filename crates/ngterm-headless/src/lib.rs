//! `ngterm-headless`: ngterm with no window, for a machine without a display (a server reached over ssh). It runs
//! what the GPUI frontend runs minus the drawing: the core over the state directory, its control socket (so
//! `ngtermctl` and the agents' hooks work as against the app), the live-session server, and a terminal per session
//! pane that nobody draws ([`panes`]). Another machine's ngterm attaches its sessions through File ▸ Attach Remote,
//! exactly as it attaches a desktop instance's: `ngtermctl live tree` over ssh lists them, each pane an ssh into its
//! live session, the presentation (status, layout, HUD, notifications) streamed over `ngtermctl live present`, and
//! asks, program overlays and pages shown on the viewer.
//!
//! - Live sessions always: the launch runs in restore mode `live` whatever the settings hold, and refuses to start
//!   when the live probe rejects this host (no `ngterm-session` beside the binary, an unsupported login shell), since
//!   without daemons there is nothing to attach.
//! - Every session's panes run, not only a shown one's (there is none): a session is attachable once every pane of it
//!   has a running daemon.
//! - One instance per state directory, as the app: the state lock refuses a second one, and an app instance on the
//!   same state directory too.
//! - It can view as well: `ngtermctl live attach` on it attaches another machine's session as a row of its own, its
//!   presentation bridged as the GPUI frontend bridges it (no presenter draws an ask or an overlay here, though).
//! - The quit (SIGTERM, SIGINT, SIGHUP; the console's Ctrl events on Windows) saves the state and detaches every
//!   terminal; the sessions live on with the session server, and the next start attaches them again.
//!
//! Module map: [`main_loop`] (the loop of a process with no toolkit), [`state`] (the requests, the viewers' streams,
//! the timers), [`panes`] (the terminals), [`signals`].

pub mod main_loop;
pub mod panes;
pub mod signals;
pub mod state;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use ngterm_core::ghostty_config::{self, GhosttyConfig};
use ngterm_core::live::build_record::{launch_cutoff, record_handoff};
use ngterm_core::live::support::Inputs;
use ngterm_core::paths::{self, Env, ProcessEnv};
use ngterm_core::persist::{LibraryPersistence, RestoreMode, SettingsStore};
use ngterm_core::restore::mode::RestoreLaunchDecision;
use ngterm_core::{Core, CoreConfig, CoreState, LiveInstall};
use ngterm_host::MainLoop;
use ngterm_proto::{AppIdentity, ControlLiveEndpoint};
use ngterm_server::ControlServer;
use ngterm_session::client::Client;
use ngterm_session::host::ServerHost;
use ngterm_session::launch::Runtime;
use ngterm_session::server::spawn::{HandoffAttempt, StartOptions};
use ngterm_term::Host;

use crate::main_loop::ThreadMainLoop;
use crate::panes::{DrainPoster, PaneEnvironment, Panes};
use crate::state::{HeadlessState, Poster, post};

/// What a start is given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// The grid the terminals start at, columns and rows.
    pub grid: (u16, u16),
}

impl Default for Options {
    fn default() -> Self {
        Self {
            grid: panes::DEFAULT_GRID,
        }
    }
}

/// The serving build, projected into `version`, `tree.app` and every shell's `TERM_PROGRAM_VERSION`.
#[must_use]
pub fn identity() -> AppIdentity {
    AppIdentity::from_recorded(env!("CARGO_PKG_VERSION"), option_env!("NGTERM_BUILD_COMMIT"))
}

/// Why a start was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused(pub String);

/// The restore decision of a headless launch: `live`, refused with the probe's reason when this host cannot run it.
///
/// # Errors
///
/// The live probe's reason.
pub fn live_decision(env: &dyn Env) -> Result<RestoreLaunchDecision, Refused> {
    let probe = Runtime::capture(env).live_probe(env);
    let decision = RestoreLaunchDecision::new(RestoreMode::Live, probe.unavailable_reason());
    match decision.live_unavailable_reason.clone() {
        Some(reason) if decision.active != RestoreMode::Live => {
            Err(Refused(format!("live sessions are unavailable here: {reason}")))
        }
        _ => Ok(decision),
    }
}

/// The model as the last run left it, with the settings in restore mode `live`.
fn load_state(env: &dyn Env) -> CoreState {
    let state_dir = paths::state_dir(env);
    let home = paths::home_dir(env).map_or_else(|| "/".to_owned(), |home| home.to_string_lossy().into_owned());
    let mut settings = SettingsStore::new(&state_dir).load();
    if settings.effective_restore_mode() != RestoreMode::Live {
        tracing::info!(
            was = settings.effective_restore_mode().wire_name(),
            "restore mode set to live: a headless instance serves live sessions"
        );
        settings.restore_mode = Some(RestoreMode::Live);
    }
    let mut persistence = LibraryPersistence::new(&state_dir);
    let library = persistence.bootstrap(&home);
    CoreState {
        library,
        settings,
        persistence: Some(persistence),
    }
}

/// libghostty's resources (themes, shell integration, terminfo) of the staged layout.
fn resources() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok();
    ngterm_term::resolve_resources(&ngterm_term::resource_candidates(exe.as_deref()), Path::exists)
}

/// The directory of this binary, where `ngtermctl` and `ngterm-session` sit.
fn bin_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
}

/// The ghostty config the terminals start with, on the light side (no desktop has an appearance here).
fn terminal_config(env: &dyn Env, resources: Option<&Path>) -> GhosttyConfig {
    let loaded = ghostty_config::load_app_config(env, resources.is_some(), None);
    let themes = resources.map(|dir| dir.join("themes"));
    let finalized = loaded.finalize(false, &ghostty_config::theme_dirs(env, themes.as_deref()));
    for diagnostic in &finalized.diagnostics {
        tracing::warn!(%diagnostic, "ghostty config");
    }
    finalized.config
}

/// The live-session host (the GPUI frontend's `session_host::install`): a server of an older build hands its sessions
/// over to this one first, then the core takes the host, and the launch reap runs.
fn install_live(core: &mut Core, env: &dyn Env) {
    let runtime = Runtime::capture(env);
    let directory = ngterm_core::live::socket_directory(&runtime.state_dir);
    let endpoint = ControlLiveEndpoint {
        executable: runtime.executable.to_string_lossy().into_owned(),
        socket_directory: directory.clone(),
    };
    let options = StartOptions::new(&runtime.executable);
    let bundled = options.build.clone();
    if let Some(bundled) = bundled.as_deref()
        && ngterm_core::live::support::is_executable_file(&runtime.executable)
    {
        match Client::new(&directory).update(&options) {
            Ok(None) => {}
            Ok(Some(HandoffAttempt::Committed { from })) => {
                tracing::info!(%from, to = bundled, "live sessions handed over to this build");
                record_handoff(Some(bundled), &runtime.state_dir, SystemTime::now());
            }
            Ok(Some(HandoffAttempt::Failed { from, reason })) => {
                tracing::warn!(%from, to = bundled, %reason, "the live sessions stay with the older build");
            }
            Err(error) => tracing::warn!(%error, "the live session server could not be updated"),
        }
    }
    let outdated_before = launch_cutoff(bundled.as_deref(), &runtime.state_dir, SystemTime::now());
    core.install_live_sessions(LiveInstall {
        host: Arc::new(ServerHost::new(runtime.executable.clone(), directory)),
        endpoint,
        wrap: Inputs {
            executable: runtime.executable.clone(),
            login_shell: runtime.login_shell.clone(),
            resources_dir: runtime.resources_dir.clone(),
            own_resources_dir: runtime.own_resources_dir.clone(),
            state_dir: runtime.state_dir.clone(),
            inherited: runtime.inherited.clone(),
            ..Inputs::default()
        },
        state_dir: runtime.state_dir.clone(),
        outdated_before,
    });
    core.run_live_launch();
}

/// The process seam of the hooks and custom commands: each exit comes back as a main-loop task that settles.
fn process_host(poster: &Poster, socket: String) -> ngterm_core::ProcessHost {
    let posting = poster.clone();
    ngterm_core::ProcessHost {
        launcher: Arc::new(ngterm_app::hook_runner::SystemLauncher),
        poster: Arc::new(move |task: ngterm_core::CoreTask| {
            post(&posting, move |state: &mut HeadlessState| {
                task(&mut state.core);
                state.settle();
            });
        }),
        socket,
        cli_dir: bin_dir(),
        path: std::env::var("PATH").ok(),
    }
}

/// The core over the state the last run left, with what it reaches the host through (the GPUI frontend's
/// `launch_core` and `install_launch_hosts`, less the window's hosts).
fn launch_core(decision: RestoreLaunchDecision, resources: Option<&Path>, poster: &Poster, socket: String) -> Core {
    let env = &ProcessEnv;
    let mut core = Core::new(
        CoreConfig {
            app: identity(),
            capabilities: Vec::new(),
        },
        load_state(env),
    );
    core.set_theme_paths(
        resources.map(|dir| dir.join("themes")),
        Some(paths::ghostty_settings_config_path(env)),
    );
    core.write_settings_config();
    core.set_installed_source(
        std::env::current_exe()
            .ok()
            .and_then(|executable| ngterm_core::identity::InstalledSource::of_executable(&executable)),
    );
    core.set_config_locator(paths::ConfigLocator::capture(env));
    core.set_restore_launch(decision);
    core.load_restore_denylist_at_launch();
    core.set_process_table(
        Box::new(ngterm_app::process_table::ProcTable::new()),
        ngterm_app::restore_launch::shell_basename(env),
    );
    core.set_process_host(process_host(poster, socket));
    install_live(&mut core, env);
    core.load_keymap_at_launch();
    core.load_hooks_at_launch();
    core.load_remotes_at_launch();
    core
}

/// Runs until the system asks it to end; the exit status.
#[must_use]
pub fn run(options: &Options) -> i32 {
    let env = &ProcessEnv;
    let decision = match live_decision(env) {
        Ok(decision) => decision,
        Err(Refused(reason)) => {
            tracing::error!("{reason}");
            eprintln!("ngterm-headless: {reason}");
            return 1;
        }
    };
    let main_loop = Arc::new(ThreadMainLoop::<HeadlessState>::default());
    let posting: Arc<dyn MainLoop<HeadlessState>> = Arc::clone(&main_loop) as _;
    let poster: Poster = Arc::downgrade(&posting);
    if let Err(error) = signals::quit_on_termination(poster.clone()) {
        tracing::error!(%error, "the system's quit is not handled: it ends the process without the quit save");
    }
    let mut server = ControlServer::new(paths::app_socket_path(env));
    let socket = server.advertised_path().to_string_lossy().into_owned();
    let resources = resources();
    let core = launch_core(decision, resources.as_deref(), &poster, socket.clone());
    let config = terminal_config(env, resources.as_deref());
    let host = Host::new(&config, Runtime::capture(env).login_shell, resources, bin_dir());
    let environment = PaneEnvironment {
        socket,
        program_version: identity().version,
        host,
        config,
        grid: options.grid,
        hostname: ngterm_term::pwd::local_hostname().unwrap_or_default(),
    };
    let draining = poster.clone();
    let drain: DrainPoster = Arc::new(move |session, token: String| {
        post(&draining, move |state| state.drain_pane(session, &token));
    });
    let mut state = HeadlessState::new(core, poster.clone(), Panes::new(environment, drain));
    state.settle();
    state.land();
    let started = server.start(Arc::clone(&posting), |state: &mut HeadlessState, request| {
        state.handle_request(&request)
    });
    match &started {
        Ok(()) => tracing::info!(socket = %server.socket_path().display(), "serving"),
        Err(error) => tracing::error!(%error, "the control server is not serving"),
    }
    let ready = started.is_ok();
    post(&poster, move |state| {
        state.core.emit_app_launched(ready);
        state.settle();
    });
    let closing = Arc::clone(&main_loop);
    main_loop.run(&mut state, |state| {
        state.land();
        if state.quit_requested() {
            state.save_on_quit();
            tracing::info!("quit: the state is saved");
            closing.close();
        }
    });
    server.stop();
    state.teardown();
    tracing::info!("quit: done");
    i32::from(!ready)
}
