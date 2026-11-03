#!/usr/bin/env bash
# The mobile core's end-to-end test (crates/ngterm-mobile/tests/headless_e2e.rs) against a real ngterm-headless: builds
# ngterm-headless, ngtermctl and ngterm-session, stages them (scripts/stage.sh) into a scratch directory under cargo's
# target dir, and runs the ignored test with the stage in MOBILE_E2E_STAGE and the scratch directory in
# MOBILE_E2E_WORK. The test starts its own instance in a private state dir there, behind an unprivileged sshd on
# 127.0.0.1, and reaps it when it returns or panics. When it dies without unwinding (Ctrl-C, a timeout's signal), this
# script's exit trap reaps the same private namespace through that namespace's own session server and stops the
# instance and sshd by the pids they recorded. The scratch directory is removed after a pass or a skip and kept after a
# failure (its logs: headless.log, sshd/sshd.log).
#
# The build and stage keep the caller's NGTERM_* build overrides (NGTERM_VENDOR, NGTERM_GHOSTTY_VT_PREFIX); the test,
# and so the instance and sshd it starts, runs with every NGTERM_* stripped, so nothing it starts can reach an ngterm
# the script was started from. Linux only. Without sshd the test skips with a reason (and the script says "skipped",
# not "passed"), unless CI is set: there it fails.
#
# Usage: scripts/test-mobile-e2e.sh [--profile debug|release]   (default: debug)
#   MOBILE_E2E_WORK_ROOT overrides where the scratch directory is made (default: cargo's target dir); keep it short,
#   since the private control socket lives under it.
set -euo pipefail

# shellcheck source=lib/native.sh
source "$(dirname "$0")/lib/native.sh"

PROFILE=debug
while (($#)); do
  case "$1" in
    --profile)
      (($# >= 2)) && [[ "$2" == debug || "$2" == release ]] || die "usage: scripts/test-mobile-e2e.sh [--profile debug|release]"
      PROFILE="$2"
      shift 2
      ;;
    *) die "usage: scripts/test-mobile-e2e.sh [--profile debug|release]" ;;
  esac
done
[[ "$(host_os)" == linux ]] || die "the mobile e2e test runs on Linux"

cd "$NATIVE_ROOT"
TARGET="${CARGO_TARGET_DIR:-$NATIVE_ROOT/target}"
release=()
[[ "$PROFILE" == debug ]] || release=(--release)

cargo build "${release[@]}" -p ngterm-headless -p ngterm-ctl -p ngterm-session

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

scripts/stage.sh "$WORK/stage" --profile "$PROFILE"

# the test, and everything it starts, gets no NGTERM_* of the caller's
strip=()
while IFS='=' read -r name _; do
  strip+=(-u "$name")
done < <(env | grep -E '^NGTERM_[A-Za-z0-9_]*=' || true)
env "${strip[@]}" MOBILE_E2E_STAGE="$WORK/stage" MOBILE_E2E_WORK="$WORK" \
  cargo test "${release[@]}" -p ngterm-mobile --test headless_e2e -- --ignored --nocapture
if [[ -e "$WORK/skipped" ]]; then
  echo "→ mobile e2e skipped: no sshd on this machine" >&2
else
  echo "→ mobile e2e passed" >&2
fi
