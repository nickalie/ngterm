# Android agent chat view

## Overview

- When Claude Code or Codex runs in a pane, the Android app shows a messenger-like chat instead of the terminal.
  Underneath it stays a regular `claude` / `codex` TUI in the pane, and the desktop and any other viewer see it as
  before.
- The chat is a full loop. It reads the conversation, sends prompts and answers permission prompts and Claude's
  `AskUserQuestion` with buttons. Stop interrupts with Esc. Typing `/` opens a dropdown of the agent's slash commands
  and skills, and the list narrows as you type.
- **A chat action takes control of the pane** the way the session view's Take control does: a phone that isn't the
  pane's leader claims the lead before it sends anything. Only the leader's input reaches the program, and the
  leader's grid sizes the TUI. Opening or reading the chat never claims the lead.
- **Mobile-only:** the desktop app's UI does not change. The origin (`ngterm-headless` or a desktop instance) gains
  only additive pieces with no UI:
  - a per-pane agent record that the agent-status hooks fill;
  - a Rust-only `agent.*` command group behind `ngtermctl agent`;
  - an `agent` kind on the presentation stream, sent only to viewers that ask for it.
- Settings has an **Agent chat view** toggle, on by default. The pane toolbar has a Chat/Terminal switch, and picking
  Terminal by hand holds for that pane until its next agent starts. An origin without the new pieces shows the
  terminal, as today.
- **Platforms:**
  - The origin pieces (the hooks' POSIX and PowerShell forms, the follower, the catalog) build and are tested on
    Linux, macOS and Windows.
  - The phone cannot reach a Windows origin yet (`docs/backlog/attach-remote-cannot-reach-a-windows-origin.md`:
    remote commands run through `/bin/sh -c`). That gap is left open here, and this plan does not widen it.
- References: slopus/happy and pingdotgg/t3code. Both drive the agent in a structured mode (SDK `stream-json`,
  `codex app-server`). This plan does not: the agent stays a real TUI, and the chat is a second view of it.

## Context (from discovery)

- **Phone:** `crates/ngterm-mobile` is a remote viewer over ssh, documented in its crate docs.
  - Its actor runs exec channels for `ngtermctl live tree`, `live present` and the per-pane `ngterm-session attach`.
  - `api` is the UniFFI facade.
  - Panes attach as managed clients (`LeadAttachment` in `pane.rs`). A follower's input is dropped, and a follower
    draws output laid out for the leader's grid (`ngterm-session/src/server/lead.rs`, `ngterm-core/src/live/lead.rs`).
  - The russh transport counts channels against the server's `MaxSessions` (`transport/russh.rs`).
  - The crate depends on `ngterm-core`, so `ngterm_core::fuzzy::fuzzy_rank` is available to it.
  - `RemoteSession` (`sessions.rs`) holds only `pane_count`, no per-pane records.
  - The phone re-lists sessions only on connect, reconnect, failure or a user action.
- **Presentation stream:** `crates/ngterm-core/src/remote/frames.rs`. The hello negotiates kinds
  (`remote/client.rs::viewer_kinds`), so a new kind reaches only viewers that list it.
- **Android:** `mobile/android/app/src/main/kotlin/com/nickalie/ngterm/android/`:
  - `ui/session/` holds `SessionScreen`, `SessionViewModel`, `SessionBar` and `SessionPanes`;
  - `ui/settings/` holds the Settings screen;
  - `ui/asks/` holds the "An agent asks" cards;
  - `data/SettingsStore.kt`, `data/Alerts.kt`, `data/Connections.kt` and `data/LiveSessions.kt` hold the app's data.
  - `mobile/android/DESIGN.md` is the design system that every screen stays inside.
- **Commands:**
  - Groups are in `crates/ngterm-proto/src/command.rs`, which asserts the group count.
  - `crates/ngterm-proto/tests/catalog.rs::command_wire_names_equal_catalog_commands` and
    `crates/ngterm-ctl/src/main.rs::the_command_tree_is_the_catalogs_cli` require the protocol and the CLI to equal
    the conformance catalog, which is generated from upstream.
  - No Rust-only command exists yet. `conformance/docs/rust-build.md` already has a "Rust-only extensions" section,
    for capabilities.
- **Foreground:** the core reads a pane's foreground on demand through the process table (`project_foreground`,
  `commands/relaunch.rs`). Headless and GPUI both install a table, but no event fires when the program exits. On
  Windows, see `docs/backlog/windows-console-foreground-can-name-a-background-child.md`.
