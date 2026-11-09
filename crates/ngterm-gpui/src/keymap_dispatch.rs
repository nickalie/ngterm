//! The keymap's key monitor and the terminal's keys (upstream's `CustomCommandRunner` local key monitor plus the
//! menu's key equivalents; port of `agterm-gtk/src/keymap_dispatch.rs`).
//!
//! One GPUI keystroke interceptor for the whole app ([`init`]) sees every key press before GPUI's own bindings and
//! element listeners, in ngterm's resolution order (`ngterm_core::ghostty_config::binds`, Technical Details >
//! Keyboard): first the window's own keys (`window::intercept_key`: the Ctrl-Tab switcher, and the picker's, palette's
//! and search bar's fields handing their navigation keys to their panel, as the GTK build's capture-phase controllers
//! did, and an ask's editor handing its dialog keys to its dialog, `crate::ask_dialog::intercept_key`), then the keymap
//! in a registered window ([`register_window`]) — custom commands, the `map` alternatives, leaders timing out after
//! 1.5 s, a `--repeat` tail live until 0.5 s after its release, then the built-in menu chords (except
//! `toggle_fullscreen`'s after a half-typed leader, and `undo_close`'s while an HTML page holds the keyboard, which
//! keeps it for its own undo: upstream's 00fc83d) — then, when a terminal's own view holds the keyboard,
//! ghostty's binds and the key encoder (`crate::terminal`), or when an HTML page holds it, the page's engine
//! (`crate::html_overlay`). A key either takes stops there: GPUI's binding matcher and
//! pending-keystroke buffering never see it, and the platform's input handler does not type it again. A focused text
//! field (an element in an `Input` key context), an open prompt, an alert (`crate::modal`) or the Settings popup
//! (`crate::settings`) keeps every key, as the GTK build's text fields and dialogs did.
//!
//! A chord is the key's physical identity resolved per layout (`terminal::input::chord`). A consumed key's next press
//! reads as its autorepeat until its release, which reaches [`key_up`] from the window's (and the terminal's) key-up
//! listener, or is stood in for right after the press when a web view holds the key (`release_dispatched`). A page's
//! queued report keeps the physical identity and autorepeat flag its callback saw, for its synchronous dispatch
//! (`dispatch_page_key`): the native event current by then may be a later autorepeat of the same key.
//! [`keyboard_left`] forgets ownership when no ngterm window holds the keyboard any more. The leader and repeat
//! timeouts are deadlines checked at the next key, which is when they matter. Matched chords fire as main-loop tasks
//! (`AppState::keymap_fired`); a custom command carries the terminal typed in, read now (its selection included), as
//! the keyboard may move before the task runs.
//!
//! | Page key | Monitor behavior |
//! |---|---|
//! | Fresh bound chord | Consumed and fired once |
//! | Owned autorepeat, even when several callbacks precede dispatch | Consumed without firing again |
//! | Owned repeat tail | Fires again and suspends its repeat timeout while held |
//! | Owned key repeating after focus moves to a terminal | Consumed under the same physical identity |
//! | Report queued ahead of a press the page keeps | Acts first; the press then ends the sequence and stays the page's |
//! | Leader tail asked about while its prefix is still queued | Taken, and fires when the pump reaches it |
//! | Press queued ahead of its own autorepeat | Fires once, and the autorepeat is consumed |
//! | Unowned autorepeat | Passed without advancing the matcher or the typed-key cutoff |
//! | Character typed after a leader, reported after an in-flight autorepeat | Ends the leader |
//!
//! A page's callback runs ahead of the reports still waiting for the pump, in the order the keys came: it answers for
//! the matcher as those reports will leave it, and a press the page keeps ends the sequence only behind them
//! ([`QueuedReport`]).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use gpui_kit::{AnyWindowHandle, App, Global, KeyUpEvent, KeystrokeEvent, Subscription, Window};
use ngterm_core::Core;
use ngterm_core::ghostty_config::binds::KeyAction;
use ngterm_core::keymap::{BuiltinAction, Chord, CustomCommand, CustomCommandEngine, EngineOutcome};
use uuid::Uuid;

use crate::state::{self, AppState, Poster};
use crate::terminal::input::{self, KeyStroke};

/// How long a half-typed leader waits for its next chord.
const LEADER_TIMEOUT: Duration = Duration::from_millis(1500);
/// How long a `--repeat` sequence's prefix stays live after its tail is released (tmux's `repeat-time`).
const REPEAT_TIMEOUT: Duration = Duration::from_millis(500);

/// Where a key monitor sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyOrigin {
    /// An ngterm window.
    Window(Uuid),
    /// The quick terminal's panel.
    Quick,
}

