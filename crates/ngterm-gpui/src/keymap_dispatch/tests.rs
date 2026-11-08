//! The key monitor's router without a window (the GTK build's `keymap_dispatch` tests, by GPUI keystrokes): chords and
//! commands consumed, autorepeats of consumed keys, `--repeat` tails, leaders and their timeouts, the menu's chords.

use std::time::{Duration, Instant};

use gpui_kit::Keystroke;
use ngterm_core::keymap::{CustomCommandEngine, linux_defaults, parse_keymap};

use super::{Fired, Press, Router};
use crate::terminal::input::{KeyStroke, NativeKey, Source};

fn stroke(text: &str) -> KeyStroke {
    let keystroke = Keystroke::parse(text).unwrap().with_simulated_ime();
    KeyStroke::new(&keystroke, &Source::Keystroke, false)
}

fn router(keymap: &str) -> Router {
    let (keymap, _) = parse_keymap(keymap, &linux_defaults());
    let mut router = Router::default();
    router.load(&keymap);
    router
}

/// Presses `text` at `now`; whether it was consumed, recording what fired.
fn press(router: &mut Router, text: &str, now: Instant) -> bool {
    press_in(router, text, now, false)
}

/// Presses `text` at `now`, with an HTML page holding the keyboard while `page_keys`.
fn press_in(router: &mut Router, text: &str, now: Instant, page_keys: bool) -> bool {
    match router.press(&stroke(text), now, page_keys) {
        Press::Passed => false,
        Press::Consumed => true,
        Press::Fired(fired) => {
            router.fired.push(fired);
            true
        }
    }
}

fn release(router: &mut Router, text: &str, now: Instant) -> bool {
    router.release(stroke(text).identity, now)
}

fn names(router: &Router) -> Vec<String> {
    router
        .fired
        .iter()
        .map(|fired| match fired {
            Fired::Command(command) => command.name.clone(),
            Fired::Builtin(action) => action.name().to_owned(),
        })
        .collect()
}

#[test]
fn menu_chords_and_commands_are_consumed_and_others_pass() {
    let now = Instant::now();
    let mut router = router("command \"x\" ctrl+shift+b true\n");
    assert!(press(&mut router, "ctrl-shift-b", now));
    assert!(press(&mut router, "ctrl-shift-t", now));
    assert!(!press(&mut router, "a", now));
    assert_eq!(names(&router), ["x", "new_session"]);
    assert!(
        release(&mut router, "ctrl-shift-t", now),
        "the keymap owns the release of its press"
    );
    assert!(
        release(&mut router, "ctrl-shift-t", now),
        "told twice (the window's and the terminal's listener)"
    );
    assert!(!router.consumed.contains(&stroke("t").identity));
    assert!(
        !release(&mut router, "a", now),
        "a passed press's release is the terminal's"
    );
}

#[test]
fn an_autorepeat_of_a_leader_prefix_is_swallowed_without_rearming() {
    let now = Instant::now();
    let mut router = router("command \"g\" ctrl+a>g true\n");
    assert!(press(&mut router, "ctrl-a", now));
    assert!(press(&mut router, "ctrl-a", now));
    assert!(router.fired.is_empty());
    assert!(router.engine.as_ref().is_some_and(CustomCommandEngine::is_armed));
    assert!(press(&mut router, "g", now));
    assert_eq!(names(&router), ["g"]);
}

/// The S key as the platform describes it (XKB keycode 39), held with `ctrl` and `shift`, pressed fresh or repeating.
fn native_s(ctrl_shift: bool, repeat: bool) -> KeyStroke {
    let modifiers = gpui_kit::Modifiers {
        control: ctrl_shift,
        shift: ctrl_shift,
        ..gpui_kit::Modifiers::default()
    };
    let keystroke = Keystroke::parse(if ctrl_shift { "ctrl-shift-s" } else { "s" })
        .unwrap()
        .with_simulated_ime();
    let native = NativeKey {
        keycode: 39,
        modifiers,
        unshifted: Some('s'),
        ascii_layout: true,
        text: if ctrl_shift { "S" } else { "s" }.into(),
        repeat,
        ..NativeKey::default()
    };
    KeyStroke::new(&keystroke, &Source::Native(native), false)
}