- **Agent hooks:** `crates/ngterm-app/src/integrations/hooks_config.rs` installs:
  - the Claude events `UserPromptSubmit`, `PostToolUse`, `Stop`, `PermissionRequest`, `Notification[permission_prompt]`
    and `SessionEnd`;
  - Claude's restore adapter on `SessionStart` and `SessionEnd`;
  - the Codex events `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `PermissionRequest` and `Stop`.

  On Windows, Codex runs the POSIX adapter through Git for Windows' bash (`WINDOWS_BASH`).
- **Hook behavior to respect:**
  - Codex's `PermissionRequest` is only a candidate: Auto Review may approve it, and the adapter's watcher reports
    `blocked` only when a real prompt is on screen.
  - The restore adapter skips workers (`CLAUDE_CODE_ENTRYPOINT=cli`, no `agent_id`).
  - Hooks pass `--target $NGTERM_SESSION_ID --pane $NGTERM_PANE --pane-id $NGTERM_PANE_ID --socket $NGTERM_SOCKET`.
  - Claude Code has no "permission answered" event.
- **Hook scripts:** `resources/agent-status/`, with upstream deviations as patches in `build/patches/agent-status/`
  (0001 to 0005) maintained through the `agterm-upstream-sync` skill's `scripts/sync-resources.sh`. Skill text
  patches are in `build/patches/skill-ngterm/`.
- **Mobile e2e:** `crates/ngterm-mobile/tests/headless_e2e.rs` runs a real `ngterm-headless` behind an unprivileged
  `sshd`, through `scripts/test-mobile-e2e.sh`.

## Development Approach

- **Testing approach:** TDD for the Rust work: tests first, seen failing, then the code. Code first for the Compose UI:
  design with `frontend-design`, build, then cover with view-model and screenshot tests in the same task.
- Complete each task fully before the next, in small, focused changes.
- **CRITICAL: every task includes new or updated tests** for its code, covering success and error paths.
- **CRITICAL: all tests pass before the next task starts.**
- **CRITICAL: update this plan when scope changes during implementation.**
- Desktop behavior stays unchanged. Every origin addition is new (a Rust-only command group, a presentation kind only
  new viewers list, a hook call) and changes no existing command, `ngtermctl tree` output or the snapshot.
- Checks are sized to the change (CLAUDE.md > Choosing checks): `cargo test -p` on the touched crates, the Gradle tasks
  for `mobile/android` (`scripts/build-android.sh check`), and CI for macOS and Windows.
- **Never touch the user's running ngterm.** Recording fixtures from real agents (Task 1) happens in a private state
  dir and a scratch project with `NGTERM_*` stripped, and every `ngtermctl` call gets an explicit `--socket`.
- **UI tasks:** invoke `frontend-design` before writing Compose code, and stay inside `mobile/android/DESIGN.md` or
  extend it in the same change. The bar is the Android plan's UI/UX standard: 48dp targets, AA contrast, every state
  (empty, loading, error, offline), TalkBack labels, font scale 2.0, reduced motion, insets, and predictive back.

## Testing Strategy

- **Rust unit tests:**
  - `ngterm-core::agent_chat`: parsers on recorded fixture transcripts with golden event output; hook payloads; the
    follower on temp files through a counting reader; the catalog on temp config trees.
  - The `agent.*` dispatcher and the presentation `agent` kind on the `ngterm-host` fakes, the fake process table
    included.
  - `ngtermctl agent` against an in-process server.
  - `ngterm-mobile`:
    - the chat channel over the scripted fake transport;
    - the leadership sequence (follower → claim → leader → match → send);
    - the screen matchers on recorded dialog screens replayed through libghostty-vt, at several widths and with
      negative cases.
- **Adapter tests:** the hook scripts' new call, the worker guards, and Codex's auto-reviewed candidate, in the
  POSIX form (bash on Windows CI too) and the Claude PowerShell form.
- **Rust e2e:** `headless_e2e.rs` gains a fake-agent case:
  - a foreground program replays a recorded idle screen and writes a fixture transcript;
  - the phone discovers it through the presentation stream, reads and pages the chat, and resumes after a severed
    connection;
  - it claims the lead from another client and gets its prompt into the PTY.
- **Android:**
  - JVM view-model tests;
  - Roborazzi goldens of the chat, the cards and the `/` dropdown at three widths, light and dark, and at font scale
    2.0;
  - `./gradlew check`.
- No conformance ids: upstream has no mobile app. CI has no agent API keys, so runs against real agents are the manual
  walkthrough in Post-Completion.

## Progress Tracking

- Mark completed items `[x]` immediately.
- Add newly found tasks with ➕, and blockers with ⚠️.
- Update the plan when the implementation departs from it.

## Solution Overview

1. **Record.** One hook command, `ngtermctl agent hook <claude|codex> <event> --target … --pane … --pane-id …
   --socket …`, reads the hook payload on stdin and parses it in Rust. It sends `agent.report` to the core, which
   keeps a record per pane identity in memory:
   - the agent;
   - the transcript path and conversation id;
   - the last permission payload, as a candidate.

   Which events call it:
   - Claude's `SessionStart` and `UserPromptSubmit`, and Codex's `SessionStart` and `UserPromptSubmit`, set or
     re-assert the record. Re-asserting covers an origin restarted while the agent kept running in its live session.
   - `PermissionRequest` stores the candidate.
   - Claude's `SessionEnd` clears the record.

   The core never persists the record. It drops it when the pane identity leaves the model, and it checks lazily
   whether the pane's foreground is back to a shell: in `agent.get` and when the presentation frame is built. That
   covers Codex, whose `SessionEnd` comes only when the CLI exits, once for every thread it loaded (Task 1).
2. **Approvals without new clearing logic.** A candidate is an approval only while that pane's agent status is
   `blocked`, which the existing adapters set and clear (Codex's watcher included, so Auto Review never makes a
   card). It is done when the status leaves `blocked` or when the transcript writes the `tool_result` for that tool
   use (a denial writes one, an interrupt writes a notice).
3. **Discovery.** The presentation stream gains an `agent` kind. Per pane, it carries the agent and conversation id,
   or nothing. It is sent at attach and whenever the record changes, and only to viewers whose hello lists `agent`, so
   desktop Attach Remote is unaffected. The tree is untouched.
4. **Stream.** `ngtermctl agent chat --target SESSION [--pane left|right]` runs on the origin under the phone's ssh:
   - it calls `agent.get`, then follows the transcript file itself, so the GUI process does no file IO;
   - it parses the file with `ngterm-core::agent_chat` and prints NDJSON;
   - it re-reads the record about once a second; a new path means a `reset` and the new file, and a cleared record
     ends the stream;
   - it exits when its stdin reaches EOF (the ssh channel gone), the way `stream_bridge.rs` watches its input.
5. **Catalog.** `ngtermctl agent commands --target SESSION [--pane …]` prints the agent's slash commands and skills
   once, as JSON.
6. **Phone.**
   - `ngterm-mobile` opens a chat channel only for panes that are shown in chat mode, and closes it on hide. That
     respects `MaxSessions`.
   - The pane's normal attach stays open behind the chat, at the grid the terminal view would have.
   - Every action is the same sequence: claim the lead if not the leader, wait for `Leader` and a settled redraw,
     check the per-agent screen matcher, then send through `Pane` input.
   - Any mismatch makes the phone offer "Answer in terminal" instead of sending keys.
7. **Android.** A chat screen takes the terminal's place, with a composer (Send, Stop, the `/` dropdown) and pinned
   Needs-you cards in the `ui/asks/` vocabulary.

**Key decisions:**

- **Transcripts, not structured modes,** so the agent stays a real TUI. The cost is that the formats are undocumented
  and drift. Fixtures record agent versions, and an entry that won't parse becomes an `unknown` event instead of an
  error.
- **Approvals come from hook payloads, gated by the agent status the adapters already maintain.** A transcript never
  records a dialog that is waiting, and the status already filters Codex's Auto Review.
- **One stdin-fed hook command:**
  - payload parsing lives in Rust, tested on fixtures;
  - nothing large travels through argv, which matters for Windows' command-line limit;
  - each hook script just pipes its stdin;
  - only four events spawn it (`SessionStart`, `UserPromptSubmit`, `PermissionRequest`, Claude's `SessionEnd`), so
    `PostToolUse` costs nothing extra.
- **The presentation stream announces agents,** not the tree. It is already live and per session, and its kinds are
  negotiated, so an agent starting in a shown pane switches it to Chat at once.
- **The follower runs in `ngtermctl`,** which keeps the GUI process out of it and is identical on every OS: size
  polling, no `tail` and no PowerShell.
- **Filtering uses `fuzzy_rank` exactly as the desktop palettes do,** so the `/` dropdown ranks the same way.
- **The agent's config dir is derived from `transcript_path`:** `<dir>/projects/…` for Claude, `<dir>/sessions/…`
  for Codex. That way `CLAUDE_CONFIG_DIR` and `CODEX_HOME` from the agent's environment hold even though `ngtermctl`
  runs under ssh's login environment. The same derivation validates the path: a path outside such a tree is refused.

## Technical Details

**The record** (core, in memory, keyed by pane identity, i.e. `NGTERM_PANE_ID`):

```text
AgentRecord { agent: claude|codex, transcript: PathBuf, conversation_id, updated_at,
              approval_candidate: Option<{ tool, title, detail, tool_use_id?, since }> }
