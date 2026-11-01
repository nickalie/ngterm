# Upstream sync v0.38.1 -> v0.39.1

## Overview

- Bring ngterm from upstream `umputun/agterm` v0.38.1 to v0.39.1 (45 commits; releases v0.39.0, v0.39.1).
- The mechanical bump is done on branch `upstream-sync-v0.39.1` (commit `c8f16b00`): parity-target mentions and the
  reference adapter's `PARITY_LEVEL`. Upstream's pins didn't move.
- Three parts of the bump didn't apply cleanly and are tasks here: the catalog generator (Task 1), the agent-status
  copy (Task 20) and the bundled skill (Task 21).
- Order of the plan:
  - the catalog and the new wire names (Task 1);
  - the smaller features (Tasks 2–12);
  - companion sessions in seven parts (Tasks 13–19);
  - the vendored copies, the split divider, Reduce Transparency and the UI-only items (Tasks 20–24);
  - the parity claim (Task 25).
- Not ported:
  - upstream CI, packaging (#801), release metadata and plugin manifests;
  - upstream's README, ARCHITECTURE, CLAUDE.md and site pages other than commands and docs;
  - the cookbook recipes (#805, #806, #830, 08ccd78), which the bundled skill fetches from upstream;
  - upstream's Ghostty teardown backport (#819), since this build doesn't use libghostty's App/Surface;
  - upstream's docs/backlog and completed plans.

## Context

- **Upstream range:** `.upstream/agterm` at v0.39.1; read a file's diff with
  `git -C .upstream/agterm diff v0.38.1 v0.39.1 -- <path>`. The release notes are in `.upstream/agterm/CHANGELOG.md`.
- **Design intent** (never a target) is in upstream's `docs/plans/completed/`: `20261028-companion-sessions.md`,
  `20261027-flagged-view-reorder.md` and `20261024-palette-membership-editors.md`.
- **Naming:**
  - upstream's `zmx.*` is `live.*` here (`conformance/docs/rust-build.md` > The `live` group);
  - upstream's session host is `ngterm-session`;
  - "Mac" in a user string stays verbatim; `agterm`/`agtermctl` in one becomes `ngterm`/`ngtermctl`.
- **Unsupported-host refusals:** Swift's `ControlActions` default overloads that refuse on an older host ("... is not
  supported on this platform / by this control host") have no target here, because one dispatcher serves both the
  app and `ngterm-headless`. Note that in the module docs and don't port the string.
- **`gen_catalog.py --tag v0.39.1` refused** (Task 1 fixes this):
  - `site/commands.html:1121`: `session duplicate` maps to both `session.duplicate` and `session.companion.duplicate`;
  - `:1150`: `session new` maps to both `session.new` and `session.companion.new`;
  - `:2137`: `session overlay open` maps to both `session.overlay.open` and `session.companion.page`.
- **Already in the Rust build** (no port task):
  - **Generated config written before the first terminal loads it (5b5c437):**
    - the app: `crates/ngterm-gpui/src/app.rs:311` comes before `:322`;
    - headless: `crates/ngterm-headless/src/lib.rs:219` comes before `:266`;
    - Task 22 adds the tests.
  - **Tooltips while the pointer moves (#816):** GPUI's tooltip timer doesn't restart on movement
    (`vendor/gpui-pre/src/elements/div.rs:3814-3870`). This is an AppKit workaround with no counterpart.
  - **Closing a busy terminal froze every window (#819):** this build's pane worker never blocks posting to the
    main thread (`crates/ngterm-gpui/src/terminal.rs:737`, `crates/ngterm-term/src/pane.rs:269`, `:693-720`). Task 22
    adds the cross-OS regression test.
- **Ratchet ids for behavior upstream reversed:** Task 12 rewrites these and swaps their ids by hand in both ratchet
  lists, because `check_ratchet.py --add` never removes an id:
  - `tests/protocol/test_zmx.py::test_new_command_interrupted_by_ctrl_c_leaves_a_prompt`;
  - `tests/protocol/test_live_any_shell.py::test_an_interrupted_live_command_leaves_the_login_shell_prompt`.

  Separately, Task 10 reorders `tests/protocol/test_overlay.py::test_hide_show_and_toggle_keep_the_overlays_running`
  so that it stays green.
- **No zsh on the sync host:** ids that need a zsh login shell only run on CI. This affects Tasks 12, 15 and 19.
- **Cross-task seams** (why the order matters):
  - Task 3's tint comes before the companions, because `session new --parent` takes the creation tint.
  - Session fields in the tree and the snapshot follow upstream's order:
    `statusChangedAt`, `lastSelectedAt` (Task 2), `tint`/`tintOn` (Task 3), `parent`/`collapsed`/`content`
    (Task 13), then `background`.
  - Result fields: `undoable` (Task 4), then `parent` and `closed` (Tasks 13–14).
  - The BuiltinAction count pin (50 today) grows in Tasks 8 (+2), 16 (+1) and 18 (+1), ending at upstream's 54.
    The palette catalog count grows in Tasks 3, 8, 16 and 18.
  - Task 6 builds on Task 5's stop predicate.
  - Task 12's Live `--command` ending comes before Tasks 15 and 18.
  - The main checkout has another session's uncommitted HTML-overlay work. Tasks 11 and 17 touch the same
    directories, so expect a merge there.

## Approach

- Each task follows CLAUDE.md > Porting a command group. Upstream's Swift sources and tests hold the exact strings.
- Size each task's checks to what it changed (CLAUDE.md > Choosing checks). Add the newly green conformance ids to
  the ratchet.
- A new conformance test of v0.39 behavior carries `@since("0.39.1")`. It skips on the Rust build until Task 25
  claims the level, so its ids go into the ratchet then.
- macOS-API behavior gets a form per OS, or a reason why an OS has none (CLAUDE.md > Platforms). A task's
  **Platforms** note says which.
- The user runs `reference-macos.yml` and the cross-platform CI on the pull request. Tasks list no manual checks.

## Implementation Steps

### Task 1: Catalog v0.39.1 and the new wire names, fields and CLI skeletons

**Upstream:** `site/commands.html` (the cards at lines 1121, 1150, 2137 and every new card), `agtermCore/Sources/agtermCore/ControlProtocol.swift`,
`ControlProjection.swift`, `ControlEvents.swift`, `ControlResponse.swift`, `agtermCore/Sources/agtermctlKit/*` (for the
CLI tree), `agterm/**/*.swift` (ui ids).

**Rust side:** `conformance/tools/gen_catalog.py` and its selftests `conformance/selftests/test_gen_catalog.py`
(modify); `conformance/catalog.yaml`, `conformance/ui-ids.yaml` (regenerated); `crates/ngterm-proto/src/command.rs`,
`request.rs`, `result.rs`, `tree.rs`, `events.rs` (modify); `crates/ngterm-core/src/dispatch.rs` (modify: the new
commands answer the refusal it gives any command it does not implement yet); `crates/ngterm-ctl/src/*` (modify: the
new subcommands and flags, parse only, sending what upstream sends).

**Conformance:** `ngterm-proto/tests/catalog.rs` and ngterm-ctl's `the_command_tree_is_the_catalogs_cli` must be
green at the end; the coverage gate lists the new keys, all owned by later tasks.

- [x] teach `gen_catalog.py` a CLI subcommand that sends one of several wire commands depending on a flag
      (`session duplicate` → `session.duplicate` | `session.companion.duplicate` with `--companion`; `session new` →
      `session.new` | `session.companion.new` with `--parent`; `session overlay open` → `session.overlay.open` |
      `session.companion.page` with `--companion`), in whatever catalog shape keeps the existing `cli:` keys stable
      and lets the coverage gate name both wires; selftests for it in `test_gen_catalog.py`
- [x] run `.venv/bin/python conformance/tools/gen_catalog.py --tag v0.39.1 --repo .upstream/agterm`; keep its reflow
      of hand-written exemption lines; then `coverage-map.py --since HEAD~1` lists every added/changed key: record
      that list in this task's commit message
- [x] `Command` variants: `session.companion.new`, `session.companion.duplicate`, `session.companion.page`,
      `session.companion.page.attach`, `session.collapse`, `session.expand`, `session.tint`, `workspace.tint` (and any
      other the regenerated catalog lists that the enum lacks)
- [x] `Args`: `tint`, `tintOn`, `before`/`after` on `session.flag`, `undoable`, `parent`-side args the companions take
      (whatever the catalog lists); `ControlResult`: `undoable`, `parent`, `closed`, `companionPage` in upstream's
      order; tree session node `lastSelectedAt`, `tint`, `tintOn`, `parent`, `collapsed`, `content`; workspace node
      `tint`, `tintOn`; top level `flaggedOrder`, `attentionIncludesActive`, `attentionNavigationAcrossWindows`; event
      payload `parent`; all optional and omitted when `None` (Context > Cross-cluster seams has the order)
- [x] CLI skeletons: `session tint`, `workspace tint`, `session collapse`, `session expand`, `session overlay
      page-attach`, flags `session new --parent/--tint/--tint-on`, `workspace new --tint/--tint-on`, `session flag
      move --before/--after`, `session close --undoable`, `session duplicate --companion --no-select`, `session
      overlay open --companion`, `browser links` new modes (parse and send only; validation strings come in each
      feature's task) (`browser links` unchanged: it already sends any MODE, and only its `LinkOpenMode` check,
      Task 9's, refuses the new ones)
- [x] write tests: proto round-trip and omission of every new field; the key-order lists in `tree.rs` tests
- [x] checks sized to the change pass; newly green ids added to the ratchet (none: no new behavior; the coverage
      gate and its selftest twin fail only on the 35 new keys, all later tasks')

### Task 2: `tree` `lastSelectedAt` per session (#818, 21dc990)

**Upstream:** `agtermCore/Sources/agtermCore/Session.swift`, `AppStore.swift` (selectedSessionID didSet),
`AppStore+Snapshot.swift`, `Snapshot.swift`, `AppStore+PendingClose.swift` (21dc990), `ControlProjection.swift`.
Tests: `AppStoreTreeProjectionTests.swift` (`selectionStampsTheSessionEnteredAndTheSessionLeft`, the projection with
1_600_000_000 / 1_700_000_100), `SnapshotRoundTripTests.swift` (`lastSelectedAtSurvivesARestoreThatReselectsTheSession`,
`snapshotWithoutLastSelectedAtDecodesNil`), `AppStorePendingCloseTests.swift`
(`batchCloseStampsTheActiveSessionLeftWhenUndoSelectsABatchMate`). Contract: `commands.html` tree session fields,
`.claude/rules/control-api.md` (session.selected bullet).

**Rust side:** `crates/ngterm-core/src/model/session.rs`, `model/store.rs` (`set_selected`), `model/pending_close.rs`
(`soft_close_sessions`), `persist/snapshot.rs`, `model/snapshot.rs`, `projection.rs`, `crates/ngterm-proto/src/tree.rs`
(modify).

**Conformance:** new `tests/protocol/test_tree.py::test_last_selected_at_stamps_the_session_entered_and_left` and
`tests/behaviors/test_relaunch.py::test_last_selected_at_survives_a_relaunch`, covering
`field:tree.session.lastSelectedAt` (key name as the regenerated catalog spells it).

- [x] `Session.last_selected_at` (wall clock, the clock of `statusChangedAt`), stamped on the session entered and
      the one left with one `now` at every selection change, never while restoring
- [x] a batch soft close stamps the selected member before removal, so an undo selecting a batch mate still records
      the departure
- [x] snapshot key `lastSelectedAt` (epoch seconds, fractional) after `context`, omitted when nil, lossy decode; a
      restore keeps the stored value
- [x] tree session node `lastSelectedAt` after `statusChangedAt`, omitted when nil
- [x] write tests: the five Swift cases above on the fakes
- [x] write tests: the two conformance tests (`@since("0.39.1")`)
- [x] checks sized to the change pass; newly green ids added to the ratchet (none yet: both new ids are
      `since("0.39.1")` and skip until Task 25; green on a local run at a temporary 0.39.1 level)

### Task 3: Tinted session and workspace rows (#836)

**Upstream:** `agtermCore/Sources/agtermCore/SidebarTint.swift`, `AppStore+Tint.swift`,
`ControlDispatcher+SessionMetadata.swift`, `ControlModes.swift` (ControlTintUpdate), `ControlProtocol.swift`,
`ControlProjection.swift`, `AppStore.swift`, `AppStore+Snapshot.swift`, `AppStore+PendingClose.swift`, `Snapshot.swift`,
`Session.swift`, `Workspace.swift`, `agtermctlKit/TintCommands.swift`, `SessionCommands.swift`, `WorkspaceCommands.swift`,
`agterm/Control/ControlServer+Tint.swift`, `ControlServer+SessionActions.swift`, `ControlServer+WorkspaceCommands.swift`,
`agterm/AppActions+Palette.swift`, `agterm/Views/SidebarRowViews.swift`, `TintColor+AppKit.swift`,
`WorkspaceSidebar+TintMenu.swift`, `WorkspaceSidebar+ContextMenu.swift`, `WorkspaceSidebar+RowRendering.swift`.
Tests: `AppStoreTintTests.swift` (all 10), `SidebarTintTests.swift` (all 9), `ControlDispatcherSessionMetadataTests.swift`
(the tint cases), `SnapshotRoundTripTests.swift` (`anUnknownTintColorDropsOnlyTheTint`, `anUntintedTreeWritesNoTintKey`),
`agtermctlKitTests/TintCommandsTests.swift` (all 8), `agtermTests/ControlServerTintTests.swift` (all 5),
`WorkspaceSidebarTintMenuTests.swift` (all 7), `AppActionsPaletteTests.swift`
(`testTintRowsTintTheSessionSelectedWhenTheyRun`), `agtermUITests/ControlAPITintUITests.swift`. Contract:
`commands.html` (`session tint`, `workspace tint`, `session new`, `workspace new`, tree `tint`/`tintOn`, the
`tree.changed` triggers), `.claude/rules/control-api.md`, `sidebar.md`.

**Rust side:** `crates/ngterm-core/src/model/tint.rs` (create), `commands/tint.rs` (create: the shared
`tint_update`/`creation_tint` parse), `model/session.rs`, `model/workspace.rs`, `model/store.rs` (`NewSession.tint`,
`WorkspaceOptions.tint`), `model/snapshot.rs`, `persist/snapshot.rs`, `model/pending_close.rs`
(`rebuilt_workspace_shell`), `model/recent_closed.rs`, `projection.rs`, `dispatch.rs`, `commands/session.rs`,
`commands/workspace.rs`, `sidebar_actions.rs`, `palette/{mod,feeds,run}.rs` (modify); `crates/ngterm-ctl/src/tint.rs`
(create), `session.rs`, `workspace.rs` (modify); `crates/ngterm-gpui/src/sidebar/{menu,context,rows,look,view,tests}.rs`
(modify). Beware grep traps: "tint" also matches the chrome's sidebar wash, the status glyph's `#rrggbb` color,
`row_icon` mask coloring, the watermark and the WPE bindings.

**Conformance:** existing `tests/protocol/test_session.py::test_duplicate_inserts_a_plain_copy_after_the_source`,
`tests/behaviors/test_relaunch.py::test_relaunch_keeps_workspaces_sessions_and_their_metadata`,
`tests/protocol/test_tree.py::test_fresh_session_node`, `tests/cli/test_cli_output.py::test_session_metadata` (listed;
extend, keep green). New: `test_session.py::test_tint_set_recolor_place_and_clear`,
`::test_tint_rejects_and_keeps_the_previous_tint`, `::test_new_carries_a_creation_tint`,
`::test_new_refuses_a_bad_creation_tint_before_creating`; `test_workspace.py::test_workspace_tint_colors_only_its_row`;
`test_recent.py::test_reopening_into_a_deleted_workspace_rebuilds_its_tint`; `tests/cli/test_cli_output.py::test_tint_commands`,
`::test_tint_refusals_at_parse_time`; `tests/ui/test_sidebar_rows.py::test_tinted_icon_names_its_tint`.

- [x] TintColor (red, orange, yellow, green, blue, purple, gray) and TintPart (icon, name); SidebarTint parts never
      empty, ordered icon then name; recolor keeps parts and starts an untinted row on the icon; place leaves an
      untinted row alone; parse_parts trims, lowercases, refuses empty or unknown
- [x] `session.tint`/`workspace.tint` (mode set|clear, args `tint`, `tintOn`), checks before the target resolves:
      "<cmd> set requires a color", "invalid tint: <raw> (red|orange|yellow|green|blue|purple|gray)", "invalid tint
      placement: <parts> (icon,name)", "<cmd> clear takes no color or placement", "invalid tint mode: <mode> (set|clear)";
      a set without `tintOn` keeps the row's parts; unchanged value: no save, no `tree.changed`
- [x] creation tint on `session.new` (every placement path) and `workspace.new`, parsed before anything is created:
      "--tint-on requires --tint" and the same invalid strings; a `--create-workspace` workspace does not take the
      session's tint; duplicate starts untinted
- [x] tree `tint`/`tintOn` on session and workspace nodes, absent when untinted; snapshot
      `{"color":"red","on":["icon"]}`, lossy (an unknown color drops only the tint); pending-close and Open Recent
      rebuild a removed workspace with its tint
- [x] CLI `session tint <COLOR|clear> [--on PARTS]`, `workspace tint`, `--tint/--tint-on` on both `new`s, with
      upstream's help and its errors: "tint must be one of: red|orange|yellow|green|blue|purple|gray, or clear", "--on
      does not apply to clear", "--on must list icon and/or name, comma-separated", "--tint-on requires --tint", "--tint
      must be one of: red|orange|yellow|green|blue|purple|gray", "--tint-on must list icon and/or name, comma-separated"
- [x] GPUI: icon and/or label in the tint color (light/dark by the chrome background's darkness; AppKit's system
      colors: red #FF3B30/#FF453A, orange #FF9500/#FF9F0A, yellow #FFCC00/#FFD60A, green #28CD41/#32D74B, blue
      #007AFF/#0A84FF, purple #AF52DE/#BF5AF2, gray #8E8E93/#98989D), kept on selected rows; accessibility value
      "<color> tint" on the icon; the row menu's Tint submenu (None, the seven colors with swatches, Show On: Icon,
      Name, Icon and Name; checks only what every target shares; Show On disabled unless every target is tinted) on
      session rows after Flag and on workspace rows after Flag/Unflag All; palette rows "Tint Session: <Color>" and
      "Tint Session: None" reading the selection at run time
- [x] ngterm's skill copy mentions are Task 21's (left for Task 21)
- [x] write tests: every Swift case listed above as Rust unit tests (core, ctl, gpui sidebar menu model and render)
- [x] write tests: the conformance tests above
- [x] checks sized to the change pass; newly green ids added to the ratchet (none yet: the new ids are
      `since("0.39.1")` and skip until Task 25; all 27 tint ids and the four extended ones green on a local run at a
      temporary 0.39.1 level)

### Task 4: `session close --undoable` (#804)

**Upstream:** `agterm/Control/ControlServer+SessionClose.swift` (moved code plus the undoable branches),
`agtermCore/Sources/agtermCore/ControlDispatcher.swift`, `ControlProtocol.swift`, `agtermctlKit/SessionCommands.swift`,
`SocketClient.swift` (formatResponse). Tests: `ControlDispatcherSessionCloseTests.swift` (all 4),
`agtermctlKitTests/SessionCloseCommandsTests.swift` (all 3), `SocketClientTests.swift` (`formatResponseUndoableClose`),
`agtermTests/ControlServerSessionCloseTests.swift` (all 5), `agtermUITests/ControlSessionCloseUITests.swift`. Contract:
`commands.html` `session close`, `.claude/rules/control-api.md`.

**Rust side:** `crates/ngterm-core/src/commands/session.rs` (`close`), `commands/tests/session.rs`;
`crates/ngterm-ctl/src/session.rs`, `output.rs` (modify).

**Conformance:** existing `test_session.py::test_close_removes_the_session_and_reselects`,
`::test_close_requires_a_target`, `::test_close_several_targets_at_once`, `::test_close_several_targets_is_atomic`,
`test_cli_output.py::test_session_create_select_and_close` (listed). New: `test_session.py::test_close_undoable_keeps_the_running_shell_for_reopen`,
`::test_close_undoable_is_hard_and_says_so_when_undo_is_off`, `::test_close_undoable_batch_reports_whether_the_group_stayed_undoable`,
`::test_plain_close_carries_no_undoable`; `test_cli_output.py::test_session_close_undoable`, covering
`field:result.undoable`.

- [x] single target (or a batch deduplicated to one) with `undoable` and close grace on: soft close as the close
      chord does, `{"id", "undoable": true}`; "could not close session" when the soft close fails
- [x] grace off: hard close, `{"id", "undoable": false}`; without `undoable`: unchanged, no field
- [x] a batch of two or more is unchanged; with `undoable` the reply adds `undoable: grace_on && affected > 0`;
      "session.close requires at least one --target" still first
- [x] CLI `--undoable` (help "Give a single close ⌘W's undo grace, so undo inside it brings the running shell back;
      closes at once when Settings turns undo off."), output `<ok|N sessions> (undoable|not undoable)`
- [x] write tests: the Swift cases above (core, ctl output)
- [x] write tests: the conformance tests above
- [x] checks sized to the change pass; newly green ids added to the ratchet (none added: the five new ids are
      since(0.39.1), green at a temporary 0.39.1 level; Task 25 lists them)

### Task 5: Attention navigation includes active sessions (#807)

**Upstream:** `agtermCore/Sources/agtermCore/AppSettings.swift`, `AppStore.swift` (attentionTarget, controlTree),
`ControlProjection.swift`, `agtermctlKit/SessionCommands.swift` (Go help), `agterm/SettingsModel.swift`,
`agterm/Views/SettingsView.swift`. Tests: `AppSettingsTests.swift` (`attentionIncludesActiveDefaultsNilAndOmitsFromJSON`,
`attentionIncludesActiveRoundTripsAndIsNotAConfigLine`), `AppStoreAutoFollowTests.swift`
(`controlTreeProjectsAttentionIncludesActive`, `controlTreeOmitsANilAttentionIncludesActive`), `AppStoreNavigationTests.swift`
(`navigateAttentionStopsOnActiveWhenIncluded`), `agtermUITests/AttentionNavigationUITests.swift`
(`testSessionGoNextAttentionStopsOnActiveWhenIncluded`), `SettingsUITests.swift`. Contract: `commands.html` (tree,
`session go`), `.claude/rules/control-api.md`, `settings.md`, `menu-actions.md`.

**Rust side:** `crates/ngterm-core/src/persist/settings.rs`, `settings_model.rs`, `model/navigation.rs` (a shared
`stops_attention_navigation(session, includes_active)`), `commands/session.rs` (`go`), `projection.rs`;
`crates/ngterm-proto/src/tree.rs`; `crates/ngterm-ctl/src/session.rs`; `crates/ngterm-gpui/src/settings/agent_status.rs`,
`settings/tests.rs` (modify).

**Conformance:** existing `test_session.py::test_go_steps_through_sessions_and_wraps`, `test_tree.py::test_auto_follow_timeout_reads_back_from_settings`
(listed). New: `test_tree.py::test_attention_includes_active_reads_back_from_settings`,
`test_session.py::test_go_next_attention_stops_on_active_when_included`,
`test_session.py::test_go_attention_steps_skip_idle_and_active`; `tests/ui/test_settings.py::test_agent_status_tab`
gains `ui:settings-attention-includes-active`.

- [x] settings.json `attentionIncludesActive` (nil = off, omitted when nil, no ghostty config line; Reset leaves it)
- [x] the attention steps (`session go --to next-attention|prev-attention`, the GUI chord/menu/palette) stop on
      active sessions too while on; `needs_attention()` unchanged, so auto-follow, the bell and pane reveal are not
- [x] tree top-level `attentionIncludesActive` after `autoFollowMs`, always present from the app
- [x] `session go` help: "Direction: next, prev, first, last, next-attention, or prev-attention (attention =
      blocked/completed, plus active when enabled in Settings)."
- [x] Settings ▸ Agent Status: the section "Auto-follow" becomes "Navigation" and starts with the toggle "Attention
      navigation includes active sessions" (`settings-attention-includes-active`)
- [x] write tests: the Swift cases above (settings, projection, navigation `next → 1, next → 2, next → 1, prev → 2`)
- [x] write tests: the conformance tests above
- [x] checks sized to the change pass; newly green ids added to the ratchet

### Task 6: Cross-window attention navigation (#824)

**Upstream:** `agtermCore/Sources/agtermCore/WindowLibrary+Attention.swift`, `AppStore+Status.swift`, `AppStore.swift`,
`AppSettings.swift`, `ControlProjection.swift`, `PaletteCatalog.swift`, `agterm/AppActions+Navigation.swift`,
`AppActions+Palette.swift`, `SettingsModel.swift`, `Views/SettingsView.swift`. Tests:
`WindowLibraryAttentionTests.swift` (all 11), `agtermTests/AppActionsNavigationTests.swift` (all 10),
`AppSettingsTests.swift` (`attentionNavigationAcrossWindowsDefaultsNilAndRoundTrips`), `AppStoreAutoFollowTests.swift`
(`controlTreeProjectsAttentionNavigationAcrossWindows`), `PaletteCatalogTests.swift`
(`attentionStepsAcrossWindowsNeedNoActiveSession`), `AttentionNavigationUITests.swift`
(`testCrossWindowSettingReachesEveryWindowStore`). Contract: `commands.html` (tree, `session go`), `control-api.md`,
`settings.md`, `menu-actions.md`.

**Rust side:** `crates/ngterm-core/src/persist/settings.rs`, `settings_model.rs`, `model/window_library.rs`
(`attention_step`), `app_actions.rs` (a `Core::step_attention` for the Previous/Next Attention built-ins, instead of
today's plain `session.go` request at `app_actions.rs:474-477`), `palette_catalog.rs`, `projection.rs`;
`crates/ngterm-proto/src/tree.rs`; `crates/ngterm-gpui/src/settings/agent_status.rs` (modify). The raise reuses
`Core::select_attention` (`crates/ngterm-core/src/palette/run.rs:144`).

**Platforms:** raising the other window reuses `WindowRequest::Raise` → GPUI `activate_window`: Wayland asks
xdg_activation_v1 (`vendor/gpui-pre-linux/src/linux/wayland/window.rs:1863`) and falls back to the compositor's
attention hint when refused (Mutter, KWin focus-stealing prevention), the model still moving frontmost and the
selection; X11 sends `_NET_ACTIVE_WINDOW` (urgent hint when the WM refuses); Windows restores and
`SetForegroundWindow` (`vendor/gpui-pre-windows/src/window.rs:815`), allowed since ngterm owns the foreground when
the chord fires; macOS `makeKeyAndOrderFront` with the GPUI macOS form (open work). The step logic lives in
`ngterm-core` (`any`); the GPUI part is "not built on macOS/Windows by CI until `ngterm-gpui` joins
`cross-platform.yml`".

**Conformance:** new `test_tree.py::test_attention_across_windows_reaches_every_window`,
`test_session.py::test_go_next_attention_stays_in_its_window_with_cross_window_on`; `test_settings.py::test_agent_status_tab`
gains `ui:settings-attention-across-windows`. The chord's cross-window raise has no protocol driver: unit tests.

- [x] settings.json `attentionNavigationAcrossWindows` (nil = off, omitted when nil; Reset leaves it)
- [x] `WindowLibrary::attention_step(source, forward, eligible)`: windows in library order × each window's navigable
      sessions in sidebar order; a window refusing UI actions (zoom, dashboard, picker) skipped; scan from the source's
      selection, wrap, exclude it; no selection enters at the first/last candidate; Task 5's stop rule
- [x] the GUI step under the setting: same-window target selects at once and reveals its captured pane; another
      window's target raises it and selects there after the palette has dismissed; no target reveals the current
      session's tagged pane. With the setting off: the in-window step plus the reveal and window-activity note (also
      closes today's gap where the chord ran a bare `session.go`)
- [x] `session go --to next-attention|prev-attention` stays within its window either way
- [x] Previous/Next Attention enabled with an active session or with the setting on; still off under zoom, dashboard,
      picker
- [x] tree top-level `attentionNavigationAcrossWindows` after `attentionIncludesActive`
- [x] Settings toggle "Attention navigation crosses windows" (`settings-attention-across-windows`) second in
      Navigation; the hint "Only applies while auto-follow is on." goes
- [x] write tests: the Swift cases above on the fakes (`take_window_requests` holds `Raise(target)`, frontmost moves)
- [x] write tests: the conformance tests above
- [x] checks sized to the change pass; newly green ids added to the ratchet (none: the three new
      tests are since(0.39.1); green at a temporary 0.39.1 level, they join the ratchet at Task 25)

### Task 7: The flagged view's saved order and `session flag move` (#833)

**Upstream:** `agtermCore/Sources/agtermCore/AppStore+FlaggedOrder.swift`, `AppStore.swift`, `AppStore+Snapshot.swift`,
`AppStore+SessionState.swift`, `AppStore+PendingClose.swift`, `AppStore+RecentClosed.swift`, `AppStore+CloseReselection.swift`,
`AppStore+Focus.swift`, `Snapshot.swift`, `SidebarDrop.swift`, `WindowLibrary.swift`, `WindowLibrary+FlaggedLayout.swift`,
`ControlDispatcher+SessionFlag.swift`, `ControlDispatcher.swift`, `ControlProjection.swift`,
`agtermctlKit/SessionMetadataCommands.swift`, `agterm/Control/ControlServer+SessionActions.swift`,
`agterm/SettingsModel.swift`, `agtermApp.swift`, `Views/WorkspaceSidebar.swift`, `WorkspaceSidebar+DragDrop.swift`. Tests:
`AppStoreFlaggedOrderTests.swift` (all 20), `SidebarDropTests.swift` (the flat and flagged-tree cases),
`AppStoreNavigationTests.swift` (`flaggedNavigationFollowsTheOrderTheLayoutShows`,
`restoreIntoTheFlaggedViewReselectsTheFirstRowOfTheLayoutsOrder`), `AppStoreCloseReselectionTests.swift` (both new),
`AppStoreTreeProjectionTests.swift` (`controlTreeReportsTheSavedFlaggedOrderAndOmitsItWhenNothingIsFlagged`),
`ControlDispatcherSessionMetadataTests.swift` (the flag-move cases), `WindowLibraryFlaggedLayoutTests.swift` (all 4),
`agtermctlKitTests/SessionMetadataCommandsTests.swift` (all 5), `agtermTests/ControlServerFlaggedLayoutTests.swift`
(all 3), `SettingsModelTests.swift` (`testSetFlaggedViewLayoutReachesTheLibraryAndEveryOpenStore`),
`agtermUITests/ReorderUITests.swift` (3), `ControlSidebarStatusUITests.swift`. Contract: `commands.html` (`session
flag`, tree `flaggedOrder`), `control-api.md`, `sidebar.md`, `menu-actions.md`.

**Rust side:** `crates/ngterm-core/src/model/flagged_order.rs` (create), `model/{mod,store,focus,navigation,pending_close,recent_closed,snapshot,window_library}.rs`,
`persist/snapshot.rs`, `persist/library.rs`, `dispatch.rs` (the layout mirror pushed on a settings change),
`sidebar_drop.rs`, `sidebar_actions.rs`, `commands/metadata.rs`, `projection.rs`; `crates/ngterm-proto/src/tree.rs`;
`crates/ngterm-ctl/src/session_meta.rs`, `tests/cli.rs` (help snapshots and the old refusal string);
`crates/ngterm-gpui/src/sidebar/{rows,drag,view,context}.rs`, `sidebar.rs` (modify).

**Conformance:** existing `test_session.py::test_flag_modes`, `::test_flag_clear_unflags_every_session`,
`::test_flag_rejects_an_unknown_mode`, `test_cli_output.py::test_session_metadata`, `test_workspace.py` flag ids,
`test_sidebar.py::test_flagged_layout_is_app_wide` (listed). New: `test_session.py::test_flag_move_reorders_the_saved_order`,
`::test_flagged_order_follows_flagging`, `::test_flag_move_refusals`, `::test_flag_move_to_the_same_place_answers_ok`;
`test_relaunch.py::test_flagged_order_survives_relaunch`; `test_cli_output.py::test_session_flag_move`.

- [x] per-store `flagged_order`; `ordered_flagged_sessions()` = the saved ids still flagged, then the missing flagged
      ones in tree order; `flagged_sessions()` stays tree order for the flagged tree and counts
- [x] maintenance: flag appends arrivals (flag-all keeps arranged positions), unflag drops, clear empties (parked
      ids too), close and workspace removal drop after reselection, finalize drops, undo appends a cleared id and keeps
      a parked one in its slot, every Recent Closed reopen appends in arrival order
- [x] `move_flagged(ids, placement, anchor)`: refuses an empty block, an anchor in the block, ids outside the flagged
      set; an unchanged order saves nothing and emits no `tree.changed`
- [x] snapshot `flaggedOrder` (upper-case uuids) after `sessionRecency`, omitted when empty, lenient; restore keeps
      flagged, de-duplicated ids and appends the rest
- [x] the flagged layout mirrored into every open store, seeded before its restore picks a selection; navigation
      (`session go`, Ctrl-Tab, ⌃P, restore reselection) follows the displayed order; close reselection takes the
      saved-order neighbour in flat/plain layouts
- [x] dispatcher (before the target resolves): "session.flag move requires exactly one of --before or --after"; any
      other mode with an anchor: "--before and --after apply only to session.flag move"; app side: "a session cannot
      be its own anchor", "session.flag move needs a flagged session and a flagged anchor"; answer `{"id"}` also when
      the order did not change
- [x] tree top-level `flaggedOrder` after `sidebarFlaggedLayout`, omitted when nothing is flagged
- [x] CLI `session flag` gains `move --before|--after`, upstream's help, "mode must be on, off, toggle, clear, or
      move", "--before and --after apply only to move", "move requires exactly one of --before or --after"
- [x] GPUI: flat and plain lists render the saved order; session rows drag in every flagged layout (workspace rows
      only in the ordinary tree); the flat drop → `move_flagged`; the flagged-tree drop through `move_sessions` with
      the no-op check; spring-load wherever workspace rows render
- [x] write tests: every Swift case listed above (core model, sidebar_drop, metadata command, settings model, ctl,
      gpui sidebar drag)
- [x] write tests: the conformance tests above
- [x] checks sized to the change pass; newly green ids added to the ratchet (none: the six new tests are
      since(0.39.1); green at a temporary 0.39.1 level, they join the ratchet at Task 25)

### Task 8: Palette membership editors: Edit Flagged Sessions, Edit Focused Workspaces (#803)

**Upstream:** `agtermCore/Sources/agtermCore/BuiltinAction.swift`, `PaletteCatalog.swift`, `agterm/AppActions+Palette.swift`,
`Views/Palette.swift`, `Views/WorkspaceSidebar.swift`, `agtermApp+Menus.swift`. Tests: `BuiltinActionTests.swift`,
`PaletteCatalogTests.swift` (`flaggedEditorNeedsAnySessionNotAnActiveOne`, `membershipEditorsStayListedInEitherSidebarMode`),
`agtermctlKitTests/SocketClientTests.swift` (the keymap listing), `agtermTests/AppActionsPaletteTests.swift` (the five
editor cases), `PaletteTests.swift` (`testOnlyTheMembershipEditorsStayOpenAfterARun`), `SidebarFlaggedLayoutTests.swift`
(the three empty-state cases), `agtermUITests/FlaggedViewUITests.swift`, `FocusWorkspaceUITests.swift`,
`PaletteUITests.swift`. Contract: `.claude/rules/menu-actions.md`, `keymap.md`, `sidebar.md`, `site/docs.html`.

**Rust side:** `crates/ngterm-core/src/keymap/builtin.rs`, `keymap/tests/builtin.rs`, `palette_catalog.rs`,
`app_actions.rs` (open, not toggle), `palette/{mod,feeds,run,search}.rs`, `commands/tests/palette.rs`;
`crates/ngterm-gpui/src/palette.rs`, `app_menu.rs`, `sidebar.rs`, `sidebar/view.rs` (modify).

**Conformance:** existing `tests/protocol/test_keymap_hooks.py::test_list_without_a_file_reports_the_builtins_alone`
(listed). New: `test_keymap_hooks.py::test_membership_editors_are_keyless_builtins`; ui ids `palette-editor-hint`,
`palette-mark`, `flagged-empty-add` (ui layer).

- [x] built-ins `edit_flagged_sessions`, `edit_focused_workspaces`, keyless; palette commands "Edit Flagged
      Sessions…" (after Clear Flagged, enabled when the window has any session) and "Edit Focused Workspaces…"
      (after Clear Focus, enabled with a current workspace), visible in either sidebar mode
- [x] palette modes FlaggedEditor ("Flag or unflag sessions…") and FocusEditor ("Add or remove focused
      workspaces…"): rows in tree order (focus-hidden ones included) with a live mark; Return or a click toggles and
      the palette stays open on the same query and highlight; an empty query keeps source order; a gone row is inert;
      marking never turns the focus filter on
- [x] footer hint "Return toggles · Esc closes · Changes apply immediately" (+ "\nFilter on|off", ", applies in tree
      view" for the focus editor); View menu items; one opener for the palette row, keymap, menu and empty-state link
- [x] empty flagged view: "No flagged sessions." with an "Add Sessions…" link that opens the session editor and
      hides once something is flagged
- [x] write tests: the Swift cases above (builtin count, catalog order and count, enablement, run keeps the palette
      open, gpui click-then-Return, the empty-state link)
- [x] write tests: the conformance test above
- [x] checks sized to the change pass; newly green ids added to the ratchet (none: the new conformance test is
      `since(0.39.1)`; green at a temporary 0.39.1 level, its id goes in at Task 25)

### Task 9: Links in a 95/75/50% overlay and the browser override chord (#812, #813)

**Upstream:** `agtermCore/Sources/agtermCore/AppSettings.swift`, `LinkPolicy.swift`, `agtermctlKit/MiscCommands.swift`,
`agterm/Control/ControlServer+SessionActions.swift`, `agterm/Ghostty/LinkOpener.swift`, `GhosttySurfaceView+Input.swift`,
`GhosttySurfaceView.swift`, `GhosttyCallbacks.swift`, `Views/SettingsView.swift`. Tests: `LinkPolicyTests.swift` (the
five new cases and the two updated), `ControlDispatcherBrowserTests.swift`, `agtermctlKitTests/MiscCommandsTests.swift`,
`agtermTests/ControlServerLinkOverlayTests.swift` (`testALinkOpensAtTheSizeItsModeNames`),
`GhosttySurfaceViewInputTests.swift` (the six ⌘⇧ cases), `SettingsUITests.swift`. Contract: `commands.html` (`browser
links`, tree `linkOpenMode`), `control-api.md`.

**Rust side:** `crates/ngterm-core/src/persist/settings.rs` (LinkOpenMode gains overlay95/75/50, `overlay_size_percent()`),
`link_policy.rs` (`Route::Overlay { size_percent }`, `wants_browser`), `commands/overlay_page.rs` (`open_link_overlay`
with a size), `commands/tests/{config_browser,overlay_page}.rs`; `crates/ngterm-ctl/src/browser.rs`, `tests/cli.rs`;
`crates/ngterm-gpui/src/settings/general.rs`, `state.rs` (`follow_link`), `terminal.rs` (`OpenUrl` carries the
override), `terminal/{mouse,links,input_tests}.rs`, `pane.rs` (modify).

**Platforms:** ngterm's link click is Ctrl or Super alone on every OS; upstream's ⌘⇧ override is exactly Ctrl+Shift
or Super+Shift (macOS ⌘+Shift). Opening in the browser goes through GPUI `cx.open_url`: Linux the `open` crate
(xdg-open, gio, …) with the portal's OpenURI fallback, Windows `ShellExecuteW`, macOS NSWorkspace with the GPUI macOS
form. The policy lives in `ngterm-core`; the GPUI mouse part is "not built on macOS/Windows by CI until
`ngterm-gpui` joins `cross-platform.yml`".

**Conformance:** existing `test_config_browser.py::test_links_sets_and_reads_the_link_open_mode`,
`::test_links_refuses_an_unknown_mode`, `::test_browser_commands_take_no_target_or_window`,
`test_cli_output.py::test_browser_and_config` (listed). New: `test_config_browser.py::test_links_takes_the_sized_overlay_modes`
(`@since("0.39.1")`), "overlay60" refused under `since`, the CLI sibling for `browser links overlay75`.

- [x] `browser.links` accepts overlay95/overlay75/overlay50 (stored raw as `linkOpenMode`; a stored `overlay` stays
      full size); refusals unchanged ("browser.links takes no target or --window", "invalid link mode: <raw>")
- [x] any non-browser mode routes a showable web link from a pane or scratch to an overlay at the mode's size, read
      back as `overlaySizePercent`
- [x] Ctrl/Super+Shift-click (exactly) in an overlay mode with no mouse capture opens the browser; Shift alone extends
      the selection; under capture the chord follows the setting; in browser mode it is not a link click; the link
      lights on hover and on a modifier change under a still pointer
- [x] CLI help and "mode must be browser, overlay, overlay95, overlay75 or overlay50"; Settings ▸ General ▸ Open
      links in gains "Session overlay at 95%", "… 75%", "… 50%"
- [x] write tests: the Swift cases above (core policy, commands, ctl, gpui mouse)
- [x] write tests: the conformance tests above
- [x] checks sized to the change pass; newly green ids added to the ratchet (none: the three new conformance
      tests are `since(0.39.1)`; green at a temporary 0.39.1 level, their ids go in at Task 25)

### Task 10: A pane overlay draws above the session overlay it opened over (#831); a pane ask over a cover is unshaded (401ff29)

**Upstream:** `agtermCore/Sources/agtermCore/PaneOverlay.swift`, `Session+OverlayVisibility.swift`, `AppStore+Panes.swift`,
`AppStore+HtmlOverlay.swift`, `Session+HtmlOverlay.swift`, `Session.swift`, `Session+Ask.swift`,
`agterm/AppActions+Overlay.swift`, `Views/WindowContentView+Detail.swift`, `Views/AskDialogView.swift`. Tests:
`AppStoreOverlayVisibilityTests.swift` (the 13 new cases and the reordered `openAll`), `AppStoreReplicaPageTests.swift`
(`aLocalPaneOpenerOverTheReplicaLeavesItsReportedStateAlone`, `anOriginHideOutlivesALocalPaneOpener`),
`SessionTests.swift` (`paneAskLeavesAShownSessionCoverUnshaded`), `agtermUITests/ControlOverlayVisibilityUITests.swift`
(`testAPanePageOverAProgramHidesItAndClosingThePageBringsItBack`). Contract: `commands.html` (`session overlay open
--pane`, hide/show), `control-api.md`.

**Rust side:** `crates/ngterm-core/src/model/{pane_overlay,html_overlay,overlay_visibility,overlay,ask}.rs`,
`projection.rs`, `app_actions.rs` (the editor-open bell), `model/tests/overlay_visibility.rs`,
`model/tests/ask_draft.rs` (or a new `ask.rs`), `commands/tests/replica_page.rs`; `crates/ngterm-gpui/src/split.rs`
(`HiddenOverlays::of`), `ask_dialog.rs`, `ask_dialog/tests.rs`, `deck.rs` (`sync_ask`) (modify).

**Conformance:** `test_overlay.py::test_hide_show_and_toggle_keep_the_overlays_running` (listed) is reordered to open
the pane overlay first so it stays green. New (`@since("0.39.1")`):
`test_overlay.py::test_a_pane_overlay_opened_over_a_session_overlay_draws_above_it` (cover and opener over
program/page), `::test_a_session_page_under_a_pane_opener_reads_back_hidden`,
`::test_a_cover_opened_after_the_pane_overlay_stays_on_top`, `::test_a_hand_hidden_cover_stays_hidden_after_the_pane_closes`,
`::test_closing_the_holding_split_brings_the_cover_back`.

- [x] a pane program or page opened while a session-wide program or page is up holds the cover; held means a
      shown pane overlay with the hold on a laid-out pane; `session_slot_shown() = !hidden && !held`; `overlay.hidden`
      stays the user's bit
- [x] the visibility predicates, `hiddenOverlays` ("session" while held) and the session page's `htmlOverlays[].hidden`
      follow it; holds drop when the cover closes or its slot is torn down, so a replacement cover is not held by an
      older opener; a hide/show/toggle round trip keeps the opener on top; a replica's reported `hidden` stays the raw
      bit
- [x] the GPUI deck keeps a held cover mounted hidden and draws the pane overlay on top
- [x] a pane-anchored ask over a shown session program or page draws no dim wash (its pane-sized click layer
      stays); a session ask, a hidden or held cover and a HUD keep the wash
- [x] scan the other ngterm-core and ngterm-gpui tests that open a cover before a pane overlay and fix their order
- [x] write tests: the Swift cases above (13 visibility cases, 2 replica cases, 6 ask shade cases plus a held-cover
      one, the gpui deck and ask layer)
- [x] write tests: the conformance tests above
- [x] checks sized to the change pass; newly green ids added to the ratchet

### Task 11: The page strip's Hide only with the Hide overlays button on (c43bf01); the undo chord on a focused page (00fc83d)

**Upstream:** `agterm/Views/HtmlOverlayView.swift`, `Views/WindowContentView+Detail.swift`, `Views/UndoCloseShortcut.swift`.
Tests: `agtermUITests/ControlOverlayVisibilityUITests.swift` (`testThePageStripOffersHideOnlyWhileTheOverlaysButtonIsOn`),
`agtermTests/UndoCloseShortcutTests.swift` (both). Contract: `commands.html` (`session.overlay.open` HTML page),
`site/docs.html`.

**Rust side:** `crates/ngterm-gpui/src/html_overlay/policy.rs` (`strip_items` takes the flag), `html_overlay/panel.rs`,
the WPE and WebView2 strip callers, `html_overlay/view/tests.rs`; `crates/ngterm-gpui/src/keymap_dispatch.rs`,
`html_overlay/registry.rs` (a `page_holds_keyboard` query), the html-off stub in `html_overlay.rs`,
`html_overlay/webview2/registry.rs` if needed (modify). The main checkout holds uncommitted HTML-overlay work in
these directories (Context): keep the change small and local.

**Platforms:** the undo_close chord is Ctrl+Shift+Z on Linux and Windows (the shipped table), ⌘Z upstream on macOS.
Linux: a focused WPE page gets the chord when `keymap_dispatch::intercept` yields before the router; Windows: a
focused WebView2 page keeps it when `keymap_dispatch::takes` returns false for it; macOS: no ngterm page host yet
(open work, upstream's NSResponder check). GPUI code: "not built on macOS/Windows by CI until `ngterm-gpui` joins
`cross-platform.yml`".

**Conformance:** `tests/ui/test_html_overlay.py::test_page_strip_hide_hides_its_own_pane` (not listed) launches with
`settings={"shownInterfaceElements": ["overlayToggle"]}`; new `tests/ui/test_html_overlay.py::test_the_page_strip_offers_hide_only_while_the_overlays_button_is_on`
(`@since("0.39.1")`, ui layer).

- [x] the strip's Hide button shows only while Settings ▸ Interface ▸ Hide overlays is on, for session and pane
      pages, live on an open page; the View menu, palette, toggle_overlay chord and `session overlay show` still
      restore a hidden page
- [x] while an HTML page holds the window's keyboard, the undo_close chord goes to the page; with focus on the
      terminal, sidebar or chrome it still undoes a pending close, even when a page covers the session
- [x] write tests: `strip_items` with the flag off and on for file, URL, navigation and replica pages; an open page's
      strip follows the setting; keymap_dispatch with a focused page (nothing fires, the page gets the key) and with
      the terminal focused (UndoClose fires); a `cfg(windows)` unit test of `takes()`
- [x] write tests: the ui tests above
- [x] checks sized to the change pass; newly green ids added to the ratchet (none: the ui layer runs only on the
      reference, and the new test is `@since("0.39.1")`)

### Task 12: Live `--command` sessions close when their command exits, `--wait` holds (#840)

**Upstream:** `agtermCore/Sources/agtermCore/ZmxSupport.swift`, `ZmxReplayScript.swift`, `agterm/Ghostty/ZmxLaunch.swift`,
`LaunchSeed.swift`. Tests: `ZmxSupportTests.swift` (`paneCommandEndsThePaneWhileCreationCommandReturnsToTheShell`,
`capturedReplayWinsOverThePaneCommand`, `fishPaneCommandEndsThePane`), `ZmxReplayScriptTests.swift`
(`endingCommandRunsItsLineAndExitsWithItsStatus`, `endingCommandStartsNoShellAfterIt`,
`fishEndingCommandRunsItsLineAndStartsNoShell`), `agtermTests/LaunchSeedTests.swift` (both), `ZmxLaunchTests.swift`,
`agtermUITests/ZmxLiveUITests.swift` (`testACommandSessionClosesWhenItsCommandExitsAndHoldsWithWait`),
`ControlServerRestartTests.swift` (`testARestartCommandStillReturnsToTheLoginShell`). Contract: CHANGELOG v0.39.0's
"Behavior change" bullet, `commands.html` (`session new --command [--wait]`).

**Rust side:** `crates/ngterm-core/src/live/support.rs` (a creation that ends the pane vs one that returns to the
shell), `live/replay.rs` (`render_ending`, `render_fish_ending`, a PowerShell ending form), `live/shell_integration.rs`,
`restore/spawn_plan.rs` (`resolve_live_seed`: the pane's command as an ending creation, `wait_after_command` from the
launch), `commands/restart_live.rs` (the returning form, explicitly), `commands/tests/live.rs` (the `:447`
expectation); `conformance/tests/protocol/test_zmx.py`, `test_live_any_shell.py`, `conformance/rust-passing.txt`,
`conformance/rust-passing-windows.txt` (modify).

**Platforms:** one ending script per session-server shell: zsh and bash `<shell> -lic "'builtin' 'eval' --
'<line>'"` (exits with the command's status, execs no shell); fish `builtin status job-control full; builtin eval
'<line>'`; PowerShell (Windows, and macOS/Linux as a login shell) `-Command` without `-NoExit`, then `exit
$LASTEXITCODE` (an ngterm adaptation: upstream has no PowerShell). The hold uses the existing `wait_after_command`
path on every OS. `ngterm-core` is `any`.

**Conformance:** the two listed ids that assert the old behavior (Context) are rewritten and their ids swapped by
hand in both ratchet lists, the commit message saying why. New: `test_zmx.py::test_a_live_command_session_closes_when_its_command_exits_and_holds_with_wait`
(LIVE), `test_live_any_shell.py::test_an_ended_live_command_closes_its_pane` and
`::test_an_interrupted_live_restart_command_leaves_the_login_shell_prompt` (LIVE_ANY_SHELL, including Windows).
`test_zmx.py::test_restart_command_interrupted_by_ctrl_c_leaves_a_prompt` (listed) stays green.

- [x] a pane's own launch command (`session new --command`, `split --command`, later Duplicate as Companion's copied
      programs) runs as an ending creation: no login shell after it, the pane closes when it exits, as outside Live
- [x] a captured argv replay still wins over the pane command and returns to the shell; a suppressed daemon attaches
      with neither; `session restart --command` and restored commands keep returning to the login shell
- [x] `--wait` keeps the final output after the command exits (left, right per its own launch; scratch never)
- [x] module docs in `live/replay.rs`, `live/support.rs`, `restore/spawn_plan.rs` describe the two forms
- [x] write tests: the Swift cases above (real zsh and bash where present, exit 7; real fish when present; a pwsh
      form test), the spawn-plan wait cases, the restart's returning form
- [x] write tests: the conformance rewrites and new tests above
- [x] checks sized to the change pass; newly green ids added to the ratchet

### Task 13: Companion sessions, part 1: the model, persistence, create, fold and move (#840)

**Upstream:** `agtermCore/Sources/agtermCore/AppStore+Companions.swift`, `AppStore+SessionMoves.swift` (replaces the old
moves in `AppStore.swift`), `AppStore.swift`, `AppStore+Snapshot.swift`, `Snapshot.swift`, `Session.swift`,
`AppStore+Events.swift`, `ControlDispatcher+Companions.swift`, `ControlDispatcher.swift`, `ControlProtocol.swift`,
`ControlProjection.swift`, `ControlEvents.swift`, `ControlResponse.swift`, `agterm/Control/ControlServer+Companions.swift`
(createCompanion, createLocalCompanion, the move refusals, setSessionExpansion), `ControlServer+SessionActions.swift`,
`agtermctlKit/Session+Companions.swift`, `SessionCommands.swift`, `EventCommands.swift`, `SocketClient.swift`. Tests:
`AppStoreCompanionsTests.swift` (the placement, fold, move and attention cases: `aBackgroundCompanionKeepsTheSelection`
… `blockedCompletedAndUnreadCompanionsNeedAttention`), `AppStoreOrganizationTests.swift` (all 11),
`AppStoreTreeProjectionTests.swift` (the parent/collapsed cases, the terminal half of the content case),
`SnapshotRoundTripTests.swift` (the three companion cases), `AppStoreEventTests.swift`
(`aCompanionsCreatedEventNamesItsParent`), `ControlDispatcherCompanionsTests.swift` (the new/collapse/expand cases),
`ControlResponseTests.swift`, `agtermTests/ControlServerCompanionsTests.swift` (create, move refusal, fold),
`agtermctlKitTests/SessionCompanionsCommandsTests.swift` (new, collapse/expand), `SocketClientTests.swift` (the two
too-old cases), `EventCommandsTests.swift`, `agtermUITests/ControlCompanionsUITests.swift`. Contract: `commands.html`
(`session new --parent`, `session collapse|expand`, tree `parent`/`collapsed`/`content`, `events`, `session move`),
`control-api.md` > Companions, `sidebar.md` > Companions.

**Rust side:** `crates/ngterm-core/src/model/companions.rs`, `model/session_moves.rs`, `commands/companions.rs`,
`model/tests/companions.rs`, `commands/tests/companions.rs` (create); `model/{session,store,duplicate,snapshot}.rs`,
`persist/snapshot.rs` (`parentID`, `collapsed` only when true), `projection.rs`, `commands/{mod,session}.rs`,
`model/tests/{organization,snapshot}.rs` (modify); `crates/ngterm-ctl/src/session.rs`, `events.rs`, a too-old mapping
in `crates/ngterm-ctl/src/companions.rs` (create), `tests/cli.rs` (modify).

**Conformance:** existing `test_session.py` move ids (`test_move_*`, 10 ids), `test_tree.py::test_fresh_session_node`,
`test_events.py::test_session_lifecycle_events`, `test_cli_output.py::test_session_move_and_go` (listed). New
`tests/protocol/test_companions.py` (create): `test_a_companion_lands_under_its_parent_and_reads_back_its_parent`,
`test_companion_new_refusals`, `test_collapse_and_expand_fold_a_parent`, `test_a_companion_moves_only_among_its_siblings`,
`test_a_companion_created_event_names_its_parent`; `test_cli_output.py::test_session_new_parent_collapse_and_expand`.

- [x] Session `parent_id` (fixed at creation), `is_expanded` (default true), `content` (terminal | page, only Task 17
      makes pages), `is_persistable = local && terminal`
- [x] `add_companion`: a companion target means its parent; lands at the parent's group end; cwd defaults to the
      parent's focused directory; `noSelect` keeps the selection; the creation tint (Task 3); a remote parent answers
      upstream's local-path refusal "companions of a remote session are not supported yet" until Task 19
- [x] insertion invariants: a top-level insertion inside a group moves past it; plain duplicate of a parent lands
      after its group; an anchored `session.new` lands outside a group
- [x] moves (port `AppStore+SessionMoves.swift`): a companion moves only among its siblings, a parent carries its
      block, batches carry companions once; `can_move_sessions`
- [x] `session.collapse`/`session.expand`: "session has no companions" for a lone session; unchanged value answers
      ok without saving
- [x] snapshot `parentID`, `collapsed` (only true), lossy; restore regroups and drops a link to no top-level session
      of that workspace; an old `windows.json` decodes top-level and expanded
- [x] tree `parent`, `collapsed` (only for a folded parent that still has companions), `content` ("page" only);
      `session.created` payload `parent`
- [x] `session.companion.new` refusal order: "a companion takes no workspace or placement: it lands under its
      parent", "--wait requires --command", the creation tint, the target; "could not create companion"; result
      `{id, parent}`
- [x] `session.move` refusal "a companion moves only among its siblings" (single, batch, anchor forms); an ordinary
      no-op stays ok
- [x] CLI `session new --parent SID` ("--parent takes no --workspace, --workspace-name, --create-workspace, --after or
      --before"), `session collapse`, `session expand`, the events line `... session.created NAME parent=ID`, and the
      too-old mapping (an `invalid request:` reply naming a companion wire command prints "error: the running app is
      too old for companions", exit 1; the raw reply under `--json`), shared with Tasks 17-18
- [x] write tests: every Swift case listed above on the fakes; ctl parse and the too-old mapping
- [x] write tests: the conformance tests above
- [x] checks sized to the change pass; newly green ids added to the ratchet (none: the six new ids are since(0.39.1)
      and skip at 0.38.1; green at a temporary 0.39.1 level, they go in at Task 25)

### Task 14: Companion sessions, part 2: group close, close undo and Recent Closed (#840)

**Upstream:** `agtermCore/Sources/agtermCore/AppStore.swift` (closeSession, removeWorkspace), `AppStore+PendingClose.swift`,
`AppStore+RecentClosed.swift`, `RecentClosed.swift`, `WindowLibrary+RecentClosed.swift`, `ControlRecent.swift`,
`agterm/Control/ControlServer+SessionClose.swift`, `ControlServer+Recent.swift`, `agtermctlKit/SocketClient.swift`. Tests:
`AppStoreCompanionsTests.swift` (the eight close/undo cases), `AppStoreEventTests.swift`
(`closingAParentEmitsOneClosedEdgePerGroupMember`, `anUndoneCompanionCloseReportsTheParentsCurrentWorkspace`),
`WindowLibraryRecentClosedTests.swift` (all 6), `ControlRecentTests.swift`, `agtermTests/ControlServerCompanionsTests.swift`
(the five close/receipt/reopen cases), `SocketClientTests.swift` (`formatRecentCountsAParentEntrysCompanions`). Contract:
`commands.html` (`session close`, `recent list`, `recent open`), `control-api.md` > Companions.

**Rust side:** `crates/ngterm-core/src/model/{store,pending_close,recent_closed,window_library}.rs`,
`persist/recent_closed.rs` (`companions`, optional), `commands/{session,recent}.rs`, their tests;
`crates/ngterm-ctl/src/recent.rs` (modify).

**Conformance:** existing close and recent ids in `test_session.py`, `test_recent.py`, `test_cli_output.py` (listed;
they assert subsets, so an added `closed` keeps them green). New: `test_companions.py::test_closing_a_parent_closes_its_group`,
`test_recent.py::test_a_parent_entry_reopens_its_companions`, `::test_a_soft_closed_group_is_one_undo`, a behaviors
test seeding `recent-closed.json` for "parent session is not open or in Recent Closed",
`test_cli_output.py::test_recent_list_counts_companions_and_close_prints_the_group`.

- [x] a parent's hard close closes its companions (reverse order) then itself, one Recent Closed entry holding the
      persistable companions; closing a companion closes only it; each member emits its own `session.closed`
- [x] a parent's soft close is one undo for the group; `soft_close_sessions` widens to companions; one entry per
      group at finalize; undo follows a parent that moved and regroups each destination; a top-level undo lands
      outside a group that grew into its slot; a parent's hard close, its finalize, or a hard workspace delete ends
      the undo of its parked companions
- [x] `session.close` result `{id, closed, affected (group > 1)}`; batches drop a companion named beside its parent;
      merge with Task 4's `undoable` in field order `undoable`, `parent`, `closed`
- [x] recent: an entry's `sessionIDs` lists its companions; a lone companion reopens under its live parent, else
      the entry holding the parent first, else "parent session is not open or in Recent Closed"; an older
      `recent-closed.json` still decodes
- [x] CLI: `recent list` row " +N companion(s)"; a parent's close prints "N sessions"
- [x] write tests: every Swift case listed above
- [x] write tests: the conformance tests above
- [x] checks sized to the change pass; newly green ids added to the ratchet (none: the new ids are since(0.39.1);
      green at a temporary 0.39.1 level, they go in at Task 25)

### Task 15: Companion sessions, part 3: a parent's exiting shell restarts while it has companions (#840)

**Upstream:** `agtermCore/Sources/agtermCore/AppStore+Companions.swift` (respawnMinimumUptime, respawnsOnLastPaneExit,
respawnPrimaryPane), `AppStore+Panes.swift` (closePrimaryPane → PrimaryPaneExit, closeSplitPane), `Session.swift`,
`agterm/Views/WindowContentView+Detail.swift`. Tests: `AppStoreCompanionsTests.swift` (the six respawn cases),
`agtermUITests/ControlCompanionsUITests.swift` (`testAParentWhoseShellExitsRespawnsAndTakesInput`,
`testACommandParentRespawnsAsAPlainShellInTheDirectoryItMovedTo`), `ZmxLiveUITests.swift`
(`testALiveParentWhoseShellExitsRespawnsAndTakesInput`). Contract: `commands.html` (`session new --parent`, its last
paragraph).

**Rust side:** `crates/ngterm-core/src/model/{companions,panes,session}.rs`, `commands/panes.rs` (`pane_exited` takes
a clock), `model/tests/panes.rs`; `crates/ngterm-gpui/src/deck.rs` (mount a fresh primary when `primary_respawns`
moves: the mechanism needs designing, today's `sync_page` mounts only the split and scratch) (modify).

**Platforms:** the fresh shell goes through the existing pane spawn (openpty with the login shell on Linux/macOS,
ConPTY with the configured shell on Windows, the session server in Live mode); "last cwd" is the pane's reported
cwd. No new OS-specific code.

**Conformance:** `test_split.py`, `test_restore.py` pane-exit ids (listed) stay green. New:
`test_companions.py::test_a_parent_whose_shell_exits_restarts_it`, under plain PTY and Live.

- [x] a live split survivor promotes first; with none, a local session with companions whose last restart is none or
      ≥ 2 s ago restarts in its current cwd as a plain login shell (creation command, wait, foreground, title and the
      left pane overlay cleared; HUD/ask on that pane ended; the pane identity kept); otherwise the group closes
      (Task 14)
- [x] the same rule through the split slot; a remote row never restarts here (Task 19)
- [x] the GPUI deck remounts the primary on a respawn
- [x] write tests: the six Swift cases with an injected clock
- [x] write tests: the conformance test above
- [x] checks sized to the change pass; newly green ids added to the ratchet (none: the new ids are since(0.39.1);
      green at a temporary 0.39.1 level, they go in at Task 25)

### Task 16: Companion sessions, part 4: sidebar nesting and folding, New Companion, `parent › child` in flat lists (#840, #841)

**Upstream:** `agterm/Views/WorkspaceSidebar+Companions.swift`, `WorkspaceSidebar.swift` (8374938's status glyph fix),
`WorkspaceSidebar+RowRendering.swift`, `+DragDrop.swift`, `+ContextMenu.swift`, `SidebarRowViews.swift`,
`agtermCore/Sources/agtermCore/SidebarDrop.swift`, `AppStore+Naming.swift`, `WindowLibrary+Attention.swift`,
`PaletteCatalog.swift`, `BuiltinAction.swift`, `agterm/AppActions+Companions.swift`, `AppActions+Palette.swift`,
`AppActions.swift`, `agtermApp+Menus.swift`, `Views/Palette.swift`, `SessionSwitcher.swift`,
`WindowContentView+RecentSessions.swift`, `DashboardView.swift`. Tests: `agtermTests/WorkspaceSidebarCompanionsTests.swift`
(all), `SidebarRowIndentTests.swift` (both), `SidebarDropTests.swift` (the two companion gaps),
`AppStoreNameSanitizeTests.swift` (all 4), `WindowLibraryAttentionTests.swift` (`anAttentionCompanionIsLabelledAfterItsParent`),
`AppActionsPaletteTests.swift` (the two companion rows), `PaletteCatalogTests.swift` (`newCompanionNeedsAnActiveSession`),
`BuiltinActionTests.swift`, `DashboardViewTests.swift`, `SidebarFlaggedLayoutTests.swift`
(`testEveryFlaggedLayoutNamesACompanionAfterItsParentAndTheTreeDoesNot`), `AppStoreCompanionsTests.swift`
(`aFlaggedCompanionOfAnUnflaggedParentCannotLeaveItsGroupInTheFlaggedTree`), `agtermUITests/SidebarUITests.swift`,
`RecentSessionsButtonUITests.swift`. Contract: `sidebar.md` > Companions, `menu-actions.md`, `keymap.md`.

**Rust side:** `crates/ngterm-core/src/model/naming.rs` (`list_label`, `list_ordered`), `palette/feeds.rs`,
`titlebar/mod.rs`, `palette_catalog.rs` (NewCompanion), `keymap/builtin.rs` (`new_companion`), `app_actions.rs`,
`sidebar_actions.rs`, `sidebar_drop.rs` (`companion_child_index`); `crates/ngterm-gpui/src/session_switcher.rs`,
`dashboard.rs`, `sidebar/{rows,view,look,drag,menu,tests}.rs`, `app_menu.rs`, `popovers/tests.rs` (modify).

**Platforms:** no OS-specific code. Upstream's Dock menu label has no ngterm counterpart yet (no Dock or jump-list
session menu is ported); that menu would use `list_label`.

**Conformance:** new `tests/ui/test_sidebar_rows.py::test_companion_rows_nest_fold_and_mark_attention` (ui layer,
`ui:companion-marker`).

- [x] `list_label`: "PARENT › CHILD" (U+203A with spaces) for a companion; `list_ordered`: A-Z by label, each parent
      followed by its companions; ⌃P's empty query uses it; Ctrl-Tab, the attention palette, both titlebar
      popovers, dashboard captions and flagged layouts label `parent › child`; the tree, rename and Copy Name keep
      the bare name
- [x] built-in `new_companion` (keyless), palette "New Companion" (needs an active session), the row menu item beside
      Duplicate Session and the Session menu item; New Companion selects and focuses the new row
- [x] GPUI sidebar: companion rows indent under their parent in the ordinary tree (unindented in flagged layouts); a
      disclosure triangle folds, persisting only on a user toggle; a selection reveal opens without persisting; a
      folded parent shows a companion marker for blocked, completed or unseen companions; no row's status glyph or
      badge leaves the sidebar's bounds (#841); a drop moves a companion only among its siblings; a parent dragged
      with its companion moves the group; Move to only for a parent
- [x] write tests: every Swift case listed above (core naming, catalog, builtin count, sidebar_drop; gpui sidebar)
- [x] write tests: the ui test above
- [x] checks sized to the change pass; newly green ids added to the ratchet (none: the ui test runs on the macOS
      reference only, and the new `test_keymap_hooks.py::test_new_companion_is_a_keyless_builtin` is since(0.39.1),
      green at a temporary 0.39.1 level; it goes in at Task 25)

### Task 17: Companion sessions, part 5: page companions (#840)

**Upstream:** `agterm/Control/ControlServer+CompanionPage.swift`, `ControlServer+Companions.swift` (createPageCompanion,
pageCompanionRefusal, pageRefusedCommands), `ControlDispatcher+Companions.swift` (dispatchCompanionPage),
`AppStore+Companions.swift` (addPageCompanion), `AppStore+ReplicaPage.swift`, `HtmlPageOutcomes.swift`,
`agtermctlKit/SessionCommands.swift` (Overlay.Open `--companion`), `OverlayPageAttach.swift`, `Session+Companions.swift`,
`ControlServer+PageServe.swift`, `ControlServer+RemotePage.swift`. Tests: `AppStoreCompanionsTests.swift` (the six
page cases), `ControlDispatcherCompanionsTests.swift` (the four page cases),
`agtermctlKitTests/SessionCompanionsCommandsTests.swift` (the five overlay cases),
`agtermTests/ControlServerCompanionPageTests.swift` (all 6), `ControlServerCompanionsTests.swift` (the three page
cases), `AppStoreReplicaPageTests.swift` (`aReplicaPageCompanionClosesOnceWithItsPage`), `HtmlPageOutcomesTests.swift`,
`PaletteCatalogTests.swift` (`aPageCompanionDisablesTerminalOnlyCommands`), `WorkspaceSidebarCompanionsTests.swift`
(`testAPageCompanionRowShowsThePageIconFilledWhenFlagged`), `agtermUITests/ControlCompanionsUITests.swift`
(`testAPageCompanionShowsItsPageWithNoTerminalAndClosesWithIt`). Contract: `commands.html` (`session overlay open
--companion`, `session overlay page-attach`), `control-api.md` > Companions.

**Rust side:** `crates/ngterm-core/src/commands/companion_page.rs` (create), `dispatch.rs` (the page-companion guard
ahead of every command), `commands/overlay.rs`, `model/{overlay,overlay_visibility,pane_overlay}.rs`,
`persist/snapshot.rs`, `model/recent_closed.rs`, `remote/{page,replica_page}.rs`, `html/outcomes.rs`, the dashboard
cells, `palette_catalog.rs` (terminal-only gating); `crates/ngterm-ctl/src/session_overlay.rs`;
`crates/ngterm-gpui/src/{deck,sidebar,html_overlay}.rs` (a terminal-less page row) (modify).

**Platforms:** no OS-specific process code; the page host is WPE WebKit on Linux, WebView2 through wry on Windows, the
macOS GPUI page host when it lands. Loopback detection reuses the existing check on every OS.

**Conformance:** existing `test_overlay.py::test_open_refusals` (32 ids), `::test_html_page_occupies_the_slot`,
`::test_page_flags_read_back`, `test_zmx.py::test_page_serve_serves_a_page_a_presenter_shows` (listed). New in
`test_companions.py`: `test_page_companion_opens_a_page_row_with_no_shell_under_its_parent`,
`test_page_companion_refusals`, `test_a_page_companion_refuses_terminal_commands`, `test_closing_the_page_closes_its_row`,
`test_page_attach_hands_out_a_claim_only_for_a_page_companion`; `test_relaunch.py::test_a_page_companion_is_not_restored_after_relaunch`;
`tests/cli/test_cli_only.py::test_overlay_open_companion_validation`.

- [x] `session.companion.page`: "a page companion needs --html or --url", "a page companion is full size: it takes no
      --pane, --size-percent, --width or --height", "invalid color: <c> (#rrggbb)", "session.companion.page:
      <failure>"; the row lands after the parent's group, named for its page, reporting its parent's cwd; the page
      opens full size in the row's own slot; "could not create page companion"; reply `{id, pageID, parent}`
- [x] never persisted, never archived, not restored after relaunch; no dashboard cell; closing the page closes the
      row (the parent stays)
- [x] one guard ahead of every command: "<cmd>: not available on a page companion" for the terminal commands upstream
      lists (type, text, copy, paste, selectall, search, background, lead, swap, focus, resize, split, split.close,
      scratch, restart, restore, overlay.open, hud.open, overlay.resize without full, ask.open unless gui,
      surface.cursor/zoom with an explicit target, `surface:<sid>:<slot>` read as its session); rename stays ok
- [x] `session.companion.page.attach`: "not a page companion", "a page on this Mac's loopback cannot be shown on
      another Mac"; a single-use token per viewer, each claim its own stream, the page's final release ends every
      token; a soft-closed page keeps serving through undo
- [x] `HtmlPageOutcomes.settle`; ctl `session overlay open --companion` ("--companion requires --html or --url",
      "--companion is full size: it takes no --pane, --size-percent, --width or --height"), `session overlay
      page-attach`, the too-old mapping (Task 13)
- [x] GPUI: a page row nests with a page glyph and no status glyph; the session view shows the page full size with no
      terminal
- [x] write tests: every Swift case listed above
      (`aReplicaPageCompanionClosesOnceWithItsPage` and `addReplicaPageCompanion` go with Task 19's remote page
      companions, which create the replica rows)
- [x] write tests: the conformance tests above
- [x] checks sized to the change pass; newly green ids added to the ratchet (none: the new ids are since(0.39.1);
      green at a temporary 0.39.1 level, they go in at Task 25)

### Task 18: Companion sessions, part 6: Duplicate as Companion (#840, #841, #842)

**Upstream:** `agtermCore/Sources/agtermCore/DuplicateReplay.swift`, `RestartReplay.swift`, `CommandRestore.swift`
(parseProcEnvironment), `agterm/Ghostty/ForegroundProcess.swift`, `ZmxForegroundResolver.swift`,
`agterm/Control/ControlServer+Companions.swift` (duplicateAsCompanion), `AppStore+Companions.swift`
(duplicateAsCompanion; 746631e's left focus), `agterm/AppActions+Companions.swift` (8374938's refusal HUD),
`AppActions+RemoteAttach.swift`, `ControlDispatcher+Companions.swift`, `agtermctlKit/SessionCommands.swift`,
`PaletteCatalog.swift`, `BuiltinAction.swift`, `agtermApp+Menus.swift`, `Views/WorkspaceSidebar+ContextMenu.swift`.
Tests: `DuplicateReplayTests.swift` (all 16), `AppStoreCompanionsTests.swift` (the three duplicate cases),
`CommandRestoreTests.swift` (`parseProcEnvironmentReadsTheStringsAfterArgvUpToTheFirstEmptyOne`),
`ControlDispatcherCompanionsTests.swift` (`companionDuplicateRoutesTargetAndWindow`), `ZmxReplayScriptTests.swift` (the
two duplicated-argv cases), `agtermTests/ControlServerCompanionsTests.swift` (the two duplicate refusals),
`AppActionsCompanionsTests.swift` (`testARefusedDuplicateShowsTheReasonInATimedHudOverTheSession`),
`PaletteCatalogTests.swift` (`duplicateAsCompanionNeedsATerminal`), `SessionCompanionsCommandsTests.swift`,
`agtermUITests/ControlCompanionsUITests.swift` (`testDuplicateAsCompanionCopiesTheSplitAndStartsEachPanesProgramAgain`).
Contract: `commands.html` (`session duplicate --companion [--no-select]`), `control-api.md` > Companions,
`menu-actions.md`, `keymap.md`.

**Rust side:** `crates/ngterm-core/src/duplicate_replay.rs`, `commands/companion_duplicate.rs` (create);
`restart_replay.rs` (Observation gains the image path and PATH; `Refusal.reason` split from `message`),
`restore/capture.rs`, `model/duplicate.rs`, `keymap/builtin.rs`, `palette_catalog.rs`, `sidebar_actions.rs`,
`app_actions.rs`, `remote/attach.rs` (the HUD-over-session failure, reusable with an action name);
`crates/ngterm-host/src/lib.rs`, `src/fakes.rs` (`ProcessTable::executable_path`, `search_path`);
`crates/ngterm-session/src/process_table/{mod,linux,macos,windows}.rs`, `procfs.rs`;
`crates/ngterm-proto/src/command.rs`; `crates/ngterm-ctl/src/session.rs`; `crates/ngterm-gpui/src/sidebar/menu.rs`,
`app_menu.rs` (modify).

**Platforms:** image path: macOS `proc_pidpath`; Linux `readlink /proc/<pid>/exe` (a " (deleted)" suffix means the
image is gone); Windows `QueryFullProcessImageNameW`. PATH: macOS from the environment strings after argv in the same
`KERN_PROCARGS2` blob (first key wins; a platform binary's environment is withheld, hence the system-directory
fallback); Linux `/proc/<pid>/environ`; Windows the PEB `ProcessParameters->Environment` block (UTF-16, `;`,
case-insensitive `Path`) read with the same `ReadProcessMemory` that gives the cwd. Lookup: POSIX `dir/name` over
absolute `:` entries with the executable bit; Windows `;` entries, drive or UNC roots, each `PATHEXT`; system dirs
`/usr/bin:/bin:/usr/sbin:/sbin` or `%SystemRoot%\System32;%SystemRoot%`. The copied command line is POSIX-quoted on
macOS/Linux and in the pane shell's dialect on Windows. The backslash refusal applies to POSIX and fish lines only
(a Windows path separator is a backslash): an ngterm adaptation to record in the module docs. `ngterm-session`,
`ngterm-host` and `ngterm-core` are `any`.

**Conformance:** existing `test_session.py::test_duplicate_inserts_a_plain_copy_after_the_source`,
`test_zmx.py::test_restart_without_a_command_replays_the_foreground_program`,
`::test_restart_replay_refuses_a_shell_at_its_prompt` (listed). New in `test_companions.py`:
`test_duplicate_as_companion_copies_the_split_and_runs_each_program`, `test_duplicate_as_companion_with_no_select_keeps_the_selection`,
`test_duplicate_as_companion_refusals`; `tests/cli/test_cli_only.py::test_duplicate_no_select_needs_companion`; under
Live, `test_a_duplicated_program_pane_closes_when_the_program_exits`.

- [x] `session.companion.duplicate` (target, window, noSelect), reply `{id, parent}`; a remote source goes to Task 19
- [x] source checks: "no such session", "a page companion has no terminal to duplicate", "a program overlay covers
      the session; nothing was created"; per pane, left then right: "<pane> pane is covered by a program overlay;
      nothing was created", "<pane> pane is not realized; nothing was created", then the replay plan from one fresh
      leader listing
- [x] `DuplicateReplay.resolve` per upstream (plain shell for a foreground shell; a bare name by the first PATH entry
      resolving to the image, else the first executable entry; a relative name under its cwd; the system dirs
      without a PATH; an image gone after an upgrade starts the first PATH entry that resolves (#841)); every
      RestartReplay refusal on both the started path and the image; refusals "<pane> pane: <reason>; nothing was
      created", with the reasons "the foreground program's arguments contain a backslash, which cannot be copied",
      "the foreground program's executable path cannot be read", "the path the foreground program was started from
      cannot be found"
- [x] the copy: a companion after the parent's group, each program as its pane's ending creation command (Task 12),
      the split's axis and ratio, focus on the left pane (#842)
- [x] the GUI refusal in a timed HUD over the session ("Duplicate as Companion: <reason>"), a banner only without a
      session (#841); built-in `duplicate_companion` (keyless), palette row (needs a terminal), row and Session menu
      items after New Companion; ctl `--companion --no-select` ("--no-select needs --companion")
- [x] write tests: every Swift case listed above, with a resolved-file seam; the environment parsers per OS; real
      zsh and fish argv round trips where present; process_table image/PATH reads per OS (CI runners)
- [x] write tests: the conformance tests above
- [x] checks sized to the change pass; newly green ids added to the ratchet

### Task 19: Companion sessions, part 7: remote companions and the parent respawn carried to viewers (#840)

**Upstream:** `agterm/Control/ControlServer+RemoteCompanions.swift`, `ControlServer+Respawn.swift`,
`agtermCore/Sources/agtermCore/RemoteRespawn.swift`, `PresentationFrames.swift`, `RemotePresentationClient.swift`,
`RemotePresentationState.swift`, `RemoteSession.swift` (controlCommand, accountShellQuoted), `RemoteTree.swift`,
`agterm/Control/ControlServer+Zmx.swift`, `ControlServer+RemotePresentation.swift`, `ControlServer+RemoteReconnect.swift`,
`ControlServer+RemotePage.swift`, `agterm/AppActions+RemoteAttach.swift`, `AppActions+Companions.swift`,
`AppStore+ReplicaPage.swift`. Tests: `agtermTests/ControlServerRemoteCompanionsTests.swift` (all 17),
`ControlServerRespawnTests.swift` (all 5), `RemoteRespawnTests.swift` (all 11), `RemotePresentationClientTests.swift`
(`aSnapshotsRespawnsAreASeedAndAFramesAreNot`), `RemoteTreeTests.swift` (both), `RemoteSessionTests.swift`
(`controlReachesTheCliWithEveryArgumentIntactUnderAnyLoginShell`), `AppStoreRemoteLayoutTests.swift`,
`agtermTests/RemoteAttachTests.swift`, `AppActionsCompanionsTests.swift`. Contract: `control-api.md` > Companions (the
remote-parent, remote page and respawn bullets, the presentation frame list), `commands.html`.

**Rust side:** `crates/ngterm-core/src/remote/{companions,respawn}.rs` (create); `remote/{frames,client,hub,layout,reconnect,ssh,tree,attach,replica_page}.rs`,
`model/remote_presentation.rs`; `crates/ngterm-proto/src/result.rs` (`ControlRemoteSession.parent`, `.content`,
optional); `crates/ngterm-headless/src/state.rs` (the origin side); `crates/ngterm-gpui/src/deck.rs` (modify);
`conformance/docs/rust-build.md` > The `live` group and `conformance/agterm_conformance/vocabulary.py` `LIVE_TEXTS`
for each new string that names zmx.

**Platforms:** the origin runs over ssh (OpenSSH on every OS; Windows ships `ssh.exe`); the remote line is `/bin/sh
-c CHAIN` with upstream's `accountShellQuoted` outer quoting so sh, zsh, tcsh and fish read it alike, used for the
companion control lines (`control_command`) only unless the change means to cover present/run-job/page-serve too; a
Windows origin needs its own remote form, already open for the existing remote lines (`remote/ssh.rs:61-62`).
Respawn leader detection uses `ngterm-session`'s leader listing; liveness `kill(pid, 0)` on POSIX, `OpenProcess` +
`GetExitCodeProcess == STILL_ACTIVE` on Windows.

**Conformance:** existing `test_zmx.py::test_present_mirrors_a_live_session`, `::test_tree_rows_carry_the_session_status`,
`::test_tree_without_a_host_lists_no_attachable_session_outside_live`, `::test_page_serve_serves_a_page_a_presenter_shows`,
`test_relaunch.py::test_a_live_session_with_a_hidden_split_stays_attachable_across_the_relaunch` (listed). New,
origin side only (LIVE): `test_zmx.py::test_tree_lists_a_page_companion_without_panes_under_its_listed_parent`,
`::test_a_respawned_live_parent_publishes_its_respawn_to_a_presenter`. Catalog exemptions, reason "needs zmx.attach
over ssh, excluded by the suite's no-network constraints": the remote creation path, the viewer reattach on respawn,
the viewer side of a remote page companion.

- [x] `createRemoteCompanion` (terminal | duplicate | page): "the remote session is not attached", "invalid host",
      "the session on <host> is not attachable now; nothing was created", one origin run (30 s, never retried, never
      done locally) with upstream's argv (`session new --parent O --no-select …`, `session duplicate --companion
      --target O --no-select`, `session overlay open --companion …`, each `--json`)
- [x] reply handling: "the ngterm on <host> is too old for companions", "<host> gave no answer[ (stderr)]; whether
      the companion was created there is unknown", "created companion <id> on <host> but the connection failed before
      attaching it; `ngtermctl live attach <host> <id>` attaches it", "the remote command failed on <host>", "<host>
      created a companion but did not name it"; then the tree poll (20 s) and the attach under the parent:
      "its parent row here was closed", "created companion <id> on <host> but could not attach it: <reason>;
      `ngtermctl live attach <host> <id>` attaches it" ("it did not become attachable", "attach failed")
- [x] remote page companions: "the ngterm on <host> is too old for page companions", "<host> did not hand the page
      over", "a page on <host>'s loopback cannot be shown on this Mac", "a loopback page cannot be a companion of a
      remote session" (before the origin is asked), "the parent row here is gone", "the page cannot be shown here",
      "the page cannot be reached on <host>"; the page's outcome there becomes the view's
- [x] `live attach HOST ID` of a page row: "attach the page's parent session from <host> first", "more than one row
      here shows the page's parent"; the Attach Remote picker hides page rows; `live tree` lists page companions with
      no panes under a listed parent, plus `parent`
- [x] respawn: the origin publishes `pane.respawn` {epoch, generations} once a different live leader runs under the
      name (wait up to 10 s), the snapshot carries every count; the viewer's RemoteRespawnBook (seed once from a
      snapshot, an epoch change never read as older, 3 attempts per generation, pruned on the tick while no open or
      soft-closed row owns the pane) reattaches a held clean exit; closing a remote companion here only detaches it
- [x] GUI: New Companion and Duplicate as Companion on a remote row show the progress HUD ("<Action>: creating a
      companion on HOST…", "attaching the companion…") and the failure HUD
- [x] write tests: every Swift case listed above on a fake remote runner in `ngterm-core`; `control_command` quoting
      through real sh/zsh/fish (tcsh where present) on POSIX CI
- [x] write tests: the origin-side conformance tests and the exemptions above
- [x] checks sized to the change pass; newly green ids added to the ratchet (none: the three new conformance ids
      are since(0.39.1), green at a temporary 0.39.1 level; they go in at Task 25)

### Task 20: Agent-status copy: merge the v0.39.1 hooks and port their tests (#802, #822, #828)

**Upstream:** `git -C .upstream/agterm diff v0.38.1 v0.39.1 -- agterm/Resources/agent-status` (the Codex blocker poll
every 2 s with `max_checks` 5400; the memory-consolidation thread's `transcript_path: null` ignored on session-start,
user-prompt-submit, pre-tool-use and post-tool-use; the bash `_ags_precmd` keeping `$?`). Tests:
`CodexStatusHookTests.swift` (`eventWithoutTranscriptLeavesStatusAndWatcherAlone`, `eventWithTranscriptReportsStatus`),
`AgentStatusIntegrationTests.swift` (`precmdKeepsTheLastCommandStatusForLaterPromptCommandEntries`).

**Rust side:** `resources/agent-status/ngterm-codex-status.sh`, `resources/agent-status/shell/integration.sh`,
`build/patches/agent-status/0001-linux-glue.patch` … `0004-windows-forms.patch` (refresh: `sync-resources.sh` reported
`0002-ngterm-identity.patch no longer applies to v0.39.1, why: error: patch failed: agterm-codex-status.sh:92`),
`crates/ngterm-app/src/integrations/mod.rs` (the release in its header), new tests under `crates/ngterm-app/tests/`.

**Platforms:** upstream detects the null `transcript_path` with `/usr/bin/plutil -type transcript_path`; the Linux
glue reads it with python3 or jq (as `0001-linux-glue.patch` already does for `last_assistant_message`), plutil kept
first on macOS; with none of the three the event is not filtered (fail open). Windows has no Codex hook script in
this build (no `.ps1` form): record that the Codex change does not reach it. The bash integration change is POSIX
only; fish is unchanged upstream.

**Conformance:** none (hook scripts).

- [x] rebuild `resources/agent-status` as upstream's v0.39.1 tree plus the refreshed patches, keeping the Linux glue
      and the `ngterm-*`/`NGTERM_*` identity; `bash .claude/skills/agterm-upstream-sync/scripts/sync-resources.sh
      --check v0.39.1 v0.39.1` (or the script's check mode) holds; bump the release in
      `crates/ngterm-app/src/integrations/mod.rs`'s header
- [x] write tests: Rust twins of the two new Codex cases (run the shipped `ngterm-codex-status.sh` with recording
      wrappers, in this build's names, with python3 and with jq) and of `precmdKeepsTheLastCommandStatusForLaterPromptCommandEntries`
      (bash with a user `PROMPT_COMMAND` entry, `agentRan` × status 0/3); the rest of the Codex suite stays the
      backlog item `docs/backlog/claude-codex-and-opencode-v2-hook-tests-have-no-rust-twin.md`
- [x] checks sized to the change pass; newly green ids added to the ratchet (none: hook scripts have no conformance ids)

### Task 21: Bundled skill: rebuild `resources/skills/ngterm` on the v0.39.1 tree

**Upstream:** `git -C .upstream/agterm diff v0.38.1 v0.39.1 -- plugins/agterm/skills/agterm` (SKILL.md, examples.md,
reference.md, troubleshooting.md: companions, tints, flagged move, sized link modes, `--undoable`, `lastSelectedAt`,
the attention settings, Ignore Reduce Transparency, the `claude --bg` status routing and the mouse-reporting
correction #821).

**Rust side:** `resources/skills/ngterm/*`, `build/patches/skill-ngterm/*` (refresh: `sync-resources.sh` reported
`0001-live-group.patch no longer applies to v0.39.1, why: error: patch failed: reference.md:15`).

**Platforms:** no OS-specific code (the copy's Linux and Windows wording, where the patches carry one, is kept).

**Conformance:** none.

- [x] rebuild the copy as upstream's v0.39.1 tree plus the refreshed patches, in order (the `live` names, the
      `ngterm`/`ngtermctl` names, the attach contract's `NGTERM_LIVE_*`, the rest of the identity); regenerate a
      conflicting patch as `diff -u` of upstream's files and the hand-merged copy; every new `agtermctl zmx`/`agtermctl`
      call becomes `ngtermctl live`/`ngtermctl`
- [x] `sync-resources.sh --check v0.39.1 v0.39.1` holds; the skill's installer tests in `ngterm-app` stay green
- [x] checks sized to the change pass; newly green ids added to the ratchet (none: the skill has no conformance ids)

### Task 22: `split-divider-color` and the themed divider (#829); the launch-order and teardown tests (5b5c437, #819)

**Upstream:** `agterm/Ghostty/GhosttyApp.swift` (configuredSplitDividerColor, splitDividerColor), `Views/SplitRatioAccessor.swift`,
`WindowContentView.swift`, `WindowContentView+Detail.swift`. Tests: `agtermTests/GhosttyAppTests.swift`
(`testTheConfiguredSplitDividerColorWins`, `testTheDefaultSplitDividerIsTheBackgroundATenthTowardTheForeground`,
`testReadsSplitDividerColorAsSetChangedAndRemoved`), `SplitRatioAccessorTests.swift`,
`agtermUITests/SeededSettingsLaunchUITests.swift` (`testFirstLaunchTerminalUsesSeededSettings`),
`scripts/test-surface-teardown.py` (its three cases, for the shape). Contract: `site/docs.html` (ghostty.conf section).

**Rust side:** `crates/ngterm-core/src/ghostty_config/model.rs` (`split_divider_color`), `ghostty_config/tests.rs`,
`chrome.rs` (`split_divider_color(configured, background, foreground)`); `crates/ngterm-gpui/src/deck.rs` (`Look`
carries it, `separator()` paints it opaque) (modify); `crates/ngterm-term/tests/platform.rs` (modify: the teardown
regression); `conformance/tests/protocol/test_font_theme.py` (modify).

**Platforms:** no OS-specific code; the teardown test runs on real PTYs on all three OSes (a POSIX shell loop, a
cmd/pwsh loop under ConPTY), in `ngterm-term` (`any`).

**Conformance:** existing `test_font_theme.py::test_font_steps_the_main_pane_by_one_point_and_resets` (listed). New:
`test_font_theme.py::test_a_fresh_state_dir_starts_its_first_terminal_on_the_seeded_settings` (seeded fontSize 27,
backgroundOpacity 0.6, backgroundBlur 20 → the first session's `fontSize` 27) and a stale-file variant (a
`ghostty-settings.conf` overwritten with `font-size = 11` before relaunch still yields 27).

- [ ] read `split-divider-color` on every config load (set, changed, removed → none); the divider takes it, else the
      theme background mixed 10% toward its foreground per sRGB channel, opaque; a reload repaints it;
      `unfocused-split-fill`/`unfocused-split-opacity` stay unread
- [ ] write tests: core (configured wins; black/white → about 25-26 per channel; "#808080" → (128,128,128),
      "#ff8000" → (255,128,0); "font-size = 13" → none; an empty later line clears it); gpui `Look::of` carries it
- [ ] write tests: the `ngterm-term` teardown regression: one pane looping on OSC 2 titles and output, dropped,
      returns within 5 s; eight such panes, one dropped, the other seven keep getting titles and screen text; eight idle
      panes, one dropped; plus a live pane's drop (its `writer.detach()` must not block)
- [ ] write tests: the two conformance tests above (no Rust change expected: Context)
- [ ] checks sized to the change pass; newly green ids added to the ratchet

### Task 23: Reduce Transparency: the system rule and Ignore Reduce Transparency (36970bc)

**Upstream:** `agtermCore/Sources/agtermCore/AppSettings.swift` (ignoreReduceTransparency,
windowReducesTransparency(system:ignoring:)), `agterm/SettingsModel.swift`, `Ghostty/GhosttyApp.swift`,
`Views/AppearanceSettingsView.swift`, `WindowAppearance.swift`, `WindowAccessor.swift`, `WindowContentView.swift`.
Tests: `AppSettingsTests.swift` (`ignoreReduceTransparencyDefaultsNilAndOmitsFromJSON`,
`ignoreReduceTransparencyRoundTripsAndIsNotAConfigLine`, `windowReducesTransparencyOnlyWhenSystemIsOnAndNotIgnored`),
`agtermTests/WindowAppearanceTests.swift` (both), `SettingsUITests.swift`
(`testIgnoreReduceTransparencyTogglePersistsAndAppearanceTabFits`). Contract: `settings.md`, `libghostty.md`,
`site/docs.html` (Window section).

**Rust side:** `crates/ngterm-core/src/persist/settings.rs`, `chrome.rs` (`window_reduces_transparency`),
`settings_model.rs`, their tests, `theme.rs` tests (no config line); `crates/ngterm-gpui/src/reduce_transparency.rs`
(create: the system flag as a Global, followed per OS like `status_glyph.rs`'s `Motion`), `chrome.rs`
(`Translucency` opaque and unblurred while the rule applies), `settings/appearance.rs`, `settings.rs`,
`settings/tests.rs`, `Cargo.toml` (the Windows `UI_ViewManagement` feature if missing) (modify).

**Platforms:** ngterm never ported the system rule under this setting either (no reduce-transparency code today), so
both land here. macOS: `NSWorkspace.shared.accessibilityDisplayShouldReduceTransparency`, followed through
`NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification` (with the GPUI macOS form, open work). Windows:
"Transparency effects", `Windows.UI.ViewManagement.UISettings::AdvancedEffectsEnabled()` (registry mirror
`HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize\EnableTransparency`), re-read on
`WM_SETTINGCHANGE` as `status_glyph.rs`'s `watch_system_motion` does. Linux: no counterpart (the XDG desktop portal's
`org.freedesktop.appearance` carries color-scheme, accent-color, contrast and reduced-motion only; GNOME's a11y schema
has no transparency key), so the system input is false there: the toggle shows and round-trips for parity and a
shared `settings.json`. The rule's logic lives in `ngterm-core`; the per-OS reads are "not built on macOS/Windows by
CI until `ngterm-gpui` joins `cross-platform.yml`".

**Conformance:** `tests/ui/test_settings.py::test_appearance_tab` gains `settings-ignore-reduce-transparency`; new
`test_settings.py::test_ignore_reduce_transparency_toggle_persists` (ui layer).

- [ ] while the OS preference is on the window presents opaque and unblurred, the saved opacity and blur untouched;
      the inactive-pane wash scales by that effective opacity; palettes and the switcher keep following the system
- [ ] settings.json `ignoreReduceTransparency` (nil by default, true stored, off removes the key, no config line);
      `window_reduces_transparency(system, ignoring) = system && !ignoring` for the window, the wash and the hint
- [ ] Appearance ▸ Window toggle "Ignore Reduce Transparency" (`settings-ignore-reduce-transparency`) after
      Background Blur; the hint "Reduce Transparency is on; saved opacity and blur apply when it is off." while the
      rule holds, else "Blur needs opacity below 100%."; follows the system flag live
- [ ] write tests: the Swift cases above (core truth table, settings round trip; gpui `Translucency`)
- [ ] write tests: the ui tests above
- [ ] checks sized to the change pass; newly green ids added to the ratchet

### Task 24: UI-only changes

- [ ] Settings ▸ Interface: each toggle beside its control's resting glyph, text-only elements (workspace, session and
      window name, session context) first in the Title Bar section (#810): `InterfaceElement::settings_order` in
      `ngterm-core` (tests `everyControlElementHasADistinctSymbolAndTextElementsHaveNone`,
      `settingsOrderListsTextOnlyElementsFirst`); one element→glyph map in `ngterm-gpui` that the titlebar, sidebar
      footer and Settings rows share, an 18px leading slot (empty for text-only); a test that every glyph resolves
      (`testEveryInterfaceElementSymbolResolves`); check Dashboard and Focus Filter glyphs read as distinct
- [ ] the GUI `ask` one-line field (#842): a light rounded box (radius 6, fill panel ink at 0.06, a 1px border at ink
      0.1), text, caret and placeholder inset 6 and centered vertically, height line + 12, the whole box a click
      target, no gpui-kit focus ring; the terminal style unchanged (`crates/ngterm-gpui/src/ask_dialog/form.rs`, tests
      after `testAGuiLineFieldEditsInsetAndCenteredInsideItsBox`, `testALineFieldDrawsNoAppKitFrameAndOnlyTheGuiOneInsetsItsText`);
      decide whether `form_min_height` follows the +12/+4 split
- [ ] no counterpart on any OS: the AppKit tooltip workaround (#816, Context); no macOS form yet for the items above
      beyond the GPUI macOS frontend's open work
- [ ] checks sized to the change pass; newly green ids added to the ratchet

### Task 25: Claim v0.39.1 parity and update the docs

- [ ] bump `PARITY_LEVEL` in `crates/ngterm-proto/src/version.rs` and its mirrors (`tag-mentions.sh v0.38.1 v0.39.1`
      class `claim`: the version.rs test, `ngterm-core/src/dispatch.rs` and `ngterm-server/tests/server.rs`
      expectations, `conformance/agterm_conformance/impls/rust.py`, `conformance/docs/rust-build.md`), only once every
      task above is done; then run the `@since("0.39.1")` tests the earlier tasks added (a partial `run-linux.sh -k`)
      and add their newly green ids with `check_ratchet.py --add`
- [ ] README.md, CLAUDE.md, conformance/README.md, conformance/docs/rust-build.md describe what changed (companions,
      tints, the flagged order, the new settings keys, the Live `--command` behavior change); `tag-mentions.sh
      v0.38.1 v0.39.1` lists no stale `claim` line; add rules to `tag-mentions.sh` for the `review` lines this sync
      met (historical citations of v0.38.1 in `build/patches/gpui-pre-linux/0004-keymap-change.patch`,
      `crates/ngterm-app/tests/opencode_status.rs`, `crates/ngterm-session/src/server/upkeep.rs`, `docs/backlog/*`,
      `scripts/vendor-gpui.sh`: `keep`)
- [ ] checks sized to the change pass; newly green ids added to the ratchet
