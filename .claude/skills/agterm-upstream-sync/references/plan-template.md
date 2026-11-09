# Plan template

The sync hands this plan to the `plan-exec` skill, which reads `## Overview` and `## Context`, then runs each
`### Task N:` in a fresh worker that sees only the plan and the repo (applicable AGENTS.md instructions included). The
`- [ ]` items are that worker's work list. So each task names its own files and sources. Write the plan in English and drop sections that would stay empty.

````markdown
# Upstream sync <old tag> -> <new tag>

## Overview

- Bring ngterm from upstream `umputun/agterm` <old tag> to <new tag> (<N> commits, releases <list>). The mechanical
  part is on branch `upstream-sync-<new tag>` (commit `<sha>`): <catalog, vendored copies, tag mentions; what was
  left out>.
- This plan ports: <one line per task, with the apps it reaches: desktop (macOS, Linux, Windows), Android, iOS>.
- Not ported: <the no-port list, one line>.

## Context

- Upstream: `.upstream/agterm` at <new tag>; diff `git -C .upstream/agterm diff <old>..<new>`. Changes: <one bullet
  per feature or fix, with its PR>.
- Baseline after the bump: <what was checked and the result; each expected red with the task that fixes it>.
- Already in the Rust build: <feature: evidence (file, test or ratchet id)>.
- Upstream's pins: <ZMX_REV / zmx patches / GHOSTTY_REV moves and the task for each, or unchanged>.
- ngterm's own features: <each upstream change that overlaps an ngterm extension or deliberate deviation: what is
  taken, what is left and why; ngterm's form wins>.

## Approach

- Each task follows AGENTS.md > Porting a command group: upstream's Swift sources and tests hold the exact strings.
- Where upstream overlaps an ngterm feature or deliberate deviation (`docs/development.md`, `conformance/docs/rust-build.md`),
  ngterm's form wins: a task adapts upstream's change onto it or leaves the contradicting part out, and never reworks
  the ngterm feature toward upstream.
- Behavior that upstream builds on a macOS API gets a form for macOS, Linux and Windows (AGENTS.md > Platforms), or
  a recorded reason why an OS has no counterpart.
- Each behavior also reaches the Android and iOS apps where a remote viewer would show or drive it, through
  `ngterm-mobile` and each app's screens (UI work through the `frontend-design` skill, inside that app's `DESIGN.md`):
  the feature's own task names the apps it reaches, and the mobile code goes into a mobile task of that feature
  (below), or the feature's task records why an app has no counterpart.
- Checks per task follow AGENTS.md > Choosing checks. A new conformance test must also pass on the macOS reference
  (`reference-macos.yml`), which the user runs.
- Commits stay local: no task pushes or opens a PR, even when a check could only run on CI. Such a check is logged as
  a `[deviation]` and left to the user's PR.
- Machines (`.claude/skills/agterm-upstream-sync/references/machines.md`): Windows work (builds, clippy, tests,
  debugging, GUI runs) runs in the Windows VM, macOS and iOS work on the Mac over ssh, Android work on the emulator;
  the VM and the emulator start on demand and stop when the task's work on them is done.
- The plan-exec run's orchestrator keeps a watchdog that wakes it when any running agent's transcript, or a running
  review, has been quiet for more than 20 minutes, whatever the others are doing, and on that wake-up and on each
  "agent waiting on its background work" notice checks the agent's latest command against the real host and remote
  processes, killing any wait loop (`references/machines.md` > Waiting on a remote run).

## Implementation Steps

### Task 1: <group or feature> (<PRs>)

**Upstream:** <implementation files>; tests <Swift test files with function names>; contract <commands.html
section, `.claude/rules/*.md`>
**Rust side:** <files, create|modify>
**Apps:** <desktop / Android / iOS: which it reaches, and why one is left out>
**ngterm's own:** <the ngterm feature or deviation it overlaps and what of upstream's change is left out, or none>
**Conformance:** existing <ids, listed or not>; new <ids with their catalog keys>

- [ ] <behavior, with the exact upstream string where there is one>
- [ ] tests: Rust unit tests with the Swift tests' cases; conformance tests <ids>
- [ ] checks pass; newly green ids added to the ratchet

### Task <N-2>: Desktop UI-only changes

- [ ] <one item per UI change: its GPUI form, per OS where it is OS-specific>

### Task <N-1>: <feature> in the Android and iOS apps (one task per feature that needs mobile code)

**Mobile core:** <`ngterm-mobile` files, create|modify; the `MobileCore`/`MobileEvent` API change>
**Android:** <`mobile/android` files>; **iOS:** <`mobile/ios` files>

- [ ] `ngterm-mobile`: <what the core reads, sends or reports>, with tests on its fakes
- [ ] Android: <the screen and its form, inside `mobile/android/DESIGN.md`>, with its JVM tests
- [ ] iOS: <the same, inside `mobile/ios/DESIGN.md`>, with its unit tests

### Task <N>: Claim <new tag> parity and update the docs

- [ ] bump `PARITY_LEVEL` and its mirrors (`tag-mentions.sh <old>`, class `claim`) once every task above is done
- [ ] README.md, docs/development.md (its Android and iOS sections included), conformance/README.md and conformance/docs/rust-build.md
  describe what changed
````

## Notes on tasks

- When the catalog gained wire names or CLI subcommands, Task 1 adds all of them as skeletons that give the
  dispatcher's "not implemented" refusal. Until then, the catalog cargo tests are red for every later task.
- One task is one reviewable change: split a large group by behavior.
- A behavior the mobile apps get with no mobile code (it arrives through `ngterm-core`'s remote client) gets no mobile
  task, but its feature task carries an item for a `ngterm-mobile` test that shows it arrives, so the apps' parity is
  checked, not assumed.
- Each task's checks name the machine for each platform it reaches (`references/machines.md`): the iOS build and
  tests on the Mac, the Android app's instrumented tests and screen checks on the emulator, Windows in the VM.
  Mobile checks follow docs/development.md > Android and > iOS.
- Facts come from this run's reports and greps, not from memory of earlier syncs.
