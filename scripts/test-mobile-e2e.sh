#!/usr/bin/env bash
# The mobile core's end-to-end test (crates/ngterm-mobile/tests/headless_e2e.rs) against a real ngterm-headless: builds
# ngterm-headless, ngtermctl and ngterm-session, stages them (scripts/stage.sh) into a scratch directory under cargo's
# target dir, and runs the ignored test with the stage in MOBILE_E2E_STAGE and the scratch directory in
# MOBILE_E2E_WORK. The test starts its own instance in a private state dir there, behind an unprivileged sshd on
# 127.0.0.1, and reaps it when it returns or panics. When it dies without unwinding (Ctrl-C, a timeout's signal), this
# script's exit trap reaps the same private namespace through that namespace's own session server and stops the
# instance and sshd by the pids they recorded. The scratch directory is removed after a pass or a skip and kept after a
# failure (its logs: headless.log, sshd/sshd.log). Its agent chat case runs a fake agent on the system's python3
# (crates/ngterm-mobile/tests/support/fake_agent.py), its files under agent/ there.
#
# The build and stage keep the caller's NGTERM_* build overrides (NGTERM_VENDOR, NGTERM_GHOSTTY_VT_PREFIX); the tests,
# and so the instances and sshd they start, run with every NGTERM_* stripped, so nothing they start can reach an ngterm
# the script was started from. Linux only. Without sshd the test skips with a reason (and the script says "skipped",
# not "passed"), unless CI is set: there it fails.
#
# --coverage DIR: the headless app's e2e line coverage (docs/plans/20261112-e2e-coverage-90.md). The three programs
# are built instrumented (cargo llvm-cov show-env, in cargo-llvm-cov's target dir, llvm-cov-target) and staged from
# there; the tests themselves are built as usual, so only what the staged programs run is measured, never the test
# processes' own code. Every process they start from the build writes its profile, pooled per binary
# (LLVM_PROFILE_FILE, `%4m`), into llvm-cov-target/headless-e2e/, emptied first. Besides the mobile e2e, the headless
# app's other e2e run on the same build: ngterm-headless's integration tests and ngterm-session's e2e test, which take
# the build's programs in place of cargo's own through E2E_BIN_DIR. Then agterm_conformance/code_coverage.py e2e --app
# headless --platform linux reads the profiles against the build's programs, checked to be the stage's byte for byte,
# into DIR: lcov.info, lcov-measured.info, summary.json and summary.md (the total line %, its distance to 90%, the
# share the e2e-coverage markers exclude, per crate), which conformance/tools/check_coverage.py holds against
# conformance/coverage/e2e-floor-headless-linux.txt. Every test runs even when one fails. Needs cargo-llvm-cov, the
# toolchain's llvm-tools and Python 3.11+ ($PYTHON when set, else the first of python3, python, python3.13,
# python3.12, python3.11 that can import tomllib; checked before the build).
#
# Usage: scripts/test-mobile-e2e.sh [--profile debug|release] [--coverage DIR]   (default: debug)
#   MOBILE_E2E_WORK_ROOT overrides where the scratch directory is made (default: cargo's target dir); keep it short,
#   since the private control socket lives under it.
# Exit 0: passed (or skipped); 1: a test failed (with --coverage, its report is still written) or bad arguments; 2 with
# --coverage: no report could be made (no profile, a stage that is not the instrumented build) or the instrumented build
# failed; any other: a plain run's build or stage failed, with that command's status.
set -euo pipefail

# shellcheck source=lib/native.sh
source "$(dirname "$0")/lib/native.sh"

usage() { die "usage: scripts/test-mobile-e2e.sh [--profile debug|release] [--coverage DIR]"; }

