# Plan template

The sync hands this plan to the `plan-exec` skill, which reads `## Overview` and `## Context`, then runs each
`### Task N:` in a fresh worker that sees only the plan and the repo (applicable AGENTS.md instructions included). The
`- [ ]` items are that worker's work list, so each task names its own files and sources. Write the
plan in English and drop sections that would stay empty.

````markdown
# Ghostty sync <old short sha> -> <new short sha>

## Overview

- Move ngterm's Ghostty pin (`LIBGHOSTTY_VT_REV`) from <old sha> to <new sha> (<target ref>; <N> merged PRs, <date
  range>) and bring every Rust, Kotlin and Swift mirror of Ghostty's embedder-side code to the new revision, so the
  terminal behaves and renders as Ghostty there.
- The bump: <done on branch `ghostty-sync-<short>` (commit `<sha>`): pins, patches, bindings, config keys | Task 1,
  because <the C API delta breaks the wrapper / a patch conflicts>>.
- This plan ports: <one line per task, with the apps it reaches: desktop (macOS, Linux, Windows), Android, iOS,
  session server>.
- Not ported: <the no-port list, one line>.

## Context

- Ghostty: `.upstream/ghostty` at <new sha>; diff `git -C .upstream/ghostty diff <old>..<new>`; changes:
  <one bullet per PR or feature that matters, with its number>.
- Pins moved (`.claude/skills/libghostty-sync/scripts/bump-pins.py`): <Zig version, themes, fonts, or only the revision>.
- Patches (`.claude/skills/libghostty-sync/scripts/patches-check.sh`): <each patch: clean | offset (rebased) | conflict (task) | upstreamed (task)>.
- C API delta used by ngterm: <removed/changed declarations and the wrapper modules they hit, or none>.
- Baseline after the bump: <what was checked and the result; each red with the task that fixes it>.
- Inherited with the library: <VT fixes and behaviors ngterm gets by the bump, with the ngterm test showing each where
  it matters>.
- Already mirrored: <change: evidence (file:line, test)>.
- ngterm's deviations met: <each Ghostty change that contradicts a recorded ngterm deviation: what is left out, which
  deviation decided it>.

## Approach

- The parity contract (`.claude/skills/libghostty-sync/references/areas.md`): the VT comes with the library; the
  embedder-side mirrors (renderer, font, sprite, termio, surface, input, config) follow Ghostty's code at the new pin,
  matching its outcome (values, bytes, pixels), each module citing its Ghostty source; an official C API replaces an
  ngterm gap-filler or a patch once it covers the case.
- Ghostty's Zig tests beside the changed code are the cases each port's unit tests assert; sprite changes keep
  `crates/ngterm-gpui/tests/golden/` equal to Ghostty's `src/font/sprite/testdata/`.
- Recorded ngterm deviations (README, `conformance/docs/rust-build.md`, `build/patches/ghostty-vt/README.md`) win over
  Ghostty's new form.
- Platforms: Ghostty's macOS form on macOS, its Linux form on Linux, the Linux form on Windows unless a Windows
  convention decides (recorded in the module).
