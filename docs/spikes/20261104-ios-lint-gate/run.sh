#!/usr/bin/env bash
# The lint gate probe of the iOS plan (docs/plans/20261103-ios-app.md, Task 4): proves that `scripts/build-ios.sh lint`
# bites. For each kind of violation it adds one new Swift file holding it to mobile/ios (the app's buildable folder or
# a package's sources, which pick a new file up by themselves), runs the gate, checks that the gate failed and that its
# output names the violation, then removes the file again. Then it checks the archive's fake-core check against a
# Debug build, which must hold the fake core, and last that the gate passes with every probe file gone.
#
# Run it on a Mac after scripts/setup-ios.sh (it takes build-ios.sh's lock, run by run):
#
#   docs/spikes/20261104-ios-lint-gate/run.sh
#
# It changes no tracked file: the probe files are new, and removed on exit however it ends. Each run's output stays in
# this directory's DerivedData/ (gitignored). It exits non-zero when a violation got through or the clean gate failed.
#
# Result on the user's M2 (2026-11-04, Xcode 27.2, SwiftLint 0.65.1): see the plan's Task 4 notes.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
IOS="$ROOT/mobile/ios"
LOGS="$HERE/DerivedData"
# nothing launched here may reach the user's ngterm
while IFS='=' read -r name _; do
  unset "$name"
done < <(env | grep '^NGTERM_' || true)

APP_PROBE="$IOS/NgTerm/GateProbe.swift"
PACKAGE_PROBE="$IOS/Packages/NgtermDesign/Sources/NgtermDesign/GateProbe.swift"
trap 'rm -f "$APP_PROBE" "$PACKAGE_PROBE"' EXIT
rm -rf "$LOGS"
mkdir -p "$LOGS"

status=0

# probe LABEL FILE MARKER: write stdin to FILE, run the gate, and check it failed with MARKER (an extended regex) in its
# output
probe() {
  local label="$1" file="$2" marker="$3" log="$LOGS/$1.log" gate=0
  cat >"$file"
  "$ROOT/scripts/build-ios.sh" lint >"$log" 2>&1 || gate=$?
  rm -f "$file"
  if ((gate != 0)) && grep -qE "$marker" "$log"; then
    echo "$label: caught ($(grep -m1 -E "$marker" "$log" | sed "s|$ROOT/||"))"
  else
    echo "$label: NOT CAUGHT (gate exit $gate; log: $log)"
    status=1
  fi
}

probe unused-declaration "$APP_PROBE" 'unused_declaration' <<'EOF'
/// Declared and never used.
internal struct GateProbeUnused {}
EOF

probe force-unwrap "$APP_PROBE" 'force_unwrapping' <<'EOF'
/// A force unwrap.
internal let gateProbeValue = Int("1")!
EOF

probe misformatting "$APP_PROBE" 'GateProbe.swift.*\[(Indentation|Spacing)\]' <<'EOF'
/// Misformatted: indented by two, with a space before the colon.
internal enum GateProbeFormat {
  internal static let value : Int = 1
}
EOF

probe data-race "$APP_PROBE" 'risks causing data races' <<'EOF'
/// A counter no actor isolates.
internal final class GateProbeCounter {
    internal var count = 0
}

/// An actor the counter is sent to.
internal actor GateProbeActor {
    internal func take(_ counter: GateProbeCounter) {
        counter.count += 1
    }
}

/// Sends the counter to the actor, then keeps using it on the main actor: a data race across actors.
@MainActor
internal func gateProbeRace(actor: GateProbeActor) async {
    let counter = GateProbeCounter()
    await actor.take(counter)
    counter.count += 1
}
EOF

# warnings as errors, in the app (SWIFT_TREAT_WARNINGS_AS_ERRORS) and in a package (treatAllWarnings)
probe deprecated-call-app "$APP_PROBE" "GateProbe.swift.*'gateProbeOld\(\)' is deprecated" <<'EOF'
/// A deprecated function.
@available(*, deprecated, message: "the probe's")
internal func gateProbeOld() {}

/// Calls the deprecated function.
internal func gateProbeCaller() {
    gateProbeOld()
}
EOF

probe deprecated-call-package "$PACKAGE_PROBE" "GateProbe.swift.*'gateProbeOld\(\)' is deprecated" <<'EOF'
/// A deprecated function.
@available(*, deprecated, message: "the probe's")
internal func gateProbeOld() {}

/// Calls the deprecated function.
public func gateProbeCaller() {
    gateProbeOld()
}
EOF

probe blanket-disable "$APP_PROBE" 'blanket_disable_command' <<'EOF'
// swiftlint:disable force_unwrapping - the probe's blanket disable, never enabled again

/// A force unwrap under a blanket disable.
internal let gateProbeValue = Int("1")!
EOF

probe disable-without-reason "$APP_PROBE" 'GateProbe.swift:[0-9]+:.*swiftlint:disable:next' <<'EOF'
/// A force unwrap whose suppression gives no reason.
// swiftlint:disable:next force_unwrapping
internal let gateProbeValue = Int("1")!
EOF

# the archive's fake-core check (build-ios.sh's check_no_fakes, the same pipeline) looks for this module's name: a
# Debug build, which compiles the fake core in, has to show it, or the Release check proves nothing. A Debug app keeps
# its code in NgTerm.debug.dylib beside a stub NgTerm.
"$ROOT/scripts/build-ios.sh" build >"$LOGS/debug-build.log" 2>&1
debug_app="$IOS/build/DerivedData/Build/Products/Debug-iphonesimulator/NgTerm.app"
if { for binary in "$debug_app"/NgTerm "$debug_app"/*.dylib; do
  [[ ! -f "$binary" ]] || {
    nm -a "$binary" 2>/dev/null
    strings -a "$binary"
  }
done; } | grep NgtermCoreFakes >/dev/null; then
  echo "fake-core check: a Debug build shows NgtermCoreFakes symbols"
else
  echo "fake-core check: NO NgtermCoreFakes symbol in the Debug build ($debug_app)"
  status=1
fi

if "$ROOT/scripts/build-ios.sh" lint >"$LOGS/clean.log" 2>&1; then
  echo "clean: the gate passes with every probe file removed"
else
  echo "clean: the gate FAILED with every probe file removed (log: $LOGS/clean.log)"
  status=1
fi
exit "$status"
