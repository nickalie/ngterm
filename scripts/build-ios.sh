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
#      IOS_SIM_TABLET_NAME, made by setup-ios.sh), one at a time (each plan is built once before any boots; each
#      simulator boots right before its run, and shuts down before the next one boots), each with its keyboard in a
#      new simulator's state (lib/native.sh's ios_sim_keyboard_as_new: no slide-to-type introduction over the keys
#      the UI tests tap, no hardware keyboard that keeps the soft keyboard below the screen) and, while it is shut
#      down before its boot, its wallpaper cache kept to BUILD_IOS_SIM_CACHE_MB MiB (default 1024,
#      lib/ios-sim-cache.sh); result bundles in
#      mobile/ios/build/, and a failed snapshot's image in mobile/ios/build/snapshot-failures/ (its diff is in the
#      result bundle). The Unit plan measures line coverage of the app's own code (Unit.xctestplan names the
#      targets): scripts/ios-coverage.py writes its summary beside the result bundle (Unit.coverage.txt and
#      Unit.coverage.json, from the run's profiles merged into Unit.profdata) and, after the UI plan, fails the
#      action when a target is below its floor in mobile/ios/coverage-floors.txt. Each plan's run ends with its tests
#      counted per simulator and its test classes' times;
#    - coverage: the Unit plan alone on both simulators, its coverage summary and the floor check, as test measures
#      them;
#    - ui: the UI plan alone, as test runs it;
#    - smoke: the smoke set of the UI plan alone (UI_SMOKE_TESTS, below), on the simulators that ui uses, with the same
#      retry and the same check of retried tests. Each push and pull request runs it in CI. The nightly run runs the
#      whole UI plan (.github/workflows/ios.yml);
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
# For the shared Mac (scripts/ios-mac-run.sh sets them):
#   --only CLASS[/METHOD]  the ui action runs only these UI tests (repeat the option for more), as a fix round does;
#                    smoke takes no --only: its tests are UI_SMOKE_TESTS;
#   BUILD_IOS_TREE_STAMP   a checksum of the whole source tree: a plan that was built from the same stamp, profile,
#                    Xcode and Generated/ files, with no other build after it, keeps its build, and its runs only
#                    test it (test-without-building), so a repeat run and the other simulator's run rebuild nothing;
#   BUILD_IOS_BUILD_NUMBER the archive's build number (CURRENT_PROJECT_VERSION; default: the commit count of HEAD),
#                    for a copy of the tree with no Git history;
#   BUILD_IOS_CORE_CACHE   a directory of prebuilt cores (lib/ios-core-cache.sh), keyed by the core's inputs: a core
#                    found there skips steps 3 to 6, and a core built here is stored there, in at most
#                    BUILD_IOS_CORE_CACHE_MB MiB (default 2048) and only while BUILD_IOS_CORE_CACHE_MIN_FREE_MB
#                    (default 8192) of the disk stay free.
#
# Usage: scripts/build-ios.sh [--profile release|debug] [--no-xcode] [--prebuilt-core] [--device phone|tablet]
#                             [--ui-shard K/N] [--only CLASS[/METHOD]]... [lint|build|test|coverage|ui|smoke|archive...]
#   --profile   the cargo profile of the archives (default: release)
#   --no-xcode  stop after step 7, the outputs
set -euo pipefail

# shellcheck source=lib/native.sh
source "$(dirname "$0")/lib/native.sh" # sources build/pins.env too
# shellcheck source=lib/ios-core-cache.sh
source "$(dirname "$0")/lib/ios-core-cache.sh"
# shellcheck source=lib/ios-sim-cache.sh
source "$(dirname "$0")/lib/ios-sim-cache.sh"

usage() {
  die "usage: scripts/build-ios.sh [--profile release|debug] [--no-xcode] [--prebuilt-core] [--device phone|tablet]
                            [--ui-shard K/N] [--only CLASS[/METHOD]]... [lint|build|test|coverage|ui|smoke|archive...]"
}

