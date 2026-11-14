#!/usr/bin/env bash
# Build the iOS app (mobile/ios) over the mobile core (crates/ngterm-mobile):
#
# 1. take the build lock, a per-user lock file, so only one of these builds (and so one xcodebuild) runs at a time on
#    a machine: the user's 8 GB Mac can't take two;
# 2. check the toolchain (scripts/setup-ios.sh --check, whose exports it takes: DEVELOPER_DIR and the vendored
#    SwiftLint on PATH) and prepare the libghostty-vt prefix (scripts/build-ghostty-vt.sh);
# 3. build libngterm_mobile.a for aarch64-apple-ios and aarch64-apple-ios-sim (`cargo rustc --crate-type staticlib`:
#    the crate type comes from the command line, so Cargo.toml and the Android build stay as they are), reading the
#    native libraries it needs from rustc's `--print native-static-libs`;
# 4. generate the Swift bindings with the crate's own `uniffi-bindgen` (feature `bindgen`) in library mode, with
#    `--language swift --no-format`, over the host build of the library (library mode reads the interface from the
#    library's metadata, the same on every target);
# 5. stage the headers: the generated NgtermCoreFFI.h, and its module map renamed to module.modulemap (clang looks for
#    that name in an XCFramework's Headers/), with a `link` line per native library of step 3, so whatever imports the
#    module links them;
# 6. make NgtermCoreFFI.xcframework from the two archives (`xcodebuild -create-xcframework`);
# 7. write the outputs into the packages' Generated/ directories, which are gitignored (SwiftPM paths can't leave a
#    package's root), replacing only the files whose content changed, so an unchanged core rebuilds nothing in Xcode:
#    - mobile/ios/Packages/NgtermCore/Generated/: NgtermCoreFFI.xcframework, NgtermCoreBindings/ (the bindings'
#      Swift) and Fixtures/rows-golden.bin (crates/ngterm-mobile/tests/fixtures/, the row buffer's shared golden);
#    - mobile/ios/Packages/NgtermDesign/Generated/Fonts/: the mono fonts shared with Android, from their one source,
#      mobile/android/app/src/main/res/font/ (JetBrains Mono, Atkinson Hyperlegible Mono), with their OFL texts from
#      mobile/android/licenses/, so the app ships each font's license beside it;
# 8. run the actions, in the order given (default: build):
#    - lint: the suppression-reason check, SwiftLint (--strict), swift-format (`swift format lint --strict`, with
#      mobile/ios/.swift-format), and SwiftLint's analyzer rules over a build log; the generated bindings are linted by
#      none of them. Each runs even when an earlier one failed, and the action fails when any did;
#    - build: the app, its tests and its packages for the simulator (Debug);
#    - test: the Unit and then the UI test plan on the pinned phone and tablet simulators (IOS_SIM_PHONE_NAME and
#      IOS_SIM_TABLET_NAME, made by setup-ios.sh), one at a time, each with its keyboard's slide-to-type introduction
#      marked as shown (it would cover the keys the UI tests tap); result bundles in mobile/ios/build/, and a failed
#      snapshot's image in mobile/ios/build/snapshot-failures/ (its diff is in the result bundle). The Unit plan
#      measures line coverage of the app's own code (Unit.xctestplan names the targets): scripts/ios-coverage.py
#      writes its summary beside the result bundle (Unit.coverage.txt and Unit.coverage.json, from the run's
#      profiles merged into Unit.profdata) and, after the UI plan, fails the action when a target is below its floor
#      in mobile/ios/coverage-floors.txt. Each plan's run ends with its tests counted per simulator and its test
#      classes' times;
#    - coverage: the Unit plan alone on both simulators, its coverage summary and the floor check, as test measures
#      them;
#    - ui: the UI plan alone, as test runs it;
#    - archive: a Release archive for devices, mobile/ios/build/NgTerm.xcarchive, unsigned unless the release
#      environment is set (APP_STORE_CONNECT_KEY_PATH, APP_STORE_CONNECT_KEY_ID, APP_STORE_CONNECT_ISSUER_ID and
#      IOS_DEVELOPMENT_TEAM: automatic signing through that App Store Connect API key). Its marketing version is the
#      workspace version (Cargo.toml's [workspace.package]) and its build number the commit count of HEAD (a release
#      checks out its tag with the full history). It's then checked to hold no NgtermCoreFakes symbol (the fake core
#      is compiled into Debug builds only), to answer the export compliance question (ITSAppUsesNonExemptEncryption),
#      and to carry a privacy manifest that covers the required-reason APIs it uses (scripts/ios-privacy-check.sh).
#
# Everything xcodebuild runs (the compilers, the simulators, the tests and the app under test) gets an environment
# without NGTERM_* (nor SIMCTL_CHILD_NGTERM_* or TEST_RUNNER_NGTERM_*, which reach a simulator's processes), so no test
# can reach the ngterm the script was started from; cargo keeps the caller's NGTERM_* build overrides (NGTERM_VENDOR,
# NGTERM_GHOSTTY_VT_PREFIX). xcodebuild runs under `caffeinate -is`, so a sleeping Mac doesn't stall a test run, and
# with its derived data in mobile/ios/build/ (gitignored).
#
# CI runs the test plans as parallel jobs over one build of the core (.github/workflows/ios.yml), with:
#   --prebuilt-core  skip steps 3 to 6: the XCFramework and the bindings are in Generated/ already, from a run of
#                    `--no-xcode` over the same sources (step 7 still writes the fixture and the fonts);
#   --device         the test plans run on that simulator alone. The coverage floors are for both simulators'
#                    profiles together, so coverage, and test's floor check, take no --device;
#   --ui-shard K/N   the UI plan runs the K-th of N shares of its test classes (mobile/ios/NgTermUITests, in name
#                    order, dealt out one by one). The last share is told what the others run and skips it, so a
#                    class this script doesn't find still runs there, and the N shares are the whole plan.
#
# Usage: scripts/build-ios.sh [--profile release|debug] [--no-xcode] [--prebuilt-core] [--device phone|tablet]
#                             [--ui-shard K/N] [lint|build|test|coverage|ui|archive...]
#   --profile   the cargo profile of the archives (default: release)
#   --no-xcode  stop after step 7, the outputs
set -euo pipefail

