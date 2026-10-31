<#
show-image.ps1 - display an image inside the current ngterm session, on Windows.

usage: powershell -NoProfile -ExecutionPolicy Bypass -File show-image.ps1 <image> [size-percent]

the Windows form of show-image.sh, which runs this script when it finds itself under Git Bash. show-image.sh draws
the image with the kitty graphics protocol in a terminal overlay, but a Windows pane's program writes through the
pseudoconsole (ConPTY), which drops the protocol's APC escapes, so no program overlay can show an image there.
this script shows it in a page overlay instead: a self-contained HTML page holding the image as a data URI,
scaled to fit the panel and centered, with "press Enter to close" below it, as the overlay show-image.sh opens.
Enter closes it (the page asks ngterm to close its overlay), as do the session's close shortcut and
`ngtermctl session overlay close`. the page sits in a fresh ngterm-show-image-* folder under TEMP, which stays while
the overlay may reload it (a theme change does) and goes on a later run once a day old.

requires running inside ngterm (NGTERM_ENABLED=1) with ngtermctl on PATH. works under Windows PowerShell 5.1 and
PowerShell 7.
#>
param(
    [Parameter(Position = 0)] [string] $Image = '',
    [Parameter(Position = 1)] [string] $Size = '60'
)

$ErrorActionPreference = 'Stop'

if (-not $Image) { [Console]::Error.WriteLine('usage: show-image.ps1 <image> [size-percent]'); exit 2 }
if (-not (Test-Path -LiteralPath $Image -PathType Leaf)) {
    [Console]::Error.WriteLine("show-image.ps1: no such file: $Image"); exit 1
}
if (-not (Get-Command ngtermctl -ErrorAction SilentlyContinue)) {
    [Console]::Error.WriteLine('show-image.ps1: ngtermctl not on PATH'); exit 1
}
if ($env:NGTERM_ENABLED -ne '1') {
    [Console]::Error.WriteLine("show-image.ps1: not inside ngterm; try: Invoke-Item `"$Image`""); exit 1
}

$path = (Resolve-Path -LiteralPath $Image).ProviderPath
$name = [System.Net.WebUtility]::HtmlEncode([System.IO.Path]::GetFileName($path))
$types = @{
    '.png' = 'image/png'; '.jpg' = 'image/jpeg'; '.jpeg' = 'image/jpeg'; '.gif' = 'image/gif'
    '.webp' = 'image/webp'; '.bmp' = 'image/bmp'; '.svg' = 'image/svg+xml'; '.ico' = 'image/x-icon'
}
$type = $types[[System.IO.Path]::GetExtension($path).ToLowerInvariant()]
if (-not $type) { $type = 'image/png' }
$data = [Convert]::ToBase64String([System.IO.File]::ReadAllBytes($path))

# the page takes the terminal theme's background and text color (it styles neither); the image fills the rows above
# the prompt at the largest scale that keeps its aspect, centered both ways
$page = @"
<!doctype html>
<html><head><meta charset="utf-8"><title>$name</title>
<style>
html, body { margin: 0; height: 100%; overflow: hidden; }
body { display: flex; flex-direction: column; font-family: monospace; }
#image { flex: 1; min-height: 0; display: flex; align-items: center; justify-content: center; }
#image img { width: 100%; height: 100%; object-fit: contain; }
#prompt { text-align: center; padding: 0.3em 0; }
</style></head>
<body><div id="image"><img src="data:$type;base64,$data" alt="$name"></div>
<div id="prompt">press Enter to close</div>
<script>
window.focus();
document.addEventListener('keydown', function (event) {
  if (event.key !== 'Enter') { return; }
  event.preventDefault();
  if (window.ngterm) {
    ngterm.request('session.overlay.close').catch(function () { window.close(); });
  } else {
    window.close();
  }
});
</script></body></html>
"@

$temp = [System.IO.Path]::GetTempPath()
Get-ChildItem -LiteralPath $temp -Directory -Filter 'ngterm-show-image-*' -ErrorAction SilentlyContinue |
    Where-Object { $_.LastWriteTime -lt (Get-Date).AddDays(-1) } |
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue
$dir = New-Item -ItemType Directory -Path (Join-Path $temp ('ngterm-show-image-' + [guid]::NewGuid().ToString('N')))
$file = Join-Path $dir.FullName 'image.html'
[System.IO.File]::WriteAllText($file, $page, (New-Object System.Text.UTF8Encoding $false))

# ngtermctl never reads NGTERM_SOCKET, and a bare call resolves the default socket, which may belong to another
# running instance than the one that spawned this session: name the session's own socket.
$socket = @()
if ($env:NGTERM_SOCKET) { $socket = @('--socket', $env:NGTERM_SOCKET) }

& ngtermctl session overlay open --html $file --js --chromeless --size-percent $Size @socket
exit $LASTEXITCODE
