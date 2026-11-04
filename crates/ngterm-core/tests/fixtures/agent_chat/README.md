# agent_chat fixtures

Real Claude Code and Codex sessions, recorded for the agent chat view (`docs/plans/20261103-android-agent-chat.md`):
the transcripts the `agent_chat` parsers read, the hook payloads `ngtermctl agent hook` parses, and (in
`crates/ngterm-mobile/tests/fixtures/agent_screens/`) the PTY bytes the phone's screen matchers read.

## Versions

| agent | version | model |
|---|---|---|
| Claude Code | 2.1.317 | `claude-sonnet-5-5` (`--model sonnet`), `claude -p --model haiku` for the worker |
| Codex CLI | 0.166.0 | `gpt-5.6-luna`, reasoning effort low, `model_reasoning_summary = "detailed"` |

Recorded on Linux on 2026-11-03.

## Layout

```text
claude/projects/-home-user-project/      a Claude config dir's projects/ tree
  f559ab9f-….jsonl                       the main conversation (see "Claude session" below)
  f559ab9f-…/subagents/agent-….jsonl     its Agent subagent, with agent-….meta.json
  2897645c-….jsonl                       the conversation /clear started
  17d240d5-….jsonl                       the `claude -p` worker run from inside the session
claude/hooks/NN-<Event>.json             every hook payload of those sessions, in firing order
claude/hooks/NN-<Event>.env              the CLAUDE_* environment the hook saw
codex/sessions/2026/11/03/rollout-….jsonl  a CODEX_HOME's sessions/ tree (see "Codex session")
codex/hooks/main/NN-<Event>.json(.env)   the main session's hook payloads (and its resume)
codex/hooks/auto-review/…                a separate session run with Auto Review
record/                                  the tools that made all this
```

Beside each transcript, `<name>.golden.ndjson` holds the chat events `ngterm-core::agent_chat` makes of it; the
parser's tests compare against it, and `UPDATE_GOLDEN=1 cargo test -p ngterm-core agent_chat` rewrites it after a
deliberate parser change (review the diff).

Paths are rewritten: the project is `/home/user/project`, Claude's config dir `/home/user/.claude`, `CODEX_HOME`
`/home/user/.codex` and the user `user`. Bulk the chat never shows is replaced by `<redacted>` or a stub that keeps
the line's `type` (system prompts, tool schemas, API request logs, attachments, encrypted reasoning, thinking
signatures, account and organization ids), and a string over 8 KiB keeps its first KiB padded back to its length
with a marker. Everything else is as the agents wrote it.

### Claude session (`f559ab9f`)

In order: a plain answer; one turn with `Bash ls -la`, `Read`, `Bash grep` (Claude grepped through Bash), `WebFetch`,
`Edit` (a diff), a `ToolSearch` and a background `Agent` subagent that ran `wc -l`; an `AskUserQuestion` with a
single-select and a multi-select question; a `Bash` approval allowed, one denied with "No", one dismissed with Esc; a
reply interrupted mid-stream with Esc; `/compact keep it short`; a `claude -p` worker (approved, run through `Bash`);
`/clear` (continues in `2897645c`); then the CLI exited and `claude --resume f559ab9f…` appended one more turn.

### Codex session (`01a1a300-e2e3`)

In order: a plain answer with reasoning; one turn with `ls -la`, `cat`, `rg TODO`, a web search, `apply_patch` (a
diff) and a spawned subagent (rollout `01a1a301-8737`); a network command approved with `y`; one denied ("No", esc);
a reply interrupted with Esc; `$greet-check please` (a skill); `/compact`; `/rename greeter recording`; `/clear`
(continues in `01a1a303-e11f`); then the CLI exited and `codex resume 01a1a300-…` appended one more turn. The
Auto Review session (`01a1a305-0669`, with `-c approvals_reviewer="auto_review"`) ran one network command that the
guardian approved; the guardian's own rollout is `01a1a305-068c`.

## Settled facts

Where these differ from the plan, the plan was updated.

**Hook events and payloads**

- Claude: payloads carry `session_id`, `transcript_path`, `cwd`, `prompt_id`, `permission_mode` and, on tool events,
  `tool_name`, `tool_input`; `PreToolUse` and `PostToolUse` carry `tool_use_id`, **`PermissionRequest` does not**
  (it fires right after the `PreToolUse` with the same `tool_input`). `Notification` fires with
  `notification_type: permission_prompt` for every dialog.
- Claude `SessionStart.source`: `startup`, `resume`, `compact` (after `/compact`) and `clear`. `/clear` fires
  `SessionEnd` (`reason: clear`) for the old conversation, then `SessionStart` (`source: clear`) with a new
  `session_id` and `transcript_path`. Exiting fires `SessionEnd` (`prompt_input_exit`).
