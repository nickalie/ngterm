//! The app-wide release updater: one worker at a time, hourly automatic checks, explicit restart.
use std::path::PathBuf;
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
use ngterm_update::{Installation, Prepared, RELEASES_URL};

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

struct Updater {
    state: WeakEntity<AppState>,
    notice: Option<String>,
    installation: Result<Installation, String>,
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

fn error_file() -> PathBuf {
    paths::state_dir(&ProcessEnv).join("update-error")
}

/// Installs the updater after the app has loaded its settings.
pub fn init(state: &Entity<AppState>, poster: Poster, cx: &mut App) {
    let notice = std::fs::read_to_string(error_file()).ok();
    let automatic = state.read(cx).core.settings().auto_update.unwrap_or(true);
    let installation = Installation::detect().map_err(|error| error.to_string());
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
    let task = cx.spawn(async move |cx| {
        let checked = cx
            .background_spawn({
                let installation = installation.clone();
                async move {
                    let _ = installation.cleanup_stale();
                    let trust = ngterm_update::Trust::release()?;
                    ngterm_update::check(&installation, env!("CARGO_PKG_VERSION"), &trust)
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
                    ngterm_update::prepare(&installation, &release, &trust, |_, _| {
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
                let _ = std::fs::remove_file(error_file());
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
                let _ = std::fs::remove_file(error_file());
                let updater = cx.global_mut::<Updater>();
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
        let _ = std::fs::write(error_file(), error);
    }
}

fn protected_directories(cx: &App) -> Result<[PathBuf; 2], String> {
    let state = cx
        .global::<Updater>()
        .state
        .upgrade()
        .ok_or("Application state is unavailable.")?;
    Ok([paths::state_dir(&ProcessEnv), state.read(cx).core.config_dir().clone()])
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
        .launch(&error_file(), reopen, &paths)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {

    #[gpui_kit::test]
    fn opting_out_cancels_automatic_work_and_keeps_manual_work(cx: &mut TestAppContext) {
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

    // What follows never reaches a real installation: these builds are not release installations, so every check is
    // refused before the network. An answer to an "Update needs attention" alert removes the update error file under
    // the user's real state directory (`error_file`, with no seam), so those alerts are shown and never answered.

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

    /// An updater over `state` that cannot update (a source build), showing `status` with nothing running.
    fn source_build(state: &Entity<AppState>, status: Status) -> Updater {
        Updater {
            state: state.downgrade(),
            notice: None,
            installation: Err(SOURCE_BUILD.into()),
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

    // `init` reads the update error file under the process's state directory (`error_file`, with no seam), so it runs
    // in a child whose state directory is a temp one, never the user's.
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
        assert_eq!(error_file(), PathBuf::from(dir).join("update-error"));
        cx.update(|cx| {
            let state = app_state(cx);
            assert_eq!(
                state.read(cx).core.settings().auto_update,
                None,
                "automatic checks default on"
            );
            init(&state, crate::state::detached_poster(), cx);
            let updater = cx.global::<Updater>();
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
        cx.update(|cx| {
            set_automatic(true, cx);
            set_automatic(false, cx);
            assert!(
                cx.try_global::<Updater>().is_none(),
                "no updater, nothing to start or stop"
            );
            let state = app_state(cx);
            cx.set_global(source_build(&state, Status::Idle));
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
        cx.update(|cx| {
            let state = app_state(cx);
            cx.set_global(source_build(&state, Status::Current));
            start_check(None, cx);
            let updater = cx.global::<Updater>();
            assert_eq!(updater.status, Status::Current, "not Checking");
            assert!(updater.job.is_none() && updater.cancelled.is_none());
        });
    }

    #[gpui_kit::test]
    fn the_protected_directories_need_the_app_state(cx: &mut TestAppContext) {
        let state = cx.update(|cx| {
            let state = app_state(cx);
            cx.set_global(source_build(&state, Status::Idle));
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
        let cx = host(cx);
        cx.update(check);
        cx.run_until_parked();
        assert_eq!(front(cx), None, "no updater, no alert");
        let state = cx.update(|_, cx| app_state(cx));
        cx.update(|_, cx| cx.set_global(source_build(&state, Status::Idle)));
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
        let cx = host(cx);
        let state = cx.update(|_, cx| app_state(cx));
        cx.update(|_, cx| cx.set_global(source_build(&state, Status::Idle)));
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

        // last: answering it would remove the user's real update error file
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
    }

    #[gpui_kit::test]
    fn an_earlier_failure_shows_until_a_result_replaces_it(cx: &mut TestAppContext) {
        let cx = host(cx);
        let state = cx.update(|_, cx| app_state(cx));
        cx.update(|_, cx| cx.set_global(source_build(&state, Status::Idle)));
        let alert = status_alert(cx, Status::Current, Some("could not start the installer"));
        assert_eq!(alert.heading, "Up to date", "a fresh result hides the old failure");
        press(cx, "OK");
        let ready = status_alert(cx, Status::Ready("0.2.0".into()), Some("could not start the installer"));
        assert_eq!(
            ready.heading, "Update ready",
            "and so does an update waiting to restart"
        );
        press(cx, "Later");
        // last: answering it would remove the user's real update error file
        let alert = status_alert(cx, Status::Checking, Some("could not start the installer"));
        assert_eq!(
            (alert.heading.as_str(), alert.body.as_str()),
            ("Update needs attention", "could not start the installer"),
            "while checking, the earlier failure stays up"
        );
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
        cx.update(gpui_kit::init);
        let (_, cx) = cx.add_window_view(|_, _| Titlebar);
        assert!(!button_shown(cx), "no updater, no button");
        let state = cx.update(|_, cx| app_state(cx));
        cx.update(|_, cx| cx.set_global(source_build(&state, Status::Idle)));
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
                ..source_build(&state, Status::Downloading("0.2.0".into()))
            });
            on_quit(false, cx);
            let updater = cx.global::<Updater>();
            assert!(cancelled.load(Ordering::Acquire), "the download is told to stop");
            assert!(updater.job.is_none() && updater.polling.is_none() && updater.cancelled.is_none());
        });
    }
}
