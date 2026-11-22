#!/usr/bin/env bash
# Go to: a navigator of the window's workspaces, sessions and panes in an overlay page; Enter switches there.
#
#   go-to-navigator.sh        open it (from a keymap.conf custom command, or a shell inside ngterm)
#   go-to-navigator.sh page   write the page to a file and print its path, to check it in a browser
set -euo pipefail

NGTERMCTL=${NGTERMCTL:-ngtermctl}
here=$(cd "$(dirname "$0")" && pwd)

socket=${AGT_SOCKET:-${NGTERM_SOCKET:-}}
window=${AGT_WINDOW_ID:-}
ctl() {
  local args=("$@")
  [ -n "$socket" ] && args+=(--socket "$socket")
  "$NGTERMCTL" "${args[@]}"
}
# Commands that address a window take this one's: a chord fires in the window it was pressed in.
wctl() {
  local args=("$@")
  [ -n "$window" ] && args+=(--window "$window")
  ctl "${args[@]}"
}

tree=$(wctl tree --json | jq -c '.result.tree')

# The branch checked out in each local session's directories.
branches=$(
  jq -r '.workspaces[].sessions[] | select(.remoteHost == null) | .cwd, .splitCwd | select(. != null)' <<<"$tree" |
    sort -u |
    while IFS= read -r dir; do
      [ -d "$dir" ] || continue
      branch=$(git -C "$dir" symbolic-ref --short -q HEAD 2>/dev/null) || continue
      jq -nc --arg d "$dir" --arg b "$branch" '{($d): $b}'
    done | jq -sc 'add // {}'
)

build_page() {
  local out=$1
  # Every `<` is encoded, so no name, title, cwd or branch can open a comment or tag inside the <script> holding it;
  # jq writes `<` only inside strings, where \u003c is the same character.
  jq -nc --argjson tree "$tree" --argjson branches "$branches" --arg home "${HOME:-}" \
    --arg session "${AGT_SESSION_ID:-}" '{tree: $tree, branches: $branches, home: $home, session: $session}' |
    sed 's#<#\\u003c#g; s#^#const GOTO = #; s#$#;#' >"$out.data"
  awk -v data="$out.data" -v js="$here/assets/go-to.js" '
    /\/\*GOTO_DATA\*\// { while ((getline line < data) > 0) print line; next }
    /\/\*GOTO_JS\*\// { while ((getline line < js) > 0) print line; next }
    { print }' "$here/assets/go-to.html" >"$out"
  rm -f "$out.data"
}

if [ "${1:-}" = page ]; then
  page=$(mktemp "${TMPDIR:-/tmp}/go-to-navigator.XXXXXX")
  build_page "$page"
  mv "$page" "$page.html"
  echo "$page.html"
  exit 0
fi

tmp=$(mktemp -d "${TMPDIR:-/tmp}/go-to-navigator.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
build_page "$tmp/go-to.html"

state=${socket%/ngterm.sock}
if [ "$state" = "$socket" ] || [ ! -f "$state/settings.json" ]; then
  data=${XDG_DATA_HOME:-}
  case $data in
    /?*) ;;
    *) data=${HOME:+$HOME/.local/share} ;;
  esac
  state=${NGTERM_STATE_DIR:-${data:+$data/ngterm}}
fi
zoom=1
if [ -n "$state" ]; then
  zoom=$(jq -s '
    (if length == 1 and (.[0] | type) == "object" then .[0].htmlOverlayZoom else null end) as $zoom
    | if ($zoom | type) != "number" or ($zoom | isnan or isinfinite) then 1
      elif $zoom < 0.5 then 0.5 elif $zoom > 3 then 3 else $zoom end' "$state/settings.json" 2>/dev/null) || zoom=1
fi
height=$(jq --argjson zoom "$zoom" '
  {top: 33, info: 51, hints: 17, gaps: 24, padding: 20, row: 21, workspace_margin: 6, empty: 35} as $px
  | [(.workspaces // [])[]
     | 1 + ([(.sessions // [])[] | select(.content != "page") | if .hasSplit then 3 else 1 end] | add // 0)]
  | (if length == 0 then $px.empty else $px.row * add + $px.workspace_margin * (length - 1) end)
    + $px.top + $px.info + $px.hints + $px.gaps + $px.padding
  | . * $zoom - 1e-3 | ceil' <<<"$tree")

# Exit 2 is the page closed without a choice: nothing to do. Any other failure (`overlay already open`, a socket
# error) keeps its status, so the keymap's --error-hud shows it.
status=0
out=$(wctl session overlay open --html "$tmp/go-to.html" --js --chromeless --size-percent 80 --height "$height" \
  --block --target "${AGT_SESSION_ID:-active}") || status=$?
[ "$status" -eq 2 ] && exit 0
[ "$status" -eq 0 ] || exit "$status"
choice=$(jq -r '.value // empty' <<<"$out")

case "$choice" in
  w:*) wctl workspace select --target "${choice#w:}" ;;
  s:*) wctl session select --target "${choice#s:}" ;;
  p:*)
    rest=${choice#p:}
    session=${rest%%:*}
    pane=${rest#*:}
    wctl session select --target "$session"
    split=$(wctl tree --json |
      jq -r --arg id "$session" '.result.tree.workspaces[].sessions[] | select(.id == $id) | .hasSplit // false')
    if [ "$split" = true ]; then
      wctl session focus "$pane" --target "$session"
    fi
    ;;
esac
