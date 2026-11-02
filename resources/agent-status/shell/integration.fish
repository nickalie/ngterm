# ngterm-agent-status — shell integration for fish.
# Sets the ngterm session's agent-status indicator to `active` while a coding
# agent runs as a foreground command, and back to `idle` at the next prompt.
# Source this from your ~/.config/fish/config.fish.
#
# Which commands count as agents is a regex, override before sourcing to taste:
#   set -gx NGTERM_AGENT_RE '^(gemini|cursor-agent|my-agent)([[:space:]]|$)'
#
# Claude Code, Codex, Pi, and OpenCode are intentionally NOT in the default list —
# their own hooks/extensions/plugins drive finer per-turn state, which the coarse
# process-level active/idle here would only fight. Add any of them here if you
# rely on the shell integration alone for it.
#
# Every entry point is best-effort and a clean no-op outside ngterm (guarded by
# $NGTERM_SESSION_ID), so sourcing it from a non-ngterm shell does nothing.

if not set -q NGTERM_SESSION_ID
    return 0 2>/dev/null
end

# Locate ngterm-agent-status.sh relative to this file
set -l _ags_dir (dirname (dirname (status filename)))
if not set -q NGTERM_AGENT_BIN
    set -g NGTERM_AGENT_BIN "$_ags_dir/ngterm-agent-status.sh"
end

# exported, unlike the shell-local state below: the Claude adapter reads an overridden value from
# its process environment, and a `set -g` would keep it invisible outside this shell. BOTH branches
# export — the one carrying an override set before sourcing (the value the adapter needs, and the
# form this file documented as `set -g` until now) as much as the default
if set -q NGTERM_AGENT_RE
    set -gx NGTERM_AGENT_RE $NGTERM_AGENT_RE
else
    set -gx NGTERM_AGENT_RE '^(gemini|cursor-agent|aider|crush|goose)([[:space:]]|$)'
end

# Codex's TUI normally attaches to one background daemon shared by every pane, and the daemon starts hooks with the
# environment of the pane that started it, so every Codex's status lands on that pane. Run each TUI in its own process
# instead, which costs control of that session from the Codex apps. `command codex` or a non-empty
# NGTERM_CODEX_KEEP_DAEMON keeps the daemon, and a codex function of the user's is left alone.
#
# Whether this codex line opens a TUI: no subcommand, or `resume` or `fork`. A single lowercase word is taken for a
# subcommand, so a one-word prompt stays on the daemon. The daemon's own subcommands keep it wherever they sit, as a
# global option can come first and Codex refuses the flag on them. An older Codex lacks the flag, hence the --help check.
function _ags_codex_own_process
    test -z "$NGTERM_CODEX_KEEP_DAEMON"; or return 1
    for arg in $argv
        switch $arg
            case --
                break
            case --no-daemon --remote '--remote=*' agents queue remote-control app-server
                return 1
        end
    end
    if test (count $argv) -gt 0
        and not contains -- $argv[1] resume fork
        and string match -rq '^[a-z-]+$' -- $argv[1]
        and not string match -q -- '-*' $argv[1]
        return 1
    end
    command codex --help 2>/dev/null | string match -q -- '*--no-daemon*'
end

if not functions -q codex
    function codex --wraps=codex
        if _ags_codex_own_process $argv
            command codex --no-daemon $argv
        else
            command codex $argv
        end
    end
end

function _ags_preexec --on-event fish_preexec
    if string match -r -q $NGTERM_AGENT_RE $argv[1]
        $NGTERM_AGENT_BIN active --blink
        set -g _ags_active 1
    end
end

function _ags_precmd --on-event fish_prompt
    if set -q _ags_active
        $NGTERM_AGENT_BIN idle
        set -e _ags_active
    end
end
