---
worth: no
where: crates/ngterm-headless/src/relaunch.rs:72
added: 2026-10-31
---
# ngterm-headless's exec restart leaves the old image's children unreaped

A confirmed Live sessions reset restarts `ngterm-headless` on Unix with `exec`: the pid and the children stay, the
threads that reaped them (`ngterm-session`'s spawn reaps the session server it starts on a thread,
`server/spawn/linux.rs`; `ngterm-app`'s hook runner one per hook) do not. The new image holds no handle on them, so the
session server, when it later exits (the namespace empties, a handoff), stays `<defunct>` until the instance exits;
at most one more per reset. Raised by the round-2 review of the headless branch.

Not fixed on purpose: the restart that avoids it (start a new process, then exit, as the GPUI app and the Windows form
do) changes the pid, and a service manager tracking the instance (systemd's main pid) takes the exit for the
service's end mid-reset. A zombie holds a pid slot and nothing else. Reaping them in the new image needs the inherited
children's pids, which only `/proc/self/task/*/children` (Linux) or `proc_listchildpids` (macOS) list: worth
reopening only if resets turn frequent on long-running servers.
