#!/usr/bin/env bash
# Build ngterm's Windows installer, an MSI, from packaging/windows/ngterm.wxs (its header says what it installs and
# why per user) with the WiX Toolset at WIX_VERSION of build/pins.env. Runs on Windows under Git Bash: WiX writes the
# package through Windows Installer.
#
# The package is made of a staged tree (scripts/stage.sh's layout, which the installed folder keeps): with --staged
# that tree, else this script builds the release binaries (ngterm with its HTML overlays, ngtermctl, ngterm-session)
# and stages them first, which needs the libghostty-vt prefix (scripts/build-ghostty-vt.sh) and the terminal fonts
# (scripts/fetch-fonts.sh). The licenses and notices go under share\doc beside the staged files: ngterm's NOTICE,
# upstream's license of the agent-status package and skill, the terminal fonts' licenses.
#
# The version is the staged ngterm's own (the first line of share/ngterm/app-BUILD, its Cargo version), or --version;
# Windows Installer compares the first three fields, which a major upgrade needs to rise. The toasts' app id the
# Start-menu shortcut carries is the app's DEFAULT_APP_ID (crates/ngterm-gpui/src/instance.rs).
#
# The `wix` tool is installed once into the vendor root (dotnet tool install, which needs a .NET SDK); NGTERM_WIX names
# a `wix` of the pinned version to use instead (not WIX, which WiX 3 sets to its own folder).
#
# Usage: scripts/build-msi.sh OUTPUT.msi [--staged DIR] [--version X.Y.Z]
set -euo pipefail

# shellcheck source=lib/native.sh
source "$(dirname "$0")/lib/native.sh"

usage() { die "usage: scripts/build-msi.sh OUTPUT.msi [--staged DIR] [--version X.Y.Z]"; }

[[ "$(host_os)" == windows ]] || die "scripts/build-msi.sh runs on Windows (Git Bash): WiX writes MSIs through Windows Installer"
(($# >= 1)) || usage
OUTPUT="$1"
shift
STAGED=""
VERSION=""
while (($#)); do
  (($# >= 2)) || usage
  case "$1" in
    --staged) STAGED="$2" ;;
    --version) VERSION="$2" ;;
    *) usage ;;
  esac
  shift 2
done
[[ "$OUTPUT" == *.msi ]] || die "the output is a .msi: $OUTPUT"

# The pinned `wix`: NGTERM_WIX when given (checked against the pin), else the vendor root's, installed on first use.
wix_tool() {
  local tool="${NGTERM_WIX:-}" dir version
  if [[ -z "$tool" ]]; then
    dir="$VENDOR_ROOT/wix-$WIX_VERSION"
    tool="$dir/wix.exe"
    if [[ ! -x "$tool" ]]; then
      command -v dotnet >/dev/null || die "no .NET SDK (dotnet) to install the WiX Toolset $WIX_VERSION with"
      echo "→ installing the WiX Toolset $WIX_VERSION into $dir" >&2
      dotnet tool install wix --version "$WIX_VERSION" --tool-path "$(cygpath -w "$dir")" >&2 \
        || die "dotnet tool install wix $WIX_VERSION failed"
    fi
  fi
  version="$("$tool" --version)" || die "$tool --version failed"
  # `5.0.2+<commit>`
  [[ "${version%%+*}" == "$WIX_VERSION" ]] || die "$tool is WiX $version, the pin is $WIX_VERSION"
  echo "$tool"
}

# WiX's -arch for the binaries cargo builds on this host.
wix_arch() {
  local host
  host="$(rustc -vV | sed -n 's/^host: //p')"
  case "$host" in
    x86_64-pc-windows-*) echo x64 ;;
    aarch64-pc-windows-*) echo arm64 ;;
    *) die "no installer architecture for $host" ;;
  esac
}

# The folder of the Visual C++ runtime DLLs this host's MSVC links against, for the package's arch (the latest Visual
# Studio with the C++ tools, vswhere's; its default redistributable version).
vc_runtime_dir() {
  local vswhere install version dir
  vswhere="$(cygpath -u 'C:\Program Files (x86)')/Microsoft Visual Studio/Installer/vswhere.exe"
  [[ -x "$vswhere" ]] || die "no vswhere ($vswhere): the Visual C++ runtime comes from a Visual Studio with the C++ tools"
  install="$("$vswhere" -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 \
    -property installationPath | tr -d '\r')"
  [[ -n "$install" ]] || die "vswhere found no Visual Studio with the C++ tools"
  install="$(cygpath -u "$install")"
  version="$(tr -d '\r\n ' <"$install/VC/Auxiliary/Build/Microsoft.VCRedistVersion.default.txt")" \
    || die "no default redistributable version under $install"
  for dir in "$install/VC/Redist/MSVC/$version/$1"/Microsoft.VC*.CRT; do
    [[ -d "$dir" ]] && { echo "$dir"; return; }
  done
  die "no Visual C++ runtime for $1 under $install/VC/Redist/MSVC/$version"
}

