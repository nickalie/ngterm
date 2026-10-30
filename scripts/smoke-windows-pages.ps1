<#
.SYNOPSIS
The Windows HTML overlays' smoke check (docs/plans/20261029-windows-support.md, Task 6): one ngterm.exe on this
desktop, on a private state directory, showing pages through `ngtermctl session overlay open --socket`.

.DESCRIPTION
Run it on the desktop the window should open on (in a VM: `vmrun runProgramInGuest -interactive`), never on a machine
whose own ngterm runs there: the instance is private (its own state directory, app id, home and control pipe;
NGTERM_*, AGTERM_* and ZMX_* stripped), but its window takes the desktop.

The check: a `--js` file page, granted its folder, shows (its load state `loaded` in the tree, a screenshot); its bridge
round-trips (the page asks for the tree, renames its session with the answer's window count, and titles itself with
it: the session's new name and the page's title read back from the tree); a persistent URL page on a loopback server
signs in (a cookie the server sets), the app quits on Ctrl+C (its orderly quit) and a second launch's persistent page is
still signed in (the cookie sent back, the page's title and the server's log), while an in-memory page there is not.
Screenshots of each page are saved. The app and the server are stopped by their own process ids at the end, whatever
happened. Every step is logged to <StateDir>\pages.log; the exit status is 0 when every step passed.

.PARAMETER Exe
The ngterm.exe to run (a debug build keeps its console, which the Ctrl+C quit needs).

.PARAMETER Ctl
The ngtermctl.exe that opens the pages.

.PARAMETER Node
The node.exe the loopback server runs on.

.PARAMETER StateDir
The private root: state\, home\, the site, the log and the screenshots go there. A fresh directory under TEMP by
default.
#>
param(
    [Parameter(Mandatory = $true)] [string] $Exe,
    [Parameter(Mandatory = $true)] [string] $Ctl,
    [string] $Node = 'node.exe',
    [string] $StateDir = (Join-Path $env:TEMP "ngterm-pages-$PID"),
    [int] $Port = 18792,
    [int] $Timeout = 90
)

$ErrorActionPreference = 'Stop'
$root = New-Item -ItemType Directory -Force -Path $StateDir
$log = Join-Path $root 'pages.log'
$state = Join-Path $root 'state'
$homeDir = Join-Path $root 'home'
$site = Join-Path $root 'site'
New-Item -ItemType Directory -Force -Path $state, $homeDir, $site | Out-Null
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

# ngtermctl against the private instance; its output as one string.
function Ctl([string[]] $arguments) {
    $output = & $Ctl @arguments 2>&1 | Out-String
    Log "ngtermctl $($arguments -join ' ') -> $LASTEXITCODE $($output.Trim())"
    return $output
}

function Tree {
    return Ctl @('tree', '--socket', $socket, '--json')
}

# Polls the tree until `test` holds on it; the last tree.
function Wait-Tree([scriptblock] $test) {
    $deadline = (Get-Date).AddSeconds($Timeout)
    do {
        $tree = & $Ctl tree --socket $socket --json 2>&1 | Out-String
        if (& $test $tree) { return $tree }
        Start-Sleep -Milliseconds 500
    } while ((Get-Date) -lt $deadline)
    Log "last tree: $tree"
    return $null
}

