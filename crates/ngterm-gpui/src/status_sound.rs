//! Status sounds and the display's alert bell (upstream's `StatusSoundPlayer.swift`; port of the GTK build's
//! `status_sound.rs`). The core queues every sound this machine plays, its own `session.status` ones and those an
//! origin handed over while this machine presents its session, already resolved to a known name or `default`.
//!
//! - Linux has none of the named macOS system sounds upstream plays, so every name, `default` and `beep` included,
//!   sounds the display's alert bell, as the GTK build's `gdk::Display::beep` did; the names are still validated
//!   against upstream's list by the core. Upstream throttles each clip on its own name ([`SoundThrottle`]); here every
//!   name rings the same bell, so the throttle keys on the bell itself, and a burst of `session.status --sound` calls,
//!   whatever names they give, rings it at most once per window.
//! - macOS plays the system sound of that name (`NSSound soundNamed:`, upstream's resolver), and the alert sound
//!   (`NSBeep`) for `default`, `beep` and a name it lacks; the throttle keys on the name, as upstream's. As upstream's
//!   player, nothing of it runs on the main thread, where the first `NSSound` playback of a process stalled keystroke
//!   delivery in every session for about 0.9 s (upstream #575): a name is resolved on a serial queue of its own, the
//!   resolved sounds are kept for the app's lifetime, and every clip is stopped and restarted on a second serial queue,
//!   so a slow lookup never holds up a cached sound and a busy playback never holds up a lookup ([`player`]).
//! - Windows plays the name as a system sound alias (`PlaySoundW` with `SND_ALIAS`), and the alert sound
//!   (`MessageBeep(MB_OK)`) for `default`, `beep` and a name with no alias; the throttle keys on the name. The alias
//!   lookup and the file it names are read inside `PlaySoundW`, so it runs on a serial queue of its own too, off the
//!   main thread.
//!
//! The Windows form is built and tested on the Windows runner (`cross-platform.yml`); the macOS form is not built on
//! macOS by CI yet.
//!
//! The alert sound for a refusal with nothing on screen to explain it ([`alert`], upstream's `NSSound.beep()`): the
//! display's alert bell on Linux, `NSBeep` on macOS and `MessageBeep(MB_OK)` on Windows.
//!
//! The bell, as GDK rings it: on Wayland the `xdg_system_bell_v1` global on GPUI's own connection, rung for the window
//! (a compositor without it has no bell to ring); on X11 the core protocol's `Bell` on a connection of its own to the
//! same display. A terminal's BEL rings nothing: upstream's `GHOSTTY_ACTION_RING_BELL` is not a notification source
//! and neither upstream nor the GTK build sounds it.

use std::cell::RefCell;
#[cfg(target_os = "linux")]
use std::error::Error;
use std::time::Instant;

use gpui_kit::{App, Window};
use ngterm_core::sound_throttle::SoundThrottle;
#[cfg(target_os = "linux")]
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};

#[cfg(target_os = "linux")]
type Failure = Box<dyn Error + Send + Sync>;

/// The one throttle key on Linux: every status sound is the same bell.
const BELL: &str = "bell";
/// Whether this OS plays named sounds rather than one bell.
const NAMED_SOUNDS: bool = cfg!(any(target_os = "macos", windows));

thread_local! {
    static THROTTLE: RefCell<SoundThrottle> = RefCell::new(SoundThrottle::default());
}

/// Whether status sound `name` may play at `now`: on Linux any status sound within the throttle window of the last ring
/// is suppressed, whatever its name; on macOS and Windows a repeat of the same name, as upstream's.
#[must_use]
pub fn allow(throttle: &mut SoundThrottle, name: &str, now: Instant) -> bool {
    throttle.allow(if NAMED_SOUNDS { name } else { BELL }, now)
}

