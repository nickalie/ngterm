# Upstream sync v0.41.1 -> v0.42.3

## Overview

- Bring ngterm from upstream `umputun/agterm` v0.41.1 to v0.42.3 (104 commits; releases v0.42.0 of 2026-11-11,
  v0.42.1 of 2026-11-12, v0.42.2 and v0.42.3 of 2026-11-13). The mechanical part is commit `ea08c441`, before this plan
  on the branch: the catalog and ui ids regenerated from v0.42.3 (11 commands, 14 fields, 11 CLI leaves, 4 ui ids),
  the parity target, the macOS reference level and the committed-catalog snapshots moved, the 4 new ui ids parked in
  `crates/ngterm-gpui/src/ui_ids.rs` `PENDING`. Left out: the Rust claim (Task 41), the agent-status package and the
  bundled skill (their first patches conflict: Tasks 6 and 35), the renderer resources (Task 2).
- The bump does not pass CI alone: it lands together with Task 1 (Context > Baseline).
- This plan ports, each task landing alone with main working:
  1 wire and CLI shapes with catalog coverage; 2 renderer resources; 3 `session go --to N`; 4 `session status
  --if-idle`; 5 the checkmark shape; 6 the agent-status package and installers; 7 scratch cwd; 8 `remoteSessionID`;
  9 the new-session folder; 10 badge reset on typing; 11 session links; 12 re-run quoting; 13 lock release;
  14 the color scheme report; 15 text wrap; 16 terminal colors in text pages; 17 Markdown local links; 18 Convert to
  Companion; 19 page sessions; 20 page sessions across machines; 21 forwarded clicks; 22 Markdown links on remote
  pages; 23 remote log pages and text controls; 24 stream upkeep; 25 the session host; 26 session-bound picks and
  timeouts; 27 held interactions on a headless origin; 28 the catalog on ngterm-headless; 29 the server's scratch
  and ended rows; 30 the remote window's core; 31 the remote window in GPUI; 32 pages in a remote window;
  33 `server install`; 34 the agterm peer at v0.42.3; 35 the bundled skill; 36 desktop UI-only changes; 37 to 40 the
  mobile apps; 41 the claim. Apps: every task reaches the desktop on macOS, Linux and Windows unless it says
  otherwise; Android and iOS are named per task.
