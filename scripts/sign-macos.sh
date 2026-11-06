#!/usr/bin/env bash
# Sign ngterm's macOS layouts under the hardened runtime, and check them.
#
# Every Mach-O but ngterm is a helper, signed first with no entitlements; ngterm, the responsible process for what its
# panes run, is signed last with the entitlements file (packaging/macos/ngterm.entitlements by default). The app is
# signed without --deep, so each nested Mach-O keeps the signature this script gave it. The identity is `-` (ad-hoc,
# the default, with no secure timestamp) or a codesigning identity's name or SHA-1 hash (with --timestamp).
#
#   scripts/sign-macos.sh bundle APP.app [--identity ID] [--entitlements FILE]
#     the app: every Mach-O in the bundle but Contents/MacOS/ngterm, then the app; then a --deep --strict verify;
#   scripts/sign-macos.sh loose BIN [--identity ID] [--entitlements FILE]
#     the archive's loose binaries: every Mach-O in BIN but BIN/ngterm, then BIN/ngterm;
#   scripts/sign-macos.sh check APP.app|BIN [--entitlements FILE]
#     the layout macos-verify checks: the hardened runtime on every Mach-O, no entitlements on a helper, and exactly
#     the file's entitlements on ngterm. It executes nothing it checks.
set -euo pipefail
# shellcheck source=lib/native.sh
source "$(dirname "$0")/lib/native.sh"
[[ "$(host_os)" == macos ]] || die "run this script on macOS"
usage() {
  die "usage: scripts/sign-macos.sh bundle APP.app | loose BIN | check APP.app|BIN [--identity ID] [--entitlements FILE]"
}
(($# >= 2)) || usage
command="$1"
target="$2"
shift 2
identity=-
entitlements="$NATIVE_ROOT/packaging/macos/ngterm.entitlements"
while (($#)); do
  (($# >= 2)) || usage
  case "$1" in
    --identity) identity="$2" ;;
    --entitlements) entitlements="$2" ;;
    *) usage ;;
  esac
  shift 2
done
[[ -f "$entitlements" ]] || die "no entitlements file: $entitlements"
[[ -d "$target" ]] || die "not a directory: $target"
case "$command" in
  bundle)
    [[ "$target" == *.app ]] || die "not an app bundle: $target"
    main="$target/Contents/MacOS/ngterm"
    ;;
  loose) main="$target/ngterm" ;;
  check)
    main="$target/ngterm"
    [[ "$target" != *.app ]] || main="$target/Contents/MacOS/ngterm"
    ;;
  *) usage ;;
esac
[[ -f "$main" ]] || die "no ngterm executable: $main"

# Every regular file under $target that is a Mach-O, NUL-separated (links are skipped: they are signed where they
# point, and a release tree holds none that leave it)
mach_o_files() {
  local file kind
  while IFS= read -r -d '' file; do
    kind="$(file -b "$file")"
    [[ "$kind" != Mach-O* ]] || printf '%s\0' "$file"
  done < <(find "$target" -type f -print0)
}

sign_options=(--force --options runtime --sign "$identity")
[[ "$identity" == - ]] || sign_options+=(--timestamp)

sign() {
  local helper count=0 file kind
  if [[ "$command" == bundle ]]; then
    # Contents/MacOS is code: --deep --strict refuses an unsigned file there, and a signature on a script lives in
    # extended attributes, which an archive drops
    while IFS= read -r -d '' file; do
      kind="$(file -b "$file")"
      [[ "$kind" == Mach-O* ]] || die "$file is not a Mach-O: only executables go in Contents/MacOS"
    done < <(find "$target/Contents/MacOS" -type f -print0)
  fi
  while IFS= read -r -d '' helper; do
    [[ "$helper" != "$main" ]] || continue
    codesign "${sign_options[@]}" "$helper"
    count=$((count + 1))
  done < <(mach_o_files)
  if [[ "$command" == bundle ]]; then
    codesign "${sign_options[@]}" --entitlements "$entitlements" "$target"
    codesign --verify --deep --strict "$target"
  else
    codesign "${sign_options[@]}" --entitlements "$entitlements" "$main"
    codesign --verify --strict "$main"
  fi
  echo "→ signed $target ($count helpers, identity $identity)" >&2
}

# check_entitlements FILE EXPECTED: FILE's signed entitlements equal EXPECTED's (a plist), or are none when EXPECTED is
# empty
check_entitlements() {
  local signed
  signed="$(codesign -d --entitlements - --xml "$1" 2>/dev/null)" || die "$1 is not signed"
  python3 -I - "$1" "$2" "$signed" <<'PY'
import plistlib
import sys

path, expected_file, signed = sys.argv[1:]
actual = plistlib.loads(signed.encode()) if signed.strip() else {}
expected = {}
if expected_file:
    with open(expected_file, "rb") as source:
        expected = plistlib.load(source)
if actual != expected:
    raise SystemExit(f"{path} has entitlements {sorted(actual)}, expected {sorted(expected)}")
PY
}

check() {
  local file details count=0
  while IFS= read -r -d '' file; do
    details="$(codesign -d --verbose=2 "$file" 2>&1)" || die "$file is not signed: $details"
    [[ "$details" =~ flags=0x[0-9a-f]+\([^\)]*runtime ]] || die "$file is signed without the hardened runtime"
    if [[ "$file" == "$main" ]]; then
      check_entitlements "$file" "$entitlements"
    else
      check_entitlements "$file" ""
    fi
    count=$((count + 1))
  done < <(mach_o_files)
  ((count > 0)) || die "no Mach-O under $target"
  echo "→ $target: $count Mach-O files, hardened runtime on each, entitlements on ngterm alone" >&2
}

if [[ "$command" == check ]]; then check; else sign; fi