/// Plays status sound `name` unless the throttle suppresses it: on Linux through `ring` (the display's bell), on
/// macOS and Windows as the system sound of that name.
pub fn play(name: &str, ring: impl FnOnce()) {
    if THROTTLE.with(|throttle| allow(&mut throttle.borrow_mut(), name, Instant::now())) {
        sound(name, ring);
    } else {
        tracing::debug!(name, "status sound throttled");
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
fn sound(_name: &str, ring: impl FnOnce()) {
    ring();
}

#[cfg(any(target_os = "macos", windows))]
fn sound(name: &str, _ring: impl FnOnce()) {
    system::play(name);
}

/// Whether `name` is the alert sound rather than a named one.
#[cfg(any(target_os = "macos", windows, test))]
fn is_alert(name: &str) -> bool {
    name == "default" || name == "beep"
}

/// A serial queue (upstream's serial `DispatchQueue`): a thread of its own, started with the queue, runs its jobs one at
/// a time in the order they came.
#[cfg(any(target_os = "macos", windows, test))]
mod queue {
    use std::sync::mpsc;

    type Job = Box<dyn FnOnce() + Send>;

    pub(super) struct Queue {
        jobs: mpsc::Sender<Job>,
    }

    impl Queue {
        /// Starts the queue's thread, named `name`; a thread that does not start leaves every job dropped, logged.
        pub(super) fn new(name: &str) -> Self {
            let (jobs, queued) = mpsc::channel::<Job>();
            let spawned = std::thread::Builder::new().name(name.to_owned()).spawn(move || {
                for job in queued {
                    job();
                }
            });
            if let Err(error) = spawned {
                tracing::warn!(%error, name, "the status sound thread did not start");
            }
            Self { jobs }
        }

        /// Runs `job` on the queue's thread after every job queued before it.
        pub(super) fn run(&self, job: impl FnOnce() + Send + 'static) {
            if self.jobs.send(Box::new(job)).is_err() {
                tracing::debug!("no status sound thread to play on");
            }
        }
    }
}

/// Upstream's `StatusSoundPlayer` over any sound backend: the alert names go straight to the playback queue, a name
/// already resolved too, and any other name is resolved on the resolution queue, kept, and then played; a name that
/// does not resolve plays the alert.
#[cfg(any(target_os = "macos", test))]
mod player {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, PoisonError};

    use super::queue::Queue;

    /// What the player drives: resolving a name, (re)starting a resolved clip, and the alert sound. Each runs on one of
    /// the player's queues, never on the thread that asks for the sound.
    pub(super) trait Clips: Send + Sync + 'static {
        type Clip: Clone + Send + 'static;
        fn resolve(&self, name: &str) -> Option<Self::Clip>;
        /// Stops `clip` if it still plays and starts it again, so a replay restarts it.
        fn restart(&self, clip: &Self::Clip);
        fn alert(&self);
    }

    type Cache<C> = Arc<Mutex<HashMap<String, <C as Clips>::Clip>>>;

    pub(super) struct Player<C: Clips> {
        clips: Arc<C>,
        cache: Cache<C>,
        resolving: Queue,
        playing: Arc<Queue>,
    }

    impl<C: Clips> Player<C> {
        pub(super) fn new(clips: C) -> Self {
            Self {
                clips: Arc::new(clips),
                cache: Arc::default(),
                resolving: Queue::new("status-sound-resolve"),
                playing: Arc::new(Queue::new("status-sound")),
            }
        }

        /// Plays `name` from the queues and returns at once.
        pub(super) fn play(&self, name: &str) {
            if super::is_alert(name) {
                let clips = Arc::clone(&self.clips);
                self.playing.run(move || clips.alert());
                return;
            }
            let cached = self
                .cache
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(name)
                .cloned();
            if let Some(clip) = cached {
                let clips = Arc::clone(&self.clips);
                self.playing.run(move || clips.restart(&clip));
                return;
            }
            let clips = Arc::clone(&self.clips);
            let cache = Arc::clone(&self.cache);
            let playing = Arc::clone(&self.playing);
            let name = name.to_owned();
            self.resolving.run(move || {
                let Some(clip) = clips.resolve(&name) else {
                    tracing::info!(name, "no such system sound here, playing the alert");
                    playing.run(move || clips.alert());
                    return;
                };
                cache
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(name, clip.clone());
                playing.run(move || clips.restart(&clip));
            });
        }
    }
}

#[cfg(target_os = "macos")]
mod system {
    use std::sync::LazyLock;

    use objc2::rc::Retained;
    use objc2_app_kit::{NSBeep, NSSound};
    use objc2_foundation::NSString;

    use super::player::{Clips, Player};

    /// The system sounds: `NSSound soundNamed:` (which also reads `~/Library/Sounds`) and `NSBeep`.
    struct System;

    #[derive(Clone)]
    struct Clip(Retained<NSSound>);

    // SAFETY: a resolved sound is handed from the resolution queue to the playback queue and only ever played and
    // stopped there, one call at a time, as upstream's player hands its `NSSound` between its two serial queues.
    unsafe impl Send for Clip {}

    impl Clips for System {
        type Clip = Clip;

        fn resolve(&self, name: &str) -> Option<Clip> {
            NSSound::soundNamed(&NSString::from_str(name)).map(Clip)
        }