/// What a chord fires, decided synchronously so the key is consumed or passed on at once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fired {
    Command(CustomCommand),
    Builtin(BuiltinAction),
}

/// The matcher state shared by every window (one keyboard).
#[derive(Default)]
struct Router {
    engine: Option<CustomCommandEngine>,
    /// The resolved menu chord of each built-in.
    menu: HashMap<Chord, BuiltinAction>,
    generation: Option<u64>,
    /// Keys whose press was consumed: their autorepeats and release stay consumed.
    consumed: HashSet<u32>,
    /// Consumed keys already released, until they are pressed again: a second key-up listener (the window's and the
    /// terminal's both report a release) still learns the release was the keymap's. A press a web view handed over
    /// waits here too, so an autorepeat that overtakes its queued report is the keymap's ([`takes`]).
    released: HashSet<u32>,
    /// The fired `--repeat` tail still held down.
    held_repeat: Option<u32>,
    /// The tail whose release last opened a repeat window, meaningful only while the matcher is repeating: an
    /// autorepeat of that key says the release was a web view's stand-in and it is held after all
    /// ([`Router::resume_held`]).
    released_tail: Option<u32>,
    /// When the armed leader or the open repeat window lapses.
    deadline: Option<Instant>,
    poster: Option<Poster>,
    /// The windows carrying the monitor.
    windows: Vec<(AnyWindowHandle, KeyOrigin)>,
    /// What fired, in order, for the unit tests.
    #[cfg(test)]
    fired: Vec<Fired>,
    /// The key the monitor is handling, under the identity it holds that key by: what a key dispatched for a web view
    /// lets go of ([`release_dispatched`]).
    pressed: Option<u32>,
    /// The page reports the callbacks took ([`takes`]) and the pump has not reached, oldest first.
    #[cfg(all(feature = "html", any(windows, target_os = "macos")))]
    queued: std::collections::VecDeque<QueuedReport>,
    /// The page report being dispatched synchronously, taken by the interceptor before any listener runs.
    #[cfg(all(feature = "html", any(windows, target_os = "macos")))]
    dispatched: Option<KeyStroke>,
    /// When that report's callback saw the press, rather than when the pump dispatched it.
    #[cfg(all(feature = "html", any(windows, target_os = "macos")))]
    dispatched_at: Option<Instant>,
    /// The key of the page report being dispatched, when a newer callback left that key to the page: the report still
    /// acts, and leaves no ownership behind ([`Router::forget_overtaken`]).
    #[cfg(all(feature = "html", any(windows, target_os = "macos")))]
    overtaken: Option<u32>,
    /// When the last fresh press went down (an autorepeat does not count): what a page's late report of an
    /// earlier key ([`page_typed`]) reads.
    #[cfg(all(feature = "html", any(windows, target_os = "macos")))]
    last_press: Option<Instant>,
}

/// A key a page's callback took, waiting in the pump's queue behind the reports before it. Callbacks and the pump share
/// the main thread and the queue is first in, first out, so these mirror it: each is taken off when the pump reaches
/// its report ([`dispatch_page_key`]) or the report is lost ([`page_report_dropped`]).
#[cfg(all(feature = "html", any(windows, target_os = "macos")))]
struct QueuedReport {
    /// The key as its callback saw it, which the dispatch reads rather than the native event current by then.
    stroke: KeyStroke,
    /// When the callback saw the key: a typed character behind it is later even if the pump stalls.
    at: Instant,
    /// A press the page kept came behind this report: it ends a live sequence once this report has acted.
    interrupted: bool,
}

#[cfg(all(feature = "html", any(windows, target_os = "macos")))]
impl QueuedReport {
    /// What this report will do to `engine` at the pump (`Router::press`).
    fn replay(&self, engine: &mut CustomCommandEngine) {
        // an autorepeat advances nothing: at most a live tail fires again, its window still open
        if self.stroke.action != KeyAction::Repeat {
            if self.stroke.is_escape() {
                engine.reset();
            } else if let Some(chord) = input::chord(&self.stroke).filter(|chord| !is_switcher(chord)) {
                let _ = engine.advance(&chord);
            }
        }
        if self.interrupted {
            engine.reset();
        }
    }
}

thread_local! {
    static ROUTER: RefCell<Router> = RefCell::new(Router::default());
}

