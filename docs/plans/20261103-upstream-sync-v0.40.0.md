# Upstream sync v0.39.1 -> v0.40.0

## Overview

- Bring ngterm from `umputun/agterm` v0.39.1 to v0.40.0 (24 commits, one release, published 2026-11-02).
  Branch `upstream-sync-v0.40.0`, mechanical commit `3c44c090`, updates the generated catalog/accessibility IDs,
  Swift reference parity, current-catalog selftest snapshots, reference commands and review profile.
- Rust still claims 0.39.1. No application behavior, native pin, ratchet entry or vendored payload changed.
  The bundled skill cannot yet be regenerated because its first local patch conflicts; its provenance header
  therefore remains v0.39.1. Markdown introduces renderer assets requiring deliberate integration.
- Port messaging; session info; duplicate identity rebinding; remote companion groups; image/Markdown pages;
  remote overlay sizing/terminfo; managed attach cleanup; refresh the bundled skill; finish desktop UI/settings;
  then claim parity. Each task is reviewed through `plan-exec-revmux` before proceeding.

## Context

Upstream is `.upstream/agterm` at v0.40.0, commit `c63e5158e637662ded7ba3a12eac4881b24caef1`.
The old tag is `405911244e469f8bf3dcce0509f4bc2f3a3f3328`. Inspect with
`git -C .upstream/agterm diff v0.39.1 v0.40.0 -- <paths>`; upstream stays read-only.

Changes, grouped by behavior rather than source-code area:

- #847 (`b083969`): in-memory pane mailboxes, send/read/ack CLI, pending counts and body-free message events.
- #864 (`80f822b`): ephemeral session info, inline markup, expiry, remote presentation and sidebar display mode.
- #843 (`8400543`): copied foreground arguments use the duplicated session/panes' identities.
- #849 (`c2c9efc`): remote group membership, partial group attachment, single-companion placement and picker/CLI.
- #844/#866 (`4cfb58c`, `fb614c9`): image/Markdown file overlays, local path routing, bounded readers and renderer.
- #850/#854 (`714ec61`, `fe3c5a6`): remote overlay points and bundled terminfo for overlay job helpers.
- #859 (`c59c839`): managed attach exit keeps output and scrollback; no replacement shell for an ended command.
- #865 (`9133258`): independent workspace/session icon settings.
- #858 (`b1ccd56`): hidden split divider cursor; #869/#856 (`3187bc4`): macOS header window-drag region.

Baseline after the mechanical bump:

- `cargo test -p ngterm-proto --lib`: 52 passed. Both catalog-test binaries build successfully.
- `cargo test -p ngterm-proto --test catalog`: 1 passed, 3 expected failures: command set, command parsing and
  event set lack the five new commands and `message`. Task 1 fixes them.
- `cargo test -p ngterm-ctl the_command_tree_is_the_catalogs_cli`: expected failure, four missing CLI leaves.
  Task 1 fixes it.
- `.venv/bin/python -m pytest conformance --coverage-gate`: 22 uncovered entries, listed in Task 1.
  Task 1 covers the contract (or narrowly exempts a remote-only result with justification), without pretending
  the Rust behavior exists yet.
- Full host-free selftests: 1138 passed, 34 skipped, three failures on the first run. Two were old committed-catalog
  tag/count snapshots; mechanically updated in the bump, then all 82 `test_gen_catalog.py` tests passed.
  The only remaining failure is `test_coverage.py::test_committed_catalog_passes_the_ci_gate`, the same expected
  22-entry coverage failure; Task 1 fixes it. Other skips are the selftests' host/platform-dependent cases.
- `cargo fmt --all --check`, `cargo metadata --locked --format-version 1` and `scripts/lint-report.sh` pass.
- No application/conformance ratchet execution was needed for this catalog/reference-only bump. Runtime code,
  Rust parity, existing test semantics and both ratchet lists are unchanged. Cross-platform builds and the full
  ratchet remain checks for plan execution/PR CI, not claimed results of this sync.

Already in Rust, with boundaries of the evidence:

