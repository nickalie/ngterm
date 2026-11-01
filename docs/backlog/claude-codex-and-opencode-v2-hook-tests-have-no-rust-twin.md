---
worth: yes
added: 2026-10-24
---
# The Claude, Codex and OpenCode v2 status hooks have no port of upstream's tests

Upstream drives each agent-status hook it ships through its real host and asserts the reporter command lines; this
build ported only the OpenCode v1 suite (`crates/ngterm-app/tests/opencode_status.rs`, upstream's
`OpenCodeStatusHookTests.swift`). Still without a twin, against the scripts under `resources/agent-status/`:

- `ClaudeStatusHookTests.swift` (13 tests: the pane-agent ancestor walk, silent workers under launcher chains and
  scripts, the `NGTERM_AGENT_RE` override and its fail-open, the pane boundary, the outside-ngterm no-op) over
  `ngterm-claude-status.sh`. `crates/ngterm-app/src/integrations/adapter_tests.rs` covers only the Stop hook's
  background-task check.
- `CodexStatusHookTests.swift` (26 `@Test`s at v0.39.1, five of them parameterized: lifecycle mapping, the Stop hook's
  trailing-question detection with code spans, fences and URLs, and the visible-prompt watcher) over
  `ngterm-codex-status.sh`. Its two v0.39.1 cases, `eventWithoutTranscriptLeavesStatusAndWatcherAlone` and
  `eventWithTranscriptReportsStatus`, have a twin in `crates/ngterm-app/tests/codex_status.rs`, whose `run` harness
  the rest can extend.
- `OpenCodeV2StatusHookTests.swift`, which runs `scripts/tests/opencode-v2-status.mjs` (35 `node --test` cases) over
  `opencode/ngterm-v2/`.

Found while porting the OpenCode v1 tests (docs/plans/20261023-upstream-sync-v0.38.1.md, Task 33), which was scoped
to that suite. The ports follow the same shape: run the shipped script with a recording wrapper, in this build's
names; the v2 one needs node, which `linux.yml` already installs for the v1 tests.
