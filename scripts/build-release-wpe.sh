#!/usr/bin/env bash
# Build the full HTML engine against Ubuntu 24.04, instead of redistributing Arch's
# glibc-2.44-only binary. The resulting prefix lives apart from the dev dependencies: releases pull it from GHCR
# (scripts/release-wpe-store.sh), keyed by --print-key.
set -euo pipefail
# shellcheck source=lib/native.sh
source "$(dirname "$0")/lib/native.sh"
# shellcheck source=../packaging/release/pins.env
source "$NATIVE_ROOT/packaging/release/pins.env"
key="$({
  grep -E '^WPE_(VERSION|API_VERSION)=' "$PINS"
  grep '^WPE_SOURCE_SHA256=' "$NATIVE_ROOT/packaging/release/pins.env"
  # the Ubuntu packages it is built and linked against, too
  cat "$0" "$NATIVE_ROOT/packaging/release/relocatable-wpe.patch" "$NATIVE_ROOT/scripts/release-linux-deps.sh"
} | sha256_of)"
if [[ "${1:-}" == --print-key ]]; then echo "$key"; exit 0; fi
(($# == 0)) || die "usage: scripts/build-release-wpe.sh [--print-key]"
PREFIX="${NGTERM_RELEASE_WPE:?set NGTERM_RELEASE_WPE to an absolute build prefix}"
[[ "$PREFIX" == /* ]] || die "NGTERM_RELEASE_WPE must be absolute"
manifest="$PREFIX/.release-pins"
if [[ -f "$manifest" && "$(cat "$manifest")" == "$key" ]]; then
  pkg-config --atleast-version="$WPE_API_VERSION" "$PREFIX/lib/pkgconfig/wpe-webkit-2.0.pc"
  exit 0
fi
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
curl --fail --location --retry 5 "https://wpewebkit.org/releases/wpewebkit-$WPE_VERSION.tar.xz" -o "$work/source.tar.xz"
echo "$WPE_SOURCE_SHA256  $work/source.tar.xz" | sha256sum --check
mkdir "$work/source"
tar -xf "$work/source.tar.xz" -C "$work/source" --strip-components=1
patch --fuzz=0 -d "$work/source" -p1 < "$NATIVE_ROOT/packaging/release/relocatable-wpe.patch"
cmake -S "$work/source" -B "$work/build" -G Ninja \
  -DPORT=WPE -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX="$PREFIX" \
  -DCMAKE_INSTALL_LIBDIR=lib -DCMAKE_INSTALL_LIBEXECDIR=lib \
  -DCMAKE_C_COMPILER=clang-18 -DCMAKE_CXX_COMPILER=clang++-18 \
  -DCMAKE_EXE_LINKER_FLAGS=-fuse-ld=lld -DCMAKE_SHARED_LINKER_FLAGS=-fuse-ld=lld \
  -DENABLE_DOCUMENTATION=OFF -DENABLE_INTROSPECTION=OFF -DENABLE_WEBDRIVER=OFF \
  -DENABLE_WPE_LEGACY_API=OFF -DUSE_SYSPROF_CAPTURE=OFF -DUSE_LIBBACKTRACE=OFF \
  -DUSE_SYSTEM_UNIFDEF=OFF -DENABLE_JOURNALD_LOG=OFF \
  -DENABLE_BUBBLEWRAP_SANDBOX=ON -DBWRAP_EXECUTABLE=/usr/bin/bwrap \
  -DDBUS_PROXY_EXECUTABLE=/usr/bin/xdg-dbus-proxy \
  -DMAX_LINK_JOBS=1 -DMAX_COMPILER_JOBS=2 -DENABLE_DEBUG_SYMBOLS=OFF
cmake --build "$work/build" --parallel 2
cmake --install "$work/build" --strip
mkdir -p "$PREFIX/share/doc/wpewebkit"
cp "$work/source/Source/WebCore/"LICENSE* "$PREFIX/share/doc/wpewebkit/"
cp "$work/source/Source/JavaScriptCore/COPYING.LIB" "$PREFIX/share/doc/wpewebkit/"
cp "$work/source/Source/ThirdParty/skia/LICENSE" "$PREFIX/share/doc/wpewebkit/LICENSE.skia"
echo "$key" > "$manifest"
