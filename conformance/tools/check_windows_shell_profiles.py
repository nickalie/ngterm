"""Windows Live shell profiles: isolated headless host/server, no GUI or installed state.

Usage: python conformance/tools/check_windows_shell_profiles.py STAGE --shell cmd
       python conformance/tools/check_windows_shell_profiles.py STAGE --shell wsl --distribution Ubuntu

Requires staged ngterm-headless, ngterm-session and ngtermctl binaries/resources.
The WSL case requires an existing distribution. Logs remain in the printed private temp directory.
"""
import argparse
import json
import os
import shlex
from pathlib import Path
import subprocess
import tempfile
import time

ap = argparse.ArgumentParser()
ap.add_argument('stage', type=Path)
ap.add_argument('--shell', choices=['cmd', 'wsl'], required=True)
ap.add_argument('--distribution', default='Ubuntu')
args = ap.parse_args()
if os.name != 'nt':
    ap.error('This smoke test requires Windows')
stage = args.stage.resolve()
root = Path(tempfile.mkdtemp(prefix='ngterm ' + args.shell + ' live ')).resolve()
state, home, local = (root / name for name in ('state', 'home with spaces', 'local'))
for directory in (state, home, local):
    directory.mkdir()
env = {k: v for k, v in os.environ.items() if not k.upper().startswith(('NGTERM_', 'AGTERM_'))}
env.update(NGTERM_STATE_DIR=str(state), HOME=str(home), USERPROFILE=str(home), LOCALAPPDATA=str(local),
           GHOSTTY_RESOURCES_DIR=str(stage / 'share' / 'ghostty'))
socket = str(state / 'ngterm.sock')
env['NGTERM_CONTROL_SOCKET'] = socket
hash_value = 0xcbf29ce484222325
for byte in str(state.resolve()).encode('utf-8'):
    hash_value = ((hash_value ^ byte) * 0x100000001b3) & 0xffffffffffffffff
namespace = local / 'ngterm' / 'live' / f'{hash_value:016x}'
namespace.mkdir(parents=True)
env['NGTERM_LIVE_DIR'] = str(namespace)
system = Path(os.environ['SystemRoot']) / 'System32'
profile = ({'id': 'cmd', 'name': 'CMD', 'executable': str(system / 'cmd.exe'), 'args': ['/D', '/U']}
           if args.shell == 'cmd' else
           {'id': 'wsl:Ubuntu', 'name': 'WSL: Ubuntu', 'executable': str(system / 'wsl.exe'),
            'args': ['--distribution', args.distribution, '--cd', '/tmp']})
settings = {'autoUpdate': False, 'restoreMode': 'live', 'welcomeShown': True, 'defaultShellProfile': profile}
settings_path = state / 'settings.json'
settings_path.write_text(json.dumps(settings), encoding='utf-8')
ctl_path = str(stage / 'bin/ngtermctl.exe')
server_path = str(stage / 'bin/ngterm-session.exe')
flags = subprocess.CREATE_NO_WINDOW
app = server = None
logs = []

def spawn(exe, arguments, log_name):
    log = (root / log_name).open('wb')
    logs.append(log)
    return subprocess.Popen([str(exe), *arguments], env=env, cwd=home, stdin=subprocess.DEVNULL,
                            stdout=log, stderr=log, creationflags=flags)

def ctl(*arguments, text=None):
    result = subprocess.run([ctl_path, *arguments, '--socket', socket, '--json'], env=env,
                            input=text, text=True, encoding='utf-8', capture_output=True,
                            timeout=10, creationflags=flags)
    if result.returncode:
        raise RuntimeError(result.stderr or result.stdout)
    data = json.loads(result.stdout)
    assert data['ok'], data
    return data.get('result')

def eventually(check, seconds=40):
    deadline = time.monotonic() + seconds
    last = None
    while time.monotonic() < deadline:
        try:
            value = check()
            if value:
                return value
        except (RuntimeError, OSError, AssertionError) as error:
            last = error
        time.sleep(.15)
    raise AssertionError(f'Timeout: {last}; logs: {root}')

def sessions():
    tree = ctl('tree')['tree']
    return [s for w in tree['workspaces'] for s in w['sessions']]

def ready():
    assert app.poll() is None, f'headless exited: {app.returncode}'
    values = sessions()
    return values if values and values[0].get('backedByLive') else None

def server_records():
    result = subprocess.run([server_path, 'list'], env=env, capture_output=True, text=True,
                            timeout=10, creationflags=flags, check=True)
    return [dict(item.split('=', 1) for item in line.split('\t')) for line in result.stdout.splitlines()]

