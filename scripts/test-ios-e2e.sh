#!/usr/bin/env bash
# The iOS app's end-to-end UI tests (mobile/ios/NgTermUITests, those that take a machine: the *OnAMachineTests) on a
# pinned simulator, through the app's real core, against a real machine: a private ngterm-headless behind a private,
# unprivileged sshd on the Mac's 127.0.0.1, which the simulator reaches as the Mac's own loopback. The iOS sibling of
# scripts/test-android-e2e.sh, and on the Mac only.
#
# It takes the iOS build lock for the whole run, as scripts/build-ios.sh does (one xcodebuild at a time on a Mac), and
# runs build-ios.sh's build under it. It builds ngterm-headless, ngtermctl and ngterm-session, stages them
# (scripts/stage.sh) into a scratch directory under cargo's target dir, starts the instance in a private state dir and
# home there (every NGTERM_* stripped, so nothing it starts can reach an ngterm the script was started from), makes a
# throwaway host key and a client key under a throwaway passphrase, installs the app anew on the simulator (the app
# removed, the simulator's keychain reset: a test signs in as from a fresh install), and runs each test with the
# machine in the test runner's environment (TEST_RUNNER_IOS_E2E_*: host, port, user, the client key's file, its
# passphrase and its fingerprint). The machine's PATH also holds `e2e-claude`, the mobile e2e's stand-in Claude Code in
# its standalone form (crates/ngterm-mobile/tests/support/fake_agent.py --standalone), which a test starts in a
# session's shell as a person starts Claude Code, and `e2e-clip-read`, which reads the clipboard once with OSC 52 and
# prints the answer. Each test runs with the app allowed to read another app's copy in the simulator (its pasteboard
# row in the simulator's TCC.db, put back after the test and on any exit).
#
# On exit it stops the instance and sshd by the pids they recorded, ends every session of the private namespace through
# that namespace's own session server and removes the namespace's directory. The scratch directory is removed after a
# pass and kept after a failure (headless.log, sshd/sshd.log). With --evidence DIR, each test's result bundle summary
# and attachments (screenshots, the log text it read) and the instance's log are copied into DIR.
#
# Usage: scripts/test-ios-e2e.sh [--profile release|debug] [--device phone|tablet] [--evidence DIR] [TEST...]
#   --profile  the cargo profile of the app's core (build-ios.sh's; default: release)
#   --device   the pinned simulator (default: phone)
#   TEST: a class or Class/method of NgTermUITests (default: LogsOnAMachineTests), each run on its own after a fresh
#   install. MOBILE_E2E_WORK_ROOT overrides where the scratch directory is made (default: cargo's target dir); keep it
#   short, since the private control socket lives under it.
set -euo pipefail

# shellcheck source=lib/native.sh
source "$(dirname "$0")/lib/native.sh" # sources build/pins.env too

usage() {
  die "usage: scripts/test-ios-e2e.sh [--profile release|debug] [--device phone|tablet] [--evidence DIR] [TEST...]"
}

