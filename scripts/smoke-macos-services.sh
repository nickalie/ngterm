#!/usr/bin/env bash
# Exercise AppKit and WKWebView directly, including in VMs without a Metal GPU.
set -euo pipefail
source "$(dirname "$0")/lib/native.sh"
[[ "$(host_os)" == macos ]] || die "macOS required"
work="$(mktemp -d "${TMPDIR:-/tmp}/ngterm-services.XXXXXX")"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/NativeServices.app/Contents/MacOS"
cat > "$work/NativeServices.app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>com.nickalie.ngterm.services-smoke</string>
<key>CFBundleExecutable</key><string>smoke</string>
<key>CFBundleName</key><string>ngterm services smoke</string>
<key>CFBundlePackageType</key><string>APPL</string>
</dict></plist>
PLIST
xcrun clang -fobjc-arc -fblocks -mmacosx-version-min=14.0 \
  "$NATIVE_ROOT/crates/ngterm-gpui/src/macos.m" "$NATIVE_ROOT/scripts/tests/macos-services.m" \
  -framework Cocoa -framework Carbon -framework UserNotifications -framework Network -framework WebKit -framework Metal \
  -o "$work/NativeServices.app/Contents/MacOS/smoke"
# LaunchServices supplies the desktop session required by nil-target Edit actions, including over SSH.
open -W -n --stdout "$work/stdout" --stderr "$work/stderr" "$work/NativeServices.app"
cat "$work/stdout" "$work/stderr"
# open waits for exit but does not propagate the child status; only a completed test prints this marker.
grep -q '^PASS:' "$work/stdout"
