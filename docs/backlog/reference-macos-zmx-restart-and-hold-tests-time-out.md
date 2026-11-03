---
worth: yes
where: conformance/tests/protocol/test_zmx.py
added: 2026-11-03
---
# reference-macos: two zmx protocol tests time out against upstream's release

`reference-macos.yml`'s protocol layer fails against upstream v0.39.1 on the runner, with the suite's own `main`
harness as much as with this branch's:

- `tests/protocol/test_zmx.py::test_restart_command_interrupted_by_ctrl_c_leaves_a_prompt` (the tree never shows the
  one session with a prompt within 10 s),
- `tests/protocol/test_zmx.py::test_a_live_command_session_closes_when_its_command_exits_and_holds_with_wait`
  (`held-output` never appears in `session.text` within 10 s).

Seen in all three runs of 2026-11-03: run 41596241267 (both attempts, branch `ci/verify-task12`) and run 41603323048
(`main`, b5e085cd). Each run also had one other live/zmx red that varied (`test_a_parent_whose_shell_exits_restarts_it[live]`,
`test_restart_without_a_command_replays_the_foreground_program`, `tests/cli/test_cli_output.py::test_live_pane_commands`).
A reference red means the test or its wait, not upstream, is wrong for that runner (the suite's reference discipline):
the zmx wrapper's timing on the macOS image is the likely cause. Found by Task 12 of
`docs/plans/20261031-e2e-ui-and-coverage.md`, which verifies the ui layer; these protocol tests are outside that plan.
