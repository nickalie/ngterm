#!/usr/bin/env bash
# Check an iOS app bundle's privacy manifest (PrivacyInfo.xcprivacy) against what its executable uses, the scan behind
# the manifest's declarations (docs/ios-release.md > Privacy manifest):
#
# - the manifest is in the bundle (the Release archive's privacy report is made from it: the app's packages link
#   statically into its executable and carry no manifest of their own), declares no tracking, no tracking domains and
#   no collected data;
# - every required-reason API the executable uses (Apple's list, "Describing use of required reason API": the C
#   functions it imports, the Objective-C selectors it calls, the Foundation keys and classes it imports) has its
#   category declared, and every declared category is one the executable uses, with only reasons Apple approves for
#   that category (the third-party SDK wrapper reasons excepted: this is an app).
#
# An API Apple lists under two categories (getattrlist and its kin: file timestamps and disk space) is covered by
# either. The scan reads the main executable only, so a bundle with frameworks or plug-ins fails until it reads theirs.
#
# Usage: scripts/ios-privacy-check.sh <App.app>   (macOS: nm, otool and plutil)
set -euo pipefail

die() {
  echo "$*" >&2
  exit 1
}

(($# == 1)) || die "usage: scripts/ios-privacy-check.sh <App.app>"
APP="${1%/}"
[[ -d "$APP" ]] || die "ios-privacy-check: no app bundle at $APP"
MANIFEST="$APP/PrivacyInfo.xcprivacy"
[[ -f "$MANIFEST" ]] || die "ios-privacy-check: $APP has no PrivacyInfo.xcprivacy"
for nested in Frameworks PlugIns Extensions; do
  [[ ! -e "$APP/$nested" ]] ||
    die "ios-privacy-check: $APP has $nested/, whose executables this check doesn't read (it reads the main one only)"
done
EXECUTABLE="$APP/$(plutil -extract CFBundleExecutable raw -o - "$APP/Info.plist")"
[[ -f "$EXECUTABLE" ]] || die "ios-privacy-check: no executable at $EXECUTABLE"

# Apple's required-reason APIs: `categories kind name`, where categories joins the NSPrivacyAccessedAPICategory*
# suffixes the API counts for with `+`, and kind is how the executable shows its use: `import`, a symbol it imports
# (the C name with its leading underscore), or `selector`, an Objective-C method name it calls.
# shellcheck disable=SC2016 # the `$` in the names ($INODE64, OBJC_CLASS_$_) is literal
API_TABLE='
FileTimestamp import _stat
FileTimestamp import _fstat
FileTimestamp import _fstatat
FileTimestamp import _lstat
FileTimestamp import _stat64
FileTimestamp import _fstat64
FileTimestamp import _lstat64
FileTimestamp import _stat$INODE64
FileTimestamp import _fstat$INODE64
FileTimestamp import _fstatat$INODE64
FileTimestamp import _lstat$INODE64
FileTimestamp import _getattrlistbulk
FileTimestamp+DiskSpace import _getattrlist
FileTimestamp+DiskSpace import _fgetattrlist
FileTimestamp+DiskSpace import _getattrlistat
FileTimestamp import _NSFileCreationDate
FileTimestamp import _NSFileModificationDate
FileTimestamp import _NSURLCreationDateKey
FileTimestamp import _NSURLContentModificationDateKey
FileTimestamp selector fileCreationDate
FileTimestamp selector fileModificationDate
SystemBootTime import _mach_absolute_time
SystemBootTime selector systemUptime
DiskSpace import _statfs
DiskSpace import _statvfs
DiskSpace import _fstatfs
DiskSpace import _fstatvfs
DiskSpace import _statfs64
DiskSpace import _fstatfs64
DiskSpace import _statfs$INODE64
DiskSpace import _fstatfs$INODE64
DiskSpace import _NSFileSystemFreeSize
DiskSpace import _NSFileSystemSize
DiskSpace import _NSURLVolumeAvailableCapacityKey
DiskSpace import _NSURLVolumeAvailableCapacityForImportantUsageKey
DiskSpace import _NSURLVolumeAvailableCapacityForOpportunisticUsageKey
DiskSpace import _NSURLVolumeTotalCapacityKey
ActiveKeyboards selector activeInputModes
UserDefaults import _OBJC_CLASS_$_NSUserDefaults
UserDefaults selector standardUserDefaults
'

# The reasons Apple approves an app to declare, per category.
reasons_for() {
  case "$1" in
    FileTimestamp) echo "DDA9.1 C617.1 3B52.1" ;;
    SystemBootTime) echo "35F9.1 8FFB.1 3D61.1" ;;
    DiskSpace) echo "85F4.1 E174.1 7D9E.1 B728.1" ;;
    ActiveKeyboards) echo "3EC4.1 54BD.1" ;;
    UserDefaults) echo "CA92.1 1C8F.1 AC6B.1" ;;
    *) return 1 ;;
  esac
}

