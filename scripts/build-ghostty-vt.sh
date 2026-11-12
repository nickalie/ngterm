#!/usr/bin/env bash
# Prepare the pinned, patched libghostty-vt source (Ghostty's VT library behind its official C API,
# include/ghostty/vt.h) and Ghostty's resources at the same revision in the vendored prefix
# ${NGTERM_VENDOR:-~/.cache/ngterm/vendor}/ghostty-vt-<key>/. The library itself is built by
# ngterm-ghostty-vt-sys's build script (`zig build -Demit-lib-vt` over this prefix's tree, for cargo's target, into its
# OUT_DIR), which never reaches the network: everything that needs it or the shell happens here. The key hashes the
# LIBGHOSTTY_VT_ pins of build/pins.env (the one Ghostty pin), build/patches/ghostty-vt/, the prefix layout and the
# host arch, so another build's pin never moves it and a complete prefix is reused as is.
#
# The patches (build/patches/ghostty-vt/*.patch, applied in name order with no fuzz) add C API the app needs and the
# pin lacks, and the recorded fixes and parity behaviors; build/patches/ghostty-vt/README.md lists each with its reason,
# origin and remove-when condition. A patch change regenerates the bindings (scripts/gen-ghostty-vt-bindings.sh), like
# a pin bump.
#
# The prefix holds:
# - src/: the patched source tree, with the Zig packages a libghostty-vt build needs prefetched into its zig-pkg/ for
#   every target the build script maps (ZIG_TARGETS below; iOS's on a Mac with Xcode's iOS SDK alone, since they
#   configure nowhere else), so `zig build` there needs no network;
# - include/ghostty/: the headers, copied from the tree (what its install step installs), for the bindings generator;
# - share/doc/ghostty-vt/LICENSE, share/ngterm/ghostty-vt-PINS (the LIBGHOSTTY_VT_ pin lines it was prepared from);
# - share/ngterm/ghostty-vt-STAMP: its inputs as `name=value` lines, which the build script reads: the pinned zig it
#   was prepared with (`zig=`, the real binary, never a version manager's shim), that zig's version, the Zig global
#   cache (`zig-global-cache=`, scripts/lib/native.sh ghostty_vt_zig_cache), the version string the build passes
#   (`version-string=`, the tree's build.zig.zon version, so the build never asks git) and each patch with its sha256;
# - the resources scripts/stage.sh stages, as Ghostty's own build installs them (src/build/GhosttyResources.zig):
#   share/ghostty/shell-integration from the source, share/ghostty/themes from the theme package the revision's
#   build.zig.zon declares (LIBGHOSTTY_VT_THEMES_*, its sha256 checked), and share/terminfo: the xterm-ghostty entry
#   Ghostty's terminfo source (src/terminfo/ghostty.zig) encodes, as source (ghostty.terminfo) on every OS and compiled
#   with `tic` where it exists (Linux, macOS, Git Bash on Windows; the last two write the hashed layout, 78/, which
#   every reader takes beside x/).
#
# On Windows the native programs ignore terminfo; what a pane's TERM means there is up to what runs inside it: an ssh
# session forwards TERM to a host that needs the entry installed (`ngtermctl terminfo install`, as from Linux), and WSL
# starts its programs with its own TERM unless WSLENV forwards the pane's.
#
# The Zig packages come from the project's own mirror when it has them: the `zig-packages-<key>` prerelease that
# zig-packages.yml publishes, one tar of the package archives Zig keeps in its global cache, each named by its package
# hash, and of the pinned theme archive. `zig fetch` recomputes each package's hash from its contents, and a name it
# doesn't reproduce fails the script, so the mirror can't change what Ghostty's pinned build.zig.zon files ask for; the
# theme archive is checked against its pinned sha256. A key with no mirror, or one out of reach, is not an error: Zig
# then fetches each package from its upstream, every attempt bounded and retried.
#
# Usage: scripts/build-ghostty-vt.sh                 prepare the prefix unless it is complete, then print its path
#        scripts/build-ghostty-vt.sh --print-prefix  print the prefix path only (ngterm-ghostty-vt-sys/build.rs)
#        scripts/build-ghostty-vt.sh --verify        exit 0 when the prefix is complete, 1 otherwise
#        scripts/build-ghostty-vt.sh --print-zig-packages-key   print the key of the Zig packages' mirror
#        scripts/build-ghostty-vt.sh --print-zig-cache          print Zig's global cache, whose p/ the mirror holds
#
# It runs on Linux, stock macOS (bash 3.2, shasum, BSD patch) and under Git Bash on Windows (scripts/lib/native.sh >
# Portable shell); on Windows the stamp records native paths, which the build script hands to zig. It needs curl, tar,
# patch, sha256sum or shasum, and tic with infocmp where tic exists; find_zig needs xz (Linux, macOS) or unzip
# (Windows) only when it fetches zig. LIBGHOSTTY_VT_ZIG_VERSION comes from NGTERM_GHOSTTY_VT_ZIG, a matching zig on
# PATH, or the pinned ziglang.org archive of the host's OS and arch (LIBGHOSTTY_VT_ZIG_SHA256_<OS>_<ARCH>).
set -euo pipefail

