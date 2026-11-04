---
name: agterm-upstream-sync
description: "Sync ngterm's parity target with a new umputun/agterm release: analyse upstream changes, make the mechanical bump, and write a docs/plans/ porting plan for plan-exec-revmux. Use for upstream release syncs, parity target bumps, or upstream change analysis, including 'синхронизируй с апстримом'. Ports no features itself."
---

# agterm-upstream-sync

Moves ngterm from the upstream release it targets to a newer one in three parts:

1. **Analysis**: what changed upstream, and how much of it the Rust build already has.
2. **Mechanical bump** on a branch: everything that needs no porting judgment.
3. **Plan** for the porting, executed later by the `plan-exec-revmux` skill. The skill ports no behavior itself, so every port
   goes through that run's per-task review.

## Claude and Codex

The shared source is `.claude/skills/agterm-upstream-sync`; `.agents/skills/agterm-upstream-sync` links to it for
Codex discovery. Edit the shared source so both agents get the same workflow.

- **Claude Code:** invoke `/agterm-upstream-sync [TARGET]`; later execute the plan with
  `/plan-exec-revmux docs/plans/<file>`.
- **Codex:** invoke `$agterm-upstream-sync [TARGET]`; later execute the plan with
  `$plan-exec-revmux docs/plans/<file>`.

Read the checkout's `AGENTS.md` (the shared architecture and check guidance) and any more specific directory
instructions before starting. Use the current agent's file, search, shell and subagent tools; Claude tool names are
not required.

Set `SKILL_DIR` to the absolute directory containing the loaded `SKILL.md`; either discovery path works.
Its `scripts/` and `references/` are meant below. The scripts act on the
repository of the current directory, so run them from the checkout (or worktree) being synced. Most take
`--upstream DIR` for a clone other than `.upstream/agterm`.

Keep scratch output (the classify report, analyses, logs, saved patches) in a `WORK` dir in the session scratchpad
when available, otherwise create one with `WORK=$(mktemp -d "${TMPDIR:-/tmp}/ngterm-upstream-sync.XXXXXX")`.
Use the same absolute `WORK` path across tool calls and pass it to subagents that need the reports.

**Sync-specific rules** (AGENTS.md covers the rest):
- `.upstream/agterm` is read-only apart from fetching and checking out the new tag.
- Commits stay local: the user pushes, opens the PR and runs the macOS reference workflow.
- The branch keeps building and keeps the ratchet green. Anything that would break either becomes a plan task
  instead of part of the bump.

## 1. Tags and upstream clone

```bash
bash $SKILL_DIR/scripts/resolve-tags.sh [TARGET]                       # CURRENT, TARGET, STATUS, RELEASES
bash $SKILL_DIR/scripts/prepare-upstream.sh [--upstream DIR] CURRENT TARGET
```

Exit 3 from `resolve-tags.sh` means there is nothing to sync: say so and stop. Several skipped releases are synced in
one go. Start from a tree with no modified tracked files, so the branch holds only this sync.

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

Analyse small syncs yourself. For more than a few clusters, delegate read-only analysis using
`references/prompts/area-analysis.md`: Claude's `Agent` with `subagent_type: "general-purpose"`, or Codex's available
subagent tool (for example, `spawn_agent`). Use up to four at a time, within the environment's available slots.
If subagents are unavailable or disallowed, analyse the clusters yourself with the same prompt. Before a
subagent's `absent` or `already-ported` claim goes into the plan, confirm it with a search of your own.

## 3. Mechanical bump

`git switch -c upstream-sync-TARGET`. If the branch already exists, it is an earlier run: report it and stop.

1. **Catalog**: `.venv/bin/python conformance/tools/gen_catalog.py --tag TARGET --repo UPSTREAM`. If it fails to
   parse, leave the catalog to a plan task.
2. **Upstream's pins**: `bash $SKILL_DIR/scripts/pins-delta.sh CURRENT TARGET`. None of upstream's pins is a pin
   here, so this only reports. A moved `ZMX_REV` or a changed zmx patch becomes a plan task that ports the behavior into
   `ngterm-session`. A moved `GHOSTTY_REV` becomes a task to review the Ghostty-level changes against this repo's own
   `LIBGHOSTTY_VT_REV`. Bumps of herdr or of this repo's Ghostty pin are separate changes, not part of the sync.
3. **Vendored copies**: `bash $SKILL_DIR/scripts/sync-resources.sh --patches WORK/patches CURRENT TARGET`.
   A `conflict` or `unmapped` line becomes a plan task. If nothing conflicted, bump the release named in the header of
   `crates/ngterm-app/src/integrations/mod.rs`.
4. **Tag mentions**: `bash $SKILL_DIR/scripts/tag-mentions.sh CURRENT TARGET --apply`. It rewrites the `bump`
   class and lists the rest. Edit the `reference` lines by hand. Leave `claim` (`PARITY_LEVEL` and its mirrors) for the plan's last
   task, because it unlocks `since(V)` tests and states a parity not yet reached. Decide each `review` line yourself.
5. **Commit** the files by name, not with `git add -A`. Then write the catalog delta:
   `.venv/bin/python $SKILL_DIR/scripts/coverage-map.py --since HEAD~1 > WORK/catalog-delta.md`.

## 4. Baseline

Check the bump as AGENTS.md > Choosing checks says, scoped to what it touched.

The usual expected reds are:
- `ngterm-proto/tests/catalog.rs` and ngterm-ctl's `the_command_tree_is_the_catalogs_cli`, while the catalog lists
  wire names or subcommands the code lacks;
- the coverage gate, while new catalog items have no test.

Each of these goes to the plan's first task. If a ratchet id goes red, find the part of the bump that caused it,
revert that part, and turn it into a plan task.

## 5. Plan

Write `docs/plans/<YYYYMMDD>-upstream-sync-TARGET.md` from `references/plan-template.md`, then commit it on the branch.
Order the tasks:
1. new wire names and CLI skeletons (when the catalog gained any);
2. one task per command group or feature;
3. vendored-copy conflicts and pin work;
4. UI-only changes;
5. "Claim TARGET parity and update the docs", always last.

Each task names its upstream files and Swift tests, its Rust files and its conformance ids, so a subagent can work
from the plan alone. List `already-ported` clusters in Context with their evidence.

## 6. Report

Reply in the user's language with:
- the range and what changed upstream (one line per feature or fix);
- the branch and its commits: what was bumped, and what was left out and why;
- the baseline result and which task fixes each expected red;
- the plan's path and its task list;
- what is already in the Rust build, and what needs no port;
- next steps: invoke `plan-exec-revmux` with the plan path using the current agent's syntax above (if that skill is
  unavailable, report the missing dependency); then the user pushes, runs `reference-macos.yml` on the branch,
  and opens a PR so the cross-platform jobs and the full suite run.
