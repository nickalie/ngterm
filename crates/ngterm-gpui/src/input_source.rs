//! The keyboard-layout lock's performer (upstream's `TerminalInputSource.swift`): while a terminal holds the keyboard
//! it selects the layout Settings ▸ Key Mapping ▸ "Lock in terminals" names and switches back any change made there;
//! the layout selected before comes back when the keyboard goes to a web overlay, a text field, another application or
//! no ngterm window, when the setting goes off, and on quit. [`InputSourceLock`] decides; this is the only code that
//! selects a layout.
//!
//! It re-evaluates on the next main-loop turn after a terminal gains or loses its window's keyboard or its window's
//! activation (`crate::terminal`'s focus listeners: main, split, scratch, overlay and quick terminals alike), after any
//! ngterm window closes (a main window or the quick terminal's: a removed window reports neither a blur nor a
//! deactivation to the terminals it held), after GPUI's `on_keyboard_layout_change` (on Linux a group change that
//! changes the effective XKB layout's name, and every new keymap, X11's `XkbNewKeyboardNotify` and `XkbMapNotify` or
//! Wayland's `wl_keyboard.keymap`, the way a changed layout list arrives, reported even when the effective layout keeps
//! its name by `gpui-pre-linux`'s patch 0004; on macOS a change of the selected input source; on Windows
//! `WM_INPUTLANGCHANGE`), and at once when the setting changes. The selectable layouts are read at each evaluation, so
//! a layout no longer enabled releases the lock, and re-read for the picker when Settings opens and when an ngterm
//! window is activated (enabling one is a trip to the system's settings and back).
//!
//! Selection is the platform's, so its restore is what keeps other applications on the user's own layout; a crash
//! runs none and leaves the locked layout selected, and the baseline is never persisted (it would overwrite a choice
//! made elsewhere at the next launch). Per platform, the layout ids and the backends:
//!
//! - macOS ([`macos`]): Text Input Source Services, the stored id the input source id (`TISCreateInputSourceList`
//!   over enabled, select-capable keyboard sources, `TISCopyCurrentKeyboardInputSource`, `TISSelectInputSource`).
//! - Linux ([`linux`]), the stored id the XKB layout name (what GPUI reports as the layout): X11, any window manager,
//!   through XKB's group lock on the display GPUI runs on; on Wayland per compositor, as no common protocol exists:
//!   sway's IPC, Hyprland's, niri's, KDE's `org.kde.keyboard` D-Bus service; on GNOME (no supported API), weston and
//!   any other the setting is disabled with the reason shown.
//! - Windows ([`windows`]): the installed layouts (`GetKeyboardLayoutList`), the stored id the KLID,
//!   `ActivateKeyboardLayout` for the process on the UI thread.
//!
//! The Windows form is built and tested on the Windows runner (`cross-platform.yml`); the macOS form is not built on
//! macOS by CI yet.

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

use std::collections::HashSet;
use std::error::Error;

use gpui_kit::{App, Global, Subscription};
use ngterm_core::input_source_lock::InputSourceLock;

use crate::state::{self, AppState, Poster};

/// A backend's failure.
pub type Failure = Box<dyn Error + Send + Sync>;

/// A layout the system can select, as the picker lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyboardLayout {
    /// The platform's id, the one the setting stores.
    pub id: String,
    /// Its name for the picker.
    pub name: String,
}

impl KeyboardLayout {
    /// A layout whose name is its id (an XKB layout name is both).
    #[must_use]
    pub fn named(name: impl Into<String>) -> Self {
        let name = name.into();
        Self { id: name.clone(), name }
    }
}

/// What the picker offers: the selectable layouts, or why there are none to lock to here.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Layouts {
    pub selectable: Vec<KeyboardLayout>,
    /// Why the setting cannot work here (no backend for this desktop); the picker is disabled with it.
    pub unavailable: Option<String>,
}

/// The platform's keyboard layouts: the selectable ones, the one selected, and selection.
pub trait Backend {
    /// The layouts the system can select now.
    ///
    /// # Errors
    /// When the system cannot be asked.
    fn layouts(&mut self) -> Result<Vec<KeyboardLayout>, Failure>;
    /// The selected layout's id.
    ///
    /// # Errors
    /// When the system cannot be asked.
    fn current(&mut self) -> Result<String, Failure>;
    /// Selects the layout with `id`.
    ///
    /// # Errors
    /// When the system refused it.
    fn select(&mut self, id: &str) -> Result<(), Failure>;
}

