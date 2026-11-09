# Running a sync unattended

Both sync skills run this way. The session that loaded the skill is the **orchestrator**: it resolves the target,
groups the changes, writes briefs and starts workers. The stages themselves run as Claude Code and Codex workers in
ngterm companion sessions, the same workers plan-exec uses, and the plan then goes straight into plan-exec.

## Autonomy

The user starts the skill and walks away: never call AskUserQuestion and never end a reply on a question. A choice is
made by a rule of the skill, else by a codex-discussion, else filed to the backlog with a reversible interim choice,
as plan-exec's "Autonomy" and "Deciding unclear situations" say. Stopping is an outcome, not a question: report why
and what is left, after stopping what the run left running.

The user's words set how far the run goes. Asked only for an analysis, skip `sync-worktree.sh`, run the read-only
workers in the checkout, and stop after them with a report; asked only for a plan, stop after the plan check and
report.

## Setup

1. `bash ~/.claude/skills/plan-exec/scripts/preflight.sh`. Exit 1 names what is missing (ngterm itself included):
   report it and stop. Keep its values, with `PLAN_EXEC_DIR` for its `SKILL_DIR`. `CODEX_WORKERS=0` makes every worker
   Claude.
2. Read `PLAN_EXEC_DIR/SKILL.md`: its "Workers", "Choosing Claude or Codex", "Deciding unclear situations" and
   "Watchdog" sections apply to this run's workers as written, with this file's labels and stages.
3. The skill's Step 1 names the branch and makes the worktree, in its one `sync-worktree.sh` call. Exit 1 means the
   branch is checked out somewhere the run must not take over: report where and stop. Then create the state dir,
   outside `/tmp` because a run lasts hours and must survive a reboot:
   `RUN_DIR=${XDG_STATE_HOME:-$HOME/.local/state}/ngterm-sync/<branch>`, with `RUN_DIR/briefs` and `WORK=RUN_DIR/work`.
   `ACTIVE` is `RUN_DIR/active.txt`, `DISCUSSION_ROOT` is `RUN_DIR/discussions`, and `PROGRESS` is
   `RUN_DIR/progress.txt`, written only through `SCRIPTS_DIR/append-progress.sh`.
4. Act on that call's `STATE`, as Step 1 says, and enter `WORKDIR` before anything else runs there: Claude through
   `EnterWorktree` with `path`, Codex by running every command in it. Every worker gets `--cwd WORKDIR`. The user's
   checkout keeps its branch and its uncommitted work.
5. Start the watchdog before the first worker, as plan-exec's Step 0 says, on this `RUN_DIR`.

### A Codex orchestrator

plan-exec is written for a Claude orchestrator. A Codex one has no Monitor and is not woken while idle, so from here
to the end of the run, plan-exec included, this overrides plan-exec's Monitor, "wait by ending the turn" and "never
`msg read`" rules. It runs `watchdog.sh` detached on the current run dir with its output in `<run dir>/watchdog.log`,
and instead of ending its turn it waits in the foreground, one bounded call at a time, on
`ngtermctl msg read --socket "$NGTERM_SOCKET" --ack --wait --timeout 590`, reading that log after each call. On a
timeout (exit 124) it calls again.

## Stages

| Stage | Label | Agent | Sections of SKILL.md |
|---|---|---|---|
| Analysis, one worker per cluster, up to four at once | `Analyse: <few words>` | by the cluster, below | the analysis prompt |
| Bump | `Bump` | Codex | "Bump" |
| Baseline | `Baseline` | Claude when it needs another machine, else Codex | "Baseline" |
| Plan | `Plan` | Claude | "Plan" |
| Plan check | `Plan check` | Codex | "Plan check" |

- **Clusters:** use plan-exec's lists. Core logic, protocol, CLI, VT, renderer, input and other code with a reference
  to port from go to Codex. UI-only and mobile-screen clusters, docs, and clusters whose answer turns on a recorded
  ngterm feature or deviation go to Claude.
- **The plan check runs on the agent that did not write the plan**, so the claims get a second reading from the other
  model. With `CODEX_WORKERS=0` it is a fresh Claude worker.
- **Stage names:** `STAGE` in the brief and the record's name are `bump`, `baseline`, `plan` and `plan-check`.
- **Log** each choice as `[agent] <label> → <claude|codex>: <reason>` in `PROGRESS`, and say in one line what started.
- **Stages run in order, one at a time,** except analysis. Each stage's brief is the fenced block of
  `references/prompts/stage.md`, or the skill's analysis prompt, saved in `RUN_DIR/briefs/` and started with
  `spawn-worker.sh`. A skipped stage (no Baseline after a libghostty bump that was not mechanical) is logged.
- **A failed attempt** retries once with a fresh worker, as plan-exec's Step 5 says, the agent chosen as "Switch
  agents" says. A cluster whose analysis fails twice goes to the Plan worker, which does that analysis itself and logs
  a `[deviation]`. Any other stage that fails twice stops the run with a report.
- **Check each stage against git and its record,** not the worker's text: the commits its section calls for exist, the
  worktree is clean, and `WORK/<stage>.md` is there.

The orchestrator reads the classify report to group the clusters, and otherwise only the workers' short reports. It
never reads or edits the code itself. What it decides in Step 2 goes in `WORK/clusters.md`, because the later workers
see nothing else of it.

## Handing over to plan-exec

The plan check done and every worker closed:

1. Write `WORK/report.md`: everything the skill's Report asks for, from the stages' records and `PROGRESS`. A long
   plan-exec run may outlive this session's memory of them.
2. Stop the watchdog (`TaskStop` on the Monitor; a Codex orchestrator kills its detached one) and empty `ACTIVE`.
   plan-exec starts its own, on its own run dir.
3. Run plan-exec in this same session, on the plan's absolute path in `WORKDIR`, with no pause and no reply in
   between. Claude invokes the `plan-exec` skill with that path as its argument; Codex reads
   `PLAN_EXEC_DIR/SKILL.md` and follows it with that path, under "A Codex orchestrator" above. On this feature branch
   plan-exec stays in place, so the tasks land on the sync branch in `WORKDIR`.
4. Follow it to its end. A plan-exec stop ends the sync too, and its reason goes into the Report in place of the
   completion line.

After plan-exec's own completion report, in the same reply, give the skill's Report from `WORK/report.md`.

## Rules that stay

- Commits stay local: nothing in the run pushes, opens a PR, merges or tags. plan-exec lets a task worker push when a
  check can't run any other way, so both plan templates' Approach rules that out for these plans.
- Nothing here deletes a branch or a worktree: a finished run leaves both for the user's PR.