        fn restart(&self, clip: &Clip) {
            // `soundNamed:` hands out one shared instance per name, whose `play` refuses while it still plays
            clip.0.stop();
            clip.0.play();
        }

        fn alert(&self) {
            NSBeep();
        }
    }

    static PLAYER: LazyLock<Player<System>> = LazyLock::new(|| Player::new(System));

    /// The system sound `name`, else the alert sound, from the player's queues.
    pub(super) fn play(name: &str) {
        PLAYER.play(name);
    }
}

#[cfg(windows)]
mod system {
    use std::sync::LazyLock;

    use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ALIAS, SND_ASYNC, SND_NODEFAULT};
    use windows_sys::Win32::System::Diagnostics::Debug::MessageBeep;
    use windows_sys::Win32::UI::WindowsAndMessaging::MB_OK;

    use super::queue::Queue;

    static QUEUE: LazyLock<Queue> = LazyLock::new(|| Queue::new("status-sound"));

    /// The system sound alias `name`, else the alert sound, from the status sound queue.
    pub(super) fn play(name: &str) {
        let name = name.to_owned();
        QUEUE.run(move || play_now(&name));
    }

    /// `PlaySoundW` stops whatever it played before, so a replay restarts the clip.
    fn play_now(name: &str) {
        if !super::is_alert(name) {
            let alias: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
            // SAFETY: a NUL-terminated wide string alive for the call, no module; `SND_ASYNC` copies what it plays.
            let played = unsafe {
                PlaySoundW(
                    alias.as_ptr(),
                    std::ptr::null_mut(),
                    SND_ALIAS | SND_ASYNC | SND_NODEFAULT,
                )
            };
            if played != 0 {
                return;
            }
            tracing::info!(name, "no such sound alias here, playing the alert");
        }
        // SAFETY: a plain call with a constant style.
        unsafe { MessageBeep(MB_OK) };
    }
}

