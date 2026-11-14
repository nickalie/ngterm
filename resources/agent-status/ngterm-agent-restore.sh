#!/usr/bin/env bash
# ngterm-agent-restore — keep a pane's restore pin on the Claude Code conversation it is in.
#
#   ngterm-agent-restore.sh claude start   # Claude Code SessionStart
#   ngterm-agent-restore.sh claude end     # Claude Code SessionEnd
#
# Installed by ngterm's Help ▸ Install Agent Status Hooks command. An ngterm extension beyond upstream, which
# ships this only as a cookbook recipe (claude-session-resume) and as the SessionStart-pin pattern in its skill.
#
# In rerun restore mode a pane re-runs its captured foreground argv, or its `session restore` pin. A bare `claude`
# argv comes back as a NEW conversation, and `claude --resume X` stays on X after /clear or /resume switched the
# process elsewhere. These hooks pin the LIVE conversation instead, on every session start (startup, resume, clear,
# compact, fork), so the next launch reattaches to the latest one:
#
#   claude --resume <session_id> || claude
#
# On POSIX, a Claude launched through ngswap instead pins helper-generated resume and fresh commands that preserve
# its account and configuration. This branch needs python3 and ngswap's ngswap-session.py recipe, configurable via
# NGSWAP_SESSION_HELPER. Its two private descriptors use stable session/pane slots and are replaced atomically on
# each start, so compaction and /clear do not accumulate unused descriptors. Windows keeps the ordinary pin form.
#
# The fallback starts a fresh claude when the conversation cannot be resumed: Claude Code fires SessionStart before
# the first message, and a conversation that never got one is not resumable ("No conversation found"). Claude Code
# exits 0 when quit (/exit, Ctrl-C twice, Ctrl-D), so the fallback runs only on that failure or a crash.
#
# The pin is shell code typed into the pane's login shell (fish, bash, zsh or PowerShell 7 all take `a || b`),
# sticky and persisted, so the payload's `session_id` must be UUID-shaped, hex digits and dashes only. The ngswap
# helper shell-quotes its interpreter, script and descriptor paths. Invalid ids or failed preparation leave the
# existing pin alone.
#
# What pins, and what clears: only the interactive CLI pins. A `claude -p` run (also one spawned by a tool call of
# the pane's interactive claude, which inherits its environment) and an Agent SDK run have CLAUDE_CODE_ENTRYPOINT
# `sdk-cli`, `sdk-ts`, `sdk-py`, …: Claude Code rewrites an inherited `cli` to `sdk-cli` for a non-interactive run
# before any hook fires. A payload with an `agent_id` comes from inside a subagent. SessionEnd clears the pin only on
# a deliberate exit (reason `prompt_input_exit` or `logout`); a killed process (SIGHUP when ngterm quits) ends with
# `other`, and `clear` and `resume` are followed by a SessionStart that re-pins, so all three keep it.
#
# ngterm's chat view: the pane's agent record follows the conversation the same way. Each start and each end (whatever
# its reason) pipes the payload to `ngtermctl agent hook claude SessionStart|SessionEnd`, under the same guards: the
# start sets the record, the end clears it when it names the record's conversation.
#
# As a hook it must never disturb the agent: no output (Claude Code adds a SessionStart hook's stdout to the
# conversation), always exit 0, and outside ngterm (NGTERM_SESSION_ID unset) it does nothing. Payload parsing uses
# bash's own regex and needs no jq or python; only the optional ngswap preparation uses python3.
#
# ngtermctl resolution: $NGTERMCTL, else the path the installer bakes in, else `ngtermctl` on PATH.

trap 'exit 0' EXIT
exec >/dev/null 2>&1

# A background job of Claude Code's daemon (CLAUDE_JOB_DIR is set: Claude Code moves a conversation there on
# /background or on a Left arrow at its empty prompt) has the daemon's NGTERM_*, which name the pane the daemon was
# started in: a session long gone, or another one's. `ngtermctl agent rerun` finds the pane that holds this
# conversation from the payload and runs this hook again as that pane; with none it runs nothing, silently.
if [ -n "${CLAUDE_JOB_DIR:-}" ] && [ -z "${NGTERM_AGENT_LOCATED:-}" ]; then
  command -v "${NGTERMCTL:-ngtermctl}" >/dev/null 2>&1 || exit 0
  rerun=(agent rerun claude)
  [ -n "${NGTERM_SOCKET:-}" ] && rerun+=(--socket "$NGTERM_SOCKET")
  exec "${NGTERMCTL:-ngtermctl}" "${rerun[@]}" -- "${BASH:-bash}" "$0" "$@"
