#!/usr/bin/env bash
# The locked-screen probe of the iOS plan (docs/plans/20261103-ios-app.md, Task 1): whether a simulator UI test
# (XCUITest) and a snapshot test (swift-snapshot-testing, recorded, then verified) run on this Mac from an ssh shell
# while its screen is locked. Run it after scripts/setup-ios.sh, with the screen locked, from ssh:
#
#   docs/spikes/20261103-ios-locked-screen/run.sh
#
# It prints whether the screen is locked, then the result of each test, and exits non-zero when one fails. Its build
# output stays in this directory's DerivedData/ (gitignored); the recorded snapshot is removed again at the end.
#
# Result on the user's M2 (2026-11-03, Xcode 27.2, iOS 27.0 runtime, screen locked, from a shell of an ngterm-headless
# the Linux host started over ssh): the UI test passed (149 s with the simulator's first boot, 59 s warm), the snapshot
# recorded and then verified.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
# nothing launched here may reach the user's ngterm
while IFS='=' read -r name _; do
  unset "$name"
done < <(env | grep '^NGTERM_' || true)

eval "$("$ROOT/scripts/setup-ios.sh" --check)"
# shellcheck source=../../../build/pins.env
source "$ROOT/build/pins.env"

locked="$(ioreg -n Root -d1 -a | grep -A1 CGSSessionScreenIsLocked | tail -n1 | tr -d '\t<>/')"
echo "screen locked: ${locked:-unknown}; SSH_CONNECTION: ${SSH_CONNECTION:-unset}"
echo "launchd session: $(launchctl managername)"

DESTINATION="platform=iOS Simulator,name=$IOS_SIM_PHONE_NAME,OS=$IOS_SIMULATOR_RUNTIME"
SNAPSHOTS="$HERE/SnapProbe/Tests/SnapProbeTests/__Snapshots__"
trap 'rm -rf "$SNAPSHOTS"' EXIT
rm -rf "$SNAPSHOTS"

status=0
run() {
  local label="$1"
  shift
  if caffeinate -is "$@" >"$HERE/DerivedData/$label.log" 2>&1; then
    echo "$label: passed"
  else
    echo "$label: FAILED (log: $HERE/DerivedData/$label.log)"
    status=1
  fi
}

mkdir -p "$HERE/DerivedData"
run ui-test xcodebuild test -project "$HERE/Probe.xcodeproj" -scheme Probe -destination "$DESTINATION" \
  -derivedDataPath "$HERE/DerivedData/probe"
# the first run records the missing snapshot (record: .missing fails the test that records), the second verifies it
(cd "$HERE/SnapProbe" && caffeinate -is xcodebuild test -scheme SnapProbe -destination "$DESTINATION" \
  -derivedDataPath "$HERE/DerivedData/snap" >"$HERE/DerivedData/snapshot-record.log" 2>&1) || true
if [[ -n "$(find "$SNAPSHOTS" -name '*.png' 2>/dev/null)" ]]; then
  echo "snapshot-record: recorded $(find "$SNAPSHOTS" -name '*.png' | wc -l | tr -d ' ') image(s)"
else
  echo "snapshot-record: FAILED, nothing recorded (log: $HERE/DerivedData/snapshot-record.log)"
  status=1
fi
cd "$HERE/SnapProbe"
run snapshot-verify xcodebuild test -scheme SnapProbe -destination "$DESTINATION" \
  -derivedDataPath "$HERE/DerivedData/snap"
exit "$status"
