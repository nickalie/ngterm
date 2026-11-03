---
worth: yes
where: crates/ngterm-mobile/src/pane.rs:759
added: 2026-11-03
---
# A mobile pane's exit or channel loss goes unseen behind an unanswered clipboard read

`feed_pane` (ngterm-mobile) reads a pane's ssh channel on one thread and hands each chunk to
`StreamFeed::deliver`, which waits while the pane's mailbox is past its high mark. A program that issues an OSC 52
read stalls the terminal worker until the app answers the prompt, so if the program then writes 1 MiB or more and
exits (or its channel alone goes), the feeder sits in `deliver` and never reads the `ChannelOutput::End` behind
that output. Neither `feed.exited` nor `Note::Ended` happens, so nothing denies the pending read, a channel-only
loss never arms its retry, and the exit or loss stays unreported until the user answers the prompt or hides the
pane. `Panes::machine_down` leaves the pending read untouched too (a whole-connection loss recovers on
`machine_up`, whose re-attach denies it).

Reviewer's suggested fix: observe channel termination independently of blocked terminal delivery, and release
pending clipboard reads before waiting for output to drain.

Not fixed in the run: seeing the end without draining the output before it needs a second reader that keeps
reading the channel while delivery is blocked, which either buffers the remote's output without bound (dropping the
ssh-window backpressure the mailbox's high mark exists for) or needs an out-of-band close signal from the
transport (`ChannelReader` delivers the end in order after the data). Either is a redesign of the feeder and the
transport seam, not a side fix. Meanwhile the pane waits for the prompt's answer, as it does now.

Reported by the revmux review of `docs/plans/20261102-android-app.md`, Task 9c, round 1 (pre-existing finding).
