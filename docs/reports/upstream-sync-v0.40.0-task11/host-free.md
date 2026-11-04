# Task 11 host-free and scoped Rust validation

Checked on Linux on 2026-11-04 with the repository's Python 3.14 venv and
`PYTHONPATH=$PWD/conformance:/usr/lib/python3.14/site-packages`. The system path
provides the AT-SPI bindings; the repository harness stays first. The pytest
plugin reported stripping inherited `NGTERM_*` variables.

| Check | Result |
| --- | --- |
| `python -m pytest conformance/selftests -ra` | 1141 passed, 34 skipped, no failures or errors; 188.34 s |
| Fresh `python -m pytest conformance/selftests/test_docs.py conformance/selftests/test_vocabulary.py -ra` after the documentation and Swift vocabulary edits | 26 passed, no skips; 0.65 s |
| `python -m pytest conformance --coverage-gate` | 1 passed; no uncovered catalog keys; 0.67 s |
| `scripts/lint-report.sh` | Passed |

The 34 host-free skips are expected platform/opt-in probes: 16 real Linux UI
integration probes, seven Windows named-pipe transport cases, and 11 Windows
launch cases. They are not counted as runtime conformance passes or ratchet
additions. The task's canonical isolated runtime run is reported separately.

Catalog coverage is 128/131 commands (three exempted), 142/147 fields (five
exempted), 11/13 events (two exempted), 132/132 CLI leaves, 20/24 behaviors (four
exempted), and 125/129 UI keys (four exempted). There are no baselined or uncovered
entries in any category. Existing narrow remote-success exemptions remain in
place; this check does not perform successful SSH attachment.

The scoped Rust checks use the same checkout and `timeout 600` for each command:

| Check | Result |
| --- | --- |
| `cargo test -p ngterm-proto --test catalog` | 4 passed |
| `cargo test -p ngterm-ctl the_command_tree_is_the_catalogs_cli` | 1 passed; 534 unrelated cases filtered out across targets |
| `cargo test -p ngterm-core --lib dispatch::tests::version_reports_app_and_profile` | 1 passed; 3448 unrelated cases filtered out |
| `cargo test -p ngterm-server --test server version_round_trips_through_the_main_loop` | 1 passed; 21 unrelated cases filtered out |
| `cargo clippy -p ngterm-proto -p ngterm-core -p ngterm-server --all-targets --all-features -- -D warnings` | Passed; 22.09 s |

The last two assertions check the exact `0.40.0` parity claim in the dispatcher
response and its private control-socket round trip.

The new shutdown-mail conformance fixture now ignores additional HUP signals
after entering its handler. `Pane::end_tree` first calls `Killer::hang_up` (the
PTY's process group) and then `signal_session(HangUp)` (each session member), so
the same trapped program can receive a second HUP. Ignoring that second signal
keeps its one CLI send deterministic. The readiness check, successful delivery
receipt assertion, and empty replacement-mailbox assertion remain unchanged.
`python -m compileall -q conformance/tests/protocol/test_messages.py` passed.
After this fixture edit, the catalog coverage gate passed again (1 passed,
0.68 s), with the same zero-uncovered counts above. Runtime validation belongs
to the canonical isolated runner's report, not this host-free check.

After the runtime restart fix, its overlapping-restart guard, and pending-kill
exit cleanup were final, strict clippy passed for the complete workspace
dependency closure of `ngterm-core` (12.22 s):

```sh
cargo clippy -p ngterm-proto -p ngterm-core -p ngterm-server -p ngterm-app \
  -p ngterm-ctl -p ngterm-gpui -p ngterm-headless -p ngterm-mobile \
  -p ngterm-session -p ngterm-term --all-targets --all-features -- -D warnings
```

The separate frontend configuration also passed (10.47 s):

```sh
cargo clippy -p ngterm-gpui --no-default-features --all-targets -- -D warnings
```

These are Linux build/lint results. Native macOS and Windows runtime checks
remain the platform-runner handoff; no platform runtime result is inferred
from this dependency-closure check.

After the final response-decoding and existing `ui-string-values` capability
branch repairs to the new runtime tests, the focused docs/vocabulary selftests
passed again: **26 passed, 0 skipped**, 0.18 s. The coverage gate passed again:
**1 passed**, 0.64 s, with the same zero-uncovered counts. An intermediate
documentation selftest caught capability-table prose drifting from its canonical
vocabulary description; the generated table wording was restored and the popup
explanation retained in the Linux profile prose.

Final `cargo fmt --all --check`, `cargo metadata --locked --format-version 1`,
`scripts/lint-report.sh` and `git diff --check` passed. No new dependency, lockfile,
native pin, renderer payload or bundled skill payload was changed by Task 11.
