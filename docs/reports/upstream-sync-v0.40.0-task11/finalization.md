# v0.40.0 finalization

All 11 implementation tasks are checked and locally verified. Remote reference acceptance and native-platform
verification remain the explicit user handoff. The plan remains at `docs/plans/20261103-upstream-sync-v0.40.0.md`
for the orchestrator's completion step; the introduction describes the historical mechanical-bump baseline.

## Rebase and commit cleanup

- Fetched `origin` and rebased onto `origin/main` **163f318f**. The source before finalization was **5ee8459b**.
  Incoming main contains remote loopback-page forwarding, mobile sign-in, Windows titlebar/icon work and the
  shared-instructions rename from `CLAUDE.md` to `AGENTS.md`.
- Resolved one content conflict in `crates/ngterm-gpui/src/window.rs`: moved incoming
  `appears_transparent: cfg!(windows)` into `custom_header_options`, preserving both the incoming Windows
  titlebar behavior and this branch's `app_owns_titlebar_drag`. No other conflict or aborted rebase occurred.
- The branch contained **29 commits** before cleanup. Combined only consecutive Task 6 review fixes
  **46db0560 / 65c48641 / e98b4f0d** into **fdb505ef**, leaving **27 implementation/history commits**.
  Feature commits, task fixes, pre-existing fixes and the user-requested English skill trigger remain separate.
  The final report adds one documentation commit, for 28 branch commits before the orchestrator's plan move.
- Source head after cleanup: **fb180c60ec4083b71f8c899c34c95e03d039fc00**. The post-rebase and post-cleanup
  trees both equal **7eb23a46ca0b4d7684e0fbeeccb0aa43706c3164**: cleanup changed no file content.
  The rebase itself incorporates main's changes; it is not represented as an unchanged tree from 5ee8459b.
- **9718858a** survives as **5b03166d**, with `sync with upstream` preserved. The separately updated home
  Claude skill is outside this repository and was not altered by finalization. Baseline untracked
  `.claude/worktrees/` is preserved. No backlog file was removed.
- Existing `docs/reports/` artifacts are byte-identical to 5ee8459b. Their historical source hashes describe
  the executions that actually occurred and have not been rewritten to imply execution on rebased commits.

## Plan comparison and review outcomes

| Task | Delivered | Review disposition |
| --- | --- | --- |
| 1 | Wire/CLI contracts and substantive reference tests; narrow remote receipt exemption | Comprehensive findings fixed, final re-review clean |
| 2 | Global pane messages, lifecycle, limits, events and CLI behavior | Four minor findings fixed |
| 3 | Info validation/styling/expiry, lifecycle and remote presentation | Findings fixed; final re-review clean; separate link-metadata fix |
| 4 | Rebound duplicate session/pane identities and persisted command arguments | UUID-casing fixture flake fixed |
| 5 | Group attach, membership, partial receipts, placement and picker/CLI behavior | Completed page-error preservation fixed |
| 6 | Bounded image/Markdown documents, safe path routing and isolated rendering | Three normal rounds plus agreed after-decision fix/review; final round clean |
| 7 | Remote points/capability fallback and job-helper terminfo | Documentation findings fixed; old ratchet ID retained with updated assertions |
| 8 | Managed attach preserves screen/scrollback and retains unmanaged reset behavior | Cleanup docs/tail tests fixed; separate mobile fixture compile fix |
| 9 | Rebased skill patch chain, full regeneration and v0.40.0 provenance | Light final review clean |
| 10 | Info picker/rendering, independent icons, hidden split regressions and drag ownership | Tooltip/alignment findings fixed; native macOS observation remains handoff |
| 11 | Parity claim/docs, actual Linux ratchet additions and restart fixes | Minor identity recovery plus separate pre-existing serialization defect fixed |

All actionable findings were addressed; reviews that found only minor/documentation or pre-existing issues did
not trigger another review under Step 5R. Every completed review had its full configured sources; the progress
file records no failed or degraded review. The wording-only English skill side task was reviewed by inspection
and its executable review was skipped explicitly. Four immaterial minor notes are retained below.

## Verification boundaries

These are distinct source states, not interchangeable evidence:

