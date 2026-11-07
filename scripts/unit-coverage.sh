#!/usr/bin/env bash
# The unit-test coverage of the Rust workspace: what `cargo test` covers, beside the conformance suite's coverage
# (run-linux.sh --coverage), which measures only what the suite drives through the staged app.
#
# One instrumented build and run of the workspace's tests (`cargo test --tests`, default features, `--no-fail-fast`,
# under `cargo llvm-cov show-env` in its own target directory), every crate but the generated -sys crates, so each
# crate is credited with every test that runs its code, its own and its dependents'; or of the `-p` crates' tests
# alone. Then agterm_conformance/code_coverage.py's `unit` report: the run's profiles read against each program this
# build built (its JSON messages name them: the test binaries and the programs they start, so a stale binary of an
# earlier build never counts) on its own and joined, as the suite's report does, then test code left out of the
# denominator (rust_test_code.py: what `#[cfg(test)]`, `#[test]` and their kin compile only under test, inline modules
# and the module files they declare, and test files by name), with vendor/ and the -sys crates:
#
# - OUT_DIR/lcov.info, the per-line report (uncovered lines by number);
# - OUT_DIR/summary.json, the per-file summary, in llvm-cov export's shape;
# - OUT_DIR/summary.md, the line, region and function % per crate and their total, which
#   conformance/tools/check_coverage.py holds against scripts/unit-coverage-floor-linux.txt;
# - OUT_DIR/summary-files.md, the line, function and region % and the uncovered lines per source file.
#
# With `-p`, the reports hold the named crates' files alone. A test child killed mid-run (a timeout, a crash test)
# leaves a truncated profile that would fail the whole merge: those are dropped and counted in the summary. Doc tests
# are not run.
#
# The default features need WPE WebKit (README > WPE WebKit) and the app's tests a display and a session bus: run it as
# README > Test runs `cargo test` (under xvfb-run and dbus-run-session). Needs cargo-llvm-cov and the toolchain's
# `llvm-tools` component, python3 for the report.
#
# Usage: scripts/unit-coverage.sh OUT_DIR [-p CRATE]...
# Exit 0: tests passed and the report is written; 1: a test failed (the report is still written); 2: usage, a failed
# build, or no report could be made.
set -euo pipefail

# shellcheck source=lib/native.sh
source "$(dirname "$0")/lib/native.sh"

usage() {
  echo "usage: scripts/unit-coverage.sh OUT_DIR [-p CRATE]..." >&2
  exit 2
}

(($# >= 1)) || usage
mkdir -p "$1"
out="$(cd "$1" && pwd)"
shift
select=()
crates=()
while (($#)); do
  [[ "$1" == "-p" && $# -ge 2 ]] || usage
  select+=(-p "$2")
  crates+=(--crate "$2")
  shift 2
done
cd "$NATIVE_ROOT"

# The tests hold more descriptors at once than a soft limit of 1024 allows (ngterm-gpui's lib tests keep about 1,200
# open by their end, at any thread count), the limit systemd sessions start with and runuser's PAM session sets for
# the CI job's test user, whatever root's was: raised here to 65536, or to the hard limit when that is lower.
hard="$(ulimit -Hn)"
want=65536
if [[ "$hard" != unlimited ]] && ((hard < want)); then
  want="$hard"
fi
soft="$(ulimit -Sn)"
if [[ "$soft" != unlimited ]] && ((soft < want)); then
  ulimit -Sn "$want"
fi

if ((${#select[@]} == 0)); then
  select=(--workspace)
  for crate in crates/*-sys; do
    select+=(--exclude "${crate#crates/}")
  done
fi

# Instrumented in cargo-llvm-cov's own target directory (as `cargo llvm-cov` builds, so its builds are reused), where
# the processes write their profiles; those of an earlier run there would count again.
target="$(cargo metadata --format-version 1 --no-deps |
  python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')"
export CARGO_TARGET_DIR="$target/llvm-cov-target"
eval "$(cargo llvm-cov show-env --sh)"
mkdir -p "$CARGO_TARGET_DIR"
find "$CARGO_TARGET_DIR" -maxdepth 1 -name '*.profraw' -delete

messages="$CARGO_TARGET_DIR/unit-coverage-build.jsonl"
if ! cargo test --tests --no-run --message-format=json "${select[@]}" >"$messages"; then
  echo "unit-coverage: the build failed" >&2
  exit 2
fi
status=0
cargo test --tests --no-fail-fast "${select[@]}" || status=1

if ! PYTHONPATH="$NATIVE_ROOT/conformance" python3 -m agterm_conformance.code_coverage unit --messages "$messages" \
  --profiles "$CARGO_TARGET_DIR" --repo "$NATIVE_ROOT" --out "$out" "${crates[@]}"; then
  echo "unit-coverage: no report could be made from the run's profiles" >&2
  exit 2
fi

if ((status != 0)); then
  echo "unit-coverage: a test failed (the report is written)" >&2
fi
exit "$status"
