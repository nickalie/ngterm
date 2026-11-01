//! The system's Reduce Transparency (upstream's `NSWorkspace.accessibilityDisplayShouldReduceTransparency`), as one
//! app global ([`ReduceTransparency`]) kept current per OS ([`watch`]), the way `status_glyph.rs` follows the motion
//! switch:
//!
//! - Windows: Settings ▸ Personalization ▸ Colors ▸ "Transparency effects" (`UISettings::AdvancedEffectsEnabled`, its
//!   registry mirror `HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize\EnableTransparency`), off
//!   meaning reduce, read at launch and again on every `WM_SETTINGCHANGE` the app's hidden window sees.
//! - macOS: `accessibilityDisplayShouldReduceTransparency`, followed through
//!   `NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification`: open work, with the GPUI macOS frontend.
//! - Linux: no counterpart (the settings portal's `org.freedesktop.appearance` carries the color scheme, the accent
//!   color, the contrast and reduced motion only; GNOME's a11y schema has no transparency key), so the system input
//!   reads off there. Settings ▸ Appearance ▸ Window's Ignore Reduce Transparency still shows and round-trips, for
//!   parity and a `settings.json` shared across OSes.
//!
//! Upstream's one rule, [`ngterm_core::chrome::window_reduces_transparency`], combines it with the setting: the window
//! presents opaque and unblurred while it holds (`crate::chrome::Translucency::of`), the inactive-pane wash scales by
//! that presented opacity, and the Settings hint names it. The palettes and the session switcher draw opaque whatever
//! the setting, so the system's preference, which upstream keeps them on, never has anything to take from them here.
//!
//! The per-OS reads are not built on macOS or Windows by CI until `ngterm-gpui` joins `cross-platform.yml`.

use gpui_kit::{App, Global};

use crate::state::Poster;

/// Whether the system asks apps to reduce transparency now; off before [`watch`] and where unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReduceTransparency {
    pub on: bool,
}

impl Global for ReduceTransparency {}

/// Whether the system asks for reduced transparency now.
#[must_use]
pub fn system(cx: &App) -> bool {
    cx.try_global::<ReduceTransparency>().is_some_and(|reduce| reduce.on)
}

/// Starts following the system's preference, once per app; a flip settles the state behind `poster`, so the windows'
/// sync re-resolves the translucency (`crate::chrome::sync`) and the open Settings popup's hint.
pub fn watch(poster: &Poster, cx: &mut App) {
    if cx.has_global::<ReduceTransparency>() {
        return;
    }
    cx.set_global(ReduceTransparency::default());
    #[cfg(windows)]
    watch_transparency_effects(poster.clone(), cx);
    #[cfg(not(windows))]
    let _ = poster;
}

/// Records `on`; whether it changed.
pub fn set(on: bool, cx: &mut App) -> bool {
    if system(cx) == on {
        return false;
    }
    tracing::debug!(on, "the system's Reduce Transparency changed");
    cx.set_global(ReduceTransparency { on });
    true
}

/// Windows's switch now, and again after each changed system setting (the app's hidden window sees them).
#[cfg(windows)]
fn watch_transparency_effects(poster: Poster, cx: &mut App) {
    let (notify, settings) = async_channel::unbounded::<()>();
    if let Err(error) = crate::system_window::on_setting_change(move |_| {
        let _ = notify.try_send(());
    }) {
        tracing::warn!(%error, "the transparency effects switch is read once, not followed");
    }
    cx.spawn(async move |cx| {
        update(cx, &poster);
        while settings.recv().await.is_ok() {
            update(cx, &poster);
        }
    })
    .detach();
}

#[cfg(windows)]
fn update(cx: &mut gpui_kit::AsyncApp, poster: &Poster) {
    let on = !transparency_effects();
    if cx.update(|cx| set(on, cx)) {
        crate::state::post(poster, crate::state::AppState::settle);
    }
}

/// "Transparency effects": on when it cannot be read.
#[cfg(windows)]
fn transparency_effects() -> bool {
    ::windows::UI::ViewManagement::UISettings::new()
        .and_then(|settings| settings.AdvancedEffectsEnabled())
        .unwrap_or_else(|error| {
            tracing::debug!(%error, "transparency effects unreadable: taken as on");
            true
        })
}

#[cfg(test)]
mod tests {
    use gpui_kit::TestAppContext;

    use super::*;

    #[gpui_kit::test]
    fn the_system_input_is_off_until_set_and_a_set_reports_a_flip(cx: &mut TestAppContext) {
        cx.update(|cx| {
            assert!(!system(cx));
            watch(&crate::state::detached_poster(), cx);
            assert!(!system(cx), "off on Linux, and until the first read elsewhere");
            assert!(set(true, cx));
            assert!(system(cx));
            assert!(!set(true, cx), "the same value is no flip");
            assert!(set(false, cx));
            assert!(!system(cx));
        });
    }
}
