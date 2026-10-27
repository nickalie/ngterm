# Plan template

The plan goes to `docs/plans/<YYYYMMDD>-upstream-sync-<new tag>.md` and is run by `/plan-exec-revmux`, which reads
the `## Overview` and `## Context` sections, then executes each `### Task N:` section in a fresh subagent: every
`- [ ]` item under a task is that subagent's work list, it ticks them, runs the validation the task names and
commits. So each task must be self-contained (a subagent sees only the plan file and the repo), name exact files
and commands, and end with its own gate. Write it in English, like the repository's other plans.

Everything in `<...>` is filled in; drop sections that would stay empty except Safety rules and the last task.

````markdown
# Upstream sync <old tag> -> <new tag>

## Overview

- Bring ngterm from upstream `umputun/agterm` <old tag> to <new tag> (<N> commits, releases <list>). The
  mechanical part is done on branch `upstream-sync-<new tag>` (commit `<sha>`): catalog and ui-ids regenerated,
  upstream's pins <moved: what | unchanged>, vendored copies <synced | conflicts: which>, parity-target mentions bumped.
- This plan ports the behavior: <one line per task group>.
- Out of scope: <the no-port list, one line, or "upstream CI, packaging and release metadata">.

## Context (from the sync)

- Upstream range: `.upstream/agterm` at <new tag> (`<sha>`); diff `git -C .upstream/agterm diff <old>..<new>`.
  Changelog of the range: <summary, a bullet per feature/fix with its PR number>.
- Reference rule (CLAUDE.md): upstream is the reference for protocol, semantics, error strings, `agtermctl`,
  on-disk formats, model and UI.
- Baseline after the mechanical commit:
  - `cargo fmt --check`: <ok | ...>; clippy: <ok | ...>; `scripts/lint-report.sh`: <ok | ...>
  - `cargo test` on the crates the bump touches (under xvfb): <ok | the failing tests, each with the task that turns
    it green>
  - coverage gate: <ok | the uncovered keys, each with its task>
  - `run-linux.sh -k <expr>` (partial, or "not run: no app behavior touched"): ratchet <holds | breaks: ids and
    cause>; newly green: <ids added | none>
- Already in the Rust build (no task): <feature — evidence: file / ratchet id>, or "nothing".
- Not run on this host: <the ratchet's `not run on this host` ids and why (no zsh login shell)>, or "none".
- Upstream's pins (none is a pin here): ZMX_REV and zmx-patches <unchanged | a -> b and the changed patches, a task
  ports them into ngterm-session>; GHOSTTY_REV <unchanged | a -> b, a task reviews it against LIBGHOSTTY_VT_REV>.

## Safety rules

<the block between the markers in references/safety.md, verbatim>

## Development Approach

- Each task follows CLAUDE.md > "Porting a command group": read upstream's implementation, its Swift tests (exact
  strings, edge cases, field order), `site/commands.html` and the matching `.claude/rules/*.md`; implement in
  `ngterm-core` against the `ngterm-host` fakes with unit tests asserting upstream's full error string; wire the
  app side (`ngterm-gpui`); add the `ngtermctl` part, ported from `agtermctlKit`; claim a capability only once its commands implement it.
- New conformance tests follow conformance/README.md > Reference discipline: a test is accepted only when green on
  the reference. reference-macos runs only in CI on macOS (`reference-macos.yml`), so a task that adds a suite test
  marks "green on reference-macos" as a manual check for the user.
- Gate of every task (CLAUDE.md > Porting a command group, step 6): README > Test's checks on the task's changed part,
  scoped as README > Test says (the window tests under `xvfb-run` and `dbus-run-session`), each pytest command with
  `PYTHONPATH=conformance` in front; then
  `PYTHONPATH=conformance .venv/bin/python -m pytest conformance --coverage-gate` (until the task that closes the
  last uncovered key, only the keys listed for later tasks may stay uncovered); then a partial
  `conformance/tools/run-linux.sh -- -k <expr>` over the ids covering the task's behavior, its newly green ids added
  with `.venv/bin/python conformance/tools/check_ratchet.py --report conformance/reports/rust-linux/nodeids.jsonl --add`.
- No task runs the whole workspace or the full suite, not even the last: CI runs both on the pull request, and a
  ratchet id red there is fixed on the branch before it merges.
- Platforms (CLAUDE.md > Platforms): macOS, Linux and Windows are equal targets and upstream is macOS only. A
  behavior upstream builds on a macOS API is ported with its macOS form and its Linux and Windows forms in the same
  task (on Linux per desktop, with a fallback); "no counterpart on <OS>" only for a behavior with no meaning there,
  with the reason. CI builds macOS and Windows code only in the crates `cross-platform.yml`'s package list names:
  the OS-specific logic goes into such a crate wherever it can, and that part ends with the manual check "green on
  `cross-platform.yml`" for the user, who opens a pull request or pushes the branch as `ci/<name>`; code left in a
  crate the list does not hold yet is recorded as "not built on macOS/Windows by CI until `<crate>` joins
  `cross-platform.yml`", never as verified.
