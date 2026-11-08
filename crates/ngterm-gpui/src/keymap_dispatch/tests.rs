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

/// A chord pressed in a web view: the release standing in for the real one comes right after each dispatched press,
/// and the key is still down. Upstream's `handleKeyEvent`: "Repeats and releases of consumed presses stay consumed
/// without firing again".
#[test]
fn an_autorepeat_after_a_dispatched_release_stays_consumed() {
    let now = Instant::now();
    let mut router = router("");
    assert!(matches!(
        router.press(&native_s(true, false), now, false),
        Press::Fired(Fired::Builtin(_))
    ));
    let identity = native_s(true, false).identity;
    assert!(router.release(identity, now));
    for repeat in 1..=3 {
        assert_eq!(
            router.press(&native_s(true, true), now, false),
            Press::Consumed,
            "autorepeat {repeat} of the held chord"
        );
        assert!(router.release(identity, now), "its release is the keymap's");
    }
    assert!(
        matches!(
            router.press(&native_s(true, false), now, false),
            Press::Fired(Fired::Builtin(_))
        ),
        "a fresh press fires again"
    );
}

/// The S key as a `--repeat` tail, with its leader typed (`ctrl+a` released) and the tail fired once and let go at
/// `now`, as a web view's dispatch does.
fn router_with_dispatched_tail(now: Instant) -> (Router, u32) {
    let mut router = router("command \"s\" ctrl+a>s --repeat true\n");
    assert!(press(&mut router, "ctrl-a", now));
    release(&mut router, "ctrl-a", now);
    assert!(matches!(
        router.press(&native_s(false, false), now, false),
        Press::Fired(Fired::Command(_))
    ));
    let identity = native_s(false, false).identity;
    router.release(identity, now);
    assert_eq!(router.deadline, Some(now + super::REPEAT_TIMEOUT), "its window opens");
    (router, identity)
}

#[test]
fn a_held_tail_keeps_firing_past_the_window_its_dispatched_release_opened() {
    let start = Instant::now();
    let (mut router, identity) = router_with_dispatched_tail(start);
    // each autorepeat within the half second after the one before, the last well past the first window
    for millis in [400, 800, 1200] {
        let now = start + Duration::from_millis(millis);
        assert!(
            matches!(
                router.press(&native_s(false, true), now, false),
                Press::Fired(Fired::Command(_))
            ),
            "the autorepeat at {millis} ms"
        );
        assert_eq!(router.deadline, None, "the window waits for the key's release");
        router.release(identity, now);
        assert_eq!(
            router.deadline,
            Some(now + super::REPEAT_TIMEOUT),
            "and reopens after it"
        );
    }
}

/// The first autorepeat comes after the system's initial delay, which can be longer than the window: the key was held
/// all along, so upstream's window, which starts at the real release, is still open.
#[test]
fn a_held_tail_outlasts_a_key_repeat_delay_longer_than_the_window() {
    let start = Instant::now();
    let (mut router, _) = router_with_dispatched_tail(start);
    let now = start + super::REPEAT_TIMEOUT + Duration::from_millis(500);
    assert!(matches!(
        router.press(&native_s(false, true), now, false),
        Press::Fired(Fired::Command(_))
    ));
}

