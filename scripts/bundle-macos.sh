#!/usr/bin/env bash
# Stage a standalone macOS application bundle; ad-hoc signed for local distribution.
#
# The bundle records the version and commit --version and --commit give (the release workflow's `version` job),
# else the staged tree's share/ngterm/app-BUILD: it never runs the bundled ngterm.
#
# Usage: scripts/bundle-macos.sh DEST.app [--profile debug|release | --staged DIR] [--version X.Y.Z] [--commit SHA]
set -euo pipefail
# shellcheck source=lib/native.sh
source "$(dirname "$0")/lib/native.sh"
[[ "$(host_os)" == macos ]] || die "run this script on macOS"
usage() {
  die "usage: scripts/bundle-macos.sh DEST.app [--profile debug|release | --staged DIR] [--version X.Y.Z] [--commit SHA]"
}
(($# >= 1)) || usage
bundle="$1"
shift
version=""
commit=""
staged=""
stage_args=()
while (($#)); do
  case "$1" in
    --version | --commit | --staged)
      (($# >= 2)) || usage
      declare "${1#--}=$2"
      shift 2
      ;;
    *)
      stage_args+=("$1")
      shift
      ;;
  esac
done
[[ -z "$staged" || ${#stage_args[@]} -eq 0 ]] || usage
[[ "$bundle" == *.app ]] || die "destination must end in .app"
[[ ! -e "$bundle" ]] || die "destination exists: $bundle"
work="$(mktemp -d "${TMPDIR:-/tmp}/ngterm-bundle.XXXXXX")"
trap 'rm -rf "$work"' EXIT
if [[ -n "$staged" ]]; then
  cp -R "$staged" "$work/root"
else
  "$NATIVE_ROOT/scripts/stage.sh" "$work/root" ${stage_args[@]+"${stage_args[@]}"}
fi
for binary in ngterm ngtermctl ngterm-session; do
  [[ -x "$work/root/bin/$binary" ]] || die "build $binary before bundling"
done
mkdir -p "$bundle/Contents/Resources"
mv "$work/root/bin" "$bundle/Contents/MacOS"
mv "$work/root/share" "$bundle/Contents/share"
cp "$NATIVE_ROOT/resources/icons/ngterm.icns" "$bundle/Contents/Resources/"
identity="$bundle/Contents/share/ngterm/app-BUILD"
[[ -f "$identity" ]] || die "the staged tree has no share/ngterm/app-BUILD"
[[ -n "$version" ]] || version="$(head -n1 "$identity" | tr -d '\r')"
[[ -n "$commit" ]] || commit="$(sed -n 2p "$identity" | tr -d '\r')"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "the bundle's version is MAJOR.MINOR.PATCH, not '$version'"
cat > "$bundle/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>com.nickalie.ngterm</string>
<key>CFBundleName</key><string>ngterm</string>
<key>CFBundleDisplayName</key><string>ngterm</string>
<key>CFBundleExecutable</key><string>ngterm</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleIconFile</key><string>ngterm.icns</string>
<key>CFBundleShortVersionString</key><string>$version</string>
<key>CFBundleVersion</key><string>$version</string>
<key>GitCommit</key><string>$commit</string>
<key>LSMinimumSystemVersion</key><string>14.0</string>
<key>NSAppTransportSecurity</key><dict>
<key>NSAllowsLocalNetworking</key><true/>
</dict>
<key>NSHighResolutionCapable</key><true/>
<key>NSSupportsAutomaticGraphicsSwitching</key><true/>
</dict></plist>
EOF
# Signing changes helper executables; installed sessions must not trust a staged build-id stamp.
rm -f "$bundle/Contents/share/ngterm/session-BUILD"
plutil -lint "$bundle/Contents/Info.plist"
codesign --force --deep --sign - "$bundle"
echo "→ bundled $bundle"
