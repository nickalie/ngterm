<#
.SYNOPSIS
The Windows app smoke check (docs/plans/20261029-windows-support.md, Tasks 4 and 5): one ngterm.exe on this desktop, on
a private state directory, driven through its control socket.

.DESCRIPTION
Run it on the desktop the window should open on (in a VM: `vmrun runProgramInGuest -interactive`, or an interactive
scheduled task), never on a machine whose own ngterm runs there: the instance is private (its own state directory,
app id, home and control pipe; NGTERM_*, AGTERM_* and ZMX_* stripped), but its window takes the desktop.

The check: the app launches and serves its control socket (the named pipe its socket path names, spoken to directly, and
with -Ctl through `ngtermctl --socket` too: its tree, a typed line and the text read back); the pane runs PowerShell (a
typed command's output read back with `session.text`), and a ^C typed through `session.type` interrupts a `ping -t`
there; `session.split` opens the split and its PowerShell answers too; a
screenshot of the desktop is saved; the app quits on Ctrl+C (the console's Ctrl event, its orderly quit) with status 0
(a debug build; a release build has no console, so it quits at the session's end instead, the step below);
a second launch restores the session with its split; keys typed on the desktop (`SendKeys`: text, Enter, the Up arrow)
run in its focused pane, and Ctrl+C pressed there interrupts a `ping -t` (`ctrl-c.png`);
the quick terminal shows, runs PowerShell and zooms (screenshots `quick.png`,
`quick-zoomed.png`); a launch beside it forwards to it over the instance pipe and exits 0; a flip of the system's app
mode (`AppsUseLightTheme`, announced as Settings does with `WM_SETTINGCHANGE` `ImmersiveColorSet`, then put back)
reloads the config of a theme with both slots; the desktop integrations: keymap.conf's `global-hotkey`, pressed while
Character Map has the keyboard, shows the quick terminal and pressed again hides it (`hotkey.png`); a `notify` to a session
not shown posts a toast under the app's id, a click on it where Windows shows it reveals that session (`toast-N.png`,
`toast-clicked.png`), a toast sits in its session's slot of the notification center (read back from its history), a
second one replaces it, and it is withdrawn once its session is selected; a status sound plays (upstream's names have no Windows sound alias:
the alert); a resume from sleep (`WM_POWERBROADCAST` sent to the hidden window) retries the remote links; with the
keyboard-layout lock on Russian (`LoadKeyboardLayout`, unloaded at the end) the pane types Cyrillic for the US keys;
the session's end (`WM_QUERYENDSESSION`, `WM_ENDSESSION` sent to the app's hidden window) quits it; `WM_CLOSE` to the
last window asks first (it runs a session), and its Close quits the app with status 0 after the quit's save
(`close-asked.png`). The app is stopped by its own process id at the end, whatever happened. Every step is
logged to <StateDir>\smoke.log; the exit status is 0 when every step passed.

.PARAMETER Exe
The ngterm.exe to run, a debug or a release build (only a debug build keeps its console, which the Ctrl+C quit needs:
on a release build that step is skipped, said so in the log), in a staged tree
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

# Writes `text` as UTF-8 with no BOM, under Windows PowerShell 5.1 too (its `Set-Content -Encoding utf8` writes one).
function Write-Utf8([string] $path, [string] $text) {
    [System.IO.File]::WriteAllText($path, $text, (New-Object System.Text.UTF8Encoding($false)))
}

# Whether the executable at `path` is a console program (its PE header's subsystem, 3): a debug build is, a release
# build (`windows_subsystem = "windows"`, 2) has no console for a Ctrl event to reach.
function Test-ConsoleProgram([string] $path) {
    $bytes = [System.IO.File]::ReadAllBytes($path)
    $header = [System.BitConverter]::ToInt32($bytes, 0x3C)
    # the optional header follows the 4-byte signature and the 20-byte file header; Subsystem is its 68th byte
    [System.BitConverter]::ToUInt16($bytes, $header + 24 + 68) -eq 3
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
    Log "last session.text: $reply" | Out-Null
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
    static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")]
    static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra);
    const uint AbortIfHung = 0x2;
    // whether the foreground window is one of `pid`'s
    public static bool InFront(uint pid) {
        uint owner;
        GetWindowThreadProcessId(GetForegroundWindow(), out owner);
        return owner == pid;
    }
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
    // the first visible GPUI window of `pid` (class `Zed::Window`): the main window while the quick terminal is hidden
    public static IntPtr MainWindow(uint pid) {
        IntPtr window = IntPtr.Zero;
        while ((window = FindWindowEx(IntPtr.Zero, window, "Zed::Window", null)) != IntPtr.Zero) {
            uint owner;
            GetWindowThreadProcessId(window, out owner);
            if (owner == pid && IsWindowVisible(window)) { return window; }
        }
        return IntPtr.Zero;
    }
    // brings `window` to the front, as Raise does
    public static bool Focus(IntPtr window) {
        keybd_event(0x12, 0, 0, UIntPtr.Zero);
        keybd_event(0x12, 0, 2, UIntPtr.Zero);
        return SetForegroundWindow(window);
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

# How many times `marker` shows in both panes together.
function Count-EitherPane([string] $marker) {
    $count = 0
    foreach ($pane in 'left', 'right') {
        $reply = Invoke-Control 'session.text' @{ pane = $pane }
        if ($reply) { $count += ($reply -split [regex]::Escape($marker)).Count - 1 }
    }
    return $count
}

# Polls both panes until `marker` shows `count` times in them together.
function Wait-EitherPane-Count([string] $marker, [int] $count) {
    $deadline = (Get-Date).AddSeconds($Timeout)
    while ((Get-Date) -lt $deadline) {
        if ((Count-EitherPane $marker) -ge $count) { return $true }
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
    Log "last quick.text: $reply" | Out-Null
    return $false
}

# The desktop integrations' helpers: keys pressed on the desktop (`keybd_event`, as any keyboard), a click, a keyboard
# layout loaded for the session, and a window message whose answer is not waited for.
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class SmokeDesktop {
    [DllImport("user32.dll")]
    static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra);
    [DllImport("user32.dll")]
    static extern uint MapVirtualKeyW(uint code, uint type);
    [DllImport("user32.dll")]
    static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")]
    static extern void mouse_event(uint flags, int x, int y, uint data, UIntPtr extra);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    static extern IntPtr LoadKeyboardLayoutW(string klid, uint flags);
    [DllImport("user32.dll")]
    static extern bool UnloadKeyboardLayout(IntPtr layout);
    [DllImport("user32.dll")]
    static extern int GetKeyboardLayoutList(int count, IntPtr[] list);
    [DllImport("user32.dll")]
    static extern bool PostMessageW(IntPtr window, uint msg, UIntPtr w, IntPtr l);
    [DllImport("user32.dll")]
    public static extern bool SetProcessDPIAware();
    // presses the virtual keys in order, each with its scan code
    public static void Press(byte[] keys) {
        foreach (byte key in keys) { keybd_event(key, (byte)MapVirtualKeyW(key, 0), 0, UIntPtr.Zero); }
    }
    // releases them in reverse
    public static void Release(byte[] keys) {
        for (int i = keys.Length - 1; i >= 0; i--) {
            keybd_event(keys[i], (byte)MapVirtualKeyW(keys[i], 0), 2, UIntPtr.Zero);
        }
    }
    // a left click at the screen point
    public static void Click(int x, int y) {
        SetCursorPos(x, y);
        mouse_event(0x2, 0, 0, 0, UIntPtr.Zero);
        mouse_event(0x4, 0, 0, 0, UIntPtr.Zero);
    }
    // loads a keyboard layout for the session; IntPtr.Zero when it was loaded already (so it is not unloaded after)
    public static IntPtr LoadLayout(string klid) {
        int count = GetKeyboardLayoutList(0, null);
        IntPtr[] before = new IntPtr[count];
        GetKeyboardLayoutList(count, before);
        IntPtr layout = LoadKeyboardLayoutW(klid, 0x80);
        return Array.IndexOf(before, layout) >= 0 ? IntPtr.Zero : layout;
    }
    public static void UnloadLayout(IntPtr layout) { if (layout != IntPtr.Zero) { UnloadKeyboardLayout(layout); } }
    public static bool Post(IntPtr window, uint msg, ulong w, long l) {
        return PostMessageW(window, msg, new UIntPtr(w), new IntPtr(l));
    }
}
'@
# screenshots and clicks in the screen's own pixels
[SmokeDesktop]::SetProcessDPIAware() | Out-Null

# A chord pressed on the desktop: the keys down in order, up in reverse.
function Send-Chord([byte[]] $keys) {
    [SmokeDesktop]::Press($keys)
    Start-Sleep -Milliseconds 50
    [SmokeDesktop]::Release($keys)
}

# A click on the newest toast where Windows shows it: the bottom right of the primary screen's work area, on its text
# (left of its buttons). UI Automation does not reach the toast's window from a plain process.
function Click-Toast {
    $area = [System.Windows.Forms.Screen]::PrimaryScreen.WorkingArea
    $graphics = [System.Drawing.Graphics]::FromHwnd([IntPtr]::Zero)
    $scale = $graphics.DpiX / 96
    $graphics.Dispose()
    $x = [int]($area.Right - 280 * $scale)
    $y = [int]($area.Bottom - 50 * $scale)
    Log "clicking the toast at $x,$y (work area $area, scale $scale)" | Out-Null
    [SmokeDesktop]::Click($x, $y)
    return $true
}

# The app's toasts in the notification center (Windows PowerShell's WinRT projection, from a helper of its own): one
# `group|tag|xml` line each.
function Get-Toasts {
    $helper = @"
`$null = [Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime]
foreach (`$toast in [Windows.UI.Notifications.ToastNotificationManager]::History.GetHistory('$appId')) {
    '{0}|{1}|{2}' -f `$toast.Group, `$toast.Tag, `$toast.Content.GetXml()
}
"@
    $encoded = [Convert]::ToBase64String([System.Text.Encoding]::Unicode.GetBytes($helper))
    @(& powershell.exe -NoProfile -NonInteractive -EncodedCommand $encoded 2>&1 | ForEach-Object { "$_" })
}

# Clears the app id's toasts from the notification center (Windows PowerShell's WinRT projection).
function Clear-Toasts {
    $helper = @"
`$null = [Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime]
[Windows.UI.Notifications.ToastNotificationManager]::History.Clear('$appId')
"@
    $encoded = [Convert]::ToBase64String([System.Text.Encoding]::Unicode.GetBytes($helper))
    & powershell.exe -NoProfile -NonInteractive -EncodedCommand $encoded 2>&1 | Out-Null
}

# Polls the app's log for `line`, up to `seconds`.
function Wait-Log-For([string] $line, [int] $seconds) {
    $file = Join-Path $state 'ngterm.log'
    $deadline = (Get-Date).AddSeconds($seconds)
    do {
        if ((Test-Path $file) -and (Select-String -Path $file -SimpleMatch $line -Quiet)) { return $true }
        Start-Sleep -Milliseconds 300
    } while ((Get-Date) -lt $deadline)
    return $false
}

# How many lines of the app's log hold `line`.
function Count-Log([string] $line) {
    $file = Join-Path $state 'ngterm.log'
    if (-not (Test-Path $file)) { return 0 }
    @(Select-String -Path $file -SimpleMatch $line).Count
}

# Polls the notification center until `test` holds for its toasts.
function Wait-Toasts([scriptblock] $test) {
    $deadline = (Get-Date).AddSeconds($Timeout)
    do {
        $toasts = Get-Toasts
        if (& $test $toasts) { return $true }
        Start-Sleep -Milliseconds 500
    } while ((Get-Date) -lt $deadline)
    Log "toasts: $($toasts -join ' / ')" | Out-Null
    return $false
}

# The session of id `sid` in the tree, or $null.
function Find-Session($node, [string] $sid) {
    if ($null -eq $node) { return $null }
    if ($node -is [System.Array]) {
        foreach ($item in $node) { $found = Find-Session $item $sid; if ($found) { return $found } }
        return $null
    }
    if ($node -isnot [System.Management.Automation.PSCustomObject]) { return $null }
    if ($node.PSObject.Properties['id'] -and $node.id -eq $sid -and $node.PSObject.Properties['active']) { return $node }
    foreach ($property in $node.PSObject.Properties) {
        $found = Find-Session $property.Value $sid
        if ($found) { return $found }
    }
    return $null
}

# Polls the tree until session `sid` is the active one.
function Wait-Active([string] $sid) {
    $deadline = (Get-Date).AddSeconds($Timeout)
    do {
        $session = Find-Session ((Invoke-Control 'tree') | ConvertFrom-Json) $sid
        if ($session -and $session.active) { return $true }
        Start-Sleep -Milliseconds 300
    } while ((Get-Date) -lt $deadline)
    return $false
}

# Polls the tree until the quick terminal's visibility is `visible`.
function Wait-Quick-Visible([bool] $visible) {
    $want = '"quickVisible":' + $visible.ToString().ToLowerInvariant()
    $deadline = (Get-Date).AddSeconds($Timeout)
    do {
        if ((Invoke-Control 'tree') -match $want) { return $true }
        Start-Sleep -Milliseconds 300
    } while ((Get-Date) -lt $deadline)
    return $false
}

# The session's end sent to the app's hidden window, asked (WM_QUERYENDSESSION), then told (WM_ENDSESSION): the app quits
# with status 0 after logging the quit.
function End-Session($process, [string] $what) {
    $quits = Count-Log 'the session is ending: quitting'
    $window = [SmokeWindows]::SystemWindow([uint32]$process.Id)
    Check ($window -ne [IntPtr]::Zero) "the app's hidden window is there ($window)"
    $logoff = [long]0x80000000
    $query = [SmokeWindows]::Send($window, 0x0011, 0, $logoff, 5000)
    Check ($query -eq 1) "WM_QUERYENDSESSION lets the session end (answer $query)"
    [SmokeWindows]::Send($window, 0x0016, 1, $logoff, 15000) | Out-Null
    $ended = $process.WaitForExit(15000)
    Check ($ended -and $process.ExitCode -eq 0) "$what with status $(if ($ended) { $process.ExitCode } else { 'none: still running' })"
    Check ((Count-Log 'the session is ending: quitting') -gt $quits) 'the session end ran the quit'
}

# Brings the first visible window of `id` to the front once it has one: polls until the foreground window is its.
function Wait-Raise([int] $id) {
    $deadline = (Get-Date).AddSeconds($Timeout)
    do {
        if ([SmokeWindows]::Raise([uint32]$id) -and [SmokeWindows]::InFront([uint32]$id)) { return $true }
        Start-Sleep -Milliseconds 300
    } while ((Get-Date) -lt $deadline)
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
$appId = 'com.nickalie.ngterm.smoke'
$env:NGTERM_STATE_DIR = $state
$env:NGTERM_APP_ID = $appId
# the global hotkey: the key left of 1 by position, with Ctrl and Shift
$config = New-Item -ItemType Directory -Force -Path (Join-Path $state 'config')
Write-Utf8 (Join-Path $config 'keymap.conf') ('global-hotkey ctrl+shift+`' + "`r`n")
$env:HOME = $homeDir
$consoleBuild = Test-ConsoleProgram $Exe
Log "exe $Exe ($(if ($consoleBuild) { 'console: a debug build' } else { 'no console: a release build' })); state $state; pipe \\.\pipe\$pipeName; PowerShell $($PSVersionTable.PSVersion)"

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
    # a ^C typed through the control socket interrupts the pane's program (the console's Ctrl+C event): ping prints
    # its statistics and `Control-C`, and the prompt is back
    Invoke-Control 'session.type' @{ text = "ping -t 127.0.0.1`r" } | Out-Null
    Check (Wait-Text 'Reply from') 'ping runs in the pane'
    Invoke-Control 'session.type' @{ text = [string][char]3 } | Out-Null
    Check (Wait-Text 'Control-C') 'a ^C typed through session.type interrupts the ping'
    $split = Invoke-Control 'session.split' @{ mode = 'on' }
    Check ($split -match '"ok":true') "session.split: $split"
    Start-Sleep -Seconds 2
    Invoke-Control 'session.type' @{ text = "Write-Output ('smoke' + '-right')`r"; pane = 'right' } | Out-Null
    Check (Wait-Text 'smoke-right' 'right') 'PowerShell in the split pane runs a typed command'
    $tree = Invoke-Control 'tree'
    Check ($tree -match '"split":true') "the tree shows the split: $tree"
    Start-Sleep -Seconds 1
    Save-Screen 'window.png'
    if ($consoleBuild) {
        $sent = Send-CtrlC $app.Id
        Check ($sent -eq 0) "Ctrl+C sent to the app's console (helper status $sent)"
        $quit = $app.WaitForExit(15000)
        Check ($quit -and $app.ExitCode -eq 0) "the app quit on Ctrl+C with status $(if ($quit) { $app.ExitCode } else { 'none: still running' })"
    } else {
        Log "SKIP the Ctrl+C quit: $Exe is a release build, with no console for the Ctrl event; it quits at the session's end"
        End-Session $app 'the app quit at the session''s end'
    }
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
    Check (Wait-Raise $app.Id) 'the app window comes to the front'
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
    # Ctrl+C pressed on the desktop interrupts the focused pane's program
    $interrupted = Count-EitherPane 'Control-C'
    [System.Windows.Forms.SendKeys]::SendWait('ping -t 127.0.0.1{ENTER}')
    Check (Wait-EitherPane 'Reply from' 2) 'ping runs in the focused pane'
    Send-Chord ([byte[]](0x11, 0x43))
    Check (Wait-EitherPane-Count 'Control-C' ($interrupted + 1)) 'Ctrl+C pressed on the desktop interrupts the ping'
    Save-Screen 'ctrl-c.png'

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

    # the global hotkey: registered at the launch, it toggles the quick terminal while another application has the
    # keyboard (Character Map, a plain Win32 window, stopped by its process id after)
    Check (Wait-Log 'global hotkey: registered') 'the keymap''s global-hotkey is registered'
    $other = Start-Process -FilePath 'charmap.exe' -PassThru
    try {
        $raised = $false
        for ($try = 0; $try -lt 60 -and -not $raised; $try++) {
            Start-Sleep -Milliseconds 500
            $raised = [SmokeWindows]::Raise([uint32]$other.Id)
        }
        Check $raised 'Character Map has the keyboard'
        Start-Sleep -Milliseconds 500
        # Ctrl, Shift and the key left of 1 (VK_OEM_3 on the US layout)
        Send-Chord ([byte[]](0x11, 0x10, 0xC0))
        Check (Wait-Quick-Visible $true) 'the global hotkey pressed in Character Map shows the quick terminal'
        Check (Wait-Log 'global hotkey: pressed') 'the press reached the main loop'
        Start-Sleep -Seconds 1
        Save-Screen 'hotkey.png'
        Send-Chord ([byte[]](0x11, 0x10, 0xC0))
        Check (Wait-Quick-Visible $false) 'the global hotkey pressed again hides it'
    } finally {
        Stop-Process -Id $other.Id -Force -ErrorAction SilentlyContinue
    }

    # notifications: toasts under the app's id, a click revealing its session, a repeat replacing, a withdrawal; the
    # toasts earlier runs left under this app id are cleared first
    Clear-Toasts
    $first = (Invoke-Control 'session.select' | ConvertFrom-Json).result.id
    $second = (Invoke-Control 'session.new' | ConvertFrom-Json).result.id
    Log "sessions: shown $first, notified $second"
    Invoke-Control 'session.select' $null 2000 $first | Out-Null
    Check (Wait-Active $first) 'the first session is shown again'
    $slot = "\|${second}:notify\|"
    # a toast's popup shows for a few seconds: each try posts one, clicks it at once and waits for the app to hear it
    $clicked = $false
    for ($try = 1; $try -le 3 -and -not $clicked; $try++) {
        $note = Invoke-Control 'notify' @{ title = 'Smoke toast'; body = "smoke-toast-click-$try" } 2000 $second
        Check ($note -match '"ok":true') "notify: $note"
        Start-Sleep -Milliseconds 1500
        Save-Screen "toast-$try.png"
        Click-Toast | Out-Null
        $clicked = Wait-Log-For 'notify: toast clicked' 15
    }
    Check $clicked 'a click on the toast on screen reaches the app'
    Check (Wait-Active $second) 'the click reveals the notified session'
    Start-Sleep -Seconds 1
    Save-Screen 'toast-clicked.png'
    Check (Wait-Toasts { param($t) -not ($t | Where-Object { $_ -match $slot }) }) 'the clicked toast is gone from the notification center'
    Invoke-Control 'session.select' $null 2000 $first | Out-Null
    Check (Wait-Active $first) 'the first session is shown again'
    Invoke-Control 'notify' @{ title = 'Smoke toast'; body = 'smoke-toast-three' } 2000 $second | Out-Null
    Check (Wait-Toasts { param($t) @($t | Where-Object { $_ -match $slot -and $_ -match 'smoke-toast-three' }).Count -eq 1 }) 'the notification center holds the toast in the session''s slot'
    Invoke-Control 'notify' @{ title = 'Smoke toast'; body = 'smoke-toast-four' } 2000 $second | Out-Null
    Check (Wait-Toasts { param($t) $held = @($t | Where-Object { $_ -match $slot }); $held.Count -eq 1 -and $held[0] -match 'smoke-toast-four' }) 'a second notify replaces the toast in its slot'
    Check (Wait-Raise $app.Id) 'the app window comes to the front'
    Invoke-Control 'session.select' $null 2000 $second | Out-Null
    Check (Wait-Toasts { param($t) -not ($t | Where-Object { $_ -match $slot }) }) 'selecting the session withdraws its toast'

    # status sounds: upstream's names are macOS sounds; Windows has no alias of theirs and plays the alert
    $status = Invoke-Control 'session.status' @{ status = 'blocked'; sound = 'Glass' } 2000 $second
    Check ($status -match '"ok":true') "session.status with a sound: $status"
    Check (Wait-Log 'no such sound alias here, playing the alert') 'the status sound reached the player'

    # the remote links: the network's connectivity hint at start, and a resume from sleep retrying them
    Check (Wait-Log "remote links: the network's connectivity hint") 'the connectivity hint reported the network'
    $hidden = [SmokeWindows]::SystemWindow([uint32]$app.Id)
    # WM_POWERBROADCAST, PBT_APMRESUMEAUTOMATIC
    Check ([SmokeDesktop]::Post($hidden, 0x0218, 0x12, 0)) 'a resume sent to the hidden window'
    Check (Wait-Log 'network back or machine awake: retrying the remote links now') 'the resume retries the remote links'

    # the session's end: asked, then told; the app quits
    End-Session $app 'the app quit at the session''s end'

    # the keyboard-layout lock: Russian for the terminals, so the US keys type Cyrillic in the pane
    $russian = [SmokeDesktop]::LoadLayout('00000419')
    try {
        $settingsFile = Join-Path $state 'settings.json'
        $settings = Get-Content -Raw -Path $settingsFile | ConvertFrom-Json
        $settings | Add-Member -NotePropertyName terminalInputSource -NotePropertyValue '00000419' -Force
        Write-Utf8 $settingsFile ($settings | ConvertTo-Json -Depth 20)
        $app = Start-App
        Log "relaunched pid $($app.Id) with the layout lock on 00000419"
        Check ($null -ne (Wait-Socket)) 'the relaunched app answers'
        Check (Wait-Raise $app.Id) 'the app window comes to the front'
        Start-Sleep -Seconds 1
        [System.Windows.Forms.SendKeys]::SendWait('{ESC}')
        Check (Wait-Log 'keyboard layout lock: layout selected') 'the lock selects Russian for the terminal'
        Start-Sleep -Milliseconds 500
        [System.Windows.Forms.SendKeys]::SendWait('qwe')
        Check (Wait-EitherPane ([string][char]0x439 + [char]0x446 + [char]0x443)) 'the US keys Q, W, E type the Russian letters in the pane'
        Save-Screen 'layout-lock.png'
    } finally {
        Stop-App $app
        [SmokeDesktop]::UnloadLayout($russian)
    }

    # the last window's close (WM_CLOSE, as its title bar's close button and the taskbar's Close window send it): a
    # window running sessions asks first (upstream's `windowShouldClose`); its Close (Return, the default) closes it, and
    # the last window closing quits the app, which saves and exits with status 0
    $saves = Count-Log 'quit: the state is saved'
    $app = Start-App
    Log "relaunched pid $($app.Id) for the last window's close"
    Check ($null -ne (Wait-Socket)) 'the relaunched app answers'
    $main = [IntPtr]::Zero
    for ($try = 0; $try -lt 60 -and $main -eq [IntPtr]::Zero; $try++) {
        Start-Sleep -Milliseconds 500
        $main = [SmokeWindows]::MainWindow([uint32]$app.Id)
    }
    Check ($main -ne [IntPtr]::Zero) "the app's main window is there ($main)"
    Start-Sleep -Seconds 1
    Check ([SmokeDesktop]::Post($main, 0x0010, 0, 0)) 'WM_CLOSE posted to the main window'
    Start-Sleep -Seconds 2
    Check (-not $app.HasExited) 'the window running a session asks before it closes'
    Save-Screen 'close-asked.png'
    Check ([SmokeWindows]::Focus($main)) 'the asking window comes to the front'
    Start-Sleep -Milliseconds 500
    # Return as a key press: SendKeys' Return did not reach the alert
    Send-Chord ([byte[]](0x0D))
    $closed = $app.WaitForExit(15000)
    Check ($closed -and $app.ExitCode -eq 0) "closing the last window quits the app with status $(if ($closed) { $app.ExitCode } else { 'none: still running' })"
    Check ((Count-Log 'quit: the state is saved') -gt $saves) 'the quit saved the state'
} catch {
    Log "ERROR $_"
    $failures++
} finally {
    Stop-App $app
}
Log "done: $failures failure(s)"
exit $failures
