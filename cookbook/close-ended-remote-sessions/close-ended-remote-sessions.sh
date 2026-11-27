#!/usr/bin/env bash
# Close every attached remote session that has ended, in every open window: the rows the sidebar marks with a
# slashed cloud whose panes all stopped on their disconnect line.
#
#   close-ended-remote-sessions.sh            close them (from a keymap.conf custom command, or a shell inside ngterm)
#   close-ended-remote-sessions.sh --dry-run  only list them
set -euo pipefail

NGTERMCTL=${NGTERMCTL:-ngtermctl}
socket=${AGT_SOCKET:-${NGTERM_SOCKET:-}}
ctl() {
  local args=("$@")
  [ -n "$socket" ] && args+=(--socket "$socket")
  "$NGTERMCTL" "${args[@]}"
}

dry_run=false
verb=closing
case "${1:-}" in
  "") ;;
  -n | --dry-run)
    dry_run=true
    verb="would close"
    ;;
  *)
    echo "usage: close-ended-remote-sessions.sh [--dry-run]" >&2
    exit 64
    ;;
esac

# The rows worth reading the screen of: attached from another machine, the presentation stream failing, retrying or
# stopped (the slashed cloud; `unsupported`, an origin too old to stream, has no stream to tell a live session by), no pane
# reconnecting, no process left in any pane, and nothing of this machine's on the row that closing it would end: a
# scratch shell or an overlay, hidden ones included. Prints `id <TAB> hasSplit` per row.
candidates='
  .workspaces[].sessions[]
  | select(.remoteHost != null and .content != "page")
  | select(.presentation.state == "failed" or .presentation.state == "connecting" or .presentation.state == "ended")
  | select(all(.surfaces[]?; .reconnect == null and (.kind == "left" or .kind == "right")))
  | select((.overlay // false) == false and .paneOverlays == null and .htmlOverlays == null and .hiddenOverlays == null)
  | select(.foreground == null and .foregroundShell == null)
  | select((.hasSplit // false) == false or (.splitForeground == null and .splitForegroundShell == null))
  | [.id, (.hasSplit // false)] | @tsv'

# Whether a pane's screen ends on its disconnect line, `ngterm: NAME (PANE) on HOST disconnected, exit N`, and the
# hold prompt under it. Whitespace is dropped before matching, so a line the pane's width wrapped still matches.
pane_ended() {
  local window=$1 session=$2 pane=$3 text
  text=$(ctl session text --target "$session" --pane "$pane" --lines 6 --window "$window" --json 2>/dev/null |
    jq -r '.result.text // empty') || return 1
  tr -d ' \t\r\n' <<<"$text" |
    grep -qE 'ngterm:.*disconnected,exit-?[0-9]*Processexited\.Pressanykeytoclosetheterminal\.$'
}

# Given the ended ids, the rows to close: an ended row whose companions are all ended or pages. Closing a parent
# closes its companions with it, so a companion whose parent closes is listed (`with`) but not targeted (`target`).
# shellcheck disable=SC2016 # $rows, $dead, $closable and $ended are jq's
plan='
  [.workspaces[].sessions[]] as $rows
  | ($rows | map(select(.id as $id | $ended | index($id)))) as $dead
  | ($dead | map(select(.id as $id
      | all($rows[] | select(.parent == $id); (.content == "page") or (.id as $c | $ended | index($c))))) | map(.id)) as $closable
  | $rows[]
  | select((.id as $id | $closable | index($id)) or (.parent != null and (.parent as $p | $closable | index($p))))
  | [(if .parent != null and (.parent as $p | $closable | index($p)) then "with" else "target" end),
     .id, .name, (.remoteHost // "page")] | @tsv'

# Read into variables rather than fed through process substitution, so a failed read (a socket error, a missing jq)
# stops the script with its status instead of reading as an empty list.
windows=$(ctl window list --json | jq -r '.result.windows[] | select(.open) | .id')
total=0
for window in $windows; do
  tree=$(ctl tree --json --window "$window" | jq -c '.result.tree')
  rows=$(jq -r "$candidates" <<<"$tree")
  ended=()
  while IFS=$'\t' read -r id split; do
    [ -n "$id" ] || continue
    pane_ended "$window" "$id" left || continue
    if [ "$split" = true ]; then
      pane_ended "$window" "$id" right || continue
    fi
    ended+=("$id")
  done <<<"$rows"
  ((${#ended[@]} > 0)) || continue

  plan_rows=$(jq -r --argjson ended "$(printf '%s\n' "${ended[@]}" | jq -R . | jq -sc .)" "$plan" <<<"$tree")
  targets=()
  while IFS=$'\t' read -r role id name host; do
    if [ "$role" = target ]; then
      targets+=(--target "$id")
      printf '%s %s (%s)\n' "$verb" "$name" "$host"
    else
      printf '  with it: %s\n' "$name"
    fi
  done <<<"$plan_rows"
  ((${#targets[@]} > 0)) || continue
  $dry_run && continue

  # One target gets ⌘W's undo grace explicitly; a batch takes the grouped undo record the Settings grace allows. The
  # flag goes on the end of the array rather than in an array of its own: bash before 4.4 (macOS's /bin/bash) calls
  # an empty array unbound under `set -u`.
  ((${#targets[@]} == 2)) && targets+=(--undoable)
  closed=$(ctl session close "${targets[@]}" --window "$window" --json | jq '.result.closed | length')
  total=$((total + closed))
done

$dry_run && exit 0
if [ "$total" -eq 0 ]; then
  message="No ended remote sessions"
else
  message="Closed $total ended remote session$([ "$total" -eq 1 ] || echo s)"
fi
echo "$message"
# From a chord, say it on screen too; a busy overlay slot is no reason to fail after the work is done.
if [ -n "${AGT_WINDOW_ID:-}" ]; then
  ctl session hud "$message" --hide-after 3 --window "$AGT_WINDOW_ID" >/dev/null 2>&1 || true
fi