/// The backend this platform and desktop has, probed on first use; or why there is none.
pub type Probe = Result<Box<dyn Backend>, String>;

/// Where the backend comes from: probed on first use, so a launch with the setting off talks to nothing.
enum Source {
    Pending(Box<dyn FnOnce() -> Probe>),
    Probed(Probe),
}

/// The performer, one per app.
struct Performer {
    source: Option<Source>,
    lock: InputSourceLock,
    target: Option<String>,
    scheduled: bool,
    stopped: bool,
    layouts: Layouts,
    /// The picker's list was read (Settings opened): from then on a window's activation re-reads it.
    listed: bool,
    /// Where a changed picker list goes: a settle, so the open Settings popup shows it.
    poster: Option<Poster>,
    _layout_change: Subscription,
    _window_closed: Subscription,
}

impl Global for Performer {}

impl Performer {
    /// The backend, probed now if it was not yet.
    fn backend(&mut self) -> Result<&mut Box<dyn Backend>, &str> {
        if let Some(Source::Pending(_)) = self.source
            && let Some(Source::Pending(probe)) = self.source.take()
        {
            let probed = probe();
            if let Err(reason) = &probed {
                tracing::info!(%reason, "keyboard layout lock: no backend");
            }
            self.source = Some(Source::Probed(probed));
        }
        match &mut self.source {
            Some(Source::Probed(Ok(backend))) => Ok(backend),
            Some(Source::Probed(Err(reason))) => Err(reason.as_str()),
            _ => Err("no backend"),
        }
    }

    /// Re-reads the picker's list; whether it changed.
    fn reload(&mut self) -> bool {
        let layouts = match self.backend() {
            Ok(backend) => Layouts {
                selectable: backend.layouts().unwrap_or_else(|error| {
                    tracing::debug!(%error, "keyboard layout lock: the layouts were not read");
                    Vec::new()
                }),
                unavailable: None,
            },
            Err(reason) => Layouts {
                selectable: Vec::new(),
                unavailable: Some(reason.to_owned()),
            },
        };
        let changed = layouts != self.layouts;
        self.layouts = layouts;
        changed
    }

    /// Brings the selection in line with the lock, a terminal holding the keyboard or not (`held`).
    fn reconcile(&mut self, held: bool) {
        if self.stopped || (self.target.is_none() && self.lock.baseline().is_none()) {
            return;
        }
        let target = self.target.clone();
        let Ok(backend) = self.backend() else {
            return;
        };
        let read = backend.current().and_then(|current| Ok((current, backend.layouts()?)));
        let (current, layouts) = match read {
            Ok(read) => read,
            Err(error) => {
                tracing::debug!(%error, "keyboard layout lock: the layouts were not read");
                return;
            }
        };
        let enabled: HashSet<String> = layouts.into_iter().map(|layout| layout.id).collect();
        let Some(wanted) = self.lock.reconcile(held, target.as_deref(), &enabled, &current) else {
            return;
        };
        self.select(&wanted);
    }

    /// Selects `id`, dropping the override when the system refuses it.
    fn select(&mut self, id: &str) {
        let Ok(backend) = self.backend() else {
            return;
        };
        if let Err(error) = backend.select(id) {
            tracing::warn!(%error, layout = id, "keyboard layout lock: the layout was not selected");
            self.lock.selection_failed();
        }
    }

    /// Ends the lock for good, the layout selected before restored.
    fn stop(&mut self) {
        self.stopped = true;
        if self.lock.baseline().is_none() {
            return;
        }
        let Ok(backend) = self.backend() else {
            return;
        };
        let current = backend.current();
        if let Some(restore) = current.ok().and_then(|current| self.lock.release(&current)) {
            self.select(&restore);
        }
    }
}

/// Installs the performer over this platform's backend, probed on first use; a changed list reaches the Settings popup
/// through `poster`.
pub fn init(poster: Poster, cx: &mut App) {
    let compositor = cx.compositor_name();
    install(Box::new(move || probe(compositor)), Some(poster), cx);
}

