#!/usr/bin/env bash
# The Android app's end-to-end tests (mobile/android/app/src/androidTest/.../e2e) on a device or emulator, against a
# real machine: a private ngterm-headless behind a private, unprivileged sshd on 127.0.0.1, which the device reaches
# through `adb reverse`. It builds ngterm-headless, ngtermctl and ngterm-session, stages them (scripts/stage.sh) into a
# scratch directory under cargo's target dir, starts the instance in a private state dir and home there (every
# NGTERM_* stripped, so nothing it starts can reach an ngterm the script was started from), makes a throwaway host key
# and client key, and runs the instrumented tests through scripts/build-android.sh with the machine's address, user and
# client key as instrumentation arguments (the tests import the key and add the machine as a person does).
#
# On exit it stops the instance and sshd by the pids they recorded, ends every session of the private namespace
# through that namespace's own session server and removes the namespace's directory, and drops the port reverse. The scratch directory is removed after a
# pass and kept after a failure (headless.log, sshd/sshd.log). With --evidence DIR, the tests' screenshots and the
# device's log of the run are copied into DIR.
#
# Usage: scripts/test-android-e2e.sh [--profile release|debug] [--evidence DIR] [TEST...]
#   TEST: a class or Class#method of com.nickalie.ngterm.android.e2e (default: the whole package)
#   One device: the one adb sees, or ANDROID_SERIAL's. MOBILE_E2E_WORK_ROOT overrides where the scratch directory is
#   made (default: cargo's target dir); keep it short, since the private control socket lives under it.
set -euo pipefail

# shellcheck source=lib/native.sh
source "$(dirname "$0")/lib/native.sh"

E2E_PACKAGE=com.nickalie.ngterm.android.e2e
# where the tests save their screenshots on the device (SoftKeyboardSwitchE2eTest's EVIDENCE_DIR)
DEVICE_EVIDENCE=/data/local/tmp/ngterm-e2e

usage() {
  die "usage: scripts/test-android-e2e.sh [--profile release|debug] [--evidence DIR] [TEST...]"
}