/// The app's one keystroke interceptor.
struct Interceptor(#[expect(dead_code, reason = "held for its drop, which removes the interceptor")] Subscription);

impl Global for Interceptor {}

/// Installs the key monitor on the app; once.
pub fn init(cx: &mut App) {
    if cx.has_global::<Interceptor>() {
        return;
    }
    let subscription = cx.intercept_keystrokes(intercept);
    cx.set_global(Interceptor(subscription));
}

/// Rebuilds the matcher and the menu chords when the keymap was loaded again (upstream's `rebuild` on
/// `.agtermKeymapChanged`).
pub fn sync(core: &Core, poster: &Poster) {
    ROUTER.with_borrow_mut(|router| {
        router.poster = Some(poster.clone());
        if router.generation == Some(core.keymap_generation()) {
            return;
        }
        router.generation = Some(core.keymap_generation());
        router.load(core.keymap());
    });
}

/// Puts window `handle` under the monitor as `origin`.
pub fn register_window(handle: AnyWindowHandle, origin: KeyOrigin) {
    ROUTER.with_borrow_mut(|router| {
        router.windows.retain(|(window, _)| *window != handle);
        router.windows.push((handle, origin));
    });
}

/// Takes window `handle` from the monitor.
pub fn unregister_window(handle: AnyWindowHandle) {
    ROUTER.with_borrow_mut(|router| router.windows.retain(|(window, _)| *window != handle));
}

/// The window of `origin`, while registered.
#[must_use]
pub fn window_of(origin: KeyOrigin) -> Option<AnyWindowHandle> {
    ROUTER.with_borrow(|router| {
        router
            .windows
            .iter()
            .find(|(_, registered)| *registered == origin)
            .map(|(handle, _)| *handle)
    })
}

/// A key went up in an ngterm window: its consumed press and an open repeat window let go. Whether the keymap took
/// the press, so its release is the keymap's too (the terminal must not report it).
pub fn key_up(event: &KeyUpEvent, _window: &mut Window, _cx: &mut App) -> bool {
    let stroke = KeyStroke::new(&event.keystroke, &input::current_source(&event.keystroke), true);
    ROUTER.with_borrow_mut(|router| router.release(stroke.identity, Instant::now()))
}

/// No ngterm window holds the keyboard: releases now land elsewhere, so the consumed keys are forgotten and a repeat
/// window closes; a half-typed leader keeps its own timeout.
pub fn keyboard_left() {
    ROUTER.with_borrow_mut(Router::keyboard_left);
}

/// Whether `active`, the window holding the keyboard, is one under the monitor.
#[must_use]
pub fn holds_keyboard(active: Option<AnyWindowHandle>) -> bool {
    active.is_some_and(|active| ROUTER.with_borrow(|router| router.windows.iter().any(|(window, _)| *window == active)))
}

/// The built-in whose menu chord `stroke` spells (the GTK build's `menu_action`): how a palette's or the search bar's
/// field recognizes its launcher's chord, which the key monitor leaves to the field.
#[must_use]
pub fn menu_action(stroke: &KeyStroke) -> Option<BuiltinAction> {
    let chord = input::chord(stroke)?;
    ROUTER.with_borrow(|router| router.menu.get(&chord).copied())
}

/// Whether the keymap or the window's own keys take `stroke`, after the reports still queued have acted. A key pressed
/// in an HTML page's native web view (macOS and Windows, `html_overlay::webview2::keys`) goes to the window only then.
/// A key taken does not advance the matcher here: its dispatch reaches the monitor, which does. A key the page keeps
/// never reaches the monitor, so this does what upstream's monitor does with it: a fresh press forgets that physical
/// key's old ownership, and unless it is a bare modifier it ends a half-typed leader or an open repeat window behind
/// those reports (at once with none queued). Esc is the keymap's only to cancel a leader. The window's own keys there
/// are Ctrl+Tab's switcher. A press taken here counts as owned from now on, as the keyboard's first autorepeat can come
/// before the press's queued report is dispatched. Every key taken here is about to be reported: the caller must post
/// it to the page's pump.
/// On Windows only the keys `WebView2` reports reach this check (`html_overlay::webview2::keys`): a typed character is
/// not one of them, and [`page_typed`] hears of it instead.
#[cfg(all(feature = "html", any(windows, target_os = "macos")))]
#[must_use]
pub fn takes(stroke: &KeyStroke) -> bool {
    ROUTER.with_borrow_mut(|router| {
        // an autorepeat's ownership follows the press, regardless of what the matcher would take now
        if stroke.action == KeyAction::Repeat {
            if !router.owns(stroke.identity) {
                return false;
            }
        } else {
            router.released.remove(&stroke.identity);
            router.consumed.remove(&stroke.identity);
            if !router.takes_fresh(stroke) {
                return false;
            }
            router.released.insert(stroke.identity);
        }
        router.queued.push_back(QueuedReport {
            stroke: stroke.clone(),
            at: Instant::now(),
            interrupted: false,
        });
        true
    })
}

/// A key went down in a Windows page at `at` (milliseconds since the epoch) that the page keeps, its page script's
/// word for it (`html_overlay::webview2::bridge`); for a typed character no key event reports it at all. An unmatched
/// press to upstream's monitor, so it ends a half-typed leader or an open repeat window. The report comes through the
/// renderer, so it can arrive after the keymap's next press; a key that went down before that press is not what the
/// press is waiting on and changes nothing, and the key stays the page's either way.
#[cfg(all(feature = "html", any(windows, target_os = "macos")))]
pub fn page_typed(at: u64) {
    let now = Instant::now();
    let clock = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
    let age = clock.map_or(0, |since| {
        u64::try_from(since.as_millis()).unwrap_or(u64::MAX).saturating_sub(at)
    });
    if let Some(pressed) = now.checked_sub(Duration::from_millis(age)) {
        ROUTER.with_borrow_mut(|router| router.page_typed(pressed));
    }
}

/// Whether the app takes `action`'s menu chord: full screen yields after a half-typed leader, and `undo_close`
/// yields while an HTML page holds the keyboard (upstream's `UndoCloseShortcut.yieldsChord`, 00fc83d), so the chord
/// undoes an edit in the page's form field rather than a pending session close. With the terminal, the sidebar or
/// the chrome focused the chord still undoes the close, even while a page covers the session.
fn menu_takes(action: BuiltinAction, was_armed: bool, page_keys: bool) -> bool {
    !(was_armed && action == BuiltinAction::ToggleFullscreen || page_keys && action == BuiltinAction::UndoClose)
}

/// A Windows page's key, named as its accelerator reports it, with the physical identity and repeat flag carried
/// by that same report. The identity also names native events after focus moves to a terminal.
#[cfg(all(feature = "html", windows))]
#[must_use]
pub fn page_stroke(keystroke: &gpui_kit::Keystroke, keycode: u32, repeat: bool) -> KeyStroke {
    let mut stroke = KeyStroke::new(keystroke, &input::Source::Keystroke, false);
    stroke.keycode = keycode;
    if keycode != 0 {
        stroke.identity = keycode;
    }
    if repeat {
        stroke.action = KeyAction::Repeat;
    }
    stroke
}

/// Dispatches the oldest queued page report. Its metadata lives only across this synchronous dispatch, so another
/// callback or a missing window cannot overwrite a later press's repeat flag or physical identity: Windows carries it
/// on the report, macOS's report names only the keystroke and the queue keeps what its callback saw.
#[cfg(all(feature = "html", any(windows, target_os = "macos")))]
pub fn dispatch_page_key(
    keystroke: &gpui_kit::Keystroke,
    #[cfg(windows)] keycode: u32,
    #[cfg(windows)] repeat: bool,
    window: &mut Window,
    cx: &mut App,
) {
    #[cfg(windows)]
    let carried = Some(page_stroke(keystroke, keycode, repeat));
    #[cfg(not(windows))]
    let carried = None;
    let interrupted = ROUTER.with_borrow_mut(|router| router.begin_report(carried));
    window.dispatch_keystroke(keystroke.clone(), cx);
    ROUTER.with_borrow_mut(|router| router.end_report(interrupted));
}

/// The oldest queued page report found nowhere to go: the queue moves on without it.
#[cfg(all(feature = "html", any(windows, target_os = "macos")))]
pub fn page_report_dropped() {
    ROUTER.with_borrow_mut(|router| {
        let interrupted = router.begin_report(None);
        router.end_report(interrupted);
    });
}

/// Lets go of a press dispatched for an HTML page's native web view (`takes`), whose release the window never sees.
/// Called right after the press's synchronous dispatch, so the key is the one the monitor just handled, under the
/// identity it holds it by, however the platform named it. Standing in for the real release, this opens a `--repeat`
/// window; an autorepeat arriving later says the key is still down ([`Router::resume_held`]).
#[cfg(all(feature = "html", any(windows, target_os = "macos")))]
pub fn release_dispatched() {
    ROUTER.with_borrow_mut(|router| {
        if let Some(identity) = router.pressed.take() {
            router.release(identity, Instant::now());
        }
    });
}

/// What fired since the last call (tests).
#[cfg(test)]
pub(crate) fn take_fired() -> Vec<Fired> {
    ROUTER.with_borrow_mut(|router| std::mem::take(&mut router.fired))
}

/// Loads a keymap into the router directly, with no core and no poster (tests).
#[cfg(test)]
pub(crate) fn load_keymap(keymap: &ngterm_core::keymap::Keymap) {
    ROUTER.with_borrow_mut(|router| {
        router.generation = None;
        router.load(keymap);
    });
}

/// Releases `text`, a simulated keystroke, as the window's key-up listener would; whether the press was consumed
/// (tests).
#[cfg(test)]
pub(crate) fn release_simulated(text: &str) -> bool {
    let keystroke = gpui_kit::Keystroke::parse(text).unwrap().with_simulated_ime();
    let stroke = KeyStroke::new(&keystroke, &input::current_source(&keystroke), true);
    ROUTER.with_borrow_mut(|router| router.release(stroke.identity, Instant::now()))
}

/// Presses `text`, a simulated keystroke, as a web view's dispatch does, its release stood in for at once: `None` if
/// it passed, else whether it fired (tests).
#[cfg(test)]
#[cfg(all(feature = "html", windows))]
pub(crate) fn press_dispatched(text: &str) -> Option<bool> {
    let keystroke = gpui_kit::Keystroke::parse(text).unwrap().with_simulated_ime();
    if !takes(&page_stroke(&keystroke, 0, false)) {
        return None;
    }
    press_page_report(&keystroke, 0, false)
}

/// Dispatches one Windows report through the same metadata handoff as the interceptor (tests).
#[cfg(all(test, feature = "html", windows))]
pub(crate) fn press_page_report(keystroke: &gpui_kit::Keystroke, keycode: u32, repeat: bool) -> Option<bool> {
    let press = ROUTER.with_borrow_mut(|router| {
        let interrupted = router.begin_report(Some(page_stroke(keystroke, keycode, repeat)));
        let stroke = router.dispatched_stroke(KeyStroke::new(keystroke, &input::Source::Keystroke, false));
        let press = router.press(&stroke, Instant::now(), true);
        router.end_report(interrupted);
        press
    });
    release_dispatched();
    match press {
        Press::Passed => None,
        Press::Consumed => Some(false),
        Press::Fired(_) => Some(true),
    }
}

/// Whether a half-typed leader or an open repeat window is live (tests).
#[cfg(test)]
#[cfg(all(feature = "html", windows))]
pub(crate) fn sequence_is_live() -> bool {
    ROUTER.with_borrow(|router| {
        router
            .engine
            .as_ref()
            .is_some_and(|engine| engine.is_armed() || engine.is_repeating())
    })
}

/// The interceptor (module docs).
fn intercept(event: &KeystrokeEvent, window: &mut Window, cx: &mut App) {
    ROUTER.with_borrow_mut(|router| router.pressed = None);
    // GPUI's keystroke for a lone modifier pressed and released: the modifier change it stands for goes on to the
    // listeners (a link under the pointer goes dark), never to the keymap or the terminal
    if is_modifier_release(&event.keystroke) {
        return;
    }
    let source = input::current_source(&event.keystroke);
    let stroke = KeyStroke::new(&event.keystroke, &source, false);
    #[cfg(all(feature = "html", any(windows, target_os = "macos")))]
    let stroke = ROUTER.with_borrow_mut(|router| router.dispatched_stroke(stroke));
    // the window's own keys first: Ctrl+Tab's switcher, the picker's, palette's and search bar's fields; their release
    // is theirs too (the switcher's walk leaves the terminal the keyboard, so its key-up listener must not report it)
    if crate::window::intercept_key(&stroke, window, cx)
        || crate::ask_dialog::intercept_key(&event.keystroke, &stroke, window, cx)
    {
        ROUTER.with_borrow_mut(|router| router.consume(stroke.identity));
        cx.stop_propagation();
        return;
    }
    if keeps_keys(event) {
        ROUTER.with_borrow_mut(Router::reset_if_live);
        return;
    }
    let origin = ROUTER.with_borrow(|router| {
        router
            .windows
            .iter()
            .find(|(handle, _)| *handle == window.window_handle())
            .map(|(_, origin)| *origin)
    });
    if let Some(origin) = origin {
        let now = Instant::now();
        let page_keys = crate::html_overlay::page_holds_keyboard(window, cx);
        let fired = ROUTER.with_borrow_mut(|router| router.press(&stroke, now, page_keys));
        match fired {
            Press::Passed => {}
            Press::Consumed => {
                cx.stop_propagation();
                return;
            }
            Press::Fired(fired) => {
                cx.stop_propagation();
                fire(origin, fired, window, cx);
                return;
            }
        }
    }
    // then the terminal holding the keyboard, else the HTML page holding it (`crate::html_overlay`)
    if crate::terminal::key_down(&stroke, window, cx)
        || crate::html_overlay::key_down(&event.keystroke, &stroke, window, cx)
    {
        cx.stop_propagation();
    }
}

/// Whether `chord` is Ctrl+Tab's, the window's own switcher: never the matcher's.
#[cfg(all(feature = "html", any(windows, target_os = "macos")))]
fn is_switcher(chord: &Chord) -> bool {
    chord.key == "tab" && chord.mods.contains(ngterm_core::keymap::Modifiers::CONTROL)
}

/// Whether `keystroke` is the one GPUI synthesizes when a lone modifier is released (`shift`, `control`, `alt`,
/// `platform` or `function`, with no modifier held and no text); a modifier key itself never comes as a key on Linux.
fn is_modifier_release(keystroke: &gpui_kit::Keystroke) -> bool {
    keystroke.key_char.is_none()
        && keystroke.modifiers == gpui_kit::Modifiers::default()
        && matches!(
            keystroke.key.as_str(),
            "shift" | "control" | "alt" | "platform" | "function"
        )
}

/// Whether the focused element keeps every key: a text field (an `Input` key context), a prompt, an alert or the
/// Settings popup.
fn keeps_keys(event: &KeystrokeEvent) -> bool {
    event.context_stack.iter().any(|context| {
        [
            "Input",
            "Prompt",
            "PromptDialog",
            "NgtermAlert",
            crate::settings::KEY_CONTEXT,
        ]
        .iter()
        .any(|name| context.contains(name))
    })
}

/// Posts what fired; a custom command carries the terminal typed in, read now.
fn fire(origin: KeyOrigin, fired: Fired, window: &Window, cx: &App) {
    #[cfg(test)]
    ROUTER.with_borrow_mut(|router| router.fired.push(fired.clone()));
    let Some(poster) = ROUTER.with_borrow(|router| router.poster.clone()) else {
        return;
    };
    let typing = match &fired {
        Fired::Command(_) => crate::terminal::typing_surface(window, cx),
        Fired::Builtin(_) => None,
    };
    state::post(&poster, move |state: &mut AppState| {
        state.keymap_fired(origin, fired, typing);
    });
}

/// What the router did with a press.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Press {
    Passed,
    Consumed,
    Fired(Fired),
}

