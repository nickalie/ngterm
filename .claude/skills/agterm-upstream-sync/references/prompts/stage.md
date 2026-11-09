# Stage worker brief

The brief of each non-analysis worker of a sync (`spawn-worker.sh` adds the reporting protocol). Replace these
placeholders: `SKILL_NAME`, `CURRENT`, `TARGET`, `BRANCH`, `WORKDIR`, `SYNC_DIR` (the sync skill's directory), `WORK`,
`SECTIONS`, `STAGE` (`bump`, `baseline`, `plan` or `plan-check`), `RUN_DIR`, `PROGRESS`, `PLAN_EXEC_DIR`,
`CODEX_DISCUSSION_PATH`, `BACKLOG_SKILL_PATH`, and `EXTRA`, which holds what this stage needs from the ones before it
(the upstream clone's path, the plan's path, the list of analyses, or nothing). `SKILL_DIR`, `SCRIPTS_DIR`,
`PROGRESS_FILE_PATH`, `CODEX_DISCUSSION` and `BACKLOG_SKILL` are names used by other files: they stay as written.

```
You carry out one stage of an unattended ngterm sync: SKILL_NAME, CURRENT -> TARGET, on branch BRANCH, in its own
worktree WORKDIR. Run every command there.

Read WORKDIR/AGENTS.md, then SYNC_DIR/SKILL.md in full. In that skill SKILL_DIR means SYNC_DIR; its reports directory is WORK.
Carry out exactly these sections of it: SECTIONS. Earlier stages are done and later ones belong to other workers;
where a section says "the orchestrator", that is not you.

EXTRA

NOBODY IS AVAILABLE. Never ask the user anything and never stop for input. Settle a judgment call in this order: the
skill's rules, AGENTS.md, the surrounding code; when those leave it 50/50, take the smaller, more reversible option.
Log each non-obvious call with bash PLAN_EXEC_DIR/scripts/append-progress.sh PROGRESS "[decision] STAGE: <what> — <why>",
and each departure from the skill the same way as [deviation].

A hard call with real consequences that those don't settle:
- If you are Claude Code: read PLAN_EXEC_DIR/references/codex-autonomous.md and follow it, with CODEX_DISCUSSION =
  CODEX_DISCUSSION_PATH and discussion dir RUN_DIR/discussions/STAGE-<topic>, at most 4 Codex turns. There and in the
  backlog-filing.md it points to, SKILL_DIR means PLAN_EXEC_DIR, SCRIPTS_DIR means PLAN_EXEC_DIR/scripts,
  PROGRESS_FILE_PATH means PROGRESS and BACKLOG_SKILL = BACKLOG_SKILL_PATH. Log the outcome as a [decision] with its
  transcript path.
- If you are Codex: ask the orchestrator, as "Asking the orchestrator" at the end of this brief says, with the options
  you see and the one you lean to, and carry on with its answer.

Long commands and waits: follow STEP 2 of PLAN_EXEC_DIR/references/prompts/task.md from "Run long commands" on, every
rule of it. Size checks as AGENTS.md > Choosing checks says.

Commit only what your sections call for, each file by name, never with `git add -A`, and never push. Before reporting,
no command you started may still run and `git status --porcelain` must be clean.

Write the stage's record to WORK/STAGE.md: what you did, each result the skill's Report needs (with its evidence), and
anything a later stage must know. Your report is its path and at most ten lines: the commits, the outcome, and each
problem left open.
```
