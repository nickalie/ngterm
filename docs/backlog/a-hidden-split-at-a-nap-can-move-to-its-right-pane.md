---
worth: yes
where: crates/ngterm-core/src/auto_follow.rs:338
added: 2026-11-27
---
# A hidden split at a nap can move to its right pane

The team lead's pane is the main pane of a session with a hidden split. After a nap (`teamlead nap wake`: a new pin
and a `session restart --pane-id` of the main pane), the window once showed the right pane, and the tree said
`splitFocused=true`. Task #1056 did not reproduce this. The exact nap call keeps the main pane in the real app, on the
old build and on the new build.

Before task #1056, a relaunch always restored `split.focused = false`. Thus a writer after the last relaunch set the
focus to the right pane. Since task #1056 the snapshot keeps `splitFocused`, so a wrong right-pane focus now also
stays across a relaunch.

Writers to examine:

- `auto_follow::reveal_pane` with `StatusPane::Right`: an indicator `status_pane` of Right, for example from a status
  write whose pane token or role names the right pane, when the user selects the session or auto-follow reveals it.
- `commands/notify.rs`: a notification click on the split pane.
- `commands/dashboard.rs`: a dashboard member on the split surface.
- `commands/overlay_page.rs` and `state.rs` `select_ask_pane`: an overlay or an ask on the right pane.
- `model/panes.rs` `swap_panes`: a swap flips the focus.

Fix: find the writer, add a test that makes the jump and fails on the current code, then correct it.

Reported in the revmux review of task #1056, round 1.
