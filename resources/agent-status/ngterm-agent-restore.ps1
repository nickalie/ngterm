# ngterm-agent-restore — keep a pane's restore pin on the Claude Code conversation it is in (Windows).
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File ngterm-agent-restore.ps1 claude start   # Claude Code SessionStart
#   powershell -NoProfile -ExecutionPolicy Bypass -File ngterm-agent-restore.ps1 claude end     # Claude Code SessionEnd
#
# The Windows form of ngterm-agent-restore.sh, for Windows PowerShell 5.1 and PowerShell 7; that script's header
# holds what it pins, what it skips and why. One difference: the pin is typed into the pane's login shell, which on
# Windows is PowerShell, and Windows PowerShell 5.1 has no `||`, so the pin is
#
#   claude --resume <session_id>; if (-not $?) { claude }
#
# which both PowerShells take: `$?` is false after a native command that exits non-zero.
#
# As a hook it must never disturb the agent: no output, always exit 0, and outside ngterm (NGTERM_SESSION_ID unset)
# it does nothing. The payload is matched with a regular expression, as the POSIX form does with bash's.
#
# ngtermctl resolution: $env:NGTERMCTL, else the path the installer bakes in ($NgtermctlBaked), else `ngtermctl` on
# PATH.

trap { exit 0 }

if (-not $env:NGTERM_SESSION_ID) { exit 0 }

$agent = if ($args.Count -gt 0) { $args[0] } else { '' }
$hookEvent = if ($args.Count -gt 1) { $args[1] } else { '' }

$payload = ''
if ([Console]::IsInputRedirected) {
  try {
    $read = [Console]::In.ReadToEndAsync()
    if ($read.Wait(5000)) { $payload = $read.Result }
  } catch { }
}

# A top-level string field of the payload. The key must not follow a backslash: a key quoted inside a string value is
# escaped (\"session_id\"), so the match cannot come from a message or a path. Claude Code writes the ids first.
function Get-StringField([string] $name) {
  $pattern = '(^|[^\\])"' + [regex]::Escape($name) + '"\s*:\s*"([^"\\]*)"'
  if ($payload -cmatch $pattern) { return $Matches[2] }
  return $null
}

function Get-SessionId {
  $id = Get-StringField 'session_id'
  if ($id -and $id -cmatch '^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$') {
    return $id
  }
  return $null
}

function Invoke-Restore([string[]] $restoreArgs) {
  $ngtermctl = 'ngtermctl'
  if ($env:NGTERMCTL) {
    $ngtermctl = $env:NGTERMCTL
  } elseif ((Get-Variable -Name NgtermctlBaked -ValueOnly -ErrorAction SilentlyContinue)) {
    $ngtermctl = $NgtermctlBaked
  }
  $cli = @('session', 'restore') + $restoreArgs + @('--target', $env:NGTERM_SESSION_ID)
  if ($env:NGTERM_PANE_ID) { $cli += @('--pane-id', $env:NGTERM_PANE_ID) }
  if ($env:NGTERM_SOCKET) { $cli += @('--socket', $env:NGTERM_SOCKET) }
  try { $null | & $ngtermctl @cli *> $null } catch { }
}

function Invoke-ClaudeHook {
  if ($env:CLAUDE_CODE_ENTRYPOINT -cne 'cli') { return }
  if (Get-StringField 'agent_id') { return }
  switch -CaseSensitive ($hookEvent) {
    'start' {
      $id = Get-SessionId
      if ($id) { Invoke-Restore @("claude --resume $id; if (-not `$?) { claude }") }
    }
    'end' {
      $reason = Get-StringField 'reason'
      if ($reason -ceq 'prompt_input_exit' -or $reason -ceq 'logout') { Invoke-Restore @('--clear') }
    }
  }
}

try {
  switch -CaseSensitive ($agent) {
    'claude' { Invoke-ClaudeHook *> $null }
  }
} catch { }
exit 0
