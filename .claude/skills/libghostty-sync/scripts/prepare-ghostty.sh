#!/usr/bin/env bash
# Make the Ghostty reference clone hold both revisions with the history between them, then check out the new one.
#
#   prepare-ghostty.sh [--ghostty DIR] [--no-checkout] OLD_REV NEW_REV
#
# DIR defaults to <repo>/.upstream/ghostty, the read-only reference clone (git-ignored through .upstream/ in
# .git/info/exclude). A missing clone is created blobless (full history, blobs fetched on demand); a shallow one is
# unshallowed, since `git log OLD..NEW` needs the history. The clone is never written to beyond fetching and checking
# out NEW_REV (detached): local changes in it stop the script. Tags are fetched with --force, since Ghostty moves its
# `tip` tag. github.com drops large fetches now and then, so every network step retries.
# Prints GHOSTTY=, OLD_SHA=, NEW_SHA=, COMMITS= (all commits in OLD..NEW), MERGES= (first-parent commits: one per
# merged pull request, the clusters of the analysis).
set -euo pipefail

root="$(git rev-parse --show-toplevel)"
clone="$root/.upstream/ghostty"
checkout=1
while [[ $# -gt 0 ]]; do
  case "$1" in
    --ghostty) clone="$2"; shift 2 ;;
    --no-checkout) checkout=0; shift ;;
    -*) echo "prepare-ghostty: unknown option $1" >&2; exit 1 ;;
    *) break ;;
  esac
done
[[ $# -eq 2 ]] || { echo "usage: prepare-ghostty.sh [--ghostty DIR] [--no-checkout] OLD_REV NEW_REV" >&2; exit 1; }
old="$1" new="$2"
URL="https://github.com/ghostty-org/ghostty"

retry() {
  local attempt
  for attempt in 1 2 3 4; do
    "$@" && return 0
    echo "prepare-ghostty: attempt $attempt failed: $*" >&2
    sleep $((attempt * 5))
  done
  return 1
}

if [[ ! -e "$clone" ]]; then
  mkdir -p "$(dirname "$clone")"
  # a failed attempt leaves a partial clone this script made itself: only that is ever removed
  clone_it() { rm -rf "$clone"; git clone -q --filter=blob:none --no-checkout "$URL" "$clone"; }
  retry clone_it
else
  # an existing DIR is used only when it is the top of a git repository (a clone, worktree or submodule); anything
  # else is refused, never removed or cloned over
  top="$(git -C "$clone" rev-parse --show-toplevel 2>/dev/null || true)"
  if [[ -z "$top" || "$(cd "$clone" && pwd -P)" != "$(cd "$top" && pwd -P)" ]]; then
    echo "prepare-ghostty: $clone exists and is not the top of a git repository; move it or pass another --ghostty" >&2
    exit 1
  fi
  # a clone never checked out has an empty index, which status reports as staged deletions: not local changes
  if [[ -n "$(git -C "$clone" ls-files | head -n 1)" \
    && -n "$(git -C "$clone" status --porcelain --untracked-files=no)" ]]; then
    echo "prepare-ghostty: $clone has local changes; it is read-only, restore it first" >&2
    exit 1
  fi
  if [[ "$(git -C "$clone" rev-parse --is-shallow-repository)" == true ]]; then
    retry git -C "$clone" fetch -q --force --unshallow --tags origin
  else
    retry git -C "$clone" fetch -q --force --tags origin
  fi
fi

for rev in "$old" "$new"; do
  git -C "$clone" rev-parse -q --verify "$rev^{commit}" >/dev/null \
    || { echo "prepare-ghostty: $rev not found in $clone" >&2; exit 1; }
done
# a patch release cut from a maintenance branch is no ancestor of the next minor, so a higher release may follow it
release_at() { git -C "$clone" tag --points-at "$1" | awk '/^v[0-9]+[.][0-9]+[.][0-9]+$/' | sort -V | tail -n 1; }
newer_release() {
  local from to
  from="$(release_at "$old")"
  to="$(release_at "$new")"
  [[ -n "$from" && -n "$to" && "$from" != "$to" \
    && "$(printf '%s\n%s\n' "$from" "$to" | sort -V | tail -n 1)" == "$to" ]]
}
git -C "$clone" merge-base --is-ancestor "$old" "$new" || newer_release \
  || { echo "prepare-ghostty: $old is not an ancestor of $new" >&2; exit 1; }

# a blobless clone fetches the blobs a diff needs lazily: fetch them now, with retries, so later reads stay local
retry git -C "$clone" diff --numstat "$old" "$new" >/dev/null

if [[ "$checkout" == 1 ]]; then
  retry git -C "$clone" -c advice.detachedHead=false checkout -q "$new"
fi

echo "GHOSTTY=$clone"
echo "OLD_SHA=$(git -C "$clone" rev-parse "$old^{commit}")"
echo "NEW_SHA=$(git -C "$clone" rev-parse "$new^{commit}")"
echo "COMMITS=$(git -C "$clone" rev-list --count "$old..$new")"
echo "MERGES=$(git -C "$clone" rev-list --count --first-parent "$old..$new")"
