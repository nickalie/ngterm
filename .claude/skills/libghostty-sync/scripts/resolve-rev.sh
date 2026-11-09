#!/usr/bin/env bash
# Resolve the Ghostty revision ngterm pins and the one to sync to.
#
#   resolve-rev.sh [TARGET]
#
# CURRENT is LIBGHOSTTY_VT_REV of build/pins.env at HEAD, what a sync branch starts from whatever the tree has
# uncommitted. TARGET is the argument (a commit sha, a tag such as vt-1.0.0 or
# v1.4.0, or a branch), `vt` for the newest vt-* tag (libghostty-vt's releases), `main-green` for the newest commit on
# ghostty main whose libghostty-vt CI is green (every check run named build-libghostty-vt* or test-lib-vt completed with
# success; a skipped or neutral one means nothing built the library there, so the commit is not green), or, by default,
# the newest Ghostty release: the highest vX.Y.Z tag (its GitHub releases hold only the rolling `tip`, so tags are the
# record). A release or a named TARGET is taken as given; its CI state is still reported.
#
# Prints CURRENT=, TARGET= (full sha), TARGET_REF= (what it was resolved from), STATUS= (ahead | identical | behind |
# diverged: TARGET relative to CURRENT), AHEAD= (commits from CURRENT to TARGET), CI= (green | red | pending |
# skipped | none: the libghostty-vt checks of TARGET; skipped when some were skipped or neutral and none failed).
# Exit 0 when TARGET is ahead of CURRENT, or diverged from it with both on release tags and TARGET the higher version (a
# patch release cut from a maintenance branch is no ancestor of the next minor: v1.2.3 and v1.3.0). Exit 3 when there is
# nothing to sync (identical or behind), 1 on an error (a gh call that failed stops the script with gh's own message)
# or any other diverged TARGET (a pin must not lose commits it has).
set -euo pipefail

REPO="ghostty-org/ghostty"
SCAN=40          # main commits searched for a green one
CHECKS='^(build-libghostty-vt|test-lib-vt)'
RELEASE='^v[0-9]+[.][0-9]+[.][0-9]+$'

root="$(git rev-parse --show-toplevel)"
pins="$(git -C "$root" show HEAD:build/pins.env)" || { echo "resolve-rev: no build/pins.env at HEAD" >&2; exit 1; }
current="$(sed -nE 's/^LIBGHOSTTY_VT_REV="([0-9a-f]+)".*/\1/p' <<<"$pins" | head -n 1)"
[[ -n "$current" ]] || { echo "resolve-rev: no LIBGHOSTTY_VT_REV in build/pins.env at HEAD" >&2; exit 1; }

die() { echo "resolve-rev: $*" >&2; exit 1; }

# The state of a commit's libghostty-vt checks: green, red, pending, skipped or none. Exits the script when gh fails.
ci_state() {
  local states
  states="$(gh api --paginate "repos/$REPO/commits/$1/check-runs?per_page=100" \
    --jq ".check_runs[] | select(.name | test(\"$CHECKS\")) | .status + \":\" + (.conclusion // \"\")")" \
    || die "cannot read the check runs of $1 (gh failed: see above)"
  if [[ -z "$states" ]]; then echo none
  elif grep -qvE '^completed:' <<<"$states"; then echo pending
  elif grep -qvE '^completed:(success|skipped|neutral)$' <<<"$states"; then echo red
  elif grep -qvE '^completed:success$' <<<"$states"; then echo skipped
  else echo green
  fi
}

# Every tag of ghostty as "name sha".
tags() {
  gh api --paginate "repos/$REPO/tags?per_page=100" --jq '.[] | .name + " " + .commit.sha' \
    || die "cannot list $REPO's tags (gh failed: see above)"
}

# Sets `tag` to the highest version among the tags whose name matches $1, and `target` to its commit.
newest_tag() {
  local all
  all="$(tags)" || exit 1
  tag="$(awk -v re="$1" '$1 ~ re { print $1 }' <<<"$all" | sort -V | tail -n 1)"
  [[ -n "$tag" ]] || die "no tag matching $1 in $REPO"
  target="$(awk -v name="$tag" '$1 == name { print $2 }' <<<"$all")"
}

# The highest release tag on commit $2 in the "name sha" list $1, or nothing.
release_at() {
  awk -v re="$RELEASE" -v sha="$2" '$1 ~ re && $2 == sha { print $1 }' <<<"$1" | sort -V | tail -n 1
}

ref="${1:-release}"
case "$ref" in
  release)
    newest_tag "$RELEASE"
    ref="$tag"
    ;;
  main-green)
    target=""
    shas="$(gh api "repos/$REPO/commits?sha=main&per_page=$SCAN" --jq '.[].sha')" \
      || die "cannot list ghostty main's commits (gh failed: see above)"
    for sha in $shas; do
      state="$(ci_state "$sha")" || exit 1
      if [[ "$state" == green ]]; then target="$sha"; break; fi
    done
    [[ -n "$target" ]] || die "none of main's last $SCAN commits has green libghostty-vt CI"
    ;;
  vt)
    newest_tag '^vt-[0-9]'
    ref="$tag"
    ;;
  *)
    target="$(gh api "repos/$REPO/commits/$ref" --jq .sha)" || die "$ref is not a commit, tag or branch of $REPO"
    ;;
esac

compare="$(gh api "repos/$REPO/compare/$current...$target" --jq '.status + " " + (.ahead_by | tostring)')" \
  || die "cannot compare $current with $target (gh failed: see above)"
read -r status ahead <<<"$compare"
[[ "$status" =~ ^(ahead|identical|behind|diverged)$ ]] || die "unexpected compare status: $compare"
ci="$(ci_state "$target")" || exit 1

echo "CURRENT=$current"
echo "TARGET=$target"
echo "TARGET_REF=$ref"
echo "STATUS=$status"
echo "AHEAD=$ahead"
echo "CI=$ci"
case "$status" in
  ahead) exit 0 ;;
  identical | behind) exit 3 ;;
  *)
    all="$(tags)" || exit 1
    from="$(release_at "$all" "$current")"
    to="$(release_at "$all" "$target")"
    if [[ -n "$from" && -n "$to" && "$from" != "$to" \
      && "$(printf '%s\n%s\n' "$from" "$to" | sort -V | tail -n 1)" == "$to" ]]; then
      exit 0
    fi
    die "$target has diverged from the pin $current"
    ;;
esac