1. **Before Task 11 review and rebase:** canonical Linux run passed **988**, failed **0**, skipped **0**,
   reruns **0**, deselected **8** in 1031.37 seconds: 806 protocol, 109 CLI, 19 behavior and 54 UI cases.
   The initial run was 986 passed / 2 failed / 0 skipped; both failures were repaired, with intermediate
   1 passed / 1 failed and then 2 passed / 0 failed preflights preserved. See [verification.md](verification.md).
2. **After Task 11 review fixes, before rebase:** rebuilt canonical runtime checks passed **3**, failed **0**,
   skipped **0**, reruns **0**, deselected **993** in 27.13 seconds. These cover shell replacement,
   foreground replay and shutdown-mail disposal. Core filters passed 55 live, 38 restart, 18 message and
   4 lifecycle tests; scopes overlap. Strict core clippy, formatting, locked metadata, lint and diff checks
   passed. This is the final pre-rebase source evidence; the full 988 cases were not rerun after review.
3. **After rebase and cleanup:** the merged-module checks are recorded below. Neither the full 988-case run
   nor native/reference acceptance is claimed for the rebased source.

Linux ratchet remains **988** (previous 932 plus 56 actually passed parameterized IDs from 39 selectors).
Windows remains **904**, byte-identical to both the task baseline and advanced origin/main. All **932 Linux**
and **904 Windows** IDs from advanced origin/main are retained; main brought no extra ratchet IDs. No old ID
was removed and no skipped ID was added.

Earlier Task 11 support checks passed: both catalog checks and version mirrors; full core **3452 passed,
0 failed, 2 existing ignored child entrypoints**; host-free selftests **1141 passed, 34 skipped, 0 failed**.
Those 34 skips are 16 opt-in Linux AT-SPI integration, 7 Windows named-pipe and 11 Windows launch cases.
The fresh docs/vocabulary checks passed 26 and coverage gate passed with zero uncovered keys and the existing
narrow successful-remote-operation exemptions. Full dependency-closure strict clippy and GPUI without HTML,
formatting, locked metadata, source lint and staging were checked at the historical states documented in
[host-free.md](host-free.md), not silently promoted to new final-head executions.

The merged Linux source at **fb180c60** passed the following checks (each command had a 600-second limit,
with inherited NGTERM_/AGTERM_ variables removed):

| Post-rebase check | Result |
| --- | --- |
| `cargo build --locked -p ngterm-gpui -p ngterm-mobile -p ngterm-session` | Passed; 75.43 seconds |
| `cargo test --locked -p ngterm-core --lib replica_page` | 48 passed, 0 failed, 0 ignored |
| `cargo test --locked -p ngterm-core --lib page_tunnel` | 7 passed, 0 failed, 0 ignored |
| `cargo test --locked -p ngterm-mobile --lib transport::russh::` | 38 passed, 0 failed, 0 ignored |
| `cargo test --locked -p ngterm-gpui --lib window::tests::` | 45 passed, 0 failed, 0 ignored |
| `cargo test --locked -p ngterm-gpui --lib titlebar::tests::` | 16 passed, 0 failed, 0 ignored |
| `cargo clippy --locked -p ngterm-core -p ngterm-mobile -p ngterm-gpui --lib --tests -- -D warnings` | Passed; 44.74 seconds |
| `cargo fmt --all --check`; `git diff --check` | Passed |

The GPUI test commands ran under `GDK_BACKEND=x11 xvfb-run -a dbus-run-session`. The **154** passing scoped
tests cover the merged remote-page port/point behavior, local scripted mobile SSH/sign-in flows, window
options and titlebar behavior. They do not exercise native Windows/macOS or Android packaging. Commands,
durations and individual logs are retained under
`/home/nick/.local/state/plan-exec-revmux/20261103-upstream-sync-v0.40.0/finalize-*`.
No source edit followed these checks; this report is the only added file. All finalizer command sessions ended.


## Obstacles and recovery

- Task 3's original agent exhausted model capacity just before commit. A fresh agent preserved its completed
  edits and checks, reran the focused info cases, checked all seven items and committed the result.
- Task 6's first decision agent and after-decision fixer were stopped by content-safety filters. Replacement
  briefs used read-only defensive API analysis and modest valid-fixture/resource-budget tests. The independent
  Codex discussion agreed to a bounded decoder fix plus one final review; that review was clean. The rejected
  initial decision artifacts remain under `discussions/decision-01-webp`; the successful transcript path is quoted
  below. No blocked-agent source edits were claimed.
