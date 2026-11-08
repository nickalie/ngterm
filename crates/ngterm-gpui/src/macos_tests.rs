//! Unregistered hotkeys still replace their callback destination, and a native callback queues the quick toggle.
use std::sync::Arc;
use std::time::Duration;

use ngterm_core::keymap::{Chord, Modifiers};
use ngterm_core::{Core, CoreConfig, CoreState};
use ngterm_host::{MainLoop, MainLoopClosed, MainTask};

use super::*;

#[derive(Default)]
struct Posted(Mutex<Vec<MainTask<AppState>>>);

impl MainLoop<AppState> for Posted {
    fn post(&self, task: MainTask<AppState>) -> Result<(), MainLoopClosed> {
        self.0.lock().unwrap().push(task);
        Ok(())
    }

    fn post_after(&self, _: Duration, _: MainTask<AppState>) -> Result<(), MainLoopClosed> {
        panic!("the native hotkey must post immediately");
    }
}

fn state() -> AppState {
    AppState::new(Core::new(
        CoreConfig {
            app: ngterm_proto::AppIdentity::from_recorded("1.2.3", None),
            capabilities: Vec::new(),
        },
        CoreState {
            library: ngterm_core::model::WindowLibrary::seeded("/tmp"),
            ..CoreState::default()
        },
    ))
}

fn toggle(posted: &Posted, state: &mut AppState) {
    let tasks = std::mem::take(&mut *posted.0.lock().unwrap());
    assert_eq!(tasks.len(), 1, "one callback queues one toggle");
    let visible = state.core.quick_visible();
    for task in tasks {
        task(state);
    }
    assert_eq!(state.core.quick_visible(), !visible);
}

/// Set only in the child process that holds the process-wide hotkey slot alone.
const CHILD: &str = "W_GPUI_MACOS_HOTKEY_CHILD";

// every `Windows::sync` of a parallel GPUI test replaces the slot, so the callbacks are checked in a process of their own
#[test]
fn unregistered_callbacks_toggle_on_the_latest_live_loop_only() {
    let mut child = std::process::Command::new(std::env::current_exe().unwrap());
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("NGTERM_") {
            child.env_remove(key);
        }
    }
    let output = child
        .args([
            "--exact",
            "macos::tests::unregistered_callbacks_child",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD, "1")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "run by unregistered_callbacks_toggle_on_the_latest_live_loop_only"]
fn unregistered_callbacks_child() {
    if std::env::var_os(CHILD).is_none() {
        return;
    }
    hotkey_fired();
    let first = Arc::new(Posted::default());
    let held: Arc<dyn MainLoop<AppState>> = first.clone();
    // no chord here resolves a key, so native code only swaps its callback: it makes no Carbon call off the main thread,
    // and this test cannot see the key or modifier mask a real chord registers
    hotkey(None, &Arc::downgrade(&held));
    assert!(first.0.lock().unwrap().is_empty(), "unregistering does not fire");
    hotkey_fired();
    let mut state = state();
    toggle(&first, &mut state);

    let latest = Arc::new(Posted::default());
    let held_latest: Arc<dyn MainLoop<AppState>> = latest.clone();
    let unsupported = Chord {
        key: "not-a-key".to_owned(),
        mods: Modifiers::empty(),
    };
    hotkey(Some(&unsupported), &Arc::downgrade(&held_latest));
    assert!(latest.0.lock().unwrap().is_empty(), "a rejected chord does not fire");
    hotkey_fired();
    toggle(&latest, &mut state);
    assert!(first.0.lock().unwrap().is_empty(), "the old destination is retired");
    drop(held_latest);
    drop(latest);
    hotkey_fired();
    assert!(
        first.0.lock().unwrap().is_empty(),
        "a dead destination does not revive the old loop"
    );
    hotkey(None, &crate::state::detached_poster());
    *HOTKEY_POSTER.get().unwrap().lock().unwrap() = None;
    hotkey_fired();
}
