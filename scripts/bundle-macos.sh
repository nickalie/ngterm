#!/usr/bin/env bash
# Stage a standalone macOS application bundle, ad-hoc signed under the hardened runtime (scripts/sign-macos.sh) with
# the shipping entitlements, or for --profile debug the debug ones, which let a debugger attach.
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
# A debug-profile bundle lets a debugger attach; a staged tree is a release's
entitlements="$NATIVE_ROOT/packaging/macos/ngterm.entitlements"
[[ " ${stage_args[*]-} " != *" --profile debug "* ]] || entitlements="$NATIVE_ROOT/packaging/macos/ngterm-debug.entitlements"
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
<key>NSAppDataUsageDescription</key><string>Command-line tools you run inside ngterm (for example, docker talking to Docker Desktop) request access to other applications' data through ngterm.</string>
<key>NSAppleEventsUsageDescription</key><string>Command-line tools you run inside ngterm (for example, an osascript one-liner or a build script) request permission to control other apps through ngterm.</string>
<key>NSAudioCaptureUsageDescription</key><string>Command-line tools you run inside ngterm (for example, a system-audio capture or transcription tool) request permission to record system audio through ngterm.</string>
<key>NSBluetoothAlwaysUsageDescription</key><string>Command-line tools you run inside ngterm request Bluetooth access through ngterm.</string>
<key>NSCalendarsFullAccessUsageDescription</key><string>Command-line tools you run inside ngterm request access to your calendars through ngterm.</string>
<key>NSCalendarsUsageDescription</key><string>Command-line tools you run inside ngterm request access to your calendars through ngterm.</string>
<key>NSCalendarsWriteOnlyAccessUsageDescription</key><string>Command-line tools you run inside ngterm request permission to add events to your calendars through ngterm.</string>
<key>NSCameraUsageDescription</key><string>Command-line tools you run inside ngterm (for example, ffmpeg capturing video) request camera access through ngterm.</string>
<key>NSContactsUsageDescription</key><string>Command-line tools you run inside ngterm request access to your contacts through ngterm.</string>
<key>NSDesktopFolderUsageDescription</key><string>Command-line tools you run inside ngterm request access to your Desktop folder through ngterm.</string>
<key>NSDocumentsFolderUsageDescription</key><string>Command-line tools you run inside ngterm request access to your Documents folder through ngterm.</string>
<key>NSDownloadsFolderUsageDescription</key><string>Command-line tools you run inside ngterm request access to your Downloads folder through ngterm.</string>
<key>NSLocalNetworkUsageDescription</key><string>Command-line tools you run inside ngterm (for example, connecting to a .local host) request local network access through ngterm.</string>
<key>NSLocationUsageDescription</key><string>Command-line tools you run inside ngterm request your location through ngterm.</string>
<key>NSMicrophoneUsageDescription</key><string>Command-line tools you run inside ngterm (for example, voice dictation in Claude Code) request microphone access through ngterm.</string>
<key>NSNetworkVolumesUsageDescription</key><string>Command-line tools you run inside ngterm request access to files on network volumes through ngterm.</string>
<key>NSPhotoLibraryUsageDescription</key><string>Command-line tools you run inside ngterm (for example, osxphotos) request access to your photo library through ngterm.</string>
<key>NSRemindersFullAccessUsageDescription</key><string>Command-line tools you run inside ngterm request access to your reminders through ngterm.</string>
<key>NSRemindersUsageDescription</key><string>Command-line tools you run inside ngterm request access to your reminders through ngterm.</string>
<key>NSRemovableVolumesUsageDescription</key><string>Command-line tools you run inside ngterm request access to files on removable volumes through ngterm.</string>
<key>NSSpeechRecognitionUsageDescription</key><string>Command-line tools you run inside ngterm request speech recognition through ngterm.</string>
<key>NSSystemAdministrationUsageDescription</key><string>Command-line tools you run inside ngterm (for example, dscl editing a directory record) request permission to change system configuration through ngterm.</string>
<key>NSSupportsAutomaticGraphicsSwitching</key><true/>
</dict></plist>
EOF
# Signing changes helper executables; installed sessions must not trust a staged build-id stamp.
rm -f "$bundle/Contents/share/ngterm/session-BUILD"
plutil -lint "$bundle/Contents/Info.plist"
"$NATIVE_ROOT/scripts/sign-macos.sh" bundle "$bundle" --entitlements "$entitlements"
echo "→ bundled $bundle"
