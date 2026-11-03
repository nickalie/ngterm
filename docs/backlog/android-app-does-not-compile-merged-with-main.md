---
worth: yes
where: crates/ngterm-core/src/remote/origin_replies.rs:16
added: 2026-11-03
---
# android-app does not compile merged with main

PR #8 (`android-app` into `main`) fails to compile in its merge commit, so every PR run of android.yml and
cross-platform.yml is red there (e.g. run 41629367994) whatever the branch's own `ci/**` runs say.

main's cee241cb ("fix(remote): create split panes on the origin") added `crates/ngterm-core/src/remote/split.rs`,
which imports `read_remote_tree` through `super::attach` and `COMPANION_DEADLINE`, `PAUSE`, `READY_BUDGET` and
`reply_of` through `super::companions`. This branch moved those items into `remote/origin_replies.rs` (the
origin-reply plumbing the mobile core shares), and `attach.rs:21` and `companions.rs:25` now import them privately, so
split.rs's paths reach private imports and the merged crate fails to compile.

The fix belongs to the branch merge, not to Task 21's CI work: merge (or rebase onto) main, point split.rs's imports
at `origin_replies`, and check `ngterm-core` (clippy and the `commands::tests::remote_split` tests cee241cb added)
before merging the PR.
