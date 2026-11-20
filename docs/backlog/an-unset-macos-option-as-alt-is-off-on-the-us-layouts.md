---
worth: yes
where: crates/ngterm-core/src/ghostty_config/model.rs:84
added: 2026-11-21
---
# an unset macos-option-as-alt is off on the U.S. layouts

`MacosOptionAsAlt` has the default `Off` and no unset value, and the GPUI frontend gives it to `with_option_as_alt`
as it is (`crates/ngterm-gpui/src/terminal/keyboard.rs`). Ghostty, and thus upstream, makes an unset key Alt on the
`com.apple.keylayout.US` and `USInternational` layouts and off on the others (`src/input/keyboard.zig`
`detectOptionAsAlt` at the libghostty-vt pin). On a U.S. layout Mac with no config line, ⌥s in ngterm types `ß`,
where upstream sends `ESC s`. The conformance test `test_option_is_alt_by_default_on_the_us_layouts`
(`conformance/tests/ui/test_pane_input.py`) asserts upstream's behavior and fails on the Rust build on such a Mac.

The fix: an unset state in the config model, and on macOS a layout check through
`TISCopyCurrentKeyboardLayoutInputSource` (`crates/ngterm-gpui/src/macos.m` already reads the layout there), read
again when the input source changes. Found in task #770 (tests only, no product code) and its review round 1.
