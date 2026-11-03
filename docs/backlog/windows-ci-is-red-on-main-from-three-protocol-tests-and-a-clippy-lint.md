---
worth: yes
where: conformance/tests/protocol/test_companions.py
added: 2026-11-03
---
# Windows CI is red on main: three ratchet ids fail intermittently and the arm64 clippy step fails

The `cross-platform` workflow's `windows` job breaks the Windows ratchet on `main` and on every branch run, from
protocol tests only:

- `tests/protocol/test_companions.py::test_a_parent_whose_shell_exits_restarts_it[plain]` (the parent's restarted
  shell never prints within 10 s),
- `tests/protocol/test_companions.py::test_page_companion_opens_a_page_row_with_no_shell_under_its_parent` (the
  control socket stops answering for 10 s),
- `tests/protocol/test_config_browser.py::test_clear_empties_the_store_persistent_pages_share` (`agt=kept` survives
  the clear).

Seen on main runs 41489621619, 41504258836, 41512502156 and 41549054401 (one to three of them per run), and on the
`ci/win-task7` runs of Task 7 of `docs/plans/20261031-e2e-ui-and-coverage.md`, whose ui layer itself ran green (41 of
41 ui ids). The `crates (windows-2025, aarch64-pc-windows-msvc)` job fails its clippy step on `main` too:
`clippy::chunks_exact` ("using `chunks_exact` with a constant chunk size") in `ngterm-session`.

Task 7 did not fix them: they are outside the ui layer and predate it. Each flaky ratchet id is a build or test bug
(CLAUDE.md > What CI enforces); the companion ones need a Windows run with the instance logs the job uploads
(`rust-windows-conformance/instances`).
