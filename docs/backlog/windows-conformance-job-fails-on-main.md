---
worth: later
added: 2026-11-03
---
# windows conformance job fails on main

cross-platform.yml's `windows` conformance job is red on three ratchet ids, all listed in
`conformance/rust-passing-windows.txt`. These are pre-existing Windows conformance failures: each also fails on
revisions that hold no android-app commit, so they are not this branch's doing. Their root causes are not
established.

- `tests/protocol/test_companions.py::test_page_companion_opens_a_page_row_with_no_shell_under_its_parent`:
  ClientTimeout. Failed in main runs 41609564950 (f61ab657) and 41639206173 (83173d75), in 41612583486
  (`ci/zmx-cleanup`) and in 41596231921 (`ci/verify-task12`).
- `tests/protocol/test_config_browser.py::test_clear_empties_the_store_persistent_pages_share`:
  `assert 'agt=kept' is None`. Failed in 41612583486 and 41596231921.
- `tests/protocol/test_companions.py::test_a_parent_whose_shell_exits_restarts_it[plain]`: WaitTimeout. Failed in
  41609564950 and 41596231921.

Surfaced while closing the android-app plan's Task 21 (CI), whose cross-platform run 41641582502 hit the same
`windows` job failure at 544b6555 while every crates leg passed.

Open question that settles the value call: whether each id is a real Windows defect in the companion and browser
code (fix it) or a ratchet entry that never held on main's Windows runners (drop it from the ratchet). Each needs its
root cause found on a Windows runner or the `win11` VM before either call.
