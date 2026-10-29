//! Following the desktop's light or dark appearance (upstream's `SystemAppearanceObserver.swift` and
//! `SettingsModel.appearanceChanged`; ported from the GTK build's `theme.rs`, which read libadwaita's style manager),
//! by the platform's source ([`DesktopSide`]):
//!
//! - Linux ([`portal`]): the XDG settings portal's `org.freedesktop.appearance` `color-scheme` over `zbus`, which is
//!   what libadwaita follows on every desktop that runs a portal backend (GNOME, KDE, the wlroots portals, darkman).
//! - Windows: GPUI's own reading of the system's app mode (`WindowAppearance`, from `UISettings`) and its windows'
//!   appearance observation, which GPUI updates on `WM_SETTINGCHANGE` (`ImmersiveColorSet`).
//!
//! - **Launch**: [`DesktopSide::read`] reads the side before the config loads, so the first terminal starts on the
//!   desktop's side of a dual `theme = light:…,dark:…`. On Linux no session bus, no portal, or a portal without the
//!   key reads as light, libadwaita's default for "no preference".
//! - **Following**: on Linux [`Portal::follow`] subscribes to the portal's `SettingChanged` for the key on a thread of
//!   its own and reads the key once more after subscribing, so no change between the launch read and the subscription
//!   is lost; on Windows every window reports its appearance when GPUI sees it change ([`observe_window`]). Each value
//!   is debounced on the main loop (upstream's 0.05 s) and lands in [`AppState::appearance_changed`].
//! - **The flip**: as upstream, a new side re-feeds the config only while following the system appearance with both
//!   theme slots set ([`Appearance::flip_reloads`]); every reload, whichever path asked for it, loads the side
//!   [`Appearance::side`] gives and records it ([`AppState::config_applied`]). The terminals keep their font sizes
//!   across it (a config change never resets a live size here), and each takes the side as its color scheme
//!   (`TerminalConfig::color_scheme`).
//! - **`debug.appearance`** forces a side for good, as the GTK build's forced style-manager scheme did: the desktop
//!   no longer moves the app, and the flip runs at once.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use gpui_kit::{App, Subscription, Window};
use ngterm_core::persist::AppSettings;

use crate::state::{AppState, Poster};

#[cfg(target_os = "linux")]
pub mod portal;
#[cfg(target_os = "linux")]
pub use portal::{ColorScheme, KEY, NAMESPACE, PORTAL, PORTAL_PATH, Portal, SETTINGS_INTERFACE};

/// How long a burst of appearance changes settles before one report (upstream's `appearanceDebounceInterval`).
pub const DEBOUNCE: Duration = Duration::from_millis(50);
/// The desktop's side as the platform reports it, read at launch and followed from then on.
pub struct DesktopSide {
    #[cfg(target_os = "linux")]
    portal: Option<Portal>,
    dark: bool,
}

impl DesktopSide {
    /// The desktop's side now, before the config loads.
    #[must_use]
    pub fn read(cx: &App) -> Self {
        #[cfg(target_os = "linux")]
        {
            let _ = cx;
            let portal = Portal::session();
            let dark = portal.as_ref().is_some_and(Portal::is_dark);
            Self { portal, dark }
        }
        #[cfg(not(target_os = "linux"))]
        {
            let dark = is_dark(cx.window_appearance());
            tracing::info!(dark, "appearance: the system's app mode");
            Self { dark }
        }
    }

    /// Whether the desktop is on the dark side.
    #[must_use]
    pub fn is_dark(&self) -> bool {
        self.dark
    }

    /// Follows the desktop into the app state behind `poster` (Linux: the portal's thread; elsewhere the windows
    /// report, [`observe_window`]).
    pub fn follow(self, poster: &Poster) {
        #[cfg(target_os = "linux")]
        if let Some(portal) = self.portal
            && let Err(error) = follow_into(portal, poster.clone())
        {
            tracing::error!(%error, "appearance: the desktop's light/dark changes are not followed");
        }
        #[cfg(not(target_os = "linux"))]
        let _ = (self, poster);
    }
}

/// Whether GPUI's `appearance` is a dark one.
#[must_use]
pub fn is_dark(appearance: gpui_kit::WindowAppearance) -> bool {
    matches!(
        appearance,
        gpui_kit::WindowAppearance::Dark | gpui_kit::WindowAppearance::VibrantDark
    )
}

/// Reports `window`'s appearance into the app state behind `poster` whenever GPUI sees it change, debounced as the
/// portal's reports are: the Windows source (see the module docs); `None` on Linux, which follows the portal.
pub fn observe_window<V: 'static>(
    poster: &Poster,
    window: &Window,
    cx: &mut gpui_kit::Context<V>,
) -> Option<Subscription> {
    #[cfg(target_os = "linux")]
    {
        let _ = (poster, window, cx);
        None
    }
    #[cfg(not(target_os = "linux"))]
    {
        static LATEST: std::sync::OnceLock<Arc<AtomicU64>> = std::sync::OnceLock::new();
        let latest = Arc::clone(LATEST.get_or_init(Arc::default));
        let poster = poster.clone();
        Some(cx.observe_window_appearance(window, move |_, window, _| {
            debounce(&poster, &latest, is_dark(window.appearance()));
        }))
    }
}