fi

[ -n "${NGTERM_SESSION_ID:-}" ] || exit 0

agent=${1:-}
event=${2:-}

payload=
if [ ! -t 0 ]; then
  IFS= read -r -d '' -t 5 payload || true # EOF ends the read: the payload carries no NUL
fi

# A top-level string field of the payload. The key must not follow a backslash: a key quoted inside a string value is
# escaped (\"session_id\"), so the match cannot come from a message or a path. Claude Code writes the ids first.
string_field() {
  local re="(^|[^\\])\"$1\"[[:space:]]*:[[:space:]]*\"([^\"\\]*)\""
  [[ $payload =~ $re ]] || return 1
  printf '%s' "${BASH_REMATCH[2]}"
}

session_id() {
  local id uuid_re='^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$'
  id=$(string_field session_id) || return 1
  [[ $id =~ $uuid_re ]] || return 1
  printf '%s' "$id"
}

restore() {
  local args=(session restore "$@" --target "$NGTERM_SESSION_ID")
  [ -n "${NGTERM_PANE_ID:-}" ] && args+=(--pane-id "$NGTERM_PANE_ID")
  [ -n "${NGTERM_SOCKET:-}" ] && args+=(--socket "$NGTERM_SOCKET")
  "${NGTERMCTL:-ngtermctl}" "${args[@]}" </dev/null || true
}

# Pipes the payload to `ngtermctl agent hook claude <event>`, the pane addressed as the status hooks address it.
agent_hook() {
  local args=(agent hook claude "$1" --target "$NGTERM_SESSION_ID")
  [ -n "${NGTERM_PANE:-}" ] && args+=(--pane "$NGTERM_PANE")
  [ -n "${NGTERM_PANE_ID:-}" ] && args+=(--pane-id "$NGTERM_PANE_ID")
  [ -n "${NGTERM_SOCKET:-}" ] && args+=(--socket "$NGTERM_SOCKET")
  printf '%s' "$payload" | "${NGTERMCTL:-ngtermctl}" "${args[@]}" || true
}

claude_hook() {
  [ "${CLAUDE_CODE_ENTRYPOINT:-}" = cli ] || return 0
  [ -n "$(string_field agent_id)" ] && return 0
  local id reason
  case "$event" in
    start)
      agent_hook SessionStart
      id=$(session_id) || return 0
      if [ "${NGSWAP_RUN_PROVIDER:-}" = claude ] || {
        [ -z "${NGSWAP_RUN_PROVIDER:-}" ] && [ -n "${NGSWAP_DEFAULT_ROOT:-}" ]
      }; then
        # A bare restore pin loses the account just as a bare companion launch
        # does. Prepare both paths in this hook's still-correct environment.
        local launcher resume_command fresh_command slot
        launcher="${NGSWAP_SESSION_HELPER:-${XDG_CONFIG_HOME:-$HOME/.config}/ngterm/ngswap-profile/ngswap-session.py}"
        slot="$NGTERM_SESSION_ID:${NGTERM_PANE_ID:-${NGTERM_PANE:-left}}"
        resume_command=$(python3 "$launcher" prepare --provider claude --slot "$slot:resume" -- --resume "$id") || return 0
        fresh_command=$(python3 "$launcher" prepare --provider claude --slot "$slot:fresh") || return 0
        restore "$resume_command || $fresh_command"
      else
        restore "claude --resume $id || claude"
      fi
      ;;
    end)
      agent_hook SessionEnd
      reason=$(string_field reason) || return 0
      case "$reason" in prompt_input_exit | logout) restore --clear ;; esac
      ;;
  esac
}

case "$agent" in
  claude) claude_hook ;;
esac
exit 0
