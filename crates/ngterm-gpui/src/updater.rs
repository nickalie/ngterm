//! The app-wide release updater: one worker at a time, hourly automatic checks, explicit restart.
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Entity, Global, ParentElement as _, Styled as _, Task, WeakEntity, Window,
    px,
};
use ngterm_core::chrome::ChromeColors;
use ngterm_core::paths::{self, ProcessEnv};
use ngterm_update::{Installation, Prepared, RELEASES_URL, Source, Trust};

use crate::chrome::button::Button;
use crate::modal::{AlertButton, AlertSpec, open_alert};
use crate::state::{AppState, Poster, post};

const INTERVAL: Duration = Duration::from_hours(1);

#[derive(Clone, Debug, PartialEq, Eq)]
enum Status {
    Idle,
    Checking,
    Downloading(String),
    Current,
    Ready(String),
    Failed(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Origin {
    Automatic,
    Manual,
}

/// Where the updater finds releases and the signing key their manifests must verify against: GitHub and the
/// committed key in the app, a local release only in tests.
#[derive(Clone)]
struct Releases {
    source: Source,
    trust: fn() -> Result<Trust, ngterm_update::Error>,
}

struct Updater {
    state: WeakEntity<AppState>,
    notice: Option<String>,
    installation: Result<Installation, String>,
    releases: Releases,
    /// The process's state directory: protected from the installer and home of the update error file.
    state_dir: PathBuf,
    status: Status,
    prepared: Option<Prepared>,
    job: Option<Task<()>>,
    origin: Origin,
    cancelled: Option<Arc<AtomicBool>>,
    polling: Option<Task<()>>,
    restart: bool,
    poster: Poster,
}
impl Global for Updater {}

impl Updater {
    fn display_status(&self) -> Status {
        match &self.status {
            Status::Ready(_) | Status::Current | Status::Failed(_) => self.status.clone(),
            _ => self.notice.clone().map_or_else(|| self.status.clone(), Status::Failed),
        }
    }
}

fn error_file(state_dir: &Path) -> PathBuf {
    state_dir.join("update-error")
}

/// Installs the updater after the app has loaded its settings.
pub fn init(state: &Entity<AppState>, poster: Poster, cx: &mut App) {
    let releases = Releases {
        source: Source::github(),
        trust: Trust::release,
    };
    let installation = Installation::detect().map_err(|error| error.to_string());
    start(state, poster, installation, releases, paths::state_dir(&ProcessEnv), cx);
}

/// Installs the updater for `installation`, checking `releases`, with the update error file under `state_dir`.
fn start(
    state: &Entity<AppState>,
    poster: Poster,
    installation: Result<Installation, String>,
    releases: Releases,
    state_dir: PathBuf,
    cx: &mut App,
) {
    let notice = std::fs::read_to_string(error_file(&state_dir)).ok();
    let automatic = state.read(cx).core.settings().auto_update.unwrap_or(true);
    if let Ok(installation) = &installation {
        let installation = installation.clone();
        cx.background_spawn(async move {
            let _ = installation.cleanup_stale();
        })
        .detach();
    }
    cx.set_global(Updater {
        state: state.downgrade(),
        notice,
        installation,
        releases,
        state_dir,
        status: Status::Idle,
        prepared: None,
        job: None,
        origin: Origin::Automatic,
        cancelled: None,
        polling: None,
        restart: false,
        poster,
    });
    set_automatic(automatic, cx);
}

/// A settings edit starts or stops polling immediately. A manual check is allowed with automatic checks disabled.
pub fn set_automatic(enabled: bool, cx: &mut App) {
    let Some(updater) = cx.try_global::<Updater>() else {
        return;
    };
    if !enabled {
        let updater = cx.global_mut::<Updater>();
        updater.polling.take();
        if updater.origin == Origin::Automatic {
            if let Some(cancelled) = updater.cancelled.take() {
                cancelled.store(true, Ordering::Release);
            }
            updater.job.take();
            updater.prepared.take();
            if updater.status != Status::Idle {
                updater.status = Status::Idle;
                cx.refresh_windows();
            }
        }
        return;
    }
    if updater.polling.is_some() || updater.installation.is_err() {
        return;
    }
    let task = cx.spawn(async move |cx| {
        loop {
            cx.update(|cx| start_check(None, cx));
            cx.background_executor().timer(INTERVAL).await;
        }
    });
    cx.global_mut::<Updater>().polling = Some(task);
}

/// Help > Check for Updates. A ready update opens its restart confirmation.
pub fn check(window: &mut Window, cx: &mut App) {
    let Some(updater) = cx.try_global::<Updater>() else {
        return;
    };
    if let Err(reason) = current_installation(cx) {
        open_alert(window, AlertSpec::info("Updates", reason), |_, _, _| {}, cx);
    } else if matches!(
        updater.status,
        Status::Ready(_) | Status::Checking | Status::Downloading(_)
    ) {
        show_status(window, cx);
    } else {
        start_check(Some(window.window_handle()), cx);
    }
}

fn start_check(manual: Option<AnyWindowHandle>, cx: &mut App) {
    let updater = cx.global::<Updater>();
    if updater.job.is_some() || updater.prepared.is_some() || updater.restart {
        return;
    }
    let Ok(installation) = current_installation(cx) else {
        return;
    };
    cx.global_mut::<Updater>().status = Status::Checking;
    let cancelled = Arc::new(AtomicBool::new(false));
    let updater = cx.global_mut::<Updater>();
    updater.origin = if manual.is_some() {
        Origin::Manual
    } else {
        Origin::Automatic
    };
    updater.cancelled = Some(Arc::clone(&cancelled));
    cx.refresh_windows();
    let Releases { source, trust } = cx.global::<Updater>().releases.clone();
    let task = cx.spawn(async move |cx| {
        let checked = cx
            .background_spawn({
                let (installation, source) = (installation.clone(), source.clone());
                async move {
                    let _ = installation.cleanup_stale();
                    let trust = trust()?;
                    source
                        .check(&installation, env!("CARGO_PKG_VERSION"), &trust)
                        .map(|release| release.map(|release| (release, trust)))
                }
            })
            .await;
        let result = match checked {
            Ok(Some((release, trust))) => {
                if cancelled.load(Ordering::Acquire) {
                    return;
                }
                let version = release.version.to_string();
                cx.update(|cx| {
                    cx.global_mut::<Updater>().status = Status::Downloading(version.clone());
                    cx.refresh_windows();
                });
                cx.background_spawn(async move {
                    source
                        .prepare(&installation, &release, &trust, |_, _| {
                            !cancelled.load(Ordering::Acquire)
                        })
                        .map(|prepared| (version, prepared))
                })
                .await
                .map(Some)
            }
            Ok(None) => Ok(None),
            Err(error) => Err(error),
        };
        cx.update(|cx| {
            let updater = cx.global_mut::<Updater>();
            updater.job.take();
            updater.cancelled.take();
            if result.is_ok() {
                updater.notice.take();
                let _ = std::fs::remove_file(error_file(&updater.state_dir));
            }
            match result {
                Ok(Some((version, prepared))) => {
                    updater.prepared = Some(prepared);
                    updater.status = Status::Ready(version);
                }
                Ok(None) => updater.status = Status::Current,
                Err(error) => {
                    tracing::warn!(%error, "release update failed");
                    updater.status = Status::Failed(error.to_string());
                }
            }
            cx.refresh_windows();
            if let Some(window) = manual {
                let _ = window.update(cx, |_, window, cx| show_status(window, cx));
            }
        });
    });
    cx.global_mut::<Updater>().job = Some(task);
}

fn show_status(window: &mut Window, cx: &mut App) {
    let status = cx.global::<Updater>().display_status();
    let ready = matches!(status, Status::Ready(_));
    let failed = matches!(status, Status::Failed(_));
    let alert = match status {
        Status::Ready(version) => AlertSpec {
            buttons: vec![
                AlertButton::cancel("Later").as_default(),
                AlertButton::plain("Restart to Update").suggested(),
                AlertButton::plain("Release Notes"),
            ],
            ..AlertSpec::info(
                "Update ready",
                format!(
                    "ngterm {version} has been downloaded and verified. Restart to install it. Live sessions reconnect after restarting; other shells close and restore according to your session settings."
                ),
            )
        },
        Status::Failed(reason) => AlertSpec {
            buttons: vec![
                AlertButton::cancel("OK").as_default(),
                AlertButton::plain("Release Downloads"),
            ],
            ..AlertSpec::info("Update needs attention", reason)
        },
        Status::Checking => AlertSpec::info("Updates", "Checking for updates…"),
        Status::Downloading(version) => {
            AlertSpec::info("Updates", format!("Downloading and verifying ngterm {version}…"))
        }
        Status::Current => AlertSpec::info(
            "Up to date",
            format!(
                "No newer stable release is available for this installation. Running ngterm {}.",
                env!("CARGO_PKG_VERSION")
            ),
        ),
        Status::Idle => AlertSpec::info("Updates", "Choose Check for Updates from the Help menu."),
    };
    open_alert(
        window,
        alert,
        move |answer, window, cx| {
            if failed {
                let updater = cx.global_mut::<Updater>();
                let _ = std::fs::remove_file(error_file(&updater.state_dir));
                updater.notice.take();
                if matches!(updater.status, Status::Failed(_)) {
                    updater.status = Status::Idle;
                }
                cx.refresh_windows();
            }
            if ready && answer.button == 1 {
                restart(window, cx);
            } else if (ready && answer.button == 2) || (failed && answer.button == 1) {
                cx.open_url(RELEASES_URL);
            }
        },
        cx,
    );
}

fn restart(window: &mut Window, cx: &mut App) {
    let Some(prepared) = cx.global_mut::<Updater>().prepared.take() else {
        return;
    };
    match install(prepared, true, cx) {
        Ok(()) => {
            let updater = cx.global_mut::<Updater>();
            updater.restart = true;
            post(&updater.poster, AppState::request_quit);
        }
        Err(error) => {
            cx.global_mut::<Updater>().status = Status::Failed(error);
            cx.refresh_windows();
            show_status(window, cx);
        }
    }
}

/// Persistent titlebar feedback while a download is in progress or waiting for a restart.
pub fn button(colors: ChromeColors, cx: &App) -> Option<impl gpui_kit::IntoElement> {
    let updater = cx.try_global::<Updater>()?;
    let label = match updater.display_status() {
        Status::Downloading(_) => "Downloading update…",
        Status::Ready(_) => "Restart to Update",
        Status::Failed(_) => "Update needs attention",
        _ => return None,
    };
    Some(
        Button::new("update-status")
            .wash(crate::chrome::color(colors.foreground, 1.0))
            .rounded(px(5.))
            .px(px(8.))
            .h(px(24.))
            .text_size(px(12.))
            .on_click(|_, window, cx| show_status(window, cx))
            .child(label),
    )
}

/// Called once after the quit save. Installs a ready update without reopening on ordinary quits. A live reset has
/// its own relauncher, so that quit discards the update to avoid racing it.
pub fn on_quit(live_reset: bool, cx: &mut App) {
    if cx.try_global::<Updater>().is_none() {
        return;
    }
    let updater = cx.global_mut::<Updater>();
    updater.polling.take();
    if let Some(cancelled) = updater.cancelled.take() {
        cancelled.store(true, Ordering::Release);
    }
    updater.job.take();
    let prepared = updater.prepared.take();
    if !live_reset
        && let Some(prepared) = prepared
        && let Err(error) = install(prepared, false, cx)
    {
        tracing::error!(%error, "could not start the update installer on quit");
        let _ = std::fs::write(error_file(&cx.global::<Updater>().state_dir), error);
    }
}

fn protected_directories(cx: &App) -> Result<[PathBuf; 2], String> {
    let updater = cx.global::<Updater>();
    let state = updater.state.upgrade().ok_or("Application state is unavailable.")?;
    Ok([updater.state_dir.clone(), state.read(cx).core.config_dir().clone()])
}

fn current_installation(cx: &App) -> Result<Installation, String> {
    let installation = cx.global::<Updater>().installation.clone()?;
    installation
        .validate_paths(&protected_directories(cx)?)
        .map_err(|error| error.to_string())?;
    Ok(installation)
}

// Both restart and ordinary quit obtain fresh paths, after the latest Settings edits and before spawning a helper.
fn install(prepared: Prepared, reopen: bool, cx: &App) -> Result<(), String> {
    let paths = protected_directories(cx)?;
    prepared
        .launch(&error_file(&cx.global::<Updater>().state_dir), reopen, &paths)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {

    #[gpui_kit::test]
    fn opting_out_cancels_automatic_work_and_keeps_manual_work(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            let state = cx.new(|_| {
                AppState::new(Core::new(
                    CoreConfig {
                        app: AppIdentity::from_recorded("0.1.0", None),
                        capabilities: Vec::new(),
                    },
                    CoreState::default(),
                ))
            });
            let posting: Arc<dyn ngterm_host::MainLoop<AppState>> =
                Arc::new(crate::main_loop::GpuiMainLoop::new(cx.background_executor().clone()));
            for origin in [Origin::Automatic, Origin::Manual] {
                for status in [
                    Status::Idle,
                    Status::Checking,
                    Status::Downloading("0.2.0".into()),
                    Status::Ready("0.2.0".into()),
                    Status::Current,
                    Status::Failed("offline".into()),
                ] {
                    let cancelled = Arc::new(AtomicBool::new(false));
                    let running = matches!(status, Status::Checking | Status::Downloading(_));
                    let job = running.then(|| cx.spawn(async |_| futures_lite::future::pending::<()>().await));
                    cx.set_global(Updater {
                        state: state.downgrade(),
                        notice: None,
                        installation: Err("source build".into()),
                        releases: offline(),
                        state_dir: dir.path().to_owned(),
                        status: status.clone(),
                        prepared: None,
                        job,
                        origin,
                        cancelled: Some(Arc::clone(&cancelled)),
                        polling: None,
                        restart: false,
                        poster: Arc::downgrade(&posting),
                    });
                    set_automatic(false, cx);
                    let automatic = origin == Origin::Automatic;
                    assert_eq!(cancelled.load(Ordering::Acquire), automatic);
                    assert_eq!(cx.global::<Updater>().job.is_some(), running && !automatic);
                    assert_eq!(
                        cx.global::<Updater>().status,
                        if automatic { Status::Idle } else { status }
                    );
                    cx.global_mut::<Updater>().job.take();
                }
            }
        });
    }

    use super::*;
    use gpui_kit::TestAppContext;
    use ngterm_core::paths::{ConfigLocator, MapEnv};
    use ngterm_core::persist::LibraryPersistence;
    use ngterm_core::settings_model::SettingsChange;
    use ngterm_core::{Core, CoreConfig, CoreState};
    use ngterm_proto::AppIdentity;
    use std::sync::Arc;

    #[gpui_kit::test]
    fn installation_reads_the_latest_configuration_and_not_the_startup_snapshot(cx: &mut TestAppContext) {
        let fixture = tempfile::tempdir().unwrap();
        let mut core = Core::new(
            CoreConfig {
                app: AppIdentity::from_recorded("0.1.0", None),
                capabilities: Vec::new(),
            },
            CoreState {
                persistence: Some(LibraryPersistence::new(fixture.path())),
                ..CoreState::default()
            },
        );
        core.set_config_locator(ConfigLocator::capture(
            &MapEnv::new().with("NGTERM_STATE_DIR", fixture.path().as_os_str()),
        ));
        cx.update(|cx| {
            let state = cx.new(|_| AppState::new(core));
            let posting: Arc<dyn ngterm_host::MainLoop<AppState>> =
                Arc::new(crate::main_loop::GpuiMainLoop::new(cx.background_executor().clone()));
            cx.set_global(Updater {
                state: state.downgrade(),
                notice: Some("previous installer failure".into()),
                installation: Err("source build".into()),
                releases: offline(),
                state_dir: fixture.path().to_owned(),
                status: Status::Idle,
                prepared: None,
                job: None,
                origin: Origin::Automatic,
                cancelled: None,
                polling: None,
                restart: false,
                poster: Arc::downgrade(&posting),
            });
            let startup = protected_directories(cx).unwrap();
            let changed = fixture.path().join("chosen-config");
            state.update(cx, |state, _| {
                state.core.change_settings(SettingsChange::ConfigDirectory(Some(
                    changed.to_string_lossy().into_owned(),
                )));
            });
            assert_ne!(startup[1], protected_directories(cx).unwrap()[1]);
            assert_eq!(protected_directories(cx).unwrap()[1], changed);
            // Exercise the status selection used by both the alert and titlebar: an old notice stays visible during
            // checking, but cannot hide a fresh result or the restart action.
            cx.global_mut::<Updater>().status = Status::Checking;
            assert_eq!(
                cx.global::<Updater>().display_status(),
                Status::Failed("previous installer failure".into())
            );
            cx.global_mut::<Updater>().status = Status::Current;
            assert_eq!(cx.global::<Updater>().display_status(), Status::Current);
            cx.global_mut::<Updater>().status = Status::Ready("0.2.0".into());
            assert_eq!(cx.global::<Updater>().display_status(), Status::Ready("0.2.0".into()));
        });
    }

    // What follows up to the release installations below never reaches an installation: these builds are not release
    // installations, so every check is refused before the network. Each updater keeps its state in a temp directory.

    use crate::modal::tests::host;
    use crate::modal::{AlertSpec, answer_front_alert, front_alert};
    use gpui_kit::{Context, IntoElement, Render, VisualTestContext, div};

    /// `Installation::detect`'s refusal for a build without the release marker, as every test binary is.
    const SOURCE_BUILD: &str = "Automatic updates are available for official release archives, AppImages, macOS app bundles and Windows installers. Update package-manager installations with your package manager; rebuild development installations from source.";

    fn app_state(cx: &mut App) -> Entity<AppState> {
        cx.new(|_| {
            AppState::new(Core::new(
                CoreConfig {
                    app: AppIdentity::from_recorded("0.1.0", None),
                    capabilities: Vec::new(),
                },
                CoreState::default(),
            ))
        })
    }

    /// Releases from `server`, whose manifests need no signature.
    fn local(server: &fakes::Server) -> Releases {
        Releases {
            source: server.source(),
            trust: || Ok(Trust::default()),
        }
    }

    /// Releases from nowhere: a closed port on 127.0.0.1.
    fn offline() -> Releases {
        local(&fakes::Server::start().unwrap())
    }

    /// An updater over `state` that cannot update (a source build), showing `status` with nothing running and its
    /// update error file under `state_dir`.
    fn source_build(state: &Entity<AppState>, status: Status, state_dir: &Path) -> Updater {
        Updater {
            state: state.downgrade(),
            notice: None,
            installation: Err(SOURCE_BUILD.into()),
            releases: offline(),
            state_dir: state_dir.to_owned(),
            status,
            prepared: None,
            job: None,
            origin: Origin::Automatic,
            cancelled: None,
            polling: None,
            restart: false,
            poster: crate::state::detached_poster(),
        }
    }

    fn pending(cx: &mut App) -> Task<()> {
        cx.spawn(async |_| futures_lite::future::pending::<()>().await)
    }

    fn front(cx: &mut VisualTestContext) -> Option<AlertSpec> {
        cx.update(|window, cx| front_alert(window.window_handle(), cx).map(|alert| alert.read(cx).spec().clone()))
    }

    fn labels(spec: &AlertSpec) -> Vec<&str> {
        spec.buttons.iter().map(|button| button.label.as_ref()).collect()
    }

    /// Answers the front alert with `label`.
    fn press(cx: &mut VisualTestContext, label: &str) {
        assert!(
            cx.update(|window, cx| answer_front_alert(window, label, cx)),
            "no {label}"
        );
        cx.run_until_parked();
    }

    /// Shows the status alert for `status` with `notice` from an earlier run, as the titlebar button does.
    fn status_alert(cx: &mut VisualTestContext, status: Status, notice: Option<&str>) -> AlertSpec {
        cx.update(|window, cx| {
            let updater = cx.global_mut::<Updater>();
            updater.status = status;
            updater.notice = notice.map(str::to_owned);
            show_status(window, cx);
        });
        cx.run_until_parked();
        front(cx).expect("the status alert is up")
    }

    /// Names the temp state directory the child runs `init` over; the child refuses any other.
    const CHILD_STATE_DIR: &str = "NGTERM_GPUI_TEST_UPDATER_STATE_DIR";

    // `init` reads the update error file under the process's own state directory, so it runs in a child whose state
    // directory is a temp one, never the user's.
    #[test]
    fn a_development_build_installs_an_updater_that_refuses_and_never_polls() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("update-error"), "could not start the installer").unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "updater::tests::child_installs_the_updater",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(paths::STATE_DIR_ENV, dir.path())
            .env(CHILD_STATE_DIR, dir.path())
            .status()
            .unwrap();
        assert!(status.success(), "the child's checks failed");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("update-error")).unwrap(),
            "could not start the installer",
            "reading the earlier failure keeps it"
        );
    }

    #[gpui_kit::test]
    #[ignore = "run by a_development_build_installs_an_updater_that_refuses_and_never_polls, with its temp state dir"]
    fn child_installs_the_updater(cx: &mut TestAppContext) {
        // only ever in the child the parent test started, whose state directory is the temp one it named
        let Some(dir) = std::env::var_os(CHILD_STATE_DIR)
            .filter(|dir| std::env::var_os(paths::STATE_DIR_ENV).as_ref() == Some(dir))
        else {
            eprintln!("skipped: not in the parent test's temp state directory");
            return;
        };
        cx.update(|cx| {
            let state = app_state(cx);
            assert_eq!(
                state.read(cx).core.settings().auto_update,
                None,
                "automatic checks default on"
            );
            init(&state, crate::state::detached_poster(), cx);
            let updater = cx.global::<Updater>();
            assert_eq!(updater.state_dir, PathBuf::from(&dir));
            assert_eq!(
                updater.installation.as_ref().err().map(String::as_str),
                Some(SOURCE_BUILD)
            );
            assert_eq!(
                updater.notice.as_deref(),
                Some("could not start the installer"),
                "an earlier failure"
            );
            assert_eq!(updater.status, Status::Idle);
            assert!(updater.polling.is_none(), "nothing to poll for");
            assert!(updater.job.is_none());
            assert_eq!(current_installation(cx).err().as_deref(), Some(SOURCE_BUILD));
        });
    }

    #[gpui_kit::test]
    fn automatic_checks_start_only_for_an_installation_that_can_update(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            set_automatic(true, cx);
            set_automatic(false, cx);
            assert!(
                cx.try_global::<Updater>().is_none(),
                "no updater, nothing to start or stop"
            );
            let state = app_state(cx);
            cx.set_global(source_build(&state, Status::Idle, dir.path()));
            set_automatic(true, cx);
            assert!(cx.global::<Updater>().polling.is_none(), "a source build polls nothing");
            let polling = pending(cx);
            cx.global_mut::<Updater>().polling = Some(polling);
            set_automatic(false, cx);
            assert!(cx.global::<Updater>().polling.is_none(), "opting out stops it");
        });
    }

    #[gpui_kit::test]
    fn a_check_for_an_installation_that_cannot_update_never_starts(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            let state = app_state(cx);
            cx.set_global(source_build(&state, Status::Current, dir.path()));
            start_check(None, cx);
            let updater = cx.global::<Updater>();
            assert_eq!(updater.status, Status::Current, "not Checking");
            assert!(updater.job.is_none() && updater.cancelled.is_none());
        });
    }

    #[gpui_kit::test]
    fn the_protected_directories_need_the_app_state(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let state = cx.update(|cx| {
            let state = app_state(cx);
            cx.set_global(source_build(&state, Status::Idle, dir.path()));
            assert_eq!(protected_directories(cx).unwrap()[1], *state.read(cx).core.config_dir());
            state
        });
        drop(state);
        cx.run_until_parked();
        cx.update(|cx| {
            assert_eq!(
                protected_directories(cx).err().as_deref(),
                Some("Application state is unavailable.")
            );
            assert_eq!(
                current_installation(cx).err().as_deref(),
                Some(SOURCE_BUILD),
                "the installation's own refusal comes first"
            );
        });
    }

    #[gpui_kit::test]
    fn check_for_updates_says_why_this_installation_cannot_update(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let cx = host(cx);
        cx.update(check);
        cx.run_until_parked();
        assert_eq!(front(cx), None, "no updater, no alert");
        let state = cx.update(|_, cx| app_state(cx));
        cx.update(|_, cx| cx.set_global(source_build(&state, Status::Idle, dir.path())));
        cx.update(check);
        cx.run_until_parked();
        let alert = front(cx).expect("the refusal is shown");
        assert_eq!((alert.heading.as_str(), alert.body.as_str()), ("Updates", SOURCE_BUILD));
        assert_eq!(labels(&alert), ["OK"]);
        assert_eq!(cx.update(|_, cx| cx.global::<Updater>().status.clone()), Status::Idle);
        assert!(
            cx.update(|_, cx| cx.global::<Updater>().job.is_none()),
            "no check started"
        );
        press(cx, "OK");
        assert_eq!(front(cx), None);
    }

    #[gpui_kit::test]
    fn the_status_alert_says_where_the_update_stands(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let cx = host(cx);
        let state = cx.update(|_, cx| app_state(cx));
        cx.update(|_, cx| cx.set_global(source_build(&state, Status::Idle, dir.path())));
        let version = env!("CARGO_PKG_VERSION");
        for (status, heading, body) in [
            (
                Status::Idle,
                "Updates",
                "Choose Check for Updates from the Help menu.".to_owned(),
            ),
            (Status::Checking, "Updates", "Checking for updates…".to_owned()),
            (
                Status::Downloading("0.2.0".into()),
                "Updates",
                "Downloading and verifying ngterm 0.2.0…".to_owned(),
            ),
            (
                Status::Current,
                "Up to date",
                format!("No newer stable release is available for this installation. Running ngterm {version}."),
            ),
        ] {
            let alert = status_alert(cx, status.clone(), None);
            assert_eq!((alert.heading.as_str(), alert.body.as_str()), (heading, body.as_str()));
            assert_eq!(labels(&alert), ["OK"], "{status:?}");
            press(cx, "OK");
            assert_eq!(
                cx.update(|_, cx| cx.global::<Updater>().status.clone()),
                status,
                "OK changes nothing"
            );
        }

        let ready = Status::Ready("0.2.0".into());
        let alert = status_alert(cx, ready.clone(), None);
        assert_eq!(alert.heading, "Update ready");
        assert_eq!(
            alert.body,
            "ngterm 0.2.0 has been downloaded and verified. Restart to install it. Live sessions reconnect after restarting; other shells close and restore according to your session settings."
        );
        assert_eq!(labels(&alert), ["Later", "Restart to Update", "Release Notes"]);
        assert_eq!(alert.default_button(), Some(0), "Later is the default");
        press(cx, "Later");
        assert_eq!(cx.opened_url(), None, "Later opens nothing");
        assert_eq!(
            cx.update(|_, cx| cx.global::<Updater>().status.clone()),
            ready,
            "the update still waits"
        );
        status_alert(cx, ready.clone(), None);
        press(cx, "Release Notes");
        assert_eq!(cx.opened_url().as_deref(), Some(RELEASES_URL));
        assert_eq!(cx.update(|_, cx| cx.global::<Updater>().status.clone()), ready);
    }

    #[gpui_kit::test]
    fn answering_a_failure_ends_it_and_the_earlier_one(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let cx = host(cx);
        let state = cx.update(|_, cx| app_state(cx));
        cx.update(|_, cx| cx.set_global(source_build(&state, Status::Idle, dir.path())));
        let error = error_file(dir.path());
        std::fs::write(&error, "could not start the installer").unwrap();
        let alert = status_alert(
            cx,
            Status::Failed("The release has an invalid version tag.".into()),
            None,
        );
        assert_eq!(
            (alert.heading.as_str(), alert.body.as_str()),
            ("Update needs attention", "The release has an invalid version tag.")
        );
        assert_eq!(labels(&alert), ["OK", "Release Downloads"]);
        press(cx, "OK");
        assert_eq!(
            cx.update(|_, cx| cx.global::<Updater>().status.clone()),
            Status::Idle,
            "an answered failure is over"
        );
        assert!(!error.exists(), "and so is the earlier one");
        assert_eq!(cx.opened_url(), None, "OK opens nothing");
    }

    #[gpui_kit::test]
    fn an_earlier_failure_shows_until_a_result_replaces_it(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let cx = host(cx);
        let state = cx.update(|_, cx| app_state(cx));
        cx.update(|_, cx| cx.set_global(source_build(&state, Status::Idle, dir.path())));
        let alert = status_alert(cx, Status::Current, Some("could not start the installer"));
        assert_eq!(alert.heading, "Up to date", "a fresh result hides the old failure");
        press(cx, "OK");
        let ready = status_alert(cx, Status::Ready("0.2.0".into()), Some("could not start the installer"));
        assert_eq!(
            ready.heading, "Update ready",
            "and so does an update waiting to restart"
        );
        press(cx, "Later");
        let error = error_file(dir.path());
        std::fs::write(&error, "could not start the installer").unwrap();
        let alert = status_alert(cx, Status::Checking, Some("could not start the installer"));
        assert_eq!(
            (alert.heading.as_str(), alert.body.as_str()),
            ("Update needs attention", "could not start the installer"),
            "while checking, the earlier failure stays up"
        );
        assert_eq!(cx.opened_url(), None);
        press(cx, "Release Downloads");
        assert_eq!(cx.opened_url().as_deref(), Some(RELEASES_URL));
        assert!(!error.exists(), "an answered failure is gone for good");
        cx.update(|_, cx| {
            let updater = cx.global::<Updater>();
            assert_eq!(updater.notice, None);
            assert_eq!(updater.status, Status::Checking, "the check goes on");
        });
    }

    /// A titlebar with the updater's button, over the window's alerts.
    struct Titlebar;

    impl Render for Titlebar {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .children(button(crate::chrome::default_colors(), cx))
                .children(crate::modal::alert_layer(window.window_handle(), cx))
        }
    }

    /// Draws a frame and says whether the titlebar shows the updater's button.
    fn button_shown(cx: &mut VisualTestContext) -> bool {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        cx.debug_bounds("update-status").is_some()
    }

    #[gpui_kit::test]
    fn the_titlebar_button_shows_only_while_an_update_needs_the_user(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(gpui_kit::init);
        let (_, cx) = cx.add_window_view(|_, _| Titlebar);
        assert!(!button_shown(cx), "no updater, no button");
        let state = cx.update(|_, cx| app_state(cx));
        cx.update(|_, cx| cx.set_global(source_build(&state, Status::Idle, dir.path())));
        for (status, shown) in [
            (Status::Idle, false),
            (Status::Checking, false),
            (Status::Current, false),
            (Status::Downloading("0.2.0".into()), true),
            (Status::Ready("0.2.0".into()), true),
            (Status::Failed("offline".into()), true),
        ] {
            cx.update(|_, cx| cx.global_mut::<Updater>().status = status.clone());
            assert_eq!(button_shown(cx), shown, "{status:?}");
        }
        cx.update(|_, cx| {
            let updater = cx.global_mut::<Updater>();
            updater.status = Status::Idle;
            updater.notice = Some("could not start the installer".into());
        });
        assert!(button_shown(cx), "an earlier failure needs the user too");

        cx.update(|_, cx| {
            let updater = cx.global_mut::<Updater>();
            updater.status = Status::Downloading("0.2.0".into());
            updater.notice = None;
        });
        assert!(button_shown(cx));
        let center = cx
            .debug_bounds("update-status")
            .expect("the button is laid out")
            .center();
        cx.simulate_click(center, gpui_kit::Modifiers::none());
        cx.run_until_parked();
        let alert = front(cx).expect("a click shows the status");
        assert_eq!(alert.body, "Downloading and verifying ngterm 0.2.0…");
        press(cx, "OK");
    }

    #[gpui_kit::test]
    fn quitting_cancels_the_download_and_drops_the_check_and_the_polling(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            on_quit(false, cx);
            assert!(cx.try_global::<Updater>().is_none(), "no updater, nothing to stop");
            let state = app_state(cx);
            let cancelled = Arc::new(AtomicBool::new(false));
            let (job, polling) = (pending(cx), pending(cx));
            cx.set_global(Updater {
                job: Some(job),
                polling: Some(polling),
                cancelled: Some(Arc::clone(&cancelled)),
                ..source_build(&state, Status::Downloading("0.2.0".into()), dir.path())
            });
            on_quit(false, cx);
            let updater = cx.global::<Updater>();
            assert!(cancelled.load(Ordering::Acquire), "the download is told to stop");
            assert!(updater.job.is_none() && updater.polling.is_none() && updater.cancelled.is_none());
        });
    }

    // What follows runs the updater over a release installation in a temp directory (a Linux archive's, which only
    // Linux accepts), against releases on 127.0.0.1 that need no signature (ngterm-update's tests cover the key).
    // An installer helper that takes a staged update over is stopped before it can outlive its test.

    #[cfg(target_os = "linux")]
    use ngterm_core::settings_model::GeneralChange;
    use ngterm_update::fakes;

    /// A release newer than this build.
    #[cfg(target_os = "linux")]
    const NEWER: &str = "999.0.0";

    /// An installation of 0.1.0 in a temp directory, a state directory beside it and a release server.
    #[cfg(target_os = "linux")]
    struct Fixture {
        dir: tempfile::TempDir,
        installation: Installation,
        server: fakes::Server,
    }

    #[cfg(target_os = "linux")]
    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            fakes::tree(&dir.path().join("ngterm"), "0.1.0").unwrap();
            std::fs::create_dir(dir.path().join("state")).unwrap();
            Self {
                installation: fakes::installation(&dir.path().join("ngterm/usr/bin/ngterm")).unwrap(),
                dir,
                server: fakes::Server::start().unwrap(),
            }
        }

        /// The same, with a newer release published.
        fn newer() -> Self {
            let fixture = Self::new();
            fixture.server.publish_archive(NEWER, &fakes::archive(NEWER).unwrap());
            fixture
        }

        fn root(&self) -> PathBuf {
            self.dir.path().join("ngterm")
        }

        fn state_dir(&self) -> PathBuf {
            self.dir.path().join("state")
        }

        /// The update stages beside the installation.
        fn stages(&self) -> Vec<PathBuf> {
            std::fs::read_dir(self.dir.path())
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|path| {
                    path.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with(".ngterm-update-")
                })
                .collect()
        }

        /// Installs the updater as `init` does, for this installation and server, with automatic checks
        /// `automatic` and the app's configuration under the state directory.
        fn start(&self, automatic: bool, cx: &mut App) -> Entity<AppState> {
            let mut core = Core::new(
                CoreConfig {
                    app: AppIdentity::from_recorded("0.1.0", None),
                    capabilities: Vec::new(),
                },
                CoreState::default(),
            );
            core.set_config_locator(ConfigLocator::capture(
                &MapEnv::new().with(paths::STATE_DIR_ENV, self.state_dir().as_os_str()),
            ));
            if !automatic {
                core.change_settings(SettingsChange::General(GeneralChange::AutoUpdate(false)));
            }
            let state = cx.new(|_| AppState::new(core));
            start(
                &state,
                crate::state::detached_poster(),
                Ok(self.installation.clone()),
                local(&self.server),
                self.state_dir(),
                cx,
            );
            state
        }
    }

    /// The requests a check and download of the newer release make.
    #[cfg(target_os = "linux")]
    fn newer_downloads() -> Vec<String> {
        vec![
            "/latest".to_owned(),
            format!("/download/v{NEWER}/SHA256SUMS"),
            format!("/download/v{NEWER}/{}", fakes::asset(NEWER)),
        ]
    }

    #[cfg(target_os = "linux")]
    fn status(cx: &mut VisualTestContext) -> Status {
        cx.update(|_, cx| cx.global::<Updater>().status.clone())
    }

    /// Stops and reaps the installer helper that took `stage` over, proving it had not exited on its own.
    #[cfg(target_os = "linux")]
    fn stop_helper(stage: &Path) {
        let owner: serde_json::Value =
            serde_json::from_slice(&std::fs::read(stage.join("owner.json")).unwrap()).unwrap();
        let pid = libc::pid_t::try_from(owner["pid"].as_u64().unwrap()).unwrap();
        assert!(pid > 0);
        assert_ne!(
            pid,
            libc::pid_t::try_from(std::process::id()).unwrap(),
            "the helper owns the stage"
        );
        // SAFETY: pid names the helper spawned by this test, never the test process or a process group.
        let killed = unsafe { libc::kill(pid, libc::SIGTERM) };
        let mut status = 0;
        loop {
            // SAFETY: pid names our child and status is a valid writable exit-status pointer.
            let waited = unsafe { libc::waitpid(pid, &raw mut status, 0) };
            if waited == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            assert_eq!(waited, pid, "the helper is reaped");
            break;
        }
        assert_eq!(killed, 0, "the helper received SIGTERM");
        assert!(
            libc::WIFSIGNALED(status) && libc::WTERMSIG(status) == libc::SIGTERM,
            "the helper was still waiting for this process to quit"
        );
    }

    #[gpui_kit::test]
    #[cfg(target_os = "linux")]
    fn an_automatic_check_stages_a_newer_release_and_restart_hands_it_to_the_installer(cx: &mut TestAppContext) {
        let fixture = Fixture::newer();
        let cx = host(cx);
        let _state = cx.update(|_, cx| fixture.start(true, cx));
        assert!(cx.update(|_, cx| cx.global::<Updater>().polling.is_some()));
        cx.run_until_parked();
        assert_eq!(status(cx), Status::Ready(NEWER.into()));
        assert_eq!(front(cx), None, "an automatic check says nothing");
        assert_eq!(fixture.server.requests(), newer_downloads());
        let [stage] = <[PathBuf; 1]>::try_from(fixture.stages()).unwrap();
        assert_eq!(
            std::fs::read_to_string(stage.join("unpacked/ngterm/usr/share/ngterm/app-BUILD")).unwrap(),
            format!("{NEWER}\ncommit\n"),
            "the verified release is unpacked beside the installation"
        );
        cx.update(|_, cx| start_check(None, cx));
        assert_eq!(
            fixture.server.requests().len(),
            3,
            "a staged update is not checked for again"
        );

        cx.update(check);
        cx.run_until_parked();
        let alert = front(cx).expect("Check for Updates shows the waiting update");
        assert_eq!(alert.heading, "Update ready");
        assert_eq!(fixture.server.requests().len(), 3);
        press(cx, "Restart to Update");
        stop_helper(&stage);
        cx.update(|_, cx| {
            let updater = cx.global::<Updater>();
            assert!(updater.restart && updater.prepared.is_none());
        });
        assert!(stage.join("install.sh").is_file(), "the helper's script is staged");
        assert_eq!(
            std::fs::read_to_string(fixture.root().join("usr/share/ngterm/app-BUILD")).unwrap(),
            "0.1.0\ncommit\n",
            "the installation waits for the app to quit"
        );
        cx.update(|_, cx| start_check(None, cx));
        assert_eq!(
            status(cx),
            Status::Ready(NEWER.into()),
            "nothing starts once restarting"
        );
        assert_eq!(fixture.server.requests().len(), 3);
        assert!(!error_file(&fixture.state_dir()).exists());
    }

    #[gpui_kit::test]
    #[cfg(target_os = "linux")]
    fn a_manual_check_says_whether_this_installation_is_current_or_why_the_check_failed(cx: &mut TestAppContext) {
        let fixture = Fixture::new();
        let error = error_file(&fixture.state_dir());
        std::fs::write(&error, "could not start the installer").unwrap();
        // an abandoned stage of an earlier run, whose owner is gone
        let abandoned = fixture.dir.path().join(".ngterm-update-abandoned");
        std::fs::create_dir(&abandoned).unwrap();
        std::fs::write(
            abandoned.join("owner.json"),
            serde_json::json!({ "installation": fixture.root(), "pid": i32::MAX }).to_string(),
        )
        .unwrap();
        fixture
            .server
            .publish_archive(env!("CARGO_PKG_VERSION"), b"the running version");
        let cx = host(cx);
        let _state = cx.update(|_, cx| fixture.start(false, cx));
        cx.update(|_, cx| {
            let updater = cx.global::<Updater>();
            assert!(updater.polling.is_none(), "automatic checks are off");
            assert_eq!(updater.notice.as_deref(), Some("could not start the installer"));
        });
        cx.run_until_parked();
        assert!(!abandoned.exists(), "starting removes abandoned stages");
        assert!(fixture.server.requests().is_empty(), "and checks nothing on its own");

        cx.update(|window, cx| {
            check(window, cx);
            assert_eq!(cx.global::<Updater>().status, Status::Checking);
            assert!(cx.global::<Updater>().origin == Origin::Manual);
            start_check(None, cx);
            assert!(cx.global::<Updater>().origin == Origin::Manual, "one check at a time");
        });
        cx.run_until_parked();
        let version = env!("CARGO_PKG_VERSION");
        let alert = front(cx).expect("a manual check shows its result");
        assert_eq!(
            (alert.heading.as_str(), alert.body.as_str()),
            (
                "Up to date",
                format!("No newer stable release is available for this installation. Running ngterm {version}.")
                    .as_str()
            )
        );
        assert!(!error.exists(), "a check that succeeds clears the earlier failure");
        assert_eq!(cx.update(|_, cx| cx.global::<Updater>().notice.clone()), None);
        press(cx, "OK");
        assert_eq!(fixture.server.requests(), ["/latest"]);

        cx.update(|window, cx| {
            check(window, cx);
            check(window, cx);
        });
        let alert = front(cx).expect("a check while one runs shows its progress");
        assert_eq!(alert.body, "Checking for updates…");
        press(cx, "OK");
        assert_eq!(front(cx).map(|alert| alert.heading), Some("Up to date".into()));
        press(cx, "OK");
        assert_eq!(fixture.server.requests().len(), 2, "the second check joined the first");

        fixture
            .server
            .route("/latest", "500 Internal Server Error", "unavailable");
        cx.update(check);
        cx.run_until_parked();
        let alert = front(cx).expect("a failed check says why");
        assert_eq!(
            (alert.heading.as_str(), alert.body.as_str()),
            ("Update needs attention", "http status: 500")
        );
        assert_eq!(status(cx), Status::Failed("http status: 500".into()));
        press(cx, "Release Downloads");
        assert_eq!(cx.opened_url().as_deref(), Some(RELEASES_URL));
        assert_eq!(status(cx), Status::Idle, "an answered failure is over");
    }

    #[gpui_kit::test]
    #[cfg(target_os = "linux")]
    fn quitting_hands_a_staged_update_to_the_installer_unless_a_live_reset_relaunches(cx: &mut TestAppContext) {
        for live_reset in [false, true] {
            let fixture = Fixture::newer();
            let _state = cx.update(|cx| fixture.start(true, cx));
            cx.run_until_parked();
            let [stage] = <[PathBuf; 1]>::try_from(fixture.stages()).unwrap();
            cx.update(|cx| {
                on_quit(live_reset, cx);
                if !live_reset {
                    stop_helper(&stage);
                }
                let updater = cx.global::<Updater>();
                assert!(updater.prepared.is_none() && updater.polling.is_none());
            });
            if live_reset {
                assert!(fixture.stages().is_empty(), "a live reset discards the update");
            } else {
                assert!(stage.join("install.sh").is_file(), "the installer takes the stage over");
            }
            assert!(!error_file(&fixture.state_dir()).exists());
        }
    }

    #[gpui_kit::test]
    #[cfg(target_os = "linux")]
    fn an_update_the_installer_cannot_take_over_says_why(cx: &mut TestAppContext) {
        let fixture = Fixture::newer();
        let cx = host(cx);
        let _state = cx.update(|_, cx| fixture.start(true, cx));
        cx.run_until_parked();
        assert_eq!(status(cx), Status::Ready(NEWER.into()));
        std::fs::remove_dir_all(fixture.root()).unwrap();
        let gone = std::fs::metadata(fixture.root()).unwrap_err().to_string();
        cx.update(show_status);
        press(cx, "Restart to Update");
        let alert = front(cx).expect("a restart that cannot install says why");
        assert_eq!(
            (alert.heading.as_str(), alert.body.as_str()),
            ("Update needs attention", gone.as_str())
        );
        assert_eq!(status(cx), Status::Failed(gone));
        assert!(!cx.update(|_, cx| cx.global::<Updater>().restart), "the app stays open");
        assert!(fixture.stages().is_empty(), "the update is discarded");
        press(cx, "OK");
        assert_eq!(status(cx), Status::Idle);

        // on quit, the error waits in the state directory for the next launch
        let fixture = Fixture::newer();
        let state = cx.update(|_, cx| fixture.start(true, cx));
        cx.run_until_parked();
        assert_eq!(status(cx), Status::Ready(NEWER.into()));
        drop(state);
        cx.run_until_parked();
        cx.update(|_, cx| on_quit(false, cx));
        assert_eq!(
            std::fs::read_to_string(error_file(&fixture.state_dir())).unwrap(),
            "Application state is unavailable."
        );
        assert!(fixture.stages().is_empty());
    }
}
