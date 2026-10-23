---
worth: yes
where: crates/ngterm-wpe/tests/headless/cases.rs:448
added: 2026-10-23
---
# The WPE headless input case misses its wheel event in the shm worker mode

`pointer_keyboard_and_wheel_input_reach_the_page` (`ngterm-wpe --test headless`) failed twice in a row in the
`shm (worker)` mode with `timed out waiting for the title "wheel" (the title is Some("key:a:shift"))`, while the
other three modes passed in the same runs; a third run of the same binary passed. The page got the clicks and the
keys, so the scroll sent right after the shifted key is the one that went missing (dropped, or delivered before the
page's listener could see it) in the worker process's SHM path.

Seen during the upstream v0.38.1 sync, Task 2 (`docs/plans/20261023-upstream-sync-v0.38.1.md`), on the 4-CPU agent
cgroup cap, with the workspace tests and, once, a separate rerun of the binary alone; Task 2 changes only the bundled
skill's text, which `ngterm-wpe` never reads. CLAUDE.md > Gates: a flaky test is a bug in the build or the test, so
find whether `Input::Scroll` in the worker mode can be lost (an unrealized or unfocused view, a frame not yet
committed, the IPC ordering against the key events) and fix that rather than retrying.
