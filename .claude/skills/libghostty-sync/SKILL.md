---
name: libghostty-sync
description: "Move ngterm's Ghostty pin (LIBGHOSTTY_VT_REV: libghostty-vt and Ghostty's resources) to the latest Ghostty release or a named commit or tag, and keep ngterm's terminal behaving and rendering as Ghostty does: analyse the Ghostty diff against the Rust, Kotlin and Swift code that mirrors Ghostty's renderer, font, sprite font, Surface, termio, input and config; bump pins, patches, bindings and config keys when that is mechanical; write a docs/plans/ porting plan and run it through plan-exec, unattended from start to end, the work split between Claude and Codex workers in ngterm companion sessions. Runs only inside ngterm. Use for 'sync with ghostty', 'bump libghostty', 'ghostty pin bump', 'update libghostty-vt', 'Ghostty parity', or Ghostty change analysis."
---

# libghostty-sync

ngterm's terminal is Ghostty's: it links libghostty-vt for the VT and re-implements in Rust (and, for the mobile
renderers, in Kotlin and Swift) what Ghostty leaves to the embedder, each module a port of Ghostty's code at the pin.
This skill moves that pin forward in four parts, without stopping between them:

1. **Analysis**: what changed in Ghostty, which of it ngterm gets with the library, and which of ngterm's mirrors must
   follow.
2. **Bump** on a branch, when it is mechanical: pins, patches, bindings, config keys.
3. **Plan** for everything else.
4. **Port**: the plan goes straight into the `plan-exec` skill, so every port goes through that run's per-task review.

## Unattended, in ngterm workers

Read `references/autonomous-run.md` first and follow it: it holds the setup, who runs each stage below (Claude and
Codex workers in ngterm companion sessions), and the hand-over to plan-exec. Nobody is asked anything from the moment
the skill starts. Steps 1 and 2's grouping are the orchestrator's; each later step names its worker.

## The parity contract

