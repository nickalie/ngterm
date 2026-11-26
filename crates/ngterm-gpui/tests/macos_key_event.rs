//! The terminal's native key for `NSEvent` key events built here, as `AppKit` would dispatch them: text a tool posts as
//! the event's Unicode string on a key that does not type it (text expanders, password managers' auto-type, the
//! conformance ax driver's `type_text`: key code 0, the A key), a hardware key whose characters differ from the layout's
//! (a dead key's composition), a Control chord on the same key, and the U.S. layout's Option dead keys composed with the
//! next key, from the keyboard and posted alike, with nothing left pending; Backspace and Escape cancel such an accent,
//! and a posted string ends it and types as posted. It runs without a harness because the
//! layout's input source services want the main thread.
#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() {
    use std::time::Duration;
    // far beyond these calls; only a main thread that will never return reaches it
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(60));
        eprintln!("macos_key_event: the main thread hung");
        std::process::exit(1);
    });
    let failures: Vec<String> = [
        keys::a_posted_unicode_string_types_its_own_text_not_the_key_codes(),
        keys::a_hardware_key_types_the_layouts_text_whatever_its_characters(),
        keys::a_control_chord_keeps_the_layouts_text_for_its_key(),
        keys::option_dead_keys_compose_with_the_next_key_and_leave_nothing_pending(),
        keys::backspace_or_escape_cancels_a_dead_key_and_a_posted_string_ends_it(),
    ]
    .into_iter()
    .flatten()
    .collect();
    for failure in &failures {
        eprintln!("macos_key_event: {failure}");
    }
    if !failures.is_empty() {
        std::process::exit(1);
    }
    println!("macos_key_event: 5 passed");
}