# shellcheck source=lib/native.sh
source "$(dirname "$0")/lib/native.sh"

PREFIX="$(ghostty_vt_prefix)"
PATCHES="$NATIVE_ROOT/build/patches/ghostty-vt"
# The Zig triples of ngterm-ghostty-vt-sys's target map (crates/ngterm-ghostty-vt-sys/zig_build.rs ZIG_TARGETS, which a
# test keeps equal to this list), without their OS version suffixes: the packages each one's build needs are fetched.
ZIG_TARGETS=(x86_64-linux-gnu aarch64-linux-gnu x86_64-linux-musl aarch64-linux-musl x86_64-macos aarch64-macos
  x86_64-windows-msvc aarch64-windows-msvc aarch64-linux-android x86_64-linux-android
  aarch64-ios aarch64-ios-simulator)

# Whether Zig triple $1 configures on this host: any but an iOS one, which needs Xcode's SDK of it (iphoneos on device,
# iphonesimulator on the simulator) on a macOS host.
ios_sdk_present() {
  local sdk
  case "$1" in
    *-ios-simulator) sdk=iphonesimulator ;;
    *-ios) sdk=iphoneos ;;
    *) return 0 ;;
  esac
  [[ "$(host_os)" == macos ]] && xcrun --sdk "$sdk" --show-sdk-path >/dev/null 2>&1
}

# Where the mirror's releases are, and how long and how often one target's package fetch from the upstreams may run: a
# stalled download otherwise hangs for as long as the job may run. The environment overrides are for testing this.
ZIG_PACKAGES_RELEASES="${NGTERM_ZIG_PACKAGES_RELEASES:-https://github.com/nickalie/ngterm/releases/download}"
ZIG_FETCH_TIMEOUT="${NGTERM_ZIG_FETCH_TIMEOUT:-300}"
ZIG_FETCH_ATTEMPTS="${NGTERM_ZIG_FETCH_ATTEMPTS:-5}"
ZIG_FETCH_BACKOFF="${NGTERM_ZIG_FETCH_BACKOFF:-15}"