#[test]
fn a_fresh_native_press_after_an_unheard_release_passes() {
    // Ctrl+Shift+S toggles the sidebar; its release reached a window with no focused element, so the monitor never
    // heard it, and the next press of S typed elsewhere must not read as its autorepeat
    let now = Instant::now();
    let mut router = router("");
    assert!(matches!(
        router.press(&native_s(true, false), now, false),
        Press::Fired(Fired::Builtin(_))
    ));
    assert_eq!(
        router.press(&native_s(true, true), now, false),
        Press::Consumed,
        "the platform's autorepeat of the chord stays the keymap's"
    );
    assert_eq!(router.press(&native_s(false, false), now, false), Press::Passed);
    assert!(
        !router.release(native_s(false, false).identity, now),
        "the passed press's release is the terminal's"
    );
}

#[test]
fn an_autorepeat_of_a_held_repeat_tail_fires_again() {
    let now = Instant::now();
    let mut router = router("command \"n\" ctrl+a>n --repeat true\n");
    assert!(press(&mut router, "ctrl-a", now));
    release(&mut router, "ctrl-a", now);
    assert!(press(&mut router, "n", now));
    assert!(press(&mut router, "n", now));
    assert_eq!(names(&router), ["n", "n"]);
    // released, the window stays open for a fresh press of the tail, until it lapses
    release(&mut router, "n", now);
    assert!(press(&mut router, "n", now + Duration::from_millis(100)));
    assert_eq!(router.fired.len(), 3);
    release(&mut router, "n", now + Duration::from_millis(100));
    assert!(
        !press(&mut router, "n", now + Duration::from_secs(1)),
        "the repeat window lapsed"
    );
}

#[test]
fn a_leader_lapses_after_its_timeout() {
    let now = Instant::now();
    let mut router = router("command \"g\" ctrl+a>g true\n");
    assert!(press(&mut router, "ctrl-a", now));
    release(&mut router, "ctrl-a", now);
    assert!(
        !press(&mut router, "g", now + Duration::from_secs(2)),
        "the leader lapsed"
    );
    assert!(router.fired.is_empty());
}

#[test]
fn a_menu_chord_ending_a_leader_still_fires_but_full_screen_does_not() {
    let now = Instant::now();
    let mut router = router("command \"g\" ctrl+a>g true\n");
    assert!(press(&mut router, "ctrl-a", now));
    release(&mut router, "ctrl-a", now);
    assert!(press(&mut router, "ctrl-shift-t", now));
    assert_eq!(names(&router), ["new_session"]);
    assert!(press(&mut router, "ctrl-a", now));
    release(&mut router, "ctrl-a", now);
    assert!(!press(&mut router, "f11", now));
    assert!(press(&mut router, "f11", now));
    assert_eq!(names(&router), ["new_session", "toggle_fullscreen"]);
}

#[test]
fn leaving_ngterm_forgets_consumed_keys_and_closes_the_repeat_window() {
    let now = Instant::now();
    let mut router = router("command \"n\" ctrl+a>n --repeat true\n");
    assert!(press(&mut router, "ctrl-shift-t", now));
    assert!(press(&mut router, "ctrl-a", now));
    assert!(press(&mut router, "n", now));
    // neither release arrives: the keyboard went to another application
    router.keyboard_left();
    assert!(press(&mut router, "ctrl-shift-t", now));
    assert_eq!(names(&router), ["new_session", "n", "new_session"]);
    assert!(!press(&mut router, "n", now));
}

#[test]
fn escape_cancels_an_armed_leader_and_is_consumed_only_then() {
    let now = Instant::now();
    let mut router = router("command \"g\" ctrl+a>g true\n");
    assert!(!press(&mut router, "escape", now));
    assert!(press(&mut router, "ctrl-a", now));
    release(&mut router, "ctrl-a", now);
    assert!(press(&mut router, "escape", now));
    assert!(press(&mut router, "escape", now), "its autorepeat is the keymap's too");
    assert!(
        release(&mut router, "escape", now),
        "the keymap owns the release of the cancelling press"
    );
    assert!(!press(&mut router, "g", now));
    assert!(
        !press(&mut router, "escape", now),
        "a fresh Escape is the terminal's again"
    );
    assert!(router.fired.is_empty());
}

