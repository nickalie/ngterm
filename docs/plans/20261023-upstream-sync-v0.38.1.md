# Upstream sync v0.36.0 -> v0.38.1

## Overview

- Bring ngterm from upstream `umputun/agterm` v0.36.0 to v0.38.1 (72 commits, releases v0.37.0, v0.38.0, v0.38.1).
  The mechanical part is done on branch `upstream-sync-v0.38.1` (commit `3c7e1d5`): the agent-status copy synced
  (OpenCode v1's pending-prompt fix), the parity-target mentions bumped (CLAUDE.md, conformance/README.md,
  .revmux/profile.md). Upstream's pins did not move (ZMX_REV and GHOSTTY_REV unchanged); one new zmx patch
  (`0003-atomic-socket-publish.patch`) is already this build's behavior (Context). Two parts did not go through
  cleanly and are tasks here: the catalog generator refused two v0.38.1 inputs (Task 1), and the bundled skill's
  `0001-live-group.patch` no longer applies (Task 2).
- This plan ports the behavior: the catalog and the new wire names first, the bundled skill, then events
  (`app.launched`, `tree.changed` on overlays, `pane.focused`), `recent`, scratch `--follow`, `session wait`,
  `version` installed, the state-directory lock, `workspace flag`, Mark for Attention, statuses in pickers, the
  hidden-split `live tree` fix, status sounds on the presenter, `ask` forms, loopback `--chromeless`, hide/show
  overlays, HTML pages on a remote presenter, point-sized overlays, the Live fish and Ctrl-C replay work, the session
  server's lock upkeep, the keyboard-layout lock, custom commands in HOME, the notify banner slot, the selection
  clipboard on macOS and Windows, the OpenCode hook tests, then the UI-only items and the parity claim.
- Out of scope: upstream CI, packaging (the Homebrew cask's quarantine fix #784), release metadata and plugin
  manifests, upstream's README/ARCHITECTURE/CLAUDE.md, the cookbook recipes (session-snapshot, antigravity
  status/resume, scheduled-jobs, html-doc-overlay, two-agent-chat: the bundled skill fetches them from upstream's
  repository, they are not part of this build), upstream's docs/backlog and completed plans (design intent only).

## Context (from the sync)

- Upstream range: `.upstream/agterm` at v0.38.1 (`097276106c6d`); diff `git -C .upstream/agterm diff
  v0.36.0..v0.38.1`. Changelog of the range (the release notes in `.upstream/agterm/CHANGELOG.md`):
  - v0.37.0: `recent list|open|clear` #744; Lock in terminals keyboard layout #727; `app.launched` and
    `tree.changed` on overlay open/close #736; loopback `--url --chromeless` #750; `version` `installed:` #723;
    session-snapshot recipe #742; scratch on a background target without switching, `--follow` #730; custom command
    in HOME when the directory is gone #740; skill/docs on upgrading with Live sessions #725; second instance on an
    owned state dir exits 1 #749; launch responsiveness with many restored sessions #747; custom theme selection
    colors in the sidebar #746; liveAttribution after the host's /tmp files aged out #733; `scratch on --command`
    against a shown scratch #730; OpenCode v1 blocked with two prompts #734.
  - v0.38.0: HTML pages on a remote session #793; hide overlays without closing them #790; forms in `ask` #778 #779;
    fish as the Live login shell #761 #760 #768 #780; `session wait` #765; `pane.focused` #785; Flag All Sessions /
    `workspace flag` #787; Mark for Attention #763; agent status in session pickers #774; Antigravity recipes #786;
    scheduled-jobs recipe #789; html-doc-overlay recipe #766; mouse selection off the system clipboard #792; Ctrl-C
    on a Live restored command #773; cmd chords through the Command table #796; cask quarantine #784; notify banner
    slot #772; ask hotkeys on non-Latin layouts #771; status sound on the presenting Mac #775; `zmx tree` with a
    hidden split #764; atomic zmx socket publish #793; Esc on an HTML page leaving full screen #754; window-close
    sheet in Live mode #743; two-agent-chat fixes #783 #752.
  - v0.38.1: `session overlay open|resize --width/--height` in points, `overlayWidth`/`overlayHeight` #798.
- Reference rule (CLAUDE.md): upstream is the reference for protocol, semantics, error strings, `agtermctl`,
  on-disk formats, model and UI. Upstream's `zmx.*` is `live.*` here (`conformance/docs/rust-build.md` > The `live`
  group); upstream's session host and zmx daemons are `ngterm-session`.
