---
worth: yes
where: crates/ngterm-app/src/remote.rs
added: 2026-11-27
---
# A multi-line ssh error does not collapse across retries

The log collapses a run of identical stderr lines of a presentation bridge (`Repeats` in
`crates/ngterm-app/src/remote.rs`). It compares each line only with the line before it. Some ssh failures write
more than one stderr line on each launch, for example a `kex_exchange_identification` line and then a
`Connection reset` line, or the block that tells that the host key changed. Then the lines alternate, each retry
logs all of its lines, and the count and the doubling wait do not apply.

Possible fix: for a bridge run, compare the full stderr block of a launch with the block of the launch before it,
and count identical blocks. Or keep a small set of recent lines for each row.

Reported in the revmux review of task #1042 (collapse repeated ssh stderr lines), round 3.
