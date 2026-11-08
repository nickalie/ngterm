//! The key monitor's router without a window (the GTK build's `keymap_dispatch` tests, by GPUI keystrokes): chords and
//! commands consumed, autorepeats of consumed keys, `--repeat` tails, leaders and their timeouts, the menu's chords.
//! The macOS web view's handoff uses native physical identities; dispatch tests also exercise the real interceptor.

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

/// Modifiers changing over a held tail leave it the same key, and upstream's window starts at that key's release:
/// past the stand-in's deadline a modified autorepeat fires nothing, and the plain one after it fires again.
#[test]
fn a_held_tails_window_follows_its_key_when_the_modifiers_change() {
    let start = Instant::now();
    let (mut router, identity) = router_with_dispatched_tail(start);
    let modified = start + super::REPEAT_TIMEOUT + Duration::from_millis(100);
    assert_eq!(router.press(&native_s(true, true), modified, false), Press::Consumed);
    assert_eq!(router.deadline, None, "the key is still down");
    assert_eq!(router.held_repeat, Some(identity));
    router.release(identity, modified);
    assert_eq!(
        router.deadline,
        Some(modified + super::REPEAT_TIMEOUT),
        "the next stand-in reopens the window"
    );
    assert!(matches!(
        router.press(&native_s(false, true), modified + Duration::from_millis(100), false),
        Press::Fired(Fired::Command(_))
    ));
}

/// Upstream's `releaseRepeatTail`: a key pressed meanwhile closed the window, and "a new leader keeps its own, longer
/// timeout", so the old tail's autorepeat has nothing to suspend.
#[test]
fn a_tails_autorepeat_leaves_a_leader_typed_since_its_release_alone() {
    let start = Instant::now();
    let (mut router, _) = router_with_dispatched_tail(start);
    let leader = start + Duration::from_millis(100);
    assert!(press(&mut router, "ctrl-a", leader));
    let deadline = Some(leader + super::LEADER_TIMEOUT);
    assert_eq!(router.deadline, deadline);
    assert_eq!(
        router.press(&native_s(false, true), leader, false),
        Press::Consumed,
        "the window it repeated in is gone"
    );
    assert_eq!(router.deadline, deadline);
    assert_eq!(router.held_repeat, None);
}

