---
name: agterm-upstream-sync
description: "Sync ngterm's parity target with a new umputun/agterm release from start to end, unattended: analyse upstream changes, make the mechanical bump, write a docs/plans/ porting plan that covers all five apps (desktop on macOS, Linux and Windows; Android; iOS), and run it through plan-exec, the work split between Claude and Codex workers in ngterm companion sessions. Runs only inside ngterm. Use for upstream release syncs, parity target bumps, or upstream change analysis, including 'sync with upstream'."
---

# agterm-upstream-sync

Moves ngterm from the upstream release it targets to a newer one, in four parts, without stopping between them:

1. **Analysis**: what changed upstream, and how much of it the Rust build already has.
2. **Mechanical bump** on a branch: everything that needs no porting judgment.
3. **Plan** for the porting.
4. **Port**: the plan goes straight into the `plan-exec` skill, so every port goes through that run's per-task review.

## Unattended, in ngterm workers

Read `references/autonomous-run.md` first and follow it: it holds the setup, who runs each stage below (Claude and
Codex workers in ngterm companion sessions), and the hand-over to plan-exec. Nobody is asked anything from the moment
the skill starts. Steps 1 and 2's grouping are the orchestrator's; each later step names its worker.

## Five apps

Upstream is one macOS app; ngterm ships five: the desktop app on macOS, Linux and Windows (`ngterm-gpui`), and the
Android and iOS apps (`mobile/android`, `mobile/ios`), remote-only viewers over `ngterm-mobile` that reach a machine as
the desktop's Attach Remote does. Weigh every upstream change against all five: the analysis says which apps a
behavior reaches, the plan gives each of them its task items, and a change that leaves an app out records why
(`references/areas.md` > Platforms and apps).

## ngterm's own features win

ngterm is more than a port: it has features upstream lacks and behaviors it chose to do differently. README lists them
(the "extension beyond upstream" and "differs from upstream by choice" bullets), and
`conformance/docs/rust-build.md` records the Rust-only extensions and deliberate deviations; the user's standing
overrides of upstream live there and in the project's own decisions (the Settings popup, the `live` names, ngterm's
identity). When an upstream change overlaps one of them, **ngterm's approach wins**:

- port only the part of the change that fits ngterm's design, adapted onto it, and never rework or remove an ngterm
  feature to match upstream;
- an upstream change that only makes sense against what ngterm replaced is not ported: it goes to the "no port
  needed" list with the ngterm feature that supersedes it;
- a behavior both have, where upstream's new form contradicts ngterm's, keeps ngterm's form; the plan task records the
  contradiction, and a conformance test that asserts upstream's form is answered by recording the deviation as
  `rust-build.md` records the others (a note in its Rust target profile, a capability where the suite must tell the
  builds apart), not by a code change toward upstream.

Weigh an upstream change as a possible improvement to ngterm's feature only where it adds something ngterm's form
lacks, and port that part in ngterm's terms.

## Claude and Codex

The shared source is `.claude/skills/agterm-upstream-sync`; `.agents/skills/agterm-upstream-sync` links to it for
Codex discovery. Edit the shared source so both agents get the same workflow.

- **Claude Code:** invoke `/agterm-upstream-sync [TARGET]`.
- **Codex:** invoke `$agterm-upstream-sync [TARGET]`.

Read the checkout's `AGENTS.md` (the shared architecture and check guidance) and any more specific directory
instructions before starting. Use the current agent's file, search and shell tools; Claude tool names are not
required.

Set `SKILL_DIR` to the absolute directory containing the loaded `SKILL.md`; either discovery path works.
Its `scripts/` and `references/` are meant below. The scripts act on the
repository of the current directory, so run them from the checkout (or worktree) being synced. Most take
`--upstream DIR` for a clone other than `.upstream/agterm`.

Scratch output (the classify report, analyses, logs, saved patches) goes in `WORK`, under the run's state dir
(`references/autonomous-run.md`), so that workers can read it and it survives a reboot. Pass its absolute path to
every worker.

**Sync-specific rules** (AGENTS.md covers the rest):
- `.upstream/agterm` is read-only apart from fetching and checking out the new tag.
- Commits stay local: the user pushes, opens the PR and runs the macOS reference workflow once the run has ended.
- The branch keeps building and keeps the ratchet green. Anything that would break either becomes a plan task
  instead of part of the bump.