# The mirror's key: what decides which packages the build asks for (the pins, the patches, the targets configured), and
# nothing about the host, so every OS and arch reads the same release.
zig_packages_key() {
  local patch
  {
    echo "targets ${ZIG_TARGETS[*]}"
    pin_lines LIBGHOSTTY_VT
    for patch in "$PATCHES"/*.patch; do
      [[ -f "$patch" ]] || continue
      echo "patch $(basename "$patch")"
      cat "$patch"
    done
  } | sha256_of | cut -c1-16
}

case "${1:-}" in
  --print-prefix)
    echo "$PREFIX"
    exit 0
    ;;
  --print-zig-packages-key)
    zig_packages_key
    exit 0
    ;;
  --print-zig-cache)
    ghostty_vt_zig_cache
    exit 0
    ;;
  --verify)
    ghostty_vt_prefix_is_complete "$PREFIX"
    exit
    ;;
  "") ;;
  *)
    die "usage: scripts/build-ghostty-vt.sh [--print-prefix | --verify | --print-zig-packages-key" \
      "| --print-zig-cache]"
    ;;
esac

if ghostty_vt_prefix_is_complete "$PREFIX"; then
  echo "complete libghostty-vt prefix already vendored at $PREFIX" >&2
  echo "$PREFIX"
  exit 0
fi

for command in curl tar patch; do
  command -v "$command" >/dev/null || die "$command is required to fetch and patch the libghostty-vt source"
done
command -v sha256sum >/dev/null || command -v shasum >/dev/null \
  || die "sha256sum or shasum is required to check the pinned archives"
if command -v tic >/dev/null; then
  command -v infocmp >/dev/null || die "infocmp is required to check the xterm-ghostty entry tic compiles"
else
  echo "no tic on this host: Ghostty's terminfo is staged as source only (share/terminfo/ghostty.terminfo)" >&2
fi
mkdir -p "$VENDOR_ROOT"
ZIG_SHA="$(zig_pin_sha256 LIBGHOSTTY_VT)"
ZIG="$(find_zig "$LIBGHOSTTY_VT_ZIG_VERSION" "$ZIG_SHA" "${NGTERM_GHOSTTY_VT_ZIG:-}")"
# The binary itself, as zig reports it: a version manager's shim found on PATH may resolve to another version from the
# directory cargo runs the build script in. It is a ZON string, whose backslashes (a Windows path) are escaped; the
# stamp records it in the native form (native_path), as it records the global cache, since cargo's build script hands
# both to zig itself.
ZIG_EXE="$("$ZIG" env | sed -n 's/^ *\.zig_exe = "\(.*\)",$/\1/p' | sed 's/\\\\/\\/g')"
[[ -n "$ZIG_EXE" ]] || die "$ZIG does not report its own binary (zig env .zig_exe)"
ZIG_EXE="$(native_path "$ZIG_EXE")"
[[ -x "$ZIG_EXE" && "$("$ZIG_EXE" version)" == "$LIBGHOSTTY_VT_ZIG_VERSION" ]] \
  || die "$ZIG reports $ZIG_EXE as its binary (zig env .zig_exe), which is not zig $LIBGHOSTTY_VT_ZIG_VERSION"
ZIG_CACHE="$(ghostty_vt_zig_cache)"
mkdir -p "$ZIG_CACHE"
ZIG_CACHE="$(native_path "$ZIG_CACHE")"
# Zig 0.17 takes the global cache from the environment only (it dropped `--global-cache-dir`): every zig run below
# uses this one, never the user's own.
export ZIG_GLOBAL_CACHE_DIR="$ZIG_CACHE"

# both scratch dirs live in the vendor root, beside the final prefix, not in a shared /tmp
BUILD_DIR="$(mktemp -d "$VENDOR_ROOT/.ghostty-vt-build.XXXXXX")"
# the stage becomes the prefix in one atomic rename
STAGE="$(mktemp -d "$VENDOR_ROOT/.ghostty-vt-stage.XXXXXX")"
trap 'rm -rf "$BUILD_DIR" "$STAGE"' EXIT
TREE="$STAGE/src"

# The source archive of the full commit id, verified against its pinned sha256 before anything is unpacked: the
# build needs no git.
echo "fetching ghostty $LIBGHOSTTY_VT_REV for libghostty-vt..." >&2
curl -fsSL --retry 5 --connect-timeout 30 --max-time 600 -o "$BUILD_DIR/source.tar.gz" \
  "$LIBGHOSTTY_VT_REPO/archive/$LIBGHOSTTY_VT_REV.tar.gz"
sha256_matches "$LIBGHOSTTY_VT_SHA256" "$BUILD_DIR/source.tar.gz" \
  || die "ghostty $LIBGHOSTTY_VT_REV source archive sha256 mismatch"
mkdir "$TREE"
tar -xzf "$BUILD_DIR/source.tar.gz" -C "$TREE" --strip-components=1

# Short options GNU patch (Linux, Git Bash) and BSD patch (macOS) both take: -t batch, -N forward, -F 0 no fuzz,
# -s quiet. Neither has a portable way to keep GNU's backup of a hunk applied at an offset (BSD makes none), so the
# backups are removed after: the pristine tree holds no .orig file.
for patch in "$PATCHES"/*.patch; do
  [[ -f "$patch" ]] || continue
  echo "applying $(basename "$patch")..." >&2
  patch -d "$TREE" -p1 -t -N -F 0 -s <"$patch" \
    || die "$(basename "$patch") does not apply to ghostty $LIBGHOSTTY_VT_REV"
done
find "$TREE" -name '*.orig' -type f -exec rm -f {} +

# Puts the mirror's packages into Zig's global cache, each checked against its name. See the header for why a missing
# mirror is fine and a wrong package is not. The check fetches into a scratch cache and the archive is copied as is:
# Zig 0.17's `zig fetch` of a cache archive caches it wrapped in one more directory, which the build then hashes wrong.
seed_zig_packages() {
  local key archive package name fetched check partial
  key="$(zig_packages_key)"
  archive="$BUILD_DIR/zig-packages.tar"
  if ! curl -fsSL --retry 3 --connect-timeout 20 --max-time 120 -o "$archive" \
    "$ZIG_PACKAGES_RELEASES/zig-packages-$key/zig-packages.tar"; then
    echo "no mirror of libghostty-vt's Zig packages for $key: fetching them from their upstreams" >&2
    return 0
  fi
  echo "seeding libghostty-vt's Zig packages from the mirror $key..." >&2
  mkdir "$BUILD_DIR/zig-packages"
  tar --no-same-owner -xf "$archive" -C "$BUILD_DIR/zig-packages" || die "the Zig package mirror $key does not unpack"
  mkdir -p "$BUILD_DIR/zig-packages-check" "$ZIG_CACHE/p"
  check="$(native_path "$BUILD_DIR/zig-packages-check")"
  for package in "$BUILD_DIR"/zig-packages/*.tar.gz; do
    [[ -f "$package" ]] || die "the Zig package mirror $key holds no packages"
    name="$(basename "$package" .tar.gz)"
    fetched="$(cd "$TREE" && ZIG_GLOBAL_CACHE_DIR="$check" "$ZIG_EXE" fetch "$(native_path "$package")")" \
      || die "zig cannot read $name from the Zig package mirror $key"
    [[ "$fetched" == "$name" ]] || die "the Zig package mirror $key holds $fetched under the name $name"
    # The cache is shared by every worktree: a copy cut short must never sit under the name zig looks up.
    partial="$(mktemp "$ZIG_CACHE/p/.$name.XXXXXX")"
    if ! cp "$package" "$partial" || ! mv -f "$partial" "$ZIG_CACHE/p/$name.tar.gz"; then
      rm -f "$partial"
      die "cannot seed $name into the Zig cache $ZIG_CACHE"
    fi
  done
}

# Runs "$@" in the background and ends it after $1 seconds, returning its status. Stock macOS has no timeout(1). The
# watchdog sleeps a second at a time so that stopping it leaves no long sleep behind.
run_bounded() {
  local limit="$1" pid watchdog status=0
  shift
  "$@" &
  pid=$!
  (
    elapsed=0
    while ((elapsed < limit)); do
      sleep 1
      elapsed=$((elapsed + 1))
    done
    echo "still running after ${limit}s: stopping it" >&2
    kill -TERM "$pid" 2>/dev/null || exit 0
    sleep 5
    kill -KILL "$pid" 2>/dev/null || true
  ) >/dev/null &
  watchdog=$!
  wait "$pid" || status=$?
  kill "$watchdog" 2>/dev/null || true
  wait "$watchdog" 2>/dev/null || true
  return "$status"
}

# Configures the tree for Zig triple $1, which fetches what that target needs. It replaces its own process with zig, so
# that run_bounded's signal reaches zig itself.
configure_for_target() {
  cd "$TREE"
  export ANDROID_NDK_HOME
  ANDROID_NDK_HOME="$(native_path "$BUILD_DIR/android-ndk")"
  exec "$ZIG_EXE" build --list-steps --cache-dir "$BUILD_DIR/zig-cache" -Demit-lib-vt=true -Dtarget="$1" >/dev/null
}

# The packages: Zig fetches a lazy dependency only when a build's configure step asks for it, so each mapped target's
# libghostty-vt configuration is run once (listing its steps builds nothing), which fetches what that target's build
# needs into the tree's zig-pkg/. The local cache is scratch: the tree is left as the build script will read it.
# An Android configuration needs an NDK to exist (Ghostty's pkg/android-ndk looks one up), though it only joins paths
# into it: an empty directory stands in, so the prefix is prepared the same with or without an NDK installed.
# An iOS configuration resolves its libc from Xcode's SDK of it (Ghostty's pkg/apple-sdk, through `xcrun --sdk`), which
# Zig doesn't bundle, so it runs only on a macOS host whose Xcode has that SDK (ios_sdk_present) and is skipped
# elsewhere: Linux, Windows, or a Mac with the command-line tools alone. Its packages are the macOS configurations' (no
# lazy dependency of libghostty-vt's build is iOS-only), so a prefix prepared without it still builds for iOS.
seed_zig_packages
echo "fetching libghostty-vt's Zig packages with zig $LIBGHOSTTY_VT_ZIG_VERSION..." >&2
mkdir "$BUILD_DIR/android-ndk"
for target in "${ZIG_TARGETS[@]}"; do
  if ! ios_sdk_present "$target"; then
    echo "skipping $target: its configuration needs Xcode's iOS SDK, which this host lacks" >&2
    continue
  fi
  attempt=1
  until run_bounded "$ZIG_FETCH_TIMEOUT" configure_for_target "$target"; do
    ((attempt < ZIG_FETCH_ATTEMPTS)) \
      || die "cannot fetch the Zig packages of libghostty-vt for $target after $attempt attempts"
    echo "fetching the Zig packages for $target failed (attempt $attempt of $ZIG_FETCH_ATTEMPTS), retrying..." >&2
    sleep $((attempt * ZIG_FETCH_BACKOFF))
    attempt=$((attempt + 1))
  done
done
[[ -d "$TREE/zig-pkg" ]] || die "the libghostty-vt configuration fetched no Zig packages"

VERSION_STRING="$(sed -n 's/^ *\.version = "\([^"]*\)",$/\1/p' "$TREE/build.zig.zon" | head -n1)"
[[ -n "$VERSION_STRING" ]] || die "no .version in ghostty $LIBGHOSTTY_VT_REV's build.zig.zon"
mkdir -p "$STAGE/include" "$STAGE/share/doc/ghostty-vt" "$STAGE/share/ngterm"
cp -R "$TREE/include/ghostty" "$STAGE/include/ghostty"
cp "$TREE/LICENSE" "$STAGE/share/doc/ghostty-vt/LICENSE"
pin_lines LIBGHOSTTY_VT >"$STAGE/share/ngterm/ghostty-vt-PINS"

# The resources (the header names them). The vt build installs none of them, so they are taken as Ghostty's full build
# takes them: the shell integration as is, the themes from the package its build.zig.zon pins, the terminfo encoded by
# the revision's own Zig and compiled here.
mkdir -p "$STAGE/share/ghostty/themes"
cp -R "$TREE/src/shell-integration" "$STAGE/share/ghostty/shell-integration"
grep -F "$LIBGHOSTTY_VT_THEMES_URL" "$TREE/build.zig.zon" >/dev/null \
  || die "ghostty $LIBGHOSTTY_VT_REV no longer declares the pinned theme package (LIBGHOSTTY_VT_THEMES_URL)"
echo "fetching the pinned Ghostty theme package..." >&2
if [[ -f "$BUILD_DIR/zig-packages/ghostty-themes.tgz" ]] \
  && sha256_matches "$LIBGHOSTTY_VT_THEMES_SHA256" "$BUILD_DIR/zig-packages/ghostty-themes.tgz"; then
  mv "$BUILD_DIR/zig-packages/ghostty-themes.tgz" "$BUILD_DIR/ghostty-themes.tgz"
else
  curl -fsSL --retry 5 --connect-timeout 30 --max-time 600 -o "$BUILD_DIR/ghostty-themes.tgz" \
    "$LIBGHOSTTY_VT_THEMES_URL"
fi
sha256_matches "$LIBGHOSTTY_VT_THEMES_SHA256" "$BUILD_DIR/ghostty-themes.tgz" \
  || die "Ghostty theme package sha256 mismatch"
tar -xzf "$BUILD_DIR/ghostty-themes.tgz" -C "$STAGE/share/ghostty/themes" --strip-components=1
pin_lines LIBGHOSTTY_VT >"$STAGE/share/ghostty/themes/.ngterm-resource-manifest"

# What Ghostty's build-data tool runs for `+terminfo` (src/main_build_data.zig), alone: the entry's encoder needs
# nothing but std, so this needs none of the build's dependencies. The helper sits in the tree only while it runs.
cat >"$TREE/src/ngterm_terminfo.zig" <<'ZIG'
const std = @import("std");

pub fn main(init: std.process.Init) !void {
    var buffer: [1024]u8 = undefined;
    var stdout_writer = std.Io.File.stdout().writerStreaming(init.io, &buffer);
    try @import("terminfo/ghostty.zig").ghostty.encode(&stdout_writer.interface);
    try stdout_writer.end();
}
ZIG
echo "encoding Ghostty's terminfo..." >&2
(
  cd "$TREE"
  "$ZIG_EXE" run --cache-dir "$BUILD_DIR/zig-cache" src/ngterm_terminfo.zig
) >"$BUILD_DIR/ghostty.terminfo"
rm "$TREE/src/ngterm_terminfo.zig"
[[ -s "$BUILD_DIR/ghostty.terminfo" ]] || die "Ghostty's terminfo encoder wrote nothing"
mkdir -p "$STAGE/share/terminfo"
cp "$BUILD_DIR/ghostty.terminfo" "$STAGE/share/terminfo/ghostty.terminfo"
if command -v tic >/dev/null; then
  tic -x -o "$STAGE/share/terminfo" "$BUILD_DIR/ghostty.terminfo"
fi

{
  echo "# The inputs of this libghostty-vt prefix (scripts/build-ghostty-vt.sh); ngterm-ghostty-vt-sys builds from it."
  echo "zig=$ZIG_EXE"
  echo "zig-version=$LIBGHOSTTY_VT_ZIG_VERSION"
  echo "zig-global-cache=$ZIG_CACHE"
  echo "version-string=$VERSION_STRING"
  for patch in "$PATCHES"/*.patch; do
    [[ -f "$patch" ]] || continue
    echo "patch=$(basename "$patch") $(sha256_of <"$patch")"
  done
} >"$STAGE/share/ngterm/ghostty-vt-STAMP"

ghostty_vt_prefix_is_complete "$STAGE" || die "the staged libghostty-vt prefix is incomplete"
chmod 755 "$STAGE"
rm -rf "$PREFIX"
mv "$STAGE" "$PREFIX"
echo "→ vendored the libghostty-vt source into $PREFIX" >&2
echo "$PREFIX"