#[cfg(target_os = "linux")]
/// Follows the portal into the app state: each scheme is debounced on the main loop by [`DEBOUNCE`] (a later one
/// supersedes it) and then lands in [`AppState::appearance_changed`].
///
/// # Errors
///
/// The following thread could not start.
pub fn follow_into(portal: Portal, poster: Poster) -> std::io::Result<()> {
    let latest = Arc::new(AtomicU64::new(0));
    portal.follow(move |scheme| debounce(&poster, &latest, scheme.is_dark()))?;
    Ok(())
}

/// Posts `dark` to land after [`DEBOUNCE`] unless a later report comes first.
fn debounce(poster: &Poster, latest: &Arc<AtomicU64>, dark: bool) {
    let Some(main_loop) = poster.upgrade() else {
        return;
    };
    let mine = latest.fetch_add(1, Ordering::AcqRel) + 1;
    let current = Arc::clone(latest);
    let task = Box::new(move |state: &mut AppState| {
        // the side rides the report, never re-read when the debounce fires
        if current.load(Ordering::Acquire) == mine {
            state.appearance_changed(dark);
        }
    });
    if main_loop.post_after(DEBOUNCE, task).is_err() {
        tracing::debug!("appearance: the report is dropped, the loop is closed");
    }
}

/// The side the terminals render and why (upstream's `lastAppliedIsDark` and the system appearance it follows).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Appearance {
    /// The desktop's side as the portal last reported it.
    desktop_dark: bool,
    /// The side `debug.appearance` forced, which the desktop no longer moves.
    forced: Option<bool>,
    /// The side the last config load applied.
    last_applied_dark: bool,
    /// A flip asked for a reload the windows have not run yet.
    reload: bool,
}

impl Appearance {
    /// The app launched with the desktop on `dark` and loaded its config on that side.
    #[must_use]
    pub fn new(dark: bool) -> Self {
        Self {
            desktop_dark: dark,
            forced: None,
            last_applied_dark: dark,
            reload: false,
        }
    }

    /// The side a config load takes now: the forced one, else the desktop's.
    #[must_use]
    pub fn side(&self) -> bool {
        self.forced.unwrap_or(self.desktop_dark)
    }

    /// The side the last config load applied.
    #[must_use]
    pub fn last_applied_dark(&self) -> bool {
        self.last_applied_dark
    }

    /// Whether a flip to `dark` re-feeds the config: only while following with both slots set (a dual line exists),
    /// and only when the side moved.
    #[must_use]
    pub fn flip_reloads(&self, settings: &AppSettings, dark: bool) -> bool {
        settings.follows_system_appearance()
            && settings.theme.is_some()
            && settings.dark_theme.is_some()
            && dark != self.last_applied_dark
    }

    /// The desktop moved to `dark`; whether that asks for a reload (never while a side is forced).
    pub fn desktop_changed(&mut self, settings: &AppSettings, dark: bool) -> bool {
        self.desktop_dark = dark;
        self.forced.is_none() && self.ask_reload(settings, dark)
    }

    /// `debug.appearance` forced `dark`; whether that asks for a reload.
    pub fn force(&mut self, settings: &AppSettings, dark: bool) -> bool {
        self.forced = Some(dark);
        self.ask_reload(settings, dark)
    }

    fn ask_reload(&mut self, settings: &AppSettings, dark: bool) -> bool {
        let reloads = self.flip_reloads(settings, dark);
        self.reload |= reloads;
        reloads
    }

    /// Takes the reload a flip asked for.
    pub fn take_reload(&mut self) -> bool {
        std::mem::take(&mut self.reload)
    }

    /// A config load applied `dark`.
    pub fn applied(&mut self, dark: bool) {
        self.last_applied_dark = dark;
    }
}

/// A config reload the windows run before they render: on `dark`; `asked` when the core asked for it (a command or a
/// settings change, which takes the diagnostic count back), else an appearance flip alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigReload {
    pub dark: bool,
    pub asked: bool,
}

impl AppState {
    /// The app launched with its config loaded on `dark`, the desktop's side.
    pub fn launched_on(&mut self, dark: bool) {
        self.appearance = Appearance::new(dark);
        self.core.report_applied_appearance(dark);
    }

    /// The desktop's appearance settled on `dark` (upstream's `SettingsModel.appearanceChanged`): while following with
    /// both slots set, a new side re-feeds the config.
    pub fn appearance_changed(&mut self, dark: bool) {
        if self.is_quitting() {
            return;
        }
        if self.appearance.desktop_changed(self.core.settings(), dark) {
            tracing::info!(dark, "appearance: the desktop flipped; reloading the config");
            self.settle();
        }
    }

    /// What the windows reload before they render: the side `debug.appearance` forced is taken first (its flip runs
    /// at once, rather than waiting for a desktop report), then a command's reload or a flip's.
    pub fn take_config_reload(&mut self) -> Option<ConfigReload> {
        if let Some(dark) = self.core.take_debug_appearance() {
            self.appearance.force(self.core.settings(), dark);
        }
        let asked = self.core.take_config_reload();
        let flipped = self.appearance.take_reload();
        (asked || flipped).then(|| ConfigReload {
            dark: self.appearance.side(),
            asked,
        })
    }

    /// A config load applied `dark`: recorded, and readable through `debug.appearance`.
    pub fn config_applied(&mut self, dark: bool) {
        self.appearance.applied(dark);
        self.core.report_applied_appearance(dark);
    }

    /// The side the terminals render.
    #[must_use]
    pub fn applied_dark(&self) -> bool {
        self.appearance.last_applied_dark()
    }
}

#[cfg(test)]
mod tests;