ngterm's terminal behaves and renders as Ghostty does, as closely as the toolkit allows. `references/areas.md` holds
the contract: the VT arrives with the library; the embedder-side mirrors follow Ghostty's code and match its outcome
(values, bytes sent, pixels, metrics, colors), not its internal design; an official C API replaces ngterm's own
gap-filler or a patch once it covers the case. Recorded ngterm deviations win over Ghostty (README's "differs from
upstream by choice", `conformance/docs/rust-build.md`, the parity patches); upstream agterm owns the app around the
terminal, so Ghostty's own app chrome is never a target. Read areas.md before the analysis.

## Claude and Codex

The shared source is `.claude/skills/libghostty-sync`; `.agents/skills/libghostty-sync` links to it for Codex
discovery. Edit the shared source so both agents get the same workflow.

- **Claude Code:** `/libghostty-sync [TARGET]`.
- **Codex:** `$libghostty-sync [TARGET]`.

TARGET is a ghostty commit, tag (`vt-1.0.0`, `v1.4.0`) or branch, `vt` for the newest `vt-*` tag (libghostty-vt's
releases), `main-green` for the newest ghostty main commit with green libghostty-vt CI, or nothing for the latest
Ghostty release (the highest `vX.Y.Z` tag).

Read the checkout's `AGENTS.md` and any more specific directory instructions first. Set `SKILL_DIR` to the absolute
directory holding this `SKILL.md`; its `scripts/` and `references/` are meant below. The scripts act on the repository
of the current directory; all but `resolve-rev.sh` (which asks GitHub, not the clone) take `--ghostty DIR` for a clone
other than `.upstream/ghostty`.

Scratch output (reports, analyses, logs) goes in `WORK`, under the run's state dir (`references/autonomous-run.md`), so
that workers can read it and it survives a reboot. Pass its absolute path to every worker.

**Sync-specific rules** (AGENTS.md covers the rest):
- `.upstream/ghostty` is read-only apart from fetching and checking out the target.
- Commits stay local: once the run has ended the user pushes and opens the PR, whose CI builds libghostty-vt on every
  OS, Android and iOS.
- The branch builds and keeps the ratchet green, or its plan's Task 1 is what makes it so (step 3).
- Never start a dev instance outside AGENTS.md > Never touch the user's running ngterm.
- Windows work runs in the Windows VM, macOS and iOS work on the Mac over ssh, Android work on the emulator; the VM
  and the emulator start on demand and stop when that work is done (`references/machines.md`).

## 1. Revisions, clone and worktree

```bash
bash $SKILL_DIR/scripts/resolve-rev.sh [TARGET]      # CURRENT, TARGET, TARGET_REF, STATUS, AHEAD, CI
bash $SKILL_DIR/scripts/prepare-ghostty.sh CURRENT TARGET
bash $SKILL_DIR/scripts/sync-worktree.sh ghostty-sync-<TARGET short sha>   # WORKDIR, PLAN, STATE
```

Exit 3 from `resolve-rev.sh` means there is nothing to sync, unless the branch `ghostty-sync-<TARGET short sha>` already
exists (`git show-ref --verify refs/heads/<branch>`): a run started from inside a worktree whose bump is committed
lands here too, so skip `prepare-ghostty.sh` and go on with `sync-worktree.sh` and its `STATE`. With no such branch, say
there is nothing to sync and stop. A release or a TARGET the user named is
synced whatever its `CI`: when that is not `green`, record it in the plan. The branch lives in its own worktree, so it
holds only this sync whatever the checkout has uncommitted. Act on `STATE`:

- `new`: go on with Step 2.
- `planned`: an earlier run got as far as the plan. Resume at Step 6 when `WORK/plan-check.md` is missing, else at
  Step 7; plan-exec picks up from the plan's checkboxes.
- `done`: the sync already ran to its end. Report the branch and stop.
- `unplanned`: an earlier run left work on the branch and died before its plan. Its state can't be trusted, so report
  the branch and the worktree and stop; nothing is deleted.

## 2. Analysis

```bash
python3 -I $SKILL_DIR/scripts/classify-diff.py CURRENT TARGET > WORK/classify.md
bash $SKILL_DIR/scripts/patches-check.sh TARGET > WORK/patches.txt
python3 -I $SKILL_DIR/scripts/bump-pins.py TARGET > WORK/pins.txt          # reads only; fetches archives to hash
```

The classify report lists each merged PR with its areas, each area's files with the Zig tests they add and the ngterm
files citing them, the C API delta (`API_BREAK=none` or `used`, with the ngterm files each used declaration hits), the
review checklist (every ngterm file citing a changed Ghostty file) and the uncited changes in mirrored areas.
`references/areas.md` maps each area to its ngterm side, platforms and apps.

Also read `docs/backlog/` for items about the Ghostty pin or Ghostty behavior (`rg -il 'ghostty|LIBGHOSTTY' docs/backlog`):
the plan covers them, and its tasks delete them through the `workflow:backlog` skill when done.

Group the PRs into **clusters**: one per PR, merging PRs that change the same ngterm mirror or the same C API family.
Put the `no-port` area and mechanical PRs (a Zig version migration, renames, allocator or API-style refactors with no
behavior change: read the diff, not the title) on the "no port needed" list. For each cluster decide the kind
(`inherited`, `api`, `port`, `build`, `resources`, `no-port`) and, for `port` and `api`, the status (`absent`, `partial`,
`already-mirrored`) as `references/prompts/pr-analysis.md` defines them, together with the apps it reaches (desktop on
macOS, Linux and Windows; the Android and iOS renderers and key mapping; the session server) and any recorded ngterm
deviation it meets. Every checklist file and every uncited mirrored-area file ends up in a cluster or on the no-port
list with a reason.

Every cluster gets one read-only analysis worker with `references/prompts/pr-analysis.md`, up to four at a time, each
on the agent `references/autonomous-run.md` > Stages picks for it. Each writes its answer to
`WORK/analysis/<cluster-slug>.md`.

The orchestrator writes `WORK/clusters.md` before starting them: each cluster with its PRs, the "no port needed" list
with the reason for each entry, and the `docs/backlog/` items found above.

## 3. Bump

Worker: `Bump`. The branch and its worktree exist since Step 1. The bump is **mechanical** when the report says `API_BREAK=none` and `patches.txt` has no `conflict`. Then:

1. `python3 -I $SKILL_DIR/scripts/bump-pins.py --apply TARGET`, then the hand edits it prints (the pins.env comment
   blocks: what the pin follows, date, release tag or merge, CI jobs, Zig lookup date, the themes' and fonts' Zig hashes).
2. `bash $SKILL_DIR/scripts/patches-check.sh --rebase TARGET`. An `upstreamed` patch is dropped only as
   `build/patches/ghostty-vt/README.md` says (its verification passes without it), its README entry with it; if that
   verification fails, the bump is not mechanical. Update each kept patch's `base:` line in the README.
3. `scripts/build-ghostty-vt.sh`, `scripts/gen-ghostty-vt-bindings.sh`, `scripts/gen-ghostty-config-keys.sh`, and
   `scripts/fetch-fonts.sh` when a `FONTS_*` pin moved. Review the regenerated `bindings.rs` and `keys.rs` diffs: they
   feed the plan (new keys, new API).
4. Build the crates the bump touches: those that link the library (`cargo tree -i ngterm-ghostty-vt-sys --workspace
   -e normal,build,dev --prefix none`) and `ngterm-core`, whose `keys.rs` step 3 regenerated. A failure that a rename
   fixes is fixed here; any other failure makes the bump not mechanical.
5. Commit the files by name, not with `git add -A`.

When the bump is **not mechanical** (a used declaration changed, a patch conflicts or cannot be dropped, the build
fails beyond renames), restore the tree (`git restore` the files the attempt touched) and make the whole bump the
plan's Task 1, listing what broke. The branch then holds only the plan.

## 4. Baseline

Worker: `Baseline`, after a mechanical bump only. Check the bump as AGENTS.md > Choosing checks says, scoped to what a VT change can reach:
the bindings `--check`; each carried patch's verification block in its README; `cargo test` and clippy for the
crates step 3.4 built (`ngterm-core` included); and `conformance/tools/run-linux.sh -- -k <expr>` on the ids that
cover behaviors the `inherited` clusters change. A Zig version move or a build change also builds libghostty-vt and
the crates that link it in the Windows VM and on the Mac, the iOS library on the Mac too (`references/machines.md`).

A red test or ratchet id is the plan's Task 1 ("Settle the bump's fallout"): the fix follows Ghostty's new behavior
unless a recorded deviation says otherwise. `plan-exec` runs on this branch, so Task 1 settles it before anything
else.

## 5. Plan

Worker: `Plan`, from the reports in `WORK` (classify, `clusters.md`, patches, pins, `analysis/`, bump, baseline). Write
`docs/plans/<YYYYMMDD>-ghostty-sync-<TARGET short sha>.md` from `references/plan-template.md`, then commit it on the
branch. It covers every `docs/backlog/` item `clusters.md` lists, and the task that resolves one deletes it through the
`workflow:backlog` skill. Order the tasks:
1. the bump (when not mechanical) or its fallout (when there is any);
2. C API adoptions: an official API that replaces an ngterm gap-filler or a patch;
3. one task per `port` cluster, grouped by mirror (renderer, font and sprite, surface and input, termio, os and cli),
   each with its desktop, mobile and session-server items;
4. config keys at the new pin (new, renamed, removed keys and defaults);
5. resources and build leftovers (terminfo, shell integration, theme or font follow-ups);
6. "Record the new pin in the docs", always last.

Each task names its Ghostty files and Zig tests, its ngterm files, the apps and platforms it reaches and any deviation
it meets, so a worker can work from the plan alone. List `inherited` and `already-mirrored` clusters in Context with
their evidence.

## 6. Plan check

Worker: `Plan check`, on the agent that did not write the plan. Read the plan against the reports in `WORK` and the
code, independently of how the analyses reached their answers:

- confirm each `absent`, `already-mirrored` and `no-behavior-change` claim with a search of your own;
- every checklist file and uncited mirrored-area file is in a cluster or on the no-port list with a reason, every
  backlog item in `WORK/clusters.md` has its task or a recorded reason, every app
  and platform each change reaches has its task or its recorded reason, and each recorded deviation is honored;
- each task names what `references/plan-template.md` asks for, so a worker can run it from the plan alone, and the
  tasks follow Step 5's order.

Correct the plan in place, commit it by name, and put in the record what changed and why.

## 7. Port

The orchestrator hands the plan to plan-exec, as `references/autonomous-run.md` > Handing over says, and follows it to
its end.

## 8. Report

Reply in the user's language, after plan-exec's own report, with:
- the range (CURRENT -> TARGET, TARGET_REF, CI state, PR count) and what changed in Ghostty that matters to ngterm
  (one line per PR or feature), with the apps each reaches;
- the branch and its commits: what was bumped, or why the bump is Task 1;
- the patches' state and the C API delta's effect on the wrapper;
- the baseline result and which task fixes each red;
- the plan's path and its task list;
- what ngterm inherits with the library, what was already mirrored, and what needs no port;
- each recorded deviation the sync met and what of Ghostty's change it left out;
- how the port ended, and which workers ran on which agent;
- next steps: the branch and its worktree are ready; the user pushes and opens a PR so CI builds every OS and the
  mobile targets.