- Not ported: cookbook recipes, upstream backlog and plan notes, upstream CI and teardown scripts, release metadata,
  CLAUDE.md, README.md, llms.txt, `site/index.html`, Package.swift, upstream's two Ghostty surface patches,
  agterm-server's packaging scripts, its musl `CleanExec`, its `XDG_RUNTIME_DIR` zmx sockets and per-pane systemd
  scopes (ngterm-session supersedes them), the `--command` doc correction (#965) outside the skill copy, the moved
  `ControlServer+RestoreMode.swift`.

## Context

- Upstream: `.upstream/agterm` at v0.42.3 (`0a844715`); diff `git -C .upstream/agterm diff v0.41.1 v0.42.3`. A task
  lists the new Swift test cases of a file with `git -C .upstream/agterm diff v0.41.1 v0.42.3 -- <test file>`.
  Upstream's design notes, read for intent: `docs/plans/completed/20261107-convert-to-companion.md`,
  `20261107-markdown-local-links.md`, `20261107-remote-link-click.md`, `20261109-agterm-server-shipping.md`,
  `20261110-subagent-status-owner.md`, `20261111-agterm-session-links.md`, `20261112-remote-text-page-controls.md`,
  and `docs/plans/20261106-linux-remote-server.md`, `20261107-page-sessions.md`. The contract in one place:
  `.claude/rules/control-api.md` (its "agterm-server and remote windows" section), `site/commands.html`,
  `site/docs.html#remote-server`. Changes:
  - v0.42.0: agterm-server and remote windows (#949); page sessions (#930); Markdown local links (#931); Convert to
    Companion (#926); text wrap (#934); session links (#964); `session go --to N` (#953); badge reset on typing
    (#960); the checkmark shape (#947); forwarded clicks (#923); `pick --session`, `pick --timeout`, `ask open
    --timeout` (#949); `remoteSessionID` (#937); the Claude status row under background subagents and `--if-idle`
    (#959); idle follower cost (#955); the layout lock in every window (#925); a workspace icon click folds at once
    (#952); the new-session folder from the destination workspace (#873); `CODEX_HOME` in the installers (#905);
    the color scheme report (#961); the rename hang (#924); terminal colors in text pages (#951); fish backslashes
    (#967); csh and tcsh `!` (#970); apostrophes with combining marks (`ab692127`); one device-attributes answer
    (#968); lock release (`37cb8f55`, #970); the attached split's left pane (#938); the flat flagged row icon
    (#954).
  - v0.42.1: log pages and text controls on remote sessions, page rows on agterm-server (#978); the scratch cwd
    (#981); a session host per helper build and login (#972); hook wrapper modes (#974, `b8afbe6d`); rc file modes
    (`f52cb2bf`).
  - v0.42.2: the session host as a launchd job on macOS 27 (#987); the layout lock only while the app is active
    (#988); the Claude hook's missing payload key (`5cc32465`).
  - v0.42.3: compact toolbar split insets (#995); an unnamed server session follows its title (#993).
- Baseline after the bump (scoped to what it touched; no runtime code changed, so no ratchet id can move):
  - `cargo test -p ngterm-proto --test catalog`: 8 passed, 2 expected failures
    (`command_wire_names_equal_catalog_commands`, `every_catalog_command_parses_to_its_own_name`). Task 1.
  - `cargo test -p ngterm-ctl the_command_tree_is_the_catalogs_cli`: expected failure. Task 1.
  - `conformance/selftests`: 1617 passed, 34 skipped, 1 expected failure
    (`test_coverage.py::test_committed_catalog_passes_the_ci_gate`: 40 uncovered keys). Task 1.
  - `cargo test -p ngterm-gpui --lib ui_ids`: the 4 new ids wait in `PENDING`; Tasks 10, 31 and 36 take them off.
  - `sync-resources.sh --check`: `resources/agent-status` conflicts at `0001-linux-glue.patch`
    (`agterm-claude-status.sh:16`), Task 6; `resources/skills/ngterm` at `0001-live-group.patch` (`SKILL.md:268`),
    Task 35; `ghostty-defaults.conf` and `LICENSE` unchanged.
- Already in the Rust build: nothing whole. By construction, with tests still to add (Task 12): fish backslashes
  (`shell_dialect::fish_quoted` escapes them) and an apostrophe before a combining mark (`str::replace` is
  scalar-exact). File-mode keeping on hook rewrites is in `crates/ngterm-app/src/integrations/files.rs`; Task 6
  checks the two new cases.
- Upstream's pins: `ZMX_REV` and `GHOSTTY_REV` unchanged. zmx patches changed: `0005-detached-create.patch` (new),
  `0006-single-da-answer.patch` (new), `README.md`, `check.py`. Task 25 ports both into `ngterm-session`.
- ngterm's own features (ngterm's form wins):
  - **agterm-server is `ngterm-headless`.** This build already has its server (docs/development.md > A server without a
    display), on all three OSes, with one session server that outlives it. Taken: the catalog stream, operation
    receipts, held interactions, the remote window, detach and reattach, the installer. Left: a second server
    binary, zmx daemons, systemd scopes per pane, the Linux-only limit. Upstream strings that name `agterm-server`
    name `ngterm-headless` here, each with a row in rust-build.md's renamed-strings table and an entry in the Swift
    adapter's vocabulary.
  - **Attach Remote stays.** A viewer's New Session and End Session on a host, and attached rows that come back
    after a relaunch, keep working beside the remote window; a remote window's rows are not saved as attached rows.
  - **Loopback pages.** `remote/page_tunnel.rs` already opens a presenter's ssh forward; upstream's `page.forward`
    kind is listed toward an agterm origin only (Task 34), and Task 32 reuses the tunnel.
  - **Pages survive.** `model/page_snapshot.rs` keeps every page; page sessions (Task 19) save through it, not
    through a second format, and parked pages keep their rules.
  - **The session server.** ngterm-session's update handoff moves sessions to a new build, so upstream's "a host per
    helper build" has nothing to fix on Linux and Windows; Task 25 says what macOS needs.
  - **Identity.** Session links are `ngterm://session/<id>`; `agterm://session/<id>` is read only in a pane attached
    from an agterm origin (Task 11). The `live` names stand for upstream's `zmx`.
  - **Lazy restore** and the **Settings popup** are untouched.

## Approach

- Each task follows AGENTS.md > Porting a command group: upstream's Swift sources and tests hold the exact strings.
  Read a task's upstream commits with `git -C .upstream/agterm show <sha>`. Before it starts, a task confirms its
  "absent" claim with a search of its own names in `crates/` and `mobile/`.
- Every task lands alone with main working. A part a user cannot use yet stays behind an inactive path: a command
  registered with the dispatcher's not-implemented refusal, a model with no caller, a stream kind no hello lists.
- Where upstream overlaps an ngterm feature or deliberate deviation (`docs/development.md`,
  `conformance/docs/rust-build.md`), ngterm's form wins: a task adapts upstream's change onto it or leaves the
  contradicting part out, and never reworks the ngterm feature toward upstream.
- Behavior that upstream builds on a macOS API gets a form for macOS, Linux and Windows (AGENTS.md > Platforms), or
  a recorded reason why an OS has no counterpart. Cmd in upstream's chords is Ctrl (or Super where the keymap says
  so) on Linux and Windows. Upstream strings that name "another Mac" or "Finder" stay verbatim.
- Each behavior also reaches the Android and iOS apps where a remote viewer would show or drive it, through
  `ngterm-mobile` and each app's screens (UI work through the `frontend-design` skill, inside that app's
  `DESIGN.md`). A feature task names the apps it reaches; mobile code goes into Tasks 37 to 40. A behavior the apps
  get with no mobile code carries a `ngterm-mobile` test item in its feature task.
- New conformance cases carry `since("0.42.0")`, `since("0.42.1")`, `since("0.42.2")` or `since("0.42.3")` by the
  release that brought the behavior (Context > Changes). They stay skipped on Rust until Task 41 raises the claim:
  do not count a skipped case as verification or add it to the ratchet before then.
- A listed (ratcheted) test never gets a whole-test `since` and never gains assertions on behavior a later task
  builds: a new case goes into a new test, or into a listed parametrized test as `pytest.param(...,
  marks=pytest.mark.since(...))`. An existing ratcheted test whose upstream string changes moves in the same commit
  as the Rust string.
- A new presentation kind is added to this origin's supported kinds only in the task that implements both ends; a
  viewer that does not list it gets the older behavior the task names.
- The bundled skill (`resources/skills/ngterm/`) is regenerated whole in Task 35; feature tasks do not edit it.
- Checks per task follow AGENTS.md > Choosing checks. A new conformance test must also pass on the macOS reference
  (`reference-macos.yml`).
- Machines (`.claude/skills/agterm-upstream-sync/references/machines.md`): Windows work runs in the Windows VM,
  macOS and iOS work on the Mac over ssh, Android work on the emulator; the VM and the emulator start on demand and
  stop when the task's work on them is done.

## Implementation Steps

### Task 1: Register the new wire and CLI shapes and cover the catalog

**Depends on:** the bump commit (lands with it). **Size:** L.
**Upstream:** `agtermCore/Sources/agtermCore/{ControlProtocol,ControlProjection,ControlPayloads,ControlModes,ControlDispatcher,ControlDispatcher+Detach,ControlDispatcherOptions}.swift`;
`agtermCore/Sources/agtermctlKit/{CatalogCommands,DetachCommands,ServerCommands,Session+Companions,Session+OverlayWrap,SessionCommands,WindowCommands,WorkspaceCommands}.swift`;
`site/commands.html`. **Swift tests:** `ControlProtocolTests.swift`, `ControlWindowProtocolTests.swift`,
`ControlDispatcher+DetachTests.swift`, `CatalogCommandsTests.swift`, `DetachCommandsTests.swift`,
`ServerCommandsTests.swift`, `WindowCommandsTests.swift`, `OverlayCommandsTests.swift`,
`SessionCompanionsCommandsTests.swift`.
**Rust side:** modify `crates/ngterm-proto/src/{command,request,result,tree}.rs`, `crates/ngterm-core/src/{dispatch,projection}.rs`,
`crates/ngterm-ctl/src/{main,session,session_overlay,companions,window,workspace,usage}.rs`; create
`crates/ngterm-ctl/src/{catalog,server}.rs`; conformance tests under `conformance/tests/{protocol,cli,ui}`.
**Apps:** desktop. Android and iOS: `ngterm-mobile` builds and its tests stay green with the new optional fields.
**ngterm's own:** none.
**Conformance:** the 40 uncovered keys: commands `session.page`, `session.companion.convert`,
`session.overlay.wrap`, `window.remote.open`, `window.remote.dismiss`, `session.detach`, `session.reattach`,
`workspace.detach`, `workspace.reattach`, `catalog.subscribe`, `catalog.op`; fields `result.headless`,
`result.versions`, `result.receipt`, `tree.remote`, `window.remote`, `tree.session.scratchCwd`,
`tree.session.liveHostStale`, `tree.session.splitLiveHostStale`, `tree.session.pick`, `tree.session.ended`,
`tree.session.pending`, `tree.session.scratchBacked`, `tree.session.interactions`, `tree.session.remoteSessionID`;
CLI `session convert`, `session overlay wrap`, `session overlay open` variant `--page`, `window remote open|dismiss`,
`session detach|reattach`, `workspace detach|reattach`, `catalog subscribe|op`, `server install`; ui
`htmlOverlay.loading`, `settings-badge-reset`, `remote-connection`, `remote-unknown`.

- [x] Register the 11 commands with their request args and the dispatcher's established not-implemented refusal
  until their tasks land; add the 14 fields as optional, omitted while unset, in upstream's key order; update the
  key-order tests.
- [x] Add the 11 CLI leaves with upstream's help and parse-time refusals (`--page cannot be combined with
  --companion`, `--op-id requires --companion or --page`, `--navigation requires --html, --url or --markdown`, `mode
  must be on, off or toggle`); no success stubs. `server install` is local-only and refuses until Task 33.
- [x] Write the reference conformance tests that cover the keys, `since`-marked, each asserting its named behavior
  or field (the feature tasks give the cases). A key the macOS reference can only refuse (`catalog.*`, an `opID`:
  `operation receipts are served only by agterm-server`) is covered by that refusal; a key no suite host can reach
  is exempted with its reason, as the catalog allows.
- [x] Both catalog cargo tests, the command-tree test, the host-free selftests and the coverage gate pass; existing
  ratchet ids unchanged.

What Task 1 left for the feature tasks:

- Nine commands answer `NAME is not supported by this host` (`session.page`, `session.companion.convert`,
  `session.overlay.wrap`, `window.remote.open|dismiss`, `session|workspace.detach|reattach`); the task that builds
  one replaces its arm and makes the `since`-marked tests of it pass on Rust.
- `conformance/tests/cli/test_registered_ahead_cli.py` holds these interim answers on the Rust build (in the ratchet;
  it skips on a build at 0.42.0, the reference included): the help of the new leaves, the nine refusals by name,
  `server install`'s refusal and the app's refusal of a markdown page's `--navigation`. The task that builds one
  removes its case there and its id from `conformance/rust-passing.txt` in the same commit (Tasks 17 and 33 theirs);
  Task 41 removes the module and its remaining ids.
- `catalog.subscribe`, `catalog.op` and any `opID` already answer upstream's refusals of an app with windows, in
  ngterm's words (`… served only by ngterm-headless`, rust-build.md > Other strings of the build's own): the suite's
  vocabulary selftest holds every translated text to a Rust literal, so the texts came with their tests.
  `ngterm-headless` shares the dispatcher and answers the same until Tasks 27 and 28 give the core its headless form.
- The `ngtermctl` leaves send upstream's requests. `session overlay open` already takes `--page`, `--op-id`,
  `--epoch` and `--expect-panes`, and a markdown page's `--navigation` reaches the app, which still refuses it
  (Task 17). The other commands' `--op-id` (upstream's `OperationOptions`), `--expect-scratch`, `pick --session`,
  `pick --timeout` and `ask open --timeout` are Tasks 26 and 28.
- Exempted in `catalog.yaml` until a suite host can reach them: `result.headless`, `result.versions`,
  `tree.session.ended`, `tree.session.interactions` (Task 27 covers them against `ngterm-headless` and removes the
  exemptions); `tree.remote`, `window.remote`, `tree.session.pending`, `tree.session.scratchBacked`,
  `tree.session.remoteSessionID`, `ui:remote-connection`, `ui:remote-unknown`, `ui:htmlOverlay.loading` (ssh);
  `tree.session.liveHostStale`, `splitLiveHostStale` (Task 25).
- The reference tests of this task were written from upstream's sources and have not run on the macOS reference
  yet: the first `reference-macos.yml` run after this lands checks them, and the feature task of a test that fails
  there corrects it.
- Upstream reworded three app-side refusals of `session.overlay.open` (`--navigation requires --html, --url or
  --markdown`, and the `--markdown` option list without `--navigation`); the ratcheted tests that assert the old
  texts move with the Rust strings in Task 17.

### Task 2: Renderer resources to v0.42.3

**Depends on:** none. **Size:** S.
**Upstream:** `agterm/Resources/markdown/{render.js,text.js,text.css}`.
**Rust side:** `resources/markdown/` through `scripts/sync-markdown-resources.py`; `crates/ngterm-core/src/html/{markdown_page,text_page}.rs` only where a page's template must keep rendering.
**Apps:** desktop. The phones show no pages.
**ngterm's own:** the script's name adaptations stay.
**Conformance:** existing text, Markdown and diff page ids stay green.

- [x] Refresh the resources with the script; `--check` passes.
- [ ] Existing pages render as before with the new scripts while wrap, link grants and log colors have no caller
  (Tasks 15 to 17): engine tests on WPE, and the page smoke on WKWebView and WebView2.

### Task 3: `session go --to N` (#953)

**Depends on:** none. **Size:** S.
**Upstream:** `487d643b`, `467df3d2`; `AppStore.swift` (navigation), `ControlDispatcher.swift`; tests
`AppStoreNavigationTests.swift`, `ControlDispatcherTests.swift`, `CommandsTests.swift`,
`ControlSessionGoUITests.swift`.
**Rust side:** modify `crates/ngterm-core/src/{model/navigation.rs,commands/session.rs}`, `crates/ngterm-ctl/src/session.rs`.
**Apps:** desktop only: the phones have no sidebar order command.
**Conformance:** new cases in `tests/protocol/test_session.py` and `tests/cli`.

- [ ] `N` is a 1-based place among the navigable sessions, a collapsed workspace's included; past the end is the
  navigation error, never a wrap; the refusal reads `session.go requires --to
  next|prev|first|last|next-attention|prev-attention|N`.
- [ ] Rust unit tests with the Swift cases; the ratcheted test that holds the old refusal string moves with it.

### Task 4: `session status --if-idle` (#959)

**Depends on:** none. **Size:** S.
**Upstream:** `56d8c77c` (`AppStore+Status.swift`, `ControlDispatcher.swift`, `ControlModes.swift`,
`SessionMetadataCommands.swift`); tests `AppStoreStatusTests.swift`,
`ControlDispatcherSessionMetadataTests.swift`, `ControlProtocolTests.swift`, `ControlServerStatusSoundTests.swift`,
`PaneAwareStatusUITests.swift`.
**Rust side:** modify `crates/ngterm-proto/src/request.rs`, `crates/ngterm-core/src/{model/status.rs,commands/metadata.rs}`, `crates/ngterm-ctl/src/session_meta.rs`.
**Apps:** desktop; Android and iOS get the status over the stream with no code: add a `ngterm-mobile` test that a
skipped write sends no status frame.
**Conformance:** new cases in `tests/protocol/test_session.py`.

- [x] `ifIdle` makes a write on a non-idle row a success with no change, stamp, event or sound; it is checked
  before pane precedence, so it never answers a refusal.
- [x] Rust unit tests with the Swift cases; conformance cases.

### Task 5: The checkmark status shape (#947)

**Depends on:** none. **Size:** S.
**Upstream:** `c41b7921` (`AgentStatus.swift`, `PresentationHub.swift` `statusCheckmarkKind` and `acceptedShapes`,
`StatusGlyph.swift`, `SettingsView.swift`); tests `AgentStatusTests.swift`, `PresentationHubTests.swift`,
`ControlDispatcherPickTests.swift`, `CommandsTests.swift`, `ControlSidebarStatusUITests.swift`, `SettingsUITests.swift`.
**Rust side:** modify `crates/ngterm-core/src/{model/status.rs,remote/hub.rs}`, `crates/ngterm-ctl/src/session_meta.rs`, `crates/ngterm-gpui/src/{status_glyph.rs,settings/agent_status.rs}`.
**Apps:** desktop; Android and iOS in Task 37.
**Conformance:** the shape-list cases in `tests/protocol/test_session.py` and `test_ask_pick.py`.

- [x] `checkmark` in the model, `session status --shape`, pick items and Settings; drawn as an outline traced after
  the SF Symbol on Linux and Windows.
- [x] The origin lists `status.checkmark`; a viewer that lists `status.shapes` without it gets the expanded set and
  draws its own Settings shape for a checkmark row.
- [x] Rust unit tests with the Swift cases; the ratcheted shape-list strings move in the same commit.

### Task 6: The agent-status package and the installers (#959, #905, #974, `f52cb2bf`, `5cc32465`)

**Depends on:** Task 4. **Size:** M.
**Upstream:** `agterm/Resources/agent-status/` at v0.42.3 (`agterm-claude-status.sh` payload reading, `agent_id` adds
`--if-idle`, a Stop with running `background_tasks` posts `active --blink`; `agterm-codex-status.sh`);
`AgentHooksFiles.swift`, `AgentHooksInstall.swift`, `SkillInstall.swift`; tests `ClaudeStatusHookTests.swift`,
`CodexStatusHookTests.swift`, `AgentHooksFilesTests.swift`, `AgentHooksInstallTests.swift`,
`CodexInstallationTests.swift`, `SkillInstallTests.swift`, `AgentStatusWrapperTests.swift`.
**Rust side:** `resources/agent-status/` through `sync-resources.sh`; refresh `build/patches/agent-status/0001-linux-glue.patch`
and any later patch that no longer applies; modify `crates/ngterm-app/src/integrations/{mod,hooks,hooks_config,files,skill,opencode}.rs`.
**Apps:** desktop and `ngterm-headless` on all three OSes (the Windows hook forms of patch 0004 get the same payload
rules); the phones see only the resulting status.
**ngterm's own:** the restore adapter, agent mail, agent transcript and ngswap patches stay as they are.
**Conformance:** none new; the package's own tests under `crates/ngterm-app/src/integrations`.

- [ ] Rebuild the package from upstream's tree plus the refreshed patches; `sync-resources.sh --check v0.42.3
  v0.42.3` reports `unchanged`; bump the release in `integrations/mod.rs`'s header.
- [ ] The payload reading works without `plutil`: the Linux glue's python3 or jq, PowerShell on Windows; a missing
  key reads as empty; an unreadable payload posts argv.
- [ ] Codex's hooks and skill go to a nonempty `CODEX_HOME`, a relative one resolved against the working directory,
  else `~/.codex`; one that does not exist is reported and skipped. Applies to the Help installers and to
  `ngterm-headless`'s agent setup.
- [ ] A baked wrapper keeps its executable bit; an rc file keeps its mode when the integration line is appended.
- [ ] Rust tests with the Swift cases, the two mode cases included.

### Task 7: The scratch shell's own cwd (#981)

**Depends on:** Task 1. **Size:** S.
**Upstream:** `82910bb8` (`Session+PaneDirectory.swift`, `AppStore+Panes.swift`, `ControlProjection.swift`,
`CustomCommandRunner.swift`, `GhosttySurfaceView+Panes.swift`); tests `AppStorePaneTests.swift`,
`AppStoreTreeProjectionTests.swift`, `SessionTests.swift`, `CustomCommandRunnerTests.swift`,
`GhosttySurfaceViewTitleTests.swift`, `SurfaceFactorySeedTests.swift`.
**Rust side:** modify `crates/ngterm-core/src/{model/session.rs,model/panes.rs,projection.rs,keymap/command.rs,link_policy.rs}`, `crates/ngterm-gpui/src/{pane.rs,keymap_dispatch.rs}`.
**Apps:** desktop only: the phones attach no scratch.
**Conformance:** `field:tree.session.scratchCwd` cases in `tests/protocol/test_tree.py`.

- [ ] `scratchCwd` reports the scratch's last reported or start directory while its surface exists, shown or
  hidden; closing the scratch clears it; its PWD never reaches the row's cwd or title.
- [ ] A custom command run from the scratch gets that directory in the session PWD token; relative links in a local
  scratch resolve against it.
- [ ] Rust unit tests with the Swift cases; conformance cases.

### Task 8: `remoteSessionID` (#937)

**Depends on:** Task 1. **Size:** S.
**Upstream:** `ebc66405`, `0ee9ed1f` (`Session.swift`, `AppStore+Events.swift`, `ControlEvents.swift`,
`ControlProjection.swift`, `EventCommands.swift`); tests `AppStoreEventTests.swift`,
`ControlEventProtocolTests.swift`, `ControlProtocolTests.swift`, `EventCommandsTests.swift`,
`ControlServerZmxTests.swift`, `ControlServerRemoteCompanionsTests.swift`.
**Rust side:** modify `crates/ngterm-proto/src/{tree,events}.rs`, `crates/ngterm-core/src/{model/session.rs,model/store.rs,remote/viewer.rs,projection.rs}`, `crates/ngterm-ctl/src/events.rs`.
**Apps:** desktop only: the phones emit no events.
**ngterm's own:** attached rows already come back after a relaunch; the field is for hooks, not for that.
**Conformance:** new cases in `tests/protocol/test_events.py` and the loopback attach tests.

- [ ] The node and `remote.opened`/`remote.closed` carry the origin's session id, set at construction; the human
  event line reads `HH:MM:SS remote.opened NAME host=HOST remote-session-id=ID`.
- [ ] Rust unit tests with the Swift cases; conformance cases.

### Task 9: The new-session folder from the destination workspace (#873)

**Depends on:** none. **Size:** S.
**Upstream:** `f2d84ca2`, `b6364256` (`AppStore+NewSessionPlacement.swift` `newSessionDirectorySource`); tests
`AppStoreOrganizationTests.swift`, `AppActionsTests.swift`.
**Rust side:** modify `crates/ngterm-core/src/{model/store.rs,model/recency.rs,app_actions.rs}`, `crates/ngterm-gpui/src/sidebar/menu.rs`.
**Apps:** desktop; the phones' New Session names no workspace row.

- [ ] With "Current session's directory", a new session from another workspace's row takes its folder from the
  selection inside that workspace, else that workspace's most recent session, else its first; only an empty
  workspace falls back to the selection.
- [ ] Rust unit tests with the Swift cases.

### Task 10: The unseen badge clears on typing (#960)

**Depends on:** Task 1. **Size:** M.
**Upstream:** `6c6faf1c` (`AgentStatus.swift` `StatusReset`, `AppSettings.swift` `badgeReset`,
`GhosttySurfaceView+Input.swift`, `SettingsView.swift`, `QuickTerminal.swift`); tests `AgentStatusTests.swift`,
`AppSettingsTests.swift`, `BadgeResetUITests.swift`, `SettingsUITests.swift`.
**Rust side:** modify `crates/ngterm-core/src/{persist/settings.rs,settings_model.rs,model/status.rs,commands/typing.rs}`, `crates/ngterm-gpui/src/{terminal.rs,settings/notifications.rs,ui_ids.rs}`.
**Apps:** desktop; Android and iOS in Task 40.
**Conformance:** `ui:settings-badge-reset` in `tests/ui/test_settings.py`; a protocol case that `session type` never
clears the badge.

- [ ] Settings ▸ Notifications ▸ Badge reset: On first key (default), On Enter, Disabled; an unknown saved value
  reads as first key. Typing in a session clears its unseen badge by that rule; `session type` never does.
- [ ] Take `settings-badge-reset` off `PENDING`.
- [ ] Rust unit tests with the Swift cases; the ui test, green on the reference first.

### Task 11: Session links (#964, #970)

**Depends on:** none. **Size:** M.
**Upstream:** `9c072af3`, `56c22500`, `ba52ae80` (`LinkPolicy.swift` `sessionLink`,
`WindowLibrary+SessionLink.swift`, `AppActions+Focus.swift`, `LinkOpener.swift`); tests `LinkPolicyTests.swift`,
`WindowLibrarySessionLinkTests.swift`, `AppActionsFocusTests.swift`, `GhosttySurfaceViewInputTests.swift`.
**Rust side:** modify `crates/ngterm-core/src/{link_policy.rs,model/window_library.rs,app_actions.rs}`, `crates/ngterm-gpui/src/terminal.rs`.
**Apps:** desktop; Android and iOS in Task 40.
**ngterm's own:** the scheme is `ngterm://session/<id>`; `agterm://session/<id>` is read only from a pane attached
from an agterm origin, where the agents print upstream's.
**Conformance:** none (a gesture with no control command); unit tests hold it.

- [ ] Exactly the scheme prefix (any case) plus a 36-character hyphenated UUID, checked on the raw string; a
  Ctrl-click (⌘ on macOS) selects that session, with Shift too; the reveal chord does nothing.
- [ ] The id's namespace comes from the clicked pane: local panes and scratches use local ids, a pane attached from
  another machine uses that origin's host and endpoint; no binding, or an ambiguous match, selects nothing. Unknown
  or closed rows, zoom, the dashboard and a pending picker refuse.
- [ ] Rust unit tests with the Swift cases.

### Task 12: Re-run quoting (#967, #970, `ab692127`)

**Depends on:** none. **Size:** S.
**Upstream:** `c6e3cbe8`, `ab692127`, `ba52ae80` (`CommandRestore.swift` `shouldRestore(receivingShell:)`,
`accountShellQuoted`, `RemoteSession.swift`, `LaunchSeed.swift`); tests `CommandRestoreTests.swift`,
`LaunchSeedTests.swift`, `SurfaceFactorySeedTests.swift`.
**Rust side:** modify `crates/ngterm-core/src/{command_restore.rs,shell_dialect.rs}`, `crates/ngterm-app/src/restore_launch.rs`.
**Apps:** desktop; the mobile core shares the remote quoting: add its test for the apostrophe case.
**ngterm's own:** the dialect layer (`shell_dialect`) already quotes fish by its own rules; only tests are added
there.

- [ ] A captured command with `!` is not typed into csh or tcsh: such a pane starts a plain shell.
- [ ] Tests: fish gets doubled and trailing backslashes with the same bytes; an apostrophe followed by a combining
  mark or emoji modifier is escaped in Live attach and ssh remote lines.

### Task 13: Locks release before close (`37cb8f55`, #970)

**Depends on:** none. **Size:** S.
**Upstream:** `StateDirectoryLock.swift`, `ControlServer.swift` (socket lock); tests `StateDirectoryLockTests.swift`,
`ControlServerTests.swift`.
**Rust side:** modify `crates/ngterm-core/src/state_lock.rs`, `crates/ngterm-server/src/lib.rs`.
**Apps:** desktop and `ngterm-headless`. Windows: handles are not inherited, so the task records that nothing is
needed there and adds the Unix test only.

- [ ] Dropping the state directory lock and the control socket lock unlocks before closing, so a child that still
  holds a copy of the descriptor until its exec does not keep the next instance out.
- [ ] A test that fails on the unfixed code: a held duplicate descriptor, then a second claim succeeds.

### Task 14: The color scheme report follows the system (#961)

**Depends on:** none. **Size:** S.
**Upstream:** `316b2e24` (`GhosttyApp.setColorScheme`, `SettingsModel.appearanceChanged`); tests
`AppearanceFlipUITests.swift`.
**Rust side:** modify `crates/ngterm-gpui/src/{appearance.rs,terminal.rs}`, `crates/ngterm-term` where the pane
answers `CSI ? 996 n` and mode 2031.
**Apps:** desktop; Android and iOS: the `Pane` API takes the phone's appearance; Task 40 wires it.
**Conformance:** a behavior case if the suite can flip the appearance; else unit tests on the pane.

- [ ] The query answers the system appearance with any theme, one theme for both appearances included; with theme
  sync off, a program in mode 2031 still gets a report on an appearance flip.
- [ ] Tests on a real PTY pane for both cases.

### Task 15: Text, log and diff pages wrap long lines (#934)

**Depends on:** Tasks 1, 2. **Size:** M.
**Upstream:** `6c082ecd` (`HtmlOverlay.swift` `textWrap`, `TextPage.swift`, `HtmlBridge.swift`,
`ControlDispatcher+Overlay.swift`, `ControlServer+TextPage.swift`, `TextPageBridge.swift`, `Session+OverlayWrap.swift`);
tests `HtmlOverlayTests.swift`, `TextPageTests.swift`, `HtmlBridgeTests.swift`, `ControlDispatcherOverlayTests.swift`,
`ControlServerTextPageTests.swift`, `TextPageRenderingTests.swift`, `OverlayCommandsTests.swift`.
**Rust side:** modify `crates/ngterm-core/src/{model/html_overlay.rs,html/text_page.rs,html/bridge.rs,commands/overlay_text.rs,projection.rs}`, `crates/ngterm-ctl/src/overlay_text.rs`, `crates/ngterm-gpui/src/html_overlay`.
**Apps:** desktop; the phones show no pages.
**Conformance:** `session.overlay.wrap`, `cli:session overlay wrap` in `tests/protocol/test_text_pages.py`.

- [ ] `session overlay wrap on|off|toggle`, the bar's Wrap button and `w`; per page, off at open, never saved;
  answers `wrapped` or `unwrapped`; read back `htmlOverlays[].text.wrap`, omitted while unwrapped; refusals are the
  text search's; `invalid overlay wrap mode: …`.
- [ ] The bridge records the page's toggle in the model and the model pushes every change back.
- [ ] Rust unit tests with the Swift cases; engine tests on WPE, WKWebView and WebView2; conformance cases.

### Task 16: Terminal colors in text and log pages (#951)

**Depends on:** Task 2. **Size:** S.
**Upstream:** `3479714e`, `b7f93fe0` (`TerminalText.swift`, `TextPage.swift`, `text.js`); tests
`TerminalTextTests.swift`, `TextPageTests.swift`, `TextPageRenderingTests.swift`.
**Rust side:** modify `crates/ngterm-core/src/html/{text_page,log_tail}.rs`; create a terminal-text module beside them.
**Apps:** desktop.

- [ ] A text or log page renders a log's SGR colors and drops other escape sequences: DCS and APC strings, private
  CSI.
- [ ] Rust unit tests with the Swift cases; an engine rendering test.

### Task 17: Markdown pages follow local links (#931)

**Depends on:** Tasks 1, 2. **Size:** L.
**Upstream:** `7c5b4b9e`, `a6c837ce`, `5ca757e2`, `e3fb2d23`, `d23611fb` (`MarkdownPage.swift` `MarkdownLinkGrant`,
`linkedDocument`; `HtmlOverlay.swift` `markdownLinks`; `MarkdownPageSource.swift`, `HtmlPageSchemeHandler.swift`,
`ControlDispatcherOptions.swift`); tests `MarkdownPageTests.swift`, `HtmlOverlayTests.swift`,
`MarkdownPageRenderingTests.swift`, `ControlServerLinkOverlayTests.swift`, `ControlDispatcherOverlayTests.swift`,
`OverlayCommandsTests.swift`, `ControlHtmlOverlayUITests.swift`.
**Rust side:** modify `crates/ngterm-core/src/{html/markdown_page.rs,html/navigation.rs,model/html_overlay.rs,commands/overlay_page.rs,link_policy.rs}`, `crates/ngterm-ctl/src/overlay_page.rs`, `crates/ngterm-app/src/documents.rs`, `crates/ngterm-gpui/src/html_overlay`, `crates/ngterm-wpe`.
**Apps:** desktop: the page loads through the page scheme on each engine (WPE's URI scheme, WKWebView's scheme
handler, WebView2's resource filter). The phones show no pages.
**Conformance:** new cases in `tests/protocol/test_document_pages.py`.

- [ ] `--markdown FILE --navigation`, which a clicked `.md` always asks for: relative links to `.md`/`.markdown`
  files in the file's folder or below open in the same page, with back and forward; links that leave the folder,
  name another kind of file or resolve through a symlink to one stay inert; a file directly in `/` or the home
  folder gets no grant and no navigation. The refusal reads `session.overlay.open: --navigation requires --html,
  --url or --markdown`.
- [ ] The policy allows only the page's own host with a Markdown path inside the root; every read is judged again;
  a link that fails after the first commit keeps the shown page loaded.
- [ ] Read back `markdown`, `navigation`, `page`, `canGoBack`, `canGoForward`.
- [ ] Rust unit tests with the Swift cases; engine tests on the three engines; conformance cases.

### Task 18: Convert to Companion (#926)

**Depends on:** Task 1. **Size:** M.
**Upstream:** `9b010148` (`AppStore+Companions.swift` `convertToCompanion`, `ConvertRefusal`,
`ControlDispatcher+Companions.swift`, `BuiltinAction.swift`, `PaletteCatalog.swift`, `AppActions+Companions.swift`,
`WorkspaceSidebar+ContextMenu.swift`, `agtermApp+Menus.swift`, `Session+Companions.swift`); tests
`AppStoreCompanionsTests.swift`, `ControlDispatcherCompanionsTests.swift`, `BuiltinActionTests.swift`,
`PaletteCatalogTests.swift`, `WindowLibraryRecentClosedTests.swift`, `AppActionsCompanionsTests.swift`,
`ControlServerCompanionsTests.swift`, `SessionCompanionsCommandsTests.swift`, `ControlCompanionsUITests.swift`.
**Rust side:** modify `crates/ngterm-core/src/{model/companions.rs,commands/companions.rs,keymap/builtin.rs,palette_catalog.rs,app_actions.rs}`, `crates/ngterm-ctl/src/companions.rs`, `crates/ngterm-gpui/src/{sidebar/menu.rs,app_menu.rs,keymap_actions.rs}`.
**Apps:** desktop; Android and iOS: the listing shows the new parent with no code: add a `ngterm-mobile` test.
**Conformance:** `session.companion.convert`, `cli:session convert` in `tests/protocol/test_companions.py`.

- [ ] `session convert --companion`: a fresh login shell takes the target's slot and focused directory and the
  target moves under it, still running, with its companions, keeping id, name, flag, status and panes, and ends
  selected. Returns `id` and `parent`. Refusals: `already a companion`; a row attached from another machine is
  refused, never forwarded.
- [ ] The Session menu, the row menu, a palette row and the keyless `convert_companion` keymap action.
- [ ] Rust unit tests with the Swift cases; conformance cases; a ui test for the row menu entry.

### Task 19: Page sessions (#930)

**Depends on:** Task 1. **Size:** L.
**Upstream:** `71cab914` (`AppStore+Companions.swift` `addPageSession`, `PageSessionRefusal.swift`,
`Snapshot.swift` `PageSnapshot`, `AppStore+Snapshot.swift`, `AppStore+RecentClosed.swift`, `LaunchSpawnPlan.swift`,
`ZmxLifecycle.swift`, `ControlServer+CompanionPage.swift`, `HtmlOverlayRegistry.closePageSession`); tests
`AppStoreCompanionsTests.swift`, `ControlDispatcherCompanionsTests.swift`, `SnapshotRoundTripTests.swift`,
`RecentClosedTests.swift`, `LaunchSpawnPlanTests.swift`, `WindowLibraryClaimsTests.swift`, `ZmxLifecycleTests.swift`,
`ControlServerCompanionPageTests.swift`, `HtmlOverlayRegistryTests.swift`, `OverlayCommandsTests.swift`.
**Rust side:** modify `crates/ngterm-core/src/{model/companions.rs,model/page_snapshot.rs,model/snapshot.rs,model/recent_closed.rs,commands/companion_page.rs,commands/companions.rs,live/inventory.rs,restore}`, `crates/ngterm-ctl/src/{session_overlay,overlay_page}.rs`, `crates/ngterm-gpui/src/{html_overlay,sidebar}`.
**Apps:** desktop; Android and iOS in Task 39.
**ngterm's own:** `pages-survive` already saves pages; a page row saves through `page_snapshot.rs`. Upstream's "page
companions are saved" replaces this build's "page companions are still not kept" note.
**Conformance:** `session.page`, the `--page` variant in `tests/protocol/test_companions.py` and `test_remote_pages.py`.

- [ ] `session overlay open --html FILE|--url URL --page`: a top-level page row placed after the target's group,
  holding companions like any session; returns `id` and `pageID`; read back `content: "page"` with no `parent`; the
  too-old text `the running app is too old for page sessions`.
- [ ] Terminal commands on a page row answer `<cmd>: not available on a page session`; the page's own
  `window.close()` closes the row like any session and waits while a picker is pending in its window.
- [ ] A local page row, top-level or companion, comes back with its companions in every restore mode and from
  Recent Closed; a saved page that does not decode restores as a failed page row; a row attached from another
  machine is never saved.
- [ ] Rust unit tests with the Swift cases; conformance cases.

### Task 20: Page sessions across machines (#930)

**Depends on:** Task 19. **Size:** M.
**Upstream:** `71cab914` (`AppStore+ReplicaPage.swift` `addReplicaPageRoot`, `RemoteTree.swift`,
`ControlServer+RemoteGroup.swift`, `ControlServer+RemoteCompanions.swift`, `ControlServer+PageServe.swift`,
`PageAttachmentBook.swift`); tests `AppStoreReplicaPageTests.swift`, `RemoteTreeTests.swift`,
`ControlServerRemoteGroupTests.swift`, `ControlServerRemoteCompanionsTests.swift`, `RemoteAttachTests.swift`.
**Rust side:** modify `crates/ngterm-core/src/remote/{tree,attach,group_attach,companion_page,replica_page,companions}.rs`, `crates/ngterm-core/src/commands/live`.
**Apps:** desktop; Android and iOS in Task 39.
**Conformance:** loopback attach cases in `tests/protocol/test_remote_pages.py`.

- [ ] `live tree` lists a top-level page session always and a page companion when its parent is listed, with no
  panes; `live attach` and the group attach open a page root as a top-level replica row in the destination's
  current workspace, its companions under it with `--companions`; a row attached from another machine is never
  listed.
- [ ] New Companion on a page root's replica row makes a companion under that root on the origin; a channel ending
  while the viewer row is soft-closed finalizes the row and the root's parked companions.
- [ ] Rust unit tests with the Swift cases; conformance cases.

### Task 21: Forwarded clicks from an attached session (#923)

**Depends on:** none. **Size:** L.
**Upstream:** `9a6f8e85` (`AppStore+ForwardedClick.swift`, `AppStore+LinkClickRequest.swift`, `LinkPolicy.swift`
`.forwardRemote`, `RemoteLinkRefusal.swift`, `PresentationFrames.swift` `link.click`/`link.refused`,
`ControlServer+ForwardedClick.swift`, `ControlServer+RemoteLinkClick.swift`); tests
`AppStoreForwardedClickTests.swift`, `AppStoreLinkClickRequestTests.swift`, `LinkPolicyTests.swift`,
`RemoteLinkRefusalTests.swift`, `PresentationFramesTests.swift`, `PresentationHubTests.swift`,
`ControlServerForwardedClickTests.swift`, `ControlServerRemoteLinkClickTests.swift`.
**Rust side:** modify `crates/ngterm-core/src/{link_policy.rs,remote/frames.rs,remote/hub.rs,remote/viewer.rs,remote/page.rs}`, `crates/ngterm-app/src/remote.rs`, `crates/ngterm-gpui/src/{terminal.rs,remote.rs}`.
**Apps:** desktop. Android and iOS show no pages, so they do not list `link.click`; recorded in rust-build.md.
**Conformance:** a loopback case in `tests/protocol/test_remote_pages.py`.

- [ ] A click on a bare, relative or `file://` path in a pane attached from another machine travels as `link.click`
  while this machine presents the session and the origin agreed the kind, with the origin's pane identity; it
  reveals nothing here.
- [ ] The origin resolves it with that pane's cwd and opens only page routes for the presenter; a refusal returns
  `link.refused` to that presenter generation only, worded on the presenting machine (`RemoteLinkRefusal`'s
  strings), shown as a HUD that hides, or the system alert when a HUD, cover, zoom or own hidden page is in the way.
- [ ] Both frames decode leniently: an unknown mode or reason never ends the stream.
- [ ] Rust unit tests with the Swift cases; the conformance case.

### Task 22: Markdown links on pages shown for another machine (#931)

**Depends on:** Tasks 17, 21. **Size:** M.
**Upstream:** `6ee8aec6` (`RemotePageResources.swift`, `AppStore+RemotePage.swift`, `PresentationHub.pageLinksKind`,
`HtmlReplicaLink.swift`, `ControlServer+PageServe.swift`); tests `RemotePageResourcesTests.swift`,
`AppStoreRemotePageTests.swift`, `AppStoreReplicaPageTests.swift`, `ControlServerPageServeTests.swift`.
**Rust side:** modify `crates/ngterm-core/src/remote/{page,page_serve,page_channel,replica_page,hub}.rs`, `crates/ngterm-app/src/page_channel.rs`.
**Apps:** desktop.

- [ ] The grant rides the page request only to a presenter listing `page.links`; any other gets the page without
  links, in the request and in the record the claim serves from.
- [ ] The origin serves each linked Markdown file under the same rules as on its own screen, and pictures anywhere
  in the grant's root.
- [ ] Rust unit tests with the Swift cases; a loopback conformance case.

### Task 23: Log pages and text-page controls on remote sessions (#978)

**Depends on:** Task 15. **Size:** L.
**Upstream:** `696edcf0` (`AppStore+RemoteTextPage.swift`, `PageServeFrames.swift`, `PageServeStream.swift`,
`PresentationFrames.swift` `pageCommand`/`pageResult`, `PresentationHub.pageTextKind`, `TextPageBridge.swift`,
`LogFollower.swift`, `ControlServer+TextPage.swift`, `RemotePageChannel.swift`); design
`20261112-remote-text-page-controls.md`; tests `AppStoreRemotePageTests.swift`, `PageServeFramesTests.swift`,
`PresentationFramesTests.swift`, `PresentationHubTests.swift`, `ControlServerTextPageTests.swift`,
`ControlServerRemotePageTests.swift`, `LogFollowerTests.swift`, `RemotePageChannelTests.swift`.
**Rust side:** modify `crates/ngterm-core/src/{remote/page.rs,remote/page_serve.rs,remote/page_channel.rs,remote/frames.rs,remote/hub.rs,commands/overlay_text.rs}`, `crates/ngterm-app/src/{log_follower.rs,page_channel.rs}`.
**Apps:** desktop.
**Conformance:** loopback cases in `tests/protocol/test_text_pages.py`.

- [ ] A presenter listing `page.text` shows log pages: the open carries the log, the claim serves the log's end over
  the page channel, the presenter follows it there. An older presenter gets the log refusal, the plain page for a
  clicked log, and `remote page: …` for search, changes and wrap.
- [ ] On the origin, `follow`, `search`, `wrap` and `changes` aimed at such a slot run on the presenter in order and
  answer with its reply and text state; no answer in 5 seconds is `presenter did not answer`; a page that ends
  meanwhile answers `the viewer showing this overlay is gone`.
- [ ] Every text-state change on a replica is mirrored into the origin's record; a re-offer seeds the new replica
  with following, query and wrap.
- [ ] Rust unit tests with the Swift cases; conformance cases.

### Task 24: Stream upkeep (#938, #955, `c9abe6ff`)

**Depends on:** none. **Size:** S.
**Upstream:** `faeaf74d` (`ControlServer+Zmx.swift`), `fd3320cd` (`ControlEvents.swift`, `ControlServer.swift`),
`PresentationHub.swift` (`pingedAt`), `ControlStreamFraming.swift`, `ControlAcceptLoop.swift`; tests
`ControlServerZmxTests.swift`, `ControlEventRingTests.swift`, `ControlServerTests.swift`,
`PresentationHubTests.swift`, `ControlStreamFramingTests.swift`, `ControlAcceptLoopTests.swift`.
**Rust side:** modify `crates/ngterm-core/src/remote/{hub,attach,split}.rs`, `crates/ngterm-core/src/events.rs`, `crates/ngterm-server/src/{lib,stream}.rs`.
**Apps:** desktop; the mobile core shares the hub's heartbeat: its tests stay green.

- [ ] A split session attached from another machine opens with its left pane focused.
- [ ] The hub closes a viewer only when a ping stays unanswered past the stale timeout: a late heartbeat pings
  before it can call the viewer stale.
- [ ] A control stream write that fails or times out closes the stream (`control stream write failed or timed out;
  closing`).
- [ ] `events.read` and `msg.read` followers: measure the server's idle cost with 20 followers; port the event ring
  change and remove any per-request work those two reads do not need.
- [ ] Rust unit tests with the Swift cases.

### Task 25: The session host (#972, #987, #968)

**Depends on:** Task 1. **Size:** L.
**Upstream:** `a1f95a1e` (`SessionHost.swift`, `LiveReset.swift`, `HostIdentity.swift`, `HostRetirement.swift`),
`2484bfe1` (`HostJob.swift`, `ClientConnector.swift`), `scripts/zmx-patches/{0005-detached-create,0006-single-da-answer}.patch`;
tests `SessionHostTests.swift`, `LiveResetTests.swift`, `AppStoreTreeProjectionTests.swift`,
`ControlServerLiveResetTests.swift`, `SessionHostServerTests.swift`, `HostJobTests.swift`,
`ClientConnectorTests.swift`, `ZmxSupportTests.swift`.
**Rust side:** modify `crates/ngterm-session/src/{server/replies.rs,server/attach.rs,server/spawn/macos.rs,job/macos.rs,launch.rs}`, `crates/ngterm-core/src/{live/reset.rs,commands/live/live_reset.rs,commands/live/launch.rs,projection.rs}`, `crates/ngterm-ctl/src/live.rs`.
**Apps:** desktop; macOS carries the host work, Linux and Windows the device-attributes fix and the fields.
**ngterm's own:** one session server per namespace with an update handoff. The handoff already moves sessions to a
new build, so no host is stale after an update on any OS; what remains is a server started in an earlier login
session on macOS.
**Conformance:** `field:tree.session.liveHostStale`, `splitLiveHostStale` in `tests/protocol/test_live_sessions.py`,
or a recorded deviation where no host here can produce them.
**Decided (team lead, 2026-11-15):** the deviation is accepted as recorded here: no Linux or Windows host produces
`liveHostStale`, and the two fields stay exempted in `catalog.yaml` unless the macOS work gives the suite a way to
reach them.

- [x] One device-attributes answer during attach: no stray `62;22c` at a new pane's prompt (a PTY test in
  `ngterm-session`). Check what `0005-detached-create` changes for a detached create and port what applies.
- [x] macOS: a session server left from an earlier login session is detected; `liveHostStale`/`splitLiveHostStale`
  report its panes; the Live sessions reset selects them as `staleHost` (marker version 3; an older build discards
  such a marker), and each launch asks an earlier server to stop once it holds no session.
- [x] macOS 27 and later: the session server starts as its own launchd job, so a force-quit of a background-disabled
  program reaches only that program; a failed job falls back to the spawn. Checked on the Mac.
- [x] Linux and Windows: record in rust-build.md why the two fields stay omitted there.
- [x] Rust unit tests with the Swift cases that apply.

### Task 26: Session-bound picks and timeouts (#949)

**Depends on:** Task 1. **Size:** L.
**Upstream:** `b8a96b1b` (`SessionPick.swift`, `Pick.swift`, `Ask.swift`, `AppStore+Interactions.swift`
`armDeadline`, `ControlDispatcher+Pick.swift`, `ControlDispatcher+Ask.swift`, `ControlServer+Pick.swift`,
`PresentationFrames.swift` pick frames, `PresentationHub.pickKinds`, `AskCommands.swift`, `MiscCommands.swift`);
tests `SessionPickTests.swift`, `PickTests.swift`, `AskTests.swift`, `ControlDispatcherPickTests.swift`,
`ControlDispatcherAskTests.swift`, `ControlServerPickTests.swift`, `AppStoreRemoteAskTests.swift`,
`AskCommandsTests.swift`, `MiscCommandsTests.swift`.
**Rust side:** modify `crates/ngterm-core/src/{modal.rs,model/ask.rs,commands/pick.rs,commands/ask.rs,remote/frames.rs,remote/hub.rs,remote/viewer.rs,projection.rs}`, `crates/ngterm-ctl/src/{pick,ask,modal}.rs`, `crates/ngterm-gpui/src/{picker.rs,remote.rs}`.
**Apps:** desktop; Android and iOS in Task 38.
**Conformance:** `field:tree.session.pick` and timeout cases in `tests/protocol/test_ask_pick.py`.

- [ ] `pick --session S` binds the picker to a session: it shows wherever that session is presented, the session
  node reports `pick`, and a presenter listing the four `pick.*` kinds is handed it; `pick already pending` and
  `unknown pick: …` as upstream's.
- [ ] `pick --timeout SEC` and `ask open --timeout SEC` cancel an unanswered picker or dialog; out of range reads
  `timeout must be more than 0 and at most 86400 seconds`.
- [ ] Rust unit tests with the Swift cases; conformance cases.

### Task 27: Held interactions on a headless origin (#949)

**Depends on:** Task 26. **Size:** L.
**Upstream:** `b8a96b1b` (`AppStore+Interactions.swift`, `AppStore+RemoteAsk.swift`, `AppStore+RemoteOverlay.swift`,
`AppStore+Presentation.swift`, `OverlayJobs.swift`, `OverlayJobStream.swift`, `PresentationStream.swift`,
`PresentationFrames.swift` pending chunks, `ZmxLead.swift` `presentsRemotely`, `RemoteTree.swift` `headless`,
`CatalogRoot.swift` `HeadlessCommandPolicy`, `OverlayRunJob.swift`, `Commands.swift`); tests
`AppStoreRemoteAskTests.swift`, `AppStoreRemoteOverlayTests.swift`, `OverlayJobsTests.swift`,
`PresentationFramesTests.swift`, `PresentationHubTests.swift`, `RemotePresentationStateTests.swift`,
`RemoteTreeTests.swift`, `AppStoreHeadlessPaneTests.swift`, `OverlayRunJobTests.swift`, `CommandsTests.swift`.
**Rust side:** modify `crates/ngterm-core/src/{remote/hub.rs,remote/frames.rs,remote/tree.rs,remote/page.rs,overlay_jobs.rs,model/remote_presentation.rs,commands/ask.rs,commands/pick.rs,commands/overlay.rs,dispatch.rs,projection.rs}`, `crates/ngterm-headless/src`, `crates/ngterm-ctl/src/{version,overlay_job,pick}.rs`, `crates/ngterm-proto/src/version.rs`.
**Apps:** desktop and `ngterm-headless` on all three OSes; Android and iOS in Task 38.
**ngterm's own:** a page with no viewer is already parked (`pages-survive`); the held-interaction list reports it,
and its parking rules stay. Agent chat and the `no-overlays` kind stay.
**Conformance:** `field:result.headless`, `result.versions`, `tree.session.interactions`; cases against
`ngterm-headless` in `tests/protocol/test_live_sessions.py` and `test_version.py`.

- [ ] On a headless origin an ask, a pick, a page or an overlay job opened with no presenter waits until one that
  can show it takes the session; `interactions` on the session node lists `{kind, id, state, deadline?}` with state
  `unpresented`, `offered` or `claimed`. A claim window starts at the offer; a claimed job never starts twice; a
  restart forgets what waits and fails the waiting call.
- [ ] A presenter listing `pending.snapshot` gets the pending set as chunks and one commit, applied together or not
  at all; `job.token` carries a job's offer token to its helper.
- [ ] `version` adds `headless: true` and `versions {server, protocol, catalog}`; `live tree` reports `headless`; an
  untargeted `pick` binds to the calling pane's session only on a headless origin.
- [ ] A command that drives a window answers `NAME is not available on ngterm-headless: it drives a window on the
  machine showing it`; `session new --wait` and a GUI ask without `--target` refuse as upstream's.
- [ ] Rust unit tests with the Swift cases; conformance cases.

### Task 28: The catalog on ngterm-headless (#949, #993)

**Depends on:** Tasks 1, 27. **Size:** L.
**Upstream:** `b8a96b1b` (`CatalogRoot.swift`, `CatalogFrames.swift`, `CatalogHub.swift`, `CatalogDiff.swift`,
`CatalogOperations.swift`, `AppStore+Catalog.swift`, `ControlStreamFraming.swift`, `CatalogCommands.swift`,
`SocketClient.swift` receipts; `AgtermServerKit` `ServerActions+Receipts.swift`, `ServerCommandPolicy.swift`,
`ServerCatalog.swift`, `ServerStreams.swift`), `0bbdaac3`, `dd3ff080`; tests `CatalogRootTests.swift`,
`CatalogFramesTests.swift`, `CatalogHubTests.swift`, `CatalogDiffTests.swift`, `CatalogOperationsTests.swift`,
`ServerActionsReceiptsTests.swift`, `ServerCatalogTests.swift`, `ServerCommandPolicyTests.swift`,
`ServerStreamsTests.swift`, `CatalogCommandsTests.swift`, `SocketClientTests.swift`.
**Rust side:** create `crates/ngterm-core/src/catalog/` (frames, hub, diff, operations, root); modify
`crates/ngterm-core/src/{dispatch.rs,commands/session.rs,commands/workspace.rs,commands/panes.rs,commands/companions.rs}`, `crates/ngterm-headless/src`, `crates/ngterm-server/src/stream.rs`, `crates/ngterm-ctl/src/{catalog,socket,stream_bridge}.rs`.
**Apps:** `ngterm-headless` on all three OSes serves it; the desktop app refuses; Android and iOS in Task 39.
**ngterm's own:** the server is `ngterm-headless` with its state dir; the strings name it.
**Conformance:** `catalog.subscribe`, `catalog.op`, `field:result.receipt` against `ngterm-headless`; the app's
refusals in `tests/protocol`.

- [ ] `catalog.subscribe` hands its connection to a stream: a hello with the versions, a snapshot with `epoch`,
  `rev` and `catalogID`, then diffs. A session's `name` is its resolved label and `customName` its manual name, `""`
  when it has none.
- [ ] A structural command with `opID` and `epoch` answers a receipt and runs once while its receipt is among the
  last 512; another epoch is refused (`the catalog is at epoch E; take its snapshot and send the command again`);
  `--expect-panes` and `--expect-scratch` refuse when the target changed. `catalog op OPID [--epoch E --rev N]`
  answers applied, noop, refused, fenced or unknown and fences an id it does not hold.
- [ ] The desktop app refuses both commands and any `opID` (`operation receipts are served only by …`).
- [ ] A cut-short session listing never ends live sessions.
- [ ] Rust unit tests with the Swift cases; conformance cases.

### Task 29: The server's scratch and ended rows (#949)

**Depends on:** Task 28. **Size:** M.
**Upstream:** `b8a96b1b` (`ZmxSupport.swift` `scratchDaemonName`, `AppStore+Panes.swift`,
`ServerActions+Scratch.swift`, `ServerPanes.swift`, `PaneObserver.swift`); tests `ServerActionsScratchTests.swift`,
`ServerPanesTests.swift`, `PaneObserverTests.swift`, `AppStorePaneSwapTests.swift`, `ZmxSupportTests.swift`.
**Rust side:** modify `crates/ngterm-headless/src/{panes.rs,state.rs}`, `crates/ngterm-core/src/{live/support.rs,model/panes.rs,commands/panes.rs,projection.rs}`.
**Apps:** `ngterm-headless`; the remote window reads the fields (Task 30).
**Conformance:** `field:tree.session.scratchBacked`, `tree.session.ended`.

- [ ] On the server a session's scratch is a live session of its own name shape, never adopted, mapped or pruned as
  a pane; it is kept while hidden and across a closed window, and ends with its own exit, the session's close, a
  server restart or a `session scratch --command` replacing it; the catalog marks it `scratchBacked`.
- [ ] A session whose last pane is lost, or whose replacement cannot start, stays in the catalog marked `ended`,
  its scratch still running if it had one.
- [ ] Rust unit tests with the Swift cases; conformance cases.

### Task 30: The remote window's core (#949)

**Depends on:** Tasks 28, 29. **Size:** L (split by behavior if a review asks: mirror, actions, detach).
**Upstream:** `b8a96b1b` (`RemoteCatalogClient.swift`, `RemoteCatalogMirror.swift`,
`RemoteCatalogMirror+Detach.swift`, `RemoteCatalogActions.swift`, `RemoteOperation+Request.swift`,
`RemoteWindowViewState.swift`, `WindowLibrary+Remote.swift`, `SshConnectionPool.swift`, `RemoteSession.swift`,
`ControlDispatcher+Detach.swift`, `ControlServer+{RemoteCatalog,RemoteActions,Detach}.swift`,
`ControlTargetResolver.swift`); tests `RemoteCatalogClientTests.swift`, `RemoteCatalogMirrorTests.swift`,
`RemoteCatalogMirror+DetachTests.swift`, `RemoteCatalogActionsTests.swift`, `RemoteOperationRequestTests.swift`,
`WindowLibraryRemoteTests.swift`, `SshConnectionPoolTests.swift`, `RemoteSessionTests.swift`,
`ControlServerRemoteActionsTests.swift`, `AppActions+RemoteCatalogTests.swift`, `ControlDispatcher+DetachTests.swift`.
**Rust side:** create `crates/ngterm-core/src/remote/catalog/` (client, mirror, actions, operations, view state,
pool); modify `crates/ngterm-core/src/{model/window_library.rs,persist/library.rs,commands/window.rs,commands/session.rs,commands/workspace.rs,remote/ssh.rs,projection.rs}`, `crates/ngterm-app/src/remote.rs`, `crates/ngterm-ctl/src/{window,session,workspace}.rs`.
**Apps:** desktop on all three OSes (a Windows viewer runs ssh through `viewer-run`); Android and iOS in Task 39.
**ngterm's own:** Attach Remote's rows, their relaunch and New/End Session on a host are untouched; a remote window
rebuilds its rows from the catalog and saves none as attached rows.
**Conformance:** `window.remote.open|dismiss`, `session.detach|reattach`, `workspace.detach|reattach`,
`field:tree.remote`, `window.remote`, `tree.session.pending`, against a loopback `ngterm-headless`.

- [ ] `window remote open HOST` opens a window whose store the mirror fills from the catalog stream, or raises the
  one already showing that host (`another window already shows the catalog of HOST`); refusals `window.remote.open
  requires a host`, `window.remote.open: not a host ssh can be given`. The node carries `remote`: host, catalogID,
  state (connecting, connected, disconnected, refused, with reason), detachedSessions, detachedWorkspaces,
  operations.
- [ ] Creating, renaming, splitting, moving and closing go to the server with an operation id; selection, focus,
  font size, dividers and the frame stay local. Disconnected, structural actions are disabled, never queued; a lost
  reply becomes pending, then unknown, settled through `catalog.op` and never resent; `window remote dismiss`
  clears the unknown marks. A command a remote row cannot take answers `… is not available in a remote window`.
- [ ] `session detach|reattach`, `workspace detach|reattach` hide and show rows here and leave them running; the
  view state is saved per window and host. Closing the window detaches it.
- [ ] The catalog mirror sets `RemoteBinding::catalog_owned` to `true` on each catalog row binding.
  Test scratch links with origin session IDs and catalog rows without an origin endpoint (Task 11).
- [ ] A pool of shared ssh connections, at most eight channels each, with the keep-alive check on each master.
- [ ] Rust unit tests on the fakes with the Swift cases; conformance cases.

### Task 31: The remote window in GPUI (#949)

**Depends on:** Task 30. **Size:** L.
**Upstream:** `b8a96b1b` (`AppActions+RemoteCatalog.swift`, `RemoteConnectionStrip.swift`, `WindowContentView.swift`,
`WorkspaceSidebar+ContextMenu.swift`, `agtermApp+Menus.swift`, `BuiltinAction.swift`, `PaletteCatalog.swift`,
`TitlebarComposition.swift`); `.claude/rules/{windows,sidebar,keymap}.md`; tests `AppActions+RemoteCatalogTests.swift`,
`BuiltinActionTests.swift`, `PaletteCatalogTests.swift`, `TitlebarCompositionTests.swift`, `SidebarRowViewsTests.swift`,
`RemoteWindowUITests.swift`.
**Rust side:** modify `crates/ngterm-gpui/src/{app_menu.rs,keymap_actions.rs,remote.rs,window.rs,sidebar/{rows,menu,view}.rs,titlebar.rs,palette.rs,ui_ids.rs}`, `crates/ngterm-core/src/{keymap/builtin.rs,palette_catalog.rs,titlebar}`.
**Apps:** desktop on all three OSes, through the `frontend-design` skill.
**Conformance:** `ui:remote-connection`, `ui:remote-unknown` in `tests/ui`.

- [ ] File ▸ Open Remote Window… (`open_remote_window`, no default chord) offers the hosts of `remotes.conf` and
  opens the only one without asking; File ▸ Detached Sessions… (`detached_sessions`) brings a detached session or
  workspace back and lists an ended one only to remove its record.
- [ ] The connection strip: Disconnected with the reason and a retry, and the unknown-outcome line with Dismiss.
  A disconnected window keeps its last sidebar and screens; nothing local takes a session's place.
- [ ] Closing a session, a group or a workspace asks Detach (default), Terminate or Cancel; Terminate needs a live
  connection and has no undo. A row whose panes ended reads Ended on HOST; pending rows are marked.
- [ ] Take `remote-connection` and `remote-unknown` off `PENDING`; ui tests, green on the reference first.

### Task 32: Pages in a remote window (#978, #949)

**Depends on:** Tasks 19, 31. **Size:** L.
**Upstream:** `696edcf0` (`ControlServer+CatalogPage.swift`, `PageAttachmentBook.swift`, `CatalogFrames.swift` page
fields, `CatalogHub.swift` `catalog.skip`, `ServerActions+Overlay.swift`, `ForwardedPage.swift`), `b8a96b1b`
(`PooledForward.swift`, `RemotePortForward.swift`, `CustomCommandRunner.swift`); tests
`ControlServerCatalogPageTests.swift`, `CatalogFramesTests.swift`, `CatalogHubTests.swift`,
`ServerActionsOverlayTests.swift`, `ForwardedPageTests.swift`, `PooledForwardTests.swift`,
`RemotePortForwardTests.swift`, `CustomCommandRunnerTests.swift`.
**Rust side:** modify `crates/ngterm-core/src/{catalog,remote/catalog,remote/companion_page.rs,remote/page_tunnel.rs,commands/companion_page.rs,keymap/command.rs}`, `crates/ngterm-headless/src`, `crates/ngterm-gpui/src/{remote.rs,html_overlay}`.
**Apps:** desktop; a viewer without web views is sent no page rows.
**ngterm's own:** a loopback page opens through `page_tunnel.rs`'s forward.

- [ ] The server keeps page rows (`session.page`, `session.companion.page`, the page attach), with receipts; the
  catalog marks one with no panes. A viewer whose hello does not ask for page rows gets the catalog without them,
  the skipped revisions padded with `catalog.skip`; a companion of a filtered page row shows top-level there.
- [ ] A remote window shows a page row through a view of its own; while it has none the row shows why and retries;
  a server refusal stays shown; detach ends the view, reattach claims a new one; only the server's removal closes
  the row. A page made from the window keeps its outcome on the server for `--block` and `overlay result --page`.
- [ ] A page session at the server's own loopback is refused; a loopback URL overlay opens through a forward kept
  for the page's life, and a failed forward shows as the page's error.
- [ ] Custom commands run on the viewer with the tokens naming the server's session and host; a remote window's
  scratch passes no cwd for links.
- [ ] Rust unit tests with the Swift cases; a loopback conformance case.

### Task 33: `ngtermctl server install HOST` (#949)

**Depends on:** Task 28. **Size:** L.
**Upstream:** `b8a96b1b` (`ServerInstall.swift`, `ServerCommands.swift`, `TerminfoInstall.swift` connection
surface, `ServerHookInstall.swift`, `scripts/server-host-install.sh`); design
`20261109-agterm-server-shipping.md`; tests `ServerInstallTests.swift`, `ServerCommandsTests.swift`,
`ServerHookInstallTests.swift`, `TerminfoInstallTests.swift`.
**Rust side:** create `crates/ngterm-core/src/remote/server_install.rs`; modify `crates/ngterm-ctl/src/{server,terminfo}.rs`, `crates/ngterm-update` (asset naming and the checksum), `crates/ngterm-headless/src/main.rs`, `scripts/package-release.sh`, `scripts/release-artifacts.py`.
**Apps:** the command runs on a desktop of any OS (upstream: macOS only). Host: a Linux host with a systemd user
manager, as upstream's; a macOS or Windows host keeps the manual install of docs/development.md > A server without a
display, left open with that reason.
**Decided (team lead, 2026-11-15):** `server install` supports a Linux host only; macOS and Windows hosts keep the
manual install, and the docs state that reason. The release-packaging change (the archive's checksum sealed into
the build) is in scope of this sync.
**ngterm's own:** the installed server is ngterm's release archive (`ngterm-headless`, `ngtermctl`,
`ngterm-session`), verified as `ngterm-update` verifies a release; the hooks step is `ngterm-headless
--install-agent-hooks`.
**Conformance:** `cli:server install` parse and refusal cases in `tests/cli/test_cli_only.py`.

- [ ] It asks the host for its architecture, takes the release archive matching this build's version, refuses it
  unless its checksum is the one this build was released with, and sends no bytes before that; `--from FILE` skips
  the download under the same check; `-p`, `-i`, `-J`, `-F` pass to ssh. A development build has no release.
- [ ] The install puts the files under the user's home with links in `~/.local/bin`, writes the systemd user unit
  only when it is absent, enables lingering (refused: stops before starting anything and prints the administrator's
  command), and starts or restarts the service; an upgrade is the same command and keeps every session.
- [ ] Exit 0 installed, 3 installed with the agent hooks incomplete, 1 on failure; a bad destination is refused at
  run time.
- [ ] Rust unit tests over injectable effects with the Swift cases; a container install check on Linux.

### Task 34: The agterm peer at v0.42.3

**Depends on:** Tasks 26, 30. **Size:** M.
**Upstream:** `PresentationHub.swift` kinds, `RemoteTree.swift`, `CatalogFrames.swift` as an agterm origin answers
them; `ControlServerZmxTests.swift` fixtures.
**Rust side:** modify `crates/ngterm-core/src/remote/peer.rs`, `conformance/agterm_conformance/fake_agterm.py`, `conformance/tests/protocol/{test_agterm_peer,test_agterm_peer_windows}.py`, `conformance/tests/behaviors/test_agterm_peer_relaunch.py`, `conformance/tests/ui/test_agterm_peer_ui.py`, `crates/ngterm-mobile/src/peer.rs`, `mobile/android/app/src/androidTest/kotlin/com/nickalie/ngterm/android/e2e/AgtermPeerE2eTest.kt`.
**Apps:** desktop, Android, iOS.
**ngterm's own:** the peer layer is this build's; upstream has no counterpart.

- [ ] The fake agterm answers as v0.42.3 does: the new hello kinds, `headless`, `remoteSessionID`, page rows in the
  listing; the peer capabilities list what agterm now offers (`page.forward` toward it).
- [ ] A remote window onto an agterm-server host (`agterm:HOST`): the catalog stream and operation ids under
  agterm's CLI names. Checked against the fake; a real host check on the Mac's side is logged if no agterm-server
  host exists.
- [ ] The peer tests and the Android peer e2e pass at the new level.

### Task 35: The bundled skill to v0.42.3

**Depends on:** every feature task above. **Size:** M.
**Upstream:** `plugins/agterm/skills/agterm/{SKILL.md,reference.md,examples.md,troubleshooting.md}` at v0.42.3.
**Rust side:** `resources/skills/ngterm/` through `sync-resources.sh`; refresh `build/patches/skill-ngterm/0001-live-group.patch`
and every later patch that no longer applies; `crates/ngterm-app/src/integrations/skill.rs` tests.
**Apps:** desktop and `ngterm-headless`.
**ngterm's own:** all 23 patches keep their content; `agterm-server` reads `ngterm-headless`, `agterm://session`
reads `ngterm://session`, and the persistent-pages patch (0021) drops what page sessions now say upstream.

- [ ] Rebuild the copy whole; `sync-resources.sh --check v0.42.3 v0.42.3` reports `unchanged` for both copies.
- [ ] Every command the skill names exists in `ngtermctl` with that spelling.

### Task 36: Desktop UI-only changes (#924, #952, #954, #995, #925, #988)

**Depends on:** none (the page spinner item: Task 19). **Size:** M.
**Upstream:** `c37c7025`, `f3755994` (`SidebarRenameController.swift`), `7ff77b61`
(`WorkspaceSidebar+ContextMenu.swift`), `762a5db1` (`SidebarRowViews.swift`), `c8c47303` (`TerminalDeck.swift`,
`SplitRatioAccessor.swift`), `e8ee2ab2`, `806b2ac8` (`TerminalInputSource.swift`), `HtmlOverlayView.swift:152`;
tests `SidebarRowViewsTests.swift`, `SidebarUITests.swift`, `SidebarRowIndentTests.swift`, `TerminalDeckTests.swift`,
`SplitRatioAccessorTests.swift`, `InputSourceLockTests.swift`.
**Rust side:** modify `crates/ngterm-gpui/src/{sidebar/{view,rows}.rs,deck.rs,split.rs,input_source.rs,settings/key_mapping.rs,html_overlay,ui_ids.rs}`, `crates/ngterm-core/src/input_source_lock.rs`.
**Apps:** desktop only.

- [ ] A sidebar rename editor survives row updates (no hang on a double-click rename), and a workspace collapse
  ends the open rename.
- [ ] A click on a workspace row's icon folds or unfolds it at once, whatever the row-click setting, without the
  double-click delay.
- [ ] In the flat and plain flagged lists the row icon sits inside the selection highlight.
- [ ] Switching to or from a left/right split session with the compact toolbar keeps both panes' sizes: no resize,
  no redraw.
- [ ] "Lock in terminals" becomes "Lock in ngterm": the layout holds in every ngterm window (the palette, pickers,
  dialogs, the find bar, rename fields), only while ngterm is the active app, and the earlier layout returns on
  leaving it; per OS as `input_source.rs` already does, with the unsupported compositors' reason unchanged.
- [ ] A page shows a loading indicator with the id `htmlOverlay.loading`; take it off `PENDING`.
- [ ] GPUI tests for each; ui tests where the reference has one.

### Task 37: The checkmark shape in the Android and iOS apps

**Depends on:** Task 5. **Size:** S.
**Mobile core:** modify `crates/ngterm-mobile/src/{presentation.rs,sessions.rs}`: the hello lists
`status.checkmark`; the shape reaches `MobileEvent`.
**Android:** the status glyph in `mobile/android/app/src/main`; **iOS:** `mobile/ios/Packages/NgtermFeatures`
(`SidebarSessionRow`) and `NgtermDesign`.

- [ ] `ngterm-mobile`: the kind and the shape, with tests on its fakes.
- [ ] Android: the checkmark outline inside `mobile/android/DESIGN.md`, with its JVM tests.
- [ ] iOS: the same inside `mobile/ios/DESIGN.md`, with its unit tests.

### Task 38: Session-bound picks and held interactions in the Android and iOS apps

**Depends on:** Tasks 26, 27. **Size:** L.
**Mobile core:** modify `crates/ngterm-mobile/src/{presentation.rs,presentation/records.rs,api.rs,actor.rs}`: the
`pick.*` kinds, `pending.snapshot`, the timeout's dismissal; `MobileCore` answers a pick; `MobileEvent` carries one.
**Android:** a picker screen beside the asks; **iOS:** `mobile/ios/Packages/NgtermFeatures/Sources/NgtermFeatures/Asks`.

- [ ] `ngterm-mobile`: a presenting phone is handed a session-bound pick and the pending set of a headless origin,
  applied together; a timed-out ask or pick is dismissed; tests on its fakes. Pages and overlay jobs stay refused,
  as the phones show neither.
- [ ] Android: the picker (query, items with status, custom entry, cancel) inside `DESIGN.md`, with JVM tests and an
  emulator check.
- [ ] iOS: the same inside its `DESIGN.md`, with unit tests and a simulator check on the Mac.

### Task 39: The server's catalog and page rows in the Android and iOS apps

**Depends on:** Tasks 20, 28, 29. **Size:** L.
**Mobile core:** modify `crates/ngterm-mobile/src/{sessions.rs,rows.rs,api.rs,transport,reconnect.rs}`: for a
headless origin the sessions list follows `catalog subscribe` (workspaces, order, names, ended rows) and structural
actions carry operation ids; a page row is listed as a page, never attached as a terminal.
**Android:** the machines and sessions screens; **iOS:** `mobile/ios/Packages/NgtermFeatures/Sources/NgtermFeatures/Machines`.

- [ ] `ngterm-mobile`: the catalog client with reconnect from a fresh snapshot, receipts for New Session, rename
  and End Session, an unknown outcome reported; a non-headless origin keeps the listing it has; tests on its fakes.
- [ ] Android: the list updates live, shows Ended and Disconnected states and a page row's kind, inside
  `DESIGN.md`, with JVM tests and an emulator check against a loopback `ngterm-headless`.
- [ ] iOS: the same inside its `DESIGN.md`, with unit tests and a simulator check.

### Task 40: Session links, badge reset and the color scheme in the Android and iOS apps

**Depends on:** Tasks 10, 11, 14. **Size:** M.
**Mobile core:** modify `crates/ngterm-mobile/src/{rows.rs,pane.rs,api.rs}`: a tapped `ngterm://session/<id>` (or
`agterm://` from an agterm origin) resolves to a listed session of that machine; typing clears a session's unseen
mark by the origin's rule; the pane answers the color scheme query from the appearance the app passes.
**Android:** the terminal view and session list; **iOS:** `NgtermTerminal` and `NgtermFeatures`.

- [ ] `ngterm-mobile`: the three behaviors with tests on its fakes; an unknown id does nothing.
- [ ] Android: a tap on a session link opens that session; the app passes dark or light to the pane and on a
  system flip; JVM tests.
- [ ] iOS: the same, with unit tests.

### Task 41: Claim v0.42.3 parity and update the docs

**Depends on:** every task above. **Size:** M.

- [ ] Bump `PARITY_LEVEL` and its mirrors (`tag-mentions.sh v0.41.1`, class `claim`) once every task above is done;
  run the `since("0.42.*")` cases, fix what they expose, add the green ids to both ratchets.
- [ ] README.md, docs/development.md (Parity, A server without a display, Android, iOS), conformance/README.md and
  conformance/docs/rust-build.md (a "Release behavior at v0.42.3" section, the renamed strings, the deviations of
  Tasks 21, 25 and 33) describe what changed.
- [ ] `docs/development.md` no longer says page companions are not kept; `fake_agterm`'s mentions name v0.42.3.