#[cfg(test)]
thread_local! {
    /// The alerts sounded on this thread, for the window tests.
    static ALERTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The alerts [`alert`] sounded on this thread so far.
#[cfg(test)]
pub(crate) fn alerts() -> usize {
    ALERTS.with(std::cell::Cell::get)
}

/// Sounds the alert for `window` (upstream's `NSSound.beep()`): the display's alert bell on Linux, `NSBeep` on macOS,
/// `MessageBeep(MB_OK)` on Windows.
pub fn alert(window: &Window, cx: &App) {
    #[cfg(test)]
    ALERTS.with(|alerts| alerts.set(alerts.get() + 1));
    alert_sound(window, cx);
}

#[cfg(not(any(target_os = "macos", windows)))]
fn alert_sound(window: &Window, cx: &App) {
    ring(window, cx);
}

#[cfg(target_os = "macos")]
fn alert_sound(_: &Window, _: &App) {
    objc2_app_kit::NSBeep();
}

#[cfg(windows)]
fn alert_sound(_: &Window, _: &App) {
    use windows_sys::Win32::System::Diagnostics::Debug::MessageBeep;
    use windows_sys::Win32::UI::WindowsAndMessaging::MB_OK;
    // SAFETY: a plain call with a constant style.
    unsafe { MessageBeep(MB_OK) };
}

/// Rings the display's alert bell for `window`, by the display server GPUI runs on; headless, nothing. Elsewhere the
/// system's alert sound ([`alert`]'s).
#[cfg(not(target_os = "linux"))]
pub fn ring(window: &Window, cx: &App) {
    alert_sound(window, cx);
}

/// Rings the display's alert bell for `window`, by the display server GPUI runs on; headless, nothing.
#[cfg(target_os = "linux")]
pub fn ring(window: &Window, cx: &App) {
    let rung = match cx.compositor_name() {
        "Wayland" => wayland::ring(window),
        "X11" => x11::ring(),
        backend => {
            tracing::debug!(backend, "no display to sound the alert bell on");
            return;
        }
    };
    if let Err(error) = rung {
        tracing::debug!(%error, "the alert bell did not ring");
    }
}

#[cfg(target_os = "linux")]
mod wayland {
    use wayland_backend::client::{Backend, ObjectId};
    use wayland_client::globals::{GlobalListContents, registry_queue_init};
    use wayland_client::protocol::{wl_registry, wl_surface::WlSurface};
    use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
    use wayland_protocols::xdg::system_bell::v1::client::xdg_system_bell_v1::XdgSystemBellV1;

    use super::{Failure, HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle, Window};

    /// The second event queue's state: no event of these objects matters here.
    struct Queue;

    impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Queue {
        fn event(
            _: &mut Self,
            _: &wl_registry::WlRegistry,
            _: wl_registry::Event,
            _: &GlobalListContents,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<XdgSystemBellV1, ()> for Queue {
        fn event(
            _: &mut Self,
            _: &XdgSystemBellV1,
            _: <XdgSystemBellV1 as Proxy>::Event,
            (): &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    /// Rings the compositor's bell for `window` on GPUI's own Wayland connection (a second event queue over it, as
    /// `activation.rs` binds `xdg_activation_v1`).
    pub(super) fn ring(window: &Window) -> Result<(), Failure> {
        let RawDisplayHandle::Wayland(display) = HasDisplayHandle::display_handle(window)?.as_raw() else {
            return Err("not a Wayland window".into());
        };
        let RawWindowHandle::Wayland(surface) = HasWindowHandle::window_handle(window)?.as_raw() else {
            return Err("not a Wayland window".into());
        };
        // SAFETY: GPUI's live `wl_display`, which outlives this call (the window holds the connection); a backend
        // made from a foreign display does not take ownership of it and never closes it.
        let backend = unsafe { Backend::from_foreign_display(display.display.as_ptr().cast()) };
        let connection = Connection::from_backend(backend);
        let (globals, mut queue) = registry_queue_init::<Queue>(&connection)?;
        let bound = globals.bind::<XdgSystemBellV1, _, _>(&queue.handle(), 1..=1, ());
        let outcome = match bound {
            Ok(bell) => {
                let surface_ptr = surface.surface.as_ptr().cast();
                // SAFETY: the window's live `wl_surface` proxy on that same connection, alive for the whole call.
                let id = unsafe { ObjectId::from_ptr(WlSurface::interface(), surface_ptr) }?;
                let surface = WlSurface::from_id(&connection, id)?;
                bell.ring(Some(&surface));
                bell.destroy();
                Ok(())
            }
            Err(error) => Err(format!("the compositor has no system bell: {error}").into()),
        };
        queue.roundtrip(&mut Queue)?;
        globals.destroy();
        outcome
    }
}

#[cfg(target_os = "linux")]
mod x11 {
    use x11rb::connection::Connection as _;
    use x11rb::protocol::xproto::ConnectionExt as _;

    use super::Failure;

    /// Rings the X server's bell at its base volume, on a connection of its own to the display GPUI runs on.
    pub(super) fn ring() -> Result<(), Failure> {
        let (conn, _) = x11rb::connect(None)?;
        conn.bell(0)?;
        conn.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, mpsc};
    use std::thread::{self, ThreadId};
    use std::time::Duration;

    use super::*;

    #[test]
    fn different_names_share_the_one_bell_throttle() {
        let mut throttle = SoundThrottle::default();
        let t0 = Instant::now();
        assert!(allow(&mut throttle, "Ping", t0), "Ping rings");
        assert_eq!(
            allow(&mut throttle, "Hero", t0 + Duration::from_millis(10)),
            NAMED_SOUNDS,
            "Hero right after rings only where it is a sound of its own"
        );
        assert!(!allow(&mut throttle, "Ping", t0 + Duration::from_millis(10)));
        assert!(allow(&mut throttle, "Ping", t0 + Duration::from_millis(200)));
    }

    /// What the fake backend saw, and on which thread.
    #[derive(Debug, PartialEq)]
    enum Seen {
        Resolve(String),
        Restart(String),
        Alert,
    }

    /// A backend that records each call instead of making a sound, optionally held inside one name's resolution or
    /// restart until the test lets it go.
    #[derive(Default)]
    struct Fake {
        seen: Option<Mutex<mpsc::Sender<(Seen, ThreadId)>>>,
        known: Vec<&'static str>,
        hold_resolve: Option<(&'static str, Mutex<mpsc::Receiver<()>>)>,
        hold_restart: Option<(&'static str, Mutex<mpsc::Receiver<()>>)>,
    }

    impl Fake {
        fn report(&self, seen: Seen) {
            if let Some(sender) = &self.seen {
                let _ = sender.lock().unwrap().send((seen, thread::current().id()));
            }
        }

        fn hold(held: Option<&(&'static str, Mutex<mpsc::Receiver<()>>)>, name: &str) {
            if let Some((_, release)) = held.filter(|(held, _)| *held == name) {
                let _ = release.lock().unwrap().recv_timeout(Duration::from_secs(10));
            }
        }
    }

    impl player::Clips for Fake {
        type Clip = String;

        fn resolve(&self, name: &str) -> Option<String> {
            self.report(Seen::Resolve(name.to_owned()));
            Self::hold(self.hold_resolve.as_ref(), name);
            self.known.contains(&name).then(|| name.to_owned())
        }

        fn restart(&self, clip: &String) {
            self.report(Seen::Restart(clip.clone()));
            Self::hold(self.hold_restart.as_ref(), clip);
        }

        fn alert(&self) {
            self.report(Seen::Alert);
        }
    }

    fn next(seen: &mpsc::Receiver<(Seen, ThreadId)>) -> (Seen, ThreadId) {
        seen.recv_timeout(Duration::from_secs(5))
            .expect("the backend was called")
    }

    #[test]
    fn a_clip_is_resolved_and_restarted_off_the_calling_thread() {
        let (sender, seen) = mpsc::channel();
        let player = player::Player::new(Fake {
            seen: Some(Mutex::new(sender)),
            known: vec!["Glass"],
            ..Fake::default()
        });
        let here = thread::current().id();
        player.play("Glass");
        let (lookup, resolver) = next(&seen);
        let (restarted, playback) = next(&seen);
        assert_eq!(lookup, Seen::Resolve("Glass".to_owned()));
        assert_eq!(restarted, Seen::Restart("Glass".to_owned()));
        assert_ne!(resolver, here, "the lookup leaves the calling thread");
        assert_ne!(playback, here, "the playback leaves the calling thread");
        assert_ne!(resolver, playback, "lookup and playback have queues of their own");
        player.play("Glass");
        assert_eq!(
            next(&seen),
            (Seen::Restart("Glass".to_owned()), playback),
            "a kept clip is restarted on the playback queue without another lookup"
        );
    }

    #[test]
    fn an_unknown_name_and_the_alert_names_play_the_alert() {
        let (sender, seen) = mpsc::channel();
        let player = player::Player::new(Fake {
            seen: Some(Mutex::new(sender)),
            ..Fake::default()
        });
        let here = thread::current().id();
        for name in ["default", "beep"] {
            player.play(name);
            let (alert, thread) = next(&seen);
            assert_eq!(alert, Seen::Alert, "{name} plays the alert without a lookup");
            assert_ne!(thread, here);
        }
        player.play("NoSuchSound");
        assert_eq!(next(&seen).0, Seen::Resolve("NoSuchSound".to_owned()));
        assert_eq!(
            next(&seen).0,
            Seen::Alert,
            "a name that does not resolve plays the alert"
        );
    }

    #[test]
    fn a_kept_clip_does_not_wait_for_a_held_lookup() {
        let (sender, seen) = mpsc::channel();
        let (release, held) = mpsc::channel();
        let player = player::Player::new(Fake {
            seen: Some(Mutex::new(sender)),
            known: vec!["Glass", "Hero"],
            hold_resolve: Some(("Hero", Mutex::new(held))),
            ..Fake::default()
        });
        player.play("Glass");
        next(&seen);
        next(&seen);
        player.play("Hero");
        assert_eq!(next(&seen).0, Seen::Resolve("Hero".to_owned()));
        player.play("Glass");
        assert_eq!(
            next(&seen).0,
            Seen::Restart("Glass".to_owned()),
            "the kept clip plays while the lookup is held"
        );
        release.send(()).unwrap();
        assert_eq!(next(&seen).0, Seen::Restart("Hero".to_owned()));
    }

    #[test]
    fn a_lookup_does_not_wait_for_a_busy_playback() {
        let (sender, seen) = mpsc::channel();
        let (release, held) = mpsc::channel();
        let player = player::Player::new(Fake {
            seen: Some(Mutex::new(sender)),
            known: vec!["Glass", "Hero"],
            hold_restart: Some(("Glass", Mutex::new(held))),
            ..Fake::default()
        });
        player.play("Glass");
        assert_eq!(next(&seen).0, Seen::Resolve("Glass".to_owned()));
        assert_eq!(next(&seen).0, Seen::Restart("Glass".to_owned()));
        player.play("Hero");
        assert_eq!(
            next(&seen).0,
            Seen::Resolve("Hero".to_owned()),
            "the lookup runs while the playback is held"
        );
        release.send(()).unwrap();
        assert_eq!(next(&seen).0, Seen::Restart("Hero".to_owned()));
    }

    #[test]
    #[cfg(not(any(target_os = "macos", windows)))]
    fn a_burst_rings_the_bell_once() {
        let mut rung = 0;
        play("Ping", || rung += 1);
        play("Hero", || rung += 1);
        play("default", || rung += 1);
        assert_eq!(rung, 1, "one bell for the burst, whatever the names");
    }
}