/// Only the tail that last fired holds the window: an older tail still repeating takes it from nobody.
#[test]
fn a_released_tails_autorepeat_leaves_a_newer_tails_window_alone() {
    let now = Instant::now();
    let mut router = router("command \"s\" ctrl+a>s --repeat true\ncommand \"p\" ctrl+a>p --repeat true\n");
    assert!(press(&mut router, "ctrl-a", now));
    release(&mut router, "ctrl-a", now);
    assert!(matches!(
        router.press(&native_s(false, false), now, false),
        Press::Fired(Fired::Command(_))
    ));
    router.release(native_s(false, false).identity, now);
    assert!(press(&mut router, "p", now));
    assert_eq!(router.press(&native_s(true, true), now, false), Press::Consumed);
    assert_eq!(router.held_repeat, Some(stroke("p").identity));
    release(&mut router, "p", now);
    assert_eq!(
        router.deadline,
        Some(now + super::REPEAT_TIMEOUT),
        "p's release opens it"
    );
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

/// Full screen still belongs to the app with no live leader, or to a custom sequence that matches its chord.
#[test]
#[cfg(all(feature = "html", any(windows, target_os = "macos")))]
fn a_web_view_hands_over_full_screen_unless_it_ends_a_live_leader() {
    for tail in ["g", "f11"] {
        let (keymap, diagnostics) = parse_keymap(&format!("command \"g\" ctrl+a>{tail} true\n"), &linux_defaults());
        assert!(diagnostics.is_empty());
        for lapsed in [false, true] {
            super::load_keymap(&keymap);
            assert!(super::takes(&stroke("f11")), "no leader");
            let now = Instant::now();
            assert!(super::takes(&stroke("ctrl-a")));
            super::ROUTER.with_borrow_mut(|router| {
                assert_eq!(router.press(&stroke("ctrl-a"), now, true), Press::Consumed);
                assert!(router.engine.as_ref().is_some_and(CustomCommandEngine::is_armed));
                assert!(router.release(stroke("ctrl-a").identity, now));
                if lapsed {
                    router.deadline = Some(now);
                }
            });
            let taken = lapsed || tail == "f11";
            assert_eq!(super::takes(&stroke("f11")), taken, "tail={tail}, lapsed={lapsed}");
            if taken {
                let fired = super::ROUTER.with_borrow_mut(|router| router.press(&stroke("f11"), now, true));
                if lapsed {
                    assert_eq!(
                        fired,
                        Press::Fired(Fired::Builtin(ngterm_core::keymap::BuiltinAction::ToggleFullscreen))
                    );
                } else {
                    assert!(matches!(fired, Press::Fired(Fired::Command(command)) if command.name == "g"));
                }
            }
        }
    }
}

#[cfg(all(feature = "html", target_os = "macos"))]
fn dispatched(text: &str) -> Keystroke {
    Keystroke::parse(text).unwrap().with_simulated_ime()
}

/// The platform's record of `text`, a letter chord or Escape: what `NSApp.currentEvent` holds while a key a web view handed over
/// is dispatched, so the press carries the physical key's identity rather than its name's.
#[cfg(all(feature = "html", target_os = "macos"))]
fn native_source(text: &str, repeat: bool) -> Source {
    let keystroke = dispatched(text);
    let letter = keystroke.key.chars().next().unwrap();
    let (code, typed) = if keystroke.key == "escape" {
        ("Escape".to_owned(), '\u{1b}')
    } else {
        (format!("Key{}", letter.to_ascii_uppercase()), letter)
    };
    Source::Native(NativeKey {
        keycode: crate::terminal::input::xkb_keycode(&code).unwrap(),
        modifiers: keystroke.modifiers,
        unshifted: Some(typed),
        ascii_layout: true,
        text: typed.to_string(),
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

/// Lets go of the last dispatch as the page does right after dispatching it.
#[cfg(all(feature = "html", target_os = "macos"))]
fn release_monitor() {
    super::release_dispatched();
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

/// The Shift key pressed alone, with a base the chord could resolve from, so only the key being a modifier keeps it
/// from being one.
#[cfg(all(feature = "html", target_os = "macos"))]
fn native_shift() -> KeyStroke {
    let native = NativeKey {
        keycode: 50,
        unshifted: Some('a'),
        ascii_layout: true,
        ..NativeKey::default()
    };
    KeyStroke::new(&dispatched("shift"), &Source::Native(native), false)
}

#[test]
#[cfg(all(feature = "html", target_os = "macos"))]
fn a_web_view_keeps_a_bare_modifier() {
    let shift = native_shift();
    assert_eq!(crate::terminal::input::chord(&shift), None);
    let (keymap, _) = parse_keymap("", &linux_defaults());
    super::load_keymap(&keymap);
    assert!(!super::takes(&shift));
}

#[test]
#[cfg(all(feature = "html", target_os = "macos"))]
fn a_web_view_hands_over_the_keymap_commands_without_advancing_the_matcher() {
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
    release_monitor();
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
    release_monitor();
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
    release_monitor();
    assert!(
        super::ROUTER.with_borrow(|router| router.deadline.is_some() && router.held_repeat.is_none()),
        "the dispatched release lets go of the tail under the identity it was pressed with, so its window opens"
    );
    assert!(
        matches!(press_monitor("n", false, now), Press::Fired(_)),
        "the tail fires again inside the window"
    );
    release_monitor();
    assert!(
        matches!(press_monitor("n", true, now), Press::Fired(_)),
        "a held tail's autorepeat fires"
    );
    release_monitor();
    assert!(super::takes(&native_stroke("n", false)), "the window is open");
    lapse_monitor();
    assert!(
        !super::takes(&native_stroke("n", false)),
        "the window lapsed, so the page keeps the next key"
    );
    assert!(
        !super::takes(&native_stroke("n", true)),
        "the page keeps that press's repeats too"
    );
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
    release_monitor();
    assert!(matches!(press_monitor("n", false, now), Press::Fired(_)));
    release_monitor();
    lapse_monitor();
    assert!(
        super::takes(&native_stroke("n", true)),
        "the key is still down, so the lapse is not one yet"
    );
    super::ROUTER.with_borrow_mut(Router::reset);
    assert_eq!(press_monitor("ctrl-a", false, now), Press::Consumed);
    release_monitor();
    assert!(matches!(press_monitor("g", false, now), Press::Fired(_)));
    release_monitor();
    assert!(
        super::takes(&native_stroke("g", true)),
        "a held last chord does not type into the page"
    );
    assert!(!super::takes(&native_stroke("g", false)), "a fresh press does");
    assert!(
        !super::takes(&native_stroke("g", true)),
        "and its autorepeat stays there"
    );
}

/// A half-typed leader run out: the page keeps the key the leader was waiting for.
#[test]
#[cfg(all(feature = "html", target_os = "macos"))]
fn a_web_view_keeps_the_chord_of_a_lapsed_leader() {
    let (keymap, _) = parse_keymap("command \"g\" ctrl+a>g true\n", &linux_defaults());
    super::load_keymap(&keymap);
    assert_eq!(press_monitor("ctrl-a", false, Instant::now()), Press::Consumed);
    release_monitor();
    assert!(super::takes(&native_stroke("g", false)), "the leader is armed");
    lapse_monitor();
    assert!(!super::takes(&native_stroke("g", false)));
}

/// A fresh press starts new ownership for the physical key (upstream's `handleKeyEvent`), even when its modifiers
/// changed and only the page sees it. Exercise a web view's dispatched release, a terminal's real release and a lost
/// release.
#[test]
#[cfg(all(feature = "html", target_os = "macos"))]
fn a_page_keeps_a_fresh_plain_keys_repeats_after_the_keymap_owned_its_chord() {
    for dispatched_release in [None, Some(false), Some(true)] {
        let (keymap, _) = parse_keymap("", &linux_defaults());
        super::load_keymap(&keymap);
        let now = Instant::now();
        assert!(super::takes(&native_stroke("ctrl-shift-t", false)));
        assert!(matches!(press_monitor("ctrl-shift-t", false, now), Press::Fired(_)));
        let identity = native_identity("t");
        assert_eq!(identity, native_identity("ctrl-shift-t"));
        assert_ne!(identity, stroke("t").identity);
        match dispatched_release {
            Some(true) => release_monitor(),
            Some(false) => assert!(super::ROUTER.with_borrow_mut(|router| router.release(identity, now))),
            None => {} // a release outside the app was never reported
        }
        page_keeps_press("t", now);
        assert!(super::takes(&native_stroke("ctrl-shift-t", false)));
        assert!(matches!(press_monitor("ctrl-shift-t", false, now), Press::Fired(_)));
    }
}

/// The page callback dispatches only a key the monitor takes, then stands in for its release. A passed key never
/// reaches `press`, which matters when the page's next event is an autorepeat of that key.
#[cfg(all(feature = "html", target_os = "macos"))]
fn page_press(text: &str, repeat: bool, now: Instant) -> Press {
    if !super::takes(&native_stroke(text, repeat)) {
        return Press::Passed;
    }
    let result = press_monitor(text, repeat, now);
    release_monitor();
    result
}

/// A press the page keeps never reaches the monitor, and its autorepeats, release and next press stay the page's.
#[cfg(all(feature = "html", target_os = "macos"))]
fn page_keeps_press(text: &str, now: Instant) {
    assert_eq!(page_press(text, false, now), Press::Passed);
    for _ in 0..3 {
        assert_eq!(page_press(text, true, now), Press::Passed);
    }
    assert!(!super::ROUTER.with_borrow_mut(|router| router.release(native_identity(text), now)));
    assert_eq!(page_press(text, false, now), Press::Passed);
}

/// Upstream's ownership table, through the page's native identity path: a plain chord fires once per press, while
/// an unbound key's press, repeats and release all stay with the page. Modifiers changing never changes the identity.
#[test]
#[cfg(all(feature = "html", target_os = "macos"))]
fn a_pages_plain_chord_and_unbound_key_keep_their_press_ownership() {
    let (keymap, _) = parse_keymap("", &linux_defaults());
    super::load_keymap(&keymap);
    let now = Instant::now();
    let fired = Press::Fired(Fired::Builtin(ngterm_core::keymap::BuiltinAction::NewSession));
    assert_eq!(page_press("ctrl-shift-t", false, now), fired);
    for repeat in ["ctrl-shift-t", "t"] {
        assert_eq!(page_press(repeat, true, now), Press::Consumed);
    }
    assert!(super::ROUTER.with_borrow_mut(|router| router.release(native_identity("t"), now)));
    assert_eq!(page_press("ctrl-shift-t", false, now), fired);
    page_keeps_press("t", now);
}

/// A leader's held prefix stays consumed without restarting its timer. Its tail fires inside the leader window and
/// belongs to the page past it; a completed nonrepeat tail's next press belongs to the page too.
#[test]
#[cfg(all(feature = "html", target_os = "macos"))]
fn a_pages_leader_and_nonrepeat_tail_follow_the_leader_window() {
    for lapsed in [false, true] {
        let (keymap, _) = parse_keymap("command \"g\" ctrl+a>g true\n", &linux_defaults());
        super::load_keymap(&keymap);
        let now = Instant::now();
        assert_eq!(page_press("ctrl-a", false, now), Press::Consumed);
        let deadline = super::ROUTER.with_borrow(|router| router.deadline);
        // the autorepeat as the matcher would read it, Ctrl held or let go, later than the press so a restart shows
        for repeat in ["ctrl-a", "a"] {
            assert_eq!(
                page_press(repeat, true, now + Duration::from_millis(100)),
                Press::Consumed
            );
            assert_eq!(super::ROUTER.with_borrow(|router| router.deadline), deadline);
        }
        if lapsed {
            lapse_monitor();
            assert_eq!(page_press("a", true, Instant::now()), Press::Consumed);
        }
        assert!(super::ROUTER.with_borrow_mut(|router| router.release(native_identity("a"), now)));
        if !lapsed {
            assert!(matches!(page_press("g", false, now), Press::Fired(Fired::Command(_))));
            assert_eq!(page_press("g", true, now), Press::Consumed);
            assert!(super::ROUTER.with_borrow_mut(|router| router.release(native_identity("g"), now)));
        }
        page_keeps_press("g", Instant::now());
        assert_eq!(page_press("ctrl-a", false, now), Press::Consumed);
    }
}

/// A repeat tail fires while held even beyond the stand-in release's window, and on a fresh press inside the window
/// the last stand-in opened; a late key-up still reports the release as the keymap's. Once that window lapses a fresh
/// press and all its repeats belong to the page.
#[test]
#[cfg(all(feature = "html", target_os = "macos"))]
fn a_pages_repeat_tail_follows_the_release_window() {
    let (keymap, _) = parse_keymap("command \"n\" ctrl+a>n --repeat true\n", &linux_defaults());
    super::load_keymap(&keymap);
    let now = Instant::now();
    assert_eq!(page_press("ctrl-a", false, now), Press::Consumed);
    assert!(matches!(page_press("n", false, now), Press::Fired(Fired::Command(_))));
    assert!(super::ROUTER.with_borrow(|router| router.deadline.is_some()));
    for delayed in [false, true] {
        if delayed {
            lapse_monitor();
        }
        assert!(matches!(
            page_press("n", true, Instant::now()),
            Press::Fired(Fired::Command(_))
        ));
    }
    assert!(super::ROUTER.with_borrow_mut(|router| router.release(native_identity("n"), now)));
    assert!(matches!(page_press("n", false, now), Press::Fired(Fired::Command(_))));
    lapse_monitor();
    page_keeps_press("n", Instant::now());
}

#[cfg(all(feature = "html", target_os = "macos"))]
mod dispatch_tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use gpui_kit::{
        AppContext as _, Context, FocusHandle, InteractiveElement as _, IntoElement, Render, Styled as _,
        TestAppContext, Window, div,
    };

    use super::*;

    struct InputTarget {
        focus: FocusHandle,
        seen: Rc<Cell<usize>>,
    }

    impl Render for InputTarget {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let seen = self.seen.clone();
            div()
                .size_full()
                .key_context("Input")
                .track_focus(&self.focus)
                .on_key_down(move |_, _, _| seen.set(seen.get() + 1))
        }
    }

    /// A dispatch passed over for a focused field lets go of nothing, including the key the monitor last handled.
    /// No assignment to `pressed`: only the real interceptor can clear the preceding dispatch's identity.
    #[gpui_kit::test]
    fn a_dispatched_release_with_no_press_lets_go_of_nothing(cx: &mut TestAppContext) {
        let seen = Rc::new(Cell::new(0));
        let (target, cx) = cx.add_window_view(|_, cx| InputTarget {
            focus: cx.focus_handle(),
            seen: seen.clone(),
        });
        cx.update(|window, cx| {
            super::super::init(cx);
            super::super::register_window(
                window.window_handle(),
                super::super::KeyOrigin::Window(uuid::Uuid::new_v4()),
            );
            let focus = target.read(cx).focus.clone();
            window.focus(&focus, cx);
        });
        cx.run_until_parked();
        let (keymap, _) = parse_keymap("", &linux_defaults());
        super::super::load_keymap(&keymap);
        assert!(matches!(
            press_monitor("ctrl-shift-t", false, Instant::now()),
            Press::Fired(_)
        ));
        let identity = native_identity("ctrl-shift-t");
        cx.update(|window, cx| window.dispatch_keystroke(dispatched("g"), cx));
        assert_eq!(seen.get(), 1, "the focused field received the dispatch");
        release_monitor();
        assert!(super::super::ROUTER.with_borrow(|router| router.consumed.contains(&identity)));
    }

    /// Ctrl+Tab goes through the window's own keys and `consume`, rather than `Router::press`. The dispatched release
    /// must still know its identity.
    #[gpui_kit::test]
    fn a_dispatched_window_owned_key_is_released(cx: &mut TestAppContext) {
        use crate::deck::{Deck, DeckEnv, Watermarks};
        use crate::terminal::TerminalConfig;

        cx.update(gpui_kit::init);
        let state_dir = tempfile::tempdir().unwrap();
        let env = Rc::new(DeckEnv::new(
            crate::pane::PaneContext {
                socket: state_dir.path().join("control.sock").to_string_lossy().into_owned(),
                program_version: "1.2.3".to_owned(),
            },
            Rc::new(|_| Box::new(|_| Err(ngterm_term::PaneError::Closed))),
            TerminalConfig::default(),
            crate::state::detached_poster(),
            Watermarks {
                state_dir: state_dir.path().to_path_buf(),
                config_with: Rc::new(|_| TerminalConfig::default()),
            },
        ));
        let (_, cx) = cx.add_window_view(move |root, cx| {
            let id = uuid::Uuid::new_v4();
            let poster = env.poster.clone();
            let deck = cx.new(|_| Deck::new(id, env));
            crate::window::WindowView::new(id, deck, poster, 0, root, cx)
        });
        let keystroke = dispatched("ctrl-tab");
        let identity = KeyStroke::new(&keystroke, &crate::terminal::input::current_source(&keystroke), false).identity;
        cx.update(|window, cx| {
            super::super::init(cx);
            window.dispatch_keystroke(keystroke, cx);
        });
        assert!(super::super::ROUTER.with_borrow(|router| router.consumed.contains(&identity)));
        release_monitor();
        assert!(!super::super::ROUTER.with_borrow(|router| router.consumed.contains(&identity)));
        assert!(super::super::ROUTER.with_borrow(|router| router.released.contains(&identity)));
    }
}

/// A repeat belongs to the page if its press did, even if a leader armed meanwhile would take a fresh tail.
#[test]
#[cfg(all(feature = "html", target_os = "macos"))]
fn a_page_keeps_an_unowned_repeat_even_when_the_matcher_would_take_its_chord() {
    let (keymap, _) = parse_keymap("command \"g\" ctrl+a>g true\n", &linux_defaults());
    super::load_keymap(&keymap);
    let now = Instant::now();
    assert_eq!(page_press("g", false, now), Press::Passed);
    assert_eq!(page_press("ctrl-a", false, now), Press::Consumed);
    assert_eq!(page_press("g", true, now), Press::Passed);
    assert!(matches!(page_press("g", false, now), Press::Fired(Fired::Command(_))));
}

/// Autorepeating a held prefix after the tail's release must not suspend the tail's timeout (upstream's
/// `testHeldRepeatTailAutorepeatsOnlyWhileTheWindowIsOpen` also checks that other consumed repeats leave the window alone).
#[test]
#[cfg(all(feature = "html", target_os = "macos"))]
fn a_pages_held_prefix_does_not_suspend_a_released_tails_window() {
    let (keymap, _) = parse_keymap("command \"n\" ctrl+a>n --repeat true\n", &linux_defaults());
    super::load_keymap(&keymap);
    let now = Instant::now();
    assert_eq!(page_press("ctrl-a", false, now), Press::Consumed);
    assert!(matches!(page_press("n", false, now), Press::Fired(_)));
    let deadline = super::ROUTER.with_borrow(|router| router.deadline);
    assert!(deadline.is_some());
    assert_eq!(page_press("ctrl-a", true, now), Press::Consumed);
    assert_eq!(super::ROUTER.with_borrow(|router| router.deadline), deadline);
    lapse_monitor();
    assert_eq!(page_press("n", false, Instant::now()), Press::Passed);
}

/// Loads `keymap` into this thread's monitor.
#[cfg(all(feature = "html", target_os = "macos"))]
fn load_monitor(keymap: &str) {
    let (keymap, _) = parse_keymap(keymap, &linux_defaults());
    super::load_keymap(&keymap);
}

/// Whether this thread's monitor has a half-typed leader or an open repeat window.
#[cfg(all(feature = "html", target_os = "macos"))]
fn monitor_is_live() -> bool {
    super::ROUTER.with_borrow(|router| {
        router
            .engine
            .as_ref()
            .is_some_and(|engine| engine.is_armed() || engine.is_repeating())
    })
}

/// Upstream's monitor sees a key typed into a page too, and an unmatched press ends the sequence: the page types the
/// leader's next chord instead of losing it to the command. Undo and full screen's menu chords count as unmatched.
#[test]
#[cfg(all(feature = "html", target_os = "macos"))]
fn a_key_the_page_keeps_drops_a_half_typed_leader() {
    for kept in ["x", "ctrl-shift-z", "ctrl-cmd-f"] {
        load_monitor("command \"g\" ctrl+a>g true\nmap cmd+ctrl+f toggle_fullscreen\n");
        assert_eq!(
            super::menu_action(&native_stroke("ctrl-cmd-f", false)),
            Some(ngterm_core::keymap::BuiltinAction::ToggleFullscreen)
        );
        let now = Instant::now();
        assert_eq!(page_press("ctrl-a", false, now), Press::Consumed);
        assert!(!super::takes(&native_stroke(kept, false)), "the page keeps {kept}");
        assert!(!monitor_is_live(), "{kept}");
        assert_eq!(super::ROUTER.with_borrow(|router| router.deadline), None, "{kept}");
        assert_eq!(page_press("g", false, now), Press::Passed, "{kept}");
    }
}

/// Upstream's "a key that repeats nothing ends the window" (`testAKeyTypedIntoATextFieldClosesTheRepeatWindow`).
#[test]
#[cfg(all(feature = "html", target_os = "macos"))]
fn a_key_the_page_keeps_closes_a_repeat_window() {
    load_monitor("command \"n\" ctrl+a>n --repeat true\n");
    let now = Instant::now();
    assert_eq!(page_press("ctrl-a", false, now), Press::Consumed);
    assert!(matches!(page_press("n", false, now), Press::Fired(Fired::Command(_))));
    assert_eq!(page_press("x", false, now), Press::Passed);
    assert!(!monitor_is_live());
    assert_eq!(super::ROUTER.with_borrow(|router| router.deadline), None);
    assert_eq!(page_press("n", false, now), Press::Passed);
}

/// A press the page keeps never advances a shift about to be part of the next chord: upstream's bare modifier "can't
/// advance; while armed, keep waiting".
#[test]
#[cfg(all(feature = "html", target_os = "macos"))]
fn a_bare_modifier_the_page_keeps_leaves_a_leader_waiting() {
    load_monitor("command \"g\" ctrl+a>g true\n");
    let now = Instant::now();
    assert_eq!(page_press("ctrl-a", false, now), Press::Consumed);
    assert!(!super::takes(&native_shift()));
    assert!(monitor_is_live());
    assert!(matches!(page_press("g", false, now), Press::Fired(Fired::Command(_))));
}

/// Upstream's Esc: it abandons a half-typed leader and is the keymap's only then, through its autorepeats and release.
#[test]
#[cfg(all(feature = "html", target_os = "macos"))]
fn escape_in_a_page_cancels_a_leader_and_is_consumed_only_then() {
    load_monitor("command \"g\" ctrl+a>g true\n");
    let now = Instant::now();
    assert!(!super::takes(&native_stroke("escape", false)));
    assert_eq!(page_press("ctrl-a", false, now), Press::Consumed);
    assert_eq!(page_press("escape", false, now), Press::Consumed);
    assert!(!monitor_is_live());
    assert_eq!(page_press("escape", true, now), Press::Consumed);
    assert!(super::ROUTER.with_borrow_mut(|router| router.release(native_identity("escape"), now)));
    assert_eq!(page_press("g", false, now), Press::Passed);
    assert!(!super::takes(&native_stroke("escape", false)));
    page_keeps_press("escape", now);
}

/// Upstream's Esc over a repeat window: "an open repeat window closes, but Esc still reaches the terminal".
#[test]
#[cfg(all(feature = "html", target_os = "macos"))]
fn escape_in_a_page_closes_a_repeat_window_and_stays_the_pages() {
    load_monitor("command \"n\" ctrl+a>n --repeat true\n");
    let now = Instant::now();
    assert_eq!(page_press("ctrl-a", false, now), Press::Consumed);
    assert!(matches!(page_press("n", false, now), Press::Fired(Fired::Command(_))));
    assert!(!super::takes(&native_stroke("escape", false)));
    assert!(!monitor_is_live());
    assert_eq!(page_press("n", false, now), Press::Passed);
}

/// A leader run out is gone before its timeout's next key reads it: Esc has nothing to cancel and goes to the page.
#[test]
#[cfg(all(feature = "html", target_os = "macos"))]
fn escape_in_a_page_after_a_lapsed_leader_stays_the_pages() {
    load_monitor("command \"g\" ctrl+a>g true\n");
    let now = Instant::now();
    assert_eq!(page_press("ctrl-a", false, now), Press::Consumed);
    lapse_monitor();
    assert!(!super::takes(&native_stroke("escape", false)));
    assert!(!monitor_is_live());
}

/// The tail held in a page with a modifier going down over it stays held: past the stand-in's deadline the matcher
/// is still repeating, and the plain autorepeat after it fires.
#[test]
#[cfg(all(feature = "html", target_os = "macos"))]
fn a_pages_held_tail_outlasts_a_modifier_held_past_its_window() {
    load_monitor("command \"n\" ctrl+a>n --repeat true\n");
    let now = Instant::now();
    assert_eq!(page_press("ctrl-a", false, now), Press::Consumed);
    assert!(matches!(page_press("n", false, now), Press::Fired(Fired::Command(_))));
    lapse_monitor();
    assert_eq!(page_press("shift-n", true, Instant::now()), Press::Consumed);
    assert!(
        super::ROUTER.with_borrow(|router| router.deadline.is_some()),
        "its stand-in reopened the window"
    );
    assert!(matches!(
        page_press("n", true, Instant::now()),
        Press::Fired(Fired::Command(_))
    ));
}
