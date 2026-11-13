#!/usr/bin/env bash
# The Android app's end-to-end tests (mobile/android/app/src/androidTest/.../e2e) on a device or emulator, against a
# real machine: a private ngterm-headless behind a private, unprivileged sshd on 127.0.0.1, which the device reaches
# through `adb reverse`. It builds ngterm-headless, ngtermctl and ngterm-session, stages them (scripts/stage.sh) into a
# scratch directory under cargo's target dir, starts the instance in a private state dir and home there (every
# NGTERM_* stripped, so nothing it starts can reach an ngterm the script was started from), makes a throwaway host key
# and client key, and runs the instrumented tests through scripts/build-android.sh with the machine's address, user and
# client key as instrumentation arguments (the tests import the key and add the machine as a person does).
#
# The same sshd answers on a second port as a Mac running upstream agterm (AgtermPeerE2eTest): its logins get a home
# and a PATH of their own holding only the conformance suite's stand-in agterm
# (conformance/agterm_conformance/fake_agterm.py: its `agtermctl` and bundled `zmx`, no `ngtermctl`), seeded with one
# session; no live agterm is involved.
#
# On exit it stops the instance and sshd by the pids they recorded, ends the stand-in agterm's processes (those with
# its private home), ends every session of the private namespace through that namespace's own session server and
# removes the namespace's directory, and drops the port reverses. The scratch directory is removed after a pass and
# kept after a failure (headless.log, sshd/sshd.log). With --evidence DIR, the tests' screenshots and the
# device's log of the run are copied into DIR.
#
# Usage: scripts/test-android-e2e.sh [--profile release|debug] [--evidence DIR] [TEST...]
#   TEST: a class or Class#method of com.nickalie.ngterm.android.e2e (default: every class of the package), each
#   run on its own
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
# the evidence directory is made and resolved now, before anything starts and before the cd below: a path it cannot
# make or write stops the run here, not in the exit trap
if [[ -n "$EVIDENCE" ]]; then
  mkdir -p "$EVIDENCE" 2>/dev/null && [[ -w "$EVIDENCE" ]] || die "cannot write the evidence directory $EVIDENCE"
  EVIDENCE="$(cd "$EVIDENCE" && pwd)"
fi

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
AGTERM_PORT=
AGTERM="$WORK/agterm"