ARGS=("$@") # for the re-run under the build lock
PROFILE=release
DEVICE=phone
EVIDENCE=
TESTS=()
while (($#)); do
  case "$1" in
    --profile)
      (($# >= 2)) && [[ "$2" == debug || "$2" == release ]] || usage
      PROFILE="$2"
      shift 2
      ;;
    --device)
      (($# >= 2)) && [[ "$2" == phone || "$2" == tablet ]] || usage
      DEVICE="$2"
      shift 2
      ;;
    --evidence)
      (($# >= 2)) || usage
      EVIDENCE="$2"
      shift 2
      ;;
    -h | --help | -*) usage ;;
    *)
      TESTS+=("$1")
      shift
      ;;
  esac
done
((${#TESTS[@]})) || TESTS=(LogsOnAMachineTests)
[[ "$(uname -s)" == Darwin ]] || die "scripts/test-ios-e2e.sh: the iOS e2e test runs on macOS (Xcode)"
# the evidence directory is made and resolved now, before anything starts and before the cd below
if [[ -n "$EVIDENCE" ]]; then
  mkdir -p "$EVIDENCE" 2>/dev/null && [[ -w "$EVIDENCE" ]] || die "cannot write the evidence directory $EVIDENCE"
  EVIDENCE="$(cd "$EVIDENCE" && pwd)"
fi

# The build lock, as build-ios.sh takes it (its header says why); build-ios.sh, run below with BUILD_IOS_LOCK_HELD
# set, then runs under this run's hold.
take_ios_build_lock test-ios-e2e.sh "$0" ${ARGS[@]+"${ARGS[@]}"}

cd "$NATIVE_ROOT"
SSHD=/usr/sbin/sshd
[[ -x "$SSHD" ]] || die "no sshd on this Mac"

scripts/build-ios.sh --profile "$PROFILE" build
eval "$(scripts/setup-ios.sh --check)"
TARGET="${CARGO_TARGET_DIR:-$NATIVE_ROOT/target}"
cargo build -p ngterm-headless -p ngterm-ctl -p ngterm-session

ROOT="${MOBILE_E2E_WORK_ROOT:-$TARGET}"
mkdir -p "$ROOT"
WORK="$(mktemp -d "$ROOT/ie2e.XXXXXX")"
WORK="$(cd "$WORK" && pwd -P)"
USER_NAME="$(id -un)"
PRIVATE=(env -i "NGTERM_STATE_DIR=$WORK/s" "HOME=$WORK/home" "PATH=$WORK/bin:$WORK/stage/bin:/usr/bin:/bin"
  SHELL=/bin/bash "USER=$USER_NAME" "LOGNAME=$USER_NAME" LANG=en_US.UTF-8)
PORT=
BUILD_DIR="$NATIVE_ROOT/mobile/ios/build"

# whether pid $1 still runs a command line holding $2
runs() {
  [[ -n "$1" ]] && ps -p "$1" -o command= 2>/dev/null | grep -qF -- "$2"
}

# The instance and sshd, by the pids they recorded and only while those pids still run this run's binary or config;
# the private namespace's live sessions, ended through its own session server, and its directory once that server,
# left empty, has gone (after its idle grace; the namespace is this run's alone, hashed from its private state dir).
reap() {
  local session="$WORK/stage/bin/ngterm-session" pid names namespace i
  if [[ -s "$WORK/headless.pid" ]]; then
    pid="$(<"$WORK/headless.pid")"
    if runs "$pid" "$WORK/stage/bin/ngterm-headless"; then
      kill -TERM "$pid" 2>/dev/null || true
    fi
  fi
  if [[ -s "$WORK/sshd/sshd.pid" ]]; then
    pid="$(<"$WORK/sshd/sshd.pid")"
    if runs "$pid" "$WORK/sshd/sshd_config"; then
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
}
# The simulator gives the app nothing of another app's copy, and asks no question, while the app's
# kTCCServicePasteboard row in the simulator's TCC.db is 3, which iOS writes at the app's first read; 2 lets the read
# through, and simctl privacy has no pasteboard service. Each test runs with the row at 2. The row as it was before
# (none, or iOS's) goes back after the test, and on any exit.
TCC_DB=
TCC_WHERE="service = 'kTCCServicePasteboard' and client = 'com.nickalie.ngterm.ios' and client_type = 0"
TCC_ROW=  # the row as it was, as an INSERT statement; empty when there was none
TCC_HELD= # set while the row is this run's
pasteboard_allow() {
  [[ -f "$TCC_DB" && -w "$TCC_DB" ]] || die "cannot write the simulator's TCC.db ($TCC_DB)"
  TCC_ROW="$(sqlite3 -bail -cmd '.timeout 5000' -cmd '.mode insert access' "$TCC_DB" \
    "select * from access where $TCC_WHERE")" || die "cannot read the app's pasteboard row in $TCC_DB"
  TCC_HELD=1
  sqlite3 -bail -cmd '.timeout 5000' "$TCC_DB" "begin; delete from access where $TCC_WHERE;
    insert into access (service, client, client_type, auth_value, auth_reason, auth_version, flags)
    values ('kTCCServicePasteboard', 'com.nickalie.ngterm.ios', 0, 2, 4, 2, 0); commit;" &&
    [[ "$(sqlite3 -cmd '.timeout 5000' "$TCC_DB" "select auth_value from access where $TCC_WHERE")" == 2 ]] ||
    die "cannot set the app's pasteboard row to 2 in $TCC_DB"
}
pasteboard_restore() {
  [[ -n "$TCC_HELD" ]] || return 0
  if sqlite3 -bail -cmd '.timeout 5000' "$TCC_DB" "begin; delete from access where $TCC_WHERE; $TCC_ROW commit;"; then
    TCC_HELD=
  else
    echo "→ cannot put the app's pasteboard row back in $TCC_DB; it was: ${TCC_ROW:-none}" >&2
    return 1
  fi
}

on_exit() {
  local status=$?
  pasteboard_restore || true
  reap
  [[ -z "$EVIDENCE" ]] || cp "$WORK/headless.log" "$EVIDENCE/" 2>/dev/null || true
  if ((status == 0)); then
    rm -rf "$WORK"
  else
    echo "→ ios e2e failed; its scratch directory is kept: $WORK" >&2
  fi
}
trap on_exit EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

SOCKET="$WORK/s/ngterm.sock"
((${#SOCKET} <= 103)) || die "the control socket path is too long ($SOCKET): set a shorter MOBILE_E2E_WORK_ROOT"
scripts/stage.sh "$WORK/stage" --profile debug
mkdir -p "$WORK/s" "$WORK/home" "$WORK/sshd" "$WORK/bin" "$WORK/client"
printf '#!/bin/sh\nexec %q -I %q --standalone "$@"\n' "$(command -v python3)" \
  "$NATIVE_ROOT/crates/ngterm-mobile/tests/support/fake_agent.py" >"$WORK/bin/e2e-claude"
chmod 755 "$WORK/bin/e2e-claude"
# `e2e-clip-read <label>`: one OSC 52 read of the clipboard on the session's terminal, its answer printed as
# R:<label>:<length>:<text> (R:<label>:none:<bytes> when no answer came)
cat >"$WORK/clip-read.py" <<'EOF'
import base64, os, select, sys, termios, time, tty
label = sys.argv[1] if len(sys.argv) > 1 else "r"
fd = os.open("/dev/tty", os.O_RDWR)
old = termios.tcgetattr(fd)
tty.setraw(fd)
buf = b""
try:
    os.write(fd, b"\x1b]52;c;?\x07")
    end = time.monotonic() + 150
    while not buf.endswith((b"\x07", b"\x1b\\")):
        ready, _, _ = select.select([fd], [], [], max(0.0, end - time.monotonic()))
        if not ready:
            break
        buf += os.read(fd, 65536)
finally:
    termios.tcsetattr(fd, termios.TCSADRAIN, old)
reply = buf.decode(errors="replace")
if "\x1b]52;" not in reply:
    print(f"R:{label}:none:{len(buf)}")
else:
    body = reply.split("\x1b]52;", 1)[1].removesuffix("\x07").removesuffix("\x1b\\")
    data = body.split(";", 1)[1] if ";" in body else ""
    text = base64.b64decode(data).decode(errors="replace") if data else ""
    print(f"R:{label}:{len(text)}:{text}")
EOF
printf '#!/bin/sh\nexec %q -I %q "$@"\n' "$(command -v python3)" "$WORK/clip-read.py" >"$WORK/bin/e2e-clip-read"
chmod 755 "$WORK/bin/e2e-clip-read"

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

# the client key under a passphrase the test types and then looks for in the log; the key's file is read by the test
# runner, which runs as this user on this Mac
PASSPHRASE="qa-pass-$(python3 -I -c 'import secrets; print(secrets.token_hex(4))')"
ssh-keygen -q -t ed25519 -N '' -C ngterm-e2e-host -f "$WORK/sshd/host_ed25519"
ssh-keygen -q -t ed25519 -N "$PASSPHRASE" -C ngterm-e2e-client -f "$WORK/client/id_ed25519"
FINGERPRINT="$(ssh-keygen -lf "$WORK/client/id_ed25519.pub" | awk '{ print $2 }')"
FINGERPRINT="${FINGERPRINT#SHA256:}"
cp "$WORK/client/id_ed25519.pub" "$WORK/sshd/authorized_keys"
cat >"$WORK/force-command.sh" <<EOF
#!/bin/sh
export NGTERM_STATE_DIR='$WORK/s'
export HOME='$WORK/home'
export PATH='$WORK/bin:$WORK/stage/bin:/usr/bin:/bin'
exec /bin/sh -c "\$SSH_ORIGINAL_COMMAND"
EOF
chmod 755 "$WORK/force-command.sh"

# a free port is taken back from the kernel before sshd binds it, so another process may win it: three tries
for ((try = 0; try < 3; try++)); do
  PORT="$(python3 -I -c '
import socket
one = socket.socket()
one.bind(("127.0.0.1", 0))
print(one.getsockname()[1])')"
  # the account's login shell runs the forced command, reading its startup files from HOME: this machine's
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
UsePAM no
SetEnv "HOME=$WORK/home"
ForceCommand '$WORK/force-command.sh'
LogLevel VERBOSE
EOF
  env -i PATH=/usr/bin:/bin:/usr/sbin:/sbin "$SSHD" -D -f "$WORK/sshd/sshd_config" -E "$WORK/sshd/sshd.log" \
    </dev/null >/dev/null 2>&1 &
  for ((i = 0; i < 150; i++)); do
    banner="$(python3 -I -c '
import socket, sys
try:
    print(socket.create_connection(("127.0.0.1", int(sys.argv[1])), timeout=2).recv(4).decode(errors="replace"))
except OSError:
    pass' "$PORT")"
    [[ "$banner" == SSH- ]] && break 2
    sleep 0.1
  done
  [[ -s "$WORK/sshd/sshd.pid" ]] && kill -TERM "$(<"$WORK/sshd/sshd.pid")" 2>/dev/null || true
  PORT=
done
[[ -n "$PORT" ]] || die "sshd never answered: $WORK/sshd/sshd.log"

case "$DEVICE" in
  phone) NAME="$IOS_SIM_PHONE_NAME" ;;
  tablet) NAME="$IOS_SIM_TABLET_NAME" ;;
esac

# The environment xcodebuild gets: the caller's, without NGTERM_* and what would forward it into a simulator
# (build-ios.sh's), and with the machine for the test runner.
UNSET_NGTERM=()
while IFS= read -r name; do
  UNSET_NGTERM+=(-u "$name")
done < <(env | sed -nE 's/^((SIMCTL_CHILD_|TEST_RUNNER_)?NGTERM_[A-Za-z0-9_]*)=.*/\1/p')
MACHINE=(TEST_RUNNER_IOS_E2E_HOST=127.0.0.1 "TEST_RUNNER_IOS_E2E_PORT=$PORT" "TEST_RUNNER_IOS_E2E_USER=$USER_NAME"
  "TEST_RUNNER_IOS_E2E_KEY=$WORK/client/id_ed25519" "TEST_RUNNER_IOS_E2E_PASSPHRASE=$PASSPHRASE"
  "TEST_RUNNER_IOS_E2E_FINGERPRINT=$FINGERPRINT")

status=0
# the keyboard as on a new simulator before its first boot here (lib/native.sh): nothing kept of a hardware keyboard.
# Once is enough: the e2e tests type no hardware key, so the simulator stays booted from one test to the next.
ios_sim_forget_hardware_keyboard "$NAME" || die "couldn't clear simulator $NAME's hardware keyboard state"
for test in "${TESTS[@]}"; do
  echo "→ $test on $NAME" >&2
  # a fresh install: booted first, so the removal and the keychain reset reach the device; the keyboard's
  # slide-to-type introduction marked as shown (it would cover the keys a test taps)
  UDID="$("$NATIVE_ROOT/scripts/setup-ios.sh" --simulator "$NAME")" ||
    die "the $NAME simulator on iOS $IOS_SIMULATOR_RUNTIME is not ready (see above)"
  TCC_DB="$(simctl_bounded "$SIMCTL_SECONDS" list devices -j | python3 -I -c '
import json, sys
for device in (d for ds in json.load(sys.stdin)["devices"].values() for d in ds):
    if device["udid"] == sys.argv[1]:
        print(device["dataPath"] + "/Library/TCC/TCC.db")' "$UDID")"
  [[ -n "$TCC_DB" ]] || die "simctl names no data directory for $NAME ($UDID)"
  ios_sim_quiet_keyboard_introduction "$UDID"
  simctl_bounded "$SIMCTL_SECONDS" uninstall "$UDID" com.nickalie.ngterm.ios 2>/dev/null || true
  simctl_bounded "$SIMCTL_SECONDS" keychain "$UDID" reset
  pasteboard_allow
  bundle="$BUILD_DIR/e2e-${test//\//-}-$DEVICE.xcresult"
  rm -rf "$bundle"
  # this build goes into the products that build-ios.sh's test plans share: its record of their last build is void
  rm -f "${BUILD_DIR:?}/built-tree"
  env ${UNSET_NGTERM[@]+"${UNSET_NGTERM[@]}"} "${MACHINE[@]}" caffeinate -is xcodebuild test \
    -project mobile/ios/NgTerm.xcodeproj -derivedDataPath "$BUILD_DIR/DerivedData" \
    -onlyUsePackageVersionsFromResolvedFile -scheme NgTerm -configuration Debug -testPlan UI \
    -destination "id=$UDID" -parallel-testing-enabled NO -resultBundlePath "$bundle" \
    "-only-testing:NgTermUITests/$test" -quiet || status=1
  pasteboard_restore || status=1
  # a test that found no machine skips itself: here that is a failure, as is a run that passed nothing
  xcrun xcresulttool get test-results summary --path "$bundle" 2>/dev/null | python3 -I -c '
import json, sys
summary = json.load(sys.stdin)
passed, skipped = summary.get("passedTests", 0), summary.get("skippedTests", 0)
failed = summary.get("failedTests", 0)
print(f"→ {passed} passed, {failed} failed, {skipped} skipped", file=sys.stderr)
sys.exit(0 if passed > 0 and skipped == 0 else 1)' || status=1
  if [[ -n "$EVIDENCE" ]]; then
    out="$EVIDENCE/${test//\//-}-$DEVICE"
    mkdir -p "$out"
    xcrun xcresulttool get test-results tests --path "$bundle" >"$out/tests.json" 2>/dev/null || true
    xcrun xcresulttool export attachments --path "$bundle" --output-path "$out" >/dev/null 2>&1 || true
  fi
done
((status == 0)) || die "→ ios e2e failed"
echo "→ ios e2e passed" >&2
