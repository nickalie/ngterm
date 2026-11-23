---
worth: yes
where: crates/ngterm-gpui/src/app.rs:587
added: 2026-11-23
---
# A quit drops the split changes still queued for the origin

A viewer sends the split changes of a remote row to its origin one line at a time. Only `remote_tick` moves the queue
(`advance_split_mirrors` in `crates/ngterm-core/src/remote/split_mirror.rs`). The app quits when its last window
closes, and the remote tick does nothing while `quitting` is set. `shut_down` does not send the lines that stay in
the queue. Thus, with one window: hide the split, close the split, then close the window, all in one ssh round trip.
The app quits before the `split close` goes out, and the next attach shows the closed pane again.

The same is true for all remote work that is in flight at quit, before task #892 too. A fix for the split queue only
does not fix the general problem.

Possible fix: at quit, wait for the remote work in flight with a bounded budget (for the split queue: call
`advance_split_mirrors` until the queue is empty or the budget ends), or give the remaining lines to a detached
process. The budget delays the quit, so it needs a decision on how long a quit can wait.

Reported in the revmux review of task #892 (a closed second pane returns on remote attach), round 4.
