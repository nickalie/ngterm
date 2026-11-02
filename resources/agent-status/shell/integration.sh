# ngterm-agent-status — shell integration for bash and zsh.
# Sets the ngterm session's agent-status indicator to `active` while a coding
# agent runs as a foreground command, and back to `idle` at the next prompt.
# Source this from your ~/.zshrc and/or ~/.bashrc.
#
# Which commands count as agents is a regex, override before sourcing to taste:
#   export NGTERM_AGENT_RE='^(gemini|cursor-agent|my-agent)([[:space:]]|$)'
#
# Claude Code, Codex, Pi, and OpenCode are intentionally NOT in the default list —
# their own hooks/extensions/plugins drive finer per-turn state, which the coarse
# process-level active/idle here would only fight. Add any of them here if you
# rely on the shell integration alone for it.
#
# Every entry point is best-effort and a clean no-op outside ngterm (guarded by
# $NGTERM_SESSION_ID), so sourcing it from a non-ngterm shell does nothing.

[ -n "${NGTERM_SESSION_ID:-}" ] || return 0 2>/dev/null

# Locate ngterm-agent-status.sh relative to this file (layout is preserved on
# install: shell/integration.sh sits next to ngterm-agent-status.sh's dir).
if [ -n "${ZSH_VERSION:-}" ]; then
  _ags_self="${(%):-%x}"
  _ags_dir="${_ags_self:a:h:h}"
else
  _ags_self="${BASH_SOURCE[0]}"
  _ags_dir="$(cd "$(dirname "$_ags_self")/.." >/dev/null 2>&1 && pwd)"
fi
: "${NGTERM_AGENT_BIN:=$_ags_dir/ngterm-agent-status.sh}"
: "${NGTERM_AGENT_RE:=^(gemini|cursor-agent|aider|crush|goose)([[:space:]]|$)}"

# Codex's TUI normally attaches to one background daemon shared by every pane, and the daemon starts hooks with the
# environment of the pane that started it, so every Codex's status lands on that pane. Run each TUI in its own process
# instead, which costs control of that session from the Codex apps. `command codex` or a non-empty
# NGTERM_CODEX_KEEP_DAEMON keeps the daemon, and a codex function of the user's is left alone.
#
# Whether this codex line opens a TUI: no subcommand, or `resume` or `fork`. A single lowercase word is taken for a
# subcommand, so a one-word prompt stays on the daemon. The daemon's own subcommands keep it wherever they sit, as a
# global option can come first and Codex refuses the flag on them. An older Codex lacks the flag, hence the --help check.
_ags_codex_own_process() {
  local arg
  [ -z "${NGTERM_CODEX_KEEP_DAEMON:-}" ] || return 1
  for arg in "$@"; do
    case "$arg" in
      --) break ;;
      --no-daemon|--remote|--remote=*|agents|queue|remote-control|app-server) return 1 ;;
    esac
  done
  case "${1:-}" in
    resume|fork|''|-*|*[!a-z-]*) ;;
    *) return 1 ;;
  esac
  command codex --help 2>/dev/null | command grep -q -e --no-daemon
}

if ! typeset -f codex >/dev/null 2>&1; then
  function codex {
    if _ags_codex_own_process "$@"; then
      command codex --no-daemon "$@"
    else
      command codex "$@"
    fi
  }
fi

if [ -n "${ZSH_VERSION:-}" ]; then
  autoload -Uz add-zsh-hook
  # the redirect stays on the match, not the whole hook: an override valid as the PCRE fish and
  # RE_MATCH_PCRE zsh accept can be invalid here, and zsh prints the compile error once per prompt
  _ags_preexec() { [[ "$1" =~ $NGTERM_AGENT_RE ]] 2>/dev/null && { "$NGTERM_AGENT_BIN" active --blink; _ags_active=1; }; }
  _ags_precmd()  { [[ -n "${_ags_active:-}" ]] && { "$NGTERM_AGENT_BIN" idle; unset _ags_active; }; }
  add-zsh-hook preexec _ags_preexec
  add-zsh-hook precmd  _ags_precmd

elif [ -n "${BASH_VERSION:-}" ]; then
  _ags_preexec() {
    local cmd="${1:-$BASH_COMMAND}"               # bash-preexec passes the command as $1; a raw DEBUG trap uses $BASH_COMMAND
    [ -n "${COMP_LINE:-}" ] && return             # ignore tab-completion
    case "$cmd" in _ags_*) return ;; esac
    if [[ "$cmd" =~ $NGTERM_AGENT_RE ]] 2>/dev/null; then
      "$NGTERM_AGENT_BIN" active --blink
      _ags_active=1
    fi
  }
  # preserve the command status for the rest of PROMPT_COMMAND
  _ags_precmd() { local s=$?; [ -n "${_ags_active:-}" ] && { "$NGTERM_AGENT_BIN" idle; unset _ags_active; }; return "$s"; }

  if [ -n "${bash_preexec_imported:-}${__bp_imported:-}" ]; then
    # bash-preexec is active: register into its additive arrays so we don't clobber other consumers
    case " ${preexec_functions[*]-} " in *" _ags_preexec "*) : ;; *) preexec_functions+=(_ags_preexec) ;; esac
    case " ${precmd_functions[*]-} "  in *" _ags_precmd "*)  : ;; *) precmd_functions+=(_ags_precmd)  ;; esac
  else
    # no bash-preexec: install the DEBUG trap only when nothing else owns it, so we never break a user's
    # existing preexec tooling. an unrelated existing trap → skip the active-state hook with a notice.
    case "$(trap -p DEBUG 2>/dev/null)" in
      "")             trap '_ags_preexec' DEBUG ;;   # nothing there: safe to install
      *_ags_preexec*) : ;;                            # already ours: idempotent
      *) printf '%s\n' "ngterm agent-status: an existing bash DEBUG trap is set; skipping the active-state hook (use bash-preexec, or wire a preexec yourself)." >&2 ;;
    esac
    # precmd via PROMPT_COMMAND is additive and safe regardless of any DEBUG trap
    case "${PROMPT_COMMAND:-}" in
      *_ags_precmd*) : ;;
      *) PROMPT_COMMAND="_ags_precmd${PROMPT_COMMAND:+; $PROMPT_COMMAND}" ;;
    esac
  fi
fi