- Claude workers: the interactive TUI's hooks see `CLAUDE_CODE_ENTRYPOINT=cli`; a `claude -p` worker's see
  `sdk-cli`. The worker fires `SessionStart`, `UserPromptSubmit`, `Stop` and `SessionEnd` with its own session id and
  a transcript in the same `projects/` directory, so it must be filtered by the entrypoint. Subagent tool events
  carry `agent_id` and `agent_type` (Claude: `PreToolUse`/`PostToolUse` of the subagent and `SubagentStop`).
  `/compact` runs as a subagent too: `SubagentStop` with `agent_type: ""` and an `agent_transcript_path` that is
  never written.
- Codex 0.166 has the hooks `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `PermissionRequest`,
  `Stop`, **`SessionEnd`**, `SubagentStart`, `SubagentStop`, `PreCompact`, `PostCompact` and `Interrupt`.
  - `SessionStart` fires at the first prompt, not when the TUI opens (`source`: `startup`, `compact`, `clear`).
  - `/clear` fires only `SessionStart` (`source: clear`) with the new thread's id and rollout; the old thread's
    `SessionEnd` comes when the CLI exits, together with one per thread it loaded (`reason: other`). An earlier,
    already exited run's thread that never ran a turn got its `SessionEnd` minutes later, during the next run.
    `SessionEnd` therefore clears a record only if its `session_id` is the record's.
  - A subagent's events fire in the parent with the parent's `session_id` but `agent_id`, `agent_type` and **the
    subagent's** `transcript_path` (`UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Stop`): a hook that re-asserts
    the record must skip payloads with `agent_id`.
  - `PreToolUse.tool_use_id` (`exec-…`) is the id of the rollout's `CommandExecution` item.
  - Esc on an approval or mid-turn fires `Interrupt` (no `Stop`).
- Codex's memory-consolidation thread (null `transcript_path`) did not run during this recording; the status
  adapter's existing guard for it stands.

**Approvals**

- Codex's `PermissionRequest` carries `tool_name: Bash` and `tool_input: { command, description }` (the description
  is the model's justification) and no `tool_use_id`. With Auto Review it still fires (a candidate); the guardian
  approves without a dialog.
- Codex's rollout does not record an approval or its answer: an approved command is an ordinary `CommandExecution`;
  a denied one leaves its `custom_tool_call_output` as `aborted by user after Ns` followed by
  `turn_aborted { reason: interrupted }`. Auto Review's verdict is only in the guardian's rollout
  (`thread_source: guardian_review`, an `AgentMessage` holding `{"outcome":"allow",…}`).
- Claude: "No" and Esc on a dialog write the same thing, a `tool_result` with `is_error: true` ("The user doesn't
  want to proceed with this tool use…") then a user text `[Request interrupted by user for tool use]`. Esc mid-reply
  writes `[Request interrupted by user]`.
- **`AskUserQuestion` fires `PermissionRequest`** (`tool_name: AskUserQuestion`, the questions in `tool_input`) and a
  `permission_prompt` notification: the approval candidate must not become a second card next to the question.

**Transcripts**

- Claude subagents are **separate files**: `<session-id>/subagents/agent-<agentId>.jsonl` (every line
  `isSidechain: true`) beside `agent-<agentId>.meta.json`, whose `toolUseId` is the parent's `Agent` tool use. The
  subagent tool is named **`Agent`**. A background agent's result reaches the parent as a `queue-operation` line and a
  user message holding a `<task-notification>`.
- Claude thinking blocks are written with an **empty `thinking` text** (only a signature): the chat can show that
  the agent thought, not what.
- Claude compaction: a `system` line with `subtype: compact_boundary`, then a `user` line with
  `isCompactSummary: true` holding the summary.
- Claude writes many bookkeeping line types the chat ignores: `mode`, `permission-mode`, `session-owner`,
  `atis-latch`, `file-history-snapshot`, `file-history-delta`, `attachment`, `api-request*`, `last-prompt`,
  `ai-title`, `tools-used`, `queue-operation`, `system` (`stop_hook_summary`, `turn_duration`).
- Codex 0.166 runs every tool through code mode: each `response_item` `custom_tool_call` is named `exec` and holds
  JavaScript (`tools.exec_command({cmd: …})`, `apply_patch`, `spawn_agent`, …), and its `custom_tool_call_output`
  is the script's output. The typed record is the `event_msg` `item_completed` items: `UserMessage`,
  `AgentMessage` (`phase`: `commentary` or `final_answer`; its id equals the matching `response_item` message's),
  `Reasoning` (`summary_text`, e.g. `**Planning…**`), `CommandExecution` (`command`, `parsed_cmd`,
  `aggregated_output`, `exit_code`, `status`), `FileChange` (`changes.<path>.unified_diff`), `Extension`
  (`kind: web.search`), `CollabAgentToolCall` (`tool: spawn_agent`, `receiver_thread_ids`) and
  `ContextCompaction`. There is no `item_started`: a call still running shows only as a `custom_tool_call` with no
  output yet. Turns are bracketed by `task_started` / `task_complete` / `turn_aborted`.
- A Codex subagent is its own rollout; its `session_meta.source.subagent.thread_spawn.parent_thread_id` names the
  parent, and the parent's `CollabAgentToolCall.receiver_thread_ids` names it.
- `codex resume` and `claude --resume` append to the same file.

**TUI input**

- A bracketed paste of `/name args` followed by Enter runs the command in both TUIs (`/compact keep it short`,
  `/compact`, `/rename greeter recording`): a prompt can always be pasted whole.
- Codex invokes skills by mention, `$name` (a pasted `$greet-check please` ran the skill); `/` lists only built-ins.
  Codex 0.166 no longer offers `<codex home>/prompts/*.md` in the `/` popup; saved presets (`/presets`,
  `[tui.presets]` in `config.toml`) took their place.
- The interrupt key is Esc in both (Ctrl+C at an empty Codex prompt quits).
- The `/` catalog (`agent_chat::catalog`, Task 10): Claude Code's command table at 2.1.317 and Codex 0.166's command
  list (read from the installed binaries) give the curated built-ins. Duplicates were not recorded; the catalog follows
  Claude Code's documented order (a personal skill over a project one, a skill over a command of the same name) and
  keeps a built-in over a user entry. A command is named by its path (`dir:name`), not by frontmatter, so a nested
  command can make a plugin's `plugin:name`; the catalog lists plugins last, and the command keeps the name.
- `AskUserQuestion` picker: digits pick an option of a single-select question (which moves to the next question,
  or submits when it is the only one); in a multi-select question digits toggle `[ ]`/`[✔]`; Tab or → moves to the
  next tab; the last tab, "Submit", reviews the answers with `1. Submit answers` (Enter). The option after the
  listed ones is "Type something." (free text), then "Chat about this". Esc cancels.
- Claude's permission dialog: `1. Yes`, `2. Yes, and always allow access to <dir> from this project` (or `Yes, and
  don't ask again for: <prefix> *`), `3. Yes, and switch to auto mode`, `4. No`; digits pick, footer
  `Esc to cancel · Tab to amend`. Codex's: `1. Yes, proceed (y)`, `2. Yes, and don't ask again for commands that
  start with \`…\` (p)`, `3. No, and tell Codex what to do differently (esc)`, footer `Press enter to confirm or esc
  to cancel`.

## PTY captures

`crates/ngterm-mobile/tests/fixtures/agent_screens/<agent>-<cols>x<rows>.bin` holds every byte the agent wrote to a
PTY of that size from launch, cut at the last mark; `<…>.marks.json` gives `cols`, `rows` and `marks`, the byte
offset at which each state was on screen. Replaying `bin[..mark]` into a terminal of that size draws the state.

| state | Claude (44x36, 100x40, 160x48) | Codex (44x36, 100x40, 160x48) |
|---|---|---|
| `idle` | the empty prompt | the empty prompt |
| `working` | a turn in progress (`esc to interrupt`) | a turn in progress (`Working`) |
| `question` | the `AskUserQuestion` picker | — (Codex has no picker) |
| `permission` | a `Bash` permission dialog | a command approval |

`claude-160x48` also marks `question_multi` (the multi-select question after the first answer) and
`question_submit` (the review tab). The recorder ran in a scratch dir; the captures show it as
`/tmp/claude-1000/-home-user-projects-example-workspace/00000000-0000-0000-0000-000000000000/scratchpad/rec/project`,
rewritten with same-length `sanitize.py bin --map` pairs (including the pieces a wrap or a partial redraw splits it
into) so the marks still hold. The 44- and 100-column Claude dialogs wrap it.

## Recording again

`record/` has the tools:

- `ptyrec.py` (needs `pyte`) runs an agent under a PTY of a given size with only the environment it is given,
  records its output and marks offsets, and is driven over a socket: `screen`, `send`, `paste`, `wait`, `quiet`,
  `mark`, `kill`. It answers the terminal queries the TUIs send (cursor position, device attributes, colors).
- `dump-hook.sh AGENT EVENT` saves each payload and the agent's `CLAUDE_*`/`CODEX_*` environment.
- `sanitize.py` rewrites a recording into a fixture (paths, user name, ids, bulk; captures keep their length).

The recording kept off the user's own setup: no `NGTERM_*` in the agents' environment (`ptyrec.py` passes only what
it is given), Claude run with `--setting-sources project,local` so only the scratch project's
`.claude/settings.local.json` hooks ran, and Codex with a private `CODEX_HOME` holding its own `config.toml` (the
dump hooks, the project trusted, `approval_policy = "on-request"`, `sandbox_mode = "workspace-write"`). Claude ran
with `--permission-mode manual` so dialogs appear. After an agent update, record the same scenarios, run the parsers
over the new files and look at the `unknown` rate: format drift shows up there first.
