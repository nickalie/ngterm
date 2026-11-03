#!/usr/bin/env bash
# The mobile core's end-to-end test (crates/ngterm-mobile/tests/headless_e2e.rs) against a real ngterm-headless: builds
# ngterm-headless, ngtermctl and ngterm-session, stages them (scripts/stage.sh) into a scratch directory under cargo's
# target dir, and runs the ignored test with the stage in MOBILE_E2E_STAGE and the scratch directory in
# MOBILE_E2E_WORK. The test starts its own instance in a private state dir there, behind an unprivileged sshd on
# 127.0.0.1, and reaps it on every exit path; the scratch directory is removed after a pass and kept after a failure
# (its logs: headless.log, sshd/sshd.log).
#
# Every NGTERM_* variable is stripped first, so nothing this runs (cargo, the test, the instance, sshd) can reach an
# ngterm the script was started from. Linux only. Without sshd the test skips with a reason, unless CI is set: there
# it fails.
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

while IFS='=' read -r name _; do
  unset "$name"
done < <(env | grep -E '^NGTERM_[A-Za-z0-9_]*=' || true)

cd "$NATIVE_ROOT"
TARGET="${CARGO_TARGET_DIR:-$NATIVE_ROOT/target}"
release=()
[[ "$PROFILE" == debug ]] || release=(--release)

cargo build "${release[@]}" -p ngterm-headless -p ngterm-ctl -p ngterm-session

ROOT="${MOBILE_E2E_WORK_ROOT:-$TARGET}"
mkdir -p "$ROOT"
WORK="$(mktemp -d "$ROOT/me2e.XXXXXX")"
scripts/stage.sh "$WORK/stage" --profile "$PROFILE"

MOBILE_E2E_STAGE="$WORK/stage" MOBILE_E2E_WORK="$WORK" \
  cargo test "${release[@]}" -p ngterm-mobile --test headless_e2e -- --ignored --nocapture
rm -rf "$WORK"
echo "→ mobile e2e passed" >&2
