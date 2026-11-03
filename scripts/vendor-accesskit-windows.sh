#!/usr/bin/env bash
# Vendor accesskit_windows (AccessKit's UI Automation mapping, which gpui-pre-windows publishes the windows'
# accessibility trees through) into vendor/accesskit_windows with build/patches/accesskit_windows applied, for the
# workspace's [patch.crates-io]. Each file's leading description says why it exists and when it can go:
#   0001 a clickable node whose selected state UIA cannot express (no SelectionItem pattern for its role: a menu item's
#        highlight) keeps the Invoke pattern (docs/plans/20261031-e2e-ui-and-coverage.md Task 7)
#
# The source is the crates.io archive of ACCESSKIT_WINDOWS_VERSION, verified against ACCESSKIT_WINDOWS_SHA256 (the
# checksum Cargo.lock recorded for it before the patch), unpacked verbatim apart from its Cargo.lock (which pins
# nothing for a dependency), then patched. Bump both constants together with the accesskit_windows gpui-pre-windows pins,
# run this, and check that the patches still apply.
#
# Usage: scripts/vendor-accesskit-windows.sh           (re)create vendor/accesskit_windows
#        scripts/vendor-accesskit-windows.sh --check   rebuild into a scratch dir and fail if the vendored crate differs
#        scripts/vendor-accesskit-windows.sh --diff    regenerate the last patch,
#                                                    build/patches/accesskit_windows/NNNN-*.patch, as the
#                                                    vendored tree against the crate with the earlier patches applied
#                                                    (after editing vendor/; the patch's leading description is kept)
set -euo pipefail
shopt -s nullglob

ACCESSKIT_WINDOWS_VERSION="0.34.0"
ACCESSKIT_WINDOWS_SHA256="106c2b961215864d1c2e703ee63269c25c4e80a577ffb2c1017b9c17dcdf83a1"

root="$(cd "$(dirname "$0")/.." && pwd)"
target="$root/vendor/accesskit_windows"
patches="$root/build/patches/accesskit_windows"

die() {
  echo "$*" >&2
  exit 1
}

mode=vendor
case "${1:-}" in
  "") ;;
  --check) mode=check ;;
  --diff) mode=diff ;;
  -h | --help) sed -n '2,18s/^# \{0,1\}//p' "${BASH_SOURCE[0]}"; exit 0 ;;
  *) die "unknown argument: $1 (see --help)" ;;
esac

scratch="$(mktemp -d "${TMPDIR:-/tmp}/ngterm-accesskit_windows-XXXXXX")"
trap 'rm -rf "$scratch"' EXIT

archive="$scratch/accesskit_windows-$ACCESSKIT_WINDOWS_VERSION.crate"
curl -sSfL -A ngterm-dev -o "$archive" \
  "https://static.crates.io/crates/accesskit_windows/accesskit_windows-$ACCESSKIT_WINDOWS_VERSION.crate"
sum="$(sha256sum "$archive" | cut -d' ' -f1)"
[[ "$sum" == "$ACCESSKIT_WINDOWS_SHA256" ]] ||
  die "accesskit_windows $ACCESSKIT_WINDOWS_VERSION: sha256 $sum, expected $ACCESSKIT_WINDOWS_SHA256"
mkdir "$scratch/unpack"
tar -xzf "$archive" -C "$scratch/unpack"
pristine="$scratch/unpack/accesskit_windows-$ACCESSKIT_WINDOWS_VERSION"
rm -f "$pristine/Cargo.lock"

# The crate with its patches applied, in $scratch/built; with $1, only the patches before that one.
built() {
  local stop="${1:-}" patch
  cp -a "$pristine" "$scratch/built"
  for patch in "$patches"/*.patch; do
    [[ "$patch" == "$stop" ]] && break
    patch -s -p1 -d "$scratch/built" <"$patch" ||
      die "$(basename "$patch") does not apply to accesskit_windows $ACCESSKIT_WINDOWS_VERSION"
  done
}

case "$mode" in
  diff)
    all=("$patches"/*.patch)
    ((${#all[@]})) || die "no patch in $patches"
    patch="${all[-1]}"
    # $(...) drops the description's trailing blank lines; one blank line separates it from the diff
    description="$(sed -n '/^diff -ruN /q;/^--- a\//q;p' "$patch")"
    built "$patch"
    mkdir "$scratch/diff"
    mv "$scratch/built" "$scratch/diff/a"
    cp -a "$target" "$scratch/diff/b"
    {
      printf '%s\n\n' "$description"
      (cd "$scratch/diff" && diff -ruN a b | sed -E 's/^(---|\+\+\+) ([ab]\/[^\t]*)\t.*/\1 \2/') || true
    } >"$patch.new"
    mv "$patch.new" "$patch"
    echo "regenerated $patch"
    ;;
  check)
    built
    diff -r "$scratch/built" "$target" >&2 ||
      die "vendor/accesskit_windows differs from accesskit_windows $ACCESSKIT_WINDOWS_VERSION + build/patches/accesskit_windows"
    echo "vendor/accesskit_windows is accesskit_windows $ACCESSKIT_WINDOWS_VERSION + build/patches/accesskit_windows"
    ;;
  vendor)
    built
    mkdir -p "$root/vendor"
    rm -rf "$target.new"
    mv "$scratch/built" "$target.new"
    rm -rf "$target"
    mv "$target.new" "$target"
    echo "vendored accesskit_windows $ACCESSKIT_WINDOWS_VERSION into $target"
    ;;
esac