- Each change reaches the Android and iOS renderers and key mapping where they draw or name that thing (UI work through
  the `frontend-design` skill inside each app's `DESIGN.md`), and the session server where its VT is involved.
- Checks per task follow AGENTS.md > Choosing checks.
- Commits stay local: no task pushes or opens a PR, even when a check could only run on CI. Such a check is logged as
  a `[deviation]` and left to the user's PR.
- Machines (`.claude/skills/libghostty-sync/references/machines.md`): Windows work (builds, clippy, tests, debugging,
  GUI runs) runs in the Windows VM, macOS and iOS work on the Mac over ssh, Android work on the emulator; the VM and
  the emulator start on demand and stop when the task's work on them is done.
- The plan-exec run's orchestrator keeps a watchdog that wakes it when any running agent's transcript, or a running
  review, has been quiet for more than 20 minutes, whatever the others are doing, and on that wake-up and on each
  "agent waiting on its background work" notice checks the agent's latest command against the real host and remote
  processes, killing any wait loop (`references/machines.md` > Waiting on a remote run).

## Implementation Steps

### Task 1: <Bump the Ghostty pin to <short> | Settle the bump's fallout>

**Ghostty:** <C API delta, patch conflicts, build changes>
**ngterm side:** `build/pins.env`, `build/patches/ghostty-vt/`, `crates/ngterm-ghostty-vt-sys/src/bindings.rs`,
`crates/ngterm-core/src/ghostty_config/keys.rs`, <wrapper modules>

- [ ] (when the bump is this task) `python3 -I .claude/skills/libghostty-sync/scripts/bump-pins.py --apply <sha>` and
  the hand edits it prints (the pins.env comments); `bash .claude/skills/libghostty-sync/scripts/patches-check.sh
  --rebase <sha>`, then each conflicting patch rebased by hand and each upstreamed one dropped once its README
  verification passes without it; `scripts/build-ghostty-vt.sh`, `scripts/gen-ghostty-vt-bindings.sh`, `scripts/gen-ghostty-config-keys.sh`
  (`scripts/fetch-fonts.sh` when fonts moved); the patches README's `base:` lines
- [ ] the wrapper (`ngterm-ghostty-vt`) on the new C API: <each removed/changed declaration and its replacement>,
  keeping the wrapper's Rust API where it can; callers updated where it cannot
- [ ] every red test from the baseline: follow Ghostty's new behavior unless a recorded deviation says otherwise
- [ ] the patches README re-verified for every patch carried (its verification block)
- [ ] checks pass, the bindings `--check` included

### Task 2: <Adopt the new C API for <behavior>> (one per adoption)

- [ ] <the official API replacing the Rust gap-filler or patch, and the code or patch removed>

### Task <N>: <Ghostty PR title> (#<PR>)

**Ghostty:** <files>; tests <Zig test names with files>
**ngterm side:** <files, create|modify>
**Apps:** <desktop / Android / iOS / session server: which it reaches, and why one is left out>
**Platforms:** <per-OS form where Ghostty's differs, or same on all three>
**Deviation:** <the recorded ngterm deviation it meets and what is left out, or none>

- [ ] <behavior as Ghostty's outcome, with the Ghostty source cited in the module docs>
- [ ] tests: <Rust unit tests with Ghostty's cases; Kotlin/Swift tests for the apps; golden images>
- [ ] checks pass

### Task <N+1>: Config keys at the new pin

- [ ] <each new, renamed or removed key and default: implemented where ngterm has the behavior, else added to
  `docs/backlog/implement-deferred-ghostty-config-keys.md`>

### Task <last>: Record the new pin in the docs

- [ ] README.md, `conformance/docs/rust-build.md` and the patches README describe the new pin and what changed
  (Zig version, patches carried, new behaviors)
- [ ] every ngterm file in the report's review checklist was reviewed by a task above; those citing a revision say
  the new one
````

## Notes on tasks

- One task is one reviewable change: split a large PR by behavior, merge small PRs that touch the same mirror.
- Task 1 comes first when the bump could not be done mechanically: every later task builds on the new library.
- `inherited` changes get no task of their own unless ngterm must act on them; they go to Context with the test that
  shows them, or into Task 1 when an ngterm test changes.
- A port whose Ghostty source has Zig tests asserts those cases in Rust; a renderer change with no Ghostty test asserts
  the computed values (colors, rects, layers), not GPUI's output.
- Each task's checks name the machine for each platform it reaches (`references/machines.md`): the iOS build and
  tests and the macOS renderer on the Mac, the Android renderer's instrumented tests and screen checks on the
  emulator, Windows in the VM. Mobile checks follow README > Android and > iOS.
- Facts come from this run's reports and greps, not from memory of earlier syncs.
