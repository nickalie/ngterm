#!/usr/bin/env bash
# Build the Android app (mobile/android) over the mobile core (crates/ngterm-mobile):
#
# 1. check the toolchain (scripts/setup-android.sh --check, whose exports it takes: JAVA_HOME, ANDROID_HOME,
#    ANDROID_NDK_HOME) and prepare the libghostty-vt prefix (scripts/build-ghostty-vt.sh);
# 2. build libngterm_mobile.so for arm64-v8a and x86_64 at API 26 (the app's minSdk) through cargo-ndk, into the core
#    module's mobile/android/core/build/generated/jniLibs/<abi>/, which Gradle packs into the APK;
# 3. generate the Kotlin bindings with the crate's own `uniffi-bindgen` (feature `bindgen`) in library mode over the
#    built .so, into mobile/android/core/build/generated/uniffi/kotlin/;
# 4. run Gradle in mobile/android with the given tasks (default: assembleDebug).
#
# The clean tasks among them (`clean`, `:core:clean`, ...) run first, before step 2, since they delete what steps 2 and
# 3 generate; the others run in step 4, which is skipped when only clean tasks were given. Gradle options (`-...`) go
# to both runs.
#
# Everything generated lives under the core module's build/, so nothing of it is committed; `./gradlew clean` removes
# it, and a Gradle build without it fails and points here. Gradle and what it runs (the JVM and instrumented tests) get
# an environment without NGTERM_*, so no test can reach the ngterm the script was started from; cargo keeps the
# caller's NGTERM_* build overrides (NGTERM_VENDOR, NGTERM_GHOSTTY_VT_PREFIX).
#
# Usage: scripts/build-android.sh [--profile release|debug] [--no-gradle] [GRADLE_TASK...]
#   --profile    the cargo profile of the .so (default: release)
#   --no-gradle  stop after the bindings
set -euo pipefail

# shellcheck source=lib/native.sh
source "$(dirname "$0")/lib/native.sh"

usage() {
  die "usage: scripts/build-android.sh [--profile release|debug] [--no-gradle] [GRADLE_TASK...]"
}

PROFILE=release
GRADLE=1
TASKS=()
while (($#)); do
  case "$1" in
    --profile)
      (($# >= 2)) && [[ "$2" == debug || "$2" == release ]] || usage
      PROFILE="$2"
      shift 2
      ;;
    --no-gradle)
      GRADLE=0
      shift
      ;;
    -h | --help) usage ;;
    *)
      TASKS+=("$1")
      shift
      ;;
  esac
done
((${#TASKS[@]})) || TASKS=(assembleDebug)
CLEAN=()
BUILD=()
for task in "${TASKS[@]}"; do
  if [[ "$task" == -* ]]; then
    CLEAN+=("$task")
    BUILD+=("$task")
  elif [[ "$task" == clean || "$task" == *:clean ]]; then
    CLEAN+=("$task")
  else
    BUILD+=("$task")
  fi
done

cd "$NATIVE_ROOT"
exports="$(scripts/setup-android.sh --check)"
eval "$exports"

scripts/build-ghostty-vt.sh >/dev/null

ANDROID_DIR="$NATIVE_ROOT/mobile/android"

# whether the arguments name a task, not only options
has_task() {
  local arg
  for arg in "$@"; do
    [[ "$arg" == -* ]] || return 0
  done
  return 1
}

# runs Gradle in mobile/android, stripping NGTERM_* from what it starts: no build or test reaches the caller's ngterm
gradle() {
  local unset_args=() name
  while IFS= read -r name; do
    unset_args+=(-u "$name")
  done < <(env | sed -n 's/^\(NGTERM_[A-Za-z0-9_]*\)=.*/\1/p')
  (cd "$ANDROID_DIR" && env ${unset_args[@]+"${unset_args[@]}"} ./gradlew "$@")
}

if ((GRADLE)) && has_task ${CLEAN[@]+"${CLEAN[@]}"}; then
  gradle "${CLEAN[@]}"
fi
GENERATED="$ANDROID_DIR/core/build/generated"
JNI_LIBS="$GENERATED/jniLibs"
BINDINGS="$GENERATED/uniffi/kotlin"
release=()
[[ "$PROFILE" == debug ]] || release=(--release)

# the API level is the app's minSdk (mobile/android/gradle/libs.versions.toml), at least libghostty-vt's floor of 24
rm -rf "$JNI_LIBS"
cargo ndk -t arm64-v8a -t x86_64 -P 26 -o "$JNI_LIBS" build -p ngterm-mobile ${release[@]+"${release[@]}"}

# library mode reads the interface from the library's metadata, which is the same for every ABI
cargo build -p ngterm-mobile --features bindgen --bin uniffi-bindgen
TARGET="${CARGO_TARGET_DIR:-$NATIVE_ROOT/target}"
rm -rf "$BINDINGS"
"$TARGET/debug/uniffi-bindgen" generate --library "$JNI_LIBS/x86_64/libngterm_mobile.so" --language kotlin \
  --no-format --out-dir "$BINDINGS"

if ((GRADLE)) && has_task ${BUILD[@]+"${BUILD[@]}"}; then
  gradle "${BUILD[@]}"
fi
