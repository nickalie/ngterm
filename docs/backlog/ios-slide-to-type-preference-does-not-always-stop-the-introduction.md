---
worth: later
where: scripts/build-ios.sh:320
added: 2026-11-09
---
# the slide-to-type preference doesn't always stop the keyboard's introduction

`quiet_keyboard_introduction` sets `DidShowContinuousPathIntroduction`, but CI's simulators still showed the
introduction on some runs after it was added (runs 42455193990 and 42605169756), which swallowed `KeyboardTests`' taps.
`KeyboardTests` now closes the introduction itself, so nothing fails today.

Unknown: why the preference sometimes doesn't take. If no other UI test needs it, the function can go instead.
