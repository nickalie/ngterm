---
worth: later
where: scripts/lib/native.sh (ios_sim_quiet_keyboard_introduction)
added: 2026-11-09
---
# the slide-to-type preference doesn't always stop the keyboard's introduction

`ios_sim_quiet_keyboard_introduction` (build-ios.sh's `quiet_keyboard_introduction` before task 706) sets
`DidShowContinuousPathIntroduction`, but CI's simulators still showed the introduction on some runs after it was added
(runs 42455193990 and 42605169756), which swallowed `KeyboardTests`' taps.
`KeyboardTests` now closes the introduction itself, so nothing fails today.

Unknown: why the preference sometimes doesn't take. If no other UI test needs it, the function can go instead.
