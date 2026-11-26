#!/usr/bin/env bash
# Tests scripts/android-emulator-tests.sh against a stand-in emulator: a `qemu-system-x86_64-headless` script that
# writes into a pipe, and an `adb` whose `emu kill` sends it SIGTERM (or never returns). A stand-in that obeys exits;
# one that ignores it also leaves a detached holder of its output pipe, as crashpad_handler does. It checks the exit
# status (the command's), that the pipe the runner reads reaches EOF, the warning and SIGKILL past the bound, the bound
# on `emu kill`, and that an emulator on another port is left alone. Ports far from adb's default keep it off a real
# emulator.
#
# Usage: scripts/test-android-emulator-tests.sh
# shellcheck disable=SC2016,SC2034 # check() takes its condition as text and evaluates it later, with the variables it reads
set -euo pipefail

source_root="$(cd "$(dirname "$0")/.." && pwd)"
script="$source_root/scripts/android-emulator-tests.sh"
work="$(mktemp -d)"
cleanup() {
  local file
  for file in "$work"/*.pid; do
    [[ -e "$file" ]] && kill -KILL "$(cat "$file")" 2>/dev/null || true
  done
  rm -rf "${work:?}"
}
trap cleanup EXIT

failures=0
pass() { printf 'ok   %s\n' "$1"; }
flunk() {
  printf 'FAIL %s\n' "$1" >&2
  failures=$((failures + 1))
}
check() { if eval "$2"; then pass "$1"; else flunk "$1"; fi; }

mkdir -p "$work/bin" "$work/emulator"
cat >"$work/emulator/qemu-system-x86_64-headless" <<'EOF'
#!/bin/bash
# -port PORT, then the mode: obey or ignore
if [[ "$3" == ignore ]]; then
  trap '' TERM
  setsid sleep 600 &
  echo "$!" >"$STANDIN_DIR/holder-$2.pid"
else
  trap 'exit 0' TERM
fi
# Written last: the test sends emu kill once this file is there, so the trap and the holder must exist first.
echo "$$" >"$STANDIN_DIR/emulator-$2.pid"
echo booted
while :; do sleep 0.1; done
EOF
cat >"$work/bin/adb" <<'EOF'
#!/usr/bin/env bash
echo "$*" >>"$STANDIN_DIR/adb.log"
[[ ! -e "$STANDIN_DIR/adb-hangs" ]] || exec sleep 600
[[ "$1 $3 $4" == "-s emu kill" ]] || exit 1
kill -TERM "$(cat "$STANDIN_DIR/emulator-${2#emulator-}.pid")"
EOF
chmod +x "$work/emulator/qemu-system-x86_64-headless" "$work/bin/adb"
export STANDIN_DIR="$work" PATH="$work/bin:$PATH"

# Starts the stand-in on port $1 in mode $2, its output on an anonymous pipe (as the runner gives it) that a reader
# drains into $work/out-$1.
start_emulator() {
  rm -f "$work/emulator-$1.pid"
  setsid "$work/emulator/qemu-system-x86_64-headless" -port "$1" "$2" 2>&1 | timeout 60 cat >"$work/out-$1" &
  echo "$!" >"$work/reader-$1.pid"
  for _ in $(seq 1 50); do
    [[ -s "$work/emulator-$1.pid" ]] && return 0
    sleep 0.1
  done
  echo "the stand-in emulator did not start" >&2
  exit 1
}

# Waits up to $2 seconds for the reader of port $1's pipe to see EOF.
reader_done() {
  local pid
  pid="$(cat "$work/reader-$1.pid")"
  for _ in $(seq 1 $(($2 * 10))); do
    kill -0 "$pid" 2>/dev/null || return 0
    sleep 0.1
  done
  return 1
}

alive() { kill -0 "$(cat "$work/$1.pid")" 2>/dev/null; }

# Runs the script with EMULATOR_PORT=$1, as the runner sets it, and the command in the rest.
run() {
  local status=0
  EMULATOR_PORT="$1" "$script" "${@:2}" >"$work/log" 2>&1 || status=$?
  echo "$status"
}

port=5798
start_emulator "$port" obey
status="$(run "$port" true)"
check "a passing command's status after an emulator that stops" '[[ "$status" == 0 ]]'
check "the emulator's output pipe reaches EOF" 'reader_done "$port" 5'
check "emu kill goes to the emulator's serial" 'grep -qxF -- "-s emulator-$port emu kill" "$work/adb.log"'
check "no warning when the emulator stops" '! grep -q warning "$work/log"'

start_emulator "$port" obey
status="$(run "$port" bash -c 'exit 3')"
check "a failing command's status is kept" '[[ "$status" == 3 ]]'
check "the emulator stops after a failing command" 'reader_done "$port" 5'

start_emulator "$port" ignore
started=$SECONDS
status="$(EMULATOR_STOP_TIMEOUT=2 run "$port" bash -c 'exit 5')"
check "a failing command's status is kept when the emulator hangs" '[[ "$status" == 5 ]]'
check "a hung emulator is stopped within the bound" '((SECONDS - started < 10))'
check "the pipe reaches EOF after a hung emulator" 'reader_done "$port" 5'
check "the detached holder of the pipe is killed" '! alive "holder-$port"'
warning="::warning::the emulator on port $port did not stop 2 s after emu kill"
check "a hung emulator logs a warning" 'grep -qF "$warning" "$work/log"'

start_emulator "$port" ignore
status="$(EMULATOR_STOP_TIMEOUT=1 run "$port" true)"
check "a passing command's status is kept when the emulator hangs" '[[ "$status" == 0 ]]'
check "the hung emulator is gone" '! alive "emulator-$port"'

start_emulator "$port" ignore
touch "$work/adb-hangs"
started=$SECONDS
status="$(EMULATOR_STOP_TIMEOUT=2 run "$port" true)"
rm "$work/adb-hangs"
check "an emu kill that never returns is bounded" '[[ "$status" == 0 ]] && ((SECONDS - started < 10))'
check "the pipe reaches EOF after an emu kill that never returns" 'reader_done "$port" 5'

other=5796
start_emulator "$other" obey
: >"$work/adb.log"
status="$(run "$port" bash -c 'exit 4')"
check "with no emulator on the port, the command's status" '[[ "$status" == 4 ]]'
check "with no emulator on the port, a warning" 'grep -q "::warning::no emulator runs on port $port" "$work/log"'
check "an emulator on another port is left alone" 'alive "emulator-$other" && [[ ! -s "$work/adb.log" ]]'

status="$(run "$port")"
check "a missing command is a usage error" '[[ "$status" == 2 ]] && grep -q "^usage:" "$work/log"'
status="$(run "" true)"
check "an empty EMULATOR_PORT is a usage error" '[[ "$status" == 2 ]] && grep -q "^usage:" "$work/log"'
status=0
env -u EMULATOR_PORT "$script" true >"$work/log" 2>&1 || status=$?
check "an unset EMULATOR_PORT is a usage error" '[[ "$status" == 2 ]] && grep -q "^usage:" "$work/log"'

((failures == 0)) || {
  echo "$failures check(s) failed" >&2
  exit 1
}
