#!/usr/bin/env bash
# Stage a standalone macOS application bundle; ad-hoc signed for local distribution.
# Usage: scripts/bundle-macos.sh DEST.app [--profile debug|release]
set -euo pipefail
source "$(dirname "$0")/lib/native.sh"
[[ "$(host_os)" == macos ]] || die "run this script on macOS"
(($# >= 1)) || die "usage: scripts/bundle-macos.sh DEST.app [--profile debug|release]"
bundle="$1"
shift
[[ "$bundle" == *.app ]] || die "destination must end in .app"
[[ ! -e "$bundle" ]] || die "destination exists: $bundle"
stage="$(mktemp -d "${TMPDIR:-/tmp}/ngterm-bundle.XXXXXX")"
trap 'rm -rf "$stage"' EXIT
"$NATIVE_ROOT/scripts/stage.sh" "$stage/root" "$@"
for binary in ngterm ngtermctl ngterm-session; do
  [[ -x "$stage/root/bin/$binary" ]] || die "build $binary before bundling"
done
mkdir -p "$bundle/Contents/Resources"
mv "$stage/root/bin" "$bundle/Contents/MacOS"
mv "$stage/root/share" "$bundle/Contents/share"
cp "$NATIVE_ROOT/resources/icons/ngterm.icns" "$bundle/Contents/Resources/"
identity="$("$bundle/Contents/MacOS/ngterm" --build-identity)"
version="$(echo "$identity" | head -n1)"
commit="$(echo "$identity" | sed -n '2p')"
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