- Task 7's coordinator was stopped by a content-safety filter. A fresh coordinator retained completed sizing
  edits and the two running helpers, integrated their results, and completed checks and the commit.
- The after-decision WebP fix intentionally omitted two existing tests that allocate tens of MiB and replaced
  the previous malformed-metadata allocation reproduction with modest normal fixtures. Ten selected document
  tests, the direct/Markdown paths and static/animated orientation/budget cases passed. Earlier document checks
  and the final review remain evidence; no full app-module rerun after that constrained fix is claimed.
- Task 11's initial runtime failures exposed a synchronous restart deadlock and a Linux accessibility assertion
  that ignored the existing capability profile. Off-main bounded kill/Pending handling and capability-aware real
  UI selection repaired them. Successful delivery then exposed a hidden dict-versus-Response test decoder error,
  repaired without weakening the success assertion. Runtime/nodeid artifacts preserve every attempt.
- An intermediate generated documentation capability-table mismatch and initial new-test compile/lint errors
  were corrected before their recorded final green checks. The Task 3 textual review-count error was corrected
  in the log to 1 actionable major, 3 actionable minor and 1 major immaterial finding.
- Post-review runtime preflight initially lacked Python `gi`; the documented system AT-SPI PYTHONPATH corrected
  it before the successful app launch. The finalization rebase conflict was resolved as described above.

## Pre-existing defects fixed

- Session-info attribute preprocessing transformed link metadata. The separate **810b8452** correction leaves
  link destinations/titles/autolinks literal while styling visible labels; final rebased commit **3dd11695**.
  The log classifies this as pre-existing/immaterial relative to the reviewed scope.
- Mobile `PresentationSnapshot` test fixture omitted `info`, causing E0063 in scoped tests. Separate fixture-only
  **ce328950** adds `info: None`, with no runtime change; final rebased commit **92e925c9**.
- An overlapping restart could release another restart's exit claim after replacement installation. Separate
  **5ee8459b** carries a scoped whole-restart permit through completion, failure, timeout and callback drop;
  final rebased commit **fb180c60**. Deterministic interleaving and cleanup regressions pass. The adjacent
  failed-kill identity/lead recovery is an own-task fix, kept separate (**ef023f23** -> **820f8426**).

The synchronous restart deadlock found by Task 11's newly unlocked test was also repaired, with its ordering,
claim ownership, rollback and dropped-callback behavior documented in [restart-fix.md](restart-fix.md).

## Open questions, retained minor notes and handoff

No `[backlog]` line was logged. No review open question or unresolved gating defect remains in the local run.
No implementation checkbox is left unchecked; the manual checkbox explicitly records an unexecuted user handoff.
These four immaterial minor notes remain, with their practical limits and possible follow-up:

- `commands/overlay.rs`: new overlay error strings are inline instead of the file's named-constant convention.
  This has no reported runtime impact; extracting constants would align local style.
- `html_overlay.rs`: no-op `set_window_scale` sits outside the unavailable-backend fallback module. Moving it
  beside the other stubs would improve organization; no broken fallback behavior was reported.
- `html/mod.rs`: lexical path normalization has two new copies of an existing helper. A shared helper could
  remove duplication; no differing behavior was identified in the review.
- `sidebar/info.rs`: styled fragments follow source order, so mixed-style RTL text can display reversed.
  A paragraph-level bidi layout across style runs would address the limitation. This was classified immaterial
  and non-gating in Task 10 and remains unfixed, rather than being described as verified parity for that case.

The report does not turn cross-platform source work into native runtime verification:

- User handoff: push the branch, dispatch `reference-macos.yml` for v0.40.0, then open the PR and run the
  cross-platform/full-suite checks; resolve any verified mismatch before merge. **No push, PR or remote
  workflow was performed**, and no successful external SSH group attach was attempted.
- Native WKWebView/WebView2 execution, Windows-specific path guards and Windows ConPTY behavior remain native
  runner checks. The Windows WebView2 helper was source/API type-checked during Task 6; that is not a complete
  native Windows application build or runtime result. Incoming Windows titlebar transparency is preserved,
  while its native behavior remains a platform check.
- Native macOS display-edge window dragging remains unobserved; shared headless option/held-press regressions
  are the available evidence. WPE renderer tests on Linux and scoped fake-runner group receipt tests remain
  valid evidence within their documented scope.
