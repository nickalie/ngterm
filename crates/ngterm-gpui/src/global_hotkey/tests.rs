//! The global hotkey's key positions, its portal trigger and its X grabs.

use ngterm_core::keymap::{Chord, Modifiers, parse_keybind};

use super::*;

fn chord(text: &str) -> Chord {
    parse_keybind(text).expect("a chord").remove(0)
}

/// Upstream's `keyCode(forChordKey:)`: the key position a chord key names, on the US layout's positions, whatever
/// layout is active, so the hotkey survives a layout switch.
#[test]
fn a_chord_key_names_its_us_position() {
    assert_eq!(keycode("`"), Some(49), "the key left of 1");
    assert_eq!(keycode("t"), Some(28));
    assert_eq!(keycode("1"), Some(10));
    assert_eq!(keycode("\\"), Some(51));
    assert_eq!(keycode("/"), Some(61));
    assert_eq!(keycode("return"), Some(36));
    assert_eq!(keycode("delete"), Some(22), "the keymap's delete is Backspace");
    assert_eq!(keycode("space"), Some(65));
    assert_eq!(keycode("f5"), Some(71));
    assert_eq!(keycode("f13"), Some(191));
    assert_eq!(keycode("up"), Some(111));
    assert_eq!(keycode("é"), None, "no position types it");
    for key in "abcdefghijklmnopqrstuvwxyz0123456789=-][';\\,/.`".chars() {
        let key = key.to_string();
        assert!(keycode(&key).is_some(), "{key} has a position");
    }
}

/// The XDG shortcuts specification's trigger: modifiers in its spelling, the key as its keysym name.
#[test]
fn a_chord_reads_as_a_portal_trigger() {
    assert_eq!(portal_trigger(&chord("ctrl+`")).as_deref(), Some("CTRL+grave"));
    assert_eq!(portal_trigger(&chord("cmd+shift+t")).as_deref(), Some("SHIFT+LOGO+t"));
    assert_eq!(
        portal_trigger(&chord("ctrl+alt+return")).as_deref(),
        Some("CTRL+ALT+Return")
    );
    assert_eq!(portal_trigger(&chord("f12")).as_deref(), Some("F12"));
    assert_eq!(portal_trigger(&chord("opt+space")).as_deref(), Some("ALT+space"));
    assert_eq!(
        portal_trigger(&Chord::new(Modifiers::CONTROL, "é")),
        None,
        "no keysym name"
    );
}

/// The X grab: the chord's modifiers as X's, under every Caps Lock and Num Lock state.
#[cfg(target_os = "linux")]
#[test]
fn an_x_grab_covers_every_lock_state() {
    use x11rb::protocol::xproto::ModMask;

    let grabs = x11::grabs(49, Modifiers::CONTROL | Modifiers::COMMAND);
    let base = ModMask::CONTROL | ModMask::M4;
    assert_eq!(
        grabs,
        vec![
            (49, base),
            (49, base | ModMask::LOCK),
            (49, base | ModMask::M2),
            (49, base | ModMask::LOCK | ModMask::M2),
        ]
    );
    assert_eq!(
        x11::modifier_mask(Modifiers::OPTION | Modifiers::SHIFT),
        ModMask::M1 | ModMask::SHIFT
    );
}

/// Every named key and every punctuation key in the XDG shortcuts specification's keysym spelling.
#[test]
fn every_named_and_punctuation_key_has_its_keysym_name() {
    let spelled = [
        ("tab", "Tab"),
        ("delete", "BackSpace"),
        ("left", "Left"),
        ("right", "Right"),
        ("up", "Up"),
        ("down", "Down"),
        ("-", "minus"),
        ("=", "equal"),
        ("[", "bracketleft"),
        ("]", "bracketright"),
        ("\\", "backslash"),
        (";", "semicolon"),
        ("'", "apostrophe"),
        (",", "comma"),
        (".", "period"),
        ("/", "slash"),
        ("7", "7"),
    ];
    for (key, keysym) in spelled {
        assert_eq!(
            portal_trigger(&Chord::new(Modifiers::CONTROL, key)).as_deref(),
            Some(format!("CTRL+{keysym}").as_str()),
            "{key}"
        );
    }
}

/// The named keys' positions: the evdev keycodes plus XKB's offset of 8.
#[test]
fn every_named_key_names_its_position() {
    for (key, keycode) in [("escape", 9), ("tab", 23), ("left", 113), ("right", 114), ("down", 116)] {
        assert_eq!(super::keycode(key), Some(keycode), "{key}");
    }
}

fn core() -> ngterm_core::Core {
    ngterm_core::Core::new(
        ngterm_core::CoreConfig {
            app: ngterm_proto::AppIdentity::from_recorded("1.2.3", None),
            capabilities: Vec::new(),
        },
        ngterm_core::CoreState {
            library: ngterm_core::model::WindowLibrary::seeded("/home/u"),
            ..ngterm_core::CoreState::default()
        },
    )
}

/// A press toggles the quick terminal, whichever application has the keyboard; one after the quit save shows nothing.
#[test]
fn a_press_toggles_the_quick_terminal_until_the_quit_save() {
    let mut state = AppState::new(core());
    state.global_hotkey_fired();
    assert!(state.core.quick_visible(), "the first press shows the panel");
    state.global_hotkey_fired();
    assert!(!state.core.quick_visible(), "the next one hides it");
    state.save_on_quit();
    state.global_hotkey_fired();
    assert!(!state.core.quick_visible(), "a press while quitting shows nothing");
}