- Windows work runs in the Windows VM, macOS and iOS work on the Mac over ssh, Android work on the emulator; the VM
  and the emulator start on demand and stop when that work is done (`references/machines.md`).

## 1. Tags, upstream clone and worktree

```bash
bash $SKILL_DIR/scripts/resolve-tags.sh [TARGET]                       # CURRENT, TARGET, STATUS, RELEASES
bash $SKILL_DIR/scripts/prepare-upstream.sh [--upstream DIR] CURRENT TARGET
bash $SKILL_DIR/scripts/sync-worktree.sh upstream-sync-TARGET          # WORKDIR, PLAN, STATE
```

Exit 3 from `resolve-tags.sh` means there is nothing to sync, unless the branch `upstream-sync-TARGET` already exists
(`git show-ref --verify refs/heads/<branch>`): a run started from inside a worktree whose bump is committed lands here
too, so skip `prepare-upstream.sh` and go on with `sync-worktree.sh` and its `STATE`. With no such branch, say there is
nothing to sync and stop. Several skipped releases are synced in one go. The branch lives in its own worktree, so it holds only this sync whatever the checkout has uncommitted. Act
on `STATE`:

- `new`: go on with Step 2.
- `planned`: an earlier run got as far as the plan. Resume at Step 6 when `WORK/plan-check.md` is missing, else at
  Step 7; plan-exec picks up from the plan's checkboxes.
- `done`: the sync already ran to its end. Report the branch and stop.
- `unplanned`: an earlier run left work on the branch and died before its plan. Its state can't be trusted, so report
  the branch and the worktree and stop; nothing is deleted.

## 2. Analysis

```bash
python3 $SKILL_DIR/scripts/classify-diff.py --upstream UPSTREAM CURRENT TARGET > WORK/classify.md
```

The report lists the changelog, each commit with its areas and command groups, and per area the files, new Swift
test functions and new user-visible strings (`(moved)` marks a string that only moved). `references/areas.md` maps
each area to its Rust side.

Group the commits into **clusters**, one per feature or fix (PR numbers are good seams). Merge clusters that touch the
same command group, and fold refactor, docs and test-only commits into the cluster they belong to. Pure moves and
`no-port` paths go to the "no port needed" list.

For each cluster, decide per behavior:
- **absent** in Rust: a port task.
- **partial**: the code is there but Rust tests don't assert what the Swift tests assert, or no conformance id covers
  a behavior the suite can test. This becomes a task that ports the tests and fixes the code if they expose a bug.
- **already-ported**: code and tests are both there. No task; record the evidence.

