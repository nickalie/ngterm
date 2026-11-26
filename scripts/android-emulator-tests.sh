#!/usr/bin/env bash
# Runs a command (the instrumented tests) against the emulator android-emulator-runner booted on EMULATOR_PORT (the
# runner sets it in the script's environment from its emulator-port input), then stops that emulator in bounded time and
# exits with the command's status. The runner's own teardown only sends `emu kill`, and its step ends only when every
# process that holds the emulator's output pipe has exited: the emulator (its qemu, which the launcher execs into) and
# the crashpad_handler it starts in a session of its own. An emulator that accepts `emu kill` but never shuts down held
# android.yml's emulator job until its timeout. So this script sends `emu kill` itself (bounded too: a stopped
# emulator's console takes the connection and never answers), waits up to EMULATOR_STOP_TIMEOUT seconds (default 60; the
# emulator gives itself 20 before it kills its own qemu) for every holder of that pipe to exit, then sends SIGKILL to
# the ones left and logs a warning. The runner's later `emu kill` then finds no device, which it logs and ignores.
#
# Usage: EMULATOR_PORT=PORT scripts/android-emulator-tests.sh COMMAND [ARG...]
set -uo pipefail

(($# >= 1)) && [[ -n "${EMULATOR_PORT:-}" ]] || {
  echo "usage: EMULATOR_PORT=PORT scripts/android-emulator-tests.sh COMMAND [ARG...]" >&2
  exit 2
}
port="$EMULATOR_PORT"
timeout="${EMULATOR_STOP_TIMEOUT:-60}"

# The pids of the emulator processes (qemu-system-*, or the launcher before its exec) started with `-port PORT`.
emulator_pids() {
  local pid
  for pid in $(pgrep -- '^(qemu-system-|emulator$)'); do
    tr '\0' '\n' <"/proc/$pid/cmdline" 2>/dev/null |
      awk -v port="$port" 'previous == "-port" && $0 == port { found = 1 } { previous = $0 } END { exit !found }' &&
      echo "$pid"
  done
}

# The pids, other than this script's, whose stdout or stderr is the pipe $1.
pipe_holders() {
  local fd pid
  for fd in /proc/[0-9]*/fd/[12]; do
    [[ "$(readlink "$fd" 2>/dev/null)" == "$1" ]] || continue
    pid="${fd#/proc/}"
    pid="${pid%%/*}"
    ((pid != $$)) && echo "$pid"
  done | sort -u
}

# What the runner waits on: the emulator's stdout pipe and everything that holds it, or the emulator's pids alone
# when its stdout is not a pipe.
holders() {
  if [[ "$pipe" == pipe:* ]]; then
    pipe_holders "$pipe"
  else
    for pid in "${emulators[@]}"; do [[ -d "/proc/$pid" ]] && echo "$pid"; done
  fi
}

"$@"
status=$?

mapfile -t emulators < <(emulator_pids)
if ((${#emulators[@]} == 0)); then
  echo "::warning::no emulator runs on port $port after the tests"
  exit "$status"
fi
pipe="$(readlink "/proc/${emulators[0]}/fd/1" 2>/dev/null)"

timeout "$timeout" adb -s "emulator-$port" emu kill || true
deadline=$((SECONDS + timeout))
while ((SECONDS < deadline)); do
  [[ -n "$(holders)" ]] || exit "$status"
  sleep 1
done

mapfile -t left < <(holders)
if ((${#left[@]} > 0)); then
  echo "::warning::the emulator on port $port did not stop ${timeout} s after emu kill; SIGKILL to: $(
    for pid in "${left[@]}"; do printf '%s(%s) ' "$pid" "$(cat "/proc/$pid/comm" 2>/dev/null)"; done
  )"
  kill -KILL "${left[@]}" 2>/dev/null || true
fi
exit "$status"