- Baseline after the mechanical commit:
  - Run on the full workspace (WPE WebKit is installed on the sync host, so no package lists were needed).
  - `cargo fmt --check`: ok; `cargo metadata --locked`: ok; `scripts/check-tracing-features.sh`: ok; the three
    `scripts/vendor-*.sh --check`: ok; clippy (workspace, all features) and clippy (ngterm-gpui without default
    features): ok; `scripts/lint-report.sh`: ok
  - `GDK_BACKEND=x11 xvfb-run -a dbus-run-session cargo test --workspace`: ok (4627 passed, 0 failed, 14 ignored)
  - selftests: ok; coverage gate: ok (the catalog is still v0.36.0's, so no new key exists yet: Task 1)
  - `run-linux.sh` (full): 653 pass, 0 fail, 14 skipped as unsupported, parity 100%; ratchet holds (667 listed ids,
    14 not run on this host); newly green: none
  - No expected red at the baseline: `ngterm-proto/tests/catalog.rs` and ngterm-ctl's
    `the_command_tree_is_the_catalogs_cli` are green only because the catalog did not move. Task 1's regeneration
    turns them red until its skeletons land in the same task.
- Already in the Rust build (no task):
  - Session host answering startup device-attribute queries while a pane is created (82764b7, 63bbfb1, c40c78a):
    the server's own terminal answers every state query from the session's first byte with no client attached
    (`crates/ngterm-session/src/server/replies.rs`, tests `crates/ngterm-session/tests/replies.rs`
    `every_query_gets_one_answer_with_no_client_attached`, `every_query_gets_one_answer_with_two_clients_attached`).
  - zmx patch `0003-atomic-socket-publish.patch` (#793): one server socket per namespace, removed only by a binding
    server under the namespace's flock held for the listener's life; clients never unlink
    (`crates/ngterm-session/src/endpoint/unix.rs`; tests `crates/ngterm-session/tests/endpoint.rs`
    `a_server_that_holds_the_lock_keeps_the_namespace`, `racing_servers_over_a_stale_socket_leave_one_listening`).
    Task 28 keeps that lock from aging out.
  - Eager terminal layout bound to the deck size (#747, a0258b1): the cause was SwiftUI probing hidden children at
    several sizes; GPUI pages are `absolute().inset_0()`, hidden ones skip paint, and the terminal element has no
    measure function (`crates/ngterm-gpui/src/deck.rs`, `crates/ngterm-gpui/src/terminal/element.rs`). An optional
    deck test is an item of the UI-only task.
  - OpenCode v1 pending prompts (#734): the vendored plugin is synced in `3c7e1d5`
    (`resources/agent-status/opencode/ngterm-status.js`); Task 33 adds the missing test twin.
  - The `cmd` chord and Command-table fix (#796) cannot diverge on Linux: one interceptor matches custom commands,
    `map` alternatives and menu chords through `input::chord` (`crates/ngterm-gpui/src/keymap_dispatch.rs`,
    `crates/ngterm-gpui/src/terminal/input.rs`, tests `crates/ngterm-gpui/src/terminal/input_tests.rs`). Its macOS
    form is an item of the UI-only task.
  - Mouse selections on Linux already go to PRIMARY and middle-click follows `middle-click-action`
    (`crates/ngterm-gpui/src/terminal.rs`, `terminal/clipboard.rs`, `terminal/mouse.rs`, tests in
    `terminal/input_tests.rs`); Task 32 adds the macOS and Windows forms and the missing assertion.
- Not run on this host: the sync host's password-database login shell is fish, so the build does not claim
  `live-sessions` here and these 14 listed ids hold the ratchet without running (CI's zsh leg runs them):
  `tests/behaviors/test_relaunch.py::test_live_session_survives_the_relaunch`,
  `tests/cli/test_cli_only.py::test_clipboard_set_reaches_every_terminal_attached_to_the_pane`,
  `tests/cli/test_cli_output.py::test_live_pane_commands`, `::test_live_present_bridges_the_stream`,
  `::test_live_run_job_runs_the_handed_over_program`, and in `tests/protocol/test_zmx.py`:
  `test_job_run_serves_an_overlay_a_presenter_shows`, `test_kill_ends_the_daemon_and_closes_its_pane`,
  `test_lead_on_a_pane_that_already_leads`, `test_live_panes_run_in_claimed_daemons`,
  `test_present_mirrors_a_live_session`, `test_restart_replaces_the_pane_shell`,
  `test_restart_replay_refuses_a_shell_at_its_prompt`, `test_restart_without_a_command_replays_the_foreground_program`,
  `test_screen_reads_the_daemon_grid`. A task touching Live cannot see them green locally until Task 27 claims
  `live-sessions` on fish; from then on they run here too and must pass.
- Upstream's pins (none is a pin here): ZMX_REV unchanged (`8bab1f01`); `scripts/zmx-patches/0003-atomic-socket-
  publish.patch` and the patches README changed, already this build's behavior (above); GHOSTTY_REV unchanged
  (`683d8db`).
- Catalog: `conformance/tools/gen_catalog.py --tag v0.38.1 --repo .upstream/agterm` refused the upstream input and
  wrote nothing:
  `site/commands.html:1570: wire name CLI only: events.read, window.list, tree is not in the Command enum` and
  `agterm/Views/AskFormViews.swift:406: unparseable identifier argument: Self.rowID(choice.id)`. So the catalog and
  `ui-ids.yaml` are still at v0.36.0, and so is the reference adapter's `PARITY_LEVEL`
  (`conformance/agterm_conformance/impls/swift_macos.py`), because `reference-macos.yml` fetches the release the
  catalog's `upstream:` names. Task 1 fixes the generator and moves all three. Until then no new catalog key exists,
  so the tasks' `covers(...)` keys below are the ones Task 1's regeneration is expected to add: Task 1 records the
  real list (`coverage-map.py --since`) in this plan.
- Catalog after Task 1's regeneration (`coverage-map.py --since HEAD` before its commit): no key changed or removed
  (the ui ids' `at` lines moved, nothing else); added, each with the task that covers it:
  - commands: `recent.list`, `recent.open`, `recent.clear` (Task 5); `workspace.flag` (Task 10);
    `session.overlay.hide`, `session.overlay.show`, `session.overlay.toggle` (Task 20); `session.overlay.page.serve`
    (Task 23, stream mode `page-serve-stream`)
  - fields: `field:result.recent`, `field:result.recentOpen` (Task 5); `field:result.installed` (Task 8);
    `field:tree.session.hiddenOverlays` (Task 20); `field:tree.session.overlayWidth`,
    `field:tree.session.overlayHeight` (Task 25)
  - events: `event:app.launched` (Task 3); `event:pane.focused` (Task 4)
  - cli: `cli:recent list`, `cli:recent open`, `cli:recent clear` (Task 5); `cli:session wait` (Task 7, a composite
    with no wire of its own, `uses: [events.read, window.list, tree]`); `cli:workspace flag` (Task 10);
    `cli:session overlay hide`, `cli:session overlay show`, `cli:session overlay toggle` (Task 20);
    `cli:session overlay page-serve` (Task 23)
  - ui ids: `ui:ask-input`, `ui:ask-choice-{id}`, `ui:ask-other` (Task 17; `ask-other` is a manual entry, its site
    `agterm/Views/AskFormViews.swift:Self.otherRowID`); `ui:overlay-toggle` (Task 21);
    `ui:settings-terminal-input-source` (Task 29)
  - ⚠️ until the last of these keys is covered, the selftest
    `conformance/selftests/test_coverage.py::test_committed_catalog_passes_the_ci_gate` fails with the coverage gate
    it runs (same 30 keys, no baseline); every other selftest passes. The five ui ids wait in `PENDING` in
    `crates/ngterm-gpui/src/ui_ids.rs` until their elements land (Tasks 17, 21, 29 take them off).
- Every task that edits `resources/skills/ngterm` does so only for ngterm-specific text: the skill's upstream text
  for every feature of this range arrives whole in Task 2.
- Upstream's presentation frame kinds: v0.38.1's `supportedKinds` order is `status, hud, notify, context, layout,
  sound, ask.form, ask.other`, then the six `page.*` kinds. Tasks 14, 18 and 22 append in that order.

## Safety rules

<!-- BEGIN HARD RULES -->
HARD RULES (ngterm upstream sync). They hold for every command you run, no matter what the task says:

1. Never touch the user's running ngterm. This session usually runs inside a pane of an installed ngterm, whose
   `NGTERM_*` variables point at it, or of an agterm, whose `AGTERM_*` variables (and `ZMX_*`, where it runs its
   sessions on zmx) do.
   - Strip them before launching anything: `env -u NGTERM_SESSION_ID ...` is not enough, use
     `env $(env | grep -oE '^(NGTERM|AGTERM|ZMX)_[A-Z0-9_]*' | sed 's/^/-u /') <command>`, or `scripts/run-gpui-dev.sh`, or
     the suite's runners, which strip them themselves.
   - Every `ngtermctl` call gets an explicit `--socket <dev instance socket>`, written after the subcommand name
     (`ngtermctl tree --socket /tmp/ngterm-dev-.../state/ngterm.sock`). Without it the CLI resolves
     `$XDG_DATA_HOME/ngterm/ngterm.sock`, the live instance's; `NGTERM_CONTROL_SOCKET` is ignored by ngtermctl.
   - A dev instance gets a private state dir, so its live-session namespace (`/tmp/ngterm-live-<FNV-1a 64 of the
     state dir>`) and the one session server serving it (`ngterm-session server <that dir>`) are private too. Reap
     only through that server (`NGTERM_LIVE_DIR=<that dir> ngterm-session kill --force <name>...`) or processes
     carrying that instance's `NGTERM_LIVE_DIR`. Never `pkill ngterm-session` or `pkill zmx`, never `killall`, never
     run `ngterm-session` without that instance's `NGTERM_LIVE_DIR`.
   - GUI runs headless only: the suite's private weston and dbus (`conformance/tools/run-linux.sh`), or
     `GDK_BACKEND=x11 xvfb-run -a dbus-run-session cargo test --workspace`, which gives the tests a display and a
     session bus of their own. Never a window on the user's desktop, never a connection to its session bus or
     compositor. GPUI renders through Vulkan: a headless app instance renders on Mesa's lavapipe alone
     (`scripts/fetch-lavapipe.sh`), never on the machine's GPU, and on Wayland it needs a seat, which only a recent
     headless weston gives (`scripts/build-weston.sh`). A compositor started by hand copies the setup in
     `conformance/agterm_conformance/impls/base.py`.
   - While an installed ngterm runs, launch a checkout's build only through `scripts/run-gpui-dev.sh` or the suite: a plain launch is forwarded to the running instance through the shared
     app id, or both write one `windows.json`.
   - If a command did reach the live instance, stop and report exactly what changed; do not undo it through the
     live socket yourself.
2. `.upstream/agterm` is read-only: read it, `git diff`/`git show`/`git log` in it, never edit, commit, reset or
   clean there. The only write allowed is the sync's own fetch and checkout of the new tag.
3. Never push, never create tags or releases, never open PRs. Commits stay local.
4. Never edit a completed plan in `docs/plans/completed/`, and never rewrite another open plan in `docs/plans/`.
5. Anything that runs longer than about 30 seconds (cargo build/test/clippy, zig builds, `run-linux.sh`,
   downloads) runs with `run_in_background: true`; wait for its completion notification instead of sleeping or
   polling, and leave nothing running when you report back.
6. Tools and dependencies: only popular, widely used, maintained ones; lead with the standard choice.
<!-- END HARD RULES -->

## Development Approach

- Each task follows CLAUDE.md > "Porting a command group": read upstream's implementation, its Swift tests (exact
  strings, edge cases, field order), `site/commands.html` and the matching `.claude/rules/*.md`; implement in
  `ngterm-core` against the `ngterm-host` fakes with unit tests asserting upstream's full error string; wire the
  app side (`ngterm-gpui`); add the `ngtermctl` part, ported from `agtermctlKit`; claim a capability only once its
  commands implement it.
- Read a commit whole with `git -C .upstream/agterm show <hash>`; a task's "Upstream" block names its commits.
  Upstream's design notes for the big features (read for intent, never port):
  `.upstream/agterm/docs/plans/completed/20261018-ask-input-choices.md`, `20261018-ask-choice-other.md`,
  `20261020-hide-overlays.md`, `20261021-remote-html-overlays.md`.
- New conformance tests follow conformance/README.md > Reference discipline: a test is accepted only when green on
  the reference. reference-macos runs only in CI on macOS (`reference-macos.yml`), so a task that adds a suite test
  marks "green on reference-macos" as a manual check for the user.
- A new suite test keeps upstream's names (`zmx.*`, `agtermctl`) and reads them through the adapter's vocabulary
  (`conformance/agterm_conformance/vocabulary.py`); a new string naming zmx gets a `LIVE_TEXTS` entry and a row in
  rust-build.md's renamed-strings table.
- Appending to an existing `parametrize` list never inserts in the middle: the listed ids must stay put.
- Gate of every task: every command of README > Test's code block up to the selftests, in its order and as written
  there (the window tests under `xvfb-run` and `dbus-run-session`), each pytest command with
  `PYTHONPATH=conformance` in front; then
  `PYTHONPATH=conformance .venv/bin/python -m pytest conformance --coverage-gate` (until the task that closes the
  last uncovered key, only the keys listed for later tasks may stay uncovered); then a full
  `conformance/tools/run-linux.sh` in the background with the ratchet holding, and the newly green ids added with
  `.venv/bin/python conformance/tools/check_ratchet.py --report conformance/reports/rust-linux/nodeids.jsonl --add`.
- Platforms (CLAUDE.md > Platforms): macOS, Linux and Windows are equal targets and upstream is macOS only. A
  behavior upstream builds on a macOS API is ported with its macOS form and its Linux and Windows forms in the same
  task (on Linux per desktop, with a fallback); "no counterpart on <OS>" only for a behavior with no meaning there,
  with the reason. CI builds macOS and Windows code only in the crates `cross-platform.yml`'s package list names:
  the OS-specific logic goes into such a crate wherever it can, and that part ends with the manual check "green on
  `cross-platform.yml`" for the user, who opens a pull request or pushes the branch as `ci/<name>`; code left in a
  crate the list does not hold yet is recorded as "not built on macOS/Windows by CI until `<crate>` joins
  `cross-platform.yml`", never as verified.
- A ratchet id never goes red; no reruns; a function over a `clippy.toml` limit is split, not excused.

## Testing Strategy

- unit: `cargo test --workspace` on the `ngterm-host` fakes; the app's tests under `xvfb-run`.
- conformance: `run-linux.sh` (protocol, cli, behaviors); `tests/ui` skips on Linux.
- macOS and Windows: `cross-platform.yml` builds and tests the crates its package list names there; the user runs
  it through a pull request or a `ci/**` push of the branch.

## Progress Tracking

- mark completed items with `[x]` immediately when done
- add newly discovered tasks with ➕ prefix
- document issues/blockers with ⚠️ prefix
- update plan if implementation deviates from original scope

## Implementation Steps

### Task 1: Catalog generator, the v0.38.1 catalog, and the new wire names and CLI skeletons

**Upstream:**
- `site/commands.html` (the `session wait` card's wire line `CLI only: events.read, window.list, tree`, around line
  1570), `agterm/Views/AskFormViews.swift:406` (`.accessibilityIdentifier(Self.rowID(choice.id))`, with `rowID`
  defined in the same file)

**Rust side:** `conformance/tools/gen_catalog.py` and `conformance/selftests/test_gen_catalog.py` (modify),
`conformance/catalog.yaml`, `conformance/ui-ids.yaml` (regenerate), `conformance/agterm_conformance/impls/swift_macos.py`
(`PARITY_LEVEL`) and `conformance/selftests/test_impls.py` (its assert), `crates/ngterm-proto/src/command.rs`,
`crates/ngterm-proto/src/events.rs`, `crates/ngterm-core/src/commands/` (dispatch arms answering the
not-yet-implemented refusal), `crates/ngterm-ctl/src/` (subcommand skeletons)

**Platforms:** no OS-specific code

**Conformance:**
- `conformance/selftests/test_gen_catalog.py`: a case per new input form
- expected newly green: none (skeletons implement nothing)

- [x] `gen_catalog.py` reads a `CLI only: a, b, c` wire line as a client-side composite: the CLI item (`cli:session
      wait`) with no wire command of its own (control-api.md: "session wait is a client-side composite, not an
      exemption"), the listed wire names checked against the Command enum as the commands it uses
- [x] `gen_catalog.py` resolves an accessibility identifier built by a static helper in the same type
      (`Self.rowID(choice.id)` → the helper's interpolated string, e.g. `ask-choice-{id}`), as it already does for
      interpolated literals; an unresolvable helper still reports `unparseable identifier argument`
- [x] `gen_catalog.py`: `session.overlay.page.serve` joins the stream-mode table beside `session.overlay.job.run`
      (a connection that outlives its reply, Task 23)
- [x] regenerate: `.venv/bin/python conformance/tools/gen_catalog.py --tag v0.38.1 --repo .upstream/agterm`; keep
      hand-written exemption lines (a reflow is formatting)
- [x] move the reference adapter's `PARITY_LEVEL` to `0.38.1` and its assert in
      `conformance/selftests/test_impls.py::test_swift_macos_profile_follows_launch_mode`, in the same commit as the
      catalog (`reference-macos.yml` fetches the catalog's tag)
- [x] run `.venv/bin/python .claude/skills/agterm-upstream-sync/scripts/coverage-map.py --since HEAD~1` and record
      the added, changed and removed keys in this plan's Context, each with the task below that covers it (➕ a task
      for any key no task covers)
- [x] skeletons, so `crates/ngterm-proto/tests/catalog.rs` and ngterm-ctl's `the_command_tree_is_the_catalogs_cli`
      are green: every new wire name (expected: `session.overlay.hide`, `session.overlay.show`,
      `session.overlay.toggle`, `session.overlay.page.serve`, `recent.list`, `recent.open`, `recent.clear`,
      `workspace.flag`) as a Command variant in upstream's order, answering the same refusal the dispatcher gives
      any command it does not implement; every new event kind the catalog lists (expected `app.launched` last,
      `pane.focused` after `pane.scratch`) if catalog.rs compares events; every new CLI subcommand (expected
      `session overlay hide|show|toggle|page-serve`, `recent list|open|clear`, `session wait`, `workspace flag`) as
      a clap skeleton reaching that refusal (or, for `session wait`, a not-implemented error). Fields, flags and
      behavior come in their tasks
- [x] the coverage gate: its uncovered keys are exactly the new ones, each named for its task in Context
- [x] write tests: the generator selftests for both input forms and the stream mode
- [x] gate (Development Approach) passes; newly green ids added with `check_ratchet.py --add` (none newly green;
      ratchet holds; the coverage gate and its selftest twin fail only on the 30 keys listed in Context)

### Task 2: Bundled skill: merge upstream v0.38.1 into `resources/skills/ngterm`

**Upstream:** `git -C .upstream/agterm diff v0.36.0 v0.38.1 -- plugins/agterm/skills/agterm` (SKILL.md,
examples.md, reference.md, troubleshooting.md)

**Rust side:** `resources/skills/ngterm/*` (rebuilt), `build/patches/skill-ngterm/0001-live-group.patch` …
`0005-restore-adapter.patch` (refreshed), `crates/ngterm-app/src/integrations/mod.rs` (the header's release)

**Platforms:** no OS-specific code

- [x] rebuild the copy as upstream v0.38.1's tree plus the recorded patches, in order. `0001-live-group.patch` no
      longer applies (`error: patch failed: SKILL.md:149`): merge by hand onto the new text, then regenerate each
      patch as `diff -u` of upstream's files and the hand-merged copy (the skill's `references/areas.md` > Vendored
      copies). New `agtermctl` calls and `zmx` names in the added text (`recent`, `session wait`, `workspace flag`,
      `session overlay hide|show|toggle`, `ask` forms, remote pages, point sizes, fish) are renamed by
      `0001`/`0002`/`0003` like the rest; links shown to users point at ngterm's own (CLAUDE.md > identity)
- [x] keep this build's own deviations where upstream's text is macOS-only (the keyboard chords on Linux, e.g.
      `toggle_overlay` on `ctrl+alt+shift+j` from Task 20; fish being one of several Live shells here) in
      `0004`/`0005`, not in the names patches
- [x] `bash .claude/skills/agterm-upstream-sync/scripts/sync-resources.sh --check v0.38.1 v0.38.1` passes
- [x] bump the release named in `crates/ngterm-app/src/integrations/mod.rs`'s header to v0.38.1 (the agent-status
      copy is already at v0.38.1, `3c7e1d5`)
- [x] gate passes (the skill install tests in ngterm-app read the copy; the coverage gate and its selftest twin fail
      only on the 30 keys listed in Context; ⚠️ the unrelated WPE flake
      `docs/backlog/the-wpe-wheel-input-test-flakes-in-the-shm-worker.md` failed `ngterm-wpe --test headless` in two of three runs)

### Task 3: Events: `app.launched` and `tree.changed` on overlay, page, HUD and remote-slot open and close (#736)

**Upstream:**
- implementation (commit `8c5ac7a`): `agtermCore/Sources/agtermCore/ControlEvents.swift`, `WindowLibrary.swift`
  (`emitAppLaunched`, `appLaunchSettled`), `AppStore+Panes.swift` (`openOverlay`, `closeOverlay`,
  `openPaneOverlay`, `closePaneOverlay`, `swapPanes`, `closeSplit`, `closePrimaryPane`,
  `dropUnrealizedPaneOverlays(of:)`), `AppStore+HtmlOverlay.swift`, `AppStore+RemoteOverlay.swift`
  (`trackingSlots`), `Session.swift`, `agterm/agtermApp.swift` (launch point),
  `agtermCore/Sources/agtermctlKit/EventCommands.swift`
- tests: `agtermCore/Tests/agtermCoreTests/AppStoreEventTests.swift` (aProgramOverlayInvalidatesTheTreeOnOpenAndCloseOnly,
  aPaneOverlayInvalidatesTheTreeOnOpenAndCloseOnly, anHtmlPageInvalidatesTheTreeOnOpenAndClose,
  aHudInvalidatesTheTreeOnOpenAndCloseButNotOnUpdate, appLaunchedIsAppendedOncePerRunAndReachesTheHookSink,
  appLaunchedReachesTheRingWithNoObserver, closingASplitInvalidatesTheTreeOnlyForAPageItTakesDown,
  promotingTheSplitInvalidatesTheTreeOnlyForAPageThatChangesPane,
  swappingPanesInvalidatesTheTreeOnlyForAPageThatChangesPane, aLaunchThatWasNotReadyNeverEmitsAppLaunchedLater);
  `AppStoreRemoteOverlayTests.swift` (aRemoteOverlayInvalidatesTheTreeWhenItsSlotIsTakenAndFreed,
  aHeldRemoteSlotInvalidatesTheTreeOnlyOnceItIsFreed, promotingTheSplitInvalidatesTheTreeForAReservationThatChangesPane,
  swappingPanesInvalidatesTheTreeForAReservationThatChangesPane, aRefusedRemoteOverlayLeavesTheTreeAlone);
  `ControlEventProtocolTests.swift` (golden `{"kind":"app.launched","payload":{},"seq":12,"ts":12.5}`);
  `agtermctlKitTests/EventCommandsTests.swift` (`00:00:00 app.launched`); `agtermUITests/EventSubscriptionUITests.swift`
  (testLaunchAndOverlayEvents)
- contract: `site/commands.html` `events` (tree.changed "…or an overlay, page or HUD opened or closed"; `app.launched`)
  and `session hud` ("A HUD opening or closing emits tree.changed; an update emits nothing"); `.claude/rules/control-api.md`
  (overlay-slot bullet, `app.launched` bullet); `site/docs.html` hooks (Kinds; a `session hud open` under
  `on tree.changed` loops)

**Rust side:** `crates/ngterm-proto/src/events.rs`, `crates/ngterm-core/src/events.rs`, `crates/ngterm-core/src/dispatch.rs`
(`Core::emit_app_launched(ready)`), `crates/ngterm-core/src/commands/overlay.rs` (`drop_stranded_pane_overlays`,
`pane_overlay_exited`), `commands/hud.rs` (`overlay_exited`), `remote/viewer.rs` (`close_replica_overlay`),
`model/overlay.rs`, `model/pane_overlay.rs`, `model/html_overlay.rs`, `model/remote_overlay.rs`, `model/panes.rs`,
`crates/ngterm-gpui/src/app.rs` (after the control server's `start`), `crates/ngterm-ctl/src/events.rs`,
`conformance/agterm_conformance/cli_format.py`, tests in `crates/ngterm-core/src/commands/tests/events.rs`,
`model/tests/hud.rs`, `model/tests/panes.rs`, `commands/tests/remote.rs` (modify)

**Platforms:** the emission and tree invalidation are OS-neutral (ngterm-core). Readiness is "the control server
started": the AF_UNIX `ControlServer::start` result on Linux and macOS; on Windows the named-pipe server's start
once that server exists. Until then a Windows launch reports not ready and emits none (upstream's unbound-socket
behavior).

**Conformance:**
- existing: `tests/protocol/test_events.py::test_session_lifecycle_events`, `tests/cli/test_cli_output.py::test_events_print_every_event_with_its_arguments`,
  `tests/behaviors/test_hooks.py` (listed)
- new: `tests/behaviors/test_hooks.py::test_app_launched_runs_its_hook_once_per_launch` (covers `event:app.launched`,
  `behavior:hooks.loaded-at-launch`: an `on app.launched` hook appending its stdin JSON runs once per relaunch with
  `payload == {}` and no window/workspace/session; `events.read kinds=["app.launched"]` from after=0 returns 1),
  `tests/protocol/test_overlay.py::test_program_and_page_open_and_close_invalidate_the_tree` (covers
  `event:tree.changed`, `session.overlay.open`, `session.overlay.close`: a program, an html page and a `--pane left`
  overlay each emit one on open and one on close), `tests/protocol/test_hud.py::test_hud_open_and_close_invalidate_the_tree`

- [x] proto: `EventKind::AppLaunched` = `app.launched`, declared last (upstream's `allCases` order feeds the starter
      hooks.conf "Kinds:" line); golden test
- [x] core: `Core::emit_app_launched(ready)`: the first call settles a run-once flag, only a ready first call appends
      one window-less event with an empty payload and runs the hooks; a later call never emits
- [x] gpui `app.rs`: post `emit_app_launched(start.is_ok())` right after the control server's `start`, which runs
      after `load_hooks_at_launch` and the `attach` post
- [x] tree.changed only on a real slot change, from the store method that made it: `occupy_overlay` success (a HUD
      replacing a HUD emits again), `close_overlay` true (close_hud, a timed HUD expiry, a page close),
      `open_html_overlay` Ok (both branches), `open_pane_overlay` Ok, `close_pane_overlay` true; `update_hud`,
      `set_hud_size`, `remeasure_hud`, `resize_overlay` and refusals emit nothing
- [x] the slot closes outside the store methods schedule too: `Core::overlay_exited`, `Core::pane_overlay_exited`,
      `close_replica_overlay`, each on the owning window's store, only for a session in the tree
- [x] remote slots: `reserve_remote_overlay` schedules; a helper compares the slot list around
      `end`/`surface_gone`/`remove` (finish, close, surface_closed, presenter_lost, reject, drop); a held `--wait`
      slot emits nothing on finish and once its surface closes
- [x] `swap_panes`/`close_primary_pane` schedule when either pane holds a pane overlay or a remote slot;
      `close_split` when the right pane holds one; `drop_stranded_pane_overlays` on a true return; session,
      workspace and window removal add none of their own
- [x] ngtermctl human form `"{time} app.launched"`; `cli_format.py` the same branch in this change (it raises on an
      unknown kind, and the listed `test_events_print_every_event_with_its_arguments` reads unfiltered events)
- [x] write tests: Rust unit tests for each Swift case (program/pane overlay open 1, refused 0, close 1, second
      close 0; page session-wide and left 1 each; HUD open 1, update 0, close 1; app.launched once over two calls,
      reaches the hook feed and the ring with no hook; not-ready then ready = 0; close split with/without a right
      page 1/0; promote and swap with/without a page 1/0; remote reserve 1, finish 1, held finish 0 then surface
      closed 1, promotion and swap with a reservation 1, refused 0); proto golden; ngterm-ctl human line
- [x] write tests: the conformance tests above
- [x] manual check (user): the new conformance tests are green on reference-macos (`gh workflow run
      reference-macos.yml --ref upstream-sync-v0.38.1` on the pushed branch, or a pull request touching `conformance/`)
      (skipped - not automatable: reference-macos runs only in CI on macOS; left for the user)
- [x] gate (Development Approach) passes; newly green ids added with `check_ratchet.py --add` (full suite deferred
      to the end of the plan per the user; the coverage gate and its selftest twin fail only on the 29 keys of later
      tasks; ⚠️ the load-sensitive `ngterm-gpui --test html_overlay` frame-rate case failed twice in the workspace run
      at load average ~18 and passed alone at 48 frames/s)

### Task 4: Events: `pane.focused` when a session's split-pane selection settles (#785)

**Upstream:**
- implementation (commit `e958477`): `agtermCore/Sources/agtermCore/AppStore+PaneFocus.swift` (new:
  selectedPaneIdentities, observePaneFocus, trackPaneFocus, emitPaneFocusChanges), `AppStore.swift`
  (paneFocusBaseline, the debouncer, the rebaseline at the end of `restore(from:)`), `ControlEvents.swift`
  (`paneFocused` after `paneScratch`; payload `paneID` after `host`), `agtermctlKit/EventCommands.swift`
- tests: `AppStoreEventTests.swift` (paneSelectionMovesEmitEachSettledPaneWithItsToken, openingANewSplitEmitsTheNewPane,
  aBurstReturningToTheSettledPaneEmitsNothing, swapsAndSameValueWritesEmitNothing, closingTheSelectedSplitEmitsThePrimary,
  promotingTheSelectedSurvivorEmitsNothing, promotingAfterThePrimaryExitsEmitsTheSurvivor,
  aReloadIsNotASelectionButALaterFlipIs, aSessionNewToTheStoreOnlyJoinsTheBaseline); `ControlEventProtocolTests.swift`
  (golden `{"kind":"pane.focused","payload":{"name":"api","pane":"right","paneID":"tok"},"seq":13,"session":"sess","ts":13.5,"window":"win","workspace":"work"}`);
  `EventCommandsTests.swift` (`00:00:00 pane.focused api right pane-id=tok`, `00:00:00 pane.focused api left pane-id=-`)
- contract: `site/commands.html` `events` (pane.focused: settled over 100 ms, `pane` and `paneID`, a swap emits nothing);
  `.claude/rules/control-api.md` pane.focused bullet

**Rust side:** `crates/ngterm-proto/src/events.rs` (`PaneFocused` after `PaneScratch`; `EventPayload.pane_id`
serialized `paneID`, after `host`), `crates/ngterm-core/src/model/store.rs` (`pane_focus_baseline` on the AppStore,
`selected_pane_identities()`, `emit_pane_focus_changes()`), `model/snapshot.rs` (restore rebaselines),
`crates/ngterm-core/src/events.rs` (a per-window `PaneFocusTimer`, like `TreeChangedTimer`, 100 ms),
`crates/ngterm-core/src/dispatch.rs` (`Core::commit` diffs and re-arms; `Core::fire_pane_focus`),
`crates/ngterm-gpui/src/state.rs` (arm the timer, skip while quitting), `crates/ngterm-ctl/src/events.rs`,
`conformance/agterm_conformance/cli_format.py`

**Platforms:** no OS-specific code

**Conformance:**
- new: `tests/protocol/test_split.py::test_pane_focused_follows_the_selected_split_pane` (covers `event:pane.focused`,
  `session.focus`, `session.split`: split on gives `pane == "right"`; `session.focus pane=left` exactly one more,
  `pane == "left"`, `paneID` equal to the left surface's from `tree`), `tests/protocol/test_split.py::test_a_swap_keeps_the_selected_pane_silent`
  (covers `event:pane.focused`, `session.swap`: no pane.focused for A before session B's in another window, as the
  read-back sentinel); extend `tests/cli/test_cli_output.py::test_events_print_every_event_with_its_arguments`'s
  covers with `event:pane.focused`

- [x] proto: `pane.focused` between `pane.scratch` and `remote.opened`; `paneID` payload field
- [x] store: `selected_pane_identities()` maps each session to `split_pane_identity.unwrap_or(pane_identity)` when
      `focuses_split()`, else `pane_identity`; `emit_pane_focus_changes()` emits one per session whose identity
      moved from the baseline (workspace, session, display `name`, `pane` left/right, `paneID` = that pane's surface
      token, absent before its surface exists); a session missing from the baseline only joins it (in
      `model/pane_focus.rs`, upstream's `AppStore+PaneFocus.swift`)
- [x] the diff lives in `Core::commit`, which every `split.focused` writer passes through; it re-arms the window's
      100 ms trailing timer whenever identities moved, and the fired timer diffs against the settled baseline (a
      move and back inside 100 ms emits nothing)
- [x] `restore` rebaselines; a swap, a hidden split shown again, a same-value write and a window coming forward emit
      nothing; a new split emits `right`; closing the selected split emits `left` with the primary token; the
      primary exiting while the survivor was selected emits nothing; while it was selected, `left` with the
      survivor's token
- [x] ngtermctl human `"{time} pane.focused {name} {pane} pane-id={paneID|-}"`; `cli_format.py` in the same change
- [x] write tests: Rust unit tests for each Swift case above; proto golden; ngterm-ctl human lines
- [x] write tests: the conformance tests above
- [x] manual check (user): green on reference-macos (skipped - not automatable: reference-macos runs only in CI on
      macOS; left for the user)
- [x] gate passes; newly green ids added with `check_ratchet.py --add` (full suite deferred to the end of the plan per
      the user; the coverage gate and its selftest twin fail only on the 28 keys of later tasks; the new and changed
      suite tests ran green in a targeted `run-linux.sh -k` run)

### Task 5: `recent list|open|clear` for File ▸ Open Recent (#744)

**Upstream:**
- implementation (commit `6b56474`): `agtermCore/Sources/agtermCore/ControlRecent.swift` (new),
  `ControlDispatcher+Recent.swift` (new), `ControlDispatcher.swift`, `ControlActionsDefaults.swift`,
  `ControlProtocol.swift`, `RecentClosed.swift`, `WindowLibrary+RecentClosed.swift`, `HtmlBridge.swift`,
  `agterm/Control/ControlServer+Recent.swift`, `agtermctlKit/RecentCommands.swift`, `Commands.swift`, `SocketClient.swift`
- tests: `ControlDispatcherRecentTests.swift` (listAndClearRouteToActions, openPassesTheEntryAndWindowThrough,
  listAndClearRefuseATargetOrWindowBeforeAnyAction); `ControlRecentTests.swift` (aSessionEntryProjectsItsParentWorkspaceAndOneSession,
  aWorkspaceEntryProjectsItselfAndEverySnapshotSession, payloadsRoundTripThroughTheWire, noTargetResolvesTheNewestEntry,
  aTargetResolvesByExactIdOrUniquePrefix, anEmptyListIsRefusedWhateverTheTarget, unknownAmbiguousAndActiveTargetsAreRefused);
  `WindowLibraryRecentClosedTests.swift` (aReopenReportsItsWindowAndConsumesTheEntry,
  aReopenReportsTheWindowHoldingTheWorkspaceNotTheRequestedOne, anEntryWhoseRemovalCannotBeWrittenReportsIncomplete,
  anUnknownEntryReopensNothing); `HtmlBridgeTests.swift` (recentOpenReopensIntoThePagesWindow and the windowless list);
  `agtermctlKitTests/RecentCommandsTests.swift` (all 4); `SocketClientTests.swift` (formatsRecentRowsWithFullIdsAndAnEmptyList,
  formatsRecentOpenAndSaysWhenTheEntryStays); `agtermTests/ControlServerRecentTests.swift` (all 9);
  `agtermUITests/ControlRecentUITests.swift` (testClosedSessionReopensWithItsIdSplitFlagContextAndPin)
- contract: `site/commands.html` `#recent`; `.claude/rules/control-api.md` (command list, "## Recent Closed commands",
  the page-bridge rule)

**Rust side:** `crates/ngterm-proto/src/command.rs` (the `recent` group from Task 1's skeleton),
`crates/ngterm-proto/src/recent.rs` (create: `ControlRecentItem {id, kind, title, subtitle?, closedAt, workspaceID,
sessionIDs}`, `ControlRecentOpen {item, window, complete}`), `crates/ngterm-proto/src/result.rs` (`recent`,
`recentOpen`), `crates/ngterm-core/src/commands/recent.rs` (create), `commands/mod.rs`, `commands/tests/recent.rs`
(create), `crates/ngterm-core/src/model/window_library.rs` (a reopen outcome `{window, complete}`),
`crates/ngterm-core/src/error.rs` (`recent item` noun), `crates/ngterm-core/src/html/bridge.rs` (windowless
`recent.list`/`recent.clear`; `recent.open` placed locally), `crates/ngterm-ctl/src/recent.rs` (create),
`crates/ngterm-ctl/src/main.rs`, `crates/ngterm-ctl/src/output.rs`, `conformance/agterm_conformance/cli_format.py`

What exists to reuse: `recent_closed_items`, `clear_recent_closed_items`, `reopen_recent_closed_reporting_window`,
`restore_destination`, `session_ids_held_outside` (`crates/ngterm-core/src/model/window_library.rs`),
`RecentClosedRestore {restored, complete}` and `restore_recent_closed_with` (`model/recent_closed.rs`), the on-disk
`recent-closed.json` (`persist/recent_closed.rs`, times in reference-date seconds), the menu path
(`app_actions.rs` ReopenRecent, `menu_lists.rs`).

**Platforms:** no OS-specific code; a write-failure unit test uses a read-only directory behind `cfg(unix)` and a
file where the directory should be on Windows.

**Conformance:**
- new: `conformance/tests/protocol/test_recent.py` (create; every test runs `recent.clear` first: the list is
  app-global): `test_list_reports_closed_sessions_newest_first_and_clear_empties_it` (covers `recent.list`,
  `recent.clear`, `field:result.recent`), `test_open_without_an_id_reopens_the_newest_and_consumes_it` (covers
  `recent.open`, `field:result.recentOpen`), `test_open_by_prefix_reopens_that_entry`,
  `test_reopened_session_keeps_its_id_split_flag_context_and_pin`,
  `test_open_routes_to_the_window_holding_the_workspace_without_moving_frontmost`,
  `test_open_during_the_grace_window_restores_the_original_group`, `test_recent_refusals`;
  `tests/cli/test_cli_output.py::test_recent_list_open_clear` (covers `cli:recent list`, `cli:recent open`,
  `cli:recent clear`)

- [x] wire: `result.recent` newest first, each `{id, kind "session"|"workspace", title, subtitle?, closedAt (Unix
      seconds, converted from the file's reference-date seconds), workspaceID (the parent for a session entry, the
      workspace itself otherwise), sessionIDs}`; `result.recentOpen` `{item, window, complete}`
- [x] recent.list / recent.clear with a target or `args.window` are refused before anything runs: "recent.list takes
      no target or --window" (and the recent.clear form)
- [x] recent.open: resolve the `--window` placement first (an unknown or closed window fails before any restore);
      then the target: none = newest; empty list "no recently closed items"; exact id or unique prefix, never
      `active`: "no such recent item: X", "ambiguous recent item prefix 'X' → …"; a failed restore "could not
      reopen <UUID>"; through the menu's restore path, without `take_frontmost`; the reply names the window it
      actually went to (the one holding the session or workspace wins over `--window`)
- [x] `complete` and recent.clear's ok are read from the reloaded list: commit, then reload through
      `RecentClosedStore::load()` when persistence is present; a failed clear "could not clear recent items" keeps
      the entries
- [x] HTML bridge: recent.list and recent.clear get no window; recent.open still gets the page's window (an explicit
      `--window` wins)
- [x] ngtermctl `recent list`, `recent open [ID] [--window W]`, `recent clear` (list and clear refuse `--window`),
      upstream's help text; output rows `<full id>  <kind>  <title> (<subtitle>)`, empty `no recently closed items`,
      open `reopened <kind> <title>` plus ` (incomplete: the entry stays in the list)` when not complete
- [x] write tests: Rust unit tests for every Swift case above (projection, round trip, resolution incl. lowercase,
      prefix, `active` and ambiguous; refusals before any action; the reopen outcome incl. workspace-owner routing
      and an unwritable removal giving incomplete; grace-window undo; clear failure; bridge window sets; clap and
      output formatting)
- [x] write tests: the conformance tests above
- [x] manual check (user): green on reference-macos (skipped - not automatable here: reference-macos runs only in CI on macOS)
- [x] gate passes; newly green ids added with `check_ratchet.py --add` (full suite deferred to the end of the plan per the user; the 8 new ids passed in a targeted run and are added in Task 35)

### Task 6: `session scratch`: showing never selects; `--follow`; `--command` against a shown scratch (#730)

**Upstream:**
- implementation (commit `da375b9`): `agterm/Control/ControlServer+SessionActions.swift`,
  `agtermCore/Sources/agtermCore/ControlDispatcher.swift`, `ControlActionsDefaults.swift`, `AppStore+Panes.swift`,
  `Session.swift` (scratchGeneration: SwiftUI view identity, no Rust counterpart), `agterm/Views/WindowContentView+Detail.swift`,
  `agtermctlKit/SessionCommands.swift`
- tests: `agtermTests/ControlServerScratchTests.swift` (testShowingABackgroundScratchLeavesTheSelection,
  testFollowSelectsTheTargetOnlyWhenShowing, testTheFourArgumentEntryPointShowsWithoutSelecting);
  `ControlDispatcherTests.swift` (follow passes through); `AppStorePaneTests.swift`; `agtermctlKitTests/CommandsTests.swift`
  (sessionScratchFollow); `agtermUITests/ControlOverlaySplitUITests.swift` (testScratchCommandReplacesAShownScratch,
  testSessionScratchOnABackgroundTargetRunsWithoutSwitchingUnlessFollowed,
  testBackgroundSessionScratchTakesTheKeyboardOnlyOnceSelected)
- contract: `site/commands.html` `session.scratch` (`[--follow]`); `.claude/rules/control-api.md` ("Showing never
  selects; --follow does, as on overlay.open")

**Rust side:** `crates/ngterm-core/src/commands/panes.rs` (the forced select at the `if want &&
store.selected_session_id() != Some(id)` branch), `commands/tests/panes.rs`, `crates/ngterm-ctl/src/session_panes.rs`
(`--follow`), `crates/ngterm-gpui/src/deck/tests.rs`, `conformance/tests/protocol/test_split.py`

**Platforms:** no OS-specific code

**Conformance:**
- existing: `tests/protocol/test_split.py::test_scratch_shows_and_hides`, `test_scratch_rejects_an_unknown_mode` (listed)
- new: `test_split.py::test_scratch_on_a_background_session_starts_without_switching_unless_followed` (covers
  `session.scratch`, `field:tree.session.scratch`: `scratch on` A with a marker-writing `--command` while B is
  selected: the marker appears, A has scratch, B stays active; then `--follow` selects A),
  `test_split.py::test_scratch_follow_selects_only_when_showing`, `test_split.py::test_scratch_command_replaces_a_shown_scratch`

- [x] `session.scratch` selects its target only when showing with `follow: true`; hiding never selects (behavior
      change: the forced switch also cleared the target's unseen badge and reordered Ctrl-Tab recency; neither may
      happen now)
- [x] ngtermctl `--follow` ("Select (switch to) the target session when showing its scratch (default: show without
      switching)."), sent only when given
- [x] confirm `--command` against a shown scratch runs the new program end to end (deck drops the torn-down token and
      mounts a fresh scratch); port nothing for scratchGeneration unless that check fails (confirmed by the deck test
      `a_scratch_command_replaces_a_shown_scratch` and the conformance test; nothing ported)
- [x] write tests: replace `showing_the_scratch_selects_its_session` with
      `showing_a_background_scratch_leaves_the_selection` and `follow_selects_the_target_only_when_showing`; a CLI
      parse test; a GPUI deck test that `focus_target` stays on the shown page with a background scratch mounted and
      moves to that scratch once its session is selected
- [x] write tests: the conformance tests above
- [x] manual check (user): green on reference-macos (skipped - not automatable here: reference-macos runs only in CI on macOS)
- [x] gate passes; newly green ids added with `check_ratchet.py --add` (full suite deferred to the end of the plan per the user;
      a targeted `run-linux.sh -- -k scratch` ran 19/19 green with the three new ids newly passing, left for Task 35's `--add`)

### Task 7: `ngtermctl session wait --until STATUS... [--timeout SEC]` (#765)

**Upstream:**
- implementation (commit `fff13ef`): `agtermCore/Sources/agtermctlKit/Session+Wait.swift` (new),
  `SessionCommands.swift`; `session-wait-watchdog-probe/main.swift` is a test fixture (no counterpart: a dedicated
  std thread cannot starve)
- tests: `agtermctlKitTests/SessionWaitCommandsTests.swift` (all 20 but the GCD probe: currentStatusMatchesWithoutReadingEvents,
  anchorIsTakenBeforeTheTreeIsRead, absentStatusReadsAsIdle, laterEventForTheSessionMatchesAndOthersAreIgnored,
  repeatedStatusEventCountsAsReached, sessionClosedEndsTheWait, deadlinePassesWithoutAMatch,
  refusedConnectRetriesWithinTheDeadline, missingSocketFailsAtOnce, runChangeFailsTheWait,
  activeResolvesToTheFrontmostWindowsSelection, prefixResolvesInABackgroundWindow,
  prefixMatchingSessionsInTwoWindowsIsAmbiguous, unresolvedTargetFails, closedWindowsAreNotRead,
  outputIsTheStatusOrOneJSONObject, rejectsInvalidArguments, watchdogFiresWhileTheCallerIsBlocked, cancelledWatchdogDoesNotFire)
- contract: `site/commands.html` session card (`CLI only: events.read, window.list, tree`); `.claude/rules/control-api.md`
  (client-side composite; cursor anchored before the tree read; the watchdog; `{id,status}` a deliberate JSON exception)

**Rust side:** `crates/ngterm-ctl/src/session_wait.rs` (create), `crates/ngterm-ctl/src/session.rs` (`Wait`, replacing
Task 1's skeleton), `crates/ngterm-ctl/src/events.rs` (`StreamState`, `retry_delay`, `consume`, `FIRST_RETRY_DELAY`
reusable within the crate, behavior unchanged), `conformance/tests/cli/test_cli_only.py`. Reuse ngterm-core's
`resolve`/`resolution_error` for the target.

**Platforms:** nothing Linux-specific: a std watchdog thread that writes `timed out` and `exit(124)` at the
deadline, `Instant` for the clock. ngtermctl builds on Linux only today (crate map); the code carries no Unix-only
call, so it is ready for the CLI's macOS and Windows forms.

**Conformance:**
- new (all covering `cli:session wait`): `test_cli_only.py::test_session_wait_returns_at_once_when_the_status_already_matches`
  (`completed\n`, exit 0; `--json` exactly `{"id":"<UUID>","status":"completed"}`),
  `test_session_wait_sees_a_later_status_event` (`--until completed --until blocked`, another session's blocked
  ignored, prints `blocked`), `test_session_wait_exits_2_when_the_session_closes` (stderr `session closed`),
  `test_session_wait_times_out_with_124` (`--timeout 0.5`, stderr `timed out`),
  `test_session_wait_refuses_bad_arguments` (parametrized: no `--until`, `--until done`, `--timeout 0|-1|inf`,
  `--target DDDD` → "no such session: DDDD", exit 1)

- [x] validation before any I/O: "provide at least one --until status"; "invalid status: X (expected idle, active,
      completed or blocked)"; "--timeout must be a positive number of seconds" (finite, > 0)
- [x] anchor the event cursor first (`events.read` kinds `["status","session.closed"]`, no `after`), then
      `window.list`, then `tree --window W` per open window; resolve the target client-side (`active` = the active
      window's active session); an absent status reads as idle; a current match returns without another read
- [x] poll from the cursor: the target's `session.closed` → exit 2 "session closed"; a status in the set (a repeat
      included) matches; 250 ms sleep after an empty page, clipped to the deadline; a refused connect retries through
      `StreamState.retry_delay`; a missing socket and "event run changed" fail at once (exit 1); deadline → exit 124
      "timed out"
- [x] output: the status word, or with `--json` one compact sorted-key `{"id","status"}` object
- [x] the watchdog enforces `--timeout` while a send blocks, cancelled on return
- [x] confirm the core emits `session.closed` on a soft close and on a window close (upstream's exit-2 contract);
      ➕ a task if it does not (it does: soft close in `model/tests/pending_close.rs`
      `soft_close_undo_and_finalize_emit_only_the_visible_membership_edges`, window close pinned by the new
      `model/tests/library.rs` `closing_a_window_emits_session_closed_for_each_of_its_sessions`; no new task)
- [x] write tests: Rust unit tests on a fake send/sleep/now for the Swift cases; the watchdog firing while blocked
      and a cancelled one not firing (inject the fire closure)
- [x] write tests: the conformance tests above (the unresolved target is `zzzz`, not `DDDD`: a hex prefix could
      match a shared instance's random session id; the relay in the module waits until the CLI polls before acting)
- [x] manual check (user): green on reference-macos (skipped - not automatable here: reference-macos runs only in CI
      on macOS; left for the user)
- [x] gate passes; newly green ids added with `check_ratchet.py --add` (full suite deferred to the end of the plan
      per the user; the targeted `run-linux.sh -- -k session_wait` passed 10/10, its ids are added in Task 35)

### Task 8: `version` reports the installed build: `installed:` / `result.installed` (#723)

**Upstream:**
- implementation (commit `18da29c`): `agtermCore/Sources/agtermCore/AppIdentity.swift`, `ControlProtocol.swift`,
  `agterm/Control/ControlServer.swift`, `ControlServer+AppCommands.swift`, `agtermctlKit/SocketClient.swift`
- tests: `AppIdentityTests.swift` (installedReadsVersionAndCommitFromTheBundlePlist,
  installedKeepsTheVersionWhenTheCommitIsUnusable, installedFollowsThePlistAsItChangesOnDisk,
  installedIsNilWithoutAReadableVersion, installedIdentityRoundTripsBesideAppAndIsOmittedWhenAbsent);
  `SocketClientTests.swift` (formatsInstalledIdentityOnlyWhenItDiffersFromTheRunningOne);
  `agtermTests/ControlServerWorkspaceCommandsTests.swift` (testVersionRereadsInstalledMetadataWhileTheRunningIdentityStaysFixed)
- contract: `site/commands.html` `version`; `.claude/rules/control-api.md` (`version` alone carries installed, re-read
  on every request, not on `tree`)

**Rust side:** `crates/ngterm-proto/src/version.rs` (`installed: Option<AppIdentity>`, skipped when None; fix the
"exactly app and conformance" doc), `crates/ngterm-core/src/identity.rs` (create: read the installed identity),
`crates/ngterm-core/src/dispatch.rs` (`version()` re-reads it per request through a path in `CoreConfig`),
`crates/ngterm-gpui/src/app.rs` (give the core that path), `crates/ngterm-ctl/src/output.rs`, `scripts/stage.sh`
(write the identity file), `conformance/agterm_conformance/match.py` (`assert_version_result` allows `installed`),
`conformance/agterm_conformance/cli_format.py`, `conformance/tests/protocol/test_version.py`

**Platforms:** the installed identity is a file of the installed layout, read on every request from the running
executable's root (`std::env::current_exe()`, with Linux's " (deleted)" suffix stripped as
`live_runtime::reopen_target` does, since an upgrade replaces the binary under the running app):
- Linux: a staged metadata file (proposed `share/ngterm/app-BUILD`: a version line and a commit line), written by
  `scripts/stage.sh` after `bin/ngterm` and published by rename, as `session-BUILD` is; any desktop or distribution
  that installs the staged layout.
- macOS: inside an `ngterm.app` bundle, `Contents/Info.plist` (CFBundleShortVersionString and a commit key) through
  the `plist` crate, as upstream; outside a bundle, the staged file.
- Windows: the staged file beside `bin\ngterm.exe`.
The reader lives in ngterm-core (cross-platform CI); the frontend only passes the path.

**Conformance:**
- `match.py::assert_version_result` accepts `installed` (an object with a non-empty `version`) in the same change:
  without it the listed `test_version_reports_the_app_version`, `test_tree_app_is_the_version_identity` and
  `tests/cli/test_cli_output.py::test_version` go red
- new: `test_version.py::test_version_reports_the_installed_build_when_it_is_known` (covers `version`,
  `field:result.installed`: on the staged instance `installed == app`, `tree.app` has no `installed`),
  `test_version.py::test_installed_follows_the_file_on_disk` (a private stage copy: rewrite the file after launch,
  `installed` changes while `app` stays; delete it, `installed` is absent); a `test_cli_output.py::test_version`
  case with differing metadata expecting `<app>\ninstalled: <installed>\nclient: <path>`

- [x] `result.installed {version, commit?}` read from disk on every request (no cache), absent when unreadable or
      versionless; an empty or `unknown` commit is dropped; not on `tree`
- [x] ngtermctl `version`: `installed: <version (commit)>` only when present and different from `app`, before the
      `client:` line; `--json` stays the raw response
- [x] `scripts/stage.sh` writes the file from the build's recorded inputs (package version, `NGTERM_BUILD_COMMIT`)
- [x] write tests: Rust unit tests for the five AppIdentity cases (incl. an empty or malformed file), the CLI
      formatting cases (none, equal, other version, same version other commit, no commit), the core re-read while
      `app` stays
- [x] write tests: the conformance changes above (green on Linux in a targeted `run-linux.sh -- -k test_version`: 6 pass)
- [x] manual check (user): green on reference-macos; green on `cross-platform.yml` (the reader in ngterm-core) (skipped - not automatable here: left to the user)
- [x] gate passes; newly green ids added with `check_ratchet.py --add` (full suite deferred to the end of the plan per the user; the coverage gate's uncovered keys are all later tasks')

### Task 9: A second instance on an owned state directory exits 1 before reading or writing it (#749)

**Upstream:**
- implementation (commit `1d70d8c`): `agtermCore/Sources/agtermCore/StateDirectoryLock.swift` (new),
  `agterm/agtermApp.swift`, `agterm/Control/ControlServer.swift` (comment)
- tests: `StateDirectoryLockTests.swift` (acquireCreatesAMissingDirectoryAndOwnsIt, aSecondAcquireWhileTheFirstIsHeldIsRefused,
  releasingTheLockLetsTheNextAcquireOwnItAndKeepsTheFile, aDirectoryPathHeldByAFileIsUnavailableNotHeld)
- contract: `.claude/rules/control-api.md` (the socket-lock bullet now separates only instances on different state
  dirs); strings `cannot create \(directory.path): …`, `cannot open \(path): …`, `cannot lock \(path): …`

**Rust side:** `crates/ngterm-core/src/state_lock.rs` (create: `StateDirectoryLock::acquire` →
Owned/HeldByAnother/Unavailable), `crates/ngterm-core/src/lib.rs`, `crates/ngterm-gpui/src/main.rs` (claim after the
D-Bus registration, before `app::run`), README.md (the "with no bus between them, shares its windows.json" sentence
becomes the exit-1 rule), `conformance/tests/behaviors/test_state_dir_lock.py` (create), `conformance/catalog.yaml`
(a hand-kept behavior key `behavior:app.state-dir-owned`, kept by the generator like the other hand-written ones)

Today a second launch is stopped only by D-Bus forwarding (`crates/ngterm-gpui/src/main.rs`, `instance.rs`); with
no session bus, or a different `NGTERM_APP_ID` on the same `NGTERM_STATE_DIR`, a second primary runs, and macOS and
Windows have no GApplication bus. A pane's PATH holds ngterm's bin dir and ngterm parses no argv, so `ngterm
--help` in a pane launches the app.

**Platforms:** `std::fs::File::try_lock` on `<state dir>/ngterm.lock` (created, never truncated, never removed):
flock(LOCK_EX|LOCK_NB) on Linux and macOS, LockFileEx(EXCLUSIVE|FAIL_IMMEDIATELY) on Windows; `WouldBlock` =
HeldByAnother, any other error Unavailable; std opens it close-on-exec / non-inheritable, so shells, the session
server and the WPE worker never inherit it; the 0o600 mode behind `cfg(unix)`, as ngterm-server does. The code is
in ngterm-core, tested on all three CI OSes.

**Conformance:**
- new: `tests/behaviors/test_state_dir_lock.py::test_a_second_instance_on_an_owned_state_dir_exits_1_without_writing`
  (covers `behavior:app.state-dir-owned`: with A running, the staged `ngterm` on A's `NGTERM_STATE_DIR` with another
  `NGTERM_APP_ID` and control socket exits 1 within seconds, stderr "ngterm: another instance is already running on
  <dir>; control it with ngtermctl", `windows.json` and the window files byte-identical, A's tree unchanged),
  `test_the_lock_is_released_at_exit` (after A stops a launch succeeds; `ngterm.lock` still exists)

- [x] `main` claims the state dir after the D-Bus registration (a forwarded launch still raises the primary with
      exit 0) and before `app::run` (before `FirstRunWelcome::has_prior_state`, `load_state`, `ControlServer::new`
      or any save); held for the process
- [x] refusal: log, stderr, exit 1: "ngterm: another instance is already running on <dir>; control it with
      ngtermctl"; "ngterm: cannot claim the state directory: <detail>" with detail "cannot create <dir>: <err>",
      "cannot open <path>: <err>" or "cannot lock <path>: <err>"
- [x] decide and record the one write before the lock: `ngterm_app::logging::init` appends to `<state>/ngterm.log`
      first (upstream: "before it reads or writes anything"); move the lock before it or keep it with the reason
      (kept, recorded in `main.rs`'s module docs: a forwarded launch logs its forwarding and a refused one its refusal
      there, and the log only appends to a file another process holds, rotating only one whose own lock it gets)
- [x] keep ngterm-server's socket lock for instances on different state dirs; update its doc comment and README
- [x] write tests: Rust unit tests for the four Swift cases (a second `File` handle conflicts with flock and
      LockFileEx alike)
- [x] write tests: the conformance tests above (the harness's relaunch waits for the old pid's exit; confirm in
      `impls/base.py`) (confirmed: `_terminate_app` waits; the second launch goes through a new harness method,
      `Adapter.launch_beside`, since the isolation selftest keeps processes out of tests; green on Linux in a targeted
      `run-linux.sh -- -k test_state_dir_lock`: 2 pass)
- [x] manual check (user): green on reference-macos; green on `cross-platform.yml` (the lock in ngterm-core) (skipped - not automatable here: left to the user; `cargo clippy -p ngterm-core --target x86_64-pc-windows-msvc` is clean)
- [x] gate passes; newly green ids added with `check_ratchet.py --add` (full suite deferred to the end of the plan per the user; the coverage gate's uncovered keys are all later tasks')

### Task 10: `workspace flag on|off|toggle` and Flag All Sessions / Unflag All Sessions (#787)

**Upstream:**
- implementation (commit `14afbfd`): `ControlProtocol.swift`, `ControlDispatcher.swift`, `AppStore+SessionState.swift`,
  `ControlActionsDefaults.swift`, `agterm/Control/ControlServer+WorkspaceCommands.swift`, `ControlServer.swift`,
  `agterm/Views/WorkspaceSidebar+ContextMenu.swift`, `agtermctlKit/WorkspaceCommands.swift`
- tests: `AppStoreSessionStateTests.swift` (workspaceFlagSetsEverySessionToOneValue ×7,
  workspaceFlagOnAnEmptyOrUnknownWorkspaceChangesNothing, workspaceFlagFlagsSessionsTheFlaggedViewHides,
  workspaceUnflagInFlaggedModeReselectsAVisibleSession); `ControlDispatcherWorkspaceTests.swift` (all 4);
  `agtermctlKitTests/WorkspaceCommandsTests.swift` (all 3); `agtermTests/SidebarWorkspaceFlagTests.swift` (all 5);
  `agtermUITests/ControlSidebarStatusUITests.swift` (testWorkspaceFlagFlagsEverySessionAndReadsBackPerSession)
- contract: `site/commands.html` `workspace flag`; `.claude/rules/control-api.md`; `.claude/rules/sidebar.md`

**Rust side:** `crates/ngterm-core/src/commands/workspace.rs`, `crates/ngterm-core/src/model/store.rs`
(`all_sessions_flagged`, `apply_workspace_flag` over `set_flags`), `model/tests/organization.rs`,
`commands/tests/workspace.rs`, `crates/ngterm-core/src/sidebar_actions.rs`, `crates/ngterm-gpui/src/sidebar/menu.rs`,
`sidebar/context.rs`, `sidebar.rs`, `crates/ngterm-ctl/src/workspace.rs`

**Platforms:** no OS-specific code

**Conformance:**
- new: `tests/protocol/test_workspace.py::test_flag_sets_every_session_to_one_value` (covers `workspace.flag`,
  `field:tree.session.flagged`), `test_flag_covers_the_sessions_the_flagged_view_hides` (covers `workspace.flag`,
  `sidebar.mode`), `test_flag_rejects_an_unknown_mode` ("invalid workspace flag mode: clear"); append
  `("workspace.flag", {"mode": "on"})` to `test_unknown_workspace_target`'s list ("no such workspace: zzz");
  `tests/cli/test_cli_output.py::test_workspace_flag` (covers `cli:workspace flag`)

- [x] core: `all_sessions_flagged(workspace)` (false for an empty or unknown one); `apply_workspace_flag(mode,
      workspace)` sets every session, the flagged layout's hidden ones included, to
      `mode.desired_value(all_sessions_flagged)` through `set_flags` (toggle is a batch rule); unknown = no-op; an
      unflag in flagged mode reselects a visible session
- [x] dispatcher: mode parsed before the target resolves, none = toggle; "invalid workspace flag mode: <mode>";
      target `active` by default, "no such workspace: X"; result `{"id": <workspace>}`
- [x] sidebar row menu: after the Focus pair and its separator, "Flag All Sessions" or "Unflag All Sessions" (every
      session flagged, hidden ones included), then a separator, then Delete Workspace; disabled for an empty
      workspace; acts on the sidebar's own window's store
- [x] ngtermctl `workspace flag [on|off|toggle]` after `filter`, default toggle, `--target`, `--window`, "mode must
      be on, off, or toggle"
- [x] write tests: Rust unit tests for every Swift case listed; menu labels and enabled state
- [x] write tests: the conformance tests above
- [x] manual check (user): green on reference-macos (skipped - not automatable: reference-macos runs only in CI on macOS; left for the user)
- [x] gate passes; newly green ids added with `check_ratchet.py --add` (full suite deferred to the end of the plan per the user; the targeted run of the new and changed tests passed, 14 ids, their ratchet add happens in Task 35)

### Task 11: Mark for Attention on a session's row menu (#763)

**Upstream:**
- implementation (commit `395030d`; `8a07ee8` rewraps a rules line): `AppStore+Status.swift` (markForAttention),
  `agterm/Views/WorkspaceSidebar+ContextMenu.swift`
- tests: `AppStoreStatusTests.swift` (markForAttentionSetsAPlainCompletedOnAnIdleSession,
  markForAttentionSurvivesLeavingAndRevisitingTheSession, markForAttentionLeavesANonIdleSessionAlone ×3,
  markForAttentionMarksOnlyTheIdleMembersOfABatch); `agtermTests/SidebarMarkForAttentionTests.swift` (all 3)
- contract: `.claude/rules/notifications.md` (row menu only; a plain `completed`, no autoReset, untagged);
  `.claude/rules/control-api.md`, `.claude/rules/sidebar.md` (batch targets); no wire command

**Rust side:** `crates/ngterm-core/src/model/store.rs` (`mark_for_attention`), the status model tests,
`crates/ngterm-core/src/sidebar_actions.rs`, `crates/ngterm-gpui/src/sidebar/menu.rs`, `crates/ngterm-gpui/src/sidebar.rs`

**Platforms:** no OS-specific code

**Conformance:** none (a row-menu-only behavior; the suite drives no context menus)

- [x] core: `mark_for_attention(&[Uuid])` sets a plain `AgentIndicator { status: Completed, .. }` (no blink,
      auto-reset, color, shape or status pane) on each idle target through `set_agent_indicator` (the status event
      fires, `statusChangedAt` stamps); non-idle and unknown ids are skipped
- [x] row menu: when any target has a status, "Clear Status" (one) or "Clear Statuses"; when any is idle, "Mark for
      Attention"; one separator after them when the targets have any status, so a plain idle row opens with "Mark
      for Attention", "-", "Rename"…; update the existing menu.rs expectations
- [x] write tests: Rust unit tests for each Swift case, including that the mark survives a visit and that a right or
      scratch keystroke leaves it (an untagged status reads as the primary pane's: assert
      `clear_status_on_input` treats `status_pane: None` that way)
- [x] gate passes (full suite deferred to the end of the plan per the user; the task adds no conformance test)

### Task 12: Agent status in session pickers, drawn trailing; `pick` item status fields; `live tree` rows (#774)

**Upstream:**
- implementation (commit `dc1110b`): `ControlPick.swift`, `ControlDispatcher+Pick.swift`, `ControlPayloads.swift`
  (ControlRemoteSession), `RemoteTree.swift`, `agterm/AppActions+Palette.swift`, `AppActions+RemoteAttach.swift`,
  `agterm/Views/Palette.swift`, `SessionSwitcher.swift`, `WindowContentView+RecentSessions.swift`, `WindowContentView.swift`
- tests: `ControlDispatcherPickTests.swift` (openRejectsAnInvalidItemStatusWithoutCallingHost ×3, openPassesItemStatusThrough);
  `ControlPickTests.swift` (statusRoundTripsAndOmitsWhenNil, indicatorDropsWhatCannotRender ×5); `RemoteTreeTests.swift`
  (aSessionCarriesItsAgentStatusAndAnIdleOneCarriesNone, aSessionFromAnOriginWithoutStatusDecodesWithNone);
  `agtermctlKitTests/CommandsTests.swift`; `agtermTests/AppActionsPaletteTests.swift`
  (testASessionRowShowsTheStatusOfTheMomentUnderTheOpenPalette); `RemoteAttachTests.swift`
- contract: `site/commands.html` `pick` (status, statusColor, statusShape) and `zmx tree` (= `live tree`) rows;
  `.claude/rules/control-api.md`; `.claude/rules/menu-actions.md` (every session list draws the glyph trailing)

**Rust side:** `crates/ngterm-proto/src/request.rs` (`PickItem` gains `status`, `statusColor`, `statusShape` with
explicit renames: it has no `rename_all`), `crates/ngterm-proto/src/result.rs` (`ControlRemoteSession` gains them after
`panes`, `#[serde(default)]`), `crates/ngterm-core/src/commands/pick.rs`, `commands/tests/pick.rs`,
`crates/ngterm-core/src/remote/tree.rs`, `remote/attach.rs`, a PickItem→AgentIndicator conversion in
`crates/ngterm-core/src/model/status.rs`, `crates/ngterm-core/src/palette/feeds.rs`, `crates/ngterm-core/src/titlebar/mod.rs`,
`crates/ngterm-gpui/src/palette.rs`, `popovers.rs`, `session_switcher.rs`, `picker.rs`

**Platforms:** no OS-specific code

**Conformance:**
- append to `tests/protocol/test_ask_pick.py::test_pick_refusals`: status-unknown → "invalid pick item status
  (idle|active|completed|blocked)", status-color → "invalid pick item color (expected #rrggbb)", status-shape →
  "invalid pick item shape (circle|square|triangle|diamond|capsule|star)"
- new: `test_ask_pick.py::test_pick_accepts_item_status` (covers `pick.open`),
  `tests/protocol/test_zmx.py::test_tree_rows_carry_the_session_status` (LIVE, covers `zmx.tree`,
  `field:result.remote`, `session.status`: blocked/#ff8800/star on the row, omitted after idle)

- [x] proto fields as above (pick item order id, label, subtitle, status, statusColor, statusShape; None omitted;
      older payloads decode)
- [x] pick.open checks item status after the control-character check and before the selection check; first item
      with an error wins; per item status, then color, then shape; the value is never echoed
- [x] conversion: an unparsable or idle status gives the default indicator, an unknown shape none
- [x] `live tree` rows copy the far node's status fields; Attach Remote's pick items carry them
- [x] every session list draws the glyph trailing through `status_glyph::resolve` (Ctrl-Tab switcher from the live
      indicator, ⌃P palette rows and the attention palette, both title-bar popovers, the pick picker, Attach Remote);
      idle draws nothing; the palette follows a status change while open (confirm a status change re-syncs it)
- [x] write tests: Rust unit tests for every Swift case; palette and recent rows carry the indicator; ctl `parse_items`
      keeps the fields
- [x] write tests: the conformance tests above (the pick ones pass on a targeted `run-linux.sh`; the LIVE
      `test_tree_rows_carry_the_session_status` skips here, no `live-sessions` on this fish host until Task 27)
- [x] manual check (user): green on reference-macos (skipped - not automatable, runs only in CI on macOS)
- [x] gate passes (full suite deferred to the end of the plan per the user; the newly green ids are added there
      with `check_ratchet.py --add`)

### Task 13: `live tree` keeps a Live session whose split stayed hidden across a relaunch (#764)

**Upstream:**
- implementation (commit `1e2dc03`): `Session.swift` (`zmxBacking(for:)`, `allPanesBackedByZmx`)
- tests: `AppStoreTreeProjectionTests.swift` (hiddenUnrealizedSplitTakesThePrimarysBacking ×2,
  hiddenUnrealizedPrimaryTakesTheFocusedSplitsBacking, realizedUnbackedSplitStillClearsTheSessionBacking)
- contract: `.claude/rules/control-api.md` (backedByZmx = backedByLive: a pane hidden since launch reports its
  realized sibling's backing)

**Rust side:** `crates/ngterm-core/src/commands/relaunch.rs` (`Core::pane_backed`: an existing pane with an empty slot
answers with its sibling's backing; its doc comment), `crates/ngterm-core/src/projection.rs` (`all_panes_backed` in
line), `crates/ngterm-core/src/commands/tests/live.rs`, `conformance/tests/behaviors/test_relaunch.py`

**Platforms:** no OS-specific code

**Conformance:**
- existing: `tests/behaviors/test_relaunch.py::test_live_session_survives_the_relaunch` (listed)
- new: `test_relaunch.py::test_a_live_session_with_a_hidden_split_stays_attachable_across_the_relaunch` (LIVE,
  covers `behavior:restore.live-session-survives-relaunch`, `zmx.tree`, `field:result.remote`: split on, both backed,
  split off, relaunch; the tree's backed key is true and the `live tree` row lists both panes)

- [x] write the Rust test first (whether a woken session's hidden split gets a slot through `take_spawn_grants` is
      unconfirmed; confirmed: waking grants the main pane alone, the split's slot stays empty until laid out)
- [x] `pane_backed(session, role)`: a pane that exists with no surface in its slot answers with the sibling's
      backing (its realized attach, or its disposition when unrealized); a realized unbacked pane still clears the
      aggregate; the hidden pane's `backedByLive` reports the same
- [x] write tests: the three Swift cases and a `live tree` merge keeping the session
- [x] write tests: the conformance test above (needs `live-sessions`; skips on a targeted `run-linux.sh` here, no
      `live-sessions` on this fish host until Task 27)
- [x] manual check (user): green on reference-macos (skipped - not automatable, runs only in CI on macOS)
- [x] gate passes (full suite deferred to the end of the plan per the user; the newly green ids are added there
      with `check_ratchet.py --add`)

### Task 14: Status sounds play on the presenting machine: the `sound` frame and negotiated kinds (#775)

**Upstream:**
- implementation (commit `989df5f`): `PresentationFrames.swift` (PresentationSound), `PresentationHub.swift`
  (supportedKinds gains `sound`; each subscriber's negotiated kinds; the send gate), `AppStore+Presentation.swift`
  (presentStatusSoundRemotely), `RemotePresentationClient.swift`, `agterm/Control/ControlServer+SessionActions.swift`,
  `ControlServer+RemotePresentation.swift`, `StatusSoundPlayer.swift`
- tests: `AppStorePresentationTests.swift` (aStatusSoundFollowsOnlyTheLeadOfThePaneThatSetIt, aScratchStatusSoundStaysLocal,
  aRightPaneSoundWithoutASplitFollowsTheMainPaneLikeItsStatus, aSoundStaysLocalWithoutAPresenter,
  aPresenterTooOldForSoundLeavesItLocal, aReconnectSnapshotCarriesNoSound); `PresentationFramesTests.swift` (sound
  round trip); `PresentationHubTests.swift` (aNegotiatedKindReachesOnlyAPresenterThatAdvertisedIt);
  `RemotePresentationClientTests.swift` (aHandedOverSoundReachesTheAppAndNothingIsSentBack);
  `agtermTests/ControlServerRemotePresentationTests.swift` (testAHandedOverSoundPlaysHere,
  testTheBlockedMarkerPlaysThisMacsOwnBlockedSound, testASoundThisMacLacksPlaysTheSystemAlert,
  testTheBlockedMarkerWithNoBlockedSoundHereStaysSilent); `ControlServerStatusSoundTests.swift` (all 4)
- contract: `site/commands.html` `session status --sound`; `.claude/rules/control-api.md` (the sound bullet)

**Rust side:** `crates/ngterm-core/src/remote/frames.rs` (`Body::Sound(PresentationSound { name })`, kind and key
`sound`, a nil name `{}`), `crates/ngterm-core/src/remote/hub.rs` (`SUPPORTED_KINDS` += `sound` after `layout`;
`Subscriber` keeps its negotiated kinds; `send_to_presenter` refuses a negotiated kind the holder's hello omitted;
a `presenter_supports(kind, session)` query Tasks 18 and 22 reuse), `remote/client.rs` (`ClientEffect::Sound`),
`remote/viewer.rs`, `remote/handoff.rs` or `remote/mod.rs` (`present_status_sound_remotely`, reusing
`follows_remotely` and the status-pane mapping), `crates/ngterm-core/src/commands/metadata.rs` (`status()` routes
before the local play), `commands/tests/metadata.rs`, `commands/tests/remote.rs`, `conformance/tests/protocol/test_zmx.py`

**Platforms:** routing is OS-neutral (ngterm-core). Playback is the frontend's: Linux rings the display bell
(`status_sound.rs`: Wayland `xdg_system_bell_v1`, X11 core Bell) for every name; macOS NSSound by name with NSBeep for
default and a missing name; Windows `PlaySoundW` with the system alias and `MessageBeep(MB_OK)` for default and the
fallback. ngterm-gpui builds on Linux only, so the macOS and Windows playback are recorded as "not built on
macOS/Windows by CI until ngterm-gpui joins `cross-platform.yml`".

**Conformance:**
- new: `tests/protocol/test_zmx.py::test_status_sound_goes_to_a_following_presenter` (LIVE, covers `session.status`,
  `zmx.present`, `field:tree.session.presenters`, streams `zmx.present`: with the left pane following and a
  presenter whose hello lists `sound`: `blocked --sound Glass` → frame `{"name":"Glass"}`; idle then blocked → `{}`;
  staying blocked → none before a `session.context set` fence; a presenter without `sound` gets none; a reconnect
  while blocked gets no sound)

- [x] frames, hub and kept kinds as above (the hub's kind order follows upstream's final list, Context)
- [x] origin: `present_status_sound_remotely` is true only when a presenter holds the session, the status pane maps
      (none/left → primary; right → split, or the primary without a split; scratch stays local), that pane follows
      remotely, and the send succeeds
- [x] `session.status`: after a successful apply, when an explicit sound was given, or the status entered blocked
      (independent of the origin's own blocked setting), try the handoff with that name (none = the marker); handed
      over → no local play; a refused write sends nothing
- [x] viewer: plays the frame's name, or its own blocked setting for the marker; empty or none plays nothing; an
      unknown name logs and plays `default`; nothing is sent back
- [x] write tests: Rust unit tests for every Swift case (the lead matrix, scratch, right without a split, no
      presenter, older presenter, reconnect snapshot with no sound, the hub gate with an ask still sent, the client
      effect order, the viewer cases, `status()` with a follower, blocked marker once)
- [x] write tests: the conformance test above (LIVE: skipped on this host until Task 27 claims `live-sessions` on fish; not seen green locally)
- [x] manual check (user): green on reference-macos; green on `cross-platform.yml` (ngterm-core) (skipped - not automatable here: left to the user; the macOS/Windows playback in `ngterm-gpui/src/status_sound.rs` is not built on macOS/Windows by CI until ngterm-gpui joins `cross-platform.yml`)
- [x] gate passes; newly green ids added with `check_ratchet.py --add` (full suite deferred to the end of the plan per the user; the coverage gate's only uncovered keys are later tasks')

### Task 15: `ask --hotkey` letters resolved by keyboard layout (#771)

**Upstream:**
- implementation (commit `e73481c`): `Ask.swift` (`AskNavigation.resolvedHotkey`), `agterm/Views/AskDialogView.swift`
  (`AskKeyCatcher.key(for:)`)
- tests: `AskTests.swift` (hotkeyOnANonLatinLayoutResolvesByUSPosition, hotkeyOnALatinLayoutResolvesByTypedLetter)
- contract: `site/commands.html` `ask` ("A Latin layout matches the typed letter; any other layout matches the key
  that types it in the US layout."); `.claude/rules/control-api.md` (the keymap's per-layout `chordKey` rule)

**Rust side:** `crates/ngterm-gpui/src/ask_dialog.rs` (the hotkey key from the stroke through `chord_key`, not
`keystroke.key`), `crates/ngterm-gpui/src/terminal/input.rs` (expose a resolver beside `chord_key` if needed),
`crates/ngterm-gpui/src/ask_dialog/tests.rs`, `crates/ngterm-gpui/tests/keyboard_layout.rs`

The keymap's rule is already ported (`chord_key` in `terminal/input.rs`, fed by `current_source` from the patched
platform's native key record; the picker uses it in `picker.rs` `selection_step`); the ask dialog takes the letter
from `modal::nav_key`, which needs an ASCII letter in `keystroke.key`, so no hotkey fires on a Cyrillic layout.

**Platforms:** Linux (X11 and Wayland, any desktop): the native record's XKB keycode, level-0 character and
ASCII-capability, as the keymap; without a native record, GPUI's keystroke with `ascii_layout: true` (today's
behavior). macOS: ASCII-capability from `TISCopyCurrentKeyboardLayoutInputSource`'s `kTISPropertyInputSourceIsASCIICapable`,
the US letter from the virtual key code; Windows: ASCII-capability of the active HKL, the US letter from the scan
code (`MapVirtualKeyExW` with 0x0409). macOS and Windows come with the frontend's keyboard form on those OSes,
through the same `chord_key` entry the keymap and picker use (not built by CI until ngterm-gpui joins
`cross-platform.yml`).

- [x] the dialog resolves a hotkey through `chord_key(keycode, unshifted, ascii_layout)`, then only a single a..z
      letter counts; Ctrl/Alt/Super keys still go on to the window; Tab/arrows/Return/Esc/Space unchanged; the
      resolver never runs inside a text field (Task 17's fields) (`ask_dialog::ask_key(keystroke, source)` and
      `resolved_hotkey`; text an input method committed resolves as GPUI names the key, as before; macOS and Windows
      reach it through `current_source`, which gives no native record there yet: not built on macOS/Windows by CI
      until `ngterm-gpui` joins `cross-platform.yml`)
- [x] write tests: resolver unit tests with XKB keycodes: non-Latin O key (32) with "щ"/"Щ" → "o", Q key (24) with
      ";"/"/" → "q", the 1 key (10) with "&" → none, ISO key (94) → none; Latin: Q position typing "a" → "a", A key
      (38) with "A" → "a", "1", "/", "é", no character → none
- [x] write tests: `keyboard_layout.rs::an_ask_hotkey_on_a_cyrillic_layout_answers_by_key_position` (private Xvfb,
      XKB `ru`, `--hotkey yes=o`, physical O through XTEST, `ask.result` answered `yes`) (➕ then XKB `gr`,
      `--hotkey yes=q` on the Q key, which types `;`: GPUI's X11/Wayland key name already guesses the US letter for a
      key typing a non-ASCII character, so the `ru` half passed before the fix too; the Greek half is the one that
      fails without it)
- [x] gate passes (full suite deferred to the end of the plan per the user; the selftest
      `test_committed_catalog_passes_the_ci_gate` and `--coverage-gate` fail only on the 16 keys of Tasks 17, 20,
      21, 23, 25 and 29, as expected)

### Task 16: `ask` forms: wire, validation, CLI and the core draft model (#778, #779)

**Upstream:**
- implementation (commits `96b0a6c`, `9b60cdf`): `ControlAsk.swift`, `ControlProtocol.swift`,
  `ControlDispatcher+Ask.swift`, `Ask.swift`, `Session+Ask.swift` (new), `Session.swift`, `Pick.swift`,
  `agtermctlKit/AskCommands.swift`; `agterm/Control/ControlServer+Ask.swift` (only `ask.anchored(at:)`, a refactor)
- tests: `ControlDispatcherAskTests.swift` (formWithoutButtonsGetsOKAndCancelWithOKDefault,
  callerButtonsReplaceTheDefaultsAndCancelMayBeTheDefault, plainAskStillRequiresButtonsAndHasNoCancelRole,
  choicesReachHostWithSubtitlesAndCheckedIDs, otherPlaceholderReachesHostWithTheChoices, invalidOtherNeverReachesHost,
  invalidFormNeverReachesHost, controlCharactersOutsideTheirAllowedFieldsAreRejected,
  multilineValueAndSubtitleKeepTheirAllowedControls, valueIsCappedInUTF8Bytes, fiftyChoicesAreAcceptedAndFiftyOneRejected);
  `AskTests.swift` (the draft, answer and validated-answer cases: aPlainResultEncodesWithoutFormFields through
  validatedOtherAnswerFollowsTheFormsShape, 26 functions); `agtermctlKitTests/AskCommandsTests.swift`
  (rootRejectsInvalidSyntax new rows, oneShotResultPrintsEveryOutcomeAndMapsExit new rows,
  inputFormNeedsNoButtonsAndMapsItsOptions, choiceFormMapsItemsSubtitlesAndChecks, otherMapsItsPlaceholderBesideTheChoices,
  answerWithOtherTextPrintsTheSelectionAndTheText, formKeepsCallerButtons)
- contract: `site/commands.html` `ask` (synopsis `[--cancel ID] [--input line|multiline [--value TEXT] [--placeholder
  TEXT]] [--choice ID=LABEL ... [--choice-subtitle ID=TEXT ...] [--multiple] [--checked ID ...] [--other PLACEHOLDER]]`;
  wire args `cancelButton, input, value, placeholder, choices ({id,label,subtitle}), multiple, checked, other`; result
  `text`, `selected`); `.claude/rules/control-api.md` `ask.open` bullets

**Rust side:** `crates/ngterm-proto/src/request.rs` (`Args` gains `cancel_button`, `input`, `placeholder`, `choices:
Vec<AskChoice>`, `multiple`, `checked`, `other`; `AskChoice {id, label, subtitle?}`; `value` already exists and is
shared with `session.overlay.submit`), `crates/ngterm-proto/src/result.rs` (`ControlAskResult` gains `text`,
`selected` after `reason`, skipped when none), `crates/ngterm-core/src/model/ask.rs` (`AskInputMode`, `AskInputForm`
with `MAX_TEXT_BYTES = 65_536`, `AskChoiceForm` with `MAX_CHOICES = 50`, `AskForm`, `AskDraft`, `PendingAsk.form` and
`cancel_id`, `answer(index, draft)`, `validated_answer`), `crates/ngterm-core/src/modal.rs` (the window slot keeps a
GUI ask's draft), `crates/ngterm-core/src/commands/ask.rs`, `commands/tests/ask.rs`, a draft test module under
`crates/ngterm-core/src/model/tests/`, `crates/ngterm-ctl/src/ask.rs`

**Platforms:** no OS-specific code

**Conformance:**
- append to `tests/protocol/test_ask_pick.py::test_ask_refusals` (each asserting the exact error and no `ask` on the
  node): "input and choices are exclusive", "unknown input", "value requires input", "placeholder requires input",
  "checked requires choices", "multiple requires choices", "other requires choices" (with input and with buttons),
  "ask choices must be 1 to 50" (0 and 51), "ask choice label must not be empty", "ask choice ids must be unique",
  "ask checked ids must be unique", "unknown checked choice: b", "radio choices accept one checked id", "ask other
  placeholder must not be blank", "ask text must not contain control characters" (each field's forbidden control),
  "ask text too long (max 65536 bytes)", "unknown cancel button: nope", "ask input does not take hotkeys"
- new: `test_ask_pick.py::test_a_form_needs_no_buttons` (covers `ask.open`, `ask.cancel`, `field:tree.session.ask`,
  `field:result.ask`), `tests/cli/test_cli_output.py::test_ask_form_options` (covers `cli:ask`: an input ask with no
  `--button`; the CLI usage errors "--choice-subtitle requires --choice", "--choice-subtitle requires ID=TEXT",
  "unknown subtitle choice: b", "subtitle already set for choice: a", "--hotkey requires --button", "--other requires
  --choice"; `--input wide` refused by the app with "unknown input")

- [x] wire fields as above, camelCase; a plain result encodes as before
- [x] dispatcher order (upstream's `dispatchAskOpen`): title; then the form: "input and choices are exclusive";
      without input "value requires input", "placeholder requires input"; without choices "checked requires
      choices", "multiple requires choices", "other requires choices"; input: "unknown input", "ask text too long
      (max 65536 bytes)", "ask text must not contain control characters" (placeholder: any control; value: a line
      forbids every C0 and DEL, multiline allows \n and \t); choices: "ask choices must be 1 to 50", "ask choice
      label must not be empty", "ask choice ids must be unique", the control rule (label any, subtitle all but \n),
      "ask checked ids must be unique", "unknown checked choice: <id>", "radio choices accept one checked id";
      other: "ask other placeholder must not be blank", then the control rule
- [x] then buttons: a form without caller buttons gets `[ok=OK, cancel=Cancel]` with defaultButton ok and
      cancelButton cancel (only for that generated pair); a plain ask still answers "ask.open requires buttons";
      roles default, destructive, cancel ("unknown cancel button: <id>", "default button must not be destructive");
      then "ask input does not take hotkeys" (an input form only)
- [x] core `AskDraft` as upstream: text clamped to 65536 bytes at a char boundary; radio starts on the first checked,
      else the first item; the cursor on the first checked in caller order, clamped, the other row last; radio picks
      one, checkbox flips; a non-blank other edit selects the row, a blank one unselects it; a radio list restores the
      pick the row took over from; the row's marker toggles only while enabled; selected = caller ids in caller order;
      other text only while selected and non-blank
- [x] core `PendingAsk.answer(index, draft)`: the cancel button gives `escaped` with no payload; input answers
      `text`, choices answer `selected` plus the row's text; without a draft, the form's initial values;
      `validated_answer` for remote answers (Task 18)
- [x] the slot owner keeps the draft (session slot and window slot): created at open, kept while the dialog is
      hidden, edited and answered by ask id (a stale id changes nothing), dropped at release and resolve; a Core API
      for the frontend (update the draft; answer with a button from the draft) (`Core::update_session_ask_draft`,
      `answer_session_ask_button`, `update_window_ask_draft`, `answer_window_ask_button`; the GPUI dialog still
      answers through `PendingAsk::answer(index, None)` until Task 17 wires the draft)
- [x] ➕ a form or a cancel-role ask stays on the origin when its session is presented: no presenter lists `ask.form`
      yet (upstream's `needsFormSupport` gate without the capability; Task 18 replaces it with the capability and fit
      check) (`remote/handoff.rs`, test `a_form_or_cancel_role_ask_stays_here_for_a_presenter_without_the_capability`)
- [x] ngtermctl: `--cancel`, `--input`, `--value`, `--placeholder`, `--choice`, `--choice-subtitle`, `--multiple`,
      `--checked`, `--other` with upstream's help, the usage errors above in upstream's order, the answer printing
      `text`/`selected` with exit 0
- [x] write tests: Rust unit tests for every Swift case listed (full strings on the dispatcher's output, nothing
      reaching the slot); proto serde (plain result, form round trip); ctl parse, mapping and output
- [x] write tests: the conformance tests above
- [x] manual check (user): green on reference-macos (skipped - not automatable here: reference-macos runs only in CI on macOS; left for the user)
- [x] gate passes; newly green ids added with `check_ratchet.py --add` (full suite deferred to the end of the plan
      per the user; the selftest `test_committed_catalog_passes_the_ci_gate` and `--coverage-gate` fail only on the 16
      keys of Tasks 17, 20, 21, 23, 25 and 29, as expected; the new and changed tests ran green in a targeted
      `run-linux.sh -k` run)

### Task 17: `ask` forms: the GPUI dialog (#778, #779)

**Upstream:**
- implementation: `agterm/Views/AskDialogView.swift`, `agterm/Views/AskFormViews.swift` (new), `WindowContentView.swift`,
  `agterm/AppActions+Focus.swift`
- tests: `agtermTests/AskDialogViewTests.swift` (testInputAskFocusesItsEditorAtOpenAndKeepsInputOwnership through
  testTheOtherRowIsReachedAndScrolledIntoViewAfterFiftyItems, 31 functions); `agtermUITests/ControlAskUITests.swift`
  (testCLIChoiceKeysCheckItemsAndAnswerTheSelection)
- contract: `site/commands.html` `ask` keyboard paragraph and the other line; `.claude/rules/control-api.md` (input
  focus at open, nothing answers or dismisses while composing, the 64 KiB and control rule held before a change,
  multiline 5..15 lines, scrolling between the pinned title and buttons, drafts survive hiding and layout changes)

**Rust side:** `crates/ngterm-gpui/src/ask_dialog.rs`, `crates/ngterm-gpui/src/ask_dialog/form.rs` (create),
`ask_dialog/tests.rs`, `crates/ngterm-gpui/src/modal.rs` (Up/Down distinct from Forward/Backward, Space as toggle,
for this dialog only), `crates/ngterm-gpui/src/state.rs` (draft posts), `deck.rs`/`window.rs` (pass the owner's draft,
keep the editor's focus across remounts), the close-chord path (`crates/ngterm-core/src/app_actions.rs`
`dismiss_pending_modal` cannot see a composition: the frontend keeps the chord from reaching it while composing).
gpui-kit's `input::{Input, InputState, Textarea, TextareaState}` (already used in `selectable.rs`, `picker.rs`,
`search.rs`), `checkbox.rs` and `radio.rs` are the standard widgets.

**Platforms:** the field and editor are gpui-kit's on all three OSes; composition goes through GPUI's platform input
handler on each (Linux: text-input-v3 on Wayland, XIM on X11, feeding IBus/Fcitx; macOS: NSTextInputClient; Windows:
IMM32/TSF), and "composing" is the input state's marked range everywhere. The multiline submit chord is Cmd-Return on
macOS and Ctrl-Return on Linux and Windows (the Linux table's ⌘ → Ctrl rule); the close chord is the keymap's close
action per OS. Not built on macOS/Windows by CI until ngterm-gpui joins `cross-platform.yml`.

**Conformance:** none on Linux (the ui layer runs on the reference); ui ids `ask-input`, `ask-choice-<id>`,
`ask-other` arrive with Task 1's regeneration and are honored as element ids

- [x] an input ask draws its one-line field or multiline editor (placeholder while empty) between message and
      buttons, takes focus at open, and keeps input ownership as the dialog does
- [x] keys in the field: Tab/Shift-Tab move the button highlight; Esc escapes; one-line Return answers; multiline
      Return inserts a newline and the submit chord answers only while the editor is focused; nothing answers or
      dismisses while composing; `ask.cancel` and owner teardown still resolve
- [x] edits, pastes and compositions are held to 65536 UTF-8 bytes and the mode's control rule before they apply,
      refused whole otherwise, the text keeping its end (if gpui-kit has no pre-change hook, restore the previous text
      and selection exactly)
- [x] the multiline editor sizes to its laid-out text, wraps included, 5..15 lines (2 when the anchor is short); a
      field or list too tall scrolls between the pinned title and buttons; when even those do not fit, the panel
      scrolls
- [x] a choice list: radio or checkbox markers, dimmed multi-line subtitles, the cursor row marked; Up/Down cursor,
      Space toggles, Tab/Left/Right buttons, Return answers with `selected`; hotkeys still answer a list ask; opens
      scrolled to the checked item; the cursor row kept in view
- [x] the other row: a one-line field after the items, marker dimmed until its text is non-blank; it holds the
      keyboard only while the cursor is on it; Space is text there, Up returns above, Down clamps; a click moves the
      cursor there; letters there never fire hotkeys; its identity is tagged, never a caller id
- [x] the draft lives on the slot owner in core (Task 16): hiding, remounting and layout swaps keep it; answers go
      through core's answer-by-button from the draft; a replacement editor takes focus only when focus fell back to
      the window
- [x] the close chord with a composing editor keeps the ask (terminal and GUI ask)
- [x] element ids `ask-input`, `ask-choice-<id>`, `ask-other`
- [x] ➕ take those three ids off `PENDING` in `crates/ngterm-gpui/src/ui_ids.rs` (Task 1 parked them there; the
      ui-ids test fails while a pending id is carried)
- [x] write tests: GPUI window tests (headless, lavapipe) for the AskDialogViewTests cases: focus at open; typing
      updates the draft; one-line Return, Tab to Cancel then Return → `escaped` without text; Esc; multiline Return
      and the submit chord; the cap and the forbidden-control paste; composing keeps the ask; the draft survives an
      unmount and a layout swap; list keys (upstream's UI test: `--multiple a,b,c`, Down Space Down Space Return →
      `["b","c"]`); a plain ask's Up/Down still move buttons; the long list opens on its checked item; the other row
      cases; after 50 items the row is reached and scrolled into view
- [x] ➕ write tests: the reference-only ui test `tests/ui/test_dialogs.py::test_ask_form_fields_answer_with_their_input`
      (covers `ui:ask-input`, `ui:ask-choice-{id}`, `ui:ask-other`; the ui layer skips on Linux)
- [x] ➕ manual check (user): green on reference-macos (skipped - not automatable here: reference-macos runs only in CI on
      macOS; left for the user)
- [x] gate passes (full suite deferred to the end of the plan per the user; the selftest
      `test_committed_catalog_passes_the_ci_gate` and `--coverage-gate` fail only on the keys of Tasks 20, 21, 23, 25
      and 29, as expected)

### Task 18: `ask` forms on a remote presenter: `ask.form` / `ask.other`, frame fit, validated answers (#778, #779)

**Upstream:**
- implementation: `AppStore+RemoteAsk.swift`, `AppStore+RemotePresentation.swift`, `PresentationFrames.swift`,
  `PresentationHub.swift`
- tests: `AppStoreRemoteAskTests.swift` (aFormOrCancelRoleAskStaysHereForAPresenterWithoutTheCapability,
  aFormAskGoesToAPresenterListingTheCapability, aFormAskWhoseFrameIsTooLargeStaysHere,
  anInputAskStaysHereWhenItsLongestButtonAndTheReservedTextCannotFitAnAnswer,
  anOtherRowAskStaysHereUnlessThePresenterListsBothCapabilities, anOtherRowAskGoesToAPresenterListingBothCapabilities,
  aCheckboxOtherAskReservesEveryIDPlusTheWidestText, slashesInAButtonIDCountAsTheCodecWritesThem,
  anAnswerThatFitsOnlyWithShortCountersIsRefused, aFormAnswerIsAppliedWithItsPayload,
  aFormAnswerWithAPayloadItsFormWouldNotProduceIsRefused, aPlainAskAnswerCarryingAPayloadIsRefused);
  `PresentationHubTests.swift` (presenterSupportReflectsTheHelloOfTheCurrentPresenterOnly); `PresentationFramesTests.swift`
  (aChoiceRequestFromAnOriginWithoutTheOtherRowDecodesWithNone); `RemotePresentationStateTests.swift`
  (aFormReplicaSendsItsDraftWithTheAnswerAndTheFrameSurvivesTheCodec,
  aChoiceReplicaSendsTheSelectionAndItsCancelButtonSendsNoButton, anOtherRowReplicaSendsItsTextAndTheOriginAppliesIt)
- contract: `site/commands.html` remote-presentation paragraph; `.claude/rules/control-api.md` presenter bullet

**Rust side:** `crates/ngterm-core/src/remote/frames.rs` (`PresentationAsk` gains `input`, `choices`, `cancelID`;
`form()`, `needs_form_support()`, `needs_other_support()`, `fits_frames()`; `PresentationAskAnswer` gains `text`,
`selected`), `remote/hub.rs` (`ask.form`, `ask.other` after `sound` in `SUPPORTED_KINDS`; `presenter_supports` from
Task 14), `remote/handoff.rs` (gate on the capability and the fit before reserving; pass the payload),
`crates/ngterm-core/src/model/ask.rs` (`resolve_remote_ask` through `validated_answer`), `remote/viewer.rs` (the
replica carries form and cancel role, its answer sends the draft), `remote/client.rs`, tests in
`commands/tests/remote.rs`, `commands/tests/viewer.rs`, `remote/hub.rs`, `remote/frames.rs`

**Platforms:** no OS-specific code

**Conformance:** none (two-instance presentation is covered by Rust unit tests on the hub, handoff and viewer)

- [ ] frames: `ask.request` carries `input {mode, value, placeholder?}`, `choices {choices, multiple, checked,
      other?}` and `cancelID`; a request without `other` decodes with none; `ask.resolve` carries `text`, `selected`
- [ ] the viewer lists `ask.form`/`ask.other` in its hello only once its dialog draws forms (Task 17 done; this task
      runs after it)
- [ ] handoff: an ask with a form or a cancel role goes to the presenter only when it listed `ask.form` and the ask
      fits; a list with an other row also needs `ask.other`; otherwise it stays here and nothing is reserved or sent
- [ ] `fits_frames`: the request under the limit at the widest counters (generation and rev at i64::MAX); an input
      ask also needs every button's answer with 65536 `"` characters of text to fit; an other-row list every button's
      answer with that text and, for checkboxes, every id selected. Measure through `frames::encode` (serde_json does
      not write `/` as `\/` as Swift's encoder does: the slash test asserts this build's encoder)
- [ ] origin: a presenter's answer goes through `validated_answer`; a payload its form would not produce is refused
      and the ask stays pending; a dismissal or a plain ask's answer carrying a payload is refused
- [ ] viewer: the replica keeps form, cancel role and its own draft; the cancel button sends no button; a handback
      restarts from the initial values on the origin
- [ ] write tests: Rust unit tests for every Swift case listed
- [ ] manual check (user): green on `cross-platform.yml` (ngterm-core)
- [ ] gate passes

### Task 19: `session overlay open --url --chromeless` for loopback hosts (#750)

**Upstream:**
- implementation (commit `2849999`): `HtmlOverlay.swift` (`HtmlSource.webURL`, `HtmlSource.isLoopback`,
  `HtmlNavigationPolicy.decide`), `ControlDispatcher+Overlay.swift` (the URL branch), `ControlProtocol.swift`
  (OverlayHtmlError), `agtermctlKit/SessionCommands.swift`
- tests: `ControlDispatcherOverlayTests.swift` (the chromeless rows, urlOpenRoutesAChromelessLoopbackPage);
  `HtmlOverlayTests.swift` (chromelessPagePolicyKeepsFramesOnItsOrigin, isLoopbackMatchesOnlyTheExactLoopbackHosts);
  `agtermctlKitTests/OverlayCommandsTests.swift` (urlOpenSendsChromelessForALoopbackHost,
  aMalformedChromelessUrlIsLeftForTheServerToName); `agtermUITests/ControlHtmlOverlayUITests.swift`
- contract: `site/commands.html` `session overlay open --url` (`[--navigation | --chromeless]`); `.claude/rules/control-api.md`

**Rust side:** `crates/ngterm-core/src/html/mod.rs` (`is_loopback` beside `web_url`), `html/navigation.rs`
(`decide`), `crates/ngterm-core/src/commands/overlay.rs` (`CHROMELESS_REQUIRES_FILE` and the URL branch),
`crates/ngterm-ctl/src/session_overlay.rs`, `commands/tests/overlay.rs`, `crates/ngterm-gpui/src/html_overlay/policy.rs`
(a strip test for a chromeless URL page), `conformance/tests/protocol/test_overlay.py`

**Platforms:** no OS-specific code (the host check and frame policy are core's; every page host calls `decide`; the
macOS and Windows page hosts do not exist yet and will reuse it)

**Conformance:**
- listed ids whose expectation changes in this change (keep the param ids): `test_open_refusals[chromeless-program]`
  ("session.overlay.open: --chromeless requires --html or a loopback --url"), `[chromeless-url]` (its URL
  `http://127.0.0.1:9/page.html` is loopback and now opens: move the refusal case to a non-loopback host with
  "session.overlay.open: --chromeless with --url requires a loopback host: 127.0.0.1, ::1 or localhost" and keep the
  id), `[chromeless-browse-url]` ("session.overlay.open: --chromeless cannot be combined with --browse")
- new params appended: other loopback `127.0.0.2`, short `127.1` (both the loopback-host error), url+navigation
  ("session.overlay.open: --chromeless cannot be combined with --navigation"), `localhost:5173` ("session.overlay.open:
  --url must be an absolute http or https URL")
- new: `test_overlay.py::test_chromeless_loopback_url_page_opens` (over 127.0.0.1, [::1], localhost; covers
  `session.overlay.open`, `field:tree.session.htmlOverlays`), `test_overlay.py::test_chromeless_url_page_keeps_frames_on_its_origin`
  (two loopback servers; a cross-origin iframe loads without `--chromeless` and is not requested with it),
  `tests/cli/test_cli_output.py::test_chromeless_url_refusals` (covers `cli:session overlay open`)

- [ ] `is_loopback(text)`: true only for a well-formed web URL whose host, as written, lowercased and with IPv6
      brackets stripped, is exactly 127.0.0.1, ::1 or localhost. Compare the raw host text, never `Url::host`
      (WHATWG normalizes 127.1, 2130706433, 0x7f.0.0.1 to 127.0.0.1, which Foundation keeps as written)
- [ ] URL branch order: command+url, wait, cwd, then chromeless+navigation, chromeless+browse, the invalid URL, then
      the non-loopback host (the strings above)
- [ ] navigation policy: same origin or browse → Allow; a subframe → Allow only when not chromeless; otherwise
      OpenExternal if clicked, else Cancel (about: still Allow first)
- [ ] ngtermctl: "--chromeless requires --html or a loopback --url" (with a COMMAND), "--chromeless cannot be
      combined with --navigation", "--chromeless cannot be combined with --browse", the loopback-host error only for
      a well-formed URL (a malformed one goes to the server); help as upstream with ngterm names
- [ ] write tests: the `is_loopback` table (true: 127.0.0.1, https 127.0.0.1, [::1], localhost, LOCALHOST; false:
      127.0.0.2, 127.1, 2130706433, 0.0.0.0, [::ffff:127.0.0.1], localhost., app.localhost, localhost.example.com,
      example.com, ftp://127.0.0.1, "localhost:5173", ""); the chromeless policy table for a page at
      http://127.0.0.1:8792/; the dispatcher rows; the ctl argv lists
- [ ] write tests: the conformance changes above
- [ ] manual check (user): green on reference-macos
- [ ] gate passes; newly green ids added with `check_ratchet.py --add`

### Task 20: Hide and show overlays: wire, core model, read-back, CLI, keymap, palette and settings (#790)

**Upstream:**
- implementation (commit `7f450d1`): `Session+OverlayVisibility.swift` (new), `AppStore+OverlayVisibility.swift`
  (new), `Session.swift`, `Session+HtmlOverlay.swift`, `AppStore+HtmlOverlay.swift`, `AppStore+Panes.swift`,
  `AppStore.swift`, `ControlProtocol.swift`, `ControlDispatcher.swift`, `ControlDispatcher+Overlay.swift`,
  `ControlActionsDefaults.swift`, `ControlProjection.swift`, `HtmlBridge.swift`, `DashboardCover.swift`,
  `PaneBackgrounds.swift`, `TerminalZoom.swift`, `BuiltinAction.swift`, `Keymap.swift`, `PaletteCatalog.swift`,
  `AppSettings.swift`; `agtermctlKit/Session+OverlayVisibility.swift` (new), `SessionCommands.swift`,
  `SocketClient.swift`; `agterm/Control/ControlServer+OverlayVisibility.swift` (new), `ControlServer+SessionActions.swift`,
  `ControlServer+Hud.swift`; `agterm/AppActions+Overlay.swift` (new), `AppActions.swift`
- tests: `AppStoreOverlayVisibilityTests.swift` (all 18), `SessionOverlayVisibilityTests.swift` (all 11),
  `ControlDispatcherOverlayTests.swift` (visibilityCommandsRouteModePaneTargetAndWindow,
  visibilityCommandsRejectAnInvalidPaneBeforeTheHost, visibilityFailuresMapToTheirMessages),
  `AppStoreTreeProjectionTests.swift` (hiddenOverlaysReportHiddenSlotsWhileOccupancyStays,
  aSessionNodeFromAnOlderServerDecodesWithNoHiddenOverlays), `AppStorePaneSwapTests.swift`
  (aHiddenPaneOverlayStaysHiddenOnItsNewPane), `DashboardCoverTests.swift` (aHiddenOverlayCoversNothing),
  `PaneBackgroundsTests.swift` (backdropRegionsSkipAHiddenPaneOverlaysBackground), `TerminalZoomTests.swift`
  (aHiddenOverlayIsNeitherTheTargetNorAvailable), `HtmlBridgeTests.swift`, `KeymapTests.swift`
  (toggleOverlayShipsOnCmdShiftJAndCanBeRemapped, anExistingCmdShiftJBindingIsNotBrokenByTheToggleOverlayDefault),
  `BuiltinActionTests.swift`, `PaletteCatalogTests.swift` (toggleOverlayNeedsAnOverlayToHideOrShow), `AppSettingsTests.swift`
  (overlayToggleIsAHiddenByDefaultTitleBarElement), `agtermctlKitTests/OverlayCommandsTests.swift`
  (visibilityCommandsSendTheirCommandPaneAndTarget, visibilityHelpStatesTheSessionScopeAndTheToggleRule),
  `SocketClientTests.swift` (formatTreeTagsAHiddenSessionOverlay), `agtermTests/ControlServerOverlayVisibilityTests.swift`
  (all 6, ported to core command tests), `agtermTests/AppActionsOverlayTests.swift` (all 5)
- contract: `site/commands.html` `session overlay hide|show|toggle [--pane left|right]`, the tree's `hiddenOverlays`
  and `htmlOverlays[].hidden`; `site/docs.html`; `.claude/rules/control-api.md`, `keymap.md`, `menu-actions.md`,
  `settings.md`

**Rust side:** `crates/ngterm-proto/src/command.rs` (`OverlayHide/Show/Toggle` from Task 1), `crates/ngterm-proto/src/tree.rs`
(`ControlSessionNode.hidden_overlays` last, after `html_overlays`; `ControlHtmlOverlayNode.hidden` after `id`),
`crates/ngterm-core/src/model/overlay_visibility.rs` (create), `model/overlay.rs` (`OverlaySlot.hidden`, cleared in
close), `model/pane_overlay.rs`, `model/mod.rs`, `model/html_overlay.rs`, `model/background.rs`,
`crates/ngterm-core/src/dashboard.rs`, `zoom.rs`, `commands/overlay.rs`, `commands/overlay_page.rs`, `commands/hud.rs`,
`projection.rs`, `html/bridge.rs` (`OWN_OVERLAY` 4 → 7), `keymap/builtin.rs`, `keymap/defaults.rs`, `keymap/linux.rs`,
`palette_catalog.rs`, `persist/settings.rs`, `settings_model.rs`, `titlebar/mod.rs`, `app_actions.rs`, their tests,
`crates/ngterm-ctl/src/session_overlay.rs`, `crates/ngterm-ctl/src/tree.rs`, `crates/ngterm-ctl/tests/cli.rs`,
`conformance/tests/protocol/test_overlay.py`, `tests/protocol/test_keymap_hooks.py`, `tests/cli/test_cli_output.py`

**Platforms:** the model, dispatcher, CLI and keymap carry no OS-specific code. The chord: macOS follows upstream's
`cmd+shift+j`; Linux and Windows follow the Linux table's convention (⌘ → Ctrl+Shift, Alt added where the letter is
taken: upstream ⌘J toggle_scratch is ctrl+shift+j), so ⇧⌘J is `ctrl+alt+shift+j`. It shadows Ghostty's Linux default
`ctrl+shift+alt+j` write_screen_file:open (`ghostty_config/binds/defaults.rs`), as upstream's ⇧⌘J shadows
write_screen_file:paste; on Windows it is AltGr+Shift+J where AltGr = Ctrl+Alt, the same exposure the existing
`ctrl+alt+shift+t/o` defaults have.

**Conformance:**
- new in `test_overlay.py`: `test_hide_show_and_toggle_keep_the_overlays_running` (covers `session.overlay.hide`,
  `.show`, `.toggle`, `field:tree.session.hiddenOverlays`: a split session with a session-wide program printing a
  marker and a right pane overlay; hide → text "hidden", `hiddenOverlays == ["session","right"]`, overlay and
  paneOverlays unchanged, `session.overlay.text` still reads the marker, result "overlay still running"; show →
  "shown", field absent; toggle `--pane right` → `["right"]`; bare toggle hides all, the next shows all),
  `test_visibility_refusals` ("no overlay" on an empty session, `--pane left` with only a right overlay, a HUD alone),
  `test_opening_into_a_hidden_slot_is_refused` ("overlay hidden" for a program and a page into a hidden session
  slot, a hidden left pane refusing `--pane left` while a session-wide open succeeds, a HUD over a hidden session
  program; a HUD over a hidden pane overlay opens; after close a reopen starts shown), `test_a_hidden_page_reads_back_hidden`
  (`htmlOverlays[0].hidden` true while hidden, same id, absent after show); append hide/show/toggle to
  `test_unknown_session_target` and `test_overlay_pane_selector_refusals` (append, never insert)
- `tests/cli/test_cli_output.py::test_session_overlay_visibility` (covers the cli keys: `hidden`/`shown`, `--pane
  scratch` refused before the server, `tree` tags " (overlay hidden)" only for a hidden session slot)
- `tests/protocol/test_keymap_hooks.py`: the builtins listing includes `toggle_overlay` on `ctrl+alt+shift+j`; new
  `test_a_file_binding_the_toggle_overlay_chord_keeps_it` (`map ctrl+alt+shift+j toggle_workspace_filter` leaves
  toggle_overlay unbound, no diagnostic)

- [ ] wire+dispatcher: parse the pane first ("session.overlay: --pane must be left or right"), resolve the session,
      apply the mode, answer `{id, text}` with "shown" while any slot in scope is still shown, else "hidden";
      "no overlay" for an empty scope (a HUD, an ask and empty panes are not hideable); "overlay is shown on another
      Mac" when the empty scope holds a remote reservation
- [ ] opening into a hidden slot answers "overlay hidden" (after the session resolves, before page/remote routing,
      in `overlay.rs::open` and `overlay_page::open`; a session-wide open checks the session bit, `--pane` that
      pane's); `session.hud.open` checks the session bit after "no such session"; a hidden pane overlay does not
      block a HUD
- [ ] model: `OverlaySlot.hidden` and `PaneOverlay.hidden`, cleared on close (a new occupant starts shown), kept by a
      page update, a `--wait` exit and a replica update, carried through swap and promote; the visible twins
      (`cover_overlay_visible`, `program_overlay_visible`, `html_overlay_visible`, `full_overlay_visible`,
      `pane_overlay_visible`, `visible_cover()`), `hideable_overlay_slots` ordered session, left, right,
      `hidden_overlay_names()`; store `set_overlays_hidden`, `toggle_overlays_hidden` (hide all while any shown, else
      show all), `hide_html_overlay(page)` with the page's scope; tree.changed only on an actual transition
- [ ] switch the "covers or owns input" sites to the visible layer, occupancy staying for the rest: focus routing,
      `html_hides_terminal`/`topmost_html_overlay` (font commands), `dashboard_cover`, `pane_wash_color_hex`, zoom
      (`zoom.rs`: a hidden overlay is neither target nor valid, so hiding a zoomed one ends the zoom), the split and
      search gates, the ⌘W ladder (session slot only for a HUD or a visible cover, then the scratch, then the focused
      pane's visible overlay; only hidden overlays → ⌘W closes the session), `drop_unrealized_pane_overlays` keeps a
      hidden overlay on a rendered pane. Occupancy stays for tree `overlay`/`paneOverlays`, result/--block,
      text/copy/navigate, page lookup and the remote local-hold
- [ ] tree read-back: `hiddenOverlays` (omitted when empty) and `htmlOverlays[].hidden` (only `true`); older
      servers decode without them
- [ ] HTML bridge: the three commands join `OWN_OVERLAY`
- [ ] ngtermctl `session overlay hide|show|toggle [--pane left|right]` after `close`, with upstream's abstracts and
      discussions (agtermctl → ngtermctl); `tree` text tags " (overlay hidden)" when `hiddenOverlays` holds "session"
- [ ] keymap: `BuiltinAction::ToggleOverlay` "toggle_overlay" after toggle_terminal_zoom (50 actions); upstream table
      `cmd+shift+j`, Linux table `ctrl+alt+shift+j` (in `linux_vacated`); a file already binding the chord keeps it
      and leaves toggle_overlay unbound; an explicit `map … toggle_overlay` opts in
- [ ] palette `ToggleOverlay` "Toggle Overlays" after Toggle Scratch (57), enabled with an active session holding a
      hideable overlay; settings `InterfaceElement::OverlayToggle` "overlayToggle" after Split, hidden by default;
      the title-bar model gains the overlay toggle {enabled, any_shown, any_hidden}
- [ ] the app action and the ⌘W/edit-config/close-confirmation logic in `app_actions.rs` (the GPUI side is Task 21)
- [ ] write tests: Rust unit tests porting every Swift case listed (store, session predicates, dispatcher routing
      and failures, tree, swap, dashboard, backdrop, zoom, bridge, keymap on both tables incl. the vacate cases and
      the Linux defaults count, BuiltinAction count, palette, settings, the six ControlServer cases as core tests,
      the five app-action cases); ngterm-ctl request, help and tree-tag cases
- [ ] write tests: the conformance tests above
- [ ] manual check (user): green on reference-macos
- [ ] gate passes; newly green ids added with `check_ratchet.py --add`

### Task 21: Hide and show overlays: the GPUI side (#790)

**Upstream:** `agterm/AppActions+Batch.swift`, `AppActions+Focus.swift`, `AppActions+Palette.swift`,
`Commands/CustomCommandRunner.swift`, `agtermApp+Menus.swift`, `agtermApp.swift`, `Views/HtmlOverlayView.swift`,
`Views/SettingsView.swift`, `Views/WindowContentView+Detail.swift`, `Views/WindowContentView+Titlebar.swift`; tests
`agtermUITests/ControlOverlayVisibilityUITests.swift` (all 6), `SettingsUITests.swift` (testInterfaceTabFitsWithTheOverlaysToggle)

**Rust side:** `crates/ngterm-gpui/src/split.rs`, `overlay.rs`, `deck.rs`, `mute.rs`, `html_overlay/policy.rs`,
`html_overlay/panel.rs`, `app_menu.rs`, `titlebar.rs`, `settings/interface.rs`, `sidebar.rs` (close confirmation),
`window.rs` (the Command-W path), their tests

**Platforms:** the alert bell for a refused edit: Linux the existing `status_sound` bell (Wayland
`xdg_system_bell_v1`, X11 core Bell), macOS NSBeep, Windows `MessageBeep(MB_OK)`, each with that OS's frontend
(not built on macOS/Windows by CI until ngterm-gpui joins `cross-platform.yml`)

- [ ] deck: a hidden program, pane overlay or page stays mounted (its terminal entity, IO thread and WPE page alive)
      with no placement; its pane shows and takes clicks and keys; the backdrop wash and mute follow the visible
      layer; nothing is rebuilt on show; the refocus fires when the visible cover changes, never on a HUD, a hidden
      overlay's close or the other pane's overlay. Confirm that mounting follows occupancy, not placement, so
      `drop_unrealized_pane_overlays` never tears a hidden one down, and that a WPE page keeps loading and running
      while unplaced
- [ ] the page strip gains Hide before Close (minus icon, label "Hide", id `htmlOverlay.hide`) with the page's scope
- [ ] View menu row after Toggle Scratch: "Hide Overlays", or "Show Overlays" when nothing is shown; enabled through
      the palette fact; chord from toggle_overlay
- [ ] title bar: the overlay toggle after split, counted for the dividers; disabled with nothing to hide or under a
      picker, zoom or dashboard; label Hide/Show Overlays, a slashed icon while anything is hidden, id
      `overlay-toggle`
- [ ] ➕ take `overlay-toggle` off `PENDING` in `crates/ngterm-gpui/src/ui_ids.rs` (Task 1 parked it there)
- [ ] Settings ▸ Interface: "Hide overlays", off by default; Multiple Windows becomes the Sidebar section's last row
      as upstream (grouping parity; Settings stays the in-window popup)
- [ ] close confirmations append " Its hidden overlays close with it." (one session) or " Their hidden overlays
      close with them." (several) when any closing session has hidden overlays
- [ ] the alert bell when Edit Keymap/Hooks/ghostty.conf is refused because a hidden overlay holds the session slot;
      the overlay stays hidden
- [ ] the toggle_overlay chord fires from a focused page (the interceptor runs the keymap before the page's engine:
      a test, no special case)
- [ ] write tests: GPUI window tests (headless, lavapipe) mirroring ControlOverlayVisibilityUITests: keys go to the
      pane after hide and back to the same program after show (one pid); a hidden pane overlay leaves its pane
      clickable; a hidden page comes back with the same load token; the title-bar button's states; the chord from
      the terminal and from a focused page; the strip's Hide hides only its own pane
- [ ] gate passes

### Task 22: HTML pages on a remote presenter, part 1: the origin side (#793)

**Upstream:**
- implementation (commit `4883f9b`): `PresentationFrames.swift`, `PresentationHub.swift`, `RemoteOverlays.swift`,
  `AppStore+RemotePage.swift` (new), `AppStore+RemoteOverlay.swift`, `AppStore+HtmlOverlay.swift`,
  `AppStore+OverlayVisibility.swift`, `Session+OverlayVisibility.swift`, `HtmlOverlay.swift`, `ControlProtocol.swift`
  (OverlayHtmlError), `ControlProjection.swift`, `agterm/Control/ControlServer+SessionActions.swift`,
  `ControlServer+SurfaceIO.swift`, `ControlServer+Ask.swift` (the presenter-frame arm)
- tests: `AppStoreRemotePageTests.swift` (the origin cases: aPageOpensOnAPresenterThatSpeaksPages,
  aPresenterWithoutPageKindsKeepsTheRefusal, aSessionWidePageWithMixedRolesStaysHere,
  aLoopbackUrlIsRefusedOnlyWhenThePageWouldGoToThePresenter, aRequestOverOneFrameIsRefusedBeforeAnythingChanges,
  aTakenSlotAndAMissingPaneAreRefused, theMirroredStateIsReportedAndOnlyAHiddenChangeTouchesTheTree,
  aPageReportFromAMirrorIsIgnored, closingSendsTheCloseAndDismissesThePage, aSubmitRecordsTheAnswerAndALaterCloseKeepsIt,
  aRejectionEndsThePageDismissed, losingThePresenterEndsThePageAndKeepsAnAcceptedAnswer, removingTheSessionEndsItsPage,
  twoTogglesBeforeAnyReportHideThenShow, aStaleReportDoesNotOverrideANewerRequest,
  aHideMadeOnThePresenterTurnsTheNextToggleIntoAShow, aRemoteProgramStillRefusesToHide, resizeReachesThePresenterAsACommand,
  aFilePageGoesBackAndForwardButFinderAndBrowserAreRefused, finderOnAUrlPageGivesTheLocalAnswer, reloadReachesThePresenter,
  aSlotWithoutARemotePageIsLeftToThisMac, aReservationWhoseStreamIsGoneRefusesWithoutReopening,
  aPageThePresenterLossEndedLeavesNoOverlay, aUrlPageNavigationReachesThePresenter);
  `PresentationFramesTests.swift` (the page.* round trips, pageStateBoundsThePagesOwnStrings);
  `HtmlOverlayTests.swift` (aLocalPageNodeOmitsTheRemoteFields, openIsRefusedWhenAFollowingPaneBelongsToAPresenterWithoutPages);
  `agtermTests/ControlServerPageServeTests.swift` (testARemoteOpenSkipsTheLocalPageFollowAndStore,
  testALoopbackUrlForThePresenterIsRefused, testAPageTooLargeForOneFrameIsRefused,
  testReloadAndNavigationOfARemotePageReachThePresenter, testAResultOrReadOfARemotePageGivesThePageAnswers)
- contract: `site/commands.html` `session overlay open --html` and the htmlOverlays read-back (`id, hidden?, remote?,
  replica?`), `remoteOverlays {pane?, sizePercent?, pageID?}`, the remote-sessions paragraph; `.claude/rules/control-api.md`
  Remote sessions bullets

**Rust side:** `crates/ngterm-core/src/remote/frames.rs`, `remote/hub.rs` (the six page kinds after `ask.other`; a
page report accepted only from the grant holder), `remote/page.rs` (create: the origin half), `remote/handoff.rs`
(page arms, presenter loss), `remote/mod.rs`, `model/remote_overlay.rs` (a RemotePage in the slot),
`crates/ngterm-core/src/html/mod.rs` (`HtmlSource::Replica`, `replica_of`, the scheme name), `model/html_overlay.rs`,
`commands/overlay_page.rs` (open routing, submit, reload, navigate; the blanket `has_presenter` refusal goes),
`commands/overlay.rs` (close, resize, result, text, copy on a page slot), `projection.rs`, `crates/ngterm-proto/src/tree.rs`
(`remote`, `replica`, `pageID`), tests in `commands/tests/remote.rs`, `remote/frames.rs`

Builds on: the program-overlay handoff (`remote/handoff.rs` `open_remote_overlay`, `follows_remotely`;
`model/remote_overlay.rs`; frames `overlay.*`), Task 14's kept kinds and `presenter_supports`, and Task 20's
hide/show/toggle.

**Platforms:** no OS-specific code (ngterm-core)

**Conformance:**
- new in `tests/protocol/test_zmx.py` (LIVE; the viewer made a follower with `leading_client`, the presenter acquired
  as in `test_job_run_serves_an_overlay_a_presenter_shows`; `_hello` takes an optional kinds list):
  `test_a_presenter_without_page_kinds_keeps_the_page_refusal` (covers `session.overlay.open`: "a viewer presents
  this session: an html overlay would open where nobody sees it", no remoteOverlays),
  `test_a_loopback_url_bound_for_the_presenter_is_refused` ("a loopback url names this Mac, so the Mac presenting the
  session cannot load it"; record the "this Mac" wording against rust-build.md's renamed-strings rule),
  `test_a_page_goes_to_the_presenter_and_its_reports_reach_the_tree` (covers `session.overlay.open`,
  `field:tree.session.remoteOverlays`, `field:tree.session.htmlOverlays`),
  `test_origin_commands_on_a_remote_page_reach_the_presenter` (covers `session.overlay.reload`, `.navigate`,
  `.resize`, `.result`, `.text`), `test_closing_or_rejecting_a_remote_page_ends_it_dismissed` (covers
  `session.overlay.close`, `session.overlay.result`)

- [ ] frames: `page.request` (page, token, source, identity, chromeless, javascript, navigation, browse, persistent,
      backgroundColor?, pane?, sizePercent?, follow), `page.rejected`/`page.close`/`page.closed` (page, reason?),
      `page.command` (page, seq, action ∈ hide|show|reloadOriginal|reloadCurrent|back|forward|browser|resize,
      sizePercent?), `page.state` (page, title?, address?, load, error?, canGoBack, canGoForward, hidden, appliedSeq;
      title ≤ 1024, address ≤ 8192, error ≤ 1024 chars), each under the payload key `page`. Confirm the source's
      wire shape against the reference build before fixing the serde shape (Swift's synthesized enum coding)
- [ ] `HtmlSource::Replica { path }` and `replica_of`; the replica scheme is `ngterm-page` (identity rule, never
      `agterm-page`)
- [ ] open routing before the local checks (the origin renders nothing): no presenter or not following → local; a
      held slot → "overlay already open" / the pane's refusal; right pane without a split → the pane-not-visible
      refusal; placement keeps it here (this Mac leads, or the session-wide slot's roles are unknown or mixed) →
      local; a presenter lacking a page kind → the presenter refusal; a loopback `--url` → the loopback refusal; a
      request over one frame → "page too large to show on another Mac" (encoded before anything is reserved); then a
      HUD in the session-wide slot closes, the slot is reserved (token a new UUID, 30 s claim deadline, owner the
      presenter generation, sizePercent clamped 1..100 for the session-wide slot), the outcome registered,
      tree.changed, `page.request` sent; reply `{id, pageId}`
- [ ] the store's own guard replaces the blanket refusal: a local open (a clicked link's browsing page) is refused
      only when placement would hand the slot over
- [ ] receive: `page.state` applies load/error/current/hidden (tree.changed only when hidden changed) and settles a
      pending visibility request once appliedSeq ≥ its seq; `page.rejected`/`page.closed` end the page (reservation
      removed, outcome dismissed unless answered, tree.changed); the reject reason is logged
- [ ] close sends `page.close` and ends the reservation at once; resize updates sizePercent and sends a `resize`
      command; presenter loss and session removal end every remote page, keeping a submitted answer
- [ ] reload [--current] → reloadOriginal/reloadCurrent; back/forward → command; browser → command for a URL page,
      "page is shown on another Mac" for a file page; finder → that refusal on a file page, the existing local answer
      on a URL page; a gone stream → "the viewer showing this overlay is gone"; result bare → "no overlay result: the
      slot holds an html page"; text/copy → "no overlay to read: the slot holds an html page"
- [ ] hide/show/toggle on a remote page: requested visibility with the next seq; toggle by the pending request, else
      the reported state; a remote program still answers "overlay is shown on another Mac"
- [ ] read-back: htmlOverlays lists a remote page in slot order with `remote: true`, fields from the record and the
      last `page.state`, no zoom; hiddenOverlays counts one reported hidden; remoteOverlays gains `pageID`; field
      order … zoom, id, hidden, remote, replica; a local page omits both
- [ ] the viewer still answers `page.rejected` until Task 24 (so this task merges on its own)
- [ ] write tests: Rust unit tests for every origin case listed; frame round trips and the bounded strings; the
      projection's omissions; refusal strings in full on the dispatcher's output
- [ ] write tests: the conformance tests above
- [ ] manual check (user): green on reference-macos; green on `cross-platform.yml` (ngterm-core)
- [ ] gate passes; newly green ids added with `check_ratchet.py --add`

### Task 23: HTML pages on a remote presenter, part 2: the page-serve channel and `session overlay page-serve` (#793)

**Upstream:**
- implementation: `PageServeFrames.swift` (new), `RemotePageResources.swift` (new), `ControlDispatcher+Overlay.swift`,
  `ControlDispatcher.swift`, `ControlActionsDefaults.swift`, `HtmlBridge.swift`, `RemoteSession.swift`
  (pageServeCommand), `AppStore+RemotePage.swift` (claimRemotePage, expireRemotePageClaims),
  `agtermctlKit/OverlayPageServe.swift` (new), `SessionCommands.swift`, `agterm/Control/ControlServer+PageServe.swift`
  (new), `ControlServer.swift`, `ControlStreamOwner.swift`
- tests: `PageServeFramesTests.swift` (all 5), `RemotePageResourcesTests.swift` (all 4), `RemoteSessionTests.swift`
  (the 4 pageServe cases), `ControlDispatcherOverlayTests.swift` (pageServeRejectsAMissingOrMalformedTokenBeforeCallingActions,
  pageServeReachesTheHostsClaimWhichDefaultsToUnsupported), `HtmlBridgeTests.swift`, `AppStoreRemotePageTests.swift`
  (aTokenClaimsOnceWithinItsDeadlineForTheSamePresenter, aLateClaimIsRefusedAndExpiryEndsOnlyAnUnclaimedFilePage),
  `agtermctlKitTests/OverlayPageServeTests.swift` (all 3), `agtermTests/ControlServerPageServeTests.swift` (the channel
  cases: testAClaimedChannelServesThePageAndAFileInsideItsGrant through testLosingThePresenterEndsThePageAndClosesItsChannel)
- contract: `site/commands.html` `session overlay page-serve TOKEN` / `session.overlay.page.serve`;
  `.claude/rules/control-api.md` (refused from a page; the streaming hand-offs; the page-serve channel); `site/docs.html`

**Rust side:** `crates/ngterm-core/src/remote/page_serve.rs` (create: frames, codec, resource resolution, the origin's
stream state), `remote/page.rs` (claim, expiry, place lookup), `commands/overlay.rs` (the dispatch arm),
`dispatch.rs`, `html/bridge.rs` (refused and not session-targeted, like `live.present`, `job.run`, `live.reset`),
`remote/ssh.rs` (`page_serve_command`), `crates/ngterm-server/src/stream.rs` (a blocking send for an off-main
producer; every queue change wakes all waiters), `crates/ngterm-gpui/src/state.rs` (adopt the claimed connection
as for `job.run`), `crates/ngterm-ctl/src/overlay_page_serve.rs` (create), `crates/ngterm-ctl/src/session_overlay.rs`,
`crates/ngterm-ctl/tests/cli.rs`

Reuse: the `job.run` claim and handoff (`commands/overlay.rs` `claim_job`; `ngterm-gpui/src/state.rs` job stream with
`LineStream`/`LineStreamOwner`), the ctl stream bridge (`ngterm-ctl/src/stream_bridge.rs`, as `live present` uses),
the ssh argv builders (`remote/ssh.rs`).

**Platforms:** the channel logic is ngterm-core (any): file serving on a worker thread with std::fs; MIME types from
`mime_guess` (the standard crate; gio stays out of core); resource resolution with `std::fs::canonicalize` and
`Path::is_absolute` (a Windows origin compares drive-letter paths and handles the `\\?\` prefix on both sides). The
connection handoff is the control server's (AF_UNIX, Linux and macOS); its Windows form waits with the server's
named-pipe form, as `job.run` does. `ngtermctl session overlay page-serve` is a stdio↔socket copy (Linux now, the
same code on macOS, a named pipe with the CLI's Windows form). `page_serve_command` builds the POSIX shell chain the
origin runs (`ssh -T -o BatchMode=yes -o ConnectTimeout=5 <host> '/bin/sh -c "<PATH chain> && exec ngtermctl session
overlay page-serve <token>"'`), so a Windows origin needs its own remote form, as `live present` and run-job do.

**Conformance:**
- append to `tests/protocol/test_zmx.py::test_stream_commands_refuse_before_streaming`: page-no-token
  ("session.overlay.page.serve requires a page token"), page-invalid ("invalid page token"), page-unknown ("page not
  claimable")
- new: `test_zmx.py::test_page_serve_serves_a_page_a_presenter_shows` (LIVE, covers `session.overlay.page.serve`,
  `session.overlay.submit`, `session.overlay.result`, streams `session.overlay.page.serve`: claim with the token from
  `page.request`; ok `{"id": pageId}`; a `resource` gives head, chunk(s) ≤ 64 KiB, end; outside the folder "file is
  outside the page's folder"; a `bridge` submit's raw reply is ok and `result --page` reads it; a second claim "page
  not claimable"), `test_zmx.py::test_losing_the_page_helper_ends_only_the_page`,
  `tests/cli/test_cli_output.py::test_page_serve_refused_claim_exits_1` (covers `cli:session overlay page-serve`:
  exit 1, `error: page not claimable` on stderr; check the prefix against the reference)

- [ ] `session.overlay.page.serve` (target = token): blank → "session.overlay.page.serve requires a page token", not
      a UUID → "invalid page token", no admissible claim → "page not claimable"; a claim admitted once, within 30 s,
      only while the slot's owner is still the session's presenter generation; ok `{id: <pageId>}`, then the
      connection is adopted as the page's channel; a page that ended between claim and adoption has its connection
      shut; a claim whose reply could not be written ends the page
- [ ] claim expiry: 30 s (+0.1 s) after a file page's open, every unclaimed file page past its deadline ends (by
      close, so `page.close` goes out); a URL page never expires
- [ ] frames as JSON lines (sorted keys): resource{id,path}, head{id,mimeType}, chunk{id,data base64},
      end{id}, failure{id,error}, cancel{id}, bridge{id,request}, reply{id,response}; unknown kind or missing field →
      malformed; a line over the frame limit or a chunk over 64 KiB refused both ways; "too many requests in flight",
      "cancelled"
- [ ] resource resolution: absolute only; no grant → only the page's own file; a grant → a file inside the canonical
      grant, not the grant itself; `..` and symlink escapes refused; a URL page serves nothing
- [ ] origin stream: cancel marks a transfer cancelled; resource and bridge each take one of 8 slots or get `failure
      busy`, and when the writer queue (8 lines, 5 s write timeout) cannot take that refusal the page ends; a frame
      only the origin sends, or a malformed one, ends the page and keeps the presenter; a transfer ends in one
      terminal frame ("file is outside the page's folder", "file cannot be read", head+chunks+end, or "cancelled");
      bridge requests dispatch in the same main-loop turn through the page bridge against the page's place at that
      moment ("page closed", "control is unavailable", "answer too large for the channel"); helper EOF ends only the
      page; a page ended elsewhere shuts its channel once
- [ ] decide whether upstream's "session.overlay.page.serve is not supported on this platform" default has a
      counterpart (the core owns the claim here) and record it
- [ ] `LineStream`: a blocking send that waits while the queue is full and returns false once closing
- [ ] ngtermctl `session overlay page-serve <token>` (upstream's about, long text and argument help, ngterm names),
      the socket options, a refused claim prints `error: <reason>` and exits 1, an accepted one pumps stdio untouched
- [ ] `page_serve_command(host, token)`: validate the host (refuse `-oProxyCommand=x`), the token plain (refuse "",
      "t 1", escapes; quote "t1;rm"), `-T`, the PATH chain as `live present`
- [ ] write tests: Rust unit tests for every PageServeFrames, RemotePageResources, RemoteSession page-serve,
      dispatcher and bridge case; the claim and expiry cases; the channel cases from ControlServerPageServeTests as
      core tests over fake line sinks and a temp dir; ngterm-ctl request, missing token, refused claim, help snapshot
- [ ] write tests: the conformance tests above
- [ ] manual check (user): green on reference-macos; green on `cross-platform.yml` (ngterm-core)
- [ ] gate passes; newly green ids added with `check_ratchet.py --add`

### Task 24: HTML pages on a remote presenter, part 3: replica pages and the `ngterm-page` scheme (#793)

**Upstream:**
- implementation: `AppStore+ReplicaPage.swift` (new), `AppStore+RemotePresentation.swift`, `AppStore+HtmlOverlay.swift`
  (replica submit, the update notify), `AppStore+OverlayVisibility.swift`, `HtmlOverlay.swift` (navigation policy,
  `onReplicaRelease`), `RemotePresentationClient.swift`, `AppStore.swift`, `agterm/Control/ControlServer+RemotePage.swift`
  (new), `ControlServer+RemotePresentation.swift`, `ControlServer+Presentation.swift`, `RemotePageChannel.swift` (new),
  `agterm/Views/HtmlPageSchemeHandler.swift` (new), `HtmlOverlayRegistry.swift`, `HtmlOverlayBridge.swift`,
  `HtmlOverlayView.swift`
- tests: `AppStoreReplicaPageTests.swift` (all 18), `HtmlOverlayTests.swift` (the replica policy arguments,
  aReplicaIsNamedByItsFileAndReportsTheOriginPathAsReplica), `agtermTests/ControlServerRemotePageTests.swift` (all 9),
  `RemotePageChannelTests.swift` (all 12), `HtmlPageSchemeHandlerTests.swift` (all 4),
  `ControlServerRemotePresentationTests.swift` (testAFilePageHandedToTheViewerSubmitsThroughItsChannel,
  testTheOriginReadsTheViewersPageStateAndPresenterLossDismissesIt, testAnOlderPresenterKeepsTheRefusal)
- contract: `site/commands.html` overlay submit ("submit on the Mac that opened the page"), the `replica` field;
  `.claude/rules/control-api.md` presenter bullets; `site/docs.html` remote sessions

**Rust side:** `crates/ngterm-core/src/remote/replica_page.rs` (create), `remote/page_channel.rs` (create: the
presenter's toolkit-neutral channel state machine), `remote/client.rs` (page effects; reject when not shown),
`remote/viewer.rs`, `html/navigation.rs` (replica policy), `commands/overlay_page.rs` (replica submit refusal, the
release signal), `projection.rs`, `crates/ngterm-app/src/page_channel.rs` (create: the ssh helper child with piped
stdio, beside `remote.rs`), `crates/ngterm-gpui/src/html_overlay/registry.rs` (register `ngterm-page` beside
`ngterm-file`; route its requests to the page's channel; answer by token), `html_overlay/page.rs`, `policy.rs`,
`view.rs`/`panel.rs` (toolbar), `crates/ngterm-gpui/src/state.rs`; possibly `crates/ngterm-wpe` (a streamed scheme
answer)

**Platforms:** Linux: WPE WebKit through ngterm-wpe; `ngterm-page` registered at engine start beside `ngterm-file`
(`html_overlay/policy.rs` `GRANT_SCHEME`, `registry.rs`, `crates/ngterm-wpe/src/event.rs` SchemeRequest/Response,
`config.rs` schemes); decide whether `SchemeResponse::Bytes` (a whole body, bounded by the 64 inbound permits) is
enough or ngterm-wpe gains a streamed answer (a GInputStream fed per chunk: that changes its worker protocol); WebKit's
request cancellation sends `cancel`; every Linux desktop (WPE renders offscreen). macOS: WKWebView with a
WKURLSchemeHandler for `ngterm-page` (upstream's HtmlPageSchemeHandler is the reference). Windows: WebView2
`CoreWebView2CustomSchemeRegistration` plus `WebResourceRequested` with a deferral. HTML overlays exist only on Linux
today, so the macOS and Windows forms belong to those web-view ports (record them there). The channel state machine
is ngterm-core (any). The ssh helper uses std::process with piped stdin/stdout on all three OSes (ssh via PATH),
not upstream's socketpair and /usr/bin/env; ngterm-app is Linux today and its `remote.rs` uses std::os::unix, so the
new helper adds no Unix-only code without a cfg'd Windows form.

**Conformance:** none in the suite (it runs no ssh); headless WPE tests in ngterm-gpui and core tests instead

- [ ] client: `page.request` → show it or answer `page.rejected{page, reason}` ("pages are not shown on this Mac" for
      a host that shows none); `page.close` → take the replica down; `page.command` → apply it; the viewer
      advertises the page kinds only when this build can show pages (feature `html` on and web views available): a
      deviation from upstream's static hello, recorded in the task
- [ ] present a replica: "no such session" (not a remote-presented session), "invalid page id", a file source →
      `HtmlSource::Replica{path}` (grant dropped), a URL that must parse as http(s) ("invalid url"), a pane mapped to
      the local role that must render ("pane not visible"); opened under a new local id with `replica_of` = origin
      id through the ordinary open (its refusals come back as the reason), no outcome registered; `follow` selects
      the session
- [ ] app side: a file page needs the session's remote host and a valid `page_serve_command` ("the origin cannot be
      reached for this page"); a `--persistent` URL page checks the saved store first; "page not shown" when it
      cannot be found again; a file page starts its ssh helper and closes with the channel; a URL page loads directly
      with this machine's store
- [ ] origin commands on a replica: hide/show that slot only; reload/back/forward/browser to the page (a refusal is
      logged); resize only for the session-wide slot; applied_seq = max; a state report queued
- [ ] state reports: `page.state` with the origin id, title, address (the origin path for a file replica, never the
      `ngterm-page` URL), load, error, history, hidden, applied_seq, coalesced to one per page per main-loop turn
- [ ] release: every way a replica leaves its slot sends `pageClosed` once and closes its channel; stream loss
      closes every replica of the session
- [ ] a socket `session.overlay.submit` on a replica → "submit on the Mac that opened the page" (it stays open);
      presenter-side navigate browser/finder on a file replica → "the page's file is on another Mac"; browser opens
      a URL replica
- [ ] replica navigation policy: a web URL → open externally when user-activated, else cancel; `ngterm-page` with
      the page's own id as host (case-insensitive) → allow; anything else → cancel; about:blank allowed
- [ ] read-back on the presenter: `replica: true`, file = origin path, no cwd
- [ ] page channel client: ≤ 8 resource and ≤ 8 bridge requests in flight, the rest queue; a slot freed only by that
      request's terminal frame; a cancel before head sends `cancel` and keeps the slot until the origin ends it; a
      cancel while queued sends nothing; 64 inbound permits, a chunk keeps its permit until consumed; an answer for a
      load the page left is dropped; a bridge reply reaches the page raw; an oversized bridge request fails alone
      ("request too large for the page channel"); failures before head keep the channel; close fails pending calls
      with "page channel closed"; helper exit closes once; a frame only the origin sends, or a malformed one, closes it
- [ ] a linked replica's bridge forwards the request unchanged and shapes the reply ("invalid reply"; ok false → its
      error or "request failed"; else its result, unknown fields kept)
- [ ] toolbar: a replica shows no Open in Browser, Show in Finder or Copy Link; the tooltip shows the origin path; a
      file replica wears the theme like a local file page, a URL replica keeps the opaque canvas
- [ ] write tests: Rust unit tests for every AppStoreReplicaPageTests and RemotePageChannelTests case and the replica
      policy and projection; headless WPE tests (lavapipe, private display and bus, as the existing html_overlay
      tests) for the scheme handler cases (stylesheet, image, ES modules; fetch and pushState/back; a stopped request
      cancelling its source; another page id never reaching it), a file replica loading an asset through a fake
      channel, a bridge reply keeping unknown fields, a tagged control relaying with `--js` off; a two-role in-process
      test (origin and presenter over a fake transport instead of ssh) for the three ControlServerRemotePresentation
      cases
- [ ] manual check (user): green on `cross-platform.yml` (ngterm-core)
- [ ] gate passes

### Task 25: Point-sized floating overlays: `--width`/`--height`, `overlayWidth`/`overlayHeight` (#798)

**Upstream:**
- implementation (commit `9bbb340`): `OverlayPointSize.swift` (new), `ControlDispatcher+Overlay.swift`,
  `ControlProtocol.swift` (PaneOverlayError.pointsConflict, OverlayPointError), `ControlDispatcherOptions.swift`,
  `ControlDispatcher.swift`, `ControlActionsDefaults.swift`, `AppStore+Panes.swift`, `AppStore+HtmlOverlay.swift`,
  `AppStore.swift`, `ControlProjection.swift`, `Session.swift`, `HtmlOverlay.swift` (strip 28, chrome 29),
  `HtmlBridge.swift`, `AppStore+RemoteOverlay.swift` and `AppStore+RemotePage.swift` (the point-size refusals),
  `agtermctlKit/SessionCommands.swift`, `agterm/Control/ControlServer+SessionActions.swift`, `ControlServer+OverlayJobs.swift`,
  `agterm/Views/WindowContentView+Detail.swift` (OverlayPanelStyle.PointSizing), `HtmlOverlayView.swift`
- tests: `OverlayPointSizeTests.swift` (all 13), `ControlDispatcherOverlayTests.swift` (sessionOverlayResizeRejectsBadArgs,
  sessionOverlayResizeRoutesPointsWithAndWithoutAPercent, sessionOverlayResizeRefusesBadPoints,
  sessionOverlayOpenRoutesPointsBesideThePercent, sessionOverlayOpenRefusesBadPoints), `AppStoreRemoteOverlayTests.swift`
  (pointSizesAreRefusedBeforeAnythingIsReserved, aSlotShownOnThePresenterTakesNoPointSizes), `AppStoreRemotePageTests.swift`
  (pointSizesAreRefusedOnlyWhenThePageWouldGoToThePresenter), `RemotePresentationStateTests.swift`
  (aHandedOverOverlayTakesNoPointSizes), `HtmlBridgeTests.swift`, `agtermctlKitTests/OverlayCommandsTests.swift`
  (openAndResizeSendPointSizes, pointSizesAreRefusedBeforeAnyConnection), `agtermTests/HudDeckGatesTests.swift`
  (testPointSizesReplaceOnlyTheirAxisAndAddThePageStrip)
- contract: `site/commands.html` tree (overlayWidth/overlayHeight after overlaySizePercent), `session overlay open`
  synopses (`[--size-percent N [--width PT] [--height PT]]`), `session overlay resize` (`(--size-percent N [--width PT]
  [--height PT] | --width PT [--height PT] | --height PT | --full)`); `.claude/rules/control-api.md`

**Rust side:** `crates/ngterm-core/src/model/overlay_points.rs` (create), `model/mod.rs`, `model/overlay.rs`
(`OverlaySlot.points`, `resize_overlay_in_points`), `model/html_overlay.rs`, `crates/ngterm-core/src/html/mod.rs`
(STRIP_HEIGHT 28, CHROME_HEIGHT 29), `commands/overlay.rs`, `commands/overlay_page.rs`, `remote/handoff.rs`
(`RemoteOverlayOpen::PointsUnsupported`), `remote/page.rs` (Task 22's page path), `html/bridge.rs` (`session.overlay.resize`
joins `OWN_OVERLAY`), `projection.rs` (from `session.hud_frames.detail`), `crates/ngterm-proto/src/tree.rs` (two
`Option<f64>` after `overlay_size_percent`), `crates/ngterm-ctl/src/session_overlay.rs`, `crates/ngterm-gpui/src/overlay.rs`,
`split.rs` (the placement lays out through core's `panel_size`), `html_overlay/panel.rs` (STRIP_HEIGHT from core),
their tests. Building blocks: `Args.width/height` (`ngterm-proto/src/request.rs`), `hud::MAX_SIZE_PERCENT = 80`,
`Session.hud_frames.detail`.

**Platforms:** no OS-specific code

**Conformance:**
- the listed `tests/protocol/test_overlay.py::test_overlay_size_and_resize` asserts "session.overlay.resize requires
  --size-percent or --full": it changes to "session.overlay.resize requires --size-percent, --width/--height or
  --full" in this change (it goes red against both the v0.38.1 reference and the port otherwise)
- new: `test_overlay.py::test_overlay_points_size_and_resize` (covers `session.overlay.open`, `session.overlay.resize`,
  `field:tree.session.overlayWidth`, `field:tree.session.overlayHeight`, `field:tree.session.overlaySizePercent`;
  compare with `pytest.approx`: the reference reports fractional doubles), `test_page_points_leave_out_the_strip`,
  `test_point_size_refusals` (parametrized, nothing opened or changed), `tests/protocol/test_hud.py::test_points_do_not_resize_a_hud`,
  `tests/protocol/test_zmx.py::test_points_are_refused_for_an_overlay_a_presenter_shows` (LIVE),
  `tests/cli/test_cli_output.py::test_overlay_points` (covers `cli:session overlay open`, `cli:session overlay resize`)

- [ ] `OverlayPointSize` (per-axis `Option<i64>`, merging, `panel_size`: a point axis = min(points + chrome on height,
      80% of the session), a percent axis keeps 1..100; `content_size` = panel minus chrome, floored at 0; chrome 29
      for a page with a strip, 0 for chromeless or a program)
- [ ] slot state: points kept only beside a percent on open (program and page), cleared on close; a percent resize
      replaces them whole; a full resize clears them
- [ ] open order (after the percent-range check): "session.overlay.open: --pane is mutually exclusive with
      --width/--height", "session.overlay: --width/--height must be positive", "session.overlay.open:
      --width/--height need --size-percent"
- [ ] resize order: the pane refusal, full+percent, "session.overlay.resize: --full is mutually exclusive with
      --width/--height", "session.overlay.resize requires --size-percent, --width/--height or --full", the percent
      range, positive points; with points: an unknown session "no overlay"; a remote slot or a replica "--width/--height
      are not supported for an overlay shown on another Mac"; a HUD "a hud measures its own height: --width/--height
      do not apply"; no covering overlay "no overlay"; with a percent, those points; without one, a full overlay
      "session.overlay.resize: --width/--height need a floating overlay; pass --size-percent too", else merge only
      the given axes
- [ ] remote: `PointsUnsupported` after the follows-remotely check and before the frame-size check (nothing reserved
      or sent); the page-to-presenter refusal in Task 22's open path
- [ ] tree: `overlayWidth`/`overlayHeight` right after `overlaySizePercent`, the content size from the last layout
      for a floating covering program or page; absent for a full overlay, a HUD, an empty slot and before the first
      layout. Confirm the GPUI detail frame is in the logical points the tree reports
- [ ] bridge: `session.overlay.resize` joins the page's own overlay commands
- [ ] ngtermctl `--width`/`--height` on open and resize with upstream's help ("Panel content width in points, in
      place of --size-percent's width (at most 80% of the session)."; the height one adds "; a page's title strip is
      added on top."); usage errors "--pane cannot be combined with --width/--height (pane overlays are always full)",
      "--width/--height must be positive", "--width/--height need --size-percent", "--full cannot be combined with
      --width/--height", "provide --size-percent PERCENT, --width/--height POINTS or --full"
- [ ] GPUI: the floating panel sized by `panel_size`, centered as now
- [ ] write tests: the OverlayPointSize table ((∅,0)→600×480, (h240,0)→600×240, (h240,29)→600×269, (w300)→300×480,
      (5000,5000)→800×640, (h630,29)→600×640; 95% width kept at 950), content size, merging, open/page/resize/merge/
      not-floating/HUD/replica cases, tree from the detail frame and a percent-only panel at 500×400; the dispatcher
      tables; the remote refusals with no frame sent; the bridge list; the ctl cases; a GPUI deck rect test (60% with
      height 240 in 1000×800 → 600×240 at midY 400; a page with width 5000 and height 240 → 800×269 at midX 500);
      the proto key order
- [ ] write tests: the conformance tests above
- [ ] manual check (user): green on reference-macos
- [ ] gate passes; newly green ids added with `check_ratchet.py --add`

### Task 26: Live replay: an interrupted restored command or `--command` leaves the login shell's prompt (#773)

**Upstream:**
- implementation (commit `be55c19`, and the fish form of `c1f5e2b`): `agtermCore/Sources/agtermCore/ZmxReplayScript.swift`
- tests: `ZmxReplayScriptTests.swift` (interruptedPayloadStillExecsTheFinalShell(form: argv|commandLine),
  fishInterruptedPayloadStillExecsTheFinalShell(form:), the updated aCustomZdotdir… and missingOriginalZdotdir…
  expected strings, fishRenderersUseFishEscapingAndRearmBeforeTheFinalShell, fishQuotingCoversRequiredAndHostileArguments,
  fishCapturedArgumentsSurviveTheParserRoundTrip, fishDurableCommandLineSurvivesTheParserRoundTrip);
  `agtermTests/SessionHostFishSeamTests.swift` (testCapturedArgvPayloadOwnsPaneForeground,
  testDurableCreationCommandOwnsPaneForeground)

**Rust side:** `crates/ngterm-core/src/live/replay.rs` (today `statement ; rearm ; exec` joined with ` ; `),
`crates/ngterm-core/src/live/support.rs` (expected strings), `crates/ngterm-session/tests/replay.rs` (create:
real-PTY interruption tests), `conformance/tests/protocol/test_zmx.py`, `tests/protocol/test_live_any_shell.py`

A probe under script(1) with a payload running `kill -INT 0`: bash in the current form never reaches the final
shell (a newline before the rearm and exec does); fish reaches it only with `builtin status job-control full;
builtin eval …` together.

**Platforms:** zsh (macOS, Linux): upstream's `always` block. bash (Linux; macOS where it is the login shell): a
newline before the rearm and exec, verified under a PTY. fish (macOS, Linux): `builtin status job-control full;
builtin eval <quoted>`, which also makes the payload own the pane's foreground. PowerShell (Windows; Linux and macOS
where it is the login shell): the integration loads before the payload and `-NoExit` keeps the shell; confirm an
interrupted payload leaves an integrated prompt in `ngterm-session`'s pwsh tests on the Windows runner. ngterm-core
and ngterm-session are in `cross-platform.yml`.

**Conformance:**
- existing: `tests/protocol/test_zmx.py::test_restart_replaces_the_pane_shell`,
  `test_restart_without_a_command_replays_the_foreground_program` (listed)
- new: `test_zmx.py::test_restart_command_interrupted_by_ctrl_c_leaves_a_prompt` (LIVE, covers `session.restart`:
  `--command 'sleep 600'`, wait for the foreground, `session.type "\x03"`; the pane and session stay, `zmx.list`
  lists the same daemon, a marker typed afterwards prints), `test_zmx.py::test_new_command_interrupted_by_ctrl_c_leaves_a_prompt`
  (covers `session.new`), `test_live_any_shell.py::test_an_interrupted_live_command_leaves_the_login_shell_prompt`
  (requires `live-any-shell`, so the bash leg runs it too)

- [ ] zsh: `{ <statement> ; } always { <rearm> ; 'builtin' 'exec' -- '<zsh>' -il ; }` (upstream's exact form)
- [ ] bash: the interruption-proof form, verified under a PTY
- [ ] fish: `builtin status job-control full; builtin eval <fish-quoted statement>; builtin set -gx
      GHOSTTY_SHELL_INTEGRATION_XDG_DIR <root>; <rearm>; exec <fish> -il`, joined with `; `, for both the
      captured-argv and the durable-line forms
- [ ] PowerShell: confirm, and fix if it does not hold
- [ ] write tests: replay.rs expected strings (zsh `{ 'printf' 'a b' ; } always { … -il ; }`; fish's quoting table:
      trailing backslash, `can't`, empty, `\'`, backslash runs, spaces, non-ASCII, `$literal`, parentheses, glob,
      `;`, `>`, `--option`, an embedded newline); ngterm-session real-PTY tests per shell present (zsh, bash, fish;
      pwsh on the Windows runner), argv and line forms: a payload logging "ran" then `kill -INT 0` runs once and the
      final-shell probe records the rearmed variables and `-il`; a fish payload owns the pane's foreground process
      group
- [ ] write tests: the conformance tests above
- [ ] manual check (user): green on reference-macos; green on `cross-platform.yml`
- [ ] gate passes; newly green ids added with `check_ratchet.py --add`

### Task 27: Live sessions with a fish login shell: the `live-sessions` claim and the XDG merge (#761 #760 #768 #780)

**Upstream:**
- implementation (commits `c1f5e2b`, `1e2cbd2`): `ZmxSupport.swift`, `ZmxReplayScript.swift` (the rearm from the
  payload's live value), `agterm/Ghostty/ZmxLaunch.swift` (visibility only), `.claude/rules/settings.md`, `site/docs.html`,
  `site/commands.html` (`--command` uses the login shell's syntax, zsh or fish)
- tests: `ZmxSupportTests.swift` (configurationUsesPasswordDatabaseFishAndFullPaneIdentity,
  fishSurfaceEnvironmentLeavesAnInheritedXDGDataDirectoriesAlone, fishAttachMergesTheIntegrationRootIntoTheLaunchEnvironment,
  missingFishResourcesAndLoaderAreRejected, fishAttachCarriesEveryPayloadFormWithAndWithoutTheSessionHost);
  `ZmxReplayScriptTests.swift` (fishReplayReachesTheFinalShellAfterFailedPayloads,
  fishReplayRearmsXDGDataDirectoriesFromTheLivePayloadState); `agtermTests/SessionHostFishSeamTests.swift`
  (testFailingPayloadsStillLandOnIntegratedFish, testSurvivingDaemonDoesNotReplay*, testKilledDaemonStartsNewFishAndRunsPayloadOnce);
  `ZmxLaunchTests.swift` (testFishConfigurationKeepsOnlyAnExplicitPaneXDGDataDirectories,
  testZshConfigurationDoesNotCopyTheInheritedXDGDataDirectories)

**Rust side:** `crates/ngterm-core/src/live/shell_integration.rs` (`inject`/`rearm` for fish),
`crates/ngterm-core/src/live/support.rs`, `crates/ngterm-core/src/live/replay.rs`, `crates/ngterm-session/src/cli.rs`
(the XDG_DATA_DIRS merge where the session's environment is prepared; confirm every attach path, in-process, CLI and
remote, reaches it), `crates/ngterm-gpui/src/session_host.rs` (claim `live-sessions` for fish too),
`conformance/agterm_conformance/impls/base.py` (`login_shell_blocker`: zsh or fish), the LIVE marker reasons in
`tests/protocol/test_zmx.py`, `tests/cli/test_cli_output.py`, `tests/cli/test_cli_only.py`, `tests/ui/test_live.py`,
`tests/behaviors/test_relaunch.py`, `conformance/selftests/test_impls.py`, `test_fixtures.py`,
`.github/workflows/rust-linux.yml` (the fish leg gets `--must-pass live-sessions`), README.md,
`conformance/docs/rust-build.md`

Already here as a Rust extension: fish eligibility and strings (`live/support.rs`, `restore/mode.rs` "the bundled
fish integration is unavailable"), its injection (`shell_integration.rs`), `fish_quoted` (`replay.rs`), the durable
line through `builtin eval` (`render_command_line`), no replay on a re-attach. What differs from upstream: the
surface's XDG_DATA_DIRS is set from the app's own environment and overrides a ghostty.conf `env` entry; an empty
value gives the root alone (upstream `root:/usr/local/share:/usr/share`); the rearm exports a launch-time value.

**Platforms:** the fish wrap and the XDG merge are Unix (macOS and Linux share the code); Windows: no counterpart,
since a Windows live pane runs PowerShell and the fish tests carry `cfg_attr(windows, ignore)`. The `live-sessions`
claim is by login shell on every OS.

**Conformance:**
- once `live-sessions` is claimed on fish, the fish CI leg runs every LIVE test as a must-pass: Task 26's fish form
  lands first
- new: `tests/protocol/test_live_any_shell.py::test_a_live_fish_pane_keeps_the_configs_xdg_data_dirs` (covers
  `behavior:restore.live-session-survives-relaunch`, skipped unless the login shell is fish: a ghostty config `env =
  XDG_DATA_DIRS=/conf/share` reaches the pane's fish as `/conf/share` with the integration active)

- [ ] claim `live-sessions` for a fish login shell as well as zsh, with its doc comment, README, rust-build.md and
      base.py's capability text; `live-any-shell` stays the Rust-only claim adding bash and PowerShell
- [ ] reference adapter: `login_shell_blocker` accepts zsh or fish ("the password-database login shell is not zsh or
      fish"); the LIVE marker reasons say zsh or fish
- [ ] CI: the fish conformance leg runs with `--must-pass live-sessions --must-pass live-any-shell`
- [ ] fish surface environment: only GHOSTTY_SHELL_INTEGRATION_XDG_DIR; XDG_DATA_DIRS merged when the session's
      environment is prepared: root, then the pane's resolved value (ghostty.conf `env` included), else
      `/usr/local/share:/usr/share` when unset or empty, dropping empty entries and copies of root; zsh's
      configuration does not copy XDG_DATA_DIRS. Ghostty's plain-pane fish injection stays Ghostty's
- [ ] fish rearm recomputes XDG_DATA_DIRS from the payload's live value (upstream's `begin … builtin set --global
      --export --path XDG_DATA_DIRS <root> $__agterm_xdg_user_dirs; end` block, with `__ngterm_` names)
- [ ] write tests: the merge table (inherited `/custom/share`; unset; empty; a pane value overriding the inherited one;
      root only; root repeated with empty entries; space, quote, backslash and glob entries); the fish configuration
      has no XDG_DATA_DIRS; with a real fish: failed payloads (a missing binary, `/usr/bin/false`) still reach the final
      shell with `<root>:/usr/local/share:/usr/share`; the five rearm cases (`set -gx XDG_DATA_DIRS /new/one:/new/two`
      → `<root>:/new/one:/new/two`; empty or `set -e` → the default; duplicates of root dropped; a path with a space
      kept); the selftests for the adapter's blocker
- [ ] write tests: the conformance test above; the LIVE set passing on the fish leg
- [ ] manual check (user): green on reference-macos; the fish leg of `rust-linux.yml` green on the pushed branch
- [ ] gate passes; newly green ids added with `check_ratchet.py --add`

### Task 28: Session server: keep its namespace lock from aging out of the temp dir (#733)

**Upstream:**
- implementation (commit `e5c043b`): `agtermCore/Sources/SessionHostRuntime/Host.swift`, `HostSocket.swift`,
  `.claude/rules/windows.md`
- tests: `SessionHostRuntimeTests/SessionHostServerTests.swift` (the upkeep cases: first call, once per interval on a
  sleep-counting clock, the lock touched through its descriptor; a deleted lock is not brought back)

**Rust side:** `crates/ngterm-session/src/endpoint/unix.rs` (a `keep_current` on the listener's lock descriptor),
`crates/ngterm-session/src/endpoint.rs` (a no-op on Windows), `crates/ngterm-session/src/server/mod.rs` (the
coordinator's wait also wakes for the hourly upkeep), `crates/ngterm-session/tests/endpoint.rs`; cite it as ngterm's
addition in the herdr-derived module's docs

The symptom (liveAttribution unknown) cannot happen here: ngterm-session writes no pidfile and no log, and Linux
attribution reads no file. But `/tmp/ngterm-live-<hash>/server.lock` is never touched after creation, so macOS's
cleaner (3 days) or systemd-tmpfiles' `/tmp` aging (10 days on Arch and Fedora) can delete it, weakening the
namespace guard the atomic-publish behavior relies on (Context).

**Platforms:** macOS: `/private/tmp`; touch `server.lock` with `futimens` on the held descriptor at start and hourly,
on a clock that counts sleep (CLOCK_MONOTONIC on macOS; std's `Instant` does not). Linux: the same touch on
CLOCK_BOOTTIME (rustix `ClockId::Boottime`); harmless on a tmpfs `/tmp` without aging. Windows: no counterpart (the
namespace is `%LOCALAPPDATA%\ngterm\live\<hash>`, which Storage Sense does not clean; the endpoint is a named pipe
with FILE_FLAG_FIRST_PIPE_INSTANCE and no lock file). ngterm-session is in `cross-platform.yml`.

**Conformance:** none (days of aging)

- [ ] the server touches its lock at start and once an hour on that clock; nothing here fails the server (errors
      ignored); a deleted lock is not recreated
- [ ] write tests: unit tests with an injected clock (the first call touches, one inside the interval does not, one
      past it does); an integration test that the lock's mtime moves after an upkeep through the held descriptor
      while the server listens
- [ ] manual check (user): green on `cross-platform.yml`
- [ ] gate passes

### Task 29: Lock a keyboard layout while a terminal has the keyboard (#727)

**Upstream:**
- implementation (commit `00dbb24`): `InputSourceLock.swift` (new), `AppSettings.swift`,
  `agterm/Commands/TerminalInputSource.swift` (new), `agterm/SettingsModel.swift`, `agterm/Views/SettingsView.swift`,
  `agterm/Ghostty/GhosttySurfaceView.swift`, `GhosttySurfaceView+FirstResponder.swift`, `agterm/AppDelegate.swift`
- tests: `InputSourceLockTests.swift` (all 9), `AppSettingsTests.swift` (terminalInputSourceDefaultsOffRoundTripsAndIsNotAGhosttyKey),
  `agtermTests/SettingsModelTests.swift` (testSetTerminalInputSourcePersistsAndOffRemovesTheKey)
- contract: `.claude/rules/settings.md` (`terminalInputSource`; the Key Mapping tab); `site/docs.html`; no control
  command (a deliberate exemption)

**Rust side:** `crates/ngterm-core/src/input_source_lock.rs` (create: `reconcile(terminal_focused, target, enabled,
current)`, `release`, `selection_failed`, `baseline`), `crates/ngterm-core/src/lib.rs`, `persist/settings.rs`
(`terminalInputSource`, nil = off, not a ghostty key), `settings_model.rs`, `crates/ngterm-gpui/src/input_source.rs`
(create: the per-OS performer and its focus wiring), `crates/ngterm-gpui/src/settings/key_mapping.rs` (a "Keyboard
Layout" section first), the focus hook in `keymap_dispatch.rs` or `window.rs`. GPUI's Linux platform already reports
the active XKB layout name and its changes (`vendor/gpui-pre-linux` wayland and x11 clients, `on_keyboard_layout_change`)
but can neither list nor select layouts.

**Platforms:** macOS: TIS (`TISCreateInputSourceList` keyboard and select-capable for the list,
`TISCopyCurrentKeyboardInputSource`, `TISSelectInputSource`; observe the selected and enabled change notifications);
the stored id is the TIS source id. Linux, per desktop (no common Wayland protocol), the stored id the XKB layout
name: X11, any WM: XKB `LatchLockState` lockGroup through x11rb's xkb extension; sway and wlroots-with-IPC: sway IPC
`input type:keyboard xkb_switch_layout <index>`; Hyprland: `hyprctl switchxkblayout <device> <index>`; niri: niri
IPC `action switch-layout <index>`; KDE: D-Bus `org.kde.keyboard` `/Layouts` `org.kde.KeyboardLayouts`
(`setLayout`, `getLayout`, `getLayoutsList`, `layoutChanged`); GNOME (no supported API since `Shell.Eval` was
restricted), weston and anything else: the setting is disabled with the reason shown. On every Linux desktop a
re-switch is detected through `on_keyboard_layout_change`. Windows: `GetKeyboardLayoutList` + `ActivateKeyboardLayout`
on the UI thread, `WM_INPUTLANGCHANGE`, the stored id the KLID (`GetKeyboardLayoutName`). On all three a crash
leaves the locked layout selected; the baseline is never persisted. The pure lock is in ngterm-core (cross-platform
CI); the performers are frontend code, not built on macOS/Windows by CI until ngterm-gpui joins `cross-platform.yml`.

**Conformance:** none (no command); a ui id `settings-terminal-input-source` comes with Task 1's regeneration

- [ ] the pure `InputSourceLock` in ngterm-core, ported from upstream
- [ ] the setting: persisted, Off removes the key, not a ghostty config line, an older settings.json decodes to None
- [ ] the performer re-evaluates on the next main-loop turn when a terminal (main, split, scratch, program overlay,
      quick terminal) gains or loses the keyboard, when the layout or the enabled list changes, when the setting
      changes; restores on quit; a web overlay, a text field, another app or no ngterm window holding the keyboard
      count as not focused
- [ ] Settings ▸ Key Mapping ▸ "Keyboard Layout": a picker "Lock in terminals" (id `settings-terminal-input-source`),
      Off first, then the selectable layouts; a saved id no longer selectable shows "<id> (unavailable)"; the list
      re-read when Settings opens and when the app regains focus; disabled with the reason where no backend exists
- [ ] ➕ take `settings-terminal-input-source` off `PENDING` in `crates/ngterm-gpui/src/ui_ids.rs` (Task 1 parked it
      there)
- [ ] the backends per OS as in Platforms, behind `cfg` or a runtime desktop probe, each with the disabled fallback
- [ ] write tests: the nine InputSourceLock cases; the settings round trip; the settings-model persist/remove case; a
      key_mapping section test with the unavailable row; a backend test on the private Xvfb's XKB (two layouts,
      lockGroup switched and read back)
- [ ] manual check (user): green on `cross-platform.yml` (the core lock); the KDE, sway, Hyprland and niri backends
      on those desktops
- [ ] gate passes

### Task 30: A custom command in a session whose directory is gone runs in HOME (#740)

**Upstream:**
- implementation (commit `e6fadf0`): `agterm/Commands/CustomCommandRunner.swift`
- tests: `agtermTests/CustomCommandRunnerTests.swift` (testALocalSessionWhoseDirectoryIsNotThereRunsTheCommandInHome;
  testSpawnFailurePostsOnlyWhenOptedIn and testKeymapRunReportsACommandThatDidNotStart now use a 0o000 directory)
- contract: `.claude/rules/keymap.md` (`AGT_SESSION_PWD` bullet); `site/commands.html` `keymap.run`

**Rust side:** `crates/ngterm-core/src/commands/keymap.rs` (the "session directory {} no longer exists" refusal goes),
`crates/ngterm-core/src/commands/tests/keymap.rs`, `conformance/tests/protocol/test_keymap_hooks.py`

**Platforms:** no OS-specific code (`Path::is_dir`; HOME from the library's home); custom commands on Windows are
separate open work (the existing `cfg_attr(windows, ignore)`)

**Conformance:**
- existing: `tests/protocol/test_keymap_hooks.py::test_run_starts_the_command_for_the_addressed_session`,
  `test_run_refusals[*]` (listed)
- new: `test_keymap_hooks.py::test_run_in_a_removed_session_directory_starts_in_home` (covers `keymap.run`: a session
  created in a temp dir that is then removed; a command writing `$AGT_SESSION_PWD|$PWD` gives the removed path and
  the instance's HOME, resolved through symlinks)

- [ ] the cwd is the session's reported directory when it is a directory here, else HOME, for every session;
      `AGT_SESSION_PWD` keeps the reported path; a real launch failure still reports "<name> did not start: <reason>"
      and the opted-in panel "<name>: <reason>"
- [ ] write tests: replace `a_session_directory_that_is_gone_refuses_the_run` with a run in HOME (a missing path and a
      plain file); rework the failure-panel test onto the fake launcher's `fail_with` (quiet: no panel; loud: one
      panel, "loud: " prefix)
- [ ] write tests: the conformance test above
- [ ] manual check (user): green on reference-macos
- [ ] gate passes; newly green ids added with `check_ratchet.py --add`

### Task 31: `notify` banners get their own notification slot per session (#772)

**Upstream:**
- implementation (commit `5a7c6a8`): `Notifications.swift` (`controlIdentity`, `sessionIdentities`, `parseIdentity`
  reads the `notify` suffix as the main pane), `agterm/Notifications/NotificationManager.swift`
- tests: `NotificationsTests.swift` (controlIdentityFormatIsWindowColonSessionColonNotify,
  controlIdentityIsStableAndNeverCollidesWithPaneIdentities, controlIdentityParsesToTheMainPane,
  sessionIdentitiesCoverEveryPaneAndTheControlSlot, parseRejectsMalformed with `…:notify:1` and `…:not-a-uuid:notify`)
- contract: `site/commands.html` `notify` ("A session keeps one such banner: a newer notify replaces it, the
  session's terminal notifications do not, and focusing the session removes it."); `.claude/rules/notifications.md`,
  `control-api.md`

**Rust side:** `crates/ngterm-core/src/commands/notify.rs` (`identity`/`parse_identity`, `withdrawn_identities`, the
post's slot), `crates/ngterm-gpui/src/notifications.rs` (module doc; the sink already posts and withdraws by identity
with `replaces_id`), `crates/ngterm-gpui/tests/notifications.rs`

**Platforms:** Linux: freedesktop `Notify` with `replaces_id` per identity (already in the sink) and
`CloseNotification` on clear. macOS: the `UNNotificationRequest` identifier is the identity,
`removeDeliveredNotifications(withIdentifiers:)` the session identities. Windows: a toast's Group = the session id and
Tag = the pane role or `notify` (both ≤ 64 characters; the window id in the toast's launch args), cleared with
`ToastNotificationHistory.RemoveGroup`. The identity logic is in ngterm-core (cross-platform CI); the macOS and Windows
sinks come with the frontend on those OSes.

**Conformance:** none in the suite (its private bus has no notification server); the GPUI integration test over its
fake freedesktop server instead

- [ ] `control_identity(window, session)` = "<windowID>:<sessionID>:notify"; `session_identities` = the three pane
      identities plus the control one; `parse_identity` reads `notify` as the main pane (a click reveals it), every
      other malformed form stays None
- [ ] `notify` posts under the control slot; a clear withdraws `session_identities`
- [ ] write tests: the four Swift cases and the two malformed strings; `notify` queues a post whose identity ends in
      ":notify"; the GPUI integration test: a `notify` then an OSC 9 from the main pane give two banners (no
      `replaces_id` between them), a second `notify` replaces the first, focusing the session closes the control
      banner too, a click reveals the main pane
- [ ] manual check (user): green on `cross-platform.yml` (ngterm-core)
- [ ] gate passes

### Task 32: The selection clipboard on macOS and Windows (#792)

**Upstream:**
- implementation (commit `bca6ad8`): `agterm/Ghostty/GhosttyCallbacks.swift` (`pasteboard(for:)`, `clipboardText(for:)`,
  `writeClipboard(…location…)`), `agterm/Ghostty/GhosttyApp.swift`
- tests: `agtermTests/GhosttyCallbacksTests.swift` (testSelectionWriteLandsOnPrivatePasteboardAndLeavesGeneralAlone,
  testStandardWriteLandsOnGeneralAndLeavesSelectionAlone, testPrimaryWriteLandsOnGeneral,
  testClipboardTextReadsTheLocationsPasteboard, testSelectionPasteboardIsNamedForThisBundle)
- contract: `.claude/rules/libghostty.md` (clipboard location); `site/docs.html` (the selection clipboard paragraph)

**Rust side:** `crates/ngterm-gpui/src/terminal.rs` (`run_effect`'s location mapping behind `cfg`),
`terminal/clipboard.rs`, `terminal/mouse.rs`, `vendor/gpui-pre` through its patch (a macOS named pasteboard: GPUI
has only `read/write_to_find_pasteboard` there; `read_from_primary`/`write_to_primary` exist under Linux/FreeBSD
only), `terminal/input_tests.rs`

**Platforms:** Linux: PRIMARY (Wayland primary selection, X11 PRIMARY), already done (Context). macOS: Selection maps
to a private `NSPasteboard(name: "<app id>.selection")`, Standard and OSC 52 to `.general` (upstream's mapping).
Windows: no primary selection: Selection maps to an app-wide in-process buffer, so `primary-paste` pastes the last
selection inside ngterm, and Standard to the Win32 clipboard. Not built on macOS/Windows by CI until ngterm-gpui joins
`cross-platform.yml`. Defaults: upstream's older libghostty defaults `copy-on-select = true` (its selection pasteboard)
on macOS; this build follows the Ghostty pin, whose macOS and Windows default is `none` and `true` means
`clipboard` (`ngterm-core/src/ghostty_config/model.rs`). Keep the pin's default, so upstream's "middle-click pastes
the last selection" needs `copy-on-select = primary` there, and record the deviation in rust-build.md; or ship
`copy-on-select = primary` for macOS and Windows in the bundled defaults (`ghostty_config/mod.rs` `defaults_text`).
Decide in the task, with the reason.

- [ ] macOS: Selection reads and writes go to the private `<app id>.selection` pasteboard, Primary and Standard to
      the general one
- [ ] Windows: an in-process selection buffer for Selection and Primary
- [ ] docs (README or rust-build.md, per OS): "Selecting text with the mouse copies it to a private selection
      clipboard, which a middle-click pastes, and leaves the system clipboard alone; ⌘C puts the selection on the
      system clipboard. To have every selection land on the system clipboard as well, set `copy-on-select =
      clipboard`, or `false` to copy nothing on select." (PRIMARY on Linux)
- [ ] write tests: in input_tests.rs, a default-config mouse selection leaves `read_from_clipboard()` at its seeded
      value and `copy-on-select = clipboard` puts it there; the five location-mapping cases behind `cfg(macos)` and
      `cfg(windows)`
- [ ] gate passes

### Task 33: OpenCode status hook tests (#734)

**Upstream:** `agterm/Resources/agent-status/opencode/agterm-status.js` (synced in `3c7e1d5`); tests
`agtermCore/Tests/agtermCoreTests/OpenCodeStatusHookTests.swift` (the whole file; the new
replyToOnePromptStaysBlockedWhileAnotherIsPending: busy, asked per_1/que_1/per_2, replied per_1, rejected que_1,
replied per_2 → "active --blink", then "blocked" ×5, then "active --blink"; promptAbandonedAtIdleDoesNotHoldBackLaterReplies
→ "active --blink", "blocked", "completed --auto-reset", "active --blink", "blocked", "active --blink")

**Rust side:** `crates/ngterm-app/tests/opencode_status.rs` (create: run `resources/agent-status/opencode/ngterm-status.js`
under `node` with a recording reporter in place of the wrapper)

**Platforms:** no OS-specific code (a JS plugin run by OpenCode); CI installs node rather than skipping the test

- [ ] port OpenCodeStatusHookTests whole, with the reporter command lines in this build's names
- [ ] add node to the CI jobs that run ngterm-app's tests
- [ ] ➕ record in docs/backlog the same missing twin for the Claude, Codex and OpenCode v2 hook tests, if this task
      does not port them too
- [ ] gate passes

### Task 34: UI-only changes

- [ ] window-close sheet in Live mode (#743, commit `f986cfe`; tests `QuitPromptTests.swift`
      windowCloseNonLiveEndsSessions(none|rerun), windowCloseLiveDropsEndingClaim): `QuitPrompt::window_close_message(sessions,
      mode)` in `crates/ngterm-core/src/first_run.rs` beside `message`; live: "This closes 1 session. The window can be
      reopened from File ▸ Open Window." / "This closes 9 sessions. …"; none and rerun: "This ends 1 running session.
      …" / "This ends 4 running sessions. …"; the mode is the launch decision's active mode, as the quit alert's;
      `crates/ngterm-gpui/src/window.rs` `close_alert` uses it; core unit tests for the five Swift cases, window
      tests for both modes (no OS-specific code)
- [ ] an unhandled Esc on an HTML page never reaches the window (#754, commit `3871300`; tests
      `HtmlOverlayRegistryTests.swift` testEscapeOnAPageWithNoKeyHandlerNeverReachesTheWindow,
      testEscapeStillReachesAPageListeningForItAndNeverReachesTheWindow): on Linux the page view consumes every key it
      forwards and full screen changes only through the app's action, so add the GPUI view test
      `escape_goes_to_the_page_and_no_further` in `crates/ngterm-gpui/src/html_overlay/view/tests.rs` (the engine gets
      one key 0xff1b and its release; a window-level action behind it does not fire; full screen unchanged). macOS:
      record in the macOS page-host work that its WKWebView answers `cancelOperation:` itself; Windows: no default
      Esc-exits-full-screen in Win32, and the future WebView2 host must not re-dispatch an unhandled Esc
      (AcceleratorKeyPressed) to a full-screen handler
- [ ] the selected sidebar row with a custom theme that sets no selection colors (#746, commit `f6cf8bf`; tests
      `GhosttyConfigTextTests.swift`, `ThemeNameTests.swift`): the lookup is already the Rust ghostty config's (it
      finds a theme by path, `~/`, user then bundled dirs, XDG); add the fallback: selected text = selection fg, else
      the contrast of selection bg, else the theme foreground (`crates/ngterm-core/src/chrome.rs`), and with no
      selection background the wash is the theme foreground at WASH_ALPHA / INACTIVE_WASH_ALPHA instead of white
      (`crates/ngterm-gpui/src/chrome.rs` `selected_row`); remove the dead `resolve_selection_colors` and
      `config_key_values` (no caller) or keep them if one is found; tests: a light theme with no selection colors in
      `selected_rows_take_the_selection_colors`, and loader tests for any ThemeName/GhosttyConfigText case not yet
      covered (`~/` on a light:/dark: side, quoted sides, `~` and `~user` left alone, a relative path refused, an
      empty XDG_CONFIG_HOME = unset; a later invalid value keeps the earlier one; an empty value resets; an explicit
      reset beats the theme) (no OS-specific code)
- [ ] `cmd` chords through the Command table on macOS (#796, commit `ebc5403`): when GPUI's macOS native key record
      is built, the produced character for a chord holding Cmd is translated with only Command applied
      (`UCKeyTranslate` with `cmdKey`, upstream's `characters(byApplyingModifiers: modifierFlags ∩ .command)`), so
      `cmd+d` on Dvorak - QWERTY ⌘ is the QWERTY D key and `cmd+ą` no longer matches; record it with the frontend's
      macOS keyboard form; a host-free unit test that `chord_key` takes the Command-translated character for a cmd
      chord (fed as data); keymap.md's manual checklist (`cmd+d` and `cmd+1` on Dvorak - QWERTY ⌘ and Lithuanian) for
      the macOS form. Linux: no counterpart (XKB has no per-modifier table); Windows: no counterpart (`ToUnicodeEx`
      without Ctrl/Alt; the Win key selects no table)
- [ ] optional (#747): a GPUI deck window test that with many pages every hidden page's terminal is laid out at the
      deck's bounds and a selection or size change re-creates no view
- [ ] gate passes

### Task 35: Claim v0.38.1 parity and update the docs

- [ ] bump `PARITY_LEVEL` in `crates/ngterm-proto/src/version.rs` and its mirrors (`tag-mentions.sh v0.36.0` class
      `claim`: the version.rs test, `ngterm-core/src/dispatch.rs` and `ngterm-server/tests/server.rs` expectations,
      `conformance/agterm_conformance/impls/rust.py`, `conformance/docs/rust-build.md`), only once every task above is
      done
- [ ] README.md, CLAUDE.md, conformance/README.md, conformance/docs/rust-build.md describe what changed (new commands
      `recent`, `session wait`, `workspace flag`, `session overlay hide|show|toggle|page-serve`, ask forms, point sizes,
      events `app.launched` and `pane.focused`, `live-sessions` on fish, the state-dir lock, the installed identity
      file, the keyboard-layout lock); `bash .claude/skills/agterm-upstream-sync/scripts/tag-mentions.sh v0.36.0`
      lists no `review` or stale `claim` line
- [ ] gate passes