#[test]
fn a_shifted_symbol_spells_its_base_key() {
    // GPUI hands `ctrl-!` for ctrl+shift+1; the keymap's chord is ctrl+shift+1
    let now = Instant::now();
    let mut router = router("command \"one\" ctrl+shift+1 true\n");
    assert!(press(&mut router, "ctrl-!", now));
    assert_eq!(names(&router), ["one"]);
    assert_eq!(
        super::input::chord(&stroke("ctrl-!")).map(|chord| chord.key),
        Some("1".to_owned())
    );
}

/// Upstream's 00fc83d (`testTextFieldAndFocusedPageKeepTheChord`, `testTerminalAndChromeFocusLeaveTheChordToUndoClose`):
/// with an HTML page holding the keyboard the `undo_close` chord passes on to the page, unconsumed, firing nothing;
/// with the terminal, the sidebar or the chrome focused it still undoes the close. Every other menu chord stays the
/// app's either way.
#[test]
fn a_page_holding_the_keyboard_keeps_the_undo_close_chord() {
    let now = Instant::now();
    let mut router = router("");
    assert!(
        !press_in(&mut router, "ctrl-shift-z", now, true),
        "the page gets the key"
    );
    assert!(router.fired.is_empty());
    assert!(
        !release(&mut router, "ctrl-shift-z", now),
        "its release is the page's too"
    );
    assert!(
        press_in(&mut router, "ctrl-shift-t", now, true),
        "other menu chords stay the app's"
    );
    assert!(press_in(&mut router, "ctrl-shift-z", now, false));
    assert_eq!(names(&router), ["new_session", "undo_close"]);
}

/// The `undo_close` chord pressed in a page's native web view (Windows, macOS) stays the page's; the app's other chords and
/// Ctrl+Tab's switcher still go to the window.
#[test]
#[cfg(all(feature = "html", any(windows, target_os = "macos")))]
fn a_web_view_keeps_the_undo_close_chord() {
    let (keymap, _) = parse_keymap("", &linux_defaults());
    super::load_keymap(&keymap);
    assert!(!super::takes(&stroke("ctrl-shift-z")));
    assert!(super::takes(&stroke("ctrl-shift-t")));
    assert!(super::takes(&stroke("ctrl-tab")));
    assert!(!super::takes(&stroke("a")));
}

/// Find's chord reaches the app action while a page holds the keyboard (upstream's 082278b: what is on top owns
/// Find, and a text page focuses its own field): the router fires it from a page, as WPE's host routes the keymap's
/// chords ahead of the page's.
#[test]
fn the_find_chord_fires_while_a_page_holds_the_keyboard() {
    let now = Instant::now();
    let mut router = router("");
    assert!(press_in(&mut router, "ctrl-shift-f", now, true));
    assert_eq!(names(&router), ["toggle_search"]);
}

/// Find's chord pressed in a page's native web view (Windows, macOS) goes to the window.
#[test]
#[cfg(all(feature = "html", any(windows, target_os = "macos")))]
fn a_web_view_hands_the_find_chord_to_the_window() {
    let (keymap, _) = parse_keymap("", &linux_defaults());
    super::load_keymap(&keymap);
    assert!(super::takes(&stroke("ctrl-shift-f")));
}

#[cfg(all(feature = "html", target_os = "macos"))]
fn dispatched(text: &str) -> Keystroke {
    Keystroke::parse(text).unwrap().with_simulated_ime()
}

/// The platform's record of `text`, a letter chord: what `NSApp.currentEvent` holds while a key a web view handed over
/// is dispatched, so the press carries the physical key's identity rather than its name's.
#[cfg(all(feature = "html", target_os = "macos"))]
fn native_source(text: &str, repeat: bool) -> Source {
    let keystroke = dispatched(text);
    let letter = keystroke.key.chars().next().unwrap();
    let code = format!("Key{}", letter.to_ascii_uppercase());
    Source::Native(NativeKey {
        keycode: crate::terminal::input::xkb_keycode(&code).unwrap(),
        modifiers: keystroke.modifiers,
        unshifted: Some(letter),
        ascii_layout: true,
        text: letter.to_string(),
        repeat,
        ..NativeKey::default()
    })
}

/// The identity the monitor holds `text` under, a key dispatched for a web view.
#[cfg(all(feature = "html", target_os = "macos"))]
fn native_identity(text: &str) -> u32 {
    KeyStroke::new(&dispatched(text), &native_source(text, false), false).identity
}