# of the given pids (all this user's when none is given), those of the processes the stand-in agterm's logins started:
# each has its private home
agterm_procs() {
  local pid pids=("$@")
  (($#)) || mapfile -t pids < <(pgrep -u "$(id -u)" 2>/dev/null || true)
  for pid in "${pids[@]}"; do
    grep -qzxF "HOME=$AGTERM/home" "/proc/$pid/environ" 2>/dev/null && echo "$pid"
  done
  return 0
}

# The instance and sshd, by the pids they recorded and only while those pids still run this run's binary or config; the
# private namespace's live sessions, ended through its own session server, and its directory once that server, left
# empty, has gone (after its idle grace; the namespace is this run's alone, hashed from its private state dir); the
# device's port reverse.
reap() {
  local session="$WORK/stage/bin/ngterm-session" pid names namespace i pids
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
  # the stand-in agterm's streams and shells, which outlive a connection the device dropped unclosed: every process its
  # logins started has its private home. An interactive shell ignores TERM, so HUP goes with it and KILL ends what is
  # left a moment later
  pids="$(agterm_procs)"
  # shellcheck disable=SC2086 # one pid per word
  if [[ -n "$pids" ]]; then
    kill -HUP $pids 2>/dev/null || true
    kill -TERM $pids 2>/dev/null || true
    for ((i = 0; i < 20; i++)); do
      pids="$(agterm_procs $pids)"
      [[ -n "$pids" ]] || break
      sleep 0.1
    done
    [[ -z "$pids" ]] || kill -KILL $pids 2>/dev/null || true
  fi
  [[ -z "$PORT" ]] || "$ADB" reverse --remove "tcp:$PORT" >/dev/null 2>&1 || true
  [[ -z "$AGTERM_PORT" ]] || "$ADB" reverse --remove "tcp:$AGTERM_PORT" >/dev/null 2>&1 || true
}
collect() {
  [[ -n "$EVIDENCE" ]] || return 0
  mkdir -p "$EVIDENCE" 2>/dev/null || {
    echo "→ cannot make $EVIDENCE; no evidence collected" >&2
    return 0
  }
  "$ADB" pull "$DEVICE_EVIDENCE/." "$EVIDENCE/" >/dev/null 2>&1 || true
  "$ADB" logcat -d >"$EVIDENCE/logcat.txt" 2>/dev/null || true
  cp "$WORK/headless.log" "$EVIDENCE/" 2>/dev/null || true
}
on_exit() {
  local status=$?
  # reap first: whatever collect meets, the processes, the namespace and the port reverse never stay behind
  reap
  collect
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

# the stand-in agterm Mac: agterm's CLI on its PATH and its app bundle's zmx, as conformance's AgtermMac lays them out,
# with one live session; its logins never see this build's binaries
mkdir -p "$AGTERM/bin" "$AGTERM/agterm.app" "$AGTERM/home" "$AGTERM/zmx-sockets"
FAKE_AGTERM="$NATIVE_ROOT/conformance/agterm_conformance/fake_agterm.py"
PYTHON="$(command -v python3)"
for program in agtermctl zmx; do
  wrapper="$AGTERM/bin/agtermctl"
  [[ "$program" == zmx ]] && wrapper="$AGTERM/agterm.app/zmx"
  printf '#!/bin/sh\nexec %q -I %q %q %s "$@"\n' "$PYTHON" "$FAKE_AGTERM" "$AGTERM" "$program" >"$wrapper"
  chmod 755 "$wrapper"
done
python3 -I - "$FAKE_AGTERM" "$AGTERM" <<'PY'
import importlib.util, os, sys, uuid
spec = importlib.util.spec_from_file_location("fake_agterm", sys.argv[1])
fake = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fake)
root = sys.argv[2]
daemon = f"agterm-{uuid.uuid4().hex}"
window = {"id": str(uuid.uuid4()).upper(), "name": "Main"}
workspace = {"id": str(uuid.uuid4()).upper(), "name": "work"}
session = {
    "id": str(uuid.uuid4()).upper(), "name": "qa-agterm-work",
    "windowID": window["id"], "windowName": window["name"],
    "workspaceID": workspace["id"], "workspaceName": workspace["name"],
    "cwd": os.path.join(root, "home"), "panes": [{"pane": "left", "daemon": daemon}],
}
fake.save_state(fake.Path(root), {
    "running": True, "restore": "live", "home": os.path.join(root, "home"),
    "zmx": os.path.join(root, "agterm.app", "zmx"), "socketDirectory": os.path.join(root, "zmx-sockets"),
    "window": window, "workspace": workspace, "sessions": [session], "daemons": [daemon],
})
PY
cat >"$WORK/agterm-command.sh" <<EOF
#!/bin/sh
export HOME='$AGTERM/home'
export PATH='$AGTERM/bin:/usr/bin:/bin'
exec /bin/sh -c "\$SSH_ORIGINAL_COMMAND"
EOF
chmod 755 "$WORK/agterm-command.sh"

# a free port is taken back from the kernel before sshd binds it, so another process may win it: three tries
for ((try = 0; try < 3; try++)); do
  read -r PORT AGTERM_PORT < <(python3 -I -c '
import socket
a, b = socket.socket(), socket.socket()
a.bind(("127.0.0.1", 0))
b.bind(("127.0.0.1", 0))
print(a.getsockname()[1], b.getsockname()[1])')
  cat >"$WORK/sshd/sshd_config" <<EOF
ListenAddress 127.0.0.1
Port $PORT
Port $AGTERM_PORT
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
Match LocalPort $AGTERM_PORT
  ForceCommand '$WORK/agterm-command.sh'
EOF
  env -i PATH=/usr/bin:/bin:/usr/sbin:/sbin "$SSHD" -D -f "$WORK/sshd/sshd_config" -E "$WORK/sshd/sshd.log" \
    </dev/null >/dev/null 2>&1 &
  for ((i = 0; i < 150; i++)); do
    banner="$(timeout 2 bash -c "exec 3<>/dev/tcp/127.0.0.1/$PORT && head -c 4 <&3" 2>/dev/null || true)"
    other="$(timeout 2 bash -c "exec 3<>/dev/tcp/127.0.0.1/$AGTERM_PORT && head -c 4 <&3" 2>/dev/null || true)"
    [[ "$banner" == SSH- && "$other" == SSH- ]] && break 2
    sleep 0.1
  done
  [[ -s "$WORK/sshd/sshd.pid" ]] && kill -TERM "$(<"$WORK/sshd/sshd.pid")" 2>/dev/null || true
  PORT=
  AGTERM_PORT=
done
[[ -n "$PORT" ]] || die "sshd never answered: $WORK/sshd/sshd.log"
"$ADB" reverse "tcp:$PORT" "tcp:$PORT" >/dev/null
"$ADB" reverse "tcp:$AGTERM_PORT" "tcp:$AGTERM_PORT" >/dev/null

"$ADB" shell rm -rf "$DEVICE_EVIDENCE"
"$ADB" shell mkdir -p "$DEVICE_EVIDENCE"
"$ADB" logcat -c || true

# each test (by default each class of the package) in an instrumentation run of its own: a run installs the app anew,
# so every test starts from a fresh process, as each signs in from a fresh install and the app's stores keep what they
# read in memory for the process's life
if ((${#TESTS[@]} == 0)); then
  for source in mobile/android/app/src/androidTest/kotlin/com/nickalie/ngterm/android/e2e/*E2eTest.kt; do
    TESTS+=("$(basename "$source" .kt)")
  done
fi
for test in "${TESTS[@]}"; do
  echo "→ $test" >&2
  scripts/build-android.sh --profile "$PROFILE" :app:connectedGithubDebugAndroidTest \
    -Pandroid.testInstrumentationRunnerArguments.class="$E2E_PACKAGE.$test" \
    -Pandroid.testInstrumentationRunnerArguments.e2eHost=127.0.0.1 \
    -Pandroid.testInstrumentationRunnerArguments.e2ePort="$PORT" \
    -Pandroid.testInstrumentationRunnerArguments.e2eAgtermPort="$AGTERM_PORT" \
    -Pandroid.testInstrumentationRunnerArguments.e2eUser="$USER_NAME" \
    -Pandroid.testInstrumentationRunnerArguments.e2eKey="$(base64 -w0 "$WORK/client_ed25519")"
done
echo "→ android e2e passed" >&2
