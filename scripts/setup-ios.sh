#!/usr/bin/env bash
# Set up the iOS toolchain the app's build (mobile/ios, crates/ngterm-mobile) and its tests need, at the IOS_* and
# SWIFTLINT_* pins of build/pins.env: it finds the pinned Xcode, adds the Rust iOS targets, downloads the pinned
# simulator runtime (arm64 only) when it is missing, creates the phone and tablet simulators on it, installs SwiftLint
# from its release zip into the vendor root after a sha256 check, and makes sure the pinned Zig of the libghostty-vt
# build is there. It doesn't install Xcode: that needs an Apple ID, so a missing one is reported with where to get it.
#
# Idempotent: what is already in place is left alone, so a second run changes nothing. It ends with a report of what it
# did, what it could not do, and the lines to export; it exits 1 when something could not be done. macOS only: on any
# other host it refuses, since Xcode and the simulators exist nowhere else.
#
# The Xcode is the first of DEVELOPER_DIR, `xcode-select -p`, /Applications/Xcode_<pin>.app and
# /Applications/Xcode*.app whose `xcodebuild -version` is "Xcode <IOS_XCODE_VERSION>"; it is exported as DEVELOPER_DIR,
# so the selected Xcode (`xcode-select -s`, which needs sudo) stays what it was.
#
# Preparing a Mac (done on the user's M2, macOS 27.2 with Xcode 27.2 installed, 2026-11-03):
#   - rustup's toolchain is in ~/.cargo/bin, which a non-login ssh shell may not put on PATH: export it first;
#   - the iOS 27.0 simulator runtime is an about 8 GB download from Apple (no login needed), into
#     /Library/Developer/CoreSimulator; keep about 20 GB free for it and the builds;
#   - a host that can't reach ziglang.org can't fetch the pinned Zig: copy ziglang.org's
#     zig-aarch64-macos-<LIBGHOSTTY_VT_ZIG_VERSION>.tar.xz from a host that can, check it against
#     LIBGHOSTTY_VT_ZIG_SHA256_MACOS_AARCH64, and unpack it (--strip-components=1) into
#     ${NGTERM_VENDOR:-~/.cache/ngterm/vendor}/zig-aarch64-macos-<version>/, where scripts/build-ghostty-vt.sh finds
#     it;
#   - a simulator UI test and a snapshot test run there with the screen locked, from an ssh-started shell
#     (docs/spikes/20261103-ios-locked-screen/run.sh records the probe), so the Mac can gate them as CI does.
#
# Usage: scripts/setup-ios.sh           set up what is missing
#        scripts/setup-ios.sh --check   change nothing: when what a build needs (the pinned Xcode, the Rust iOS
#                                       targets, the pinned simulator runtime and SwiftLint) is in place, print the
#                                       DEVELOPER_DIR export (and the PATH one for a vendored SwiftLint) and exit 0,
#                                       else list what is missing and exit 1 (the iOS build evals the exports)
set -euo pipefail

CHECK=0
case "${1:-}" in
  "") ;;
  --check) CHECK=1 ;;
  *)
    echo "usage: scripts/setup-ios.sh [--check]" >&2
    exit 2
    ;;
esac

if [[ "$(uname -s)" != Darwin ]]; then
  echo "scripts/setup-ios.sh: the iOS toolchain needs macOS (Xcode and its simulators); this host is $(uname -s)" >&2
  exit 1
fi

# shellcheck source=lib/native.sh
source "$(dirname "$0")/lib/native.sh" # sources build/pins.env too

RUST_TARGETS=(aarch64-apple-ios aarch64-apple-ios-sim)
RUNTIME_ID="com.apple.CoreSimulator.SimRuntime.iOS-${IOS_SIMULATOR_RUNTIME//./-}"
SWIFTLINT_DIR="$VENDOR_ROOT/swiftlint-$SWIFTLINT_VERSION"
SWIFTLINT_URL="https://github.com/realm/SwiftLint/releases/download/$SWIFTLINT_VERSION/portable_swiftlint.zip"

