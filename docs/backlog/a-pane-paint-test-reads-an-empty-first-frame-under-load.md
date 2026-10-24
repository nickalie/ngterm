---
worth: yes
where: crates/ngterm-term/src/pane.rs:1239
added: 2026-10-24
---
# A pane paint test reads an empty first frame under load

`pane::tests::a_view_that_paints_while_another_does_never_publishes_older_rows` failed once in a headless
`cargo test --workspace` on 2026-10-24 with `index out of bounds: the len is 0 but the index is 0` at
`first.lines()[0]`: the first `pane.frame()` right after the direct `vt_write(b"abc")` came back as the stale
initial frame, as the IO thread still held a lock, while the conformance selftests loaded the 4-core agent slice.
Five runs of the test alone passed. The test should wait for a fresh frame before reading its rows, as its neighbour
`a_busy_lock_keeps_the_previous_frame_and_marks_it_stale` waits out `stale`. Surfaced by the gate of Task 22 of
`docs/plans/20261023-upstream-sync-v0.38.1.md`, whose change touches no `ngterm-term` code, so it was left as it was.
