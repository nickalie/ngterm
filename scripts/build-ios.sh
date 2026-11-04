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
#      mobile/android/app/src/main/res/font/ (JetBrains Mono, Atkinson Hyperlegible Mono);
# 8. run the actions, in the order given (default: build):
#    - lint: the suppression-reason check, SwiftLint (--strict), swift-format (`swift format lint --strict`, with
#      mobile/ios/.swift-format), and SwiftLint's analyzer rules over a build log; the generated bindings are linted by
#      none of them. Each runs even when an earlier one failed, and the action fails when any did;
#    - build: the app, its tests and its packages for the simulator (Debug);
#    - test: the Unit and then the UI test plan on the pinned phone and tablet simulators (IOS_SIM_PHONE_NAME and
#      IOS_SIM_TABLET_NAME, made by setup-ios.sh), one at a time; result bundles in mobile/ios/build/;
#    - archive: a Release archive for devices, mobile/ios/build/NgTerm.xcarchive, unsigned unless the release
#      environment is set (APP_STORE_CONNECT_KEY_PATH, APP_STORE_CONNECT_KEY_ID, APP_STORE_CONNECT_ISSUER_ID and
#      IOS_DEVELOPMENT_TEAM: automatic signing through that App Store Connect API key), then checked to hold no
#      NgtermCoreFakes symbol: the fake core is compiled into Debug builds only.
#
# Everything xcodebuild runs (the compilers, the simulators, the tests and the app under test) gets an environment
# without NGTERM_* (nor SIMCTL_CHILD_NGTERM_* or TEST_RUNNER_NGTERM_*, which reach a simulator's processes), so no test
# can reach the ngterm the script was started from; cargo keeps the caller's NGTERM_* build overrides (NGTERM_VENDOR,
# NGTERM_GHOSTTY_VT_PREFIX). xcodebuild runs under `caffeinate -is`, so a sleeping Mac doesn't stall a test run, and
# with its derived data in mobile/ios/build/ (gitignored).
#
# Usage: scripts/build-ios.sh [--profile release|debug] [--no-xcode] [lint|build|test|archive...]
#   --profile   the cargo profile of the archives (default: release)
#   --no-xcode  stop after step 7, the outputs
set -euo pipefail

# shellcheck source=lib/native.sh
source "$(dirname "$0")/lib/native.sh" # sources build/pins.env too

usage() {
  die "usage: scripts/build-ios.sh [--profile release|debug] [--no-xcode] [lint|build|test|archive...]"
}

PROFILE=release
XCODE=1
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
    lint | build | test | archive)
      ACTIONS+=("$1")
      shift
      ;;
    *) usage ;;
  esac
