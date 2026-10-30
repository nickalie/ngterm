<#
.SYNOPSIS
The Windows app smoke check (docs/plans/20261029-windows-support.md, Task 4): one ngterm.exe on this desktop, on a
private state directory, driven through its control socket.

.DESCRIPTION
Run it on the desktop the window should open on (in a VM: `vmrun runProgramInGuest -interactive`, or an interactive
scheduled task), never on a machine whose own ngterm runs there: the instance is private (its own state directory,
app id, home and control pipe; NGTERM_*, AGTERM_* and ZMX_* stripped), but its window takes the desktop.

The check: the app launches and serves its control socket (the named pipe its socket path names, spoken to directly,
and with -Ctl through `ngtermctl --socket` too: its tree, a typed line and the text read back); the pane runs
PowerShell (a typed command's output read back with `session.text`); `session.split` opens the split and its
PowerShell answers too; a screenshot of the desktop is saved;
the app quits on Ctrl+C (the console's Ctrl event, its orderly quit) with status 0; a second launch restores the
session with its split. The app is stopped by its own process id at the end, whatever happened. Every step is logged
to <StateDir>\smoke.log; the exit status is 0 when every step passed.

.PARAMETER Exe
The ngterm.exe to run (a debug build keeps its console, which the Ctrl+C quit needs).

.PARAMETER Ctl
An ngtermctl.exe; without it the script speaks the control protocol itself.

.PARAMETER StateDir
The private root: state\, home\, the log and the screenshot go there. A fresh directory under TEMP by default.
#>
param(
    [Parameter(Mandatory = $true)] [string] $Exe,
    [string] $Ctl = '',
    [string] $StateDir = (Join-Path $env:TEMP "ngterm-smoke-$PID"),
    [int] $Timeout = 90
)

$ErrorActionPreference = 'Stop'
$root = New-Item -ItemType Directory -Force -Path $StateDir
$log = Join-Path $root 'smoke.log'
$state = Join-Path $root 'state'
$homeDir = Join-Path $root 'home'
New-Item -ItemType Directory -Force -Path $state, $homeDir | Out-Null
$socket = Join-Path $state 'ngterm.sock'
$failures = 0

function Log([string] $line) {
    $stamped = "{0:HH:mm:ss.fff} {1}" -f (Get-Date), $line
    Add-Content -Path $log -Value $stamped
    Write-Output $stamped
}

function Check([bool] $ok, [string] $what) {
    if ($ok) { Log "PASS $what" } else { Log "FAIL $what"; $script:failures++ }
}

# The pipe the socket path names (ngterm_core::paths::control_pipe_name): `ngterm-control-` and the FNV-1a 64 hash of
# the absolute path, case-folded, with `\` separators and no trailing one.
Add-Type -TypeDefinition @'
public static class SmokeHash {
    public static string Fnv1a64(byte[] bytes) {
        ulong hash = 14695981039346656037UL;
        foreach (byte b in bytes) { hash ^= b; unchecked { hash *= 1099511628211UL; } }
        return hash.ToString("x16");
    }
}
'@
function Get-ControlPipeName([string] $path) {
    $normal = [System.IO.Path]::GetFullPath($path).Replace('/', '\').ToLowerInvariant().TrimEnd('\')
    'ngterm-control-' + [SmokeHash]::Fnv1a64([System.Text.Encoding]::UTF8.GetBytes($normal))
}

$pipeName = Get-ControlPipeName $socket

# One control request (`{"cmd": ..., "args": ...}`), its reply line; $null when the pipe is not there.
function Invoke-Control([string] $cmd, [hashtable] $arguments = $null, [int] $connectMs = 2000) {
    $request = @{ cmd = $cmd }
    if ($arguments) { $request.args = $arguments }
    $line = $request | ConvertTo-Json -Compress -Depth 5
    $pipe = New-Object System.IO.Pipes.NamedPipeClientStream('.', $pipeName, [System.IO.Pipes.PipeDirection]::InOut)
    try {
        $pipe.Connect($connectMs)
    } catch {
        $pipe.Dispose()
        return $null
    }
    try {
        $utf8 = New-Object System.Text.UTF8Encoding($false)
        $writer = New-Object System.IO.StreamWriter($pipe, $utf8)
        $writer.AutoFlush = $true
        $writer.Write($line + "`n")
        $reader = New-Object System.IO.StreamReader($pipe, $utf8)
        return $reader.ReadLine()
    } finally {
        $pipe.Dispose()
    }
}

# Polls `session.text` of `pane` until it holds `marker`.
function Wait-Text([string] $marker, [string] $pane = $null) {
    $deadline = (Get-Date).AddSeconds($Timeout)
    $arguments = @{}
    if ($pane) { $arguments.pane = $pane }
    while ((Get-Date) -lt $deadline) {
        $reply = Invoke-Control 'session.text' $arguments
        if ($reply -and $reply.Contains($marker)) { return $true }
        Start-Sleep -Milliseconds 500
    }
    Log "last session.text: $reply"
    return $false
}

function Wait-Socket {
    $deadline = (Get-Date).AddSeconds($Timeout)
    while ((Get-Date) -lt $deadline) {
        $reply = Invoke-Control 'version' $null 500
        if ($reply) { return $reply }
        Start-Sleep -Milliseconds 300
    }
    return $null
}

# Sends the console's Ctrl+C to the console process `id` runs on, from a helper of its own (attaching to another
# console detaches the caller from its own).
function Send-CtrlC([int] $id) {
    $helper = @"
Add-Type -Namespace Smoke -Name Console -MemberDefinition '
[DllImport("kernel32.dll", SetLastError = true)] public static extern bool AttachConsole(uint pid);
[DllImport("kernel32.dll", SetLastError = true)] public static extern bool FreeConsole();
[DllImport("kernel32.dll", SetLastError = true)] public static extern bool SetConsoleCtrlHandler(System.IntPtr handler, bool add);
[DllImport("kernel32.dll", SetLastError = true)] public static extern bool GenerateConsoleCtrlEvent(uint ctrlEvent, uint group);
'
[Smoke.Console]::FreeConsole() | Out-Null
if (-not [Smoke.Console]::AttachConsole($id)) { exit 2 }
[Smoke.Console]::SetConsoleCtrlHandler([System.IntPtr]::Zero, `$true) | Out-Null
if (-not [Smoke.Console]::GenerateConsoleCtrlEvent(0, 0)) { exit 3 }
Start-Sleep -Milliseconds 500
exit 0
"@
    $encoded = [Convert]::ToBase64String([System.Text.Encoding]::Unicode.GetBytes($helper))
    $sent = Start-Process -FilePath 'powershell.exe' -ArgumentList '-NoProfile', '-NonInteractive', '-EncodedCommand', $encoded -WindowStyle Hidden -PassThru -Wait
    return $sent.ExitCode
}

function Start-App {
    # a console window of its own: the app's log lines and Ctrl events
    Start-Process -FilePath $Exe -WorkingDirectory $homeDir -PassThru
}

function Stop-App($process) {
    if ($process -and -not $process.HasExited) {
        Log "stopping pid $($process.Id) by its process id"
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
        $process.WaitForExit(10000) | Out-Null
    }
}

Get-ChildItem env: | Where-Object { $_.Name -match '^(NGTERM|AGTERM|ZMX)_' } | ForEach-Object {
    Remove-Item -LiteralPath "env:$($_.Name)"
}
$env:NGTERM_STATE_DIR = $state
$env:NGTERM_APP_ID = 'com.nickalie.ngterm.smoke'
$env:HOME = $homeDir
Log "exe $Exe; state $state; pipe \\.\pipe\$pipeName"

$app = $null
try {
    $app = Start-App
    Log "launched pid $($app.Id)"
    $version = Wait-Socket
    Check ($null -ne $version) "the control socket answers: $version"
    if ($Ctl) {
        $ctlTree = & $Ctl tree --socket $socket 2>&1 | Out-String
        Check ($LASTEXITCODE -eq 0) "ngtermctl tree --socket: $ctlTree"
        $typed = & $Ctl session type --socket $socket "Write-Output ('smoke' + '-ctl')`r" 2>&1 | Out-String
        Check ($LASTEXITCODE -eq 0) "ngtermctl session type --socket: $typed"
        Check (Wait-Text 'smoke-ctl') 'the line ngtermctl typed runs in the pane'
        $text = & $Ctl session text --socket $socket 2>&1 | Out-String
        Check ($text.Contains('smoke-ctl')) 'ngtermctl session text --socket reads it back'
    }
    $tree = Invoke-Control 'tree'
    Log "tree: $tree"
    Invoke-Control 'session.type' @{ text = "Write-Output ('smoke' + '-left-' + `$PSVersionTable.PSEdition)`r" } | Out-Null
    Check (Wait-Text 'smoke-left-') 'PowerShell in the pane runs a typed command'
    $split = Invoke-Control 'session.split' @{ mode = 'on' }
    Check ($split -match '"ok":true') "session.split: $split"
    Start-Sleep -Seconds 2
    Invoke-Control 'session.type' @{ text = "Write-Output ('smoke' + '-right')`r"; pane = 'right' } | Out-Null
    Check (Wait-Text 'smoke-right' 'right') 'PowerShell in the split pane runs a typed command'
    $tree = Invoke-Control 'tree'
    Check ($tree -match '"split":true') "the tree shows the split: $tree"
    Start-Sleep -Seconds 1
    Add-Type -AssemblyName System.Windows.Forms, System.Drawing
    $bounds = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $bitmap = New-Object System.Drawing.Bitmap($bounds.Width, $bounds.Height)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.CopyFromScreen($bounds.Location, [System.Drawing.Point]::Empty, $bounds.Size)
    $shot = Join-Path $root 'window.png'
    $bitmap.Save($shot, [System.Drawing.Imaging.ImageFormat]::Png)
    $graphics.Dispose(); $bitmap.Dispose()
    Log "screenshot $shot"
    $sent = Send-CtrlC $app.Id
    Check ($sent -eq 0) "Ctrl+C sent to the app's console (helper status $sent)"
    $quit = $app.WaitForExit(15000)
    Check ($quit -and $app.ExitCode -eq 0) "the app quit on Ctrl+C with status $(if ($quit) { $app.ExitCode } else { 'none: still running' })"
    Stop-App $app
    $app = Start-App
    Log "relaunched pid $($app.Id)"
    Check ($null -ne (Wait-Socket)) 'the relaunched app answers'
    $deadline = (Get-Date).AddSeconds($Timeout)
    do {
        $tree = Invoke-Control 'tree'
        if ($tree -match '"hasSplit":true') { break }
        Start-Sleep -Milliseconds 500
    } while ((Get-Date) -lt $deadline)
    Check ($tree -match '"hasSplit":true') "the session comes back with its split: $tree"
} catch {
    Log "ERROR $_"
    $failures++
} finally {
    Stop-App $app
}
Log "done: $failures failure(s)"
exit $failures