- The live application was never contacted. Checks stripped inherited NGTERM_/AGTERM_ variables; GUI checks
  used private headless display/session-bus instances and runtime conformance used private state/sockets.
  Finalization launched no built ngterm executable or control client. Module tests used in-process fakes,
  local scripted SSH fixtures and private headless test windows, without contacting an external SSH host.

## Complete decision and deviation record

The entire progress log and plan were read. Every `[decision]` and `[deviation]` line through finalization is
quoted verbatim below, including timestamp, reason and any recorded transcript path. These are historical
entries: original hashes/paths/counts have not been rewritten. Where no discussion transcript appears, the log
records a repository rule, upstream intent, direct evidence or authorized mechanical choice; no transcript is
invented. The one successful independent discussion transcript was also read and agrees with the bounded-reader
exception, including the limit that it establishes header/EXIF behavior rather than a pixel-decoding guarantee.

```text
[2026-11-03 19:34:55] [decision] task 1: exempt field:result.remoteGroup only — a successful group receipt requires SSH parent attachment; existing suite has no network-free successful attach seam; Task 5 supplies fake-runner receipt behavior tests
[2026-11-03 19:34:55] [deviation] task 1: new CLI contracts use tests/cli/test_cli_messages.py rather than proposed test_messages.py — pytest current import mode rejects duplicate basenames across CLI and protocol directories
[2026-11-03 19:34:55] [decision] task 1: change existing move-rejection case table from stack array to Vec — added Args fields crossed clippy large_stack_arrays threshold; no behavior changed
[2026-11-03 19:58:31] [decision] task 2: share the in-memory message bus between WindowLibrary and AppStore using the existing Open Recent Rc<RefCell> runtime pattern — keeps standalone stores testable, cross-window addressing global and mailbox disposal synchronous without persistence.
[2026-11-03 20:16:38] [decision] orchestrator: user authorized up to four agents; use independent read-only preparation and disjoint within-task delegation while keeping task implementation/reviews ordered.
[2026-11-03 20:42:32] [deviation] task 3: first agent failed at model capacity before checkbox update and commit; fresh retry agent retained validated edits and is completing task.
[2026-11-03 20:43:04] [decision] task 3: reuse the existing pulldown-cmark dependency with inline-only wrappers and Foundation color/dim attribute translation — avoids a new dependency while preserving toolkit-neutral styled runs and upstream validation.
[2026-11-03 20:43:04] [deviation] task 3: a fresh retry finalized the task after the original agent hit model capacity just before commit — preserved completed edits and successful checks, reran the focused info tests, then marked and committed all seven items.
[2026-11-03 21:21:39] [decision] task 5: share one deferred group run between command and picker, reusing page handover and placement seams — the existing asynchronous flow pattern keeps control requests responsive while explicit Response::failure_with_result preserves partial receipts.
[2026-11-03 21:28:13] [decision] fixer task 5: preserve completed page-specific errors and use parent row was closed only for a member not started — upstream attachMember explicitly distinguishes these cases, settling the plan generic wording through its exact-upstream-parity intent; no discussion needed.
[2026-11-03 21:48:16] [decision] task 6: keep document builders/types in core and bounded probes in ngterm-app behind HtmlHost; control checks stay synchronous as upstream openImageFile specifies, while click/renderer reads run off-main with generation and eligibility rechecks — respects existing app->core dependency and upstream semantics.
[2026-11-03 21:48:16] [decision] task 6: run trusted Markdown renderer in WPE named UserScript world, WKUserScript named WKContentWorld, and WebView2 CDP worldName before loads — supported native APIs preserve authored-JavaScript disablement without granting bridge or file access.
[2026-11-03 22:23:29] [deviation] task 6: decision agent was blocked by a content-safety filter while inspecting malformed-image example; fresh decision agent uses defensive bounded-reader API/design discussion without reproducing example. Initial artifacts discussions/decision-01-webp; no source edits from decision agent.
[2026-11-03 22:26:35] [decision] orchestrator: task 6 review cap with nested WebP metadata allocation bound gap → one guided fixer using direct image-webp decoder metadata limit before EXIF, then exactly one 04-after-decision final review; stop dependent tasks if confirmed gating remains — codex: agreed, transcript /home/nick/.local/state/plan-exec-revmux/20261103-upstream-sync-v0.40.0/discussions/decision-01-bounded-reads/transcript.md
[2026-11-03 22:26:35] [deviation] task 6: review not converging — documents.rs WebP metadata range validation → guided direct-decoder fix plus one after-decision final round; independent peer agreed finite metadata limit avoids duplicating nested dependency parsing.
[2026-11-03 22:27:16] [deviation] task 6: after-decision fixer brief containing full malformed-input report was blocked by content-safety filter; replacement scope limited to agreed defensive decoder-memory-limit change and modest tests, without malformed-input reproduction.
[2026-11-03 22:39:21] [deviation] task 7: kept ratcheted test_points_are_refused_for_an_overlay_a_presenter_shows nodeid and replaced obsolete origin refusals with meaningful older-presenter percent-only downgrade assertions; replica-local refusal is covered in core tests because creating a replica requires successful SSH outside network-free conformance seams. No ratchet removed or skipped.
[2026-11-03 22:40:26] [deviation] task 7: coordinator stopped by content-safety filter; fresh retry coordinates preserved sizing edits and two running terminfo/conformance helpers.
[2026-11-03 22:45:10] [decision] task 7: use computer rather than Mac in replica-local point-resize refusal, mapping the complete upstream sentence in Swift vocabulary and documenting it beside the target contract — ngterm supports macOS/Linux/Windows and the plan requires aligned adapter wording.
[2026-11-03 23:00:50] [decision] fixer task 8: correct the discovered pre-existing PresentationSnapshot test-build defect in a separate fixture-only commit — the minimal info: None addition is required to compile scoped mobile tests; parent confirmed scope, no runtime behavior changes.
[2026-11-03 23:05:14] [decision] task 9: retained reduced 0010 for ngterm's single create-and-attach output preservation and returning replay; removed obsolete correction of upstream pane-exit semantics — v0.40.0 corrected the latter but newly documents an upstream ensure/attach race that ngterm's implementation does not have
[2026-11-03 23:05:14] [decision] task 9: added 0016-remote-name-mirroring.patch to preserve existing live name-mirroring prose — comparison of the old patch chain with committed resources found the earlier local feature was documented only in generated payloads; encoding it makes whole-tree regeneration reproducible without losing local behavior
[2026-11-03 23:05:14] [decision] task 9: kept Markdown resources under Task 6's separate sync-markdown-resources.py generator and references/areas.md mapping — sync-resources.sh's old/new report still labels the upstream directory unmapped, but the dedicated byte check passed and no duplicate copying or renderer changes are needed
[2026-11-04 10:46:34] [decision] task 10: Use a small GPUI canvas with shaped consecutive text/emoji runs for info typography — StyledText cannot give individual runs upstream emoji point sizes and baseline offsets; Unicode properties and segmentation preserve emoji presentation/VS16/VS15 behavior. Reuse the existing GPUI-pinned unicode-properties =0.1.3, verified in the live registry, rather than incompatible latest 0.1.5.
[2026-11-04 10:46:34] [decision] task 10: Compact only the Interface page vertical padding/section gaps — the meaningful bounds test exposed Size clipped four points below the viewport after adding the controls; the upstream rule requires no scrolling and all other pages retain their layout.
[2026-11-04 10:46:34] [decision] task 10: Keep existing new conformance IDs rather than duplicate the plan pseudonyms, and leave the ratchet unchanged — new since-0.40 cases correctly skip under the current parity profile until Task 11; only actual green IDs may be added.
[2026-11-04 10:46:34] [decision] task 10: Retain existing hidden split behavior and use the supported app_owns_titlebar_drag option without vendor changes — actual hidden-page/session-ask input regressions exposed no split bug and macOS already implements the corresponding native hook.
[2026-11-04 10:46:34] [deviation] task 10: macOS display-edge drag observation skipped here and recorded in the plan — this Linux/headless execution cannot observe the native macOS window server; option and held-press regressions pass, VM/CI observation remains required.
[2026-11-04 10:57:16] [decision] task 11: canonical ratchet-only run includes all existing Linux ratchet IDs plus 39 new feature-test selectors derived from definitions added since mechanical catalog bump; nodeid outcomes will be committed under docs/reports/upstream-sync-v0.40.0-task11. Old minimum versions, dependency versions and attribution history remain unchanged; gh release tag v0.40.0 verified live (published 2026-11-02).
[2026-11-04 11:01:32] [decision] task 11: added explicit Swift vocabulary translations for ordinary attach remote-session and both group-attach required-field refusals; upstream ControlDispatcher+Zmx.swift formats these from the command raw value, and existing vocabulary selftests validate the final mappings. Old-tag audit reports no remaining claim/reference/resource0.39.1 mirrors; review-classed occurrences are minimum since markers, dependency/API versions or source attribution and remain intact.
[2026-11-04 11:12:48] [decision] task 11: full eligible runtime exposed test_restart_drops_mail_sent_during_shutdown failing before mailbox read. Two independent source audits verified live restart blocks core/UI dispatch in synchronous SessionHost::kill while the old program HUP handler waits on CLI msg.send; ngterm-session then force-kills it after250ms. Repair through existing Dispatched::Pending worker/channel pattern, preserving upstream shutdown-message semantics and mailbox-disposal order; add deterministic blocked-host regression/failure cleanup. Test handler also ignores repeated HUP because end_tree sends both process-group and session-member hangups; assertions stay unchanged. First full run continues, and a fresh full eligible run is required after runtime repair.
[2026-11-04 11:20:45] [decision] task 11: initial complete eligible run finished986passed/2failed/0skipped, all932oldratchet IDs hold. UI session-info picker failure was an unconditional string-value assertion despite documented Linux ui-string-values capability absence (rust-build.md; existing sidebar/titlebar tests branch). Use that existing capability branch for popup value predicates and assert initial/default plus Flagged/Off/Always settings persistence through actual UI selection; no feature assertions skipped. Initial actual nodeid report, runner report and trimmed failure tracebacks preserved in docs/reports/upstream-sync-v0.40.0-task11.
[2026-11-04 11:23:35] [decision] task 11: repaired-stage preflight now produces successful shutdown CLI delivery, proving main-loop deadlock repaired; it exposed a second test-only error previously hidden by empty receipt: assert_ok expects client.Response, while json.loads returned dict. Decode the raw receipt with existing client.decode_response before the unchanged success/delivered assertion. Settings picker preflight passed with existing capability branch. Preserve preflight actual nodeids/report; no expectation weakened.
[2026-11-04 11:25:57] [decision] task 11: asynchronous restart guard follows existing5s/100ms shell wait for overlapping same-pane restart, preserving timeout text and claim ownership. Pending-kill-only map records suppressed old-attachment exits and replays one on failed/stale kill after releasing claim; success discards it before normal replacement. These preserve old synchronous callback ordering and do not change other claimed-exit lifetimes. Deterministic channel tests cover delivery/empty replacement, lead reports, overlapping restarts, failure/soft-close/deferred exit. Full core3452passed/2existingchild-entrypointignored, relevantmodules79passed; final strict10crateclosure and GPUI no-default pass. Report docs/reports/upstream-sync-v0.40.0-task11/restart-fix.md.
[2026-11-04 12:00:11] [decision] fixer Task11 r1: keep exit claims independent from whole-restart serialization; carry a private RAII permit through every Pending continuation rather than manually clear a raw marker on Done paths — frontend installation legitimately releases the old exit claim before the restart reply, while dropped/unwinding callbacks require automatic marker cleanup. Existing main-loop sequential ownership and mutex poison recovery patterns settle the narrow mechanism; no Codex discussion needed. Separate read-only interleaving/cleanup audit confirmed all error/timeout/drop paths and no mutex nesting.
[2026-11-04 12:03:42] [decision] finalize: safely resolve the sole rebase conflict in crates/ngterm-gpui/src/window.rs by moving origin/main Windows appears_transparent: cfg!(windows) into the extracted custom_header_options helper alongside app_owns_titlebar_drag — preserves both incoming native titlebar behavior and the ported drag ownership; upstream 163f318f also renames CLAUDE.md to AGENTS.md, whose unchanged check/live-app rules were read. No external discussion required for this mechanical composition.
[2026-11-04 12:03:42] [decision] finalize: consolidate only the consecutive Task6 review fixes 46db0560/65c48641/e98b4f0d into one commit fdb505ef after rebasing; keep feature/task/pre-existing/user-change boundaries and every backlog/report file — their final combined document-safety/link-routing change is coherent; other fixes have meaningful independent boundaries. Cleanup changes 29 branch commits to 27 and preserves the post-rebase tree exactly (7eb23a46ca0b4d7684e0fbeeccb0aa43706c3164). Historical evidence hashes remain unchanged in reports.
```
