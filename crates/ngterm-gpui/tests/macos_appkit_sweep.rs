//! What `AppKit` does to the app by itself, on demand: calls that arrive when the system pleases (a menu opening, the
//! system's `UIIntelligenceSupport` scan simulating every menu's opening, a click on an Edit item), made here at once
//! against the real menu bar on GPUI's real app delegate. A main thread stuck in one of them fails the run with its
//! stack and the step it was on, rather than leaving a frozen window to find later. Key equivalents are not driven yet.
//!
//! No window opens, so this needs neither a Metal device nor an Accessibility grant, and it runs without a harness
//! because `AppKit` wants the main thread. A new kind of system callback is one more step in [`sweep`].
#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() {
    sweep::run();
}

#[cfg(target_os = "macos")]
mod sweep {
    use std::fs;
    use std::process::{Command, exit};
    use std::sync::Mutex;
    use std::thread;
    use std::time::Duration;

    use gpui_kit::QuitMode;
    use ngterm_core::model::WindowLibrary;
    use ngterm_core::persist::AppSettings;
    use ngterm_core::{Core, CoreConfig, CoreState};
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, Sel};
    use objc2::{class, msg_send};
    use objc2_foundation::NSString;

    /// Far beyond any of these calls; only a main thread that will never return reaches it.
    const HANG_LIMIT: Duration = Duration::from_secs(30);

    /// The step in progress, for the watchdog's report.
    static STEP: Mutex<String> = Mutex::new(String::new());

    fn step(text: String) {
        if let Ok(mut current) = STEP.lock() {
            *current = text;
        }
    }

    /// The main thread's part of `sample`'s report on this process.
    fn main_thread_stack() -> String {
        let Ok(dir) = tempfile::tempdir() else {
            return "no temporary directory for sample".to_owned();
        };
        let report = dir.path().join("sample.txt");
        let sampled = Command::new("/usr/bin/sample")
            .arg(std::process::id().to_string())
            .arg("1")
            .arg("-file")
            .arg(&report)
            .output();
        if let Err(error) = sampled {
            return format!("sample did not run: {error}");
        }
        let text = fs::read_to_string(&report).unwrap_or_default();
        let lines = text.lines().skip_while(|line| !line.contains("com.apple.main-thread"));
        let mut stack = String::new();
        for (index, line) in lines.enumerate() {
            if index > 0 && line.contains("Thread_") {
                break;
            }
            stack.push_str(line);
            stack.push('\n');
        }
        stack
    }

    /// Fails the run with the main thread's stack once [`HANG_LIMIT`] has passed: the sweep exits first when it returns.
    fn start_watchdog() {
        thread::spawn(|| {
            thread::sleep(HANG_LIMIT);
            let current = STEP.lock().map(|text| text.clone()).unwrap_or_default();
            eprintln!(
                "macos_appkit_sweep: the main thread hung during: {current}\n{}",
                main_thread_stack()
            );
            exit(1);
        });
    }

    /// The real menu bar's model, from a core with one seeded window.
    fn install_menu_bar() {
        let core = Core::new(
            CoreConfig {
                app: ngterm_proto::AppIdentity::from_recorded("0.1.0", None),
                capabilities: Vec::new(),
            },
            CoreState {
                library: WindowLibrary::seeded("/tmp"),
                settings: AppSettings::default(),
                persistence: None,
            },
        );
        ngterm_gpui::app_menu::macos::sync(&core);
    }

    /// The items of `menu`.
    fn items_of(menu: &AnyObject) -> Vec<Retained<AnyObject>> {
        // SAFETY: main thread; a live `NSMenu`, and this message's signature.
        let count: isize = unsafe { msg_send![menu, numberOfItems] };
        (0..count)
            .map(|index| {
                // SAFETY: main thread; `index` is below the menu's item count, so an item comes back.
                unsafe { msg_send![menu, itemAtIndex: index] }
            })
            .collect()
    }

    /// `menu` and every menu below it, each named by the item that opens it: a menu's own title is often empty.
    fn menus_below(name: &str, menu: Retained<AnyObject>, out: &mut Vec<(String, Retained<AnyObject>)>) {
        for item in items_of(&menu) {
            // SAFETY: main thread; a live `NSMenuItem`, whose submenu may be nil.
            let submenu: Option<Retained<AnyObject>> = unsafe { msg_send![&*item, submenu] };
            if let Some(submenu) = submenu {
                menus_below(&title_of(&item), submenu, out);
            }
        }
        out.push((name.to_owned(), menu));
    }

    fn title_of(object: &AnyObject) -> String {
        // SAFETY: main thread; `NSMenu` and `NSMenuItem` both answer `title` with a string.
        let title: Retained<NSString> = unsafe { msg_send![object, title] };
        title.to_string()
    }

    /// What opening `menu` does: `AppKit` validates each item against its target, here through the responder chain.
    fn validate(name: &str, menu: &AnyObject) {
        step(format!("validating menu {name:?}"));
        // SAFETY: main thread; a live `NSMenu`.
        let () = unsafe { msg_send![menu, update] };
    }

    /// The action of `item` when the responder chain hands it to `delegate`: no target of its own, and that one found.
    fn action_reaching(item: &AnyObject, app: &AnyObject, delegate: &AnyObject) -> Option<Sel> {
        // SAFETY: main thread; a live `NSMenuItem`, whose target may be nil.
        let target: *mut AnyObject = unsafe { msg_send![item, target] };
        // SAFETY: as above; its action may be null.
        let action: Option<Sel> = unsafe { msg_send![item, action] };
        let action = action.filter(|_| target.is_null())?;
        // SAFETY: main thread; only looks up the responder chain, sending nothing.
        let resolved: *mut AnyObject = unsafe { msg_send![app, targetForAction: action] };
        std::ptr::eq(resolved, delegate).then_some(action)
    }

    /// Sends each item of `menu` whose action the responder chain hands to GPUI's delegate, as a click on it does, and
    /// counts them. Other targets are left alone: their actions hide or quit the process.
    fn send_actions_reaching_gpui(menu: &AnyObject, app: &AnyObject, delegate: &AnyObject) -> usize {
        let mut reached = 0;
        for item in items_of(menu) {
            let Some(action) = action_reaching(&item, app, delegate) else {
                continue;
            };
            let title = title_of(&item);
            // SAFETY: main thread; a live `NSMenuItem`.
            let enabled: bool = unsafe { msg_send![&*item, isEnabled] };
            assert!(
                !enabled,
                "{title:?} has no action in GPUI, and validating it must disable it"
            );
            step(format!("sending the action of {title:?}"));
            let to = std::ptr::null_mut::<AnyObject>();
            // SAFETY: main thread; a nil target sends it down the responder chain, as the item's click does.
            let sent: bool = unsafe { msg_send![app, sendAction: action, to: to, from: &*item] };
            assert!(sent, "{title:?}'s action found no target");
            reached += 1;
        }
        reached
    }

    /// Validates every menu, then sends every action that reaches GPUI's delegate, on the main thread with the app
    /// launched.
    fn sweep() {
        install_menu_bar();
        // SAFETY: main thread; the shared application exists from here on.
        let app: Retained<AnyObject> = unsafe { msg_send![class!(NSApplication), sharedApplication] };
        // SAFETY: main thread, past `applicationDidFinishLaunching`: GPUI's delegate is set.
        let delegate: Retained<AnyObject> = unsafe { msg_send![&*app, delegate] };
        // SAFETY: main thread; `install_menu_bar` set the main menu.
        let bar: Retained<AnyObject> = unsafe { msg_send![&*app, mainMenu] };
        let mut menus = Vec::new();
        menus_below("the menu bar", bar, &mut menus);
        for (name, menu) in &menus {
            validate(name, menu);
        }
        let reached: usize = menus
            .iter()
            .map(|(_, menu)| send_actions_reaching_gpui(menu, &app, &delegate))
            .sum();
        assert!(
            reached > 0,
            "no menu item reaches GPUI's delegate: this sweep no longer tests that path"
        );
        println!(
            "macos_appkit_sweep: ok: {} menus validated, {reached} actions sent to GPUI's delegate",
            menus.len()
        );
    }

    pub fn run() {
        start_watchdog();
        gpui_kit::application().with_quit_mode(QuitMode::Explicit).run(|_| {
            sweep();
            exit(0);
        });
    }
}