PROFILE=debug
COVERAGE=""
while (($#)); do
  case "$1" in
    --profile)
      (($# >= 2)) && [[ "$2" == debug || "$2" == release ]] || usage
      PROFILE="$2"
      shift 2
      ;;
    --coverage)
      (($# >= 2)) && [[ -n "$2" ]] || usage
      COVERAGE="$2"
      shift 2
      ;;
    *) usage ;;
  esac
done
[[ "$(host_os)" == linux ]] || die "the mobile e2e test runs on Linux"

if [[ -n "$COVERAGE" ]]; then
  mkdir -p "$COVERAGE"
  COVERAGE="$(cd "$COVERAGE" && pwd)"
fi
cd "$NATIVE_ROOT"
TARGET="${CARGO_TARGET_DIR:-$NATIVE_ROOT/target}"
mkdir -p "$TARGET"
TARGET="$(cd "$TARGET" && pwd)"
release=()
[[ "$PROFILE" == debug ]] || release=(--release)

python=""
if [[ -n "$COVERAGE" ]]; then
  if [[ -n "${PYTHON:-}" ]]; then
    "$PYTHON" -c 'import tomllib' >/dev/null 2>&1 || {
      echo "mobile e2e: PYTHON=$PYTHON cannot import tomllib (Python 3.11+ required)" >&2
      exit 2
    }
    python="$PYTHON"
  else
    for candidate in python3 python python3.13 python3.12 python3.11; do
      if "$candidate" -c 'import tomllib' >/dev/null 2>&1; then
        python="$candidate"
        break
      fi
    done
    [[ -n "$python" ]] || {
      echo "mobile e2e: Python 3.11+ with tomllib is required for --coverage (set PYTHON)" >&2
      exit 2
    }
  fi
fi

# --coverage: the instrumented build's own target dir (cargo-llvm-cov's, as run-linux.sh --coverage uses), its programs
# under BUILT, and the pool the processes started from them write their profiles into
COV_TARGET="$TARGET/llvm-cov-target"
BUILT="$TARGET/$PROFILE"
PROFILES=""
# COMMAND... in the instrumented build's environment (cargo llvm-cov show-env: its rustc wrapper adds
# -C instrument-coverage and cfg(coverage); the report finds the toolchain's llvm-tools through it)
instrumented() (
  export CARGO_TARGET_DIR="$COV_TARGET"
  eval "$(cargo llvm-cov show-env --sh)"
  "$@"
)
if [[ -n "$COVERAGE" ]]; then
  BUILT="$COV_TARGET/$PROFILE"
  PROFILES="$COV_TARGET/headless-e2e"
  if ! instrumented cargo build "${release[@]}" -p ngterm-headless -p ngterm-ctl -p ngterm-session; then
    echo "mobile e2e: the instrumented build failed" >&2
    exit 2
  fi
  rm -rf "$PROFILES"
  mkdir -p "$PROFILES"
else
  cargo build "${release[@]}" -p ngterm-headless -p ngterm-ctl -p ngterm-session
fi

ROOT="${MOBILE_E2E_WORK_ROOT:-$TARGET}"
mkdir -p "$ROOT"
WORK="$(mktemp -d "$ROOT/me2e.XXXXXX")"
# the test runs from its crate's directory: hand it absolute paths
WORK="$(cd "$WORK" && pwd)"

# What the test leaves running when it dies without unwinding: every session of its private namespace, ended through
# that namespace's own server (which then goes after its idle grace), and the instance and sshd, by the pids they
# recorded and only while those pids still run this run's binary or config.
reap() {
  local user
  user="$(id -un)"
  local private=(env -i "NGTERM_STATE_DIR=$WORK/s" "HOME=$WORK/home" "PATH=$WORK/stage/bin:/usr/bin:/bin"
    "USER=$user" "LOGNAME=$user" LANG=C.UTF-8)
  local session="$WORK/stage/bin/ngterm-session" pid names
  # an instrumented session client's profile goes to the pool, not to a default.profraw in the checkout
  [[ -z "$PROFILES" ]] || private+=("LLVM_PROFILE_FILE=$PROFILES/headless-%4m.profraw")
  if [[ -s "$WORK/live-dir" && -x "$session" ]]; then
    private+=("NGTERM_LIVE_DIR=$(<"$WORK/live-dir")")
    names="$("${private[@]}" "$session" list --short 2>/dev/null || true)"
    # shellcheck disable=SC2086 # one session name per word
    [[ -z "$names" ]] || "${private[@]}" "$session" kill --force $names >/dev/null 2>&1 || true
  fi
  if [[ -s "$WORK/headless.pid" ]]; then
    pid="$(<"$WORK/headless.pid")"
    if [[ "$(readlink -f "/proc/$pid/exe" 2>/dev/null)" == "$(readlink -f "$WORK/stage/bin/ngterm-headless")" ]]; then
      kill -TERM "$pid" 2>/dev/null || true
    fi
  fi
  if [[ -s "$WORK/sshd/sshd.pid" ]]; then
    pid="$(<"$WORK/sshd/sshd.pid")"
    if tr '\0' ' ' 2>/dev/null <"/proc/$pid/cmdline" | grep -qF "$WORK/sshd/sshd_config"; then
      kill -TERM "$pid" 2>/dev/null || true
    fi
  fi
}
on_exit() {
  local status=$?
  reap
  if ((status == 0)); then
    rm -rf "$WORK"
  else
    echo "→ mobile e2e failed; its scratch directory is kept: $WORK" >&2
  fi
}
trap on_exit EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

stage_target="$TARGET"
[[ -z "$COVERAGE" ]] || stage_target="$COV_TARGET"
CARGO_TARGET_DIR="$stage_target" scripts/stage.sh "$WORK/stage" --profile "$PROFILE"

# the tests, and everything they start, get no NGTERM_* of the caller's; under --coverage the pooled profile and the
# instrumented build's programs
strip=()
while IFS='=' read -r name _; do
  strip+=(-u "$name")
done < <(env | grep -E '^NGTERM_[A-Za-z0-9_]*=' || true)
measured=()
[[ -z "$COVERAGE" ]] || measured=("LLVM_PROFILE_FILE=$PROFILES/headless-%4m.profraw" "E2E_BIN_DIR=$BUILT")
run_tests() {
  env "${strip[@]}" ${measured[@]+"${measured[@]}"} MOBILE_E2E_STAGE="$WORK/stage" MOBILE_E2E_WORK="$WORK" \
    cargo test "${release[@]}" "$@"
}

if [[ -z "$COVERAGE" ]]; then
  run_tests -p ngterm-mobile --test headless_e2e -- --ignored --nocapture || exit 1
  if [[ -e "$WORK/skipped" ]]; then
    echo "→ mobile e2e skipped: no sshd on this machine" >&2
  else
    echo "→ mobile e2e passed" >&2
  fi
  exit 0
fi

# --coverage: every e2e of the headless app, each run even when another failed, then the report
status=0
if run_tests -p ngterm-mobile --test headless_e2e -- --ignored --nocapture; then
  if [[ -e "$WORK/skipped" ]]; then
    echo "→ mobile e2e skipped: no sshd on this machine" >&2
  else
    echo "→ mobile e2e passed" >&2
  fi
else
  status=1
fi
headless_tests=()
for test in crates/ngterm-headless/tests/*.rs; do
  name="$(basename "$test" .rs)"
  headless_tests+=(--test "$name")
done
run_tests -p ngterm-headless --no-fail-fast "${headless_tests[@]}" || status=1
run_tests -p ngterm-session --test e2e || status=1

if ! PYTHONPATH="$NATIVE_ROOT/conformance" instrumented "$python" -m agterm_conformance.code_coverage e2e \
  --app headless --platform linux --profiles "$PROFILES" --release-dir "$BUILT" --stage-bin "$WORK/stage/bin" \
  --repo "$NATIVE_ROOT" --out "$COVERAGE" >/dev/null; then
  echo "mobile e2e: no coverage report could be made from the run's profiles" >&2
  exit 2
fi
echo "→ headless e2e coverage: $COVERAGE/summary.md" >&2
if ((status != 0)); then
  echo "mobile e2e: a test failed (the coverage report is written)" >&2
  exit "$status"
fi
