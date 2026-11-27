---
worth: yes
where: crates/ngterm-core/src/commands/tests/remote_split.rs:394
added: 2026-11-27
---
# Two remote split retry tests fail under load

`remote_split::a_transient_listing_failure_is_retried_without_recreating_the_origin_split` (line 394) and
`remote_split::closing_the_sibling_during_creation_does_not_change_the_requested_origin_daemon` (line 410) failed
once in a `cargo test -p ngterm-core` that took 110 s, while a release build for the conformance suite ran beside
it. Both tests wait for a second listing after `PAUSE` (1 s). In both, `Remote::spawn` got a grant, but its command
did not name the target daemon: the split flow failed (`fail_remote_split`) instead of attaching on the retry. The
next full run (22 s) passed, and nine runs of `remote_split` alone, six of them at the same time, passed. The cause
is not found yet: find which `fail_remote_split` reason the flow gives under load (log it in the test on failure),
then make the test wait on that state, not on wall time. Surfaced by the review fix of task #1044, whose change does
not touch `remote/split.rs`, so it was left as it was.