PROFILE=release
EVIDENCE=
TESTS=()
while (($#)); do
  case "$1" in
    --profile)
      (($# >= 2)) && [[ "$2" == debug || "$2" == release ]] || usage
      PROFILE="$2"
      shift 2
      ;;
    --evidence)
      (($# >= 2)) || usage
      EVIDENCE="$2"
      shift 2
      ;;
    -h | --help) usage ;;
    -*) usage ;;
    *)
      TESTS+=("$1")
      shift
      ;;
  esac
done
[[ "$(host_os)" == linux ]] || die "the Android e2e test runs on Linux"

cd "$NATIVE_ROOT"
eval "$(scripts/setup-android.sh --check)"
ADB="$ANDROID_HOME/platform-tools/adb"
[[ -x "$ADB" ]] || ADB="$(command -v adb)" || die "no adb"
devices="$("$ADB" devices | awk 'NR > 1 && $2 == "device" { print $1 }')"
if [[ -z "${ANDROID_SERIAL:-}" ]]; then
  [[ -n "$devices" ]] || die "no device or emulator: start one (an AVD scripts/setup-android.sh made)"
  (($(wc -l <<<"$devices") == 1)) || die "adb sees several devices: pick one with ANDROID_SERIAL"
  export ANDROID_SERIAL="$devices"
fi
SSHD=
for candidate in "$(command -v sshd 2>/dev/null || true)" /usr/sbin/sshd /usr/local/sbin/sshd /usr/bin/sshd; do
  [[ -n "$candidate" && -x "$candidate" ]] && SSHD="$candidate" && break
done
[[ -n "$SSHD" ]] || die "no sshd on this machine"

TARGET="${CARGO_TARGET_DIR:-$NATIVE_ROOT/target}"
cargo build -p ngterm-headless -p ngterm-ctl -p ngterm-session

ROOT="${MOBILE_E2E_WORK_ROOT:-$TARGET}"
mkdir -p "$ROOT"
WORK="$(mktemp -d "$ROOT/ae2e.XXXXXX")"
WORK="$(cd "$WORK" && pwd)"
USER_NAME="$(id -un)"
PRIVATE=(env -i "NGTERM_STATE_DIR=$WORK/s" "HOME=$WORK/home" "PATH=$WORK/stage/bin:/usr/bin:/bin" SHELL=/bin/bash
  "USER=$USER_NAME" "LOGNAME=$USER_NAME" LANG=C.UTF-8)
PORT=

# The instance and sshd, by the pids they recorded and only while those pids still run this run's binary or config; the
# private namespace's live sessions, ended through its own session server, and its directory once that server, left
# empty, has gone (after its idle grace; the namespace is this run's alone, hashed from its private state dir); the
# device's port reverse.
reap() {
  local session="$WORK/stage/bin/ngterm-session" pid names namespace
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
  if [[ -s "$WORK/live-dir" && -x "$session" ]]; then
    namespace="$(<"$WORK/live-dir")"
    names="$("${PRIVATE[@]}" "NGTERM_LIVE_DIR=$namespace" "$session" list --short 2>/dev/null || true)"
    # shellcheck disable=SC2086 # one session name per word
    [[ -z "$names" ]] || "${PRIVATE[@]}" "NGTERM_LIVE_DIR=$namespace" "$session" kill --force $names \
      >/dev/null 2>&1 || true
    for ((i = 0; i < 300; i++)); do
      [[ -e "$namespace/server.sock" ]] || break
      sleep 0.1
    done
    if [[ -e "$namespace/server.sock" ]]; then
      echo "→ the session server of $namespace is still running; it and the directory are left behind" >&2
    else
      rm -rf "$namespace"
    fi
  fi
  [[ -z "$PORT" ]] || "$ADB" reverse --remove "tcp:$PORT" >/dev/null 2>&1 || true
}
collect() {
  [[ -n "$EVIDENCE" ]] || return 0
  mkdir -p "$EVIDENCE"
  "$ADB" pull "$DEVICE_EVIDENCE/." "$EVIDENCE/" >/dev/null 2>&1 || true
  "$ADB" logcat -d >"$EVIDENCE/logcat.txt" 2>/dev/null || true
  cp "$WORK/headless.log" "$EVIDENCE/" 2>/dev/null || true
}
on_exit() {
  local status=$?
  collect
  reap
  if ((status == 0)); then
    rm -rf "$WORK"
  else
    echo "→ android e2e failed; its scratch directory is kept: $WORK" >&2
  fi
}
trap on_exit EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

SOCKET="$WORK/s/ngterm.sock"
((${#SOCKET} <= 103)) || die "the control socket path is too long ($SOCKET): set a shorter MOBILE_E2E_WORK_ROOT"
scripts/stage.sh "$WORK/stage" --profile debug
mkdir -p "$WORK/s" "$WORK/home" "$WORK/sshd"

# the instance's live namespace: /tmp/ngterm-live-<FNV-1a 64 of the resolved state dir> (ngterm_core::live)
python3 -I - "$WORK/s" >"$WORK/live-dir" <<'EOF'
import os, sys
h = 14695981039346656037
for b in os.fsencode(os.path.realpath(sys.argv[1])):
    h = ((h ^ b) * 1099511628211) % (1 << 64)
print(f"/tmp/ngterm-live-{h:016x}", end="")
EOF

"${PRIVATE[@]}" "$WORK/stage/bin/ngterm-headless" --skip-agent-setup </dev/null >"$WORK/headless.log" 2>&1 &
echo $! >"$WORK/headless.pid"
for ((i = 0; i < 300; i++)); do
  "${PRIVATE[@]}" "$WORK/stage/bin/ngtermctl" live tree --json --socket "$SOCKET" >/dev/null 2>&1 && break
  ((i < 299)) || die "the private ngterm-headless never answered: $WORK/headless.log"
  sleep 0.1
done

ssh-keygen -q -t ed25519 -N '' -C ngterm-e2e-host -f "$WORK/sshd/host_ed25519"
ssh-keygen -q -t ed25519 -N '' -C ngterm-e2e-client -f "$WORK/client_ed25519"
cp "$WORK/client_ed25519.pub" "$WORK/sshd/authorized_keys"
cat >"$WORK/force-command.sh" <<EOF
#!/bin/sh
export NGTERM_STATE_DIR='$WORK/s'
export HOME='$WORK/home'
export PATH='$WORK/stage/bin:/usr/bin:/bin'
exec /bin/sh -c "\$SSH_ORIGINAL_COMMAND"
EOF
chmod 755 "$WORK/force-command.sh"

# a free port is taken back from the kernel before sshd binds it, so another process may win it: three tries
for ((try = 0; try < 3; try++)); do
  PORT="$(python3 -I -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')"
  cat >"$WORK/sshd/sshd_config" <<EOF
ListenAddress 127.0.0.1
Port $PORT
HostKey "$WORK/sshd/host_ed25519"
PidFile "$WORK/sshd/sshd.pid"
AuthorizedKeysFile "${WORK//%/%%}/sshd/authorized_keys"
AuthenticationMethods publickey
PubkeyAuthentication yes
PasswordAuthentication no
KbdInteractiveAuthentication no
AllowUsers $USER_NAME
StrictModes no
PermitUserRC no
PermitUserEnvironment no
PermitTTY yes
AllowTcpForwarding no
AllowAgentForwarding no
X11Forwarding no
PrintMotd no
ForceCommand '$WORK/force-command.sh'
LogLevel VERBOSE
EOF
  env -i PATH=/usr/bin:/bin:/usr/sbin:/sbin "$SSHD" -D -f "$WORK/sshd/sshd_config" -E "$WORK/sshd/sshd.log" \
    </dev/null >/dev/null 2>&1 &
  for ((i = 0; i < 150; i++)); do
    banner="$(timeout 2 bash -c "exec 3<>/dev/tcp/127.0.0.1/$PORT && head -c 4 <&3" 2>/dev/null || true)"
    [[ "$banner" == SSH- ]] && break 2
    sleep 0.1
  done
  [[ -s "$WORK/sshd/sshd.pid" ]] && kill -TERM "$(<"$WORK/sshd/sshd.pid")" 2>/dev/null || true
  PORT=
done
[[ -n "$PORT" ]] || die "sshd never answered: $WORK/sshd/sshd.log"
"$ADB" reverse "tcp:$PORT" "tcp:$PORT" >/dev/null

"$ADB" shell rm -rf "$DEVICE_EVIDENCE"
"$ADB" shell mkdir -p "$DEVICE_EVIDENCE"
"$ADB" logcat -c || true

which=(-Pandroid.testInstrumentationRunnerArguments.package="$E2E_PACKAGE")
if ((${#TESTS[@]})); then
  joined="$(printf "$E2E_PACKAGE.%s," "${TESTS[@]}")"
  which=(-Pandroid.testInstrumentationRunnerArguments.class="${joined%,}")
fi
scripts/build-android.sh --profile "$PROFILE" :app:connectedGithubDebugAndroidTest "${which[@]}" \
  -Pandroid.testInstrumentationRunnerArguments.e2eHost=127.0.0.1 \
  -Pandroid.testInstrumentationRunnerArguments.e2ePort="$PORT" \
  -Pandroid.testInstrumentationRunnerArguments.e2eUser="$USER_NAME" \
  -Pandroid.testInstrumentationRunnerArguments.e2eKey="$(base64 -w0 "$WORK/client_ed25519")"
echo "→ android e2e passed" >&2