impl Router {
    fn load(&mut self, keymap: &ngterm_core::keymap::Keymap) {
        self.engine = Some(CustomCommandEngine::from_keymap(keymap));
        self.menu = BuiltinAction::all()
            .filter_map(|action| keymap.equivalent(action).map(|chord| (chord.clone(), action)))
            .collect();
        self.reset();
    }

    /// Takes the report installed immediately before the synchronous dispatch, or the native window's stroke.
    #[cfg(all(feature = "html", any(windows, target_os = "macos")))]
    fn dispatched_stroke(&mut self, fallback: KeyStroke) -> KeyStroke {
        let Some(stroke) = self.dispatched.take() else {
            return fallback;
        };
        if !self.owns(stroke.identity) {
            // a newer callback left the key to the page, or the keyboard left ngterm; the callback still took this
            // report, so an autorepeat stays consumed, for this dispatch only
            self.overtaken = Some(stroke.identity);
            if stroke.action == KeyAction::Repeat {
                self.consumed.insert(stroke.identity);
            }
        }
        stroke
    }

    /// The pump reached the oldest queued report: it is off the queue, and its stroke (`carried`, else what its callback
    /// saw) is the next dispatch's. Whether a press the page kept came behind it, for [`Router::end_report`].
    #[cfg(all(feature = "html", any(windows, target_os = "macos")))]
    fn begin_report(&mut self, carried: Option<KeyStroke>) -> bool {
        let report = self.queued.pop_front();
        let interrupted = report.as_ref().is_some_and(|report| report.interrupted);
        self.dispatched_at = report.as_ref().map(|report| report.at);
        self.dispatched = carried.or(report.map(|report| report.stroke));
        interrupted
    }

