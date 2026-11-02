#!/usr/bin/env bash
# Set up the Android toolchain the app's build (scripts/build-android.sh, mobile/android) and its tests need, at the
# ANDROID_* and CARGO_NDK_* pins of build/pins.env: the SDK's command-line tools, platform, build-tools, NDK,
# platform-tools, emulator and system image, two AVDs (a phone and a tablet), the Rust Android targets and cargo-ndk.
# It checks for JDK 21 and for sshd (the mobile e2e test) and only advises when they are missing: they are system
# packages.
#
# Idempotent: what is already in place is left alone, so a second run changes nothing. It ends with a report of what it
# did, what it could not do, and the lines to export; it exits 1 when something could not be done.
#
# The SDK root is ANDROID_HOME, else ANDROID_SDK_ROOT, else ~/Android/Sdk. Hosts: Linux and macOS (the command-line
# tools archive of the host is pinned); the AVDs need an x86_64 host, since the pinned system image is x86_64. Windows
# is left open: use Android Studio's SDK manager there with the same pins.
#
# Usage: scripts/setup-android.sh
set -euo pipefail

# shellcheck source=lib/native.sh
source "$(dirname "$0")/lib/native.sh" # sources build/pins.env too

DID=()
COULD_NOT=()
did() { DID+=("$*"); echo "did: $*" >&2; }
could_not() { COULD_NOT+=("$*"); echo "could not: $*" >&2; }

# JDK 21: JAVA_HOME, then java on PATH, then the usual install locations. Sets JAVA_HOME for the SDK tools.
java_major() {
  "$1" -version 2>&1 | sed -n 's/.*version "\([0-9]*\).*/\1/p' | head -n1
}
find_jdk21() {
  local candidate
  local candidates=()
  [[ -n "${JAVA_HOME:-}" ]] && candidates+=("$JAVA_HOME")
  if command -v java >/dev/null; then
    candidates+=("$(dirname "$(dirname "$(readlink -f "$(command -v java)")")")")
  fi
  candidates+=(/usr/lib/jvm/java-21-openjdk /usr/lib/jvm/java-21-openjdk-amd64 /usr/lib/jvm/java-21-openjdk-arm64
    /usr/lib/jvm/temurin-21-jdk /usr/lib/jvm/temurin-21-jdk-amd64)
  if [[ -x /usr/libexec/java_home ]]; then
    candidates+=("$(/usr/libexec/java_home -v 21 2>/dev/null || true)")
  fi
  for candidate in "${candidates[@]}"; do
    if [[ -n "$candidate" && -x "$candidate/bin/java" && -x "$candidate/bin/javac" ]] &&
      [[ "$(java_major "$candidate/bin/java")" == 21 ]]; then
      echo "$candidate"
      return 0
    fi
  done
  return 1
}

jdk_advice() {
  case "$(uname -s)" in
    Darwin) echo "brew install openjdk@21" ;;
    *)
      if command -v pacman >/dev/null; then
        echo "sudo pacman -S jdk21-openjdk"
      elif command -v apt-get >/dev/null; then
        echo "sudo apt-get install openjdk-21-jdk"
      elif command -v dnf >/dev/null; then
        echo "sudo dnf install java-21-openjdk-devel"
      else
        echo "install a JDK 21 and set JAVA_HOME"
      fi
      ;;
  esac
}

sshd_advice() {
  case "$(uname -s)" in
    Darwin) echo "macOS ships /usr/sbin/sshd" ;;
    *)
      if command -v pacman >/dev/null; then
        echo "sudo pacman -S openssh"
      elif command -v apt-get >/dev/null; then
        echo "sudo apt-get install openssh-server"
      elif command -v dnf >/dev/null; then
        echo "sudo dnf install openssh-server"
      else
        echo "install OpenSSH's server (sshd)"
      fi
      ;;
  esac
}