failures=()
fail() {
  failures+=("$*")
}

# --- the executable's use ---------------------------------------------------------------------------------------------

imports="$(nm -u "$EXECUTABLE")"
selectors="$(otool -v -s __TEXT __objc_methname "$EXECUTABLE" | sed -nE 's/^[0-9a-f]+  //p')"
# SwiftUI's @AppStorage reads and writes UserDefaults on the app's behalf
if grep -q '7SwiftUI10AppStorageV' <<<"$imports"; then
  API_TABLE="$API_TABLE"$'\nUserDefaults import @AppStorage'
  imports="$imports"$'\n@AppStorage'
fi

used=""       # `categories name` per API found
used_any=" "  # every category some found API counts for, space-separated
while read -r categories kind name; do
  [[ -n "${categories:-}" ]] || continue
  case "$kind" in
    import) grep -qxF -- "$name" <<<"$imports" || continue ;;
    selector) grep -qxF -- "$name" <<<"$selectors" || continue ;;
  esac
  used="$used$categories ${name#_}"$'\n'
  for category in ${categories//+/ }; do
    [[ "$used_any" == *" $category "* ]] || used_any="$used_any$category "
  done
done <<<"$API_TABLE"

# --- the manifest ------------------------------------------------------------------------------------------------------

value() {
  plutil -extract "$1" raw -o - "$MANIFEST" 2>/dev/null
}

[[ "$(value NSPrivacyTracking)" == false ]] || fail "NSPrivacyTracking is not false"
[[ "$(value NSPrivacyTrackingDomains)" == 0 ]] || fail "NSPrivacyTrackingDomains is not an empty list"
[[ "$(value NSPrivacyCollectedDataTypes)" == 0 ]] || fail "NSPrivacyCollectedDataTypes is not an empty list"

declared=" "
count="$(value NSPrivacyAccessedAPITypes)" || count=""
[[ "$count" =~ ^[0-9]+$ ]] || {
  fail "NSPrivacyAccessedAPITypes is not a list"
  count=0
}
for ((i = 0; i < count; i++)); do
  type="$(value "NSPrivacyAccessedAPITypes.$i.NSPrivacyAccessedAPIType")" || type=""
  category="${type#NSPrivacyAccessedAPICategory}"
  if [[ "$type" == "$category" ]] || ! approved="$(reasons_for "$category")"; then
    fail "NSPrivacyAccessedAPITypes[$i]: '$type' is not a required-reason API category"
    continue
  fi
  [[ "$declared" != *" $category "* ]] || fail "$type is declared twice"
  declared="$declared$category "
  reasons="$(value "NSPrivacyAccessedAPITypes.$i.NSPrivacyAccessedAPITypeReasons")" || reasons=""
  if ! [[ "$reasons" =~ ^[0-9]+$ ]] || ((reasons == 0)); then
    fail "$type has no NSPrivacyAccessedAPITypeReasons"
    continue
  fi
  for ((r = 0; r < reasons; r++)); do
    reason="$(value "NSPrivacyAccessedAPITypes.$i.NSPrivacyAccessedAPITypeReasons.$r")" || reason=""
    [[ " $approved " == *" $reason "* ]] || fail "$type: '$reason' is not a reason an app may declare ($approved)"
  done
  [[ "$used_any" == *" $category "* ]] ||
    fail "$type is declared, but $(basename "$EXECUTABLE") uses none of its APIs: remove it"
done

while read -r categories name; do
  [[ -n "${categories:-}" ]] || continue
  covered=0
  for category in ${categories//+/ }; do
    [[ "$declared" != *" $category "* ]] || covered=1
  done
  ((covered)) ||
    fail "$(basename "$EXECUTABLE") uses $name: declare NSPrivacyAccessedAPICategory${categories//+/ or } with its reason"
done <<<"$used"

if ((${#failures[@]})); then
  printf 'ios-privacy-check: %s\n' "${failures[@]}" >&2
  echo "ios-privacy-check: $MANIFEST doesn't match $(basename "$EXECUTABLE")" >&2
  exit 1
fi
echo "ios-privacy-check: $MANIFEST covers $(basename "$EXECUTABLE")'s required-reason APIs:" \
  "$(sed '/^$/d; s/ /: /' <<<"$used" | paste -sd, - | sed 's/,/, /g')" >&2