#[cfg(target_os = "macos")]
mod keys {
    use gpui_kit::Keystroke;
    use ngterm_gpui::terminal::input::{Compose, DeadKeys, KeyStroke, NativeKey, Source, event_source};
    use objc2::rc::Retained;
    use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType};
    use objc2_core_graphics::{CGEvent, CGEventField};
    use objc2_foundation::{NSPoint, NSString};

    /// The A key's XKB keycode, where key code 0 lands.
    const KEY_A: u32 = 38;

    /// A key-down event with `characters` on key code `code` whose Quartz event carries `characters` and the pid of
    /// the process that posted it, 0 for the keyboard's.
    fn quartz(characters: &str, code: u16, flags: NSEventModifierFlags, pid: i64) -> Result<Retained<NSEvent>, String> {
        let text = NSString::from_str(characters);
        let built = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
            NSEventType::KeyDown,
            NSPoint::new(0.0, 0.0),
            flags,
            0.0,
            0,
            None,
            &text,
            &text,
            false,
            code,
        )
        .ok_or("AppKit built no key event")?;
        let event = built.CGEvent().ok_or("the key event has no Quartz event")?;
        let units: Vec<u16> = characters.encode_utf16().collect();
        // SAFETY: `units` outlives the call, which copies its length of UTF-16 units.
        unsafe { CGEvent::keyboard_set_unicode_string(Some(&event), units.len() as _, units.as_ptr()) };
        CGEvent::set_integer_value_field(Some(&event), CGEventField::EventSourceUnixProcessID, pid);
        NSEvent::eventWithCGEvent(&event).ok_or_else(|| "AppKit built no event from the Quartz event".to_owned())
    }

    /// As a tool that posts text builds it: this process posts it, its Quartz event carrying the text.
    fn posted(characters: &str, code: u16, flags: NSEventModifierFlags) -> Result<Retained<NSEvent>, String> {
        quartz(characters, code, flags, i64::from(std::process::id()))
    }

    /// As the keyboard delivers it: no process posted it.
    fn from_hardware(characters: &str, code: u16, flags: NSEventModifierFlags) -> Result<Retained<NSEvent>, String> {
        quartz(characters, code, flags, 0)
    }

    fn read(event: &NSEvent) -> Result<NativeKey, String> {
        // SAFETY: a live key event, on the main thread.
        match unsafe { event_source(std::ptr::from_ref(event).cast_mut().cast()) } {
            Source::Native(key) => Ok(key),
            other => Err(format!("key code {} has no native record: {other:?}", event.keyCode())),
        }
    }

    fn native(characters: &str, code: u16, flags: NSEventModifierFlags) -> Result<NativeKey, String> {
        let event = posted(characters, code, flags)?;
        read(&event)
    }

    fn expect_eq<T: PartialEq + std::fmt::Debug + ?Sized>(what: &str, got: &T, want: &T) -> Result<(), String> {
        if got == want {
            Ok(())
        } else {
            Err(format!("{what}: got {got:?}, want {want:?}"))
        }
    }

    pub fn a_posted_unicode_string_types_its_own_text_not_the_key_codes() -> Option<String> {
        let run = || -> Result<(), String> {
            for text in ["u", "Q", "é", "ж", "你", "🙂"] {
                let key = native(text, 0, NSEventModifierFlags::empty())?;
                expect_eq(&format!("the text of {text:?}"), key.text.as_str(), text)?;
                expect_eq(
                    &format!("the Option-free text of {text:?}"),
                    &key.without_option.as_deref(),
                    &Some(text),
                )?;
                expect_eq("the key's position", &key.keycode, &KEY_A)?;
                expect_eq(&format!("{text:?} is a posted string"), &key.posted_text, &true)?;
            }
            let key = native("ж", 0, NSEventModifierFlags::Shift)?;
            expect_eq("the text with Shift", key.text.as_str(), "ж")?;
            expect_eq("Shift held", &key.modifiers.shift, &true)
        };
        run().err()
    }

    pub fn a_hardware_key_types_the_layouts_text_whatever_its_characters() -> Option<String> {
        let run = || -> Result<(), String> {
            let event = from_hardware("ä", 0, NSEventModifierFlags::empty())?;
            let key = read(&event)?;
            let plain = key.unshifted.ok_or("the A key types no character")?.to_string();
            expect_eq("the text of the key", key.text.as_str(), plain.as_str())?;
            expect_eq("a keyboard key is no posted string", &key.posted_text, &false)?;
            expect_eq(
                "the Option-free text of the key",
                &key.without_option.as_deref(),
                &Some(plain.as_str()),
            )
        };
        run().err()
    }

    pub fn a_control_chord_keeps_the_layouts_text_for_its_key() -> Option<String> {
        let run = || -> Result<(), String> {
            let key = native("\u{1}", 0, NSEventModifierFlags::Control)?;
            expect_eq("Control held", &key.modifiers.control, &true)?;
            let plain = key.unshifted.ok_or("the A key types no character")?.to_string();
            expect_eq(
                "the text before Control's transformation",
                key.text.as_str(),
                plain.as_str(),
            )
        };
        run().err()
    }

    /// The press of the key at macOS key code `code` with `flags`, as the keyboard sends it or as a process posts it
    /// (its Quartz string what the key types with no modifier, as Quartz fills it in), as the terminal reads it.
    fn press(
        name: &'static str,
        code: u16,
        flags: NSEventModifierFlags,
        posted_by_us: bool,
    ) -> Result<KeyStroke, String> {
        let plain = match code {
            0 => "a",
            1 => "s",
            14 => "e",
            18 => "1",
            32 => "u",
            45 => "n",
            49 => " ",
            51 => "\u{7f}",
            53 => "\u{1b}",
            _ => return Err(format!("no plain text for key code {code}")),
        };
        let event = if posted_by_us {
            posted(plain, code, flags)?
        } else {
            from_hardware(plain, code, flags)?
        };
        let keystroke = Keystroke::parse(name).map_err(|err| format!("{name}: {err}"))?;
        // SAFETY: a live key event, on the main thread.
        let source = unsafe { event_source(std::ptr::from_ref(&*event).cast_mut().cast()) };
        Ok(KeyStroke::new(&keystroke, &source, false))
    }

    pub fn option_dead_keys_compose_with_the_next_key_and_leave_nothing_pending() -> Option<String> {
        let option = NSEventModifierFlags::Option;
        let none = NSEventModifierFlags::empty();
        let run = |posted_by_us: bool| -> Result<(), String> {
            let path = if posted_by_us { "posted" } else { "keyboard" };
            let sharp_s = press("alt-s", 1, option, posted_by_us)?;
            let acute = press("alt-e", 14, option, posted_by_us)?;
            if sharp_s.text != "ß" || acute.text != "´" {
                return Err(format!(
                    "the test wants the U.S. layout: Option-s typed {:?}, Option-e {:?}",
                    sharp_s.text, acute.text
                ));
            }
            let mut dead = DeadKeys::default();
            expect_eq(&format!("{path}: Option-s"), &dead.compose(&sharp_s), &Compose::Pass)?;
            for (accent, accent_key, next, next_key, composed) in [
                ("´", ("alt-e", 14), "e", ("e", 14), "é"),
                ("¨", ("alt-u", 32), "u", ("u", 32), "ü"),
                ("˜", ("alt-n", 45), "n", ("n", 45), "ñ"),
                ("¨", ("alt-u", 32), "a", ("a", 0), "ä"),
                ("´", ("alt-e", 14), "space", ("space", 49), "´"),
                ("´", ("alt-e", 14), "1", ("1", 18), "´1"),
            ] {
                let what = format!("{path}: {} then {next}", accent_key.0);
                let first = press(accent_key.0, accent_key.1, option, posted_by_us)?;
                expect_eq(
                    &format!("{what}, the dead key"),
                    &dead.compose(&first),
                    &Compose::Pending {
                        accent: accent.to_owned(),
                        committed: String::new(),
                    },
                )?;
                let second = press(next_key.0, next_key.1, none, posted_by_us)?;
                expect_eq(&what, &dead.compose(&second), &Compose::Composed(composed.to_owned()))?;
                expect_eq(
                    &format!("{what}, a key code is no posted string"),
                    &second.posted_text,
                    &false,
                )?;
                let after = press("1", 18, none, posted_by_us)?;
                expect_eq(&format!("{what}, the key after"), &dead.compose(&after), &Compose::Pass)?;
                expect_eq(&format!("{what}, its text"), after.text.as_str(), "1")?;
            }
            Ok(())
        };
        run(false).and_then(|()| run(true)).err()
    }

    pub fn backspace_or_escape_cancels_a_dead_key_and_a_posted_string_ends_it() -> Option<String> {
        let option = NSEventModifierFlags::Option;
        let none = NSEventModifierFlags::empty();
        let run = |posted_by_us: bool| -> Result<(), String> {
            let path = if posted_by_us { "posted" } else { "keyboard" };
            let mut dead = DeadKeys::default();
            for (name, code) in [("backspace", 51), ("escape", 53)] {
                dead.compose(&press("alt-e", 14, option, posted_by_us)?);
                let cancel = press(name, code, none, posted_by_us)?;
                expect_eq(
                    &format!("{path}: Option-e then {name}"),
                    &dead.compose(&cancel),
                    &Compose::Cancelled,
                )?;
                expect_eq(&format!("{path}: {name} again"), &dead.compose(&cancel), &Compose::Pass)?;
            }
            dead.compose(&press("alt-e", 14, option, posted_by_us)?);
            let keystroke = Keystroke::parse("a").map_err(|err| format!("a: {err}"))?;
            let event = posted("ж", 0, none)?;
            let string = KeyStroke::new(&keystroke, &Source::Native(read(&event)?), false);
            expect_eq(
                &format!("{path}: Option-e then a posted string"),
                &dead.compose(&string),
                &Compose::Dropped,
            )?;
            expect_eq(&format!("{path}: the posted string's text"), string.text.as_str(), "ж")?;
            expect_eq(&format!("{path}: nothing pending"), &dead.is_pending(), &false)?;
            let keystroke = Keystroke::parse("alt-e").map_err(|err| format!("alt-e: {err}"))?;
            let event = posted("ж", 14, option)?;
            let on_a_dead_key = KeyStroke::new(&keystroke, &Source::Native(read(&event)?), false);
            expect_eq(
                &format!("{path}: a posted string on Option-e"),
                &dead.compose(&on_a_dead_key),
                &Compose::Pass,
            )?;
            expect_eq(
                &format!("{path}: it is no composition"),
                &on_a_dead_key.composing,
                &false,
            )?;
            expect_eq(&format!("{path}: its text"), on_a_dead_key.text.as_str(), "ж")?;
            expect_eq(&format!("{path}: still nothing pending"), &dead.is_pending(), &false)
        };
        run(false).and_then(|()| run(true)).err()
    }
}
