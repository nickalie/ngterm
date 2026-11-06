# ngterm-agent-mail — hand the pane's mail to the Claude Code that runs in it (Windows).
#
#   powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File ngterm-agent-mail.ps1 claude          # PostToolUse
#   powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File ngterm-agent-mail.ps1 claude --wait   # SessionStart, Stop (asyncRewake)
#
# The Windows form of ngterm-agent-mail.sh, for Windows PowerShell 5.1 and PowerShell 7; its header holds what the
# hooks do. The payload is read and handed on as UTF-8 whatever the console's code page, `ngtermctl agent mail` writes
# the hook's answer straight to the streams Claude Code reads, and the exit status is the CLI's: 2 when --wait took
# mail (the CLI has turned the pane's status back to active by then). Unlike the POSIX form it cannot hand its process
# over to ngtermctl, and Windows does not re-parent, so a waiter here does not notice its agent exiting: it ends with
# its claim (the next agent's waiter), its pane, its app or the hook's timeout. Outside ngterm it does nothing and
# exits 0.
#
# ngtermctl resolution: $env:NGTERMCTL, else the path the installer bakes in ($NgtermctlBaked), else `ngtermctl` on
# PATH.

if (-not $env:NGTERM_SESSION_ID -or -not $env:NGTERM_PANE_ID) { exit 0 }

$ngtermctl = 'ngtermctl'
if ($env:NGTERMCTL) {
  $ngtermctl = $env:NGTERMCTL
} elseif ((Get-Variable -Name NgtermctlBaked -ValueOnly -ErrorAction SilentlyContinue)) {
  $ngtermctl = $NgtermctlBaked
}

$payload = ''
if ([Console]::IsInputRedirected) {
  try {
    $stdin = [System.IO.StreamReader]::new([Console]::OpenStandardInput(), [System.Text.UTF8Encoding]::new($false))
    $payload = $stdin.ReadToEnd()
  } catch { }
}
if (-not $payload) { exit 0 }

$cli = @('agent', 'mail') + @($args)
if ($env:NGTERM_SOCKET) { $cli += @('--socket', $env:NGTERM_SOCKET) }

$status = 1
try {
  # a `New-Object` encoding is wrapped, which Windows PowerShell 5.1 does not take for a native command's input
  $OutputEncoding = [System.Text.UTF8Encoding]::new($false)
  $payload | & $ngtermctl @cli
  $status = $LASTEXITCODE
} catch { }
exit $status