```

**`agent.*` commands** (Rust-only; listed as extensions per Task 2):

- `agent.report`: from `agent hook`. It takes the pane identity, the agent, the event, and the parsed payload fields
  (transcript path, conversation id, permission tool and detail).
- `agent.get`: the pane's record and whether an approval is live, after the lazy foreground check; or `no agent in
  this pane`.

**The presentation `agent` kind:** per pane role, `{ role, agent, conversation_id }` or `{ role, agent: null }`. It is
sent at attach, on every record change, and when the lazy check clears a record.

**Chat events** (NDJSON, one per line). Each has `id` (stable, so a later line updates the same item) and `cursor` (the
byte offset of its source line):

| event | fields |
|---|---|
| `conversation` | agent, conversation id, cwd, model |
| `user` | text, image count |
| `assistant` | Markdown text |
| `thinking` | text (shown collapsed; Claude writes it empty, Codex writes a summary title) |
| `tool` | kind (shell, edit, read, search, web, task, other), one-line title, status (running, ok, error), output excerpt capped at 4 KiB, unified diff for edits, parent task id for subagent entries |
| `question` | `AskUserQuestion`: each question, its options (label, description), and whether several can be picked |
| `approval`, `approval_done` | tool, title, detail, tool use id when known |
| `notice` | compaction, interrupt, API error, with text |
| `unknown` | the raw entry type, or `oversized` for a line over the cap (8 MiB) |
| `reset` | the conversation switched |
| `live` | the backlog is done and following has started |

**`agent chat` modes:**

- the default: read backward from the end until about 200 events, then `live` and follow;
- `--before CURSOR --limit N`: one older page, then exit;
- `--after CURSOR`: everything after the cursor, then `live` and follow (used on reconnect).

Following polls the file size every 250 ms and buffers a partial last line. A file that shrinks or is replaced means a
`reset`. Subagent entries fold under their task tool's id. Claude writes them as separate files,
`<session-id>/subagents/agent-<id>.jsonl`, whose `.meta.json` names the parent `Agent` tool use (`toolUseId`); the
follower follows them as extra sources. Codex 0.166 runs every tool through code mode (one `exec` custom tool call
holding JavaScript), so its chat events come from the `event_msg` `item_completed` items, and a `custom_tool_call`
with no output yet is a running call (Task 1's fixture README).

**Command catalog** (`agent commands`): `{ name, description, kind: builtin|skill|command|plugin, argument_hint }`.

- **Claude:**
  - the curated built-ins, with the version they were checked against;
  - user skills from `<config>/skills/*/SKILL.md`; a directory without a `SKILL.md` is skipped, and so is a skill
    whose frontmatter says `user-invocable: false`;
  - project skills from `<cwd>/.claude/skills`;
  - custom commands from `<config>/commands/**/*.md` and `<cwd>/.claude/commands`, with subdirectories as `a:b`;
  - the skills and commands of each plugin enabled in `<config>/settings.json` (`enabledPlugins`), found through
    `<config>/plugins/installed_plugins.json`, named `plugin:name`.
- **Codex:**
  - the curated built-ins;
  - skills from `<codex home>/skills/*/SKILL.md`, as `$name`, the mention Codex invokes them with (Task 1);
  - no custom prompts: Codex 0.166 no longer offers `<codex home>/prompts/*.md` (Task 1).
- A name and description come from the file's frontmatter, falling back to the file name and its first line. One
  unreadable file is skipped with no error.

**Phone actions** (`ngterm-mobile`). Each is lead, then settle, then match, then send:

- **Prompt:** a bracketed paste of the text, then Enter. It needs the idle or working prompt matcher, and is refused
  (`DialogOpen`) while a dialog matcher fires. A pasted `/name args` runs as a command in both TUIs (Task 1), so a
  leading `/command` is pasted with the rest.
- **Stop:** Esc, in both agents (Task 1).
- **Approval:**
  - Card labels come from the rows' dialog when the matcher can read them (`1. Yes`, `2. Yes, and don't ask
    again…`, `3. No…`), and otherwise from the payload's tool only.
  - On tap, after taking the lead, the matcher re-reads the dialog at the phone's grid. The phone sends the key of the
    option with the same label, or returns `NotOnScreen`.
- **Question:** the picker is confirmed on screen, then digit keys pick an option (single-select, which moves on) or
  toggle it (multi-select), Tab moves to the next question, "Other" text goes through the "Type something." option,
  and the review tab's `1. Submit answers` is sent with Enter (Task 1).
- **Settled** means a `Leader` role plus no new output for a short quiet period (150 ms, injectable) after the
  resize redraw.

## What Goes Where

- **Implementation Steps** (`[ ]`): code, tests, fixtures and docs in this repo.
- **Post-Completion** (no checkboxes): the walkthrough on a device against real agents, and reinstalling hooks on
  origins.

## Implementation Steps

### Task 1: Record fixtures and settle the agents' facts

**Files:**
- Create: `crates/ngterm-core/tests/fixtures/agent_chat/{claude,codex}/…` (transcripts and hook payloads),
  `crates/ngterm-core/tests/fixtures/agent_chat/README.md` (versions, how each was recorded, the settled facts)
- Create: `crates/ngterm-mobile/tests/fixtures/agent_screens/…` (PTY captures, next to `rows-golden.bin`)

- [x] record real Claude Code and Codex sessions in a scratch project with `NGTERM_*` stripped. Capture hook payloads
      with a scratch-project hook that dumps stdin (`.claude/settings.local.json`, and Codex's project config). The
      user's installed hooks are never touched. Cover:
  - plain chat, thinking, shell, edit with a diff, read, search, web, and a subagent;
  - an `AskUserQuestion` with a single-select and a multi-select question;
  - an approval allowed, one denied and one interrupted; a Codex call that Auto Review approved;
  - `/compact`, `/clear` (a new conversation) and a resume;
  - a `claude -p` worker run from inside the session, for the worker guard.

  Note the agent versions.
- [x] capture the raw PTY bytes of each agent's permission dialog, `AskUserQuestion` picker, idle prompt and working
      turn, at three widths (a phone portrait grid, a tablet grid, a desktop grid)
- [x] settle and write down in the README:
  - what Codex's `PermissionRequest` payload carries, and whether its rollout records approvals;
  - where Claude subagent entries live (inline `isSidechain` or `<session-id>/subagents/` files) and the current name
    of the subagent tool;
  - whether `AskUserQuestion` also fires `PermissionRequest` (if so, Task 6 suppresses the duplicate approval);
  - how Codex invokes skills from its composer;
  - whether a bracketed paste of `/name args` runs as a command in each TUI;
  - which key submits an `AskUserQuestion` and how multi-select toggles;
  - Codex's interrupt key.

  Where a fact differs from this plan's Technical Details, update the plan.
- [x] strip personal paths and content from the fixtures (rewrite them to `/home/user/project`)

### Task 2: Rust-only commands in the catalog tests

**Files:**
- Modify: `crates/ngterm-proto/src/command.rs` (a Rust-only command list, the group count),
  `crates/ngterm-proto/tests/catalog.rs`, `crates/ngterm-ctl/src/main.rs` (`the_command_tree_is_the_catalogs_cli`),
  `conformance/docs/rust-build.md` (Rust-only extensions: commands)

- [x] write the tests first:
  - both equality tests compare against the catalog plus an explicit Rust-only list (wire names and CLI paths);
  - a name in both the list and the catalog fails;
  - a Rust-only name missing from the list fails.
- [x] add the list (empty until Task 6) and document the rule in `rust-build.md`. The generated catalog is not hand
      edited; it is regenerated from upstream.
- [x] `cargo test -p ngterm-proto -p ngterm-ctl` must pass

### Task 3: `agent_chat` event model and the Claude parser

**Files:**
- Create: `crates/ngterm-core/src/agent_chat.rs`, `crates/ngterm-core/src/agent_chat/{event,claude}.rs`,
  `crates/ngterm-core/src/agent_chat/tests/claude.rs`
- Modify: `crates/ngterm-core/src/lib.rs`

- [ ] write golden tests first: each Claude fixture produces its expected event list (`serde_json` goldens next to the
      fixture)
- [ ] define `ChatEvent` and its NDJSON form (serde, `skip_serializing_none`) as in Technical Details
- [ ] implement the Claude line parser:
  - `user`, `assistant` text, thinking, and `tool_use` paired with `tool_result` by id;
  - edit diffs from the edit tools' inputs;
  - `AskUserQuestion` becomes `question`;
  - subagent entries are parented to their task, as Task 1 found;
  - summary and compaction entries become `notice`.
- [ ] test the error and edge cases: a malformed line, an unknown type (becomes `unknown`), a `tool_result` with no
      `tool_use` in the window, oversized output (capped), and non-UTF-8 output
- [ ] `cargo test -p ngterm-core agent_chat` must pass

### Task 4: The Codex parser

**Files:**
- Create: `crates/ngterm-core/src/agent_chat/codex.rs`, `crates/ngterm-core/src/agent_chat/tests/codex.rs`

- [ ] write golden tests first, one per Codex fixture
- [ ] map `session_meta` to `conversation`, the user and agent messages, reasoning to `thinking`, the tool items
      (Codex 0.166, Task 1: `CommandExecution` → shell, `FileChange` → edit with its `unified_diff`, `Extension`
      `web.search` → web, `CollabAgentToolCall` → task, and an `exec` `custom_tool_call` with no output yet → a running
      tool), and interrupts (`turn_aborted`) and errors to `notice`
- [ ] when an `event_msg` and a `response_item` describe the same message, emit it once
- [ ] test the error and edge cases: an unknown item type, a truncated last line, and a call with no output yet
      (`running`)
- [ ] `cargo test -p ngterm-core agent_chat` must pass

### Task 5: The follower: backlog, paging and following

**Files:**
- Create: `crates/ngterm-core/src/agent_chat/follow.rs`, `crates/ngterm-core/src/agent_chat/tests/follow.rs`

- [ ] write tests first on temp files, read through a byte-counting reader:
  - the backlog finds the last N events by reading backward, including a tool call whose result lies past the window
    start;
  - `--before` and `--after` cursors;
  - appended lines, and a partial line completed later;
  - truncation and replacement emit `reset`;
  - opening a 50 MB file reads no more than the backlog window plus a bounded margin (the bytes are asserted, not
    timed);
  - a line over 8 MiB becomes `unknown{oversized}` without being buffered whole;
  - extra subagent sources, if Task 1 found separate files.
- [ ] implement a toolkit-neutral `Follower` over `Read + Seek` plus a size probe, with an injectable clock and poll
      interval (250 ms by default), the same code on every OS
- [ ] `cargo test -p ngterm-core agent_chat` must pass

### Task 6: The origin's agent record, `agent.report` and `agent.get`

**Files:**
- Modify: `crates/ngterm-proto/src/command.rs` (the `AgentCmd` group, added to the Rust-only list),
  `crates/ngterm-proto/src/result.rs`
- Create: `crates/ngterm-core/src/agent_chat/hook.rs` (payload parsing), `crates/ngterm-core/src/commands/agent.rs`,
  `crates/ngterm-core/src/commands/tests/agent.rs`
- Modify: `crates/ngterm-core/src/commands/mod.rs`, and the core state that holds per-pane runtime data (in memory,
  never in the snapshot)

- [ ] write payload tests first on Task 1's payloads:
  - each event's transcript path, conversation id, permission tool and detail (command, file path) for both agents;
  - Codex's null-`transcript_path` memory thread is ignored;
  - a payload missing fields is rejected with a message;
  - a transcript path outside a `projects/` or `sessions/` tree is refused.
- [ ] write dispatcher tests first on the `ngterm-host` fakes:
  - report, then re-assert (no change), then a new path (a new conversation), then clear;
  - an approval candidate is live only while the pane's status is `blocked`, and done when the status changes;
  - an `AskUserQuestion` that triggered `PermissionRequest` makes no duplicate (Task 1 found that it does fire);
  - an unknown pane identity;
  - a record dropped when its pane identity leaves the model;
  - the record never appears in the snapshot.
- [ ] write the lazy foreground tests first, through the fake process table: `agent.get` on a pane whose foreground is
      back to a shell clears the record and answers `no agent in this pane`
- [ ] implement `agent.report` and `agent.get` keyed by pane identity
- [ ] `cargo test -p ngterm-proto -p ngterm-core` must pass

### Task 7: The presentation stream's `agent` kind

**Files:**
- Modify: `crates/ngterm-core/src/remote/{frames,client}.rs` and the origin side that builds presentation frames
- Modify: the matching tests under `crates/ngterm-core/src/remote/`

- [ ] write tests first:
  - a viewer whose hello lists `agent` gets the per-pane agent frame at attach and on each record change, including a
    clear by the lazy foreground check (made when the frame is built);
  - a viewer that does not list it (desktop Attach Remote) gets the same frames as before, byte for byte;
  - `ngtermctl tree` output is unchanged.
- [ ] implement the kind; desktop's own viewer does not list it
- [ ] `cargo test -p ngterm-core remote` must pass

### Task 8: `ngtermctl agent hook` and the hook scripts

**Files:**
- Create: `crates/ngterm-ctl/src/agent.rs` (the `hook` subcommand), `crates/ngterm-ctl/tests/agent_hook.rs`
- Modify: `crates/ngterm-ctl/src/main.rs` (and the Rust-only CLI list)
- Modify: `resources/agent-status/ngterm-claude-status.{sh,ps1}`, `ngterm-codex-status.sh`,
  `ngterm-agent-restore.{sh,ps1}` (whichever owns each event)
- Create: `build/patches/agent-status/0006-agent-transcript.patch`, through the `agterm-upstream-sync` skill's
  `scripts/sync-resources.sh` convention
- Modify: `crates/ngterm-app/src/integrations/adapter_tests.rs`, the deviations list in `integrations/mod.rs`

- [ ] write `ngtermctl agent hook` tests first: it reads stdin, passes `--target`, `--pane`, `--pane-id` and
      `--socket` as the existing hooks do, and sends `agent.report`. Also test empty stdin, a malformed payload (a
      non-zero exit that never blocks the agent) and a missing socket.
- [ ] write adapter tests first, with a stub `ngtermctl` on `PATH` that logs its argv and stdin:
  - the four events pipe their payload to `agent hook`;
  - a `claude -p` worker and a subagent payload make no call (`CLAUDE_CODE_ENTRYPOINT`, `agent_id`, and the status
    adapter's process-topology guard); Codex's subagent events carry `agent_id` and the subagent's transcript path
    (Task 1);
  - `PostToolUse` and `Stop` make no new call;
  - a Codex call that Auto Review approved never yields a live approval (end to end with the Task 6 dispatcher fake).
- [ ] implement the calls in the POSIX scripts, then in the Claude PowerShell forms. Codex on Windows runs the POSIX
      adapter through `WINDOWS_BASH`, so its test also runs under bash on Windows CI.
- [ ] record the patch as 0006
- [ ] `cargo test -p ngterm-ctl agent -p ngterm-app integrations` must pass

### Task 9: `ngtermctl agent chat`

**Files:**
- Modify: `crates/ngterm-ctl/src/agent.rs`
- Create: `crates/ngterm-ctl/tests/agent_chat.rs`

- [ ] write tests first against an in-process server with a recorded transcript:
  - the default mode prints the backlog then `live`, and appended lines follow;
  - `--before` and `--after`;
  - a new record path emits `reset` and the new file;
  - a cleared record ends the stream with a zero exit;
  - with no record, the exit is non-zero with `no agent in this pane`;
  - stdin EOF ends the process within a bound, on Unix and Windows;
  - approvals are interleaved as `approval` and `approval_done`, including `done` from the transcript's `tool_result`
    after a denial.
- [ ] implement `agent chat --target SESSION [--pane left|right]`: `agent.get`, then `agent_chat::Follower`, with
      the record re-checked about once a second and stdin watched as in `stream_bridge.rs`
- [ ] `cargo test -p ngterm-ctl agent` must pass

### Task 10: `ngtermctl agent commands`: the slash-command catalog

**Files:**
- Create: `crates/ngterm-core/src/agent_chat/catalog.rs`, `crates/ngterm-core/src/agent_chat/tests/catalog.rs`
- Modify: `crates/ngterm-ctl/src/agent.rs`

- [ ] write tests first on temp config trees:
  - user and project skills, with a directory missing `SKILL.md` skipped and `user-invocable: false` hidden;
  - nested custom commands become `a:b`;
  - enabled versus disabled plugins;
  - Codex skills (`$name`; 0.166 offers no custom prompts, Task 1);
  - the config dir derived from `transcript_path` (a custom `CLAUDE_CONFIG_DIR` or `CODEX_HOME`);
  - an unreadable or malformed file is skipped;
  - a duplicate name resolves the way the agent resolves it (per Task 1).
- [ ] add the curated built-in lists for both agents, citing the versions from Task 1
- [ ] implement the catalog and `ngtermctl agent commands --target SESSION [--pane …]` (JSON)
- [ ] `cargo test -p ngterm-core agent_chat -p ngterm-ctl agent` must pass

### Task 11: `ngterm-mobile`: agent discovery, the chat channel and the catalog

**Files:**
- Create: `crates/ngterm-mobile/src/chat.rs`, `crates/ngterm-mobile/src/chat/{records,tests}.rs`
- Modify: `crates/ngterm-mobile/src/{lib,api,actor,presentation}.rs`, and the fake transport

- [ ] write tests first on the scripted fake transport:
  - the hello lists `agent`, and an agent frame becomes a `MobileEvent::PaneAgent { session, role, agent,
    conversation_id }`;
  - opening a chat channel delivers `ChatEvent`s in order, and a later line updates the same `id`;
  - `load_older` runs `--before`;
  - a severed connection resumes with `--after` (no duplicates, no gaps);
  - `reset`;
  - the channel ending tells the app to show the terminal;
  - an origin without `agent` (the command fails, or no frame ever arrives) degrades to the terminal with no error
    surfaced;
  - chat channels exist only for panes shown in chat mode and close on hide, so the channel count stays bounded;
  - the catalog is fetched once per conversation and refetched after `reset`.
- [ ] add the UniFFI surface:
  - `open_chat(pane)` returns a `ChatEvent` stream through `EventSink`;
  - `load_older`;
  - `close_chat`;
  - `agent_commands(pane)`;
  - a pure `filter_commands(query, items)` that calls `ngterm_core::fuzzy::fuzzy_rank` as the palettes do.
- [ ] `cargo test -p ngterm-mobile` and `scripts/build-android.sh check` must pass (new UniFFI records reach Kotlin)

### Task 12: `ngterm-mobile`: leadership, screen matchers and verified input

**Files:**
- Create: `crates/ngterm-mobile/src/chat/{matchers,input}.rs`, `crates/ngterm-mobile/src/chat/tests/{matchers,input}.rs`
- Modify: `crates/ngterm-mobile/src/api/pane.rs`, `crates/ngterm-mobile/src/pane.rs`

- [ ] write matcher tests first on Task 1's PTY captures, replayed through libghostty-vt into rows at each recorded
      width:
  - each agent's permission dialog yields its option labels and keys;
  - the picker yields its questions and options;
  - the idle prompt and a working turn are recognized;
  - negatives: a dialog that has scrolled away, a half-drawn frame, a user's shell output that quotes a dialog, and
    another agent's dialog.
- [ ] write the sequence tests first on the fake:
  - a follower claims the lead, waits for `Leader` and the settled redraw, re-matches, then sends;
  - a lead claim that fails or times out sends nothing;
  - the dialog vanishing during the claim gives `NotOnScreen`;
  - a leader skips the claim;
  - opening or reading the chat never claims the lead.
- [ ] implement the per-agent matchers and `Pane::send_prompt`, `interrupt`, `answer_approval(label)` and
      `answer_question(selection, other)`, with the outcomes `NotOnScreen`, `DialogOpen` and `LeadRefused`
- [ ] test the key sequences each call sends (bracketed paste and Enter; the slash-command form per Task 1; the
      interrupt key; option keys; picker keys)
- [ ] the hidden pane in chat mode keeps the grid the terminal view would have, so a claim lays the TUI out for the
      phone. Test that switching Chat ↔ Terminal does not resize.
- [ ] `cargo test -p ngterm-mobile` must pass

### Task 13: Android: chat data, view model, Settings and mode switching

**Files:**
- Create: `mobile/android/app/src/main/kotlin/com/nickalie/ngterm/android/ui/chat/ChatViewModel.kt`,
  `ui/chat/ChatItems.kt` (the folded list model)
- Modify: `data/SettingsStore.kt`, `ui/settings/SettingsSections.kt`, `ui/session/SessionViewModel.kt`,
  `ui/session/SessionBar.kt`
- Create: matching tests under `app/src/test/…/ui/chat/`

- [ ] add the **Agent chat view** setting (on by default) to `SettingsStore` and the Settings screen
- [ ] write `ChatViewModel` tests first:
  - events upsert by `id`, and tool results fold into their calls and subagents under their task;
  - loading older items keeps the scroll anchor;
  - `reset` starts a new conversation divider;
  - approval and question cards come up and go away, including when another viewer answers (`approval_done`, or a
    matcher miss on the next check);
  - send and stop, and the outcomes `NotOnScreen`, `DialogOpen` and `LeadRefused`, which show "Answer in terminal" or
    the reason.
- [ ] write the mode tests first:
  - a `PaneAgent` event for a shown pane with the setting on switches it to Chat;
  - the setting off always shows Terminal;
  - picking Terminal by hand holds until the pane's next conversation id;
  - the agent ending returns to Terminal.
- [ ] implement both, plus the Chat/Terminal switch in `SessionBar`
- [ ] `scripts/build-android.sh check` must pass

### Task 14: Android: the chat screen (frontend-design)

**Files:**
- Create: `ui/chat/ChatScreen.kt`, `ui/chat/MessageRows.kt`, `ui/chat/ToolRow.kt`, `ui/chat/DiffView.kt`,
  `ui/chat/NeedsYouCard.kt`
- Modify: `mobile/android/DESIGN.md`, `gradle/libs.versions.toml` (mikepenz `multiplatform-markdown-renderer`), the
  screenshot tests under `app/src/test/screenshots/`

- [ ] invoke `frontend-design` for the chat. Extend `DESIGN.md` with the chat's components:
  - prompt bubbles, right-aligned on a container tone;
  - full-width Markdown replies with no bubble, and mono code blocks with no highlighting;
  - one-line tool rows (kind glyph, mono title, status glyph and word, tap to expand);
  - diffs with added lines in verdigris and removed lines in oxide;
  - collapsed thinking and subagents;
  - Needs-you cards built from the `ui/asks/` vocabulary ("An agent asks") in the Attention tone, pinned above the
    composer.
- [ ] build the screen with its empty, loading (backlog), reconnecting and "agent ended" states, and loading older
      items on scroll
- [ ] approval cards: the tool, its command or diff, and the buttons (labels from the matcher, or the generic set when
      the rows can't be read yet). Question cards: radio buttons or checkboxes plus a "Type an answer" field.
- [ ] Roborazzi goldens at three widths, light and dark, and at font scale 2.0, covering a long conversation, an open
      approval, a multi-select question, an expanded diff, and the empty and error states. Add TalkBack labels and a
      test for them.
- [ ] `scripts/build-android.sh check` must pass

### Task 15: Android: the composer and the `/` dropdown

**Files:**
- Create: `ui/chat/Composer.kt`, `ui/chat/SlashCommandMenu.kt`, and their tests
- Modify: `ui/chat/ChatViewModel.kt`, `mobile/android/DESIGN.md`

- [ ] write the view-model tests first:
  - a `/` at the start of the field opens the menu with the full catalog;
  - each keystroke narrows it through `filter_commands`;
  - a space or deleting the `/` closes it;
  - picking an item inserts `/name ` and keeps the keyboard up;
  - an empty catalog (an older origin, or a fetch still running) shows a quiet loading row, never an error.
- [ ] build the composer: a multi-line field, Send, Stop while the status is working, and disabled with a reason while
      a dialog is open
- [ ] build the dropdown above the composer (frontend-design, recorded in `DESIGN.md`): each row has the name, a
      one-line description and a kind label (built-in, skill, plugin, command); it shows at most about 6 rows and
      scrolls; it works with the keyboard and with TalkBack
- [ ] goldens of the open menu, a filtered menu and a menu with no match, at the same widths, themes and font scale
- [ ] `scripts/build-android.sh check` must pass

### Task 16: Android: notifications open the chat

**Files:**
- Modify: `mobile/android/app/src/main/kotlin/com/nickalie/ngterm/android/data/Alerts.kt`, the notification's open
  request handling, and their tests

- [ ] write a test first: a "needs you" notification for a pane in chat mode opens the chat scrolled to its card,
      and with the setting off it opens the terminal
- [ ] implement the deep link
- [ ] `scripts/build-android.sh check` must pass

### Task 17: End to end against `ngterm-headless`

**Files:**
- Modify: `crates/ngterm-mobile/tests/headless_e2e.rs`, `tests/support/*`, `scripts/test-mobile-e2e.sh` (if it needs
  the fixtures staged)

- [ ] add a fake-agent program. It stays in the pane's foreground, replays Task 1's recorded idle-screen capture,
      writes a fixture transcript, and pipes a fixture payload to `ngtermctl agent hook` under the private socket.
- [ ] through `MobileCore`:
  - the presentation stream announces the agent;
  - open the chat and see the backlog;
  - append lines and see them;
  - page back;
  - sever the connection and resume with no duplicates;
  - with another client leading, `send_prompt` claims the lead and the text appears in the pane's rows;
  - set a `blocked` status plus a candidate and see the approval;
  - end the fake agent (the foreground goes back to the shell) and see the agent cleared and the stream end.
- [ ] run `scripts/test-mobile-e2e.sh`; it must pass

### Task 18: Verify acceptance criteria

- [ ] every Overview point works:
  - chat in place of the terminal, and the full loop;
  - the `/` dropdown narrowing as you type;
  - the Settings toggle, and the toolbar switch with its sticky Terminal choice;
  - lead taken only on action;
  - an older origin falling back to the terminal.
- [ ] the desktop is unchanged: `cargo test -p ngterm-core -p ngterm-server -p ngterm-ctl -p ngterm-app`, scoped to
      the touched crates, with the catalog tests green
- [ ] `cargo test -p ngterm-mobile`, `scripts/test-mobile-e2e.sh` and `scripts/build-android.sh check`
- [ ] strict clippy on the touched crates; CI green on Linux, macOS and Windows
- [ ] a revmux round over the whole change, with its verified findings fixed

### Task 19: [Final] Update documentation

- [ ] README: the Android section gains the chat view, its Settings toggle, taking the lead on action, and the origin
      requirement (hooks reinstalled); the Windows origin stays listed as unreachable
- [ ] the crate docs of `ngterm-mobile` (module map: `chat`) and `ngterm-core` (`agent_chat`); the `ngtermctl agent`
      help text
- [ ] if agents should know about `ngtermctl agent`, the `ngterm` skill through a `build/patches/skill-ngterm/` patch
- [ ] move this plan to `docs/plans/completed/`

## Post-Completion

*Needs a device, real agents or other machines; no checkboxes.*

**Manual verification:**

- On a phone and a tablet against a real Linux or macOS origin, with live Claude Code and Codex:
  - a long session with tools, diffs and subagents;
  - approvals answered from the phone while the desktop leads (the lead moves, and the TUI reflows to the phone);
  - one answered from the desktop while the phone shows its card;
  - a Codex call approved by Auto Review (no card);
  - an `AskUserQuestion` with both kinds of question;
  - `/compact`, `/clear` and a resume;
  - the `/` dropdown with user, project and plugin skills;
  - Stop mid-turn;
  - flight mode and back mid-stream;
  - restarting the origin while an agent runs (the chat comes back at the next prompt).
- After an agent update, re-record a fixture and check the parsers' `unknown` rate. Drift shows up there first.

**External:**

- Origins need the new `ngtermctl` and a reinstall of the agent-status hooks (the integrations installer) before their
  panes offer chat. Until then the app shows the terminal.
- A Windows origin waits on `docs/backlog/attach-remote-cannot-reach-a-windows-origin.md`.
