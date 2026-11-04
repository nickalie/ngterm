# v0.40.0 final parity validation

Task 11 completed on Linux on 2026-11-04. Rust now claims upstream **0.40.0** in
`PARITY_LEVEL`, its serialized/version assertions, the Rust adapter example and
the documentation. The Swift reference already targeted 0.40.0. GitHub's live
release endpoint confirmed tag v0.40.0, published 2026-11-02 at 18:28:31 UTC.

## Runtime gate and ratchets

The canonical headless runner exercised every existing Linux ratchet ID plus all
39 newly added feature selectors from Tasks 1-10, expanding to 56 new parameterized
IDs. Every application used private state, sockets, displays and session buses;
inherited NGTERM_/AGTERM_ variables were removed before launching the runner.
The repository venv used system AT-SPI bindings through explicit PYTHONPATH.

| Run | Passed | Failed | Skipped | Reruns | Duration |
| --- | ---: | ---: | ---: | ---: | --- |
| Initial entire eligible ratchet + new cases | 986 | 2 | 0 | 0 | 1084.38 s |
| Rebuilt repair preflight | 1 | 1 | 0 | 0 | 34.44 s |
| Final focused preflight | 2 | 0 | 0 | 0 | 34.03 s |
| **Final entire eligible ratchet + new cases** | **988** | **0** | **0** | **0** | **1031.37 s** |

The final run passed 806 protocol, 109 CLI, 19 behavior and 54 UI cases. Its
selection was `ratchet-only`, with the entire previous ratchet and all new
selectors, not a filtered substitute. Eight other collected tests were deselected
by that selection; this is not a claim that the unrestricted suite or another
platform ran. One existing AT-SPI deprecation warning remained.

`check_ratchet.py --add` consumed [final-nodeids.jsonl](final-nodeids.jsonl),
adding only its 56 actual passing IDs: **Linux 932 -> 988**. All 932 old IDs remain.
The **Windows ratchet remains 904**, unchanged; Linux outcomes were not copied to
it. [final-runtime.md](final-runtime.md) is the original runner report before the
add operation. Actual reports from all earlier attempts are retained alongside it.

## Verified failures repaired

The shutdown message case exposed a synchronous Live-session kill that blocked
the main loop, preventing an ending program's CLI request from completing. The
[restart fix report](restart-fix.md) records the worker/Pending implementation,
lead and overlapping-restart guards, deferred exit replay on kill failure, and
deterministic tests. The HUP fixture ignores duplicate teardown signals. Once
real delivery worked, its previously hidden raw-dict/Response matcher error was
fixed by using the harness's existing response decoder; delivery and empty-new-
mailbox assertions are preserved.

The session-info picker test assumed string accessibility values on Linux,
contrary to the existing `ui-string-values` capability/profile. It now uses the
same capability branch as the sidebar/titlebar tests while exercising the real
popup and asserting default, Flagged, Off and restored Always persistence. The
reference retains its string-value predicates. No feature test was skipped or
removed to pass the gate.

## Other validation

- Both catalog checks: 4 protocol catalog tests and 1 CLI catalog test passed;
  both exact parity version mirror tests passed as well (7 total).
- Full core library: **3452 passed, 0 failed, 2 existing ignored child entrypoints**.
  The directly relevant live/message/restart modules contain 79 passing cases.
- All host-free selftests: **1141 passed, 34 platform/opt-in skips**, no failures.
  Fresh final docs/vocabulary checks: 26 passed. Coverage gate: 1 passed, zero
  uncovered entries, with existing narrow remote-success exemptions preserved.
- Strict clippy passed for the 10-crate dependency closure and GPUI without
  default features. Global fmt, locked metadata, source lint and diff checks pass.
  [host-free.md](host-free.md) records commands, counts and skip reasons.
- The canonical runner rebuilt and staged the final source before the successful
  preflight. The full final run used that same immutable stage; no application or
  runtime-test source changed during or after it. The build's recorded HEAD is the
  task-start commit with the task changes dirty, as expected for validation before this task's commit.

## Version classification and documentation

[tag-mentions.sh output](old-tag-audit.txt) has no remaining old claim, reference
or resource mirror. Review-classified old strings are WebView2 dependency/API
versions, minimum `since("0.39.1")` annotations, or historical/source attribution;
they remain intact, as do historical test fixtures and plan baseline notes.
The catalog and implementation claim are documented as independent values.

README and conformance documentation cover messages, info/expiry/settings and
independent row icons, local image/Markdown pages, remote group/point behavior,
managed attach exit and adapted vocabulary including group-attach refusals.
Bundled skill provenance was already 0.40.0 after Task 9; this task changes no
resource payload, native pin, dependency or Cargo.lock.

## Manual user handoff and limits

**Not run:** push, reference-macos.yml, PR creation and cross-platform CI. The
plan assigns these remote actions to the user; its manual checkbox is marked as
a handoff, not a verified result. Reference acceptance and any resulting verified
mismatch must be resolved before merge. The plan remains in its original path
for the orchestrator to review/finalize.

Native WKWebView/WebView2 runtime behavior, macOS display-edge window drag and
Windows ConPTY remain platform-runner checks. Successful external SSH group
attachment was not attempted; its narrow catalog exemption and core fake-runner
receipt tests remain the evidence. No platform runtime or skipped case is counted
as a passing ratchet addition. The user's live ngterm was never contacted.

## Post-review restart verification

The 988-case full pass above preceded the review fixes. Review confirmed two restart defects: failed asynchronous
kills lost lead restoration and recorded exits after a swap or soft close; a pre-existing overlapping-restart race
allowed A's final shell poll to release B's exit claim. The first fix restores by pane identity and replays exits by
their current token location. The second holds a scoped serialization permit through the final restart reply,
including error, timeout and dropped-callback cleanup. They land in separate commits; [restart-fix.md](restart-fix.md)
describes the deterministic regressions and cleanup audit.

After both fixes, targeted core checks passed: 55 live-module tests, 38 tests matching `restart` (overlapping scope),
18 message-module tests and 4 lifecycle tests, each with zero failures or ignores. Strict `cargo clippy -p ngterm-core --all-targets
--all-features -- -D warnings`, global formatting, locked metadata, source lint report and diff checks passed.

The canonical runner rebuilt and staged the final source, then reran shell replacement, foreground replay and
shutdown-message disposal: **3 passed, 0 failed, 0 skipped, 0 reruns, 993 deselected**, in 27.13 seconds. The partial
ratchet and private-instance cleanup checks passed; no ratchet entry changed. The launch stripped inherited
NGTERM_/AGTERM_ variables and used private state, explicit instance sockets and headless displays. Its first
preflight stopped before building or launching because the venv lacked `gi`; using the documented system AT-SPI
Python path resolved it. [review-r1-runtime.md](review-r1-runtime.md) and [review-r1-nodeids.jsonl](review-r1-nodeids.jsonl)
record the successful run. Its recorded HEAD is the minor-fix commit with the major fix dirty before commit.
The full 988-case suite was not rerun after review; the final-head evidence is the affected checks listed here.
