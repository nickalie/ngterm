<#
.SYNOPSIS
The Windows installer's check (docs/plans/20261029-windows-support.md, Task 7): installs an MSI scripts/build-msi.sh
built for this user, checks what it installed, optionally upgrades it to a later one, and uninstalls it.

.DESCRIPTION
For a throwaway machine or account (a CI runner, a test VM): it installs into this user's real per-user locations
(%LOCALAPPDATA%\Programs\ngterm, the Start menu, the user's PATH) and removes them again, and it fails when an ngterm is
installed there already. No app is started: ngterm.exe runs only for its build identity.

After the install: the binaries, Ghostty's resources (shell integration, the themes, the compiled xterm-ghostty),
ngterm's PowerShell integration, the agent-status package, the agent skill, the build identity and the licenses are in
the staged layout; no session build id (packaging/windows/ngterm.wxs says why); `ngterm.exe --build-identity` names the
package's version; the user's PATH in the registry holds bin\ and keeps its value kind (REG_EXPAND_SZ), and a new
PowerShell given that PATH finds the installed ngtermctl and runs it; the Start-menu shortcut targets ngterm.exe, starts
in the user's profile and carries the app id as its System.AppUserModel.ID. With -UpgradeMsi, that MSI (a higher
version) replaces the first: the first product is gone, the second installed, the files in place, and a file in the
app's state directory is kept. Then the uninstall, after the app id's toast registration is written as the app writes it
at each launch: no install folder, shortcut, product registration, PATH entry (the user's PATH is back as it was, kind
included), HKCU\Software\ngterm or app id registration is left, and the state directory's file stays (user state is
never the installer's).

Each msiexec run logs verbosely under -Log. The exit status is 0 when every check passed.

.PARAMETER Msi
The MSI to install.

.PARAMETER UpgradeMsi
An MSI of a higher version to upgrade to before the uninstall.

.PARAMETER Log
The directory the msiexec logs go to.

.PARAMETER AppId
The app id the shortcut carries (crates/ngterm-gpui/src/instance.rs DEFAULT_APP_ID).
#>
param(
    [Parameter(Mandatory = $true)] [string] $Msi,
    [string] $UpgradeMsi = '',
    [Parameter(Mandatory = $true)] [string] $Log,
    [string] $AppId = 'com.nickalie.ngterm'
)

$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $Log | Out-Null
$root = Join-Path $env:LOCALAPPDATA 'Programs\ngterm'
$bin = Join-Path $root 'bin'
$shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'ngterm.lnk'
$aumidKey = "HKCU:\Software\Classes\AppUserModelId\$AppId"
$failures = 0

function Check([string] $what, [bool] $ok) {
    # to the host, not the pipeline: Test-Installed returns its product code through the pipeline
    if ($ok) { Write-Host "ok    $what" } else { Write-Host "FAIL  $what"; $script:failures++ }
}