- Safe listener ownership/shutdown (#844): `crates/ngterm-server/src/lib.rs:252` moves the owned listener into
  its thread; `stop` at line 285 wakes and joins it before endpoint removal. Default Unix streams are blocking
  (`src/endpoint/unix.rs`). `tests/server.rs::stop_removes_the_socket_and_releases_the_lock` and
  `stop_returns_every_time` cover restart and 100 stop/wake cycles. No Swift poll/wake-pipe port is needed.
- Local overlay points: `crates/ngterm-core/src/model/overlay_points.rs` and
  `model/tests/overlay_points.rs::` tests; `commands/tests/overlay_points.rs` asserts merging and sizing.
  Listed conformance IDs include `tests/protocol/test_overlay.py::test_overlay_points_size_and_resize` and
  `::test_page_points_leave_out_the_strip`. Remote points are still absent (Task 7).
- Ending Live creation commands and short-output hold: `crates/ngterm-term/src/session_io.rs` creates+attaches
  in one operation and reports exit without RIS; reconnect sends no new creation spec. Listed
  `tests/protocol/test_live_sessions.py::test_a_live_command_session_closes_when_its_command_exits_and_holds_with_wait`
  and `tests/protocol/test_live_any_shell.py::test_an_ended_live_command_closes_its_pane` cover current behavior.
  CLI cleanup and more-than-one-screen regression coverage remain partial (Task 8).
- Shared interface visibility, metadata/context and companion/page machinery are implementation seams only;
  they do not establish that new messages, info, icon toggles or group attach are ported.

Pins and resources:

- Upstream `ZMX_REV` remains `8bab1f0173b07e79835ea372d749af3dbf0d0842`, `GHOSTTY_REV` remains
  `683d8db643b95cf229bfb5fe9fab9ae677920343`. No local pin bump, including herdr or `LIBGHOSTTY_VT_REV`.
- New upstream `scripts/zmx-patches/0004-managed-exit-keeps-screen.patch` and its README/checker change become
  Rust behavior in Task 8. This build runs no zmx.
- Agent-status payload, ghostty defaults and upstream license are unchanged. Skill patch
  `build/patches/skill-ngterm/0001-live-group.patch` fails at `reference.md:274`; Task 9 refreshes all affected
  patches and regenerates the full payload. Markdown resources are integrated in Task 6.

No port needed:

- Upstream CI compile parallelization (#845), build/release helper orchestration and macOS helper prerequisites.
- Fish seam, keymap marker and session-wait/host fixture changes: diagnostic output, content-based waits and
  bounded test cleanup, with no new app contract (`46eac03`, `0716007`, diagnostic parts of `80f822b`).
- `ControlActions.swift`/`ControlDispatcher+Restore.swift` extraction and relocated code: no behavior change.
- Full embedded-Ghostty surface-teardown mailbox patch: ngterm embeds libghostty-vt and owns its worker/mailbox,
  not upstream's `App.zig`/`Surface.zig` app runtime. Its mailbox is a `VecDeque` with nonblocking delivery and
  explicit shutdown (`crates/ngterm-term/src/mailbox.rs`, tests `output_comes_in_order_and_ends_after_the_last_read`,
  `a_shutdown_passes_queued_output`, `the_childs_exit_ends_a_wait_on_an_answer_and_every_later_one`). Do not copy
  the unrelated App.zig teardown implementation or its benchmark script.
- Upstream cookbook recipes `named-companion`, `end-of-task-offer`, `container-overlays` and its follow-up,
  plus `session-context-nudge` hook chaining: optional user-installed workflows, not shipped ngterm resources
  or app behavior. No installation into the user's setup is part of this sync.
- Upstream `docs/plans`/`docs/backlog` are design/background, not targets; architecture/README/release metadata,
  plugin manifests, project.yml packaging and site index/style/llms/companions marketing need no port.
- Tag review: Cargo.lock/WebView2 0.39.1 is a dependency version, existing `since("0.39.1")` is a minimum release,
  source-attribution comments and patch provenance are historical. Leave each intact. Task 9 handles the
  obsolete v0.39.1 skill-correction patch based on the new upstream text.

## Approach

- Follow `CLAUDE.md` > Porting a command group and Choosing checks. Upstream Swift files/tests hold the exact
  validation order, strings and optional-field shapes. All upstream paths below are relative to `.upstream/agterm`;
  Rust paths are relative to this checkout. Test names without a directory are in the named test file.
- macOS, Linux and Windows are equal targets. Put platform-neutral work in `any` crates and make bounded file
  IO/renderer/platform-reveal choices explicitly for each OS. Preserve ngterm naming, `live.*`, `NGTERM_*`,
  platform quoting and vocabulary adapters; do not add agterm/zmx fallbacks.
- Add `since("0.40.0")` to new release-specific conformance cases. Do not claim a skipped case as verification
  or add skipped IDs to the ratchet. Feature tasks validate through Rust unit/integration tests and tests
  executable at the current level; Task 11 unlocks the new cases, verifies and ratchets them.
- The user runs new conformance tests on the macOS reference (`reference-macos.yml`). Successful remote SSH
  dependencies remain outside the network-free suite: use fake-runner Rust tests and precise existing-style
  coverage exemptions where no supported conformance seam can observe a result.
- Do not push/open a PR/run the reference workflow. Do not touch the live ngterm: strip `NGTERM_*` before
  launching tests, use private state, explicit sockets and the existing headless GUI test runner.

## Implementation Steps

### Task 1: Register new wire/CLI shapes and cover the reference contract

**Upstream:** `agtermCore/Sources/agtermCore/{ControlProtocol,ControlEvents,ControlPayloads,ControlProjection,ControlDispatcher+Messages,ControlDispatcher+SessionMetadata,ControlDispatcher+Zmx}.swift`;
`agtermCore/Sources/agtermctlKit/{Commands,MessageCommands,SessionMetadataCommands,ZmxCommands}.swift`;
`site/commands.html`, `.claude/rules/control-api.md`.
**Swift tests:** `agtermCore/Tests/agtermCoreTests/{ControlDispatcherMessagesTests,ControlDispatcherSessionMetadataTests,ControlDispatcherZmxTests,ControlEventProtocolTests}.swift`;
`agtermCore/Tests/agtermctlKitTests/{MessageCommandsTests,SessionMetadataCommandsTests,ZmxCommandsTests}.swift`;
`agtermUITests/{ControlMessageUITests,ControlSessionInfoUITests,SettingsUITests}.swift`.
**Rust side:** modify `crates/ngterm-proto/src/{command,request,result,tree,events}.rs`,
`crates/ngterm-core/src/dispatch.rs`, `crates/ngterm-ctl/src/{main,session,session_meta,live,events}.rs`;
create `crates/ngterm-ctl/src/messages.rs` and conformance files named in Tasks 2/3/5/10.
**Conformance:** all new 22 keys below are currently uncovered; no existing IDs cover them.

- [x] Register `msg.send`, `msg.read`, `msg.ack`, `session.info`, `live.attach.group` and event `message`, plus
  request/result types; use the dispatcher's established not-implemented refusal until their feature task lands.
  Preserve existing scalar `args.to`; message recipients use their own list. Add the CLI leaves `msg send`,
  `msg read`, `msg ack`, `session info` and `live attach --companions`; no success stubs.
- [x] Introduce substantive reference tests for these keys: the five commands; `field:result.messageID`,
  `field:result.delivered`, `field:result.rejected`, `field:result.messages`, `field:result.remaining`,
  `field:result.acked`, `field:result.remoteGroup`; `field:tree.session.info`,
  `field:tree.session.infoMarkdown`, `field:tree.session.infoHideAfter`; `event:message`; the four CLI leaves;
  `ui:session-info`, `ui:settings-session-info-display`. Tasks 2/3/5/10 specify source cases and proposed IDs.
  Assertions must inspect their named behavior/field, not merely label a skeleton test as coverage.
- [x] For `remoteGroup`, first look for a supported network-free receipt seam. If none, add a narrow catalog
  exemption with the existing remote-attach rationale, backed by the fake-runner receipt tests in Task 5;
  do not invent a successful SSH call against an external machine for the conformance gate.
- [x] The current generator does not list `tree.surface.pendingMessages`; still assert omission at zero and
  changes on delivery/ack. If extending catalog generation for this field, add generator tests and cover the new
  key in this task as well. Keep generated files reproducible through `gen_catalog.py`.
- [x] Both catalog cargo checks, all host-free selftests and the coverage gate pass. Existing ratchet IDs stay
  present. Reference-specific new tests remain gated on Rust until the final parity task.

### Task 2: Implement the pane message bus (#847)

**Upstream:** `agtermCore/Sources/agtermCore/{MessageAddress,MessageBus,AppStore+Messages,ControlDispatcher+Messages,ControlEvents,ControlProjection,AppStore+Panes,AppStore+Companions,AppStore+RemoteOverlay}.swift`;
`agterm/Control/{ControlServer+Messages,ControlServer+Restart}.swift`;
`agtermCore/Sources/agtermctlKit/{MessageCommands,EventCommands}.swift`.
**Swift tests:** all cases in `agtermCore/Tests/agtermCoreTests/{MessageAddressTests,MessageBusTests,AppStoreMessagesTests,ControlDispatcherMessagesTests}.swift`,
especially `aBatchNeverExceedsTheCapAndReadAndAckKeepsTheRest`, `ackIsScopedToTheReadersMailbox`,
`aFullMailboxRefusesThatRecipientOnly`; all `agtermCore/Tests/agtermctlKitTests/MessageCommandsTests.swift`;
`agtermTests/ControlServerMessagesTests.swift`, `ControlServerRestartTests.swift::{testARestartDropsTheOldProgramsMailBeforeTheNewOneStarts,testMailSentWhileTheOldProgramEndsDoesNotReachTheNewOne}`;
`agtermUITests/ControlMessageUITests.swift::{testASwapKeepsTheMailboxWithItsPaneID,testReadWithAckEmptiesTheMailboxAndTheNextReadIsEmpty,testTheBundledCLISendsAndReadsAsAPane,testTwoPanesExchangeAMessageThroughTheSocketAndAckClearsThePendingCount}`.
**Rust side:** create `crates/ngterm-core/src/messages.rs`, `commands/messages.rs`, `commands/tests/messages.rs`;
modify `lib.rs`, `dispatch.rs`, `projection.rs`, `commands/{mod,restart,restart_live}.rs`, model pane/session/window
lifecycle, protocol shapes from Task 1; finish `crates/ngterm-ctl/src/messages.rs`, events/output/usage.
**Conformance:** new `tests/protocol/test_messages.py::{test_send_read_ack,test_read_ack_is_atomic_and_bounded,test_message_limits_and_partial_rejections,test_broadcast_scope_and_identity,test_aliases_deliver_once,test_message_event_has_recipient_location_and_no_body,test_mailbox_follows_pane_identity,test_mailbox_lifecycle,test_restart_drops_mail_sent_during_shutdown}`;
new `tests/cli/test_messages.py::{test_send_read_ack_output,test_identity_environment,test_send_partial_failure,test_read_wait_timeout,test_read_ack_requires_confirmation}`;
cover the message command/result/event/CLI keys listed in Task 1. No preexisting feature IDs.

- [x] Implement an app-global in-memory bus keyed by stable pane identity across windows. Messages carry id,
  optional from, canonical to, epoch-seconds ts and body; a send uses one ID across recipients. Recipients allow
  UUID/role aliases, but sender identity must be UUID. Exact errors include `invalid address "X": expected SESSION:PANE`,
  `address "X" names a session, not a pane`, `identity "X" must name a pane id, not a role`.
- [x] Validate in Swift order: body required; exclusive recipient modes; one mode required; workspace only for
  broadcast; UTF-8 body size; sender; recipients. Preserve `msg.send requires a body`,
  `msg.send takes --to or --all, not both`, `msg.send requires --to or --all`,
  `msg.send: --workspace narrows --all only`, `message body too large (N bytes, max 65536)`,
  `msg.read requires an identity: --as SESSION:PANE-ID`, corresponding ack identity error,
  `msg.ack requires message ids`, `invalid message id: X`.
- [x] Realized local primary/split/scratch panes qualify; remote rows and unrealized splits do not. Reject forged
  session/pane pairs (`pane P is not a live pane of session S`), never degrade stale identity to anonymous, and
  deduplicate recipients by pane token. Broadcast spans all windows, skips sender and optionally narrows workspace.
  Explicit missing recipients get `unknown pane` per-recipient rejections.
- [x] Cap body at 65536 bytes, mailbox at 256 messages/1 MiB body bytes, read batch at 32. Overfull recipient
  reports `mailbox full` without stopping others. Reads repeat oldest-first; read+ack is atomic and removes exactly
  that batch; ack is reader-scoped/idempotent. Emit one body-free `message` event at each recipient's owning
  window/workspace. Tree pending counts are omitted at zero.
- [x] Dispose on pane/session/workspace/window close, soft close, respawn and restart. Preserve surviving pane's
  mail on promotion, realized hidden split mail and identity through swaps. Dispose restart mail after old program
  shutdown and immediately before replacement; do not let mail sent during shutdown reach the new program.
  No persistence or undo resurrection.
- [x] CLI supports TEXT xor `--stdin`, repeated `--to`, `--all`, workspace, `--as` overriding the complete
  `NGTERM_SESSION_ID`/`NGTERM_PANE_ID` pair. Keep raw JSON and exact human records (`ID`,
  `rejected ADDRESS: REASON`, `ID from SENDER`, body, `N more pending`, `acked N`). Adapt app identity through
  vocabulary and document the wording; no upstream-name fallback.
- [x] `read --wait` polls real reads every 250 ms with a monotonic positive finite timeout, exits 124 with
  `timed out`. With ack, print nothing unless the app confirms precisely the returned IDs; preserve
  `error: the app did not confirm which messages it removed; nothing was printed`.
- [x] Port all Swift cases, validate protocol/CLI/lifecycle through Rust tests and scoped checks, and ratchet only
  IDs that actually ran green (new since-gated IDs may wait for Task 11).

### Task 3: Implement session info, expiry and remote presentation (#864)

**Upstream:** `agtermCore/Sources/agtermCore/{SessionInfoMarkup,Session,ControlDispatcher+SessionMetadata,ControlProjection,AppStore+SessionState,AppStore+PendingClose,AppStore+Presentation,AppStore+RemotePresentation,PresentationFrames,PresentationHub,RemotePresentationClient}.swift`;
`agterm/Control/{ControlServer+SessionInfo,ControlServer+RemotePresentation}.swift`;
`agtermCore/Sources/agtermctlKit/SessionMetadataCommands.swift`.
**Swift tests:** all 12 `SessionInfoMarkupTests.swift` cases;
`ControlDispatcherSessionMetadataTests.swift::{sessionInfoRoutesSetAndClear,sessionInfoRequestEncodesItsWireShape,sessionInfoRejectionNeverReachesTheHost}`;
`AppStoreEventTests.swift::settingSessionInfoEmitsTreeChangedOnlyWhenItChanges`,
`AppStorePendingCloseTests.swift::softCloseDropsATimedInfoLineAndUndoKeepsAnUntimedOne`,
`SnapshotRoundTripTests.swift::sessionInfoIsNotSnapshottedOrDuplicated`, info tree projection cases;
`PresentationFramesTests.swift::aSnapshotFromAnOriginWithoutInfoDecodesWithNone`, remote presentation info cases;
all `agtermTests/ControlServerSessionInfoTests.swift` timer cases and remote presentation info tests;
`agtermCore/Tests/agtermctlKitTests/SessionMetadataCommandsTests.swift::{infoSetsMarkdownAndHideAfterVerbatim,infoDefaultsToPlainUntimedOnTheActiveSession,infoClears,infoRejectsBadFormsAtParseTime}`.
**Rust side:** create `crates/ngterm-core/src/session_info.rs` and tests; modify
`commands/metadata.rs`, `commands/tests/metadata.rs`, `model/{session,store,pending_close,snapshot,remote_presentation}.rs`,
`projection.rs`, `dispatch.rs`, `remote/{frames,client,viewer,hub}.rs`;
`crates/ngterm-ctl/src/{session,session_meta}.rs`, protocol shapes from Task 1.
**Conformance:** new `tests/protocol/test_session_info.py::{test_info_set_reject_and_clear,test_info_validation,test_info_expiry_refresh_and_cancel,test_info_not_persisted_or_duplicated,test_info_soft_close_undo,test_info_tree_changed}`;
CLI info cases in `tests/cli/test_cli_output.py`; cover session.info, all three tree info fields, cli:session info,
event:tree.changed. Existing ratcheted context tests assert another field and are not info coverage.

- [x] Implement source/markdown/hideAfter and toolkit-neutral spans. Plain text stays literal; inline markup
  supports bold/italic/code, color/dim attributes and combined traits; links keep labels only, unknown attributes
  become unstyled. Colors red/orange/yellow/green/blue/purple/gray match case-insensitively.
- [x] Enforce 512 UTF-8 source bytes and decoded nonblank, single-line, control-free visible text, including
  LF/CR/U+2028/U+2029/U+0085 handling. Exact `session.info: ` errors: `info text is over 512 bytes`,
  `--hide-after must be 0...86400 seconds`, `info text is empty`, `info text must be one line`,
  `info text contains a control character`, `unknown info color: pink (red|orange|yellow|green|blue|purple|gray)`,
  `invalid info markdown; quote attribute values: ^[text](color: "green")`.
- [x] Set requires text (`session.info set requires text`); clear refuses text or even explicit false/zero flags
  (`session.info clear takes no text, markdown or hide-after`). Bad/missing mode:
  `invalid info mode: X (set|clear)`. Validate before resolving target or mutating old value/deadline.
- [x] Each set replaces the whole value; defaults plain/untimed. Finite fractional seconds 0...86400 are valid.
  Identical timed writes restart expiry; zero/omitted timeout cancels it. Superseded callbacks do nothing. Session,
  workspace/window teardown cancels timers; HUD/overlay close does not. Expiry runs in GUI and headless.
- [x] Keep local/mirrored runtime slots, local wins. Omit all three tree fields when absent; otherwise expose
  configured hideAfter, not countdown. Emit tree.changed only on effective value changes. Do not snapshot,
  relaunch or duplicate info. Soft close drops timed info, while untimed info survives undo.
- [x] Add backward-compatible snapshot info and info frames. Invalid mirrored markup becomes absent; disconnect
  drops mirror only. Origin expiry sends clear; viewer never starts a mirror expiry timer.
- [x] Finish CLI TEXT/clear/defaults/markdown/hide-after/target/window and parser errors
  `provide exactly one of a TEXT or --clear`, `--clear takes no --markdown or --hide-after`,
  `hide-after must be 0...86400 seconds`. Port all named Swift cases, add conformance assertions and scoped checks.

### Task 4: Rebind duplicated companion commands to their own identities (#843)

**Upstream:** `agtermCore/Sources/agtermCore/{DuplicateReplay,AppStore+Companions}.swift`;
`site/commands.html` Duplicate as Companion; `.claude/rules/control-api.md`.
**Swift tests:** `DuplicateReplayTests.swift::rebindingReplacesEachIdInEveryArgumentAndKeepsLowercaseLowercase`;
`AppStoreCompanionsTests.swift::{duplicateAsCompanionRebindsTheSourcesIdentitiesInBothPanes,duplicateAsCompanionPersistsTheReboundCommandOfASinglePane}`.
**Rust side:** modify `crates/ngterm-core/src/duplicate_replay.rs`, `duplicate_replay/tests.rs`,
`model/duplicate.rs`, `model/tests/companion_duplicate.rs`. Today model/duplicate.rs copies the old command/argv
before creating split identities; no rebinding helper exists.
**Conformance:** existing listed `tests/protocol/test_companions.py::{test_duplicate_as_companion_copies_the_split_and_runs_each_program,test_duplicate_as_companion_with_no_select_keeps_the_selection,test_duplicate_as_companion_refusals,test_a_duplicated_program_pane_closes_when_the_program_exits[plain],test_a_duplicated_program_pane_closes_when_the_program_exits[live]}`;
new `test_duplicate_as_companion_rebinds_session_and_pane_ids`, covers session.companion.duplicate.

- [x] Allocate copied session and primary/split identities before launch commands. Replace source session UUID,
  both pane UUIDs and live daemon names everywhere inside raw arguments, case-insensitively; wholly lowercase
  occurrences stay lowercase, mixed case maps to canonical uppercase. Leave unrelated IDs/shells/cwd alone.
- [x] Recompute the platform-quoted command from rebound argv; persist it after rebinding for single and split
  panes. Handle macOS/Linux POSIX and Windows quoting without diverging command versus argv representations.
- [x] Port all three Swift regressions and add a live foreground-argv conformance check of both panes, unrelated
  IDs, embedded/lowercase/mixed-case identities and persistence. Existing IDs remain green; run scoped checks.

### Task 5: Attach remote companion groups and place individual companions (#849)

**Upstream:** `agterm/Control/{ControlServer+RemoteGroup,ControlServer+Zmx,ControlServer+RemoteCompanions,ControlServer}.swift`;
`agterm/AppActions+RemoteAttach.swift`; `agtermCore/Sources/agtermCore/{ControlDispatcher+Zmx,ControlPayloads,RemoteTree,HtmlBridge}.swift`;
`agtermCore/Sources/agtermctlKit/{ZmxCommands,SocketClient,Session+Companions}.swift`;
`site/commands.html`, `.claude/rules/{control-api,menu-actions}.md`.
**Swift tests:** `ControlDispatcherZmxTests.swift::{aGroupAttachReachesTheGroupActionWhoseDefaultRefusesByName,aGroupAttachIsValidatedLikeAPlainOne,theGroupReceiptSurvivesTheWire}`;
`RemoteTreeTests.swift::aParentListsEveryCompanionInSidebarOrderAttachableOrNot`; local-placement HtmlBridge test;
all 11 `agtermTests/ControlServerRemoteGroupTests.swift` cases; single-companion placement cases in
`ControlServerZmxTests.swift`; picker cases in `RemoteAttachTests.swift`; `ControlServerTests.swift::testAGroupAttachDoesNotHoldTheAcceptLoop`;
`ZmxCommandsTests.swift::{attachWithCompanionsSendsTheGroupCommandWithTheSameArguments,aGroupAttachPrintsTheParentThenEachCompanionOriginToLocal,theMembershipNoteNamesTheHost}`;
`SocketClientTests.swift::{anAppTooOldForGroupAttachIsNamedAndNothingElseIsSent,aPartialGroupAttachNamesTheFailureAndTheUnknownMembershipThenFails,aGroupAttachUnderJSONPrintsTheRawReplyAndNoNote}`.
**Rust side:** modify `crates/ngterm-core/src/remote/{attach,tree,companions}.rs`, `html/bridge.rs`, `dispatch.rs`;
create `remote/group_attach.rs` and fake-runner tests as needed; finish `crates/ngterm-proto/src/{command,result}.rs`,
`crates/ngterm-ctl/src/live.rs` and socket/output tests; existing GPUI remote picker integration.
**Conformance:** existing listed `tests/protocol/test_live_sessions.py::{test_tree_lists_a_page_companion_without_panes_under_its_listed_parent,test_tree_rows_carry_the_session_status,test_tree_without_a_host_lists_no_attachable_session_outside_live}`,
`tests/cli/test_cli_output.py::test_live_group_commands_refused_before_acting`;
new `test_group_attach_validates_before_remote_discovery`, membership-order tests and CLI --companions refusal case.
Cover live.attach.group and field:result.remoteGroup as decided in Task 1; success uses fake runners.

- [ ] Add optional remote-session `companions:[{id,name}]` and `remoteGroup:{membershipKnown,companions:[{origin,id?,error?}]}`
  receipt. Distinguish unknown membership from empty []. Project parents' membership from the unfiltered
  workspace, including unattachable members in sidebar order; omit it on companion rows.
- [ ] Validate host before remote session: `live.attach.group requires a host`,
  `live.attach.group requires a remote session`, `invalid remote session`. Fetch one fresh remote tree; refuse
  companion target before creating anything (`<id> is a companion, not a group's parent`). No fallback single attach.
- [ ] Attach parent once, then terminal/page children in order, selecting parent only. Keep successful rows on
  failures and return receipt even with ok:false. Preserve summaries `kept <name> (<parent-id>) and <N> companion(s); not attached: <name> (<reason>), …`,
  missing `unavailable on <host>`, fallback `attach failed`. Old origins attach listed children with membershipKnown:false.
- [ ] Re-read parent location before each async member operation; follow parent moves and stop after close.
  Closed summary `parent row <id> was closed; not attached: <names>`; member reason `parent row was closed`.
  Avoid orphan rows and do not block unrelated control requests during discovery/page attach.
- [ ] Individual terminal companion joins parent only if exactly one destination-window row matches origin
  parent ID, host and endpoint; otherwise top-level. Reuse page placement machinery. Group command is locally
  placed in HTML bridge. Preserve ngterm's existing Attach All features without double attaching children.
- [ ] CLI prints parent ID then `  <origin> -> <local>`; unknown membership emits
  `note: <host> does not report group membership; unlisted companions cannot be detected` even on partial failure.
  Raw JSON prints only original reply, exits nonzero for failure. Unknown group command reports
  `the running app is too old for group attach` and sends no fallback.
- [ ] Picker attaches selected parent through fresh group discovery even if old picker list showed zero children;
  selected companion attaches alone. Labels are `parent › child` when parent listed; partial error HUD moves to
  newly attached parent. Port Swift fake-runner/socket/picker tests, all three OS SSH forms, scoped checks.

### Task 6: Add image and Markdown overlays, path routing and bounded rendering (#844/#866)

**Upstream:** `agtermCore/Sources/agtermCore/{ControlDispatcher+Overlay,ControlDispatcher+Companions,ControlProtocol,ControlProjection,HtmlOverlay,ImagePage,MarkdownPage,LinkPolicy,AppStore+HtmlOverlay,AppStore+RemotePage}.swift`;
`agtermCore/Sources/agtermctlKit/SessionCommands.swift`; `agterm/Control/{ControlServer+ImagePage,ControlServer+MarkdownPage,ControlServer+SessionActions,ControlServer+RemoteCompanions}.swift`;
`agterm/{Ghostty/LinkOpener,Views/HtmlOverlayRegistry,Views/HtmlOverlayView,Views/ImageProbe,Views/MarkdownAssets,Views/MarkdownProbe}.swift`;
`agterm/Resources/markdown/*`; contract `site/commands.html`, `.claude/rules/control-api.md`.
**Swift tests:** new image/markdown validation/routing cases and `pageCompanionRefusesAMarkdownFile`/`pageCompanionRefusesAnImage`
in `ControlDispatcherOverlayTests.swift`; all `ImagePageTests.swift`, `MarkdownPageTests.swift`, path/remote-pane
cases in `LinkPolicyTests.swift`; `HtmlOverlayTests.swift`, source projection tests;
`OverlayCommandsTests.swift::{imageOpenSendsTheResolvedPathAndNoPageFlags,markdownOpenSendsTheResolvedPathInItsOwnField,openRejectsAMissingOrConflictingContent}`;
all `agtermTests/{ImageProbeTests,MarkdownProbeTests,MarkdownAssetsTests,ControlServerLinkOverlayTests}.swift`;
image/Markdown/remote path cases in `GhosttySurfaceViewInputTests.swift`; new renderer cases in `HtmlOverlayRegistryTests.swift`,
especially `testAMarkdownPageKeepsAuthoredMarkupInertAndLoadsOnlyEmbeddedPictures`,
`testARepeatedPictureIsSetThroughTheDomAndCappedAtTheEmbeddedLimit`, `testManyIdenticalHeadingsGetTheirSuffixesInLinearTime`,
`testAnImageReadFinishingAfterCloseIsDropped`;
`agtermUITests/ControlHtmlOverlayUITests.swift::testAMarkdownFileRendersAndTheTreeNamesItsSource`.
**Rust side:** modify `crates/ngterm-proto/src/{request,tree}.rs`,
`crates/ngterm-core/src/{link_policy,projection}.rs`, `html/{mod,navigation,bridge,theme}.rs`,
`commands/{overlay,overlay_page,companions}.rs`, `model/{html_overlay,remote_overlay}.rs`, `remote/page.rs`;
create `html/{image_page,markdown_page}.rs` and frontend-neutral bounded probes in `crates/ngterm-app/src/`;
modify `crates/ngterm-ctl/src/{session_overlay,overlay_page}.rs`;
`crates/ngterm-gpui/src/terminal/links.rs`, OpenUrl routing and `html_overlay/{page,registry,policy,sandbox,engine}.rs`,
including wkwebview/webview2 backends; stage new renderer assets/licenses through existing build/bundle scripts.
**Conformance:** listed `tests/protocol/test_overlay.py::{test_html_page_occupies_the_slot,test_open_refusals,test_reload_and_navigate_a_page,test_page_flags_read_back,test_overlay_opens_in_the_background_unless_it_follows,test_page_in_a_pane_slot,test_overlay_points_size_and_resize,test_page_points_leave_out_the_strip}`;
new `test_image_page_reads_back_its_source`, `test_image_open_refusals`, `test_markdown_page_reads_back_its_source`,
`test_markdown_open_refusals`, `test_local_document_pages_are_not_remote_pages`;
CLI source/conflict cases; `tests/ui/test_html_overlay.py::test_a_markdown_file_renders_and_the_tree_names_its_source`.
Cover session.overlay.open/reload/navigate, session.companion.page, field:tree.session.htmlOverlays,
cli:session overlay open and ui:htmlOverlay.page; do not invent nested catalog keys.

- [ ] Add image and markdown sources to args/tree, `markdownFile` distinct from HUD markdown. Exactly one source
  field projects. Current HtmlSource has File/Url/Replica only; add variants without granting normal file-page powers.
- [ ] Match validation order/conflicts and exact strings: `session.overlay.open takes one of a command, --html, --url, --image or --markdown`;
  `session.overlay.open: --image takes no --wait, --cwd, --navigation, --js, --chromeless, --persistent or --browse`
  and corresponding --markdown form; absolute paths/extensions refused with Swift's strings, including
  `session.overlay.open: --image must be a png, jpg, jpeg, gif or webp file` and
  `session.overlay.open: --markdown must be a .md or .markdown file`. CLI resolves relative paths, rejects
  --companion/--block/incompatible flags; companion API says `a page companion needs --html or --url`.
- [ ] Open one descriptor and probe regular files only; follow symlinks to target, reject FIFO/devices before any
  blocking read. Bound image file to 64 MiB/100 million pixels and sniff PNG/JPEG/GIF/WebP; reject disguised
  markup/SVG. Preserve ImageProbe.Failure strings/prefix. Handle Unix nonblocking IO and Windows regular handles;
  macOS dataless placeholder refusal has no literal Linux/Windows flag counterpart.
- [ ] Images embed data URIs with no bridge/file grant/theme/remote form. Natural sizing uses display scale,
  current zoom, EXIF 5–8 swapped dimensions, 240-point minimum width, base percent 95 and aspect-preserving
  fit within 80% detail bounds including strip chrome. Keep explicit size explicit.
- [ ] Markdown input is at most 4 MiB UTF-8. Embed raster images only within lexical and canonical parent-folder
  containment; canonical aliases share payload. Budget 32 MiB image bytes/100 million pixels/256 distinct probes
  including failures. Existing embedded aliases still resolve after budget exhaustion.
- [ ] Integrate upstream markdown-it/highlight assets, CSS, render.js and licenses with deterministic staging.
  Markdown follows terminal theme, no bridge/file grant/remote form. Authored markup stays inert, authored scripts
  disabled, no network fetches. Trusted renderer runs in an isolated helper context on Linux WPE, macOS WKWebView
  and Windows WebView2. Never solve helper execution by enabling arbitrary authored JS.
- [ ] Permit only http(s) external links through existing confirmation and #anchor scrolling; other links become
  text. Preserve CSP/escaping (including combining marks), highlighted code, tight/loose task boxes, Unicode
  heading slugs and linear duplicate suffixing. Render at most 256 image occurrences; set repeated image data
  through DOM instead of multiplying base64 markup.
- [ ] Reload rereads original source despite about:blank/fragments; theme change rerenders Markdown. Drop async
  loads overtaken by close/newer load. Markdown browser navigation says `a markdown page opens only in its overlay`;
  Finder error stays `show in Finder requires a file page` with per-OS reveal equivalents.
- [ ] Local path clicks open on owning session without selecting it, respecting HUD/zoom/busy slots and revealing
  on refusal. Bare paths treat %, #, ?, spaces literally; expand home, resolve relative paths against trustworthy
  local pane cwd; other local files reveal. Remote pane bare paths are ignored, remote scratch remains local,
  remote file:// targets reveal instead of opening local document overlays. Preserve authority/automount checks.
- [ ] Port all Swift loader/parser/renderer tests and backend invariants, with meaningful cross-platform Rust
  tests and reference UI cases. Keep existing overlay IDs green and run scoped checks. This is one feature cluster;
  image foundation then Markdown renderer are acceptable internal review seams if needed.

### Task 7: Carry remote overlay points and supply job-helper terminfo (#850/#854)

**Upstream:** `agtermCore/Sources/agtermCore/{AppStore+RemoteOverlay,AppStore+RemotePage,AppStore+RemotePresentation,AppStore+ReplicaPage,OverlayPointSize,PresentationFrames,PresentationHub,RemoteOverlays,TerminfoInstall}.swift`;
`agtermCore/Sources/agtermctlKit/OverlayRunJob.swift`; `agterm/Control/ControlServer+SessionActions.swift`;
contract remote sizing/job environment in `site/commands.html`, `.claude/rules/control-api.md`.
**Swift tests:** `AppStoreRemoteOverlayTests.swift::{pointSizesReachOnlyAPresenterThatTakesThem,aResizeIsResolvedAgainstTheSlotAndSentWhole,aRefusedRemoteResizeNamesItsError,anOlderPresenterGetsAPointResizeAsItsPercentAlone,pointsAloneCannotResizeAFullRemoteOverlay,aPointResizeAfterThePresenterChangedReportsTheViewerGone,aSlotShownOnThePresenterTakesNoPointSizes}`;
`AppStoreRemotePageTests.swift::{pointSizesReachOnlyAPresenterThatTakesThem,aPointResizeReachesThePresenterMergedWithTheSlot}`;
`AppStoreReplicaPageTests.swift::aHandedOverPageTakesTheOriginsPointSizesAndItsResizes`, remote presentation points
and frame roundtrips including `aSizedFrameFromAnOriginWithoutPointSizesDecodesWithNone`;
`agtermCore/Tests/agtermctlKitTests/OverlayRunJobTests.swift::theBundledTerminfoBacksAGhosttyTerminal` (five cases).
**Rust side:** modify `crates/ngterm-core/src/remote/{frames,hub,page,replica_page,viewer}.rs`,
`model/{remote_overlay,remote_presentation,overlay_points}.rs`, `commands/{overlay,overlay_page}.rs` and remote tests;
`crates/ngterm-ctl/src/overlay_job.rs` with existing `terminfo.rs::terminfo_directory`.
**Conformance:** listed `tests/protocol/test_live_sessions.py::{test_points_are_refused_for_an_overlay_a_presenter_shows,test_job_run_serves_an_overlay_a_presenter_shows,test_origin_commands_on_a_remote_page_reach_the_presenter}`;
listed `tests/cli/test_cli_output.py::test_live_run_job_runs_the_handed_over_program`;
new `test_remote_overlay_points_reach_capable_presenters`, `test_older_presenter_gets_percent_without_points`,
`test_remote_page_points_and_resizes_reach_presenter`, `test_live_run_job_uses_bundled_terminfo`.
Cover live.present (stream), session.overlay.open/resize/job.run, field:tree.session.remoteOverlays,
cli:session overlay run-job.

- [ ] Add optional point axes to overlay/page request, resize and presentation frames; absent decodes as none.
  Advertise `overlay.points`; origin stores axes and sends only to capable presenters, older ones receive percent
  alone and still open. Presenter installs/replaces points using its own bounds/local 80% sizing cap.
- [ ] Origin resize merges axes when points-only, replaces axes when percent+points, clears them when percent-only,
  clears everything on full. Full+points-only says
  `session.overlay.resize: --width/--height need a floating overlay; pass --size-percent too`.
  Stale presenter generation preserves state with viewer-gone error. Origin tree still exposes remote percent only.
- [ ] Replica-local point resize remains refused:
  `--width/--height are not supported for an overlay this Mac shows for another Mac; resize it on that Mac`;
  decide ngterm's platform wording and update adapter/string documentation together.
- [ ] Preserve existing ratchet ID `test_points_are_refused_for_an_overlay_a_presenter_shows` while adapting
  its assertions to the new contract (version-aware until parity claim, or retain ID testing replica-side refusal).
  Do not delete its ratchet line or leave obsolete origin-side refusal assertions.
- [ ] Overlay helper fills absent/empty TERMINFO from actual bundled database only for TERM=xterm-ghostty,
  before context overlays the base environment. Preserve nonempty value, other TERM and unrelated environment;
  missing database is not an error. Reuse lookup without creating directories, SSH or control requests.
- [ ] Port point/capability/backward-compatibility tests and all five terminfo cases; assert actual helper
  environment in conformance; run scoped checks on all applicable OS mechanisms.

### Task 8: Preserve managed attach output and test Live wait scrollback (#859, zmx patch behavior)

**Upstream:** `scripts/zmx-patches/{0004-managed-exit-keeps-screen.patch,README.md,check.py}`;
`agtermCore/Sources/SessionHostRuntime/Client.swift`, `agtermCore/Sources/agterm-session-host/main.swift`,
`agtermCore/Sources/agtermCore/ZmxSupport.swift`; `site/commands.html`, `.claude/rules/windows.md`.
**Swift tests:** `agtermUITests/ZmxLiveUITests.swift::{testAWaitCommandKeepsScrollbackOnceThePaneIsAttached,testAWaitCommandKeepsScrollbackPastOneScreenAfterExit}`;
`SessionHostClientTests.swift::{successAttachesWithTheGoneGuardInsteadOfThePayload,plainShellRequestKeepsTheBareAttach,returningPayloadKeepsTheBareAttach}`;
`SessionHostServerTests.swift::paneAttachAfterTheCreatedCommandEndedStartsNoShell`.
**Rust side:** modify `crates/ngterm-session/src/cli/attach.rs`, matching tests and `tests/cli.rs`;
`crates/ngterm-term/tests/live.rs`; inspect `src/session_io.rs` only if new regressions expose a problem.
**Conformance:** listed short-output hold and ended-command IDs from Context;
new `tests/protocol/test_live_sessions.py::test_a_wait_command_keeps_scrollback_after_attached_exit`,
covering session.new and session.text.

- [ ] CLI currently writes RESTORE including RIS unconditionally (`attach.rs:60,69`). Managed attach must preserve
  screen/scrollback while resetting SGR; mouse 9/1000/1002/1003/1006; focus 1004; paste 2004; color-scheme 2031;
  in-band resize 2048; OSC 110/111/112. Unmanaged clients retain RIS. Test valid managed nonce, invalid/absent
  nonce, detach and process exit, retained text and disabled input-report modes.
- [ ] Add real PTY/ConPTY regression with an attached marker and file gate, then 300 lines including first/last
  markers, exit, held prompt and full-buffer text assertions. Synchronize attachment before releasing the command;
  do not assert raw Unix byte timing on Windows. Port both short and past-one-screen Swift cases.
- [ ] Keep ngterm's single create+attach design. It has no upstream ensure-then-second-attach race, so do not add
  --ends-pane trampoline or copy upstream supervisor architecture. Verify no-create reconnect and returning
  restore/restart replay semantics; document the already-covered behavior. No native pin changes.
- [ ] Run scoped session/term tests and add newly green eligible IDs to ratchet.

### Task 9: Refresh bundled skill patches and provenance

**Upstream:** `plugins/agterm/skills/agterm/{SKILL.md,reference.md}` v0.39.1..v0.40.0; full skill tree at target.
**Swift tests:** no direct Swift tests for vendored prose; use the feature contracts/tests from Tasks 2–8 to
validate the command examples. Upstream skill documentation must not override actual code.
**Rust side:** `build/patches/skill-ngterm/*.patch`, regenerated `resources/skills/ngterm/`,
`crates/ngterm-app/src/integrations/mod.rs` release header; existing integration payload tests.
**Conformance:** existing `conformance/selftests/test_gen_catalog.py`, existing integration tests;
no invented feature coverage IDs for resource copying.

- [ ] Rebase 0001-live-group.patch and every later conflicting patch against v0.40.0. Retain ngterm/live names,
  NGTERM_LIVE_* environment, remote far-side ngtermctl, Linux/platform glue and local identity changes.
  Reconcile/remove redundant portions of 0010-live-creation-ends-pane.patch now upstream corrected its text.
- [ ] Regenerate whole skill via `.claude/skills/agterm-upstream-sync/scripts/sync-resources.sh`, never hand-edit
  only generated payloads. Record patches as unified diffs of upstream versus intended local copy. Verify
  `sync-resources.sh --check v0.40.0 v0.40.0` says unchanged for patched copies.
- [ ] Bump integration provenance only after successful regeneration. Confirm agent-status/defaults/LICENSE
  remain unchanged and Markdown resources from Task 6 are accounted for. Run scoped resource/integration checks.

### Task 10: Finish sidebar settings and desktop cursor/window-drag behavior

**Upstream:** #864/#865 `agtermCore/Sources/agtermCore/AppSettings.swift`,
`agterm/{SettingsModel,Ghostty/GhosttyApp}.swift`,
`agterm/Views/{SettingsView,SidebarInfoCell,SidebarRowViews,WorkspaceSidebar+RowRendering,WorkspaceSidebar,WindowAccessor}.swift`;
#858 `agterm/Views/{SplitCursorRects,SplitRatioAccessor}.swift`;
#869/#856 `agterm/Views/WindowControlArea.swift`; `.claude/rules/{sidebar,settings,libghostty}.md`.
**Swift tests:** `AppSettingsTests.swift::{sessionInfoDisplayDefaultsToAlwaysResolvesTolerantlyAndIsNotAGhosttyKey,rowIconElementsAreShownByDefaultAndHideIndependently}`;
all eight `agtermTests/SidebarInfoRowTests.swift` cases, all four new `SidebarRowViewsTests.swift` cases;
`agtermUITests/SettingsUITests.swift::{testInterfaceTabFitsWithTheOverlaysToggle,testSessionIconsToggleHidesOnlyTheSessionRowIcon}`;
`agtermTests/SplitCursorRectsTests.swift::{testASuppressedSplitRegistersNoDividerRect,testAnUnmanagedSplitKeepsItsDividerRect,testLiftingSuppressionRestoresTheDividerRect,testSuppressingOneSplitLeavesAnotherAlone}`;
offscreen/detach cases in `SplitRatioAccessorTests.swift`. Header drag change has no new Swift regression test.
**Rust side:** `crates/ngterm-core/src/{persist/settings,settings_model}.rs` and tests;
`crates/ngterm-gpui/src/sidebar/{context,rows,view,tests,look}.rs`, `settings/{interface,form,draw}.rs`,
`interface_glyph.rs`, `deck.rs`, `deck/tests.rs`, `split/tests.rs`, `window.rs`, `titlebar.rs`, `titlebar/tests.rs`.
**Conformance:** existing listed `tests/ui/test_sidebar_rows.py::test_rows_carry_the_names_the_tree_reads`,
`tests/ui/test_settings.py::test_interface_tab_toggles_title_bar_buttons`; new
`test_sidebar_rows.py::test_info_line_display_and_clear`,
`test_settings.py::{test_session_info_display_persists,test_row_icons_hide_independently}`;
cover ui:session-info, ui:settings-session-info-display, ui:settings-interface-{element}, ui:session-icon,
ui:workspace-icon. Cursor/header tests belong in GPUI/platform tests where driver cannot observe cursor/window manager.

- [ ] Persist optional raw sessionInfoDisplay, tolerant default always, values always/flagged/off and labels
  Always/Flagged/Off; no Ghostty key. Add Interface Sidebar picker, visible within the settings window.
- [ ] Render info below name, info size base minus 1.5 points (minimum 9), dynamic row height and accessibility
  session-info. Refresh on value/expiry/setting/rename completion. Preserve explicit colors on selected rows,
  selection foreground for uncolored spans, bold/italic/code/dim, subdued normal state and upstream emoji sizing
  (0.7 info size, VS16 versus VS15). Row tint does not affect info. Flagged mode means flagged sidebar view,
  not merely a flagged session in normal tree. Off changes layout only, keeping value/tree/timer.
- [ ] Add shown-by-default InterfaceElement workspaceIcons/sessionIcons in hiddenInterfaceElements, independent
  toggles for all normal/companion/flagged row forms. Reclaim icon width/gap, hide icon state cues; only remote
  connection notice moves to name tooltip and clears after reconnect. Port all settings/layout Swift cases.
- [ ] Hidden splits are GPUI content, not NSSplitView: Deck wraps inactive pages with invisible; vendored Div
  skips painting/event installation for hidden elements (`vendor/gpui-pre/src/elements/div.rs:2560`). Add a
  meaningful two-split-session/session-ask regression, showing hidden divider cannot supply resize cursor or
  consume input and visibility restores it. Fix only if exposed; do not copy AppKit class swizzling. Cover
  macOS/Linux/Windows through shared GPUI behavior.
- [ ] Header already calls start_window_move once per held press, tested by
  `titlebar/tests.rs::only_a_held_press_moves_the_window`, but `window.rs` leaves app_owns_titlebar_drag at its
  false default. Review/enable the existing WindowOptions mechanism for the custom header so macOS window
  server does not concurrently initiate native drag. GPUI macOS backend already implements
  `_opaqueRectForWindowMoveWhenInTitlebar`; use that supported option, no vendor edit unless evidence requires it.
  Linux uses compositor window move and Windows its existing window mechanism; SwiftUI contentShape is not
  a mechanism to copy. Add option/gesture regression and record the macOS display-edge drag check for VM/CI.
- [ ] Port reference UI cases, keep added controls visible, run headless GPUI tests and scoped checks; new
  since-gated IDs join the ratchet only after actually passing.

### Task 11: Claim v0.40.0 parity and update the docs

**Upstream:** target release's `site/commands.html`, `site/docs.html`, CHANGELOG and named Swift tests above.
**Rust side:** `crates/ngterm-proto/src/version.rs`, `crates/ngterm-core/src/dispatch.rs`,
`crates/ngterm-server/tests/server.rs`, `conformance/agterm_conformance/impls/rust.py`,
`conformance/docs/rust-build.md`, README.md, conformance/README.md; ratchet lists only from actual reports.
**Conformance:** every new ID in prior tasks, unchanged existing ratchet IDs, and both catalog checks.

- [ ] Only after all prior tasks pass, bump PARITY_LEVEL and every claim mirror to 0.40.0; use
  `tag-mentions.sh v0.39.1` to distinguish claim/reference/history. Swift reference already targets 0.40.0.
  Correct rust-build.md's temporary statement that claimed release equals catalog release.
- [ ] Describe messaging, info/expiry/settings, local document pages, remote group/points and exit behavior.
  Document new platform-adapted strings and reference vocabulary, including new live.attach.group refusals.
  Preserve minimum-version annotations, dependency versions and attribution history.
- [ ] Run the newly unlocked conformance cases and the existing ratchet; no skips counted as implemented.
  Recheck catalog tests, host-free selftests, coverage gate and appropriate build/clippy checks. Add newly
  passing IDs from actual nodeid reports to the correct OS ratchet, never remove old ones to hide failures.
- [ ] Manual (user): push branch, run reference-macos.yml against it to verify new tests on v0.40.0, then open
  PR for cross-platform jobs/full suite. Resolve any resulting verified mismatch before merge. Commits stay local
  during this plan; remote workflow execution and PR creation belong to the user.
