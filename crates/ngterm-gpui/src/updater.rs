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
                    ngterm_update::check(&installation, env!("CARGO_PKG_VERSION"))
                }
            })
            .await;
        let result = match checked {
            Ok(Some(release)) => {
                if cancelled.load(Ordering::Acquire) {
                    return;
                }
                let version = release.version.to_string();
                cx.update(|cx| {
                    cx.global_mut::<Updater>().status = Status::Downloading(version.clone());
                    cx.refresh_windows();
                });
                cx.background_spawn(async move {
                    ngterm_update::prepare(&installation, &release, |_, _| !cancelled.load(Ordering::Acquire))
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
}
