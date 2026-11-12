"""Exercise the bundled WSL launcher with real Bash/Zsh, without changing user startup files.

Usage: python conformance/tools/check_wsl_integration.py GHOSTTY_RESOURCES
       [--distribution Ubuntu] [--shell /bin/bash] [--zsh-functions /temporary/zsh/functions]
No GUI, ngterm server, distro shutdown or installation is performed.
"""
import argparse
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile

ap = argparse.ArgumentParser()
ap.add_argument('ghostty_resources', type=Path)
ap.add_argument('--distribution', default='Ubuntu')
ap.add_argument('--shell', default='/bin/bash')
ap.add_argument('--zsh-functions')
ap.add_argument('--zsh-modules')
args = ap.parse_args()
repo = Path(__file__).resolve().parents[2]
root = Path(tempfile.mkdtemp(prefix="ngterm WSL ' integration "))
env = {k: v for k, v in os.environ.items() if not k.upper().startswith(('NGTERM_', 'AGTERM_'))}
flags = subprocess.CREATE_NO_WINDOW
wsl = [str(Path(os.environ['SystemRoot']) / 'System32/wsl.exe'), '-d', args.distribution, '--exec']

def run(arguments, data=None):
    return subprocess.run(wsl + arguments, env=env, cwd=root, input=data, capture_output=True,
                          timeout=40, creationflags=flags)

def linux(path):
    result = run(['wslpath', '-a', '-u', str(path)])
    assert result.returncode == 0, result.stderr
    return result.stdout.decode().strip()

own = root / 'own resources'
shutil.copytree(repo / 'resources/shell-integration/wsl', own / 'shell-integration/wsl')
ghostty = root / 'ghostty resources'
shutil.copytree(args.ghostty_resources / 'shell-integration', ghostty / 'shell-integration')
home = root / 'home'
home.mkdir()
work = home / "folder with spaces % # '"
work.mkdir()
home_linux, work_linux = linux(home), linux(work)
shell = Path(args.shell).name
if shell == 'bash':
    (home / '.bash_profile').write_text('PROFILE_COUNT=$(( ${PROFILE_COUNT:-0} + 1 )); export PROFILE_COUNT\n. "$HOME/.bashrc"\n', newline='\n')
    (home / '.bashrc').write_text("PS1='CUSTOM_PROMPT> '\nPROMPT_COMMAND=('true')\n", newline='\n')
else:
    functions = '' if not args.zsh_functions else 'fpath=(' + shlex.quote(args.zsh_functions) + '/* $fpath)\n'
    modules = '' if not args.zsh_modules else 'module_path=(' + shlex.quote(args.zsh_modules) + ' $module_path)\n'
    (home / '.zshenv').write_text(functions + modules + 'ENV_COUNT=$(( ${ENV_COUNT:-0} + 1 )); export ENV_COUNT\n', newline='\n')
    (home / '.zprofile').write_text('PROFILE_COUNT=$(( ${PROFILE_COUNT:-0} + 1 )); export PROFILE_COUNT\n', newline='\n')
    (home / '.zshrc').write_text("PS1='CUSTOM_PROMPT> '\n", newline='\n')
launch = (repo / 'resources/shell-integration/wsl/launch.sh').read_text()
command = ['/usr/bin/env', f'HOME={home_linux}', f'ZDOTDIR={home_linux}', f'SHELL={args.shell}',
           f'ENV={home_linux}/original-env', f'HISTFILE={home_linux}/custom-history', 'TERM=xterm-256color',
           '/bin/sh', '-c', launch, 'ngterm-wsl', str(ghostty), str(own), 'login', ':', work_linux]
script = (f"cd {shlex.quote(work_linux)}\n"
          "printf 'PROFILE=%s ENVCOUNT=%s ENV=%s HIST=%s ZDOT=%s\\n' \"$PROFILE_COUNT\" \"${ENV_COUNT:-0}\" \"$ENV\" \"$HISTFILE\" \"$ZDOTDIR\"\n"
          "/bin/sh -c 'exit 23'\nexit\n").encode()
result = run(['/usr/bin/script', '-qefc', shlex.join(command), '/dev/null'], script)
output = result.stdout + result.stderr
(root / 'output.bin').write_bytes(output)
assert b'PROFILE=1' in output, output
assert b'CUSTOM_PROMPT>' in output, output
assert b'\x1b]133;C' in output, output
assert b'\x1b]133;D;23' in output, output
assert ('\x1b]7;kitty-shell-cwd://localhost' + work_linux + '\x07').encode() in output, output
assert (f'HIST={home_linux}/custom-history').encode() in output, output
if shell == 'bash':
    assert (f'ENV={home_linux}/original-env').encode() in output, output
else:
    assert b'ENVCOUNT=1' in output, output
    assert (f'ZDOT={home_linux}').encode() in output, output
# Missing or inaccessible integration must leave a usable selected shell.
fallback = command.copy()
fallback[-5] = str(root / 'missing ghostty')
result = run(['/usr/bin/script', '-qefc', shlex.join(fallback), '/dev/null'], b"printf 'FALLBACK_OK\\n'\nexit\n")
assert b'FALLBACK_OK' in result.stdout, result.stderr
returning = command.copy()
returning[-3] = 'return'
returning[-2] = "export NGTERM_TEST_RETURN='quoted words'"
result = run(['/usr/bin/script', '-qefc', shlex.join(returning), '/dev/null'],
             b"printf 'RETURN=%s\\n' \"$NGTERM_TEST_RETURN\"\nexit\n")
assert b'RETURN=quoted words' in result.stdout, result.stderr
assert b'\x1b]133;C' in result.stdout, result.stderr
ending = command.copy()
ending[-3] = 'end'
ending[-2] = 'exit 19'
result = run(ending)
assert result.returncode == 19, (result.returncode, result.stderr)
missing_cwd = command.copy()
missing_cwd[-1] = '/nonexistent/ngterm-integration-test'
result = run(['/usr/bin/script', '-qefc', shlex.join(missing_cwd), '/dev/null'], b"printf 'CWD_FALLBACK_OK\\n'\nexit\n")
assert b'CWD_FALLBACK_OK' in result.stdout, result.stderr
print(f'PASS: WSL {shell} command markers/exit status, cwd, startup files, history, custom prompt and fallback. Logs: {root}')