    /// The report has acted: its metadata goes, and the press the page kept behind it ends the sequence now.
    #[cfg(all(feature = "html", any(windows, target_os = "macos")))]
    fn end_report(&mut self, interrupted: bool) {
        self.dispatched = None;
        self.dispatched_at = None;
        self.forget_overtaken();
        if interrupted {
            self.reset_if_live();
        }
    }

    /// Ends the dispatch of an overtaken report: the key is the page's, as the newer callback left it.
    #[cfg(all(feature = "html", any(windows, target_os = "macos")))]
    fn forget_overtaken(&mut self) {
        if let Some(identity) = self.overtaken.take() {
            self.consumed.remove(&identity);
            self.released.remove(&identity);
        }
    }

    /// Forgets key `identity`'s release, as it is pressed again. `repeat` says the key never went up, so that
    /// release was the one a web view's dispatch reports at once ([`release_dispatched`]): the press's ownership
    /// stands. Only the live repeat tail's key suspends its window, which counts from the real release (upstream's
    /// `heldRepeatKeyCode`), whatever its modifiers are now; a window another key closed since is left as it is.
    fn resume_held(&mut self, identity: u32, repeat: bool) {
        if self.released.remove(&identity) && repeat {
            self.consumed.insert(identity);
            if self.released_tail == Some(identity)
                && self.engine.as_ref().is_some_and(CustomCommandEngine::is_repeating)
            {
                self.released_tail = None;
                self.held_repeat = Some(identity);
                self.deadline = None;
            }
        }
    }

