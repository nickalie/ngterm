# Plan template

`/plan-exec-revmux` reads `## Overview` and `## Context`, then runs each `### Task N:` in a fresh subagent that sees
only the plan and the repo (CLAUDE.md included). The `- [ ]` items are that subagent's work list. So each task names
its own files and sources. Write the plan in English and drop sections that would stay empty.

````markdown
# Upstream sync <old tag> -> <new tag>

## Overview

- Bring ngterm from upstream `umputun/agterm` <old tag> to <new tag> (<N> commits, releases <list>). The mechanical
  part is on branch `upstream-sync-<new tag>` (commit `<sha>`): <catalog, vendored copies, tag mentions; what was
  left out>.
- This plan ports: <one line per task>.
- Not ported: <the no-port list, one line>.

## Context

- Upstream: `.upstream/agterm` at <new tag>; diff `git -C .upstream/agterm diff <old>..<new>`. Changes: <one bullet
  per feature or fix, with its PR>.
- Baseline after the bump: <what was checked and the result; each expected red with the task that fixes it>.
- Already in the Rust build: <feature: evidence (file, test or ratchet id)>.
- Upstream's pins: <ZMX_REV / zmx patches / GHOSTTY_REV moves and the task for each, or unchanged>.

## Approach

- Each task follows CLAUDE.md > Porting a command group: upstream's Swift sources and tests hold the exact strings.
- Behavior that upstream builds on a macOS API gets a form for macOS, Linux and Windows (CLAUDE.md > Platforms), or
  a recorded reason why an OS has no counterpart.
- Checks per task follow CLAUDE.md > Choosing checks. A new conformance test must also pass on the macOS reference
  (`reference-macos.yml`), which the user runs.

## Implementation Steps

### Task 1: <group or feature> (<PRs>)

**Upstream:** <implementation files>; tests <Swift test files with function names>; contract <commands.html
section, `.claude/rules/*.md`>
**Rust side:** <files, create|modify>
**Conformance:** existing <ids, listed or not>; new <ids with their catalog keys>

- [ ] <behavior, with the exact upstream string where there is one>
- [ ] tests: Rust unit tests with the Swift tests' cases; conformance tests <ids>
- [ ] checks pass; newly green ids added to the ratchet

### Task <N-1>: UI-only changes

- [ ] <one item per UI change: its GPUI form, per OS where it is OS-specific>

### Task <N>: Claim <new tag> parity and update the docs

- [ ] bump `PARITY_LEVEL` and its mirrors (`tag-mentions.sh <old>`, class `claim`) once every task above is done
- [ ] README.md, conformance/README.md and conformance/docs/rust-build.md describe what changed
- [ ] manual (user): the PR's CI holds the ratchet
````

## Notes on tasks

- When the catalog gained wire names or CLI subcommands, Task 1 adds all of them as skeletons that give the
  dispatcher's "not implemented" refusal. Until then, the catalog cargo tests are red for every later task.
- One task is one reviewable change: split a large group by behavior.
- Facts come from this run's reports and greps, not from memory of earlier syncs.