/// Installs the performer over the backend `probe` makes on first use (a test's fake).
pub fn install(probe: Box<dyn FnOnce() -> Probe>, poster: Option<Poster>, cx: &mut App) {
    let layout_change = cx.on_keyboard_layout_change(request_refresh);
    // a removed window drops its terminals unseen by their focus listeners: the one holding the keyboard may be gone
    let window_closed = cx.on_window_closed(|cx, _| request_refresh(cx));
    cx.set_global(Performer {
        source: Some(Source::Pending(probe)),
        lock: InputSourceLock::new(),
        target: None,
        scheduled: false,
        stopped: false,
        layouts: Layouts::default(),
        listed: false,
        poster,
        _layout_change: layout_change,
        _window_closed: window_closed,
    });
}

/// This platform's backend on the display server GPUI runs on (`compositor`, GPUI's name for it).
#[cfg_attr(
    any(target_os = "macos", windows),
    expect(
        clippy::unnecessary_wraps,
        reason = "Linux's probe can fail; the macOS and Windows ones cannot"
    )
)]
fn probe(compositor: &'static str) -> Probe {
    #[cfg(target_os = "linux")]
    return linux::probe(compositor);
    #[cfg(target_os = "macos")]
    return {
        let _ = compositor;
        Ok(Box::new(macos::InputSources))
    };
    #[cfg(windows)]
    return {
        let _ = compositor;
        Ok(Box::new(windows::KeyboardLayouts))
    };
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = compositor;
        Err("Not available on this system.".to_owned())
    }
}

/// Re-evaluates on the next main-loop turn: a focus listener runs before every view has seen the move, and a window's
/// activation and another's deactivation come as two events.
pub fn request_refresh(cx: &mut App) {
    let Some(performer) = cx.try_global::<Performer>() else {
        return;
    };
    if performer.scheduled || performer.stopped {
        return;
    }
    cx.global_mut::<Performer>().scheduled = true;
    cx.spawn(async move |cx| cx.update(refresh)).detach();
}

/// Re-evaluates now.
fn refresh(cx: &mut App) {
    let Some(performer) = cx.try_global::<Performer>() else {
        return;
    };
    let idle = performer.target.is_none() && performer.lock.baseline().is_none();
    let held = !idle && crate::terminal::terminal_holds_keyboard(cx);
    let performer = cx.global_mut::<Performer>();
    performer.scheduled = false;
    performer.reconcile(held);
}

/// The setting's layout, `None` for off: applied to the keyboard's holder at once when it changed.
pub fn set_target(target: Option<String>, cx: &mut App) {
    let Some(performer) = cx.try_global::<Performer>() else {
        return;
    };
    if performer.target == target {
        return;
    }
    cx.global_mut::<Performer>().target = target;
    refresh(cx);
}

/// Restores the layout selected before and ends the lock, for the quit.
pub fn stop(cx: &mut App) {
    if cx.has_global::<Performer>() {
        cx.global_mut::<Performer>().stop();
    }
}

/// What the picker offers, as last read.
#[must_use]
pub fn layouts(cx: &App) -> Layouts {
    cx.try_global::<Performer>()
        .map(|performer| performer.layouts.clone())
        .unwrap_or_default()
}

/// Re-reads what the picker offers now (Settings opens).
pub fn reload_layouts(cx: &mut App) {
    if cx.has_global::<Performer>() {
        let performer = cx.global_mut::<Performer>();
        performer.listed = true;
        performer.reload();
    }
}

/// An ngterm window was activated: the picker's list, once Settings read it, is re-read on the next turn, and an open
/// Settings popup shows a change.
pub fn window_activated(cx: &mut App) {
    if !cx.try_global::<Performer>().is_some_and(|performer| performer.listed) {
        return;
    }
    cx.spawn(async move |cx| {
        cx.update(|cx| {
            let performer = cx.global_mut::<Performer>();
            if performer.reload()
                && let Some(poster) = &performer.poster
            {
                state::post(poster, AppState::settle);
            }
        });
    })
    .detach();
}

#[cfg(test)]
mod tests;
