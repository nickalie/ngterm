# Restart shutdown control requests

The newly unlocked `test_restart_drops_mail_sent_during_shutdown` exposed a real restart deadlock. The core called
`SessionHost::kill` synchronously on the app's main loop. The session server sends HUP and waits for the old process
before answering that call. An old program's HUP handler could invoke `ngtermctl msg send`, but that request needed
the blocked app loop. The session server's forced shutdown then killed the waiting handler, leaving its redirected
receipt empty. The test's successful-delivery assertion remains unchanged.

`commands/restart_live.rs` now runs the bounded host kill on a named worker and polls its result through the
existing `Dispatched::Pending` mechanism. The main loop can serve the ending program's control request. Mail is
still discarded after shutdown, immediately before the replacement is requested, so mail delivered during shutdown
never reaches the new program. The validation order, confirmed-kill requirement and existing failure text remain.

The asynchronous interval needs three safeguards:

- Claim the old attachment's exit and temporarily forget its lead before starting the worker. An `unowned` report
  cannot start a competing attachment. An unconfirmed kill restores the previous lead by pane identity, including
  after a swap or soft close while the worker waits.
- A separate scoped permit serializes restarts of the same pane through the final new-shell observation. The
  frontend can release the old terminal's exit claim when it installs the replacement while the permit still makes
  a second restart follow the existing five-second shell wait and 100 ms polling interval. A restart's final poll
  therefore cannot clear another kill's claim. The existing timeout response is preserved.
- A map active only during the pending host kill records the first suppressed exit's session and timestamp.
  Success discards it; failure releases the claim and replays the recorded exit through normal pane-exit handling
  at the token's current location. Thus an unconfirmed kill cannot swallow the only exit event and leave a dead pane.
  Spawn failure clears the map immediately. Other claimed-exit lifetimes are unchanged. A pane soft-closed during
  the kill is finalized without rebuilding it or restoring a dead pane through undo.

The reference is `.upstream/agterm/agterm/Control/ControlServer+Restart.swift`: `restartSessionPane` ends the daemon,
then awaits `programEnded` while other requests run, and disposes the mailbox immediately before replacement.
`Ghostty/ZmxClient.swift::killConfirmed` sends the old foreground job its hangup; the reference's synchronous kill
excludes overlapping entry into that phase. The Rust session host's different blocking behavior is visible in
`ngterm-session/src/server/api.rs::kill_one` and `server/pane.rs::{kill,end_tree}`. Its 250 ms forced-shutdown grace
is unchanged by this fix.

Validation:

- Deterministic channel-blocked host tests prove shutdown-message delivery while kill waits, mailbox disposal before
  replacement, suppression of competing lead reports, sequential overlapping restarts, normal exit processing after
  failed/stale kill results, and cleanup when the pane closes during the kill. They use no timing sleeps.
- The final live test module passes all 52 cases. Together with the 18 message and 9 restart cases, the directly
  relevant modules contain 79 passing cases; the full core run includes them all.
- `timeout 600 cargo test -p ngterm-core --lib -- --quiet`: **3452 passed, 0 failed, 2 ignored**, 2.18 seconds.
  Both ignores are existing `argv_recorder` child entrypoints used by shell argument and PowerShell replay tests.
- Scoped rustfmt and `git diff --check` pass. Final downstream strict clippy and the rebuilt black-box conformance
  result are recorded with the task's other validation artifacts.

Review correction: blocked-kill regressions now swap a pane before both stale and failed kill results, verify its
recorded exit closes the correct pane, and check lead restoration after a swap or soft close. The live module passes
54 tests at the minor-fix commit, including these two new cases; scoped strict core clippy passes.

The separate pre-existing overlap correction extends serialization beyond frontend installation through the final
restart reply. Its deterministic regression pauses A after replacement installation, polls B, finishes A's new-shell
observation, then verifies B's kill still claims its exit and preserves the pane and replacement lead. The permit is
owned by the pending continuation, so every refusal, kill failure, rebuild failure, old-program timeout, new-shell
timeout and successful reply releases it. A dropped callback or panic unwinding releases it too; a waiting restart
owns no permit and cannot release another restart's. Replacement callbacks still release only the old exit claim.
Additional regressions cover a dropped new-shell poll followed by another restart, retry after failed kills, and
expired waiter/owner deadlines without sleeps. Targeted final results are in [verification.md](verification.md).
