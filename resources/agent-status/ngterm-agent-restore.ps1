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
# ngterm's chat view: each start and each end also pipe the payload to `ngtermctl agent hook claude SessionStart|SessionEnd`
# under the same guards, as the POSIX form does. The payload is read and handed on as UTF-8 whatever the console's code
# page, so a path arrives as the agent wrote it.
#
# As a hook it must never disturb the agent: no output, always exit 0, and outside ngterm (NGTERM_SESSION_ID unset)
# it does nothing. The payload is matched with a regular expression, as the POSIX form does with bash's.
#
# ngtermctl resolution: $env:NGTERMCTL, else the path the installer bakes in ($NgtermctlBaked), else `ngtermctl` on
# PATH.

trap { exit 0 }

# A background job of Claude Code's daemon (CLAUDE_JOB_DIR is set: Claude Code moves a conversation there on
# /background or on a Left arrow at its empty prompt) has the daemon's NGTERM_*, which name the pane the daemon was
# started in: a session long gone, or another one's. `ngtermctl agent rerun` finds the pane that holds this
# conversation from the payload and runs this hook again as that pane; with none it runs nothing, silently.
if ($env:CLAUDE_JOB_DIR -and -not $env:NGTERM_AGENT_LOCATED) {
  $rerunStatus = 0
  try {
    $rerunCtl = $env:NGTERMCTL
    if (-not $rerunCtl) { $rerunCtl = Get-Variable -Name NgtermctlBaked -ValueOnly -ErrorAction SilentlyContinue }
    if (-not $rerunCtl) { $rerunCtl = 'ngtermctl' }
    $rerun = @('agent', 'rerun', 'claude')
    if ($env:NGTERM_SOCKET) { $rerun += @('--socket', $env:NGTERM_SOCKET) }
    $rerun += @('--', (Get-Process -Id $PID).Path, '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass',
      '-File', $PSCommandPath) + @($args)
    if ([Console]::IsInputRedirected) {
      $rerunInput = [System.IO.StreamReader]::new([Console]::OpenStandardInput(), [System.Text.UTF8Encoding]::new($false))
      $OutputEncoding = [System.Text.UTF8Encoding]::new($false)
      $rerunInput.ReadToEnd() | & $rerunCtl @rerun
      $rerunStatus = $LASTEXITCODE
    }
  } catch { }
  exit $rerunStatus
}

if (-not $env:NGTERM_SESSION_ID) { exit 0 }

$agent = if ($args.Count -gt 0) { $args[0] } else { '' }
$hookEvent = if ($args.Count -gt 1) { $args[1] } else { '' }

$payload = ''
if ([Console]::IsInputRedirected) {
  try {
    $reader = New-Object System.IO.StreamReader([Console]::OpenStandardInput(), (New-Object System.Text.UTF8Encoding $false))
    $read = $reader.ReadToEndAsync()
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

function Get-Ngtermctl {
  if ($env:NGTERMCTL) { return $env:NGTERMCTL }
  $baked = Get-Variable -Name NgtermctlBaked -ValueOnly -ErrorAction SilentlyContinue
  if ($baked) { return $baked }
  return 'ngtermctl'
}

function Invoke-Restore([string[]] $restoreArgs) {
  $ngtermctl = Get-Ngtermctl
  $cli = @('session', 'restore') + $restoreArgs + @('--target', $env:NGTERM_SESSION_ID)
  if ($env:NGTERM_PANE_ID) { $cli += @('--pane-id', $env:NGTERM_PANE_ID) }
  if ($env:NGTERM_SOCKET) { $cli += @('--socket', $env:NGTERM_SOCKET) }
  try { $null | & $ngtermctl @cli *> $null } catch { }
}

# Pipes the payload to `ngtermctl agent hook claude <event>`, the pane addressed as the status hooks address it.
function Send-AgentHook([string] $agentEvent) {
  $ngtermctl = Get-Ngtermctl
  $cli = @('agent', 'hook', 'claude', $agentEvent, '--target', $env:NGTERM_SESSION_ID)
  if ($env:NGTERM_PANE) { $cli += @('--pane', $env:NGTERM_PANE) }
  if ($env:NGTERM_PANE_ID) { $cli += @('--pane-id', $env:NGTERM_PANE_ID) }
  if ($env:NGTERM_SOCKET) { $cli += @('--socket', $env:NGTERM_SOCKET) }
  try {
    # a `New-Object` encoding is wrapped, which Windows PowerShell 5.1 does not take for a native command's input
    $OutputEncoding = [System.Text.UTF8Encoding]::new($false)
    $payload | & $ngtermctl @cli *> $null
  } catch { }
}

function Invoke-ClaudeHook {
  if ($env:CLAUDE_CODE_ENTRYPOINT -cne 'cli') { return }
  if (Get-StringField 'agent_id') { return }
  switch -CaseSensitive ($hookEvent) {
    'start' {
      Send-AgentHook 'SessionStart'
      $id = Get-SessionId
      if ($id) { Invoke-Restore @("claude --resume $id; if (-not `$?) { claude }") }
    }
    'end' {
      Send-AgentHook 'SessionEnd'
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