# The command-line tools archive and its sha256 for this host.
cmdline_tools_archive() {
  case "$(uname -s)-$(uname -m)" in
    Linux-*) echo "commandlinetools-linux-${ANDROID_CMDLINE_TOOLS_BUILD}_latest.zip $ANDROID_CMDLINE_TOOLS_SHA256_LINUX" ;;
    Darwin-x86_64)
      echo "commandlinetools-mac_x86_64-${ANDROID_CMDLINE_TOOLS_BUILD}_latest.zip $ANDROID_CMDLINE_TOOLS_SHA256_MACOS_X86_64"
      ;;
    Darwin-arm64)
      echo "commandlinetools-mac_arm64-${ANDROID_CMDLINE_TOOLS_BUILD}_latest.zip $ANDROID_CMDLINE_TOOLS_SHA256_MACOS_ARM64"
      ;;
    *) return 1 ;;
  esac
}

# Fetch the pinned command-line tools into $SDK/cmdline-tools/latest unless they are there.
ensure_cmdline_tools() {
  if [[ -x "$SDKMANAGER" ]]; then
    return 0
  fi
  local spec archive sha work
  spec="$(cmdline_tools_archive)" || {
    could_not "command-line tools: no pinned archive for $(uname -s) $(uname -m)"
    return 1
  }
  read -r archive sha <<<"$spec"
  for command in curl unzip; do
    command -v "$command" >/dev/null || {
      could_not "command-line tools: $command is required to fetch them"
      return 1
    }
  done
  mkdir -p "$SDK/cmdline-tools"
  work="$(mktemp -d "$SDK/cmdline-tools/.fetch.XXXXXX")"
  echo "fetching $archive..." >&2
  if ! curl -fsSL -A ngterm-dev -o "$work/$archive" "https://dl.google.com/android/repository/$archive"; then
    rm -rf "$work"
    could_not "command-line tools: download of $archive failed"
    return 1
  fi
  if ! sha256_matches "$sha" "$work/$archive"; then
    rm -rf "$work"
    could_not "command-line tools: $archive sha256 mismatch"
    return 1
  fi
  unzip -q "$work/$archive" -d "$work"
  rm -rf "$SDK/cmdline-tools/latest"
  mv "$work/cmdline-tools" "$SDK/cmdline-tools/latest"
  rm -rf "$work"
  did "command-line tools $ANDROID_CMDLINE_TOOLS_BUILD into $SDK/cmdline-tools/latest"
}