/// The warnings logged while it is the thread's default subscriber.
#[cfg(target_os = "linux")]
#[derive(Default)]
struct Warnings(std::sync::Mutex<Vec<String>>);

#[cfg(target_os = "linux")]
impl Warnings {
    fn seen(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

/// An event's message.
#[cfg(target_os = "linux")]
struct Message(String);

#[cfg(target_os = "linux")]
impl tracing::field::Visit for Message {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

#[cfg(target_os = "linux")]
impl tracing::Subscriber for Warnings {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        *metadata.level() <= tracing::Level::WARN
    }

    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }

    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        let mut message = Message(String::new());
        event.record(&mut message);
        self.0.lock().unwrap().push(message.0);
    }

    fn enter(&self, _: &tracing::span::Id) {}

    fn exit(&self, _: &tracing::span::Id) {}
}

/// A display GPUI names neither X11 nor Wayland (the test platform's) has no global hotkey: said once, however many
/// chords the keymap names after, and not at all while it names none.
#[cfg(target_os = "linux")]
#[gpui_kit::test]
fn a_display_without_a_global_hotkey_says_so_once(cx: &mut gpui_kit::TestAppContext) {
    let warnings = std::sync::Arc::new(Warnings::default());
    let poster = crate::state::detached_poster();
    let mut hotkey = GlobalHotkey::default();
    tracing::subscriber::with_default(std::sync::Arc::clone(&warnings), || {
        cx.update(|cx| {
            hotkey.sync(None, &poster, cx);
            assert_eq!(warnings.seen(), Vec::<String>::new(), "no chord, nothing to say");
            hotkey.sync(Some(&chord("ctrl+`")), &poster, cx);
            hotkey.sync(Some(&chord("ctrl+alt+t")), &poster, cx);
        });
    });
    assert_eq!(
        warnings.seen(),
        ["this display has no global hotkey: global-hotkey is ignored"],
        "said once"
    );
}

#[cfg(windows)]
#[test]
fn keymap_sync_changes_the_real_windows_hotkey_in_a_child() {
    crate::system_window::tests::run_child("global_hotkey::tests::windows_registration_child");
}

#[cfg(windows)]
#[gpui_kit::test]
#[ignore = "run by keymap_sync_changes_the_real_windows_hotkey_in_a_child"]
fn windows_registration_child(cx: &mut gpui_kit::TestAppContext) {
    use std::time::{Duration, Instant};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        MOD_ALT, MOD_CONTROL, MOD_SHIFT, RegisterHotKey, UnregisterHotKey, VK_F20,
    };

    if !crate::system_window::tests::is_child("global_hotkey::tests::windows_registration_child") {
        return;
    }
    let flags = MOD_CONTROL | MOD_ALT | MOD_SHIFT;
    let probe = || {
        // SAFETY: a thread-owned registration with no HWND and an id private to this test thread.
        let registered = unsafe { RegisterHotKey(std::ptr::null_mut(), 0x601, flags, u32::from(VK_F20)) } != 0;
        if registered {
            // SAFETY: releasing the registration just taken above.
            assert_ne!(unsafe { UnregisterHotKey(std::ptr::null_mut(), 0x601) }, 0);
        }
        registered
    };
    assert!(probe(), "the test chord must initially be available");
    let poster = crate::state::detached_poster();
    let mut hotkey = GlobalHotkey::default();
    cx.update(|cx| hotkey.sync(None, &poster, cx));
    assert!(probe(), "an empty keymap reserves no chord");
    let wanted = chord("ctrl+alt+shift+f20");
    cx.update(|cx| hotkey.sync(Some(&wanted), &poster, cx));
    let deadline = Instant::now() + Duration::from_secs(5);
    while probe() {
        assert!(Instant::now() < deadline, "keymap sync did not reserve its chord");
        std::thread::sleep(Duration::from_millis(10));
    }
    cx.update(|cx| hotkey.sync(Some(&wanted), &poster, cx));
    cx.update(|cx| hotkey.sync(None, &poster, cx));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !probe() {
        assert!(
            Instant::now() < deadline,
            "clearing the keymap did not release its chord"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(windows)]
#[gpui_kit::test]
fn the_windows_press_callback_posts_the_toggle_onto_the_main_loop(cx: &mut gpui_kit::TestAppContext) {
    use ngterm_host::MainLoop;
    use std::sync::Arc;

    let main_loop = Arc::new(crate::main_loop::GpuiMainLoop::<AppState>::new(cx.executor()));
    let owner: Arc<dyn MainLoop<AppState>> = main_loop.clone();
    let callback = Arc::new(fired(&Arc::downgrade(&owner)));
    let tasks = main_loop.tasks();
    let on_thread = Arc::clone(&callback);
    std::thread::spawn(move || on_thread()).join().unwrap();
    let mut state = AppState::new(core());
    assert!(
        !state.core.quick_visible(),
        "the callback only posts; it cannot mutate the state on its thread"
    );
    tasks.try_recv().unwrap()(&mut state);
    assert!(state.core.quick_visible());
    assert!(tasks.try_recv().is_err(), "one press posts one toggle");
    main_loop.close();
    callback();
    assert!(tasks.try_recv().is_err(), "a press after shutdown queues nothing");
}