# Copies into BIN the Visual C++ runtime DLLs its executables import (vcruntime140.dll, vcruntime140_1.dll, ...):
# the app is built against the dynamic runtime, which a fresh Windows does not carry, so the package deploys it beside
# the executables (Microsoft's app-local deployment; the redistributable files of the Visual Studio license). The
# Universal CRT (api-ms-win-crt-*) is part of Windows 10 and later.
bundle_vc_runtime() {
  local bin="$1" crt dll name
  crt="$(vc_runtime_dir "$(wix_arch)")"
  for dll in "$crt"/*.dll; do
    name="$(basename "$dll")"
    if grep -a -i -q -F "$name" "$bin"/*.exe; then
      cp "$dll" "$bin/$name"
      echo "→ bundling $name from $crt" >&2
    fi
  done
  grep -a -i -q -F vcruntime140.dll "$bin"/*.exe && [[ ! -f "$bin/vcruntime140.dll" ]] \
    && die "the executables import vcruntime140.dll, which $crt lacks"
  return 0
}

# DEFAULT_APP_ID of crates/ngterm-gpui/src/instance.rs.
app_id() {
  local id
  id="$(sed -n 's/^pub const DEFAULT_APP_ID: &str = "\([^"]*\)";$/\1/p' "$NATIVE_ROOT/crates/ngterm-gpui/src/instance.rs")"
  [[ -n "$id" ]] || die "no DEFAULT_APP_ID in crates/ngterm-gpui/src/instance.rs"
  echo "$id"
}

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

if [[ -z "$STAGED" ]]; then
  echo "→ building the release binaries" >&2
  cargo build --locked --release -p ngterm-gpui -p ngterm-ctl -p ngterm-session
  STAGED="$WORK/stage"
  "$NATIVE_ROOT/scripts/stage.sh" "$STAGED" --profile release
fi
[[ "$STAGED" == [A-Za-z]:[\\/]* ]] && STAGED="$(cygpath -u "$STAGED")"
for file in bin/ngterm.exe bin/ngtermctl.exe bin/ngterm-session.exe share/ngterm/app-BUILD; do
  [[ -f "$STAGED/$file" ]] || die "the staged tree $STAGED has no $file (scripts/stage.sh stages it)"
done

if [[ -z "$VERSION" ]]; then
  VERSION="$(head -n1 "$STAGED/share/ngterm/app-BUILD" | tr -d '\r')"
fi
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "an MSI's version is MAJOR.MINOR.PATCH, not '$VERSION' (give --version)"

# the package's root: the staged tree with the licenses and notices beside it, and the Visual C++ runtime beside the
# executables
ROOT="$WORK/root"
cp -R "$STAGED" "$ROOT"
FONTS="$(fonts_prefix)"
fonts_prefix_is_complete "$FONTS" || die "no complete terminal fonts prefix at $FONTS: run scripts/fetch-fonts.sh"
mkdir -p "$ROOT/share/doc/ngterm" "$ROOT/share/doc/fonts"
cp "$NATIVE_ROOT/NOTICE" "$ROOT/share/doc/ngterm/NOTICE.txt"
cp "$NATIVE_ROOT/resources/LICENSE.upstream" "$ROOT/share/doc/ngterm/LICENSE.upstream.txt"
cp "$FONTS/share/doc/fonts/"* "$ROOT/share/doc/fonts/"
bundle_vc_runtime "$ROOT/bin"

WIX_TOOL="$(wix_tool)"
mkdir -p "$(dirname "$OUTPUT")"
echo "→ building the ngterm $VERSION installer ($(wix_arch))" >&2
"$WIX_TOOL" build -nologo -arch "$(wix_arch)" -pdbtype none \
  -d "Version=$VERSION" -d "AppId=$(app_id)" \
  -d "Root=$(cygpath -w "$ROOT")" -d "Icon=$(cygpath -w "$NATIVE_ROOT/resources/icons/ngterm.ico")" \
  -o "$(cygpath -w "$OUTPUT")" "$(cygpath -w "$NATIVE_ROOT/packaging/windows/ngterm.wxs")"
# Windows Installer's internal consistency evaluators, but the three the package's header says do not apply per user
echo "→ validating the installer" >&2
"$WIX_TOOL" msi validate -nologo -sice ICE38 -sice ICE64 -sice ICE91 "$(cygpath -w "$OUTPUT")" \
  || die "the installer fails validation: $OUTPUT"
echo "→ $OUTPUT" >&2