DID=()
COULD_NOT=()
did() {
  DID+=("$*")
  echo "did: $*" >&2
}
could_not() {
  COULD_NOT+=("$*")
  echo "could not: $*" >&2
}

# The first line of `xcodebuild -version` under developer directory $1 ("Xcode 27.2").
xcode_version_of() {
  DEVELOPER_DIR="$1" xcodebuild -version 2>/dev/null | head -n1
}

# Print the developer directory of the pinned Xcode; fail, printing what was found instead, when there is none.
find_xcode() {
  local candidate found="" seen="" tried=""
  local candidates=("${DEVELOPER_DIR:-}" "$(xcode-select -p 2>/dev/null || true)"
    "/Applications/Xcode_$IOS_XCODE_VERSION.app/Contents/Developer")
  for candidate in /Applications/Xcode*.app/Contents/Developer; do
    candidates+=("$candidate")
  done
  for candidate in "${candidates[@]}"; do
    [[ -n "$candidate" && -d "$candidate" ]] || continue
    case "$seen" in *"|$candidate|"*) continue ;; esac
    seen="$seen|$candidate|"
    found="$(xcode_version_of "$candidate")"
    if [[ "$found" == "Xcode $IOS_XCODE_VERSION" ]]; then
      echo "$candidate"
      return 0
    fi
    tried="$tried${tried:+; }${found:-no Xcode} at $candidate"
  done
  echo "${tried:-no Xcode}"
  return 1
}

# Whether the pinned runtime is installed and available: `simctl list runtimes` prints it as
# "iOS 27.0 (27.0 - <build>) - com.apple.CoreSimulator.SimRuntime.iOS-27-0", with a trailing "(unavailable, ...)"
# when it can't be used.
runtime_installed() {
  xcrun simctl list runtimes 2>/dev/null | awk -v head="iOS $IOS_SIMULATOR_RUNTIME ($IOS_SIMULATOR_RUNTIME - " \
    -v tail=" - $RUNTIME_ID" '
      index($0, head) == 1 && substr($0, length($0) - length(tail) + 1) == tail { found = 1 }
      END { exit !found }'
}

installed_rust_targets() {
  if command -v rustup >/dev/null; then
    # rustup picks the toolchain from the working directory: run it in the checkout, so the rust-toolchain.toml pin
    # is the one asked whatever directory the script is called from
    (cd "$NATIVE_ROOT" && rustup target list --installed)
  fi
}

# The SwiftLint to use: the vendored one, else one on PATH. Prints its path; fails when there is none.
find_swiftlint() {
  if [[ -x "$SWIFTLINT_DIR/swiftlint" ]]; then
    echo "$SWIFTLINT_DIR/swiftlint"
  elif command -v swiftlint >/dev/null; then
    command -v swiftlint
  else
    return 1
  fi
}

swiftlint_version_of() {
  "$1" version 2>/dev/null | head -n1
}

XCODE_ADVICE="install Xcode $IOS_XCODE_VERSION (the App Store or developer.apple.com/download/applications), then run \
scripts/setup-ios.sh"