done
((${#ACTIONS[@]})) || ACTIONS=(build)

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

# A directory, since mkdir is atomic and macOS has no flock(1); it holds the owner's pid, so a lock whose owner died
# (a killed build) is taken over instead of blocking every later build.
LOCK="${XDG_CACHE_HOME:-$HOME/.cache}/ngterm/build-ios.lock"
mkdir -p "$(dirname "$LOCK")"
waited=0
until mkdir "$LOCK" 2>/dev/null; do
  owner="$(cat "$LOCK/pid" 2>/dev/null || true)"
  if [[ -n "$owner" ]] && ! kill -0 "$owner" 2>/dev/null; then
    echo "build-ios.sh: taking over the lock of pid $owner, which is gone" >&2
    rm -rf "$LOCK"
    continue
  fi
  if ((!waited)); then
    echo "build-ios.sh: waiting for the build lock ($LOCK) held by pid ${owner:-unknown}" >&2
    waited=1
  fi
  sleep 5
done
echo "$$" >"$LOCK/pid"
trap 'rm -rf "$LOCK"' EXIT

# --- 2. the toolchain ------------------------------------------------------------------------------------------------

cd "$NATIVE_ROOT"
exports="$(scripts/setup-ios.sh --check)"
eval "$exports"
scripts/build-ghostty-vt.sh >/dev/null

# --- 3. the archives -------------------------------------------------------------------------------------------------

release=()
[[ "$PROFILE" == debug ]] || release=(--release)
NATIVE_LIBS=""
for target in "${IOS_TARGETS[@]}"; do
  log="$(cargo rustc -p ngterm-mobile --lib --crate-type staticlib --target "$target" ${release[@]+"${release[@]}"} \
    -- --print native-static-libs 2>&1)" || {
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

# --- 4. the bindings -------------------------------------------------------------------------------------------------

cargo build -p ngterm-mobile --lib
cargo build -p ngterm-mobile --features bindgen --bin uniffi-bindgen
rm -rf "$STAGING/bindings"
"$TARGET/debug/uniffi-bindgen" generate --library "$TARGET/debug/libngterm_mobile.dylib" --language swift \
  --no-format --out-dir "$STAGING/bindings"

# --- 5. the headers --------------------------------------------------------------------------------------------------

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

# --- 6. the XCFramework ----------------------------------------------------------------------------------------------

rm -rf "$STAGING/NgtermCoreFFI.xcframework"
xcframework_args=()
for target in "${IOS_TARGETS[@]}"; do
  xcframework_args+=(-library "$TARGET/$target/$PROFILE/libngterm_mobile.a" -headers "$HEADERS")
done
xcodebuild -create-xcframework "${xcframework_args[@]}" -output "$STAGING/NgtermCoreFFI.xcframework" >/dev/null

# --- 7. the outputs --------------------------------------------------------------------------------------------------

# Make directory $2 hold what directory $1 holds, rewriting only the files whose content differs (no times are
# copied, so an unchanged file keeps its own and Xcode sees nothing to rebuild).
sync_dir() {
  mkdir -p "$2"
  rsync -r --checksum --delete "$1/" "$2/"
}

sync_dir "$STAGING/NgtermCoreFFI.xcframework" "$CORE_GENERATED/NgtermCoreFFI.xcframework"
mkdir -p "$STAGING/swift"
cp "$STAGING/bindings/NgtermCoreBindings.swift" "$STAGING/swift/"
sync_dir "$STAGING/swift" "$CORE_GENERATED/NgtermCoreBindings"
mkdir -p "$STAGING/fixtures"
cp crates/ngterm-mobile/tests/fixtures/rows-golden.bin "$STAGING/fixtures/"
sync_dir "$STAGING/fixtures" "$CORE_GENERATED/Fixtures"
rm -rf "$STAGING/fonts"
mkdir -p "$STAGING/fonts"
cp mobile/android/app/src/main/res/font/jetbrains_mono_*.ttf \
  mobile/android/app/src/main/res/font/atkinson_hyperlegible_mono.ttf "$STAGING/fonts/"
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
    (cd "$IOS_DIR" && swiftlint analyze --strict --quiet --compiler-log-path "$log") || failed+=("SwiftLint analyzer")
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

action_test() {
  local phone tablet plan
  phone="$(simulator_udid "$IOS_SIM_PHONE_NAME")"
  tablet="$(simulator_udid "$IOS_SIM_TABLET_NAME")"
  [[ -n "$phone" && -n "$tablet" ]] ||
    die "no $IOS_SIM_PHONE_NAME or $IOS_SIM_TABLET_NAME simulator on iOS $IOS_SIMULATOR_RUNTIME: run scripts/setup-ios.sh"
  mkdir -p "$BUILD_DIR"
  for plan in Unit UI; do
    rm -rf "$BUILD_DIR/$plan.xcresult"
    project_xcodebuild test -scheme NgTerm -configuration Debug -testPlan "$plan" \
      -destination "id=$phone" -destination "id=$tablet" -disable-concurrent-destination-testing \
      -parallel-testing-enabled NO -resultBundlePath "$BUILD_DIR/$plan.xcresult" -quiet
  done
}

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

action_archive() {
  local archive="$BUILD_DIR/NgTerm.xcarchive" signing=()
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
    -archivePath "$archive" -quiet "${signing[@]}"
  check_no_fakes "$archive/Products/Applications/NgTerm.app/NgTerm" \
    "$archive/dSYMs/NgTerm.app.dSYM/Contents/Resources/DWARF/NgTerm"
  echo "archive: $archive (no fake core)" >&2
}

for action in "${ACTIONS[@]}"; do
  "action_$action"
done
