#!/bin/bash
# create or find the worktree of a sync branch, so an hours-long unattended run never takes over the shared checkout
# usage: sync-worktree.sh <branch>        (run from the checkout; a new branch starts at its HEAD)
# the worktree is <repo>/.claude/worktrees/<branch>; it links the checkout's git-ignored .upstream, .venv and
# machines.local.md, which the sync's scripts, plans and workers expect at the same paths as in the checkout
# exit 1, creating nothing, when the branch is checked out anywhere else
# prints WORKDIR=, PLAN= (absolute, empty when there is none) and STATE=, from the branch's state, the same on a repeat:
#   new        the branch has no commits of its own and a clean tree: nothing to lose, start from the top
#   unplanned  the branch has work but no plan: an earlier run died before writing one
#   planned    the branch holds docs/plans/*-<branch>.md
#   done       the branch holds docs/plans/completed/*-<branch>.md
set -euo pipefail

[ $# -eq 1 ] || { echo "error: usage: sync-worktree.sh <branch>" >&2; exit 1; }
branch=$1
common=$(git rev-parse --path-format=absolute --git-common-dir)
root=$(cd "$(dirname "$common")" && pwd -P)
expected="$root/.claude/worktrees/$branch"

workdir=$(git worktree list --porcelain | awk -v b="branch refs/heads/$branch" '
    /^worktree / { w = substr($0, 10) }
    $0 == b { print w; exit }')
if [ -n "$workdir" ] && [ "$workdir" != "$expected" ]; then
    echo "error: $branch is checked out in $workdir, not in its own worktree $expected" >&2
    exit 1
fi
if [ -z "$workdir" ]; then
    workdir=$expected
    if git show-ref --verify --quiet "refs/heads/$branch"; then
        git worktree add --quiet "$workdir" "$branch"
    else
        git worktree add --quiet -b "$branch" "$workdir" HEAD
        # git drops this with the branch; a tip still equal to it means the branch has no commits of its own
        git config "branch.$branch.syncBase" "$(git -C "$workdir" rev-parse HEAD)"
    fi
fi

# a "dir/" ignore pattern does not match a symlink, so the links get patterns of their own
for link in .upstream .venv .claude/skills/agterm-upstream-sync/references/machines.local.md; do
    [ -e "$root/$link" ] || continue
    [ -e "$workdir/$link" ] || ln -s "$root/$link" "$workdir/$link"
    if ! git -C "$workdir" check-ignore -q "$link"; then
        mkdir -p "$common/info"
        echo "/$link" >>"$common/info/exclude"
    fi
done

state=unplanned
base=$(git config "branch.$branch.syncBase" || true)
if [ "$base" = "$(git -C "$workdir" rev-parse HEAD)" ] && [ -z "$(git -C "$workdir" status --porcelain)" ]; then
    state=new
fi
plan=
while IFS= read -r path; do
    case "$path" in
        "docs/plans/completed/"*"-$branch.md") plan="$workdir/$path" state="done" ;;
        "docs/plans/"*/*) ;;
        "docs/plans/"*"-$branch.md") plan="$workdir/$path" state=planned ;;
    esac
done < <(git -C "$workdir" ls-tree -r --name-only HEAD -- docs/plans)

echo "WORKDIR=$workdir"
echo "PLAN=$plan"
echo "STATE=$state"