function Wait-Socket {
    $deadline = (Get-Date).AddSeconds($Timeout)
    do {
        & $Ctl version --socket $socket *> $null
        if ($LASTEXITCODE -eq 0) { return $true }
        Start-Sleep -Milliseconds 300
    } while ((Get-Date) -lt $deadline)
    return $false
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

function Stop-ById($process, [string] $what) {
    if ($process -and -not $process.HasExited) {
        Log "stopping $what pid $($process.Id) by its process id"
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
        $process.WaitForExit(10000) | Out-Null
    }
}

Add-Type -AssemblyName System.Windows.Forms, System.Drawing

# the file page: its script asks for the tree over the bridge, renames its session with the answer and titles itself
Set-Content -Encoding utf8 -Path (Join-Path $site 'page.html') -Value @'
<!doctype html>
<html><head><meta charset="utf-8"><title>bridge pending</title>
<link rel="stylesheet" href="style.css"></head>
<body><h1>WebView2 page</h1><p id="out">asking the app...</p>
<script>
ngterm.request('tree').then((tree) => {
  const count = (tree && tree.windows) ? tree.windows.length : 0;
  document.getElementById('out').textContent = 'bridge reply: ' + count + ' window(s)';
  return ngterm.request('session.rename', {args: {name: 'bridged-' + count}}).then(() => {
    document.title = 'bridged ' + count;
  });
}).catch((error) => {
  document.getElementById('out').textContent = 'bridge error: ' + error.message;
  document.title = 'bridge error';
});
</script></body></html>
'@
Set-Content -Encoding utf8 -Path (Join-Path $site 'style.css') -Value 'body { font: 28px sans-serif; color: var(--ngterm-foreground); } h1 { color: #4caf50; }'

# the loopback server: /login sets a day-long cookie and goes home, home says whether the cookie came back
$serverLog = Join-Path $root 'server.log'
Set-Content -Encoding utf8 -Path (Join-Path $root 'server.js') -Value @'
const http = require('http');
const fs = require('fs');
const [port, log] = process.argv.slice(2);
http.createServer((request, response) => {
  const cookie = request.headers.cookie || '';
  fs.appendFileSync(log, `${request.url} cookie=${cookie}\n`);
  if (request.url.startsWith('/login')) {
    response.writeHead(302, {'Set-Cookie': 'session=signed-in; Max-Age=86400; Path=/; HttpOnly', Location: '/'});
    response.end();
    return;
  }
  const signed = cookie.includes('session=signed-in');
  const text = signed ? 'signed in' : 'signed out';
  response.writeHead(200, {'Content-Type': 'text/html; charset=utf-8'});
  response.end(`<!doctype html><title>${text}</title><body style="font:32px sans-serif">${text}</body>`);
}).listen(Number(port), '127.0.0.1');
'@

Get-ChildItem env: | Where-Object { $_.Name -match '^(NGTERM|AGTERM|ZMX)_' } | ForEach-Object {
    Remove-Item -LiteralPath "env:$($_.Name)"
}
$env:NGTERM_STATE_DIR = $state
$env:NGTERM_APP_ID = 'com.nickalie.ngterm.pages'
$env:HOME = $homeDir
Log "exe $Exe; ctl $Ctl; state $state"

$app = $null
$server = $null
try {
    $server = Start-Process -FilePath $Node -ArgumentList (Join-Path $root 'server.js'), $Port, $serverLog -WindowStyle Hidden -PassThru
    Log "server pid $($server.Id) on 127.0.0.1:$Port"

    $app = Start-Process -FilePath $Exe -WorkingDirectory $homeDir -PassThru
    Log "launched pid $($app.Id)"
    Check (Wait-Socket) 'the control socket answers'

    # a page shows, and its bridge round-trips
    Ctl @('session', 'overlay', 'open', '--socket', $socket, '--html', (Join-Path $site 'page.html'), '--cwd', $site, '--js', '--navigation') | Out-Null
    $tree = Wait-Tree { param($t) $t -match '"state":"loaded"' }
    Check ($null -ne $tree) 'the file page loads'
    $tree = Wait-Tree { param($t) $t -match '"title":"bridged \d+"' }
    Check ($null -ne $tree) 'the page titled itself with the bridge reply'
    Check ($tree -match '"name":"bridged-\d+"') 'the bridge request renamed the session'
    Start-Sleep -Seconds 1
    Save-Screen 'page.png'
    Ctl @('session', 'overlay', 'close', '--socket', $socket) | Out-Null

    # a login on the saved store survives a restart
    Ctl @('session', 'overlay', 'open', '--socket', $socket, '--url', "http://127.0.0.1:$Port/login", '--persistent') | Out-Null
    $tree = Wait-Tree { param($t) $t -match '"title":"signed in"' }
    Check ($null -ne $tree) 'the persistent page signs in'
    Save-Screen 'signed-in.png'
    $sent = Send-CtrlC $app.Id
    Check ($sent -eq 0) "Ctrl+C sent to the app's console (helper status $sent)"
    $quit = $app.WaitForExit(15000)
    Check ($quit -and $app.ExitCode -eq 0) "the app quit on Ctrl+C with status $(if ($quit) { $app.ExitCode } else { 'none: still running' })"
    Stop-ById $app 'app'

    $app = Start-Process -FilePath $Exe -WorkingDirectory $homeDir -PassThru
    Log "relaunched pid $($app.Id)"
    Check (Wait-Socket) 'the relaunched app answers'
    # the session may come back with its page: a fresh one replaces it either way
    Ctl @('session', 'overlay', 'close', '--socket', $socket) | Out-Null
    Ctl @('session', 'overlay', 'open', '--socket', $socket, '--url', "http://127.0.0.1:$Port/", '--persistent') | Out-Null
    $tree = Wait-Tree { param($t) $t -match '"title":"signed in"' }
    Check ($null -ne $tree) 'the persistent page is still signed in after the restart'
    Save-Screen 'still-signed-in.png'
    Ctl @('session', 'overlay', 'close', '--socket', $socket) | Out-Null
    Ctl @('session', 'overlay', 'open', '--socket', $socket, '--url', "http://127.0.0.1:$Port/") | Out-Null
    $tree = Wait-Tree { param($t) $t -match '"title":"signed (in|out)"' }
    Check ($tree -match '"title":"signed out"') 'an in-memory page has no saved login'
    $served = Get-Content -Path $serverLog -Raw
    Log "server log:`n$served"
    Check (($served -split "`n" | Where-Object { $_ -match '^/ cookie=.*session=signed-in' }).Count -ge 2) 'the saved cookie came back before and after the restart'
} finally {
    Stop-ById $app 'app'
    Stop-ById $server 'server'
}

Log "failures: $failures"
exit $failures