/// Presses `text` on this thread's monitor, as the dispatch of a key a web view handed over does.
#[cfg(all(feature = "html", target_os = "macos"))]
fn press_monitor(text: &str, repeat: bool, now: Instant) -> Press {
    let stroke = KeyStroke::new(&dispatched(text), &native_source(text, repeat), false);
    super::ROUTER.with_borrow_mut(|router| router.press(&stroke, now, false))
}

/// Lets go of `text` as the page does right after dispatching it, with the platform's record still the press's.
#[cfg(all(feature = "html", target_os = "macos"))]
fn release_monitor(text: &str) {
    super::ROUTER.with_borrow_mut(|router| router.dispatched_source = Some(native_source(text, false)));
    super::release_dispatched(&dispatched(text));
}

#[test]
#[cfg(all(feature = "html", target_os = "macos"))]
fn a_web_view_keeps_every_key_until_a_keymap_is_loaded_except_the_switcher() {
    assert!(!super::takes(&stroke("ctrl-shift-t")));
    assert!(!super::takes(&stroke("a")));
    assert!(super::takes(&stroke("ctrl-tab")));
    assert!(!super::takes(&stroke("tab")), "Tab alone stays the page's");
}

#[test]
#[cfg(all(feature = "html", target_os = "macos"))]
fn a_web_view_keeps_a_bare_modifier() {
    let keystroke = dispatched("shift");
    // a base the chord could resolve from, so only the key being a modifier keeps it from being one
    let native = NativeKey {
        keycode: 50,
        unshifted: Some('a'),
        ascii_layout: true,
        ..NativeKey::default()
    };
    let shift = KeyStroke::new(&keystroke, &Source::Native(native), false);
    assert_eq!(crate::terminal::input::chord(&shift), None);
    let (keymap, _) = parse_keymap("", &linux_defaults());
    super::load_keymap(&keymap);
    assert!(!super::takes(&shift));
}

#[test]
#[cfg(all(feature = "html", target_os = "macos"))]
fn a_web_view_hands_over_the_keymap_commands_and_asking_changes_nothing() {
    let (keymap, _) = parse_keymap(
        "command \"x\" ctrl+shift+b true\ncommand \"g\" ctrl+a>g true\n",
        &linux_defaults(),
    );
    super::load_keymap(&keymap);
    assert!(super::takes(&stroke("ctrl-shift-b")));
    assert!(
        super::takes(&stroke("ctrl-a")),
        "a leader's first chord is the keymap's"
    );
    assert!(!super::takes(&stroke("g")), "asking about the leader did not arm it");
    assert_eq!(press_monitor("g", false, Instant::now()), Press::Passed);
}

#[test]
#[cfg(all(feature = "html", target_os = "macos"))]
fn a_dispatched_key_is_released_for_the_monitor() {
    let (keymap, _) = parse_keymap("", &linux_defaults());
    super::load_keymap(&keymap);
    let now = Instant::now();
    assert!(matches!(
        press_monitor("ctrl-shift-t", false, now),
        Press::Fired(Fired::Builtin(_))
    ));
    assert_eq!(
        press_monitor("ctrl-shift-t", true, now),
        Press::Consumed,
        "an autorepeat before the release"
    );
    release_monitor("ctrl-shift-t");
    let identity = native_identity("ctrl-shift-t");
    assert!(
        !super::ROUTER.with_borrow(|router| router.consumed.contains(&identity)),
        "the dispatched release lets go of the key the press held"
    );
    assert!(
        super::ROUTER.with_borrow_mut(|router| router.release(identity, now)),
        "a late key-up listener still learns the release was the keymap's"
    );
}

#[test]
#[cfg(all(feature = "html", target_os = "macos"))]
fn releasing_a_dispatched_repeat_tail_opens_its_repeat_window() {
    let (keymap, _) = parse_keymap("command \"n\" ctrl+a>n --repeat true\n", &linux_defaults());
    super::load_keymap(&keymap);
    let now = Instant::now();
    assert_eq!(press_monitor("ctrl-a", false, now), Press::Consumed);
    assert!(matches!(press_monitor("n", false, now), Press::Fired(_)));
    release_monitor("n");
    assert!(
        matches!(press_monitor("n", false, now), Press::Fired(_)),
        "the tail fires again inside the window"
    );
    release_monitor("n");
    let past = Instant::now() + super::REPEAT_TIMEOUT + Duration::from_millis(50);
    assert_eq!(press_monitor("n", false, past), Press::Passed, "the window lapsed");
}
