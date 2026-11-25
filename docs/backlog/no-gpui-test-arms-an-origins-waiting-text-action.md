---
worth: yes
where: crates/ngterm-gpui/src/state.rs:1068
added: 2026-11-25
---
# No GPUI test arms an origin's waiting text action

A presenter runs an origin's text action (follow, search, wrap, changes) on a page it shows for that origin. When a
step waits on the page's renderer, the core keeps it until the frontend takes it (`Core::take_replica_text_steps`).
`AppState::arm_replica_text_steps`, which `settle` calls, puts each step on the main loop. The core tests run the
steps themselves, and the conformance case for log pages on a presenter is not on the ratchet yet. Thus no test that
CI runs fails if `settle` stops to arm these steps. Then the origin gets no answer for that action, and the page's
later text actions stay in its queue until the page closes.

Possible fix: a GPUI state test with a presentation viewer (a remote runner, a presentation transport and a page
host whose change step waits), as the core's `commands/tests/viewer.rs` fixture has, that sends an origin `changes`
action, answers the renderer, runs the main loop and reads the `page.result`. Or put the conformance case
`test_a_log_page_follows_on_the_presenting_machine_which_runs_the_origins_text_actions` on the ratchet once it runs.

Reported in the revmux review of task #945 (log pages on remote sessions: the presenter half), round 1.