ARGS=("$@") # for the re-run under the build lock
PROFILE=release
XCODE=1
PREBUILT_CORE=0
DEVICE=""
UI_SHARD=""
ONLY=()
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
    --only)
      (($# >= 2)) && [[ "$2" =~ ^[A-Za-z_][A-Za-z0-9_]*(/[A-Za-z_][A-Za-z0-9_]*)?$ ]] || usage
      ONLY+=("-only-testing:NgTermUITests/$2")
      shift 2
      ;;
    lint | build | test | coverage | ui | smoke | archive)
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
if ((${#ONLY[@]})) && [[ -n "$UI_SHARD" || " ${ACTIONS[*]} " != " ui " ]]; then
  die "build-ios.sh: --only is for the ui action alone, with no --ui-shard"
fi
if [[ -n "$UI_SHARD" && " ${ACTIONS[*]} " == *" smoke "* ]]; then
  die "build-ios.sh: smoke takes no --ui-shard: its set is UI_SMOKE_TESTS"
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
# that run ends. BUILD_IOS_LOCK_HELD marks the run under the lock; under an outer `lockf -k <lock> ...` the script
# runs on under that hold (take_ios_build_lock, lib/native.sh).
take_ios_build_lock build-ios.sh "$0" ${ARGS[@]+"${ARGS[@]}"}

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

# What step 7 writes into Generated/: the outputs of steps 3 to 6, or a cached core's.
CORE_XCFRAMEWORK="$STAGING/NgtermCoreFFI.xcframework"
CORE_BINDINGS="$STAGING/bindings/NgtermCoreBindings.swift"

# Steps 3 to 6, or a core of BUILD_IOS_CORE_CACHE with the same inputs; a core built here goes into the cache.
cached_core() {
  local cache="$BUILD_IOS_CORE_CACHE" toolchain key entry
  [[ "$cache" == /* ]] || die "build-ios.sh: BUILD_IOS_CORE_CACHE=$cache is not an absolute path"
  toolchain="$(rustc -vV && cargo -V && xcodebuild -version)" || toolchain=""
  if [[ -z "$toolchain" ]] || ! key="$(ios_core_cache_key "$NATIVE_ROOT" ngterm-mobile "$PROFILE" "$toolchain")"; then
    echo "build-ios.sh: no key for the core cache (cargo metadata failed): building the core" >&2
    build_core
    return
  fi
  if entry="$(ios_core_cache_entry "$cache" "$key")"; then
    echo "build-ios.sh: the core comes from the cache ($key)" >&2
    CORE_XCFRAMEWORK="$entry/NgtermCoreFFI.xcframework"
    CORE_BINDINGS="$entry/NgtermCoreBindings.swift"
    return
  fi
  echo "build-ios.sh: no cached core for $key: building it" >&2
  build_core
  ios_core_cache_store "$cache" "$key" "$CORE_XCFRAMEWORK" "$CORE_BINDINGS" "${BUILD_IOS_CORE_CACHE_MB:-2048}" \
    "${BUILD_IOS_CORE_CACHE_MIN_FREE_MB:-8192}"
}

if ((PREBUILT_CORE)); then
  [[ -d "$CORE_GENERATED/NgtermCoreFFI.xcframework" &&
    -f "$CORE_GENERATED/NgtermCoreBindings/NgtermCoreBindings.swift" ]] ||
    die "build-ios.sh: --prebuilt-core, but $CORE_GENERATED holds no XCFramework and bindings"
elif [[ -n "${BUILD_IOS_CORE_CACHE:-}" ]]; then
  cached_core
else
  build_core
fi

# --- 7. the outputs --------------------------------------------------------------------------------------------------

# sync_dir (lib/native.sh) writes only the files whose content changed, so Xcode sees nothing to rebuild.
if ((!PREBUILT_CORE)); then
  sync_dir "$CORE_XCFRAMEWORK" "$CORE_GENERATED/NgtermCoreFFI.xcframework"
  rm -rf "${STAGING:?}/swift"
  mkdir -p "$STAGING/swift"
  cp "$CORE_BINDINGS" "$STAGING/swift/"
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
# NgTerm.xcodeproj's Package.resolved pins. Every action but test-without-building can build into the shared
# products, so it removes the record of the last test plan build (build_test_plan).
project_xcodebuild() {
  [[ "$1" == test-without-building ]] || rm -f "${BUILD_DIR:?}/built-tree"
  xcodebuild_clean_env -project "$IOS_DIR/NgTerm.xcodeproj" -derivedDataPath "$DERIVED_DATA" \
    -onlyUsePackageVersionsFromResolvedFile "$@"
}

# Apple Silicon simulators only: the core has no x86_64 simulator slice, and a generic destination would build one for
# the packages (the project's EXCLUDED_ARCHS covers its own targets only)
SIMULATOR_BUILD=(-scheme NgTerm -configuration Debug -destination "generic/platform=iOS Simulator" ARCHS=arm64)

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

# Keeps the wallpaper cache of the pinned simulator $1 to BUILD_IOS_SIM_CACHE_MB MiB while it is shut down
# (lib/ios-sim-cache.sh). Each boot adds up to about 350 MB, so with the default of 1024 the two simulators keep less
# than 3 GB of it. PosterBoard writes it again at each boot: a boot after a removal took 11 s on the shared Mac. A
# failure here only warns: the run goes on.
bound_wallpaper_cache() {
  local devices
  if ! devices="$(simctl_bounded "$SIMCTL_SECONDS" list -j devices)" ||
    ! ios_sim_cache_bound "$1" "com.apple.CoreSimulator.SimRuntime.iOS-${IOS_SIMULATOR_RUNTIME//./-}" \
      "${BUILD_IOS_SIM_CACHE_MB:-1024}" <<<"$devices"; then
    echo "build-ios.sh: couldn't bound the wallpaper cache of simulator $1" >&2
  fi
}

# Run test plan $1 on the phone and tablet simulators, one after the other (or on --device's alone), into
# mobile/ios/build/$1.xcresult, with xcodebuild's options $2... (the UI plan's retry and shard), then count its tests
# (report_test_counts); its status is that of the runs. When the plan's build fails, no simulator boots, nothing is
# counted and the status is the build's.
#
# The plan is built once, before any simulator boots, and each simulator's run tests that build. Only one simulator runs
# at a time: each boots right before its own run, and shuts down before the next one boots; the last one shuts down
# before the function returns. A CI Mac has 3 CPUs and 7 GB: the unit job, with both simulators booted for its whole
# run, twice went silent until its job timeout (d0bd06ee, 9a986aa3), and a build beside a simulator in its first
# minutes took a UI leg to a load of 800, 1 GB of swap and a 97 s stall (ui-tablet-1 of ios run 44064367405). Each run
# writes its own result bundle, $1-<simulator>.xcresult (CI uploads it if the job ends before the merge);
# xcresulttool merges two into $1.xcresult, and one is moved there.
run_test_plan() {
  local plan="$1" name udid booted="" status=0 bundle bundles=() start
  shift
  mkdir -p "$BUILD_DIR"
  # a failed snapshot's image goes to snapshot-failures/<test file>/ (xcodebuild hands TEST_RUNNER_* to the test
  # processes without the prefix; the simulator's processes write the Mac's paths), its diffs into the result bundle
  export TEST_RUNNER_SNAPSHOT_ARTIFACTS="$BUILD_DIR/snapshot-failures"
  # the profiles of this plan's runs only: each simulator's are in Build/ProfileData/<its UDID>/, and a later run
  # leaves an earlier one's in place
  rm -rf "${BUILD_DIR:?}/$plan.xcresult" "${BUILD_DIR:?}/$plan-"*.xcresult "${DERIVED_DATA:?}/Build/ProfileData"
  build_test_plan "$plan" || return
  for name in "$IOS_SIM_PHONE_NAME" "$IOS_SIM_TABLET_NAME"; do
    case "$DEVICE" in
      phone) [[ "$name" == "$IOS_SIM_PHONE_NAME" ]] || continue ;;
      tablet) [[ "$name" == "$IOS_SIM_TABLET_NAME" ]] || continue ;;
    esac
    if [[ -n "$booted" ]]; then
      simctl_bounded "$SIMCTL_SECONDS" shutdown "$booted" ||
        die "couldn't shut down simulator $booted before the next one boots"
    fi
    bound_wallpaper_cache "$name"
    # found, made again when it is gone, and booted right before the run: a CI Mac's simulators can still come and go
    udid="$("$NATIVE_ROOT/scripts/setup-ios.sh" --simulator "$name")" ||
      die "the $name simulator on iOS $IOS_SIMULATOR_RUNTIME is not ready (see above)"
    booted="$udid"
    ios_sim_keyboard_as_new "$udid" || die "couldn't set simulator $udid's keyboard preferences"
    bundle="$BUILD_DIR/$plan-$name.xcresult"
    start=$SECONDS
    project_xcodebuild test-without-building -scheme NgTerm -configuration Debug -testPlan "$plan" \
      -destination "id=$udid" -parallel-testing-enabled NO -resultBundlePath "$bundle" "$@" -quiet || status=$?
    echo "build-ios.sh: the $plan plan ran on $name in $((SECONDS - start)) s" >&2
    [[ ! -d "$bundle" ]] || bundles+=("$bundle")
  done
  # the last one too: the next plan's run (test runs Unit, then UI) starts with its own first simulator. A failed
  # shutdown fails the plan, but the merge and the counts still come.
  if [[ -n "$booted" ]] && ! simctl_bounded "$SIMCTL_SECONDS" shutdown "$booted"; then
    echo "build-ios.sh: couldn't shut down simulator $booted after the $plan plan's runs" >&2
    ((status)) || status=1
  fi
  if ((${#bundles[@]} > 1)); then
    xcrun xcresulttool merge "${bundles[@]}" --output-path "$BUILD_DIR/$plan.xcresult" ||
      die "couldn't merge the $plan plan's result bundles (they stay beside it)"
  elif ((${#bundles[@]} == 1)); then
    mv "${bundles[0]}" "$BUILD_DIR/$plan.xcresult"
  fi
  rm -rf "${BUILD_DIR:?}/$plan-"*.xcresult
  report_test_counts "$plan" "$BUILD_DIR/$plan.xcresult"
  return "$status"
}

# Builds test plan $1 for testing, unless the products are its build from this tree already. The plans build into one
# set of products with other settings (Unit has code coverage, UI has not), so one record, $BUILD_DIR/built-tree, names
# the plan, BUILD_IOS_TREE_STAMP, the profile, the Xcode and the Generated/ files of the last build, and each other
# build removes it (project_xcodebuild, test-ios-e2e.sh). The tree stamp does not include Generated/: a core with
# other settings (--env RUSTFLAGS=..., another cached core) changes only Generated/, and step 7 writes a changed file
# again, which changes its fingerprint. The plan's test run file must still be in the derived data.
build_test_plan() {
  local plan="$1" built="$BUILD_DIR/built-tree" stamp start status=0
  stamp="$(test_build_record "$plan" "${BUILD_IOS_TREE_STAMP:-}" "$PROFILE" "${DEVELOPER_DIR:-}" \
    "$CORE_GENERATED" "$DESIGN_GENERATED")"
  if [[ -n "$stamp" && -f "$built" && "$(cat "$built")" == "$stamp" ]] &&
    compgen -G "$DERIVED_DATA/Build/Products/NgTerm_${plan}_iphonesimulator*.xctestrun" >/dev/null; then
    echo "build-ios.sh: the $plan plan was built from this tree already (BUILD_IOS_TREE_STAMP): no new build" >&2
    return 0
  fi
  start=$SECONDS
  echo "build-ios.sh: building the $plan plan" >&2
  project_xcodebuild build-for-testing "${SIMULATOR_BUILD[@]}" -testPlan "$plan" -quiet || {
    status=$?
    echo "build-ios.sh: the $plan plan's build failed" >&2
    return "$status"
  }
  echo "build-ios.sh: the $plan plan built in $((SECONDS - start)) s" >&2
  [[ -z "$stamp" ]] || echo "$stamp" >"$built"
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
  run_test_plan UI -retry-tests-on-failure -test-iterations 2 ${shard[@]+"${shard[@]}"} ${ONLY[@]+"${ONLY[@]}"} ||
    status=$?
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

# The smoke set of the UI plan: a small number of UI tests that show the main paths of the app on the fake core. CI
# runs them on each push and pull request, and the whole UI plan nightly and before a release
# (.github/workflows/ios.yml). Keep the set short: each test adds about a minute on each simulator of a CI run.
UI_SMOKE_TESTS=(
  # the app starts, Settings opens and shows the core version
  LaunchTests/testLaunchesOnTheFakeCore
  # the first run: the welcome, a new key, then the machines
  FirstRunTests/testCreatingAKeyLeadsToTheMachines
  # add a machine, trust its host key, and it connects
  FirstConnectTests/testAddMachineConfirmFingerprintConnected
  # open a session of a machine on the fake core, then type in it
  SessionKeyboardTests/testASessionOpenedByItsRowOrNewSessionTakesTheTypingAtOnce
)

# The smoke set as ui runs it. A test name that is not in mobile/ios/NgTermUITests stops the run: xcodebuild would run
# no test for it and pass.
action_smoke() {
  local test file
  local -a ONLY=()
  for test in "${UI_SMOKE_TESTS[@]}"; do
    if ! file="$(grep -lE "^[a-z ]*class ${test%/*}: XCTestCase" "$IOS_DIR"/NgTermUITests/*.swift)" ||
      ! grep -qE "func ${test#*/}\(" "$file"; then
      die "build-ios.sh: the smoke test $test is not in mobile/ios/NgTermUITests"
    fi
    ONLY+=("-only-testing:NgTermUITests/$test")
  done
  action_ui
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
  build="${BUILD_IOS_BUILD_NUMBER:-}"
  [[ -n "$build" ]] || build="$(git -C "$NATIVE_ROOT" rev-list --count HEAD)"
  [[ "$build" =~ ^[0-9]+$ ]] || die "build-ios.sh: the build number '$build' is not a number"
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
