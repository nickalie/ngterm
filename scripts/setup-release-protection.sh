#!/usr/bin/env bash
# Set up the GitHub protection the release workflow (.github/workflows/release.yml) relies on, through `gh api`. The
# repository admin runs it once; running it again converges to the same state, and it reads the state back at the end.
#
# - environment `release-gate`: the reviewer (by default the repository's owner) must approve, self-review allowed,
#   since the one who pushes the tag also approves; deployments only from `v*` tags. It holds no secrets: release.yml's
#   `approve` job waits on it, and every job that signs or publishes needs `approve`.
# - environment `release`: deployments only from `v*` tags, no reviewer. It holds every signing secret.
# - the tag ruleset `release tags`: only the repository admin role creates, moves or deletes a `v*` tag, so only an
#   admin can start a run that reaches `release`.
#
# Each environment keeps exactly the `v*` tag policy: any other branch or tag policy on it is removed.
#
# --dry-run sends no write: it prints each write as one JSON object per line ({"method", "path", "body"}) and skips the
# read-back. It still reads what exists, to know which writes are needed.
#
# Usage: scripts/setup-release-protection.sh [--dry-run] [--repo OWNER/NAME] [--reviewer LOGIN]
#   --repo      the repository (default: gh's repository for the current directory)
#   --reviewer  the user who approves releases (default: the owner, when the owner is a user)
set -euo pipefail

die() {
  echo "setup-release-protection: $*" >&2
  exit 1
}
usage() { die "usage: scripts/setup-release-protection.sh [--dry-run] [--repo OWNER/NAME] [--reviewer LOGIN]"; }

dry_run=false
repo=""
reviewer=""
while (($#)); do
  case "$1" in
    --dry-run) dry_run=true ;;
    --repo) (($# >= 2)) || usage; repo="$2"; shift ;;
    --reviewer) (($# >= 2)) || usage; reviewer="$2"; shift ;;
    *) usage ;;
  esac
  shift
done
command -v gh > /dev/null || die "needs the GitHub CLI (gh), logged in as a repository admin"
command -v jq > /dev/null || die "needs jq"

TAG_PATTERN='v*'
RULESET_NAME='release tags'
# the built-in Repository admin role's id as a ruleset bypass actor
ADMIN_ROLE_ID=5

[[ -n "$repo" ]] || repo="$(gh repo view --json nameWithOwner --jq .nameWithOwner)"
[[ "$(gh api "repos/$repo" --jq .permissions.admin)" == true ]] || die "you are not an admin of $repo"
if [[ -z "$reviewer" ]]; then
  [[ "$(gh api "repos/$repo" --jq .owner.type)" == User ]] || die "$repo belongs to an organization: give --reviewer"
  reviewer="$(gh api "repos/$repo" --jq .owner.login)"
fi
reviewer_id="$(gh api "users/$reviewer" --jq .id)"

# write METHOD PATH [BODY]: send one write, or print it on a dry run
write() {
  local method="$1" path="$2" body="${3:-null}"
  if $dry_run; then
    jq -cn --arg method "$method" --arg path "$path" --argjson body "$body" \
      '{method: $method, path: $path, body: $body}'
  else
    echo "$method $path" >&2
    if [[ "$body" == null ]]; then
      gh api --method "$method" "$path" > /dev/null
    else
      gh api --method "$method" "$path" --input - <<< "$body" > /dev/null
    fi
  fi
}

environment_exists() {
  gh api "repos/$repo/environments" --paginate --jq '.environments[].name' | grep -qxF "$1"
}

# the environment's deployment policies, one "ID TYPE NAME" line each
environment_policies() {
  gh api "repos/$repo/environments/$1/deployment-branch-policies" --paginate \
    --jq '.branch_policies[] | "\(.id) \(.type) \(.name)"'
}

# environment NAME REVIEWERS_JSON: create or update it, then leave it the `v*` tag policy alone
environment() {
  local name="$1" reviewers="$2" existed=false kept=false id type pattern
  environment_exists "$name" && existed=true
  write PUT "repos/$repo/environments/$name" "$(jq -cn --argjson reviewers "$reviewers" '{
    wait_timer: 0,
    prevent_self_review: false,
    reviewers: $reviewers,
    deployment_branch_policy: {protected_branches: false, custom_branch_policies: true}
  }')"
  if $existed || ! $dry_run; then
    while read -r id type pattern; do
      [[ -n "$id" ]] || continue
      if [[ "$type" == tag && "$pattern" == "$TAG_PATTERN" ]] && ! $kept; then
        kept=true
      else
        write DELETE "repos/$repo/environments/$name/deployment-branch-policies/$id"
      fi
    done < <(environment_policies "$name")
  fi
  $kept || write POST "repos/$repo/environments/$name/deployment-branch-policies" \
    "$(jq -cn --arg pattern "$TAG_PATTERN" '{name: $pattern, type: "tag"}')"
}

environment release-gate "$(jq -cn --argjson id "$reviewer_id" '[{type: "User", id: $id}]')"
environment release null

ruleset="$(jq -cn --arg pattern "refs/tags/$TAG_PATTERN" --arg name "$RULESET_NAME" --argjson admin "$ADMIN_ROLE_ID" '{
  name: $name,
  target: "tag",
  enforcement: "active",
  conditions: {ref_name: {include: [$pattern], exclude: []}},
  rules: [
    {type: "creation"},
    {type: "update", parameters: {update_allows_fetch_and_merge: false}},
    {type: "deletion"}
  ],
  bypass_actors: [{actor_id: $admin, actor_type: "RepositoryRole", bypass_mode: "always"}]
}')"
ruleset_id() {
  gh api "repos/$repo/rulesets?includes_parents=false" --paginate \
    --jq ".[] | select(.name == \"$RULESET_NAME\" and .target == \"tag\") | .id" | head -n 1
}
id="$(ruleset_id)"
if [[ -n "$id" ]]; then
  write PUT "repos/$repo/rulesets/$id" "$ruleset"
else
  write POST "repos/$repo/rulesets" "$ruleset"
fi

$dry_run && exit 0

# Read the state back: each environment has exactly the tag policy and its reviewers, the ruleset is active.
for name in release-gate release; do
  [[ "$(environment_policies "$name" | cut -d' ' -f2-)" == "tag $TAG_PATTERN" ]] \
    || die "environment $name: expected the single deployment policy tag $TAG_PATTERN"
done
reviewers="$(gh api "repos/$repo/environments/release-gate" --jq '.protection_rules[]
  | select(.type == "required_reviewers") | .reviewers[].reviewer.id')"
[[ "$reviewers" == "$reviewer_id" ]] || die "environment release-gate: expected $reviewer as its only reviewer"
reviewers="$(gh api "repos/$repo/environments/release" \
  --jq '[.protection_rules[] | select(.type == "required_reviewers")] | length')"
[[ "$reviewers" == 0 ]] || die "environment release: expected no reviewer"
id="$(ruleset_id)"
[[ -n "$id" && "$(gh api "repos/$repo/rulesets/$id" --jq .enforcement)" == active ]] \
  || die "ruleset $RULESET_NAME: expected it active"
echo "release protection is set up on $repo (reviewer $reviewer)" >&2
