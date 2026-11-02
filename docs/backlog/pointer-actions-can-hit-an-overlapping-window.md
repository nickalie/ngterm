---
worth: yes
where: conformance/agterm_conformance/drivers/atspi.py:356
added: 2026-11-02
---
# Pointer actions can hit an overlapping window instead of their target

Save the first window's `sidebar-toggle-button` element, create a second window at the same
bounds, and leave the second raised and focused. `ui.mouse_click(saved_element)` reads the first
window's coordinates, but `_pointer_move` calls `_activate()` without the target and preserves
the second window's focus and stacking. The click can toggle the second sidebar. The menu-specific
activation fix does not protect general pointer actions.

Retain the resolved pointer target's containing X11 window and raise and focus it before input.
Add a real private-Xvfb regression with overlapping windows that asserts the target window changes
and the other does not; preserve existing keyboard-focus and menu-scoping regressions.

Introduced by Task 5 of `docs/plans/20261031-e2e-ui-and-coverage.md`, reported as major by revmux
round `e2e-ui-and-coverage-task-5/04-after-decision` (one correctness/implementation reviewer,
no degradation). This is a confirmed defect, not an unresolved product decision. The run stopped
before Task 6 because its normal review cap and one authorized additional fix/review were exhausted.
Task 5 remains blocked; this item must be fixed before continuing dependent tasks.

Review report:
`~/.local/state/plan-exec-revmux/20261031-e2e-ui-and-coverage/revmux-task-5-04-after-decision.json`.
Stop decision:
`~/.local/state/plan-exec-revmux/20261031-e2e-ui-and-coverage/discussions/decision-task5-final-review/transcript.md`.