Then decide per app (Five apps above): which of the desktop OSes, Android and iOS the behavior reaches, and for each
of them absent, partial or already-ported in the same sense, and whether it overlaps an ngterm feature or deliberate
deviation (ngterm's own features win above): such a cluster ports only what fits ngterm's form, or nothing, and says
which feature decided it. A cluster is already-ported only when every app it
reaches has it: one the desktop has and a mobile app lacks still gets that app's task. The mobile apps share
`ngterm-core`'s remote client with Attach Remote, so a change there can reach them with no mobile code; an app screen
that should show it still can't.

Every cluster gets one read-only analysis worker with `references/prompts/area-analysis.md`, up to four at a time,
each on the agent `references/autonomous-run.md` > Stages picks for it. Each writes its answer to
`WORK/analysis/<cluster-slug>.md`.

The orchestrator writes `WORK/clusters.md` before starting them: each cluster with its commits, and the "no port
needed" list with the reason for each entry.

## 3. Mechanical bump

Worker: `Bump`. The branch and its worktree exist since Step 1.

1. **Catalog**: `.venv/bin/python conformance/tools/gen_catalog.py --tag TARGET --repo UPSTREAM`. If it fails to
   parse, leave the catalog to a plan task.
2. **Upstream's pins**: `bash $SKILL_DIR/scripts/pins-delta.sh CURRENT TARGET`. None of upstream's pins is a pin
   here, so this only reports. A moved `ZMX_REV` or a changed zmx patch becomes a plan task that ports the behavior into
   `ngterm-session`. A moved `GHOSTTY_REV` becomes a task to review the Ghostty-level changes against this repo's own
   `LIBGHOSTTY_VT_REV`. Bumps of herdr or of this repo's Ghostty pin are separate changes, not part of the sync (the
   Ghostty pin's is the `libghostty-sync` skill).
3. **Vendored copies**: `bash $SKILL_DIR/scripts/sync-resources.sh --patches WORK/patches CURRENT TARGET`.
   A `conflict` or `unmapped` line becomes a plan task. If nothing conflicted, bump the release named in the header of
   `crates/ngterm-app/src/integrations/mod.rs`.
4. **Tag mentions**: `bash $SKILL_DIR/scripts/tag-mentions.sh CURRENT TARGET --apply`. It rewrites the `bump`
   class and lists the rest. Edit the `reference` lines by hand. Leave `claim` (`PARITY_LEVEL` and its mirrors) for the plan's last
   task, because it unlocks `since(V)` tests and states a parity not yet reached. Decide each `review` line yourself.
5. **Commit** the files by name, not with `git add -A`. Then write the catalog delta:
   `.venv/bin/python $SKILL_DIR/scripts/coverage-map.py --since HEAD~1 > WORK/catalog-delta.md`.

## 4. Baseline

Worker: `Baseline`. Check the bump as AGENTS.md > Choosing checks says, scoped to what it touched, each platform it reaches on that
platform's machine (`references/machines.md`).

The usual expected reds are:
- `ngterm-proto/tests/catalog.rs` and ngterm-ctl's `the_command_tree_is_the_catalogs_cli`, while the catalog lists
  wire names or subcommands the code lacks;
- the coverage gate, while new catalog items have no test.

Each of these goes to the plan's first task. If a ratchet id goes red, find the part of the bump that caused it,
revert that part, and turn it into a plan task.

## 5. Plan

Worker: `Plan`, from the reports in `WORK` (classify, `clusters.md`, `analysis/`, bump, baseline, catalog delta). Write
`docs/plans/<YYYYMMDD>-upstream-sync-TARGET.md` from `references/plan-template.md`, then commit it on the branch.
Order the tasks:
1. new wire names and CLI skeletons (when the catalog gained any);
2. one task per command group or feature;
3. vendored-copy conflicts and pin work;
4. desktop UI-only changes;
5. the mobile apps: one task per feature that needs mobile code, its `ngterm-mobile` change first, then the Android
   and iOS apps in the same task;
6. "Claim TARGET parity and update the docs", always last.

Each task names its upstream files and Swift tests, its Rust, Kotlin and Swift files, its conformance ids and the apps
it reaches, so a worker can work from the plan alone. List `already-ported` clusters in Context with their evidence.

## 6. Plan check

Worker: `Plan check`, on the agent that did not write the plan. Read the plan against the reports in `WORK` and the
code, independently of how the analyses reached their answers:

- confirm each `absent`, `already-ported` and `no-behavior-change` claim with a search of your own;
- every cluster and every no-port entry of `WORK/clusters.md` is accounted for, every app each behavior reaches has its task or its recorded
  reason, and each overlap with an ngterm feature follows "ngterm's own features win";
- each task names what `references/plan-template.md` asks for, so a worker can run it from the plan alone, and the
  tasks follow Step 5's order.

Correct the plan in place, commit it by name, and put in the record what changed and why.

## 7. Port

The orchestrator hands the plan to plan-exec, as `references/autonomous-run.md` > Handing over says, and follows it to
its end.

## 8. Report

Reply in the user's language, after plan-exec's own report, with:
- the range and what changed upstream (one line per feature or fix), with the apps each one reaches;
- the branch and its commits: what was bumped, and what was left out and why;
- the baseline result and which task fixes each expected red;
- the plan's path and its task list;
- what is already in the Rust build and the mobile apps, and what needs no port;
- each overlap with an ngterm feature or deliberate deviation: what of upstream's change was taken, what was left,
  and the feature that decided it;
- how the port ended, and which workers ran on which agent;
- next steps: the branch and its worktree are ready; the user pushes, runs `reference-macos.yml` on the branch, and
  opens a PR so the cross-platform jobs and the full suite run.