# shellcheck source=lib/native.sh
source "$(dirname "$0")/lib/native.sh" # sources build/pins.env too

usage() {
  die "usage: scripts/build-ios.sh [--profile release|debug] [--no-xcode] [--prebuilt-core] [--device phone|tablet]
                            [--ui-shard K/N] [lint|build|test|coverage|ui|archive...]"
}

ARGS=("$@") # for the re-run under the build lock
PROFILE=release
XCODE=1
PREBUILT_CORE=0
DEVICE=""
UI_SHARD=""
ACTIONS=()
while (($#)); do
  case "$1" in
    --profile)
      (($# >= 2)) && [[ "$2" == debug || "$2" == release ]] || usage
      PROFILE="$2"
      shift 2
      ;;
    --no-xcode)
      XCODE=0
      shift
      ;;
    --prebuilt-core)
      PREBUILT_CORE=1
      shift
      ;;
    --device)
      (($# >= 2)) && [[ "$2" == phone || "$2" == tablet ]] || usage
      DEVICE="$2"
      shift 2
      ;;
    --ui-shard)
      (($# >= 2)) && [[ "$2" =~ ^([1-9][0-9]*)/([1-9][0-9]*)$ ]] || usage
      ((BASH_REMATCH[1] <= BASH_REMATCH[2])) || usage
      UI_SHARD="$2"
      shift 2
      ;;
    lint | build | test | coverage | ui | archive)
      ACTIONS+=("$1")
      shift
      ;;
    *) usage ;;
  esac
done
((${#ACTIONS[@]})) || ACTIONS=(build)
if [[ -n "$DEVICE" && " ${ACTIONS[*]} " == *" coverage "* ]]; then
  die "build-ios.sh: coverage takes no --device: the floors are for both simulators' profiles"
fi

[[ "$(uname -s)" == Darwin ]] || die "scripts/build-ios.sh: the iOS build needs macOS (Xcode); this host is $(uname -s)"

IOS_DIR="$NATIVE_ROOT/mobile/ios"
CORE_GENERATED="$IOS_DIR/Packages/NgtermCore/Generated"
DESIGN_GENERATED="$IOS_DIR/Packages/NgtermDesign/Generated"
BUILD_DIR="$IOS_DIR/build"
DERIVED_DATA="$BUILD_DIR/DerivedData"
TARGET="${CARGO_TARGET_DIR:-$NATIVE_ROOT/target}"
STAGING="$TARGET/ios-core"
IOS_TARGETS=(aarch64-apple-ios aarch64-apple-ios-sim)

# --- 1. the lock -----------------------------------------------------------------------------------------------------

# A flock(2) lock, which the kernel drops when its holder dies, so a killed build leaves no stale lock to take over:
# the script runs itself again under lockf(1), which holds the lock (its child doesn't inherit the descriptor) until
# that run ends. BUILD_IOS_LOCK_HELD marks the run under the lock.
LOCK="${XDG_CACHE_HOME:-$HOME/.cache}/ngterm/build-ios.flock"
if [[ "${BUILD_IOS_LOCK_HELD:-}" != "$LOCK" ]]; then
  mkdir -p "$(dirname "$LOCK")"
  lockf -ks -t 0 "$LOCK" true || echo "build-ios.sh: waiting for the build lock ($LOCK)" >&2
  BUILD_IOS_LOCK_HELD="$LOCK" exec lockf -k "$LOCK" "$BASH" "$0" ${ARGS[@]+"${ARGS[@]}"}
fi

# --- 2. the toolchain ------------------------------------------------------------------------------------------------

cd "$NATIVE_ROOT"
exports="$(scripts/setup-ios.sh --check)"
eval "$exports"

# Steps 3 to 6: the archives, the bindings, the headers and the XCFramework, into $STAGING.
build_core() {
  scripts/build-ghostty-vt.sh >/dev/null

  # --- 3. the archives -----------------------------------------------------------------------------------------------

  release=()
  [[ "$PROFILE" == debug ]] || release=(--release)
  NATIVE_LIBS=""
  for target in "${IOS_TARGETS[@]}"; do
    # --color never: a colored `note:` (CARGO_TERM_COLOR=always, as CI sets it) wouldn't match the sed below
    log="$(cargo rustc --color never -p ngterm-mobile --lib --crate-type staticlib --target "$target" \
      ${release[@]+"${release[@]}"} -- --print native-static-libs 2>&1)" || {
      echo "$log" >&2
      die "build-ios.sh: the $target archive failed to build"
    }
    # rustc prints the note only when it links, so a fresh build prints it and an up-to-date one doesn't: keep the
    # last one seen beside the archive
    libs="$(sed -n 's/.*note: native-static-libs: //p' <<<"$log" | tail -n1)"
    mkdir -p "$STAGING/$target"
    if [[ -n "$libs" ]]; then
      echo "$libs" >"$STAGING/$target/native-static-libs"
    fi
    [[ -f "$STAGING/$target/native-static-libs" ]] ||
      die "build-ios.sh: no native-static-libs for $target: remove $TARGET/$target and run again"
    libs="$(cat "$STAGING/$target/native-static-libs")"
    [[ -z "$NATIVE_LIBS" || "$NATIVE_LIBS" == "$libs" ]] ||
      die "build-ios.sh: the device and simulator archives need different libraries ($NATIVE_LIBS; $libs)"
    NATIVE_LIBS="$libs"
  done

  # --- 4. the bindings -----------------------------------------------------------------------------------------------

  cargo build -p ngterm-mobile --lib
  cargo build -p ngterm-mobile --features bindgen --bin uniffi-bindgen
  rm -rf "$STAGING/bindings"
  "$TARGET/debug/uniffi-bindgen" generate --library "$TARGET/debug/libngterm_mobile.dylib" --language swift \
    --no-format --out-dir "$STAGING/bindings"

  # --- 5. the headers ------------------------------------------------------------------------------------------------

  HEADERS="$STAGING/headers"
  rm -rf "$HEADERS"
  mkdir -p "$HEADERS"
  cp "$STAGING/bindings/NgtermCoreFFI.h" "$HEADERS/"
  # the module's closing brace gets a `link` line per -l library before it; the system's own (System, c, m) are linked
  # into every iOS binary anyway, and the linker takes them again without complaint
  link_lines=""
  for flag in $NATIVE_LIBS; do
    case "$flag" in
      -l*) link_lines="$link_lines    link \"${flag#-l}\""$'\n' ;;
      *) die "build-ios.sh: native-static-libs has $flag, which a module map can't carry" ;;
    esac
  done
  LINKS="$link_lines" awk '
    $0 == "}" { printf "%s", ENVIRON["LINKS"]; print; closed = 1; next }
    { print }
    END { if (!closed) exit 1 }' "$STAGING/bindings/NgtermCoreFFI.modulemap" >"$HEADERS/module.modulemap" ||
    die "build-ios.sh: the generated NgtermCoreFFI.modulemap has no closing brace"

  # --- 6. the XCFramework --------------------------------------------------------------------------------------------

  rm -rf "$STAGING/NgtermCoreFFI.xcframework"
  xcframework_args=()
  for target in "${IOS_TARGETS[@]}"; do
    xcframework_args+=(-library "$TARGET/$target/$PROFILE/libngterm_mobile.a" -headers "$HEADERS")
  done
  xcodebuild -create-xcframework "${xcframework_args[@]}" -output "$STAGING/NgtermCoreFFI.xcframework" >/dev/null
}

if ((PREBUILT_CORE)); then
  [[ -d "$CORE_GENERATED/NgtermCoreFFI.xcframework" &&
    -f "$CORE_GENERATED/NgtermCoreBindings/NgtermCoreBindings.swift" ]] ||
    die "build-ios.sh: --prebuilt-core, but $CORE_GENERATED holds no XCFramework and bindings"
else
  build_core
fi

# --- 7. the outputs --------------------------------------------------------------------------------------------------

# Make directory $2 hold what directory $1 holds, rewriting only the files whose content differs (no times are
# copied, so an unchanged file keeps its own and Xcode sees nothing to rebuild).
sync_dir() {
  mkdir -p "$2"
  rsync -r --checksum --delete "$1/" "$2/"
}

if ((!PREBUILT_CORE)); then
  sync_dir "$STAGING/NgtermCoreFFI.xcframework" "$CORE_GENERATED/NgtermCoreFFI.xcframework"
  mkdir -p "$STAGING/swift"
  cp "$STAGING/bindings/NgtermCoreBindings.swift" "$STAGING/swift/"
  sync_dir "$STAGING/swift" "$CORE_GENERATED/NgtermCoreBindings"
fi
mkdir -p "$STAGING/fixtures"
cp crates/ngterm-mobile/tests/fixtures/rows-golden.bin "$STAGING/fixtures/"
sync_dir "$STAGING/fixtures" "$CORE_GENERATED/Fixtures"
rm -rf "$STAGING/fonts"
mkdir -p "$STAGING/fonts"
cp mobile/android/app/src/main/res/font/jetbrains_mono_*.ttf \
  mobile/android/app/src/main/res/font/atkinson_hyperlegible_mono.ttf \
  mobile/android/licenses/OFL-jetbrains-mono.txt mobile/android/licenses/OFL-atkinson-hyperlegible-mono.txt \
  "$STAGING/fonts/"
sync_dir "$STAGING/fonts" "$DESIGN_GENERATED/Fonts"
echo "build-ios.sh: the core and the shared inputs are in mobile/ios/Packages/*/Generated/" >&2

((XCODE)) || exit 0

# --- 8. the actions --------------------------------------------------------------------------------------------------

# The environment xcodebuild gets: the caller's, without NGTERM_* and what would forward it into a simulator.
UNSET_NGTERM=()
while IFS= read -r name; do
  UNSET_NGTERM+=(-u "$name")
done < <(env | sed -nE 's/^((SIMCTL_CHILD_|TEST_RUNNER_)?NGTERM_[A-Za-z0-9_]*)=.*/\1/p')

xcodebuild_clean_env() {
  env ${UNSET_NGTERM[@]+"${UNSET_NGTERM[@]}"} caffeinate -is xcodebuild "$@"
}

# xcodebuild over the project, with its derived data under mobile/ios/build/ and the packages at the versions
# NgTerm.xcodeproj's Package.resolved pins
project_xcodebuild() {
  xcodebuild_clean_env -project "$IOS_DIR/NgTerm.xcodeproj" -derivedDataPath "$DERIVED_DATA" \
    -onlyUsePackageVersionsFromResolvedFile "$@"
}

# Apple Silicon simulators only: the core has no x86_64 simulator slice, and a generic destination would build one for
# the packages (the project's EXCLUDED_ARCHS covers its own targets only)
SIMULATOR_BUILD=(-scheme NgTerm -configuration Debug -destination "generic/platform=iOS Simulator" ARCHS=arm64)

# The UDID of the simulator named $1 on the pinned runtime (as scripts/ios-sim-runner.sh finds it).
simulator_udid() {
  xcrun simctl list devices 2>/dev/null | awk -v header="-- iOS $IOS_SIMULATOR_RUNTIME --" -v name="    $1 (" '
    /^-- / { inside = ($0 == header); next }
    inside && index($0, name) == 1 {
      rest = substr($0, length(name) + 1)
      print substr(rest, 1, index(rest, ")") - 1)
      exit
    }'
}

# The Swift sources swift-format and the suppression check read: everything under mobile/ios but the generated
# bindings and build output. SwiftLint picks the same files through .swiftlint.yml's included and excluded, which
# leave the package manifests out too.
lint_sources() {
  find "$IOS_DIR" \( -path "$IOS_DIR/build" -o -path '*/Generated' -o -path '*/.build' -o -path '*/.swiftpm' \) \
    -prune -o -name '*.swift' -type f -print | LC_ALL=C sort
}

# Every swiftlint command that switches rules off (disable, disable:next, :this, :previous) has to say why, after
# ` - ` on its own line: `// swiftlint:disable:next force_unwrapping - the literal is a valid URL`. Prints each one
# that doesn't.
check_suppression_reasons() {
  (($#)) || return 0
  ! grep -nE 'swiftlint:disable(:next|:this|:previous)?([[:space:]]|$)' "$@" |
    grep -vE 'swiftlint:disable(:next|:this|:previous)?[[:space:]]+[^-[:space:]].*[[:space:]]-[[:space:]]+[^[:space:]]'
}

action_lint() {
  local sources=() source failed=() compiled=() log="$BUILD_DIR/lint-analyze.log"
  while IFS= read -r source; do
    sources+=("$source")
    [[ "$(basename "$source")" == Package.swift ]] || compiled+=("$source")
  done < <(lint_sources)

  echo "lint: suppression reasons" >&2
  if ! check_suppression_reasons "${sources[@]}"; then
    echo "lint: a swiftlint:disable above has no reason (add ' - <why>' after its rules)" >&2
    failed+=("suppression reasons")
  fi
  echo "lint: SwiftLint" >&2
  (cd "$IOS_DIR" && swiftlint lint --strict --quiet) || failed+=(SwiftLint)
  echo "lint: swift-format" >&2
  (cd "$IOS_DIR" && xcrun swift-format lint --strict --configuration .swift-format "${sources[@]}") ||
    failed+=(swift-format)

  # the analyzer reads each file's compiler arguments from a build log, so every linted file is compiled into it:
  # touching them makes an incremental build recompile their modules
  echo "lint: SwiftLint analyzer (building)" >&2
  mkdir -p "$BUILD_DIR"
  touch "${compiled[@]}"
  if project_xcodebuild build-for-testing "${SIMULATOR_BUILD[@]}" >"$log" 2>&1; then
    # the analyzer type-checks every file of the log in SourceKit, in its own process and on several threads at once,
    # and loading the macro plugins (Swift Testing's) into that process crashed it most runs on Xcode 27.2 ("unowned
    # reference ... already destroyed" expanding one @Suite while another expands): it reads a copy of the log that
    # runs them in the toolchain's plugin server instead (-external-plugin-path), as a build may
    local server analyzed="$BUILD_DIR/lint-analyze-plugins.log"
    server="$(xcrun --find swift-plugin-server)"
    sed -E "s# -plugin-path ([^ ]+)# -external-plugin-path \\1\\#$server#g" "$log" >"$analyzed"
    (cd "$IOS_DIR" && swiftlint analyze --strict --quiet --compiler-log-path "$analyzed") ||
      failed+=("SwiftLint analyzer")
  else
    grep -E ': (error|warning): ' "$log" | sort -u >&2 || tail -n 40 "$log" >&2
    echo "lint: the build failed (log: $log)" >&2
    failed+=("build for the analyzer")
  fi

  if ((${#failed[@]})); then
    printf 'lint: FAILED: %s\n' "${failed[@]}" >&2
    return 1
  fi
  echo "lint: passed" >&2
}

action_build() {
  project_xcodebuild build-for-testing "${SIMULATOR_BUILD[@]}" -quiet
}

# Tells the keyboard of the simulator $1 that its slide-to-type introduction was shown. A simulator made or erased since
# shows that introduction the first time a keyboard that can slide to type comes up, over the soft keyboard's lower
# half and the extra keys, and it stays until its Continue is tapped: the taps the UI tests make there land on it, and
# the keys type nothing (KeyboardTests). The system writes the same preference once it has shown the introduction. A
# booted simulator takes it through its own preferences daemon; a shut-down one in its preferences file, so it needn't
# boot before xcodebuild boots it.
quiet_keyboard_introduction() {
  local key=DidShowContinuousPathIntroduction preferences
  # grep reads to the end (no -q), so simctl isn't cut short by SIGPIPE, which pipefail would count as a failure
  if xcrun simctl list devices booted | grep -F "($1)" >/dev/null; then
    xcrun simctl spawn "$1" defaults write com.apple.keyboard.preferences "$key" -bool true
  else
    preferences="$HOME/Library/Developer/CoreSimulator/Devices/$1/data/Library/Preferences"
    mkdir -p "$preferences" && defaults write "$preferences/com.apple.keyboard.preferences.plist" "$key" -bool true
  fi || die "couldn't set simulator $1's keyboard preferences"
}

# Run test plan $1 on the phone and tablet simulators, one after the other (or on --device's alone), into
# mobile/ios/build/$1.xcresult, with xcodebuild's options $2... (the UI plan's retry and shard), then count its tests
# (report_test_counts); its status is xcodebuild's.
run_test_plan() {
  local plan="$1" name udid destinations=() status=0
  shift
  for name in "$IOS_SIM_PHONE_NAME" "$IOS_SIM_TABLET_NAME"; do
    case "$DEVICE" in
      phone) [[ "$name" == "$IOS_SIM_PHONE_NAME" ]] || continue ;;
      tablet) [[ "$name" == "$IOS_SIM_TABLET_NAME" ]] || continue ;;
    esac
    udid="$(simulator_udid "$name")"
    [[ -n "$udid" ]] || die "no $name simulator on iOS $IOS_SIMULATOR_RUNTIME: run scripts/setup-ios.sh"
    quiet_keyboard_introduction "$udid"
    destinations+=(-destination "id=$udid")
  done
  mkdir -p "$BUILD_DIR"
  # a failed snapshot's image goes to snapshot-failures/<test file>/ (xcodebuild hands TEST_RUNNER_* to the test
  # processes without the prefix; the simulator's processes write the Mac's paths), its diffs into the result bundle
  export TEST_RUNNER_SNAPSHOT_ARTIFACTS="$BUILD_DIR/snapshot-failures"
  # the profiles of this run only: each simulator's are in Build/ProfileData/<its UDID>/
  rm -rf "$BUILD_DIR/$plan.xcresult" "$DERIVED_DATA/Build/ProfileData"
  project_xcodebuild test -scheme NgTerm -configuration Debug -testPlan "$plan" \
    "${destinations[@]}" -disable-concurrent-destination-testing \
    -parallel-testing-enabled NO -resultBundlePath "$BUILD_DIR/$plan.xcresult" "$@" -quiet || status=$?
  report_test_counts "$plan" "$BUILD_DIR/$plan.xcresult"
  return "$status"
}

# The UI test classes, in name order: the XCTestCase subclasses of mobile/ios/NgTermUITests.
ui_test_classes() {
  sed -nE 's/^[a-z ]*class ([A-Za-z0-9_]+): XCTestCase.*/\1/p' "$IOS_DIR"/NgTermUITests/*.swift | LC_ALL=C sort -u
}

# xcodebuild's options that keep the UI plan to --ui-shard's share (K/N), one per line: the classes are dealt out in
# name order, the i-th to share i mod N; a share but the last runs only its own, and the last skips the others'.
ui_shard_options() {
  [[ -n "$UI_SHARD" ]] || return 0
  local share="${UI_SHARD%/*}" shares="${UI_SHARD#*/}" classes class index=0
  classes="$(ui_test_classes)"
  while IFS= read -r class; do
    [[ -n "$class" ]] || continue
    if ((share < shares && index % shares == share - 1)); then
      echo "-only-testing:NgTermUITests/$class"
    elif ((share == shares && index % shares != share - 1)); then
      echo "-skip-testing:NgTermUITests/$class"
    fi
    index=$((index + 1))
  done <<<"$classes"
  ((index >= shares)) || die "build-ios.sh: --ui-shard $UI_SHARD, but there are $index UI test classes"
}

# The UI plan: a UI test that fails runs once more; report_retried_tests then judges each that passed so, red runs
# included. Status 0 only when the plan passed and none was flaky.
run_ui_plan() {
  local status=0 options option shard=()
  rm -f "$BUILD_DIR/retried-tests.txt"
  options="$(ui_shard_options)"
  while IFS= read -r option; do
    [[ -z "$option" ]] || shard+=("$option")
  done <<<"$options"
  run_test_plan UI -retry-tests-on-failure -test-iterations 2 ${shard[@]+"${shard[@]}"} || status=$?
  report_retried_tests "$BUILD_DIR/UI.xcresult" "$BUILD_DIR/retried-tests.txt" || status=1
  return "$status"
}

# The Unit plan's coverage (scripts/ios-coverage.py): summarize reads the profiles the run left in the derived data
# into Unit.coverage.txt and Unit.coverage.json beside its result bundle, so it runs before the UI plan's run replaces
# them; check fails when a target is below its floor.
coverage_summarize() {
  rm -f "$BUILD_DIR/Unit.coverage.txt" "$BUILD_DIR/Unit.coverage.json" "$BUILD_DIR/Unit.profdata"
  scripts/ios-coverage.py summarize
}

action_test() {
  rm -rf "$BUILD_DIR/snapshot-failures"
  run_test_plan Unit
  coverage_summarize
  run_ui_plan
  # one simulator's profiles are not what the floors were measured on
  [[ -n "$DEVICE" ]] || scripts/ios-coverage.py check
}

action_ui() {
  rm -rf "$BUILD_DIR/snapshot-failures"
  run_ui_plan
}

action_coverage() {
  rm -rf "$BUILD_DIR/snapshot-failures"
  run_test_plan Unit
  coverage_summarize
  scripts/ios-coverage.py check
}

# Says how many tests of plan $1 the result bundle $2 holds per simulator, by result, and its slowest test classes'
# times, so a run split over jobs can be checked against a whole one. It only reports: no status of its own.
report_test_counts() {
  [[ -d "$2" ]] || return 0
  local tests
  if tests="$(xcrun xcresulttool get test-results tests --path "$2" --compact)"; then
    TEST_PLAN="$1" python3 -c "$TEST_COUNTS_PY" <<<"$tests" ||
      echo "build-ios.sh: couldn't count the $1 plan's tests" >&2
  else
    echo "build-ios.sh: couldn't read the $1 plan's tests from $2" >&2
  fi
}

# report_test_counts' reading of `xcresulttool get test-results tests`: a Test Case node carries its result, or one
# Device node per simulator when the plan ran on several (as report_retried_tests reads them). A test that ran again
# counts once, with its last result, and its time is all its attempts' (a simulator is named by its model, as the
# Device nodes name it).
TEST_COUNTS_PY='
import collections
import json
import os
import sys

SLOWEST = 15
plan = os.environ.get("TEST_PLAN", "")
results = json.load(sys.stdin)
devices = [device.get("modelName", "") for device in results.get("devices", [])]
only = devices[0] if len(devices) == 1 else "the simulator"
counts = collections.defaultdict(collections.Counter)
seconds = collections.defaultdict(collections.Counter)


def walk(nodes, suite):
    for node in nodes:
        kind = node.get("nodeType")
        children = node.get("children", [])
        if kind == "Test Suite":
            walk(children, node.get("name", ""))
        elif kind == "Test Case":
            runs = [child for child in children if child.get("nodeType") == "Device"]
            for run in runs or [node]:
                device = run.get("name", "") if runs else only
                counts[device][run.get("result", "no result")] += 1
                attempts = [run] + [child for child in run.get("children", []) if child.get("nodeType") == "Repetition"]
                seconds[suite][device] += sum(attempt.get("durationInSeconds") or 0 for attempt in attempts)
        else:
            walk(children, suite)


walk(results.get("testNodes", []), "")
names = sorted(counts)
for device in names:
    by_result = ", ".join(str(n) + " " + result.lower() for result, n in sorted(counts[device].items()))
    print("build-ios.sh: " + plan + " plan on " + device + ": " + str(sum(counts[device].values())) + " tests ("
          + by_result + ")")
slowest = sorted(seconds.items(), key=lambda item: -sum(item[1].values()))[:SLOWEST]
if any(sum(times.values()) for _, times in slowest):
    print("build-ios.sh: " + plan + " plan, its slowest test classes, seconds on " + " / ".join(names) + ":")
    for suite, times in slowest:
        print("  " + suite + ": " + " / ".join(str(round(times[device])) for device in names))
'

# Judges each UI test of the result bundle $1 that failed and then passed on its retry, and lists them in the file $2
# (CI keeps the result bundles and attachments of a run that has one). XCTest itself fails a UI test now and then on
# CI's virtual Macs, the app and the test doing nothing wrong: it gives up terminating the app the previous test left
# running ("Failed to terminate"), or launching the next ("does not have a process ID"), or evaluating a query ("Timed
# out while evaluating UI query"), each after a minute. A retry that passed after one of those alone is named as a
# warning; one whose first attempt failed for anything else is a flaky test, which fails the run (exit 1) as any
# failure does.
report_retried_tests() {
  [[ -d "$1" ]] || return 0
  local tests
  tests="$(xcrun xcresulttool get test-results tests --path "$1" --compact)" ||
    die "couldn't read the UI tests' results from $1"
  RETRIED_TESTS_FILE="$2" python3 -c "$RETRIED_TESTS_PY" <<<"$tests"
}

# report_retried_tests' reading of `xcresulttool get test-results tests`: a test's attempts are its Repetition (or Test
# Case Run) nodes, under its Test Case node, or under each of its Device nodes when the plan ran on several. Exit 1
# when a retried test's first attempt failed for anything but XCTest's own lifecycle failures.
RETRIED_TESTS_PY='
import json
import os
import sys

LIFECYCLE = ("Failed to terminate", "does not have a process ID", "Timed out while evaluating UI query")
ATTEMPTS = ("Repetition", "Test Case Run")
github = os.environ.get("GITHUB_ACTIONS") == "true"
flaky = False
retried = []


def annotate(level, title, text):
    if github:
        # a workflow command ends at its first newline and decodes %-sequences
        escaped = text.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")
        print("::" + level + " title=" + title + "::" + escaped)
    else:
        print("build-ios.sh: " + title + ": " + text)


def walk(nodes, test, device):
    global flaky
    for node in nodes:
        kind = node.get("nodeType")
        name = node.get("name", "")
        here_test, here_device = test, device
        if kind == "Test Suite":
            here_test = name
        elif kind == "Test Case":
            here_test = test + "/" + name
        elif kind == "Device":
            here_device = name
        children = node.get("children", [])
        attempts = [child for child in children if child.get("nodeType") in ATTEMPTS]
        failed = [attempt for attempt in attempts if attempt.get("result") == "Failed"]
        if failed and attempts[-1].get("result") == "Passed":
            texts = [m.get("name", "") for m in failed[0].get("children", []) if m.get("nodeType") == "Failure Message"]
            where = here_test + (" on " + here_device if here_device else "")
            lifecycle = bool(texts) and all(any(mark in text for mark in LIFECYCLE) for text in texts)
            flaky = flaky or not lifecycle
            retried.append(where)
            verdict = "XCTest lifecycle failure, passed on its retry" if lifecycle else "flaky: passed only on its retry"
            annotate("warning" if lifecycle else "error", "UI test retried",
                     where + ": " + verdict + ": " + "; ".join(texts)[:2000])
        walk(children, here_test, here_device)


walk(json.load(sys.stdin).get("testNodes", []), "", "")
if retried and os.environ.get("RETRIED_TESTS_FILE"):
    with open(os.environ["RETRIED_TESTS_FILE"], "w", encoding="utf-8") as listing:
        listing.write("\n".join(retried) + "\n")
sys.exit(1 if flaky else 0)
'

# Fail when a binary (an app's executable or its dSYM's DWARF) holds a symbol or string of the fake core's module.
check_no_fakes() {
  local binary found=0
  for binary in "$@"; do
    [[ -f "$binary" ]] || die "build-ios.sh: no $binary to check for the fake core"
    # grep reads to the end (no -q): stopping at the first match would end nm with SIGPIPE, which pipefail turns
    # into a miss
    if { nm -a "$binary" 2>/dev/null; strings -a "$binary"; } | grep NgtermCoreFakes >/dev/null; then
      echo "build-ios.sh: $binary holds NgtermCoreFakes symbols: the fake core is for Debug builds only" >&2
      found=1
    fi
  done
  ((!found))
}

# The workspace version, Cargo.toml's [workspace.package] version.
workspace_version() {
  sed -n '/^\[workspace\.package\]$/,/^\[/s/^version = "\(.*\)"$/\1/p' "$NATIVE_ROOT/Cargo.toml"
}

action_archive() {
  local archive="$BUILD_DIR/NgTerm.xcarchive" signing=() version build app
  version="$(workspace_version)"
  [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "build-ios.sh: no MAJOR.MINOR.PATCH workspace version in Cargo.toml"
  build="$(git -C "$NATIVE_ROOT" rev-list --count HEAD)"
  if [[ -n "${APP_STORE_CONNECT_KEY_PATH:-}" && -n "${APP_STORE_CONNECT_KEY_ID:-}" &&
    -n "${APP_STORE_CONNECT_ISSUER_ID:-}" && -n "${IOS_DEVELOPMENT_TEAM:-}" ]]; then
    signing=(-allowProvisioningUpdates -authenticationKeyPath "$APP_STORE_CONNECT_KEY_PATH"
      -authenticationKeyID "$APP_STORE_CONNECT_KEY_ID" -authenticationKeyIssuerID "$APP_STORE_CONNECT_ISSUER_ID"
      DEVELOPMENT_TEAM="$IOS_DEVELOPMENT_TEAM" CODE_SIGN_STYLE=Automatic)
  else
    echo "archive: unsigned (the release environment is not set)" >&2
    signing=(CODE_SIGNING_ALLOWED=NO CODE_SIGNING_REQUIRED=NO CODE_SIGN_IDENTITY="")
  fi
  rm -rf "$archive"
  project_xcodebuild archive -scheme NgTerm -configuration Release -destination "generic/platform=iOS" \
    -archivePath "$archive" -quiet MARKETING_VERSION="$version" CURRENT_PROJECT_VERSION="$build" "${signing[@]}"
  app="$archive/Products/Applications/NgTerm.app"
  check_no_fakes "$app/NgTerm" "$archive/dSYMs/NgTerm.app.dSYM/Contents/Resources/DWARF/NgTerm"
  plutil -extract ITSAppUsesNonExemptEncryption raw -o - "$app/Info.plist" >/dev/null ||
    die "build-ios.sh: $app answers no export compliance question (ITSAppUsesNonExemptEncryption)"
  scripts/ios-privacy-check.sh "$app"
  echo "archive: $archive ($version, build $build; no fake core)" >&2
}

for action in "${ACTIONS[@]}"; do
  "action_$action"
done
