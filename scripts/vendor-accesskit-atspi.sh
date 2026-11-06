#!/usr/bin/env bash
# Vendor accesskit_atspi_common (AccessKit's AT-SPI mapping, which gpui-pre-linux's accesskit_unix publishes the
# windows' accessibility trees through) into vendor/accesskit_atspi_common with build/patches/accesskit_atspi_common
# applied, for the workspace's [patch.crates-io]. Each file's leading description says why it exists and when it can
# go:
#   0001 a disabled node's state without Enabled and Sensitive whatever its role, ReadOnly only for a read-only node, no
#        Editable on a disabled field; a button that opens a menu (`has_popup`) as a push button menu with HasPopup
#        (docs/plans/20261031-e2e-ui-and-coverage.md Task 6)
#
# The source is the crates.io archive of ACCESSKIT_ATSPI_VERSION, verified against ACCESSKIT_ATSPI_SHA256 (the
# checksum Cargo.lock recorded for it before the patch), unpacked verbatim apart from its Cargo.lock (which pins
# nothing for a dependency), then patched. Bump both constants together with the accesskit_unix gpui-pre-linux pins,
# run this, and check that the patches still apply.
#
# Usage: scripts/vendor-accesskit-atspi.sh           (re)create vendor/accesskit_atspi_common
#        scripts/vendor-accesskit-atspi.sh --check   rebuild into a scratch dir and fail if the vendored crate differs
#        scripts/vendor-accesskit-atspi.sh --diff    regenerate the last patch,
#                                                    build/patches/accesskit_atspi_common/NNNN-*.patch, as the
#                                                    vendored tree against the crate with the earlier patches applied
#                                                    (after editing vendor/; the patch's leading description is kept)
set -euo pipefail
shopt -s nullglob

ACCESSKIT_ATSPI_VERSION="0.19.1"
ACCESSKIT_ATSPI_SHA256="023da0e5097f46df7092d5280b02efb9bbf8d93298daeced42652463e357d636"

root="$(cd "$(dirname "$0")/.." && pwd)"
target="$root/vendor/accesskit_atspi_common"
patches="$root/build/patches/accesskit_atspi_common"

die() {
  echo "$*" >&2
  exit 1
}

mode=vendor
case "${1:-}" in
  "") ;;
  --check) mode=check ;;
  --diff) mode='diff' ;;
  -h | --help) sed -n '2,20s/^# \{0,1\}//p' "${BASH_SOURCE[0]}"; exit 0 ;;
  *) die "unknown argument: $1 (see --help)" ;;
esac

scratch="$(mktemp -d "${TMPDIR:-/tmp}/ngterm-accesskit_atspi_common-XXXXXX")"
trap 'rm -rf "$scratch"' EXIT

archive="$scratch/accesskit_atspi_common-$ACCESSKIT_ATSPI_VERSION.crate"
curl -sSfL -A ngterm-dev -o "$archive" \
  "https://static.crates.io/crates/accesskit_atspi_common/accesskit_atspi_common-$ACCESSKIT_ATSPI_VERSION.crate"
sum="$(sha256sum "$archive" | cut -d' ' -f1)"
[[ "$sum" == "$ACCESSKIT_ATSPI_SHA256" ]] ||
  die "accesskit_atspi_common $ACCESSKIT_ATSPI_VERSION: sha256 $sum, expected $ACCESSKIT_ATSPI_SHA256"
mkdir "$scratch/unpack"
tar -xzf "$archive" -C "$scratch/unpack"
pristine="$scratch/unpack/accesskit_atspi_common-$ACCESSKIT_ATSPI_VERSION"
rm -f "$pristine/Cargo.lock"

# The crate with its patches applied, in $scratch/built; with $1, only the patches before that one.
built() {
  local stop="${1:-}" patch
  cp -a "$pristine" "$scratch/built"
  for patch in "$patches"/*.patch; do
    [[ "$patch" == "$stop" ]] && break
    patch -s -p1 -d "$scratch/built" <"$patch" ||
      die "$(basename "$patch") does not apply to accesskit_atspi_common $ACCESSKIT_ATSPI_VERSION"
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
      die "vendor/accesskit_atspi_common differs from accesskit_atspi_common $ACCESSKIT_ATSPI_VERSION + build/patches/accesskit_atspi_common"
    echo "vendor/accesskit_atspi_common is accesskit_atspi_common $ACCESSKIT_ATSPI_VERSION + build/patches/accesskit_atspi_common"
    ;;
  vendor)
    built
    mkdir -p "$root/vendor"
    rm -rf "$target.new"
    mv "$scratch/built" "$target.new"
    rm -rf "$target"
    mv "$target.new" "$target"
    echo "vendored accesskit_atspi_common $ACCESSKIT_ATSPI_VERSION into $target"
    ;;
esac
