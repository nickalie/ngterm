#!/usr/bin/env bash
# The unit-test coverage of the Rust workspace: what `cargo test` covers, beside the conformance suite's coverage
# (run-linux.sh --coverage), which measures only what the suite drives through the staged app.
#
# One instrumented `cargo llvm-cov` run of the workspace's tests (default features, `--no-fail-fast`), every crate but
# the generated -sys crates, so each crate is credited with every test that runs its code, its own and its
# dependents'. Then the report, with test code (tests/, tests.rs, *_tests.rs, test_support), vendor/ and the -sys
# crates out of the denominator:
#
# - OUT_DIR/lcov.info, the per-line report;
# - OUT_DIR/summary.json, llvm-cov's per-file summary;
# - OUT_DIR/summary.md, the line, region and function % per workspace crate and their total, in the table
#   conformance/agterm_conformance/code_coverage.py writes (its own code renders it), which
#   conformance/tools/check_coverage.py holds against scripts/unit-coverage-floor-linux.txt.
#
# A test child killed mid-run (a timeout, a crash test) leaves a truncated profile that fails the whole merge: the run's
# profiles that `llvm-profdata show` cannot read are dropped before the report. Doc tests are not measured
# (cargo-llvm-cov runs them only on nightly).
#
# The default features need WPE WebKit (README > WPE WebKit) and the app's tests a display and a session bus: run it as
# README > Test runs `cargo test` (under xvfb-run and dbus-run-session). Needs cargo-llvm-cov and the toolchain's
# `llvm-tools` component, python3 for the summary.
#
# Usage: scripts/unit-coverage.sh OUT_DIR
# Exit 0: tests passed and the report is written; 1: a test failed (the report is still written); 2: usage, or no
# report could be made.
set -euo pipefail

# shellcheck source=lib/native.sh
source "$(dirname "$0")/lib/native.sh"

if (($# != 1)); then
  echo "usage: scripts/unit-coverage.sh OUT_DIR" >&2
  exit 2
fi
mkdir -p "$1"
out="$(cd "$1" && pwd)"
cd "$NATIVE_ROOT"

# Test code, patched dependencies and generated bindings: not what the floors measure.
ignore='(/tests/|/tests\.rs$|_tests\.rs$|/test_support|/vendor/|-sys/)'
excludes=()
for crate in crates/*-sys; do
  excludes+=(--exclude "${crate#crates/}")
done

# cargo-llvm-cov's profile directory (its default, under the target directory, unless CARGO_LLVM_COV_TARGET_DIR moves
# it); profiles of an earlier run there would count again
target="$(cargo metadata --format-version 1 --no-deps |
  python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')"
profiles="${CARGO_LLVM_COV_TARGET_DIR:-$target/llvm-cov-target}"
if [[ -d "$profiles" ]]; then
  find "$profiles" -maxdepth 1 -name '*.profraw' -delete
fi

status=0
cargo llvm-cov --no-report --workspace "${excludes[@]}" --no-fail-fast || status=1

profdata="${LLVM_PROFDATA:-$(dirname "$(rustc --print target-libdir)")/bin/llvm-profdata}"
dropped=0
for profile in "$profiles"/*.profraw; do
  [[ -e "$profile" ]] || continue
  if ! "$profdata" show "$profile" >/dev/null 2>&1; then
    echo "unit-coverage: dropping a truncated profile: $profile" >&2
    rm -f "$profile"
    dropped=$((dropped + 1))
  fi
done

report=(cargo llvm-cov report --ignore-filename-regex "$ignore")
if ! "${report[@]}" --lcov --output-path "$out/lcov.info" ||
  ! "${report[@]}" --json --summary-only --output-path "$out/summary.json"; then
  echo "unit-coverage: no report could be made from the run's profiles" >&2
  exit 2
fi

if ! PYTHONPATH="$NATIVE_ROOT/conformance" python3 - "$out" "$NATIVE_ROOT" "$dropped" <<'PY'; then
import json
import sys
from pathlib import Path

from agterm_conformance.code_coverage import crate_rows, render

out, repo, dropped = Path(sys.argv[1]), Path(sys.argv[2]), int(sys.argv[3])
export = json.loads((out / "summary.json").read_text(encoding="utf-8"))
# render's own first line describes the suite's report; this one leaves test code out too
table = render(crate_rows(export, repo.resolve())).split("\n", 1)[1]
intro = (
    "Line, region and function coverage of `cargo test` per workspace crate (test code, `vendor/` and the `-sys` "
    "crates excluded)."
)
notes = f"\n{dropped} truncated profile(s) of killed test processes dropped.\n" if dropped else ""
markdown = f"{intro}\n{table}{notes}"
(out / "summary.md").write_text(markdown, encoding="utf-8")
print(markdown, end="")
PY
  echo "unit-coverage: no summary could be made from $out/summary.json" >&2
  exit 2
fi

if ((status != 0)); then
  echo "unit-coverage: a test failed (the report is written)" >&2
fi
exit "$status"
