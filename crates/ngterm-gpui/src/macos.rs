//! Native `AppKit` services. UI entry points are called only on GPUI's main thread.
use crate::state::{AppState, Poster, post};
use std::ffi::{c_char, c_void};
use std::sync::{Mutex, OnceLock};

unsafe extern "C" {
    pub fn ngterm_reduce_transparency() -> bool;
    pub fn ngterm_reduce_motion() -> bool;
    pub fn ngterm_minimized(view: *mut c_void) -> bool;
    pub fn ngterm_maximized(view: *mut c_void) -> bool;
    fn ngterm_hotkey(key: i32, mods: u32, callback: extern "C" fn()) -> i32;
    pub fn ngterm_key(
        mods: *mut u32,
        plain: *mut u32,
        command: *mut u32,
        text: *mut c_char,
        without_option: *mut c_char,
        capacity: u32,
    ) -> i32;
    fn ngterm_accessibility_start(changed: extern "C" fn(bool, bool));
}

/// Ghostty's macOS and XKB columns joined by physical key (keycodes.zig, Chromium BSD table).
pub const KEYCODES: &[(u32, u32)] = &[
    (0, 38),
    (1, 39),
    (2, 40),
    (3, 41),
    (4, 43),
    (5, 42),
    (6, 52),
    (7, 53),
    (8, 54),
    (9, 55),
    (10, 94),
    (11, 56),
    (12, 24),
    (13, 25),
    (14, 26),
    (15, 27),
    (16, 29),
    (17, 28),
    (18, 10),
    (19, 11),
    (20, 12),
    (21, 13),
    (22, 15),
    (23, 14),
    (24, 21),
    (25, 18),
    (26, 16),
    (27, 20),
    (28, 17),
    (29, 19),
    (30, 35),
    (31, 32),
    (32, 30),
    (33, 34),
    (34, 31),
    (35, 33),
    (36, 36),
    (37, 46),
    (38, 44),
    (39, 48),
    (40, 45),
    (41, 47),
    (42, 51),
    (43, 59),
    (44, 61),
    (45, 57),
    (46, 58),
    (47, 60),
    (48, 23),
    (49, 65),
    (50, 49),
    (51, 22),
    (53, 9),
    (54, 134),
    (55, 133),
    (56, 50),
    (57, 66),
    (58, 64),
    (59, 37),
    (60, 62),
    (61, 108),
    (62, 105),
    (64, 195),
    (65, 91),
    (67, 63),
    (69, 86),
    (71, 77),
    (72, 123),
    (73, 122),
    (74, 121),
    (75, 106),
    (76, 104),
    (78, 82),
    (79, 196),
    (80, 197),
    (81, 125),
    (82, 90),
    (83, 87),
    (84, 88),
    (85, 89),
    (86, 83),
    (87, 84),
    (88, 85),
    (89, 79),
    (90, 198),
    (91, 80),
    (92, 81),
    (93, 132),
    (94, 97),
    (95, 129),
    (96, 71),
    (97, 72),
    (98, 73),
    (99, 69),
    (100, 74),
    (101, 75),
    (102, 131),
    (103, 95),
    (104, 130),
    (105, 191),
    (106, 194),
    (107, 192),
    (109, 76),
    (110, 135),
    (111, 96),
    (113, 193),
    (114, 118),
    (115, 110),
    (116, 112),
    (117, 119),
    (118, 70),
    (119, 115),
    (120, 68),
    (121, 117),
    (122, 67),
    (123, 113),
    (124, 114),
    (125, 116),
    (126, 111),
];

static HOTKEY_POSTER: OnceLock<Mutex<Option<Poster>>> = OnceLock::new();
extern "C" fn hotkey_fired() {
    if let Some(poster) = HOTKEY_POSTER
        .get()
        .and_then(|slot| slot.lock().ok())
        .and_then(|slot| slot.clone())
    {
        post(&poster, AppState::global_hotkey_fired);
    }
}

pub fn hotkey(chord: Option<&ngterm_core::keymap::Chord>, poster: &Poster) {
    use ngterm_core::keymap::Modifiers;
    let slot = HOTKEY_POSTER.get_or_init(Mutex::default);
    *slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(poster.clone());
    let key = chord
        .and_then(|chord| crate::global_hotkey::keycode(&chord.key))
        .and_then(|code| KEYCODES.iter().find(|(_, xkb)| *xkb == code))
        .and_then(|(mac, _)| i32::try_from(*mac).ok());
    let mods = chord.map_or(0, |chord| {
        [
            (Modifiers::COMMAND, 1 << 8),
            (Modifiers::SHIFT, 1 << 9),
            (Modifiers::OPTION, 1 << 11),
            (Modifiers::CONTROL, 1 << 12),
        ]
        .into_iter()
        .filter(|(flag, _)| chord.mods.contains(*flag))
        .map(|(_, value)| value)
        .sum()
    });
    // SAFETY: called on the main thread; the callback is static and posts to the main loop.
    let status = unsafe { ngterm_hotkey(key.unwrap_or(-1), mods, hotkey_fired) };
    if status != 0 || (chord.is_some() && key.is_none()) {
        tracing::warn!(status, "macOS could not register the global hotkey");
    }
}

static ACCESSIBILITY: OnceLock<async_channel::Sender<(bool, bool)>> = OnceLock::new();
extern "C" fn accessibility_changed(transparency: bool, motion: bool) {
    if let Some(changes) = ACCESSIBILITY.get() {
        let _ = changes.try_send((transparency, motion));
    }
}

/// Follow the workspace's display-accessibility notification without an idle timer.
pub fn watch(poster: Poster, cx: &mut gpui_kit::App) {
    let (changes, queue) = async_channel::unbounded();
    if ACCESSIBILITY.set(changes).is_err() {
        return;
    }
    // SAFETY: main-thread registration retains only a static callback.
    unsafe { ngterm_accessibility_start(accessibility_changed) };
    cx.spawn(async move |cx| {
        while let Ok((reduce, motion)) = queue.recv().await {
            crate::status_glyph::set_motion(cx, !motion);
            cx.update(|cx| {
                if crate::reduce_transparency::set(reduce, cx) {
                    post(&poster, AppState::settle);
                }
            });
        }
    })
    .detach();
}

/// Whether this machine can render GPUI windows.
#[must_use]
pub fn graphics_available() -> bool {
    unsafe extern "C" {
        fn ngterm_metal_available() -> bool;
    }
    // SAFETY: a device availability query without application state.
    unsafe { ngterm_metal_available() }
}

#[cfg(test)]
#[path = "macos_tests.rs"]
mod tests;
