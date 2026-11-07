#!/usr/bin/env bash
# Check how build/patches/ghostty-vt/*.patch apply to a Ghostty revision, as scripts/build-ghostty-vt.sh applies them
# (in name order, `patch -p1 --fuzz=0`), each onto the tree the earlier ones left.
#
#   patches-check.sh [--ghostty DIR] [--rebase] NEW_REV
#
# Only the files the patches touch are taken from the clone (default <repo>/.upstream/ghostty). Prints one line per
# patch:
#   clean      applies as is
#   offset     applies, at other line numbers (--rebase rewrites it onto NEW_REV, no fuzz needed either way)
#   upstreamed does not apply, but reverses cleanly: NEW_REV already has its change. Drop it only as
#              build/patches/ghostty-vt/README.md says: once its verification passes without it
#   conflict   neither: rebase it by hand, then re-run its verification (the failing hunks follow the line)
# With --rebase, each `offset` patch is rewritten in place: its header (everything before the first `--- a/`) kept,
# its hunks regenerated with `diff -u` against NEW_REV. The last line is PATCHES=clean, or PATCHES=work with the
# patches that need a hand.
set -euo pipefail

root="$(git rev-parse --show-toplevel)"
clone="$root/.upstream/ghostty"
rebase=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --ghostty) clone="$2"; shift 2 ;;
    --rebase) rebase=1; shift ;;
    -*) echo "patches-check: unknown option $1" >&2; exit 1 ;;
    *) break ;;
  esac
done
[[ $# -eq 1 ]] || { echo "usage: patches-check.sh [--ghostty DIR] [--rebase] NEW_REV" >&2; exit 1; }
rev="$(git -C "$clone" rev-parse "$1^{commit}")"
dir="$root/build/patches/ghostty-vt"

work="$(mktemp -d "${TMPDIR:-/tmp}/ngterm-patches-check.XXXXXX")"
trap 'rm -rf "$work"' EXIT
tree="$work/tree"      # NEW_REV plus every patch applied so far
mkdir -p "$tree"

targets() { sed -nE 's#^\+\+\+ b/([^[:space:]]+).*#\1#p' "$1"; }

# take every file any patch touches from NEW_REV, once
for p in "$dir"/*.patch; do
  for f in $(targets "$p"); do
    [[ -e "$tree/$f" ]] && continue
    mkdir -p "$(dirname "$tree/$f")"
    git -C "$clone" show "$rev:$f" >"$tree/$f" 2>/dev/null || rm -f "$tree/$f"
  done
done

work_needed=()
for p in "$dir"/*.patch; do
  name="$(basename "$p")"
  missing=()
  for f in $(targets "$p"); do [[ -e "$tree/$f" ]] || missing+=("$f"); done
  if ((${#missing[@]})); then
    echo "conflict   $name: gone at ${rev:0:9}: ${missing[*]}"
    work_needed+=("$name")
    continue
  fi
  if out="$(patch -p1 --fuzz=0 --dry-run -d "$tree" <"$p" 2>&1)"; then
    if grep -q "offset" <<<"$out"; then
      status=offset
    else
      status=clean
    fi
    before="$work/before"
    rm -rf "$before" && cp -a "$tree" "$before"
    patch -p1 --fuzz=0 -s -d "$tree" <"$p"
    echo "$status      $name"
    if [[ "$status" == offset && "$rebase" == 1 ]]; then
      {
        sed '/^--- a\//,$d' "$p"
        for f in $(targets "$p"); do
          diff -u --label "a/$f" --label "b/$f" "$before/$f" "$tree/$f" || true
        done
      } >"$work/rebased.patch"
      mv "$work/rebased.patch" "$p"
      echo "           rebased onto ${rev:0:9}"
    fi
  elif patch -p1 -R --fuzz=0 --dry-run -d "$tree" <"$p" >/dev/null 2>&1; then
    echo "upstreamed $name"
    work_needed+=("$name")
  else
    echo "conflict   $name"
    grep -E "checking file|patching file|FAILED|malformed|can.t find" <<<"$out" | sed 's/^/           /'
    work_needed+=("$name")
  fi
done

if ((${#work_needed[@]})); then
  echo "PATCHES=work ${work_needed[*]}"
else
  echo "PATCHES=clean"
fi