# The licenses directory's content, to tell whether accepting changed anything.
licenses_digest() {
  if [[ -d "$SDK/licenses" ]]; then
    cat "$SDK/licenses"/* 2>/dev/null | sha256_of
  fi
}

ensure_licenses() {
  local before
  before="$(licenses_digest)"
  # sdkmanager asks once per license; `yes` closes early once it stops reading
  if ! (yes 2>/dev/null || true) | "$SDKMANAGER" --sdk_root="$SDK" --licenses >/dev/null 2>&1; then
    could_not "licenses: sdkmanager --licenses failed"
    return 1
  fi
  if [[ "$(licenses_digest)" != "$before" ]]; then
    did "accepted the SDK licenses"
  fi
}

# An SDK package is installed when its directory holds a package.xml (sdkmanager writes it last).
package_dir() {
  echo "$SDK/${1//;//}"
}

ensure_package() {
  local package="$1"
  if [[ -f "$(package_dir "$package")/package.xml" ]]; then
    return 0
  fi
  echo "installing $package..." >&2
  if "$SDKMANAGER" --sdk_root="$SDK" --install "$package" >/dev/null; then
    did "installed $package"
  else
    could_not "install $package (sdkmanager failed)"
  fi
}

# Create AVD $1 on device profile $2 from the pinned system image unless it exists.
ensure_avd() {
  local name="$1" device="$2"
  if [[ -f "$AVD_HOME/$name.ini" ]]; then
    return 0
  fi
  if [[ "$(uname -m)" != x86_64 ]]; then
    could_not "AVD $name: the pinned system image is x86_64, this host is $(uname -m)"
    return 0
  fi
  if [[ ! -f "$(package_dir "$ANDROID_EMULATOR_IMAGE")/package.xml" ]]; then
    could_not "AVD $name: $ANDROID_EMULATOR_IMAGE is not installed"
    return 0
  fi
  if echo no | "$AVDMANAGER" create avd --name "$name" --package "$ANDROID_EMULATOR_IMAGE" --device "$device" \
    >/dev/null; then
    did "created AVD $name ($device)"
  else
    could_not "create AVD $name (avdmanager failed)"
  fi
}

ensure_rust_targets() {
  if ! command -v rustup >/dev/null; then
    could_not "Rust Android targets: rustup is missing"
    return 0
  fi
  # rustup picks the toolchain from the working directory: run it in the checkout, so the targets go to the
  # rust-toolchain.toml pin whatever directory the script is called from.
  local installed target
  installed="$(cd "$NATIVE_ROOT" && rustup target list --installed)"
  for target in aarch64-linux-android x86_64-linux-android; do
    if ! grep -qx "$target" <<<"$installed"; then
      if (cd "$NATIVE_ROOT" && rustup target add "$target") >/dev/null 2>&1; then
        did "rustup target add $target"
      else
        could_not "rustup target add $target"
      fi
    fi
  done
}

ensure_cargo_ndk() {
  if ! command -v cargo >/dev/null; then
    could_not "cargo-ndk: cargo is missing"
    return 0
  fi
  # `cargo ndk --version` prints `cargo-ndk X.Y.Z`
  if [[ "$(cargo ndk --version 2>/dev/null | awk '{print $2}')" == "$CARGO_NDK_VERSION" ]]; then
    return 0
  fi
  echo "installing cargo-ndk $CARGO_NDK_VERSION..." >&2
  if cargo install --locked cargo-ndk --version "$CARGO_NDK_VERSION" >/dev/null 2>&1; then
    did "cargo install cargo-ndk $CARGO_NDK_VERSION"
  else
    could_not "cargo install --locked cargo-ndk --version $CARGO_NDK_VERSION"
  fi
}

SDK="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Android/Sdk}}"
SDKMANAGER="$SDK/cmdline-tools/latest/bin/sdkmanager"
AVDMANAGER="$SDK/cmdline-tools/latest/bin/avdmanager"
AVD_HOME="${ANDROID_AVD_HOME:-${ANDROID_USER_HOME:-$HOME/.android}/avd}"

if JDK="$(find_jdk21)"; then
  export JAVA_HOME="$JDK"
else
  JDK=""
  could_not "JDK 21 not found (JAVA_HOME or PATH): $(jdk_advice)"
fi

if [[ ! -d "$SDK" ]]; then
  mkdir -p "$SDK"
  did "created the SDK root $SDK"
fi

# the SDK tools run on any JDK 17+, so they still run when only an older-than-21 one is around
if [[ -z "$JDK" ]] && ! command -v java >/dev/null; then
  could_not "SDK packages and AVDs: no java to run sdkmanager"
elif ensure_cmdline_tools && ensure_licenses; then
  for package in "platforms;android-$ANDROID_PLATFORM_VERSION" "build-tools;$ANDROID_BUILD_TOOLS_VERSION" \
    "ndk;$ANDROID_NDK_VERSION" platform-tools emulator "$ANDROID_EMULATOR_IMAGE"; do
    ensure_package "$package"
  done
  ensure_avd "$ANDROID_AVD_PHONE" "$ANDROID_AVD_PHONE_DEVICE"
  ensure_avd "$ANDROID_AVD_TABLET" "$ANDROID_AVD_TABLET_DEVICE"
fi

ensure_rust_targets
ensure_cargo_ndk

if ! command -v sshd >/dev/null && [[ ! -x /usr/sbin/sshd && ! -x /usr/bin/sshd ]]; then
  could_not "sshd not found (the mobile e2e test needs it): $(sshd_advice)"
fi

echo >&2
if ((${#DID[@]})); then
  echo "done:" >&2
  printf '  %s\n' "${DID[@]}" >&2
else
  echo "nothing to do: the Android toolchain is already in place" >&2
fi
if ((${#COULD_NOT[@]})); then
  echo "could not:" >&2
  printf '  %s\n' "${COULD_NOT[@]}" >&2
fi
echo >&2
echo "export these (shell profile or CI env):" >&2
[[ -n "$JDK" ]] && echo "export JAVA_HOME=\"$JDK\""
echo "export ANDROID_HOME=\"$SDK\""
echo "export ANDROID_NDK_HOME=\"$SDK/ndk/$ANDROID_NDK_VERSION\""

((${#COULD_NOT[@]} == 0))