    /// Whether the keymap took the press of the key `identity`, released or not.
    #[cfg(all(feature = "html", any(windows, target_os = "macos")))]
    fn owns(&self, identity: u32) -> bool {
        self.consumed.contains(&identity) || self.released.contains(&identity)
    }

    /// Whether a fresh press in a page is taken after the queued reports act, ending a live sequence behind them if
    /// not ([`takes`]).
    #[cfg(all(feature = "html", any(windows, target_os = "macos")))]
    fn takes_fresh(&mut self, stroke: &KeyStroke) -> bool {
        // Esc is not bindable, and as a bare keystroke it spells no chord: it is the keymap's only to cancel a leader
        let taken = if stroke.is_escape() {
            self.leader_waits()
        } else {
            // a bare modifier is the next chord being typed: the leader keeps waiting
            let Some(chord) = input::chord(stroke) else {
                return false;
            };
            if is_switcher(&chord) {
                return true;
            }
            self.page_press_taken(&chord)
        };
        if !taken {
            self.interrupt();
        }
        taken
    }

    /// A press the page keeps ends a live sequence, behind the reports still queued (module docs).
    #[cfg(all(feature = "html", any(windows, target_os = "macos")))]
    fn interrupt(&mut self) {
        match self.queued.back_mut() {
            Some(report) => report.interrupted = true,
            None => self.reset_if_live(),
        }
    }

