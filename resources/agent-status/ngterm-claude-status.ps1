# Claude Code status adapter installed by ngterm's Help ▸ Install Agent Status Hooks command (Windows).
#
# The Windows form of ngterm-claude-status.sh, for Windows PowerShell 5.1 and PowerShell 7: Claude Code runs a hook's
# command in Git Bash, or in PowerShell without Git Bash, and the installer's command starts this script through
# powershell.exe, which both run. Its header holds the reasoning; in short:
#
# A worker agent spawned from inside a session inherits the spawner's NGTERM_* environment, so its own hooks would
# repaint the SPAWNER's row. The hook is a descendant of the agent that fired it, so exactly one agent between here and
# the pane means "I am the pane's agent"; a second one means another agent spawned mine, so stay silent. A consecutive
# run of one name counts once. The walk reads the process table (Win32_Process: each process's parent, image name and
# command line) where the POSIX form asks `ps`; an image name is matched without its `.exe` and in any case, as
# Windows names it. A parent id that names a process started after its child is a reused id: the chain is severed.
#
# It fails OPEN (reports) whenever the chain is unreadable or severed: a missed guard is the behavior without this
# adapter, while a false silence is a bug with no symptom. The argv is forwarded to the shared wrapper verbatim, with
# one exception: a Stop whose payload lists work still in flight in `background_tasks` posts `active --blink` instead
# of `completed`, so the row keeps pulsing until a Stop with nothing in flight, or the SessionEnd hook the installer
# adds. Only a `completed` call reads stdin, where Claude Code writes the payload as JSON; a key quoted inside a string
# value is escaped (`\"background_tasks\"`), so the match cannot hit the last assistant message.

if (-not $env:NGTERM_SESSION_ID) { exit 0 }

$statusWrapper = $env:NGTERM_STATUS_WRAPPER
if (-not $statusWrapper) { $statusWrapper = Join-Path $PSScriptRoot 'ngterm-agent-status.ps1' }

# exact names, not a pattern: the union of the hook-driven agents and the shell integration's NGTERM_AGENT_RE
# default, as in the POSIX form; keep the lists in sync. An overridden NGTERM_AGENT_RE extends the set, matched
# against the same image name; an expression .NET cannot compile lands on "not an agent", keeping the walk fail-open.
$agentNames = @('claude', 'codex', 'kimi', 'opencode', 'pi', 'gemini', 'cursor-agent', 'aider', 'crush', 'goose')

function Test-Agent([string] $name) {
  if ($agentNames -contains $name) { return $true }
  if ($env:NGTERM_AGENT_RE) {
    try { if ($name -cmatch $env:NGTERM_AGENT_RE) { return $true } } catch { }
  }
  return $false
}

# The file name of a path or command-line token without a trailing `.exe`.
function Get-BaseName([string] $path) {
  $leaf = ($path -split '[\\/]')[-1]
  return ($leaf -replace '\.exe$', '')
}

# The argument after the program in a Windows command line (quoted or bare), or nothing.
function Get-HostedScript([string] $commandLine) {
  if ($commandLine -match '^\s*(?:"[^"]*"|\S+)\s+(?:"([^"]*)"|(\S+))') {
    if ($Matches[1]) { return $Matches[1] }
    return $Matches[2]
  }
  return ''
}

# Whether a second agent sits above this hook, between it and the pane: then it is a worker's.
function Test-Worker {
  try {
    $table = @{}
    $processes = Get-CimInstance -ClassName Win32_Process -Property ProcessId, ParentProcessId, Name, CommandLine, CreationDate -ErrorAction Stop
    foreach ($process in $processes) { $table[[int] $process.ProcessId] = $process }
  } catch {
    return $false
  }
  $child = $table[[int] $PID]
  if (-not $child) { return $false }
  $agents = 0
  $previous = ''
  for ($step = 0; $step -lt 8; $step++) {
    $parentId = [int] $child.ParentProcessId
    if ($parentId -le 4) { break } # the idle and system processes: the top
    $process = $table[$parentId]
    if (-not $process) { break }
    if ($process.CreationDate -and $child.CreationDate -and $process.CreationDate -gt $child.CreationDate) { break }
    $base = Get-BaseName $process.Name
    # node/bun-hosted CLIs present as `node ...\cli.js`, so test the script too
    if (@('node', 'bun', 'deno', 'python3', 'python') -contains $base) {
      $hosted = Get-BaseName (Get-HostedScript $process.CommandLine)
      if (Test-Agent $hosted) { $base = $hosted }
    }
    if (Test-Agent $base) {
      if ($base -ne $previous) { $agents++ }
      $previous = $base
    } else {
      $previous = ''
    }
    if ($agents -gt 1) { return $true } # a second agent above mine: I am a worker, not the pane's agent
    if (@('login', 'ngterm') -contains $base) { break } # reached the pane boundary
    $child = $process
  }
  return $false
}

if (Test-Worker) { exit 0 }

if ($args.Count -gt 0 -and $args[0] -ceq 'completed' -and [Console]::IsInputRedirected) {
  $payload = ''
  try {
    $read = [Console]::In.ReadToEndAsync()
    if ($read.Wait(2000)) { $payload = $read.Result }
  } catch { }
  $stopPattern = '"hook_event_name"\s*:\s*"Stop"'
  $inFlightPattern = '"background_tasks"\s*:\s*\[\s*\{'
  if ($payload -cmatch $stopPattern -and $payload -cmatch $inFlightPattern) {
    try { & $statusWrapper active --blink *> $null } catch { }
    exit 0
  }
}

try { & $statusWrapper @args *> $null } catch { }
exit 0