# --check: report what a build needs and is missing, change nothing.
check_build_toolchain() {
  local missing=() xcode target installed swiftlint version
  if xcode="$(find_xcode)"; then
    export DEVELOPER_DIR="$xcode"
  else
    missing+=("Xcode $IOS_XCODE_VERSION (found: $xcode): $XCODE_ADVICE")
    xcode=""
  fi
  # a probe that fails (a rustup that can't fetch the pinned toolchain, a swiftlint that won't run) reports its piece
  # as missing instead of ending the check under errexit
  installed="$(installed_rust_targets)" || installed=""
  for target in "${RUST_TARGETS[@]}"; do
    grep -qx "$target" <<<"$installed" || missing+=("Rust target $target (rustup target add $target)")
  done
  runtime_installed || missing+=("the iOS $IOS_SIMULATOR_RUNTIME simulator runtime ($RUNTIME_ID)")
  if swiftlint="$(find_swiftlint)"; then
    version="$(swiftlint_version_of "$swiftlint")" || version=""
    [[ "$version" == "$SWIFTLINT_VERSION" ]] ||
      missing+=("SwiftLint $SWIFTLINT_VERSION (found: ${version:-no version} at $swiftlint)")
  else
    missing+=("SwiftLint $SWIFTLINT_VERSION (not found in $SWIFTLINT_DIR or on PATH)")
  fi
  if ((${#missing[@]})); then
    echo "the iOS toolchain is incomplete (run scripts/setup-ios.sh):" >&2
    printf '  missing: %s\n' "${missing[@]}" >&2
    return 1
  fi
  echo "the iOS toolchain is in place (Xcode $IOS_XCODE_VERSION at $xcode)" >&2
  print_exports "$xcode" "$(vendored_swiftlint_dir)"
}

# The vendored SwiftLint's directory, for the PATH export, when it is there. A SwiftLint on PATH needs no export, and
# exporting its directory (/opt/homebrew/bin, say) would put everything else there ahead of rustup's cargo.
vendored_swiftlint_dir() {
  [[ ! -x "$SWIFTLINT_DIR/swiftlint" ]] || echo "$SWIFTLINT_DIR"
}

print_exports() {
  [[ -z "$1" ]] || echo "export DEVELOPER_DIR=\"$1\""
  [[ -z "$2" ]] || echo "export PATH=\"$2:\$PATH\""
}

if ((CHECK)); then
  check_build_toolchain
  exit
fi

ensure_rust_targets() {
  if ! command -v rustup >/dev/null; then
    could_not "Rust iOS targets: rustup is missing (put ~/.cargo/bin on PATH)"
    return 0
  fi
  local installed target
  installed="$(installed_rust_targets)" || installed=""
  for target in "${RUST_TARGETS[@]}"; do
    if ! grep -qx "$target" <<<"$installed"; then
      if (cd "$NATIVE_ROOT" && rustup target add "$target") >/dev/null 2>&1; then
        did "rustup target add $target"
      else
        could_not "rustup target add $target"
      fi
    fi
  done
}

ensure_runtime() {
  if runtime_installed; then
    return 0
  fi
  echo "downloading the iOS $IOS_SIMULATOR_RUNTIME simulator runtime (arm64, about 8 GB)..." >&2
  if xcodebuild -downloadPlatform iOS -buildVersion "$IOS_SIMULATOR_RUNTIME" -architectureVariant arm64 >&2 &&
    runtime_installed; then
    did "installed the iOS $IOS_SIMULATOR_RUNTIME simulator runtime"
  else
    could_not "install the iOS $IOS_SIMULATOR_RUNTIME simulator runtime (xcodebuild -downloadPlatform iOS)"
  fi
}

# Whether a simulator named $1 exists on the pinned runtime: `simctl list devices` lists it as "    <name> (<udid>)
# (<state>)" under the runtime's "-- iOS 27.0 --" header.
simulator_exists() {
  xcrun simctl list devices 2>/dev/null | awk -v header="-- iOS $IOS_SIMULATOR_RUNTIME --" -v name="    $1 (" '
    /^-- / { inside = ($0 == header); next }
    inside && index($0, name) == 1 { found = 1 }
    END { exit !found }'
}

# Create simulator $1 of device type $2 on the pinned runtime unless it exists.
ensure_simulator() {
  local name="$1" device="$2"
  if simulator_exists "$name"; then
    return 0
  fi
  if ! runtime_installed; then
    could_not "simulator $name: the iOS $IOS_SIMULATOR_RUNTIME runtime is not installed"
    return 0
  fi
  if xcrun simctl create "$name" "$device" "$RUNTIME_ID" >/dev/null; then
    did "created simulator $name ($device, iOS $IOS_SIMULATOR_RUNTIME)"
  else
    could_not "create simulator $name (xcrun simctl create failed)"
  fi
}

# Fetch the pinned portable_swiftlint.zip into the vendor root unless that SwiftLint is there.
ensure_swiftlint() {
  if [[ -x "$SWIFTLINT_DIR/swiftlint" && "$(swiftlint_version_of "$SWIFTLINT_DIR/swiftlint")" == "$SWIFTLINT_VERSION" ]]
  then
    return 0
  fi
  local command work
  for command in curl unzip; do
    command -v "$command" >/dev/null || {
      could_not "SwiftLint: $command is required to fetch it"
      return 0
    }
  done
  mkdir -p "$VENDOR_ROOT"
  work="$(mktemp -d "$VENDOR_ROOT/.swiftlint.XXXXXX")"
  echo "fetching SwiftLint $SWIFTLINT_VERSION..." >&2
  if ! curl -fsSL --retry 5 -o "$work/portable_swiftlint.zip" "$SWIFTLINT_URL"; then
    rm -rf "$work"
    could_not "SwiftLint: download of $SWIFTLINT_URL failed"
    return 0
  fi
  if ! sha256_matches "$SWIFTLINT_SHA256" "$work/portable_swiftlint.zip"; then
    rm -rf "$work"
    could_not "SwiftLint: portable_swiftlint.zip sha256 mismatch"
    return 0
  fi
  if ! unzip -q "$work/portable_swiftlint.zip" -d "$work/unpacked" || [[ ! -x "$work/unpacked/swiftlint" ]] ||
    [[ "$(swiftlint_version_of "$work/unpacked/swiftlint")" != "$SWIFTLINT_VERSION" ]]; then
    rm -rf "$work"
    could_not "SwiftLint: portable_swiftlint.zip holds no swiftlint $SWIFTLINT_VERSION"
    return 0
  fi
  rm -rf "$SWIFTLINT_DIR"
  mv "$work/unpacked" "$SWIFTLINT_DIR"
  rm -rf "$work"
  did "SwiftLint $SWIFTLINT_VERSION into $SWIFTLINT_DIR"
}

# The pinned Zig of the libghostty-vt build (scripts/build-ghostty-vt.sh fetches it too; here a host that can't reach
# ziglang.org learns it up front).
ensure_zig() {
  local sha
  if ! sha="$(zig_pin_sha256 LIBGHOSTTY_VT)" ||
    ! (find_zig "$LIBGHOSTTY_VT_ZIG_VERSION" "$sha" "${NGTERM_GHOSTTY_VT_ZIG:-}" >/dev/null); then
    could_not "zig $LIBGHOSTTY_VT_ZIG_VERSION: stage ziglang.org's archive into $VENDOR_ROOT (see this script's header)"
  fi
}

XCODE=""
if XCODE="$(find_xcode)"; then
  export DEVELOPER_DIR="$XCODE"
  ensure_runtime
  ensure_simulator "$IOS_SIM_PHONE_NAME" "$IOS_SIM_PHONE"
  ensure_simulator "$IOS_SIM_TABLET_NAME" "$IOS_SIM_TABLET"
else
  could_not "Xcode $IOS_XCODE_VERSION (found: $XCODE): $XCODE_ADVICE"
  XCODE=""
fi
ensure_rust_targets
ensure_swiftlint
ensure_zig

echo >&2
if ((${#DID[@]})); then
  echo "done:" >&2
  printf '  %s\n' "${DID[@]}" >&2
elif ((${#COULD_NOT[@]} == 0)); then
  echo "nothing to do: the iOS toolchain is already in place" >&2
fi
if ((${#COULD_NOT[@]})); then
  echo "could not:" >&2
  printf '  %s\n' "${COULD_NOT[@]}" >&2
fi
echo >&2
echo "export these (shell profile or CI env):" >&2
print_exports "$XCODE" "$(vendored_swiftlint_dir)"

((${#COULD_NOT[@]} == 0))