function Invoke-Msiexec([string[]] $arguments, [string] $name) {
    $logFile = Join-Path $Log "$name.log"
    $process = Start-Process msiexec.exe -ArgumentList ($arguments + @('/qn', '/l*v', "`"$logFile`"")) -Wait -PassThru
    Check "msiexec $name exits 0 (got $($process.ExitCode), $logFile)" ($process.ExitCode -eq 0)
}

# A property of an MSI's Property table, through Windows Installer's automation interface.
function Get-MsiProperty([string] $path, [string] $name) {
    $installer = New-Object -ComObject WindowsInstaller.Installer
    $database = $installer.GetType().InvokeMember('OpenDatabase', 'InvokeMethod', $null, $installer, @($path, 0))
    $query = "SELECT ``Value`` FROM ``Property`` WHERE ``Property`` = '$name'"
    $view = $database.GetType().InvokeMember('OpenView', 'InvokeMethod', $null, $database, @($query))
    $view.GetType().InvokeMember('Execute', 'InvokeMethod', $null, $view, $null) | Out-Null
    $record = $view.GetType().InvokeMember('Fetch', 'InvokeMethod', $null, $view, $null)
    $value = $record.GetType().InvokeMember('StringData', 'GetProperty', $null, $record, 1)
    $view.GetType().InvokeMember('Close', 'InvokeMethod', $null, $view, $null) | Out-Null
    $value
}

# 5 installed for this user, -1 unknown.
function Get-ProductState([string] $code) {
    (New-Object -ComObject WindowsInstaller.Installer).ProductState($code)
}

function Get-UserPath { [string] [Environment]::GetEnvironmentVariable('Path', 'User') }

# The user's Path as the registry holds it (unexpanded) and its kind (REG_EXPAND_SZ keeps %VAR% entries working).
function Get-RawUserPath {
    $key = Get-Item 'HKCU:\Environment'
    if ($null -eq $key.GetValue('Path')) { return @('', 'absent') }
    @([string] $key.GetValue('Path', '', 'DoNotExpandEnvironmentNames'), [string] $key.GetValueKind('Path'))
}

function Test-OnUserPath {
    @((Get-UserPath) -split ';' | Where-Object { $_.TrimEnd('\') -ieq $bin }).Count -gt 0
}

function Test-Installed([string] $msi) {
    $version = Get-MsiProperty $msi 'ProductVersion'
    $code = Get-MsiProperty $msi 'ProductCode'
    Check "product $code ($version) is installed" ((Get-ProductState $code) -eq 5)
    foreach ($file in 'bin\ngterm.exe', 'bin\ngtermctl.exe', 'bin\ngterm-session.exe',
        'share\ghostty\shell-integration\bash\ghostty.bash', 'share\ngterm\shell-integration\pwsh\ngterm.ps1',
        'share\ngterm\agent-status\ngterm-agent-status.ps1', 'share\ngterm\skills\ngterm\SKILL.md',
        'share\ngterm\app-BUILD', 'share\doc\ngterm\NOTICE.txt', 'share\doc\fonts\JetBrainsMono-OFL.txt') {
        Check "$file is installed" (Test-Path -PathType Leaf (Join-Path $root $file))
    }
    $terminfo = @('x', '78') | Where-Object { Test-Path -PathType Leaf (Join-Path $root "share\terminfo\$_\xterm-ghostty") }
    Check 'the compiled xterm-ghostty is installed' (@($terminfo).Count -gt 0)
    $themes = @(Get-ChildItem -File (Join-Path $root 'share\ghostty\themes') -ErrorAction SilentlyContinue).Count
    Check "Ghostty's themes are installed ($themes)" ($themes -gt 700)
    Check 'no session build id is installed' (-not (Test-Path (Join-Path $root 'share\ngterm\session-BUILD')))

    $identity = @(& (Join-Path $bin 'ngterm.exe') --build-identity)
    Check "ngterm.exe --build-identity names $version (got '$($identity[0])')" ($identity[0] -eq $version)

    Check "the user's PATH holds $bin" (Test-OnUserPath)
    $kind = (Get-RawUserPath)[1]
    Check "the user's PATH keeps its kind ($($originalPath[1]), now $kind)" ($originalPath[1] -eq 'absent' -or $kind -eq $originalPath[1])
    $machinePath = [Environment]::GetEnvironmentVariable('Path', 'Machine')
    $found = & powershell.exe -NoProfile -Command "`$env:Path = '$machinePath;$(Get-UserPath)'; (Get-Command ngtermctl).Source; ngtermctl --help | Out-Null; `$LASTEXITCODE"
    Check "a new shell finds the installed ngtermctl (got '$($found[0])')" ($found[0] -ieq (Join-Path $bin 'ngtermctl.exe'))
    Check "and runs it (status $($found[-1]))" ($found[-1] -eq '0')

    Check 'the Start-menu shortcut is installed' (Test-Path -PathType Leaf $shortcut)
    if (Test-Path $shortcut) {
        $link = (New-Object -ComObject WScript.Shell).CreateShortcut($shortcut)
        Check "the shortcut targets ngterm.exe (got '$($link.TargetPath)')" ($link.TargetPath -ieq (Join-Path $bin 'ngterm.exe'))
        Check "the shortcut starts in the profile (got '$($link.WorkingDirectory)')" ($link.WorkingDirectory.TrimEnd('\') -ieq $env:USERPROFILE)
        $folder = (New-Object -ComObject Shell.Application).Namespace((Split-Path $shortcut))
        $aumid = $folder.ParseName('ngterm.lnk').ExtendedProperty('System.AppUserModel.ID')
        Check "the shortcut's System.AppUserModel.ID is $AppId (got '$aumid')" ($aumid -eq $AppId)
    }
    $code
}

$code = Get-MsiProperty $Msi 'ProductCode'
if ((Get-ProductState $code) -eq 5 -or (Test-Path $root)) {
    Write-Output "FAIL  ngterm is installed for this user already ($root); the check needs a clean account"
    exit 1
}

$originalPath = Get-RawUserPath
Invoke-Msiexec @('/i', "`"$Msi`"") 'install'
$installed = Test-Installed $Msi

# user state the installer never owns: a file in the app's state directory (the Windows state dir is
# %LOCALAPPDATA%\ngterm, crates/ngterm-core/src/paths.rs)
$stateDir = Join-Path $env:LOCALAPPDATA 'ngterm'
$stateDirExisted = Test-Path $stateDir
$stateFile = Join-Path $stateDir "check-msi-$PID.txt"
New-Item -ItemType Directory -Force -Path $stateDir | Out-Null
Set-Content -Path $stateFile -Value 'user state'

if ($UpgradeMsi) {
    Invoke-Msiexec @('/i', "`"$UpgradeMsi`"") 'upgrade'
    Check "the upgrade removed product $installed" ((Get-ProductState $installed) -eq -1)
    $installed = Test-Installed $UpgradeMsi
    Check 'the upgrade kept the state directory' (Test-Path -PathType Leaf $stateFile)
}

# the app id's registration as the app writes it at each launch (crates/ngterm-gpui/src/notifications/windows.rs)
New-Item -Force -Path $aumidKey | Out-Null
New-ItemProperty -Force -Path $aumidKey -Name DisplayName -Value 'ngterm' | Out-Null

Invoke-Msiexec @('/x', $installed) 'uninstall'
Check "product $installed is gone" ((Get-ProductState $installed) -eq -1)
Check "no install folder is left ($root)" (-not (Test-Path $root))
Check 'no Start-menu shortcut is left' (-not (Test-Path $shortcut))
Check "the user's PATH no longer holds bin\" (-not (Test-OnUserPath))
$restored = Get-RawUserPath
Check "the user's PATH is as it was ($($restored[1]): '$($restored[0])')" (
    ($restored[0] -eq $originalPath[0] -and $restored[1] -eq $originalPath[1]) -or
    ($originalPath[1] -eq 'absent' -and $restored[0] -eq ''))
Check 'no HKCU\Software\ngterm is left' (-not (Test-Path 'HKCU:\Software\ngterm'))
Check "no app id registration is left ($aumidKey)" (-not (Test-Path $aumidKey))
Check 'the state directory is kept' (Test-Path -PathType Leaf $stateFile)
Remove-Item -Force $stateFile
if (-not $stateDirExisted) { Remove-Item -Force $stateDir }

if ($failures -gt 0) {
    Write-Output "$failures check(s) failed"
    exit 1
}
Write-Output 'every check passed'