    /// The matcher as the reports still queued will leave it, a lapsed leader or repeat window counting as gone.
    #[cfg(all(feature = "html", any(windows, target_os = "macos")))]
    fn matcher_after_queue(&self) -> Option<CustomCommandEngine> {
        let mut engine = self.engine.clone()?;
        if self.lapsed() {
            engine.reset();
        }
        for report in &self.queued {
            report.replay(&mut engine);
        }
        Some(engine)
    }

    /// Whether a half-typed leader will still be waiting for its next chord.
    #[cfg(all(feature = "html", any(windows, target_os = "macos")))]
    fn leader_waits(&self) -> bool {
        self.matcher_after_queue()
            .as_ref()
            .is_some_and(CustomCommandEngine::is_armed)
    }

    /// Whether a fresh press spelled `chord`, in a page's web view, is the keymap's: `press` decides the same way, the
    /// page holding the keyboard.
    #[cfg(all(feature = "html", any(windows, target_os = "macos")))]
    fn page_press_taken(&self, chord: &Chord) -> bool {
        let Some(mut engine) = self.matcher_after_queue() else {
            return false;
        };
        let armed = engine.is_armed();
        self.menu
            .get(chord)
            .is_some_and(|action| menu_takes(*action, armed, true))
            || !matches!(engine.advance(chord), EngineOutcome::Unmatched)
    }

    /// A key the page keeps went down at `pressed` ([`page_typed`]): after the monitor's last press, it ends the sequence.
    #[cfg(all(feature = "html", any(windows, target_os = "macos")))]
    fn page_typed(&mut self, pressed: Instant) {
        if self.last_press.is_none_or(|last| pressed >= last) {
            self.reset_if_live();
        }
    }

    /// Whether the leader or repeat window has run out (the next press lapses it for real).
    #[cfg(all(feature = "html", any(windows, target_os = "macos")))]
    fn lapsed(&self) -> bool {
        self.deadline.is_some_and(|deadline| Instant::now() >= deadline)
    }

