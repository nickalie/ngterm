<#
.SYNOPSIS
The Windows app smoke check (docs/plans/20261029-windows-support.md, Task 4): one ngterm.exe on this desktop, on a
private state directory, driven through its control socket.

.DESCRIPTION
Run it on the desktop the window should open on (in a VM: `vmrun runProgramInGuest -interactive`, or an interactive
scheduled task), never on a machine whose own ngterm runs there: the instance is private (its own state directory,
app id, home and control pipe; NGTERM_*, AGTERM_* and ZMX_* stripped), but its window takes the desktop.

The check: the app launches and serves its control socket (the named pipe its socket path names, spoken to directly, and
with -Ctl through `ngtermctl --socket` too: its tree, a typed line and the text read back); the pane runs PowerShell (a
typed command's output read back with `session.text`); `session.split` opens the split and its PowerShell answers too; a
screenshot of the desktop is saved; the app quits on Ctrl+C (the console's Ctrl event, its orderly quit) with status 0;
a second launch restores the session with its split; keys typed on the desktop (`SendKeys`: text, Enter, the Up arrow)
run in its focused pane; the quick terminal shows, runs PowerShell and zooms (screenshots `quick.png`,
`quick-zoomed.png`); a launch beside it forwards to it over the instance pipe and exits 0; a flip of the system's app
mode (`AppsUseLightTheme`, announced as Settings does with `WM_SETTINGCHANGE` `ImmersiveColorSet`, then put back)
reloads the config of a theme with both slots; the session's end (`WM_QUERYENDSESSION`, `WM_ENDSESSION` sent to the
app's hidden window) quits it. The app is stopped by its own process id at the end, whatever happened. Every step is
logged to <StateDir>\smoke.log; the exit status is 0 when every step passed.

.PARAMETER Exe
The ngterm.exe to run (a debug build keeps its console, which the Ctrl+C quit needs), in a staged tree
(`bin\ngterm.exe` beside `share\ghostty\themes`, scripts/stage.sh's layout), so the theme slots find Ghostty's themes.

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
function Invoke-Control([string] $cmd, [hashtable] $arguments = $null, [int] $connectMs = 2000, [string] $target = '') {
    $request = @{ cmd = $cmd }
    if ($arguments) { $request.args = $arguments }
    if ($target) { $request.target = $target }
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

# The app's hidden top-level window (`system_window.rs`), the session's end sent to it, and a setting change broadcast.
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class SmokeWindows {
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    static extern IntPtr FindWindowEx(IntPtr parent, IntPtr after, string cls, string title);
    [DllImport("user32.dll")]
    static extern uint GetWindowThreadProcessId(IntPtr window, out uint pid);
    [DllImport("user32.dll", EntryPoint = "SendMessageTimeoutW")]
    static extern IntPtr SendNumbers(IntPtr window, uint msg, UIntPtr w, IntPtr l, uint flags, uint ms, out UIntPtr result);
    [DllImport("user32.dll", EntryPoint = "SendMessageTimeoutW", CharSet = CharSet.Unicode)]
    static extern IntPtr SendText(IntPtr window, uint msg, UIntPtr w, string l, uint flags, uint ms, out UIntPtr result);
    [DllImport("user32.dll")]
    static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")]
    static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")]
    static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra);
    const uint AbortIfHung = 0x2;
    // brings the first visible top-level window of `pid` to the front; false when it has none or Windows refused.
    // An Alt press and release first: Windows lets the process that sent the last input set the foreground window.
    public static bool Raise(uint pid) {
        IntPtr window = IntPtr.Zero;
        while ((window = FindWindowEx(IntPtr.Zero, window, null, null)) != IntPtr.Zero) {
            uint owner;
            GetWindowThreadProcessId(window, out owner);
            if (owner == pid && IsWindowVisible(window)) {
                keybd_event(0x12, 0, 0, UIntPtr.Zero);
                keybd_event(0x12, 0, 2, UIntPtr.Zero);
                return SetForegroundWindow(window);
            }
        }
        return false;
    }
    public static IntPtr SystemWindow(uint pid) {
        IntPtr window = IntPtr.Zero;
        while ((window = FindWindowEx(IntPtr.Zero, window, "ngterm-system-window", null)) != IntPtr.Zero) {
            uint owner;
            GetWindowThreadProcessId(window, out owner);
            if (owner == pid) { return window; }
        }
        return IntPtr.Zero;
    }
    // the window's answer, or -1 when the call failed or timed out
    public static long Send(IntPtr window, uint msg, ulong w, long l, uint ms) {
        UIntPtr result;
        IntPtr sent = SendNumbers(window, msg, new UIntPtr(w), new IntPtr(l), AbortIfHung, ms, out result);
        return sent == IntPtr.Zero ? -1 : (long)result.ToUInt64();
    }
    public static void BroadcastSettingChange(string area) {
        UIntPtr result;
        SendText(new IntPtr(0xffff), 0x001A, UIntPtr.Zero, area, AbortIfHung, 5000, out result);
    }
}
'@

# Polls the app's log until it holds `line`.
function Wait-Log([string] $line) {
    $file = Join-Path $state 'ngterm.log'
    $deadline = (Get-Date).AddSeconds($Timeout)
    while ((Get-Date) -lt $deadline) {
        if ((Test-Path $file) -and (Select-String -Path $file -SimpleMatch $line -Quiet)) { return $true }
        Start-Sleep -Milliseconds 300
    }
    return $false
}

# Saves a screenshot of the desktop as `name` under the root.
function Save-Screen([string] $name) {
    $bounds = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $bitmap = New-Object System.Drawing.Bitmap($bounds.Width, $bounds.Height)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.CopyFromScreen($bounds.Location, [System.Drawing.Point]::Empty, $bounds.Size)
    $shot = Join-Path $root $name
    $bitmap.Save($shot, [System.Drawing.Imaging.ImageFormat]::Png)
    $graphics.Dispose(); $bitmap.Dispose()
    Log "screenshot $shot"
}

# Polls `session.text` of both panes until one holds `marker` `count` times.
function Wait-EitherPane([string] $marker, [int] $count = 1) {
    $deadline = (Get-Date).AddSeconds($Timeout)
    while ((Get-Date) -lt $deadline) {
        foreach ($pane in 'left', 'right') {
            $reply = Invoke-Control 'session.text' @{ pane = $pane }
            if ($reply -and ($reply -split [regex]::Escape($marker)).Count - 1 -ge $count) { return $true }
        }
        Start-Sleep -Milliseconds 500
    }
    return $false
}

# Polls `quick.text` until it holds `marker`.
function Wait-Quick([string] $marker) {
    $deadline = (Get-Date).AddSeconds($Timeout)
    while ((Get-Date) -lt $deadline) {
        $reply = Invoke-Control 'quick.text'
        if ($reply -and $reply.Contains($marker)) { return $true }
        Start-Sleep -Milliseconds 500
    }
    Log "last quick.text: $reply"
    return $false
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

Add-Type -AssemblyName System.Windows.Forms, System.Drawing

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
    Save-Screen 'window.png'
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

    # the keyboard: keys typed on the desktop, the window brought to the front, reach the focused pane
    Check ([SmokeWindows]::Raise([uint32]$app.Id)) 'the app window comes to the front'
    Start-Sleep -Seconds 1
    # Escape first: a sheet the launch shows (the first-run welcome) takes the keyboard until dismissed
    [System.Windows.Forms.SendKeys]::SendWait('{ESC}')
    Start-Sleep -Milliseconds 500
    [System.Windows.Forms.SendKeys]::SendWait("Write-Output {(}'Smoke'{+}'-Keys'{)}{ENTER}")
    $typed = Wait-EitherPane 'Smoke-Keys'
    # the Up arrow brings the line back from PowerShell's history, and Enter runs it again
    [System.Windows.Forms.SendKeys]::SendWait('{UP}{ENTER}')
    $again = Wait-EitherPane 'Smoke-Keys' 2
    Save-Screen 'keys.png'
    Check $typed 'keys typed on the desktop run in the focused pane'
    Check $again 'the Up arrow and Enter typed on the desktop run the line again'

    # the quick terminal: its window host shows PowerShell, zoomed too
    $shown = Invoke-Control 'quick' @{ mode = 'show' }
    Check ($shown -match '"ok":true') "quick show: $shown"
    Check ((Invoke-Control 'tree') -match '"quickVisible":true') 'the tree shows the quick terminal'
    Invoke-Control 'quick.type' @{ text = "Write-Output ('smoke' + '-quick')`r" } | Out-Null
    Check (Wait-Quick 'smoke-quick') 'PowerShell in the quick terminal runs a typed command'
    Save-Screen 'quick.png'
    $zoom = Invoke-Control 'surface.zoom' @{ mode = 'show' } 2000 'quick'
    Check ($zoom -match '"ok":true') "surface.zoom quick show: $zoom"
    Start-Sleep -Seconds 2
    Save-Screen 'quick-zoomed.png'
    $unzoom = Invoke-Control 'surface.zoom' @{ mode = 'hide' } 2000 'quick'
    Check ($unzoom -match '"ok":true') "surface.zoom quick hide: $unzoom"
    Invoke-Control 'quick' @{ mode = 'hide' } | Out-Null
    Check ((Invoke-Control 'tree') -match '"quickVisible":false') 'quick hide hides it'

    # single instance: a launch beside the running one hands it an activation and leaves
    $second = Start-App
    $left = $second.WaitForExit(30000)
    Check ($left -and $second.ExitCode -eq 0) "a second launch exits with status $(if ($left) { $second.ExitCode } else { 'none: still running' })"
    Stop-App $second
    Check (Wait-Log 'activation requested over the instance pipe') 'the running app took the activation'

    # appearance: both theme slots set, so a flip of the system's app mode reloads the config
    # bundled themes: a staged build's share\ghostty\themes (scripts/stage.sh's layout)
    $set = Invoke-Control 'theme.set' @{ light = 'Dracula'; dark = 'Nord' }
    Check ($set -match '"ok":true') "theme.set of both slots: $set"
    $personalize = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize'
    $light = (Get-ItemProperty -Path $personalize -Name AppsUseLightTheme -ErrorAction SilentlyContinue).AppsUseLightTheme
    if ($null -eq $light) { $light = 1 }
    try {
        Set-ItemProperty -Path $personalize -Name AppsUseLightTheme -Value ([int](-not $light)) -Type DWord
        [SmokeWindows]::BroadcastSettingChange('ImmersiveColorSet')
        Check (Wait-Log 'appearance: the desktop flipped') 'a flip of the system app mode reloads the config'
    } finally {
        Set-ItemProperty -Path $personalize -Name AppsUseLightTheme -Value ([int]$light) -Type DWord
        [SmokeWindows]::BroadcastSettingChange('ImmersiveColorSet')
    }

    # the session's end: asked, then told; the app quits
    $window = [SmokeWindows]::SystemWindow([uint32]$app.Id)
    Check ($window -ne [IntPtr]::Zero) "the app's hidden window is there ($window)"
    $logoff = [long]0x80000000
    $query = [SmokeWindows]::Send($window, 0x0011, 0, $logoff, 5000)
    Check ($query -eq 1) "WM_QUERYENDSESSION lets the session end (answer $query)"
    [SmokeWindows]::Send($window, 0x0016, 1, $logoff, 15000) | Out-Null
    $ended = $app.WaitForExit(15000)
    Check ($ended -and $app.ExitCode -eq 0) "the app quit at the session's end with status $(if ($ended) { $app.ExitCode } else { 'none: still running' })"
    Check (Wait-Log 'the session is ending: quitting') 'the session end ran the quit'
} catch {
    Log "ERROR $_"
    $failures++
} finally {
    Stop-App $app
}
Log "done: $failures failure(s)"
exit $failures
