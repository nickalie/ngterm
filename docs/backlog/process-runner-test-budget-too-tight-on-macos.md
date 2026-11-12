---
worth: yes
where: crates/ngterm-core/src/remote/tree.rs:1055
added: 2026-11-10
---
# a remote process runner test's 300 ms budget is too tight on a loaded Mac

`remote::tree::tests::the_process_runner_captures_output_and_bounds_the_wait` failed on `crates (macos-26)` in
CI run 42673623143 (commit 28c5938c): the stalled command's `printf` output hadn't been read when its 300 ms deadline
ran out, so the error came back as "the remote did not answer in time" without the ssh line it should carry. Seen once.
The fix is likely a wait on the output rather than a fixed budget, without weakening what the assertion checks.
