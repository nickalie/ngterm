---
worth: later
where: conformance/tools/check_coverage.py
added: 2026-11-03
---
# Do the coverage floors need per-crate allowances instead of one 0.5 pt tolerance?

Three CI runs of one build gave Linux ngterm-gpui 63.80-64.38% lines and ngterm-ghostty-vt 48.33-49.78% (timing in
the GUI and the terminal decides which paths run); the floor seeded from the first run broke on the third. Task 12 of
`docs/plans/20261031-e2e-ui-and-coverage.md` reseeded both floor files from the per-crate minimum of three same-build
runs and made `check_coverage.py --add` take a batch of at least two summaries (each floor raised to the batch's
lowest), keeping the 0.5 pt tolerance; Codex agreed and opposed a global 1.0 pt (ghostty's spread is 1.45 pt), but
two or three runs are no statistical bound (transcript
`~/.local/state/plan-exec-revmux/20261031-e2e-ui-and-coverage/discussions/task-12-coverage-floor-noise/transcript.md`).

The unknown that settles it: whether a further same-build run breaks a floor. If one does, measure the per-crate spread
over more runs and give the noisy crates their own allowance (or report them without blocking) while the flaky paths
are found; if none does, delete this item.
