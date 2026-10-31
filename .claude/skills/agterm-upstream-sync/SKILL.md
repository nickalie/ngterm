---
name: agterm-upstream-sync
description: "Bring ngterm up to a new release of upstream umputun/agterm, its parity target: find what changed upstream since the current tag, sort it by area and command group, do the mechanical bump (catalog, vendored copies, tag mentions) on branch upstream-sync-TAG, and write a docs/plans/ plan for /plan-exec-revmux with one task per feature to port. It ports no features itself. Use it in ngterm whenever the user mentions a new upstream agterm release, syncing or catching up with upstream, bumping the parity target, or what changed upstream: 'sync with upstream', 'upstream released v0.37', 'синхронизируй с апстримом', 'вышел новый релиз agterm', 'что изменилось в upstream'."
allowed-tools: Read, Write, Edit, Glob, Grep, Bash, Agent
---

# agterm-upstream-sync

Moves ngterm from the upstream release it targets to a newer one in three parts:

1. **Analysis**: what changed upstream, and how much of it the Rust build already has.
2. **Mechanical bump** on a branch: everything that needs no porting judgment.
3. **Plan** for the porting, executed later by `/plan-exec-revmux`. The skill ports no behavior itself, so every port
   goes through that run's per-task review.

`SKILL_DIR` is this file's directory; its `scripts/` and `references/` are meant below. The scripts act on the
repository of the current directory, so run them from the checkout (or worktree) being synced. Most take
`--upstream DIR` for a clone other than `.upstream/agterm`.

Keep scratch output (the classify report, analyses, logs, saved patches) in a `WORK` dir in the session scratchpad.

**Sync-specific rules** (CLAUDE.md covers the rest):
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

Analyse small syncs yourself. For more than a few clusters, spawn read-only `general-purpose` subagents with
`references/prompts/area-analysis.md`, up to four at a time. Before a subagent's `absent` or `already-ported` claim
goes into the plan, confirm it with a grep of your own.

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

Check the bump as CLAUDE.md > Choosing checks says, scoped to what it touched.

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
- next steps: `/plan-exec-revmux docs/plans/<file>`; then the user pushes, runs `reference-macos.yml` on the branch,
  and opens a PR so the cross-platform jobs and the full suite run.
