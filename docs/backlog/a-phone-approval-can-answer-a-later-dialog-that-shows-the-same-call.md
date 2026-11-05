---
worth: later
where: crates/ngterm-mobile/src/chat/input.rs:384
added: 2026-11-05
---
# A phone approval can answer a later dialog that shows the same call, and an MCP call's can't be answered at all

The phone answers an approval card by typing the option's key into the pane, but only after it reads the permission
dialog on screen and finds the card's call there (`Plan::approval`, `shown_subjects`, `Dialog::asks`): the command or
URL, or a file tool's path relative to the agent's working directory. Two gaps remain that screen text cannot close:

- **Same subject, different request.** Card A is answered in the terminal and the agent asks B, whose dialog shows the
  same text: the same command (in the same or another working directory), or another edit of the same file. Until the
  origin's `approval_done` for A (or B's `approval`) reaches the phone, a tap on A's still-drawn card sends the key to
  B. The window is the chat events' lag behind the terminal stream (the origin's follower polls the agent record).
- **Nothing to match.** A card whose call names nothing a dialog shows (an MCP tool's, Codex's `apply_patch` as its
  hook reports it) never confirms: it stays up with "Answer in terminal".

Surfaced by the revmux review of Task 18 (round 2) of `docs/plans/20261103-android-agent-chat.md`; the fixer kept the
screen check, made unconfirmed cards stay up instead of being dropped, and carried the hook's `cwd` so Edit/Write
approvals confirm by their relative path.

Options discussed with Codex (transcript
`~/.local/state/plan-exec-revmux/20261103-android-agent-chat/discussions/fixer-approval-tie/transcript.md`):

- A request-bound answer: the agent's `PermissionRequest` hook returns the decision (Claude's hook can answer
  allow/deny) while the phone's tap reaches it through the origin, so the answer names the request instead of typing a
  key into whatever dialog is up. Closes both gaps; a redesign (the hook would block on the phone, Codex needs its own
  path), not a review fix.
- An origin-side check of the card's approval id before the keys go. Narrows the lag window but is not authoritative:
  Claude has no "permission answered" hook, so the record may keep an answered candidate while the pane stays
  `blocked` (`crates/ngterm-core/src/agent_chat/approvals.rs`), and the keys still travel after the check. Codex's
  point: useful, insufficient alone.

The unknown that settles it: whether the request-bound path is worth its cost (a blocking hook per approval, a new
origin command, Codex parity) against a race that needs the terminal and the phone answering within the event lag.
Meanwhile: screen confirmation as above; nothing to undo.