#[test]
fn a_fresh_press_after_a_dispatched_release_finds_the_window_lapsed() {
    let start = Instant::now();
    let (mut router, _) = router_with_dispatched_tail(start);
    let now = start + super::REPEAT_TIMEOUT;
    assert_eq!(
        router.press(&native_s(false, false), now, false),
        Press::Passed,
        "not an autorepeat, so the key went up and the window ran out"
    );
    assert!(!router.engine.as_ref().is_some_and(CustomCommandEngine::is_repeating));
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

/// What `text` reads as when a web view holds the key, `repeat` when the platform says it is an autorepeat.
#[cfg(all(feature = "html", target_os = "macos"))]
fn native_stroke(text: &str, repeat: bool) -> KeyStroke {
    KeyStroke::new(&dispatched(text), &native_source(text, repeat), false)
}

/// Presses `text` on this thread's monitor, as the dispatch of a key a web view handed over does.
#[cfg(all(feature = "html", target_os = "macos"))]
fn press_monitor(text: &str, repeat: bool, now: Instant) -> Press {
    let stroke = native_stroke(text, repeat);
    super::ROUTER.with_borrow_mut(|router| router.press(&stroke, now, false))
}

/// Lets go of `text` as the page does right after dispatching it.
#[cfg(all(feature = "html", target_os = "macos"))]
fn release_monitor(text: &str) {
    super::release_dispatched(&dispatched(text));
}

/// Ends this thread's monitor's window, as time passing would.
#[cfg(all(feature = "html", target_os = "macos"))]
fn lapse_monitor() {
    super::ROUTER.with_borrow_mut(|router| router.deadline = Some(Instant::now()));
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
    release_monitor("ctrl-shift-t");
    let identity = native_identity("ctrl-shift-t");
    assert_ne!(
        identity,
        stroke("ctrl-shift-t").identity,
        "the press is held by the physical key, not by the key's name"
    );
    assert!(
        !super::ROUTER.with_borrow(|router| router.consumed.contains(&identity)),
        "the dispatched release lets go of the key the press held"
    );
    assert_eq!(
        press_monitor("ctrl-shift-t", true, now),
        Press::Consumed,
        "the key is still down: its autorepeat does not fire again"
    );
    release_monitor("ctrl-shift-t");
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
        super::ROUTER.with_borrow(|router| router.deadline.is_some() && router.held_repeat.is_none()),
        "the dispatched release lets go of the tail under the identity it was pressed with, so its window opens"
    );
    assert!(
        matches!(press_monitor("n", false, now), Press::Fired(_)),
        "the tail fires again inside the window"
    );
    release_monitor("n");
    assert!(
        matches!(press_monitor("n", true, now), Press::Fired(_)),
        "a held tail's autorepeat fires"
    );
    release_monitor("n");
    assert!(super::takes(&native_stroke("n", false)), "the window is open");
    lapse_monitor();
    assert!(
        !super::takes(&native_stroke("n", false)),
        "the window lapsed, so the page keeps the next key"
    );
    let past = Instant::now() + super::REPEAT_TIMEOUT + Duration::from_millis(50);
    assert_eq!(press_monitor("n", false, past), Press::Passed, "the window lapsed");
}

/// Asked before the press reaches the monitor, as the web view does: a held tail stays the keymap's even with its
/// window run out, and so does a leader's last chord, which the matcher no longer knows.
#[test]
#[cfg(all(feature = "html", target_os = "macos"))]
fn a_web_view_hands_over_the_autorepeat_of_a_key_the_keymap_took() {
    let (keymap, _) = parse_keymap(
        "command \"n\" ctrl+a>n --repeat true\ncommand \"g\" ctrl+a>g true\n",
        &linux_defaults(),
    );
    super::load_keymap(&keymap);
    let now = Instant::now();
    assert_eq!(press_monitor("ctrl-a", false, now), Press::Consumed);
    release_monitor("ctrl-a");
    assert!(matches!(press_monitor("n", false, now), Press::Fired(_)));
    release_monitor("n");
    lapse_monitor();
    assert!(
        super::takes(&native_stroke("n", true)),
        "the key is still down, so the lapse is not one yet"
    );
    super::ROUTER.with_borrow_mut(Router::reset);
    assert_eq!(press_monitor("ctrl-a", false, now), Press::Consumed);
    release_monitor("ctrl-a");
    assert!(matches!(press_monitor("g", false, now), Press::Fired(_)));
    release_monitor("g");
    assert!(
        super::takes(&native_stroke("g", true)),
        "a held last chord does not type into the page"
    );
    assert!(!super::takes(&native_stroke("g", false)), "a fresh press does");
}

/// A half-typed leader run out: the page keeps the key the leader was waiting for.
#[test]
#[cfg(all(feature = "html", target_os = "macos"))]
fn a_web_view_keeps_the_chord_of_a_lapsed_leader() {
    let (keymap, _) = parse_keymap("command \"g\" ctrl+a>g true\n", &linux_defaults());
    super::load_keymap(&keymap);
    assert_eq!(press_monitor("ctrl-a", false, Instant::now()), Press::Consumed);
    release_monitor("ctrl-a");
    assert!(super::takes(&native_stroke("g", false)), "the leader is armed");
    lapse_monitor();
    assert!(!super::takes(&native_stroke("g", false)));
}

/// A dispatch the monitor passed over lets go of nothing, the key it last handled included.
#[test]
#[cfg(all(feature = "html", target_os = "macos"))]
fn a_dispatched_release_with_no_press_lets_go_of_nothing() {
    let (keymap, _) = parse_keymap("", &linux_defaults());
    super::load_keymap(&keymap);
    assert!(matches!(
        press_monitor("ctrl-shift-t", false, Instant::now()),
        Press::Fired(_)
    ));
    release_monitor("ctrl-shift-t");
    let identity = native_identity("ctrl-shift-t");
    super::ROUTER.with_borrow_mut(|router| router.consume(identity));
    super::ROUTER.with_borrow_mut(|router| router.pressed = None);
    release_monitor("ctrl-shift-t");
    assert!(super::ROUTER.with_borrow(|router| router.consumed.contains(&identity)));
}