    /// One press, with an HTML page holding the keyboard while `page_keys`: whether it is consumed, and what it fired.
    fn press(&mut self, stroke: &KeyStroke, now: Instant, page_keys: bool) -> Press {
        self.pressed = Some(stroke.identity);
        let repeat = stroke.action == KeyAction::Repeat;
        #[cfg(all(feature = "html", any(windows, target_os = "macos")))]
        if !repeat {
            self.last_press = Some(self.dispatched_at.take().unwrap_or(now));
        }
        self.resume_held(stroke.identity, repeat);
        if stroke.keycode != 0 && stroke.action == KeyAction::Press {
            // the platform says the key went up since its consumed press: a release the monitor never heard (one
            // dispatched while its window had no focused element, which GPUI routes to the root view alone) must not
            // swallow the next press as an autorepeat
            self.consumed.remove(&stroke.identity);
        }
        if self.deadline.is_some_and(|deadline| now >= deadline) {
            self.lapse();
        }
        let chord = input::chord(stroke);
        let Some(engine) = self.engine.as_mut() else {
            return Press::Passed;
        };
        // an autorepeat of a consumed press: swallowed, unless it is the live tail of a repeatable sequence
        if self.consumed.contains(&stroke.identity) {
            let tail = chord.filter(|chord| engine.is_repeating() && engine.is_repeat_tail(chord));
            let fired = tail.and_then(|chord| match engine.advance(&chord) {
                EngineOutcome::Fired(command) => Some(Fired::Command(command)),
                EngineOutcome::FiredBuiltin(action) => Some(Fired::Builtin(action)),
                EngineOutcome::Armed | EngineOutcome::Unmatched => None,
            });
            if fired.is_some() {
                self.hold_repeat_or_cancel(stroke.identity);
            }
            return fired.map_or(Press::Consumed, Press::Fired);
        }
        // an autorepeat of a press the keymap did not take is not the keymap's either (upstream's `handleKeyEvent`):
        // the matcher never sees it, so it neither fires nor ends a sequence
        if repeat {
            return Press::Passed;
        }
        if stroke.is_escape() {
            let armed = engine.is_armed();
            if armed || engine.is_repeating() {
                self.reset();
            }
            if !armed {
                return Press::Passed;
            }
            // the cancelling press is the keymap's: its autorepeats and its release stay off the terminal
            self.consumed.insert(stroke.identity);
            return Press::Consumed;
        }
        // a bare modifier is the next chord being typed: the leader keeps waiting
        let Some(chord) = chord else {
            return Press::Passed;
        };
        let outcome = self.advance(stroke.identity, &chord, now, page_keys);
        if outcome != Press::Passed {
            self.consumed.insert(stroke.identity);
        }
        outcome
    }

    fn advance(&mut self, identity: u32, chord: &Chord, now: Instant, page_keys: bool) -> Press {
        let Some(engine) = self.engine.as_mut() else {
            return Press::Passed;
        };
        let was_armed = engine.is_armed();
        match engine.advance(chord) {
            EngineOutcome::Fired(command) => {
                self.hold_repeat_or_cancel(identity);
                Press::Fired(Fired::Command(command))
            }
            EngineOutcome::FiredBuiltin(action) => {
                self.hold_repeat_or_cancel(identity);
                Press::Fired(Fired::Builtin(action))
            }
            EngineOutcome::Armed => {
                self.deadline = Some(now + LEADER_TIMEOUT);
                Press::Consumed
            }
            EngineOutcome::Unmatched => {
                self.deadline = None;
                // the menu's key equivalents: consumed even when the item is disabled, as a menu consumes its own
                let Some(action) = self.menu.get(chord).copied() else {
                    return Press::Passed;
                };
                if !menu_takes(action, was_armed, page_keys) {
                    return Press::Passed;
                }
                Press::Fired(Fired::Builtin(action))
            }
        }
    }

    /// A press taken outside the keymap (the window's own keys): its autorepeats and release stay off the terminal.
    fn consume(&mut self, identity: u32) {
        self.pressed = Some(identity);
        self.released.remove(&identity);
        self.consumed.insert(identity);
    }

    /// A key went up; whether its press was consumed.
    fn release(&mut self, identity: u32, now: Instant) -> bool {
        if self.held_repeat == Some(identity) {
            self.held_repeat = None;
            if self.engine.as_ref().is_some_and(CustomCommandEngine::is_repeating) {
                self.deadline = Some(now + REPEAT_TIMEOUT);
                self.released_tail = Some(identity);
            }
        }
        if self.consumed.remove(&identity) {
            self.released.insert(identity);
            return true;
        }
        self.released.contains(&identity)
    }

    fn hold_repeat_or_cancel(&mut self, identity: u32) {
        self.deadline = None;
        self.released_tail = None;
        let repeating = self.engine.as_ref().is_some_and(CustomCommandEngine::is_repeating);
        self.held_repeat = repeating.then_some(identity);
    }

    fn keyboard_left(&mut self) {
        self.consumed.clear();
        self.released.clear();
        self.held_repeat = None;
        if self.engine.as_ref().is_some_and(CustomCommandEngine::is_repeating) {
            self.reset();
        }
    }

    fn reset_if_live(&mut self) {
        if self
            .engine
            .as_ref()
            .is_some_and(|engine| engine.is_armed() || engine.is_repeating())
        {
            self.reset();
        }
    }

    /// The leader or repeat window lapsed (the GTK build's timer).
    fn lapse(&mut self) {
        self.deadline = None;
        if let Some(engine) = self.engine.as_mut() {
            engine.reset();
        }
        self.held_repeat = None;
    }

    fn reset(&mut self) {
        if let Some(engine) = self.engine.as_mut() {
            engine.reset();
        }
        self.held_repeat = None;
        self.deadline = None;
    }
}

#[cfg(test)]
mod tests;
