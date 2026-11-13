---
worth: later
where: conformance/tests/ui/test_terminal_bindings.py
added: 2026-11-13
---
# The Windows wheel-scroll ui test lost its wheel once

`tests/ui/test_terminal_bindings.py::test_the_wheel_scrolls_the_viewport` passed in 33 of 34 Windows conformance
runs (cross-platform.yml `windows`, 2026-11-07..13) and failed once, in main run 43259281468 (c9eccd0d): after
`ui.scroll(ui.window(), -5)` the viewport still showed the last row for the whole 10 s wait, so the five wheel
notches never scrolled the pane. Nothing in the test, `drivers/uia.py` or the wheel path changed since. It is held
by the Linux ratchet but left out of `conformance/rust-passing-windows.txt` (plan
`20261112-e2e-coverage-90.md` Task 47), since a ratchet id that fails by timing turns the Windows job red.

Open: whether SendInput's wheel went to another window (focus or the pointer's position at that moment) or the app
dropped it. Needs that run's trace (`rust-windows-conformance` artifact, expired after its retention) or a repeat
loop on the `win11` VM. Once it holds over a loop of runs, add the id with `check_ratchet.py --add`.