- A ratchet id never goes red; no reruns; a function over a `clippy.toml` limit is split, not excused.

## Testing Strategy

- unit: `cargo test` on each task's crates, on the `ngterm-host` fakes; the app's tests under `xvfb-run`.
- conformance: partial `run-linux.sh -k` runs per task locally, the full suite in CI on the pull request; `tests/ui`
  skips on Linux.
- macOS and Windows: `cross-platform.yml` builds and tests the crates its package list names there; the user runs
  it through a pull request or a `ci/**` push of the branch.

## Progress Tracking

- mark completed items with `[x]` immediately when done
- add newly discovered tasks with ➕ prefix
- document issues/blockers with ⚠️ prefix
- update plan if implementation deviates from original scope

## Implementation Steps

### Task 1: <group or feature> (<upstream PR numbers>)

**Upstream:**
- implementation: `<agtermCore/Sources/agtermCore/ControlDispatcher+X.swift>`, `<agterm/Control/ControlServer+X.swift>`,
  `<agtermCore/Sources/agtermctlKit/XCommands.swift>`, `<core files>`
- tests: `<agtermCore/Tests/agtermCoreTests/XTests.swift>` (`<testFunction>`, ...), `<agtermTests/...>`
- contract: `site/commands.html` (<the command's section>), `.claude/rules/<x>.md`

**Rust side:** `<crates/...>` (create/modify)

**Platforms:** <the mechanism per OS (macOS / Linux / Windows) for each OS-specific behavior, or "no OS-specific
code">

**Conformance:**
- existing ids that cover it (from `coverage-map.py`): `<tests/protocol/test_x.py::test_y>` (<listed | not listed>)
- new tests to add: `<tests/protocol/test_x.py::test_new_behavior>` covering `<catalog key>`
- expected newly green: <ids>

- [ ] <behavior 1, with the exact upstream string it must produce>
- [ ] <behavior 2>
- [ ] write tests: Rust unit tests on the fakes asserting upstream's full strings (<cases from the Swift tests>)
- [ ] write tests: conformance <new test ids> with `covers(<keys>)`
- [ ] manual check (user): the new conformance tests are green on reference-macos (`reference-macos.yml`, started
      by hand on the pushed branch, `gh workflow run reference-macos.yml --ref <branch>`, or by a pull request that
      touches `conformance/`)
- [ ] gate (Development Approach) passes; the new suite tests green in a targeted `run-linux.sh -k` run

### Task <N-1>: UI-only changes

- [ ] <one item per UI change: the GPUI form, with its macOS, Linux and Windows mechanism where it is OS-specific,
      or "no counterpart on <OS>" with the reason>
- [ ] gate passes

### Task <N>: Claim <new tag> parity and update the docs

- [ ] bump `PARITY_LEVEL` in `crates/ngterm-proto/src/version.rs` and its mirrors (`tag-mentions.sh <old>` class
      `claim`: the version.rs test, `ngterm-core/src/dispatch.rs` and `ngterm-server/tests/server.rs` expectations,
      `conformance/agterm_conformance/impls/rust.py`, `conformance/docs/rust-build.md`), only once every task above is
      done
- [ ] README.md, CLAUDE.md, conformance/README.md, conformance/docs/rust-build.md describe what changed (new
      capabilities, commands, on-disk files); `tag-mentions.sh <old>` lists no `review` or stale `claim` line
- [ ] gate passes
- [ ] manual check (user): the pull request's CI (`rust-linux.yml`'s full suite) holds the ratchet; an id it finds
      red is fixed on the branch before merge
````

## Writing the tasks

- Order: protocol first when the release adds wire names, because `ngterm-proto/tests/catalog.rs` and
  `ngterm-ctl`'s `the_command_tree_is_the_catalogs_cli` compare the code with the regenerated catalog: until the
  new names exist those cargo tests are red for every later task. The first task adds every new `Command`
  variant, field, event and CLI subcommand skeleton (answering the same refusal the dispatcher gives any command it
  does not yet implement), so each later task starts from a green `cargo test`; when those tests are already green
  at the baseline, there is no such task. Then one task per command group or
  feature, then vendored-copy conflicts and native pin work, then the UI-only task, then the claim-and-docs
  task, always last.
- One task is one reviewable change: a group with a large diff splits by behavior (validation, read-back, GUI).
- Every task names its upstream files and Swift tests by path, so the task subagent reads the right sources without
  re-running the analysis, and lists the conformance ids it expects to turn green.
- Facts in the plan come from this run's commands (classify report, coverage-map, gate logs), not from memory.
