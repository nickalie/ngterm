# ngterm-agent-status — set the current ngterm session's agent-status indicator (Windows).
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File ngterm-agent-status.ps1 active            # agent is busy
#   powershell -NoProfile -ExecutionPolicy Bypass -File ngterm-agent-status.ps1 completed         # agent finished a turn
#   powershell -NoProfile -ExecutionPolicy Bypass -File ngterm-agent-status.ps1 blocked --blink   # agent is waiting on you
#   powershell -NoProfile -ExecutionPolicy Bypass -File ngterm-agent-status.ps1 idle              # clear the indicator
#
# The Windows form of ngterm-agent-status.sh, for Windows PowerShell 5.1 (inbox on every Windows) and PowerShell 7;
# cmd and Git Bash start it through powershell.exe as above, and another script calls it in-process (`& <path> ...`).
#
# States: idle | active | completed | blocked. An optional --blink / --auto-reset
# (and any further args) is forwarded verbatim to `ngtermctl session status`.
#
# Outside ngterm this is a silent no-op, so it is safe to call from any hook.
#
# As a hook it must never interfere with the agent: its output and the CLI's are suppressed (Claude Code injects a
# UserPromptSubmit/SessionStart hook's stdout into the prompt context) and it always exits 0 (a non-zero exit can
# block the turn).
#
# ngtermctl resolution order (the binary that talks to the control socket):
#   1. $env:NGTERMCTL — an explicit override the caller set.
#   2. the absolute bundled-binary path the installer bakes in ($NgtermctlBaked, set by the block it writes at the
#      top), so the hook fires even when the CLI was never put on PATH.
#   3. `ngtermctl` on PATH — the fallback when no override was set and the baked path no longer exists.

if (-not $env:NGTERM_SESSION_ID -or $args.Count -lt 1) { exit 0 } # not inside ngterm (or no state): nothing to do

$state = $args[0]
$rest = @($args | Select-Object -Skip 1)

$ngtermctl = 'ngtermctl'
if ($env:NGTERMCTL) {
  $ngtermctl = $env:NGTERMCTL
} elseif ((Get-Variable -Name NgtermctlBaked -ValueOnly -ErrorAction SilentlyContinue)) {
  $ngtermctl = $NgtermctlBaked
}

# --socket is a SUBCOMMAND option, so it comes AFTER `session status`; passed only when NGTERM_SOCKET is set.
# The pane discriminators go along when the app injected them: NGTERM_PANE (the role, which can go stale) and
# NGTERM_PANE_ID (the surface's stable token, which the app resolves to its current slot, #199); ngtermctl
# validates both, so they pass through verbatim.
$cli = @('session', 'status', $state, '--target', $env:NGTERM_SESSION_ID)
if ($env:NGTERM_SOCKET) { $cli += @('--socket', $env:NGTERM_SOCKET) }
if ($env:NGTERM_PANE) { $cli += @('--pane', $env:NGTERM_PANE) }
if ($env:NGTERM_PANE_ID) { $cli += @('--pane-id', $env:NGTERM_PANE_ID) }
$cli += $rest

try { & $ngtermctl @cli *> $null } catch { }
exit 0