def probe(session_id, name, initialize=False):
    marker = home / (name + '.txt')
    if args.shell == 'cmd':
        if initialize:
            ctl('session', 'type', '--target', session_id, '--stdin', text='set "PROFILE_FEATURE_TEST=preserved"\rcd /d "%SystemRoot%"\r')
        script = f'echo %COMSPEC%^|%CD%^|%PROFILE_FEATURE_TEST%>"{marker}"\r'
    else:
        translated = subprocess.run([profile['executable'], '-d', args.distribution, '--exec', 'wslpath', '-a', str(marker)],
                                    env=env, capture_output=True, timeout=20, creationflags=flags, check=True)
        linux_marker = translated.stdout.decode('utf-8').strip()
        script = 'cd /; PROFILE_FEATURE_TEST=preserved; ' if initialize else ''
        script += f'printf "%s\\n" "$WSL_DISTRO_NAME|$$|$PWD|$PROFILE_FEATURE_TEST" > "{linux_marker}"\r'
    ctl('session', 'type', '--target', session_id, '--stdin', text=script)
    return eventually(lambda: marker.read_text(encoding='utf-16-le' if args.shell == 'cmd' else 'utf-8').strip(), seconds=60)

try:
    server = spawn(server_path, ['server', str(namespace)], 'server.log')
    time.sleep(.5)
    app = spawn(stage / 'bin/ngterm-headless.exe', ['--skip-agent-setup'], 'headless-first.log')
    first_session = eventually(ready)[0]
    first = probe(first_session['id'], 'first', True)
    assert '|preserved' in first, first
    record = server_records()[0]
    assert record['pid'] != '-', record
    eventually(lambda: any('shellProfile' in p.read_text(encoding='utf-8') for p in (state / 'windows').glob('*.json')))
    app.terminate()
    app.wait(timeout=10)
    ps = Path(os.environ['SystemRoot']) / 'System32/WindowsPowerShell/v1.0/powershell.exe'
    settings['defaultShellProfile'] = {'id': 'windows-powershell', 'name': 'Windows PowerShell',
                                     'executable': str(ps), 'args': ['-NoLogo']}
    settings_path.write_text(json.dumps(settings), encoding='utf-8')
    app = spawn(stage / 'bin/ngterm-headless.exe', ['--skip-agent-setup'], 'headless-second.log')
    second_session = eventually(ready)[0]
    assert second_session['id'] == first_session['id']
    second = probe(second_session['id'], 'second')
    assert first == second, (first, second)
    restored = server_records()[0]
    assert restored['pid'] == record['pid'], (record, restored)
    # A foreground job must remain interruptible after reattachment.
    job = 'ping -t 127.0.0.1\r' if args.shell == 'cmd' else '/bin/sleep 60\r'
    ctl('session', 'type', '--target', first_session['id'], '--stdin', text=job)
    time.sleep(.4)
    ctl('session', 'type', '--target', first_session['id'], '--stdin', text='\x03')
    assert probe(first_session['id'], 'after-interrupt') == first
    # Explicit restart commands preserve quoting and return to the selected shell.
    restarted_marker = home / 'restart with spaces.txt'
    if args.shell == 'cmd':
        creation = f'echo "quoted words">"{restarted_marker}"'
        expected = '"quoted words"'
    else:
        translated = subprocess.run([profile['executable'], '-d', args.distribution, '--exec', 'wslpath', '-a', str(restarted_marker)],
                                    env=env, capture_output=True, timeout=20, creationflags=flags, check=True)
        creation = "printf '%s' 'quoted words' > " + shlex.quote(translated.stdout.decode('utf-8').strip())
        expected = 'quoted words'
    ctl('session', 'restart', '--target', first_session['id'], '--pane', 'primary', '--command', creation)
    encoding = 'utf-16-le' if args.shell == 'cmd' else 'utf-8'
    assert eventually(lambda: restarted_marker.read_text(encoding=encoding).strip()) == expected
    assert '|preserved' in probe(first_session['id'], 'after-restart', True)
    ctl('session', 'new')
    added = eventually(lambda: next((s for s in sessions() if s['id'] != first_session['id']), None))
    marker = str(home / 'powershell.txt').replace("'", "''")
    ctl('session', 'type', '--target', added['id'], '--stdin',
        text=f"[System.IO.File]::WriteAllText('{marker}', $PSVersionTable.PSEdition)\r")
    assert eventually(lambda: (home / 'powershell.txt').read_text(encoding='utf-8')) == 'Desktop'
    print(f'PASS: {args.shell} Live preserves leader PID/cwd/variables and distribution across host restart; Ctrl+C and quoted restart commands work; new sessions use changed PowerShell default.')
    print('Leader PID:', record['pid'], 'Probe:', first)
finally:
    if app and app.poll() is None:
        app.terminate()
        app.wait(timeout=10)
    if server and server.poll() is None:
        try:
            listing = subprocess.run([server_path, 'list', '--short'], env=env, capture_output=True, text=True,
                                     timeout=10, creationflags=flags)
            names = listing.stdout.splitlines() if listing.returncode == 0 else []
            if names:
                subprocess.run([server_path, 'kill', '--force', *names], env=env, capture_output=True,
                               timeout=15, creationflags=flags, check=True)
        finally:
            server.terminate()
            server.wait(timeout=10)
    for log in logs:
        log.close()
    print('Private test logs:', root)
