"""An `ssh` that reaches no machine, first on a viewer's PATH (`Adapter.launch(path_prefix=[stand_in.bin])`): it notes
each call and answers a remote command from the answers a test sets, so a remote window's ways of not connecting run on
any host, Windows included, where the harness has no sshd for a real origin (`remote_origin`).

A remote command with no answer set fails as ssh does when it cannot connect: exit 255 and ssh's words on stderr. The
answers are read on each call, so a test changes them while the viewer runs. On Windows the stand-in is distlib's
`ssh.exe` script launcher over a Python script, as pip makes console scripts: the app finds a program by `.exe` alone.
"""

from __future__ import annotations

import json
import os
import stat
import sys
from pathlib import Path

# The stand-in's body: `{answers}` and `{calls}` are filled in.
_SCRIPT = """\
import json
import sys

words = sys.argv[1:]
with open({calls!r}, "a", encoding="utf-8", newline="\\n") as out:
    out.write("\\t".join(words) + "\\n")
remote = words[-1] if words else ""
host = words[-2] if len(words) > 1 else ""
with open({answers!r}, encoding="utf-8") as source:
    answers = json.load(source)
for answer in answers:
    if answer["contains"] in remote:
        sys.stdout.write(answer["stdout"])
        sys.stdout.flush()
        sys.exit(answer["status"])
sys.stderr.write(f"ssh: connect to host {{host}} port 22: Connection refused\\r\\n")
sys.exit(255)
"""


class SshStandIn:
    """The stand-in under `root`: `bin` holds the `ssh` a viewer takes first, `answer` sets what a remote command
    gets, and `invocations` lists each call's words."""

    def __init__(self, root: Path) -> None:
        self.directory = root / "ssh-stand-in"
        self.bin = self.directory / "bin"
        self.bin.mkdir(parents=True)
        self.calls = self.directory / "calls"
        self.answers = self.directory / "answers.json"
        self.answer()
        body = _SCRIPT.format(calls=os.fspath(self.calls), answers=os.fspath(self.answers))
        if sys.platform == "win32":
            self._windows_launcher(body)
        else:
            script = self.bin / "ssh"
            script.write_text(f"#!{sys.executable}\n{body}", encoding="utf-8")
            script.chmod(script.stat().st_mode | stat.S_IXUSR)

    def answer(self, *answers: tuple[str, str, int]) -> None:
        """Each `(contains, stdout, status)`: a remote command holding `contains` prints `stdout` and exits with
        `status`; the first match wins, and any other command fails to connect. No answers: every call fails."""
        rules = [{"contains": contains, "stdout": stdout, "status": status} for contains, stdout, status in answers]
        temporary = self.answers.with_name(f".{self.answers.name}.tmp")
        temporary.write_text(json.dumps(rules), encoding="utf-8")
        # a call never reads a half-written file
        os.replace(temporary, self.answers)

    def invocations(self) -> list[list[str]]:
        try:
            lines = self.calls.read_text(encoding="utf-8").splitlines()
        except FileNotFoundError:
            return []
        return [line.split("\t") for line in lines]

    def _windows_launcher(self, body: str) -> None:
        from distlib.scripts import ScriptMaker

        source = self.directory / "source"
        source.mkdir()
        (source / "ssh.py").write_text(f"#!python\n{body}", encoding="utf-8")
        maker = ScriptMaker(os.fspath(source), os.fspath(self.bin))
        maker.executable = sys.executable
        maker.make("ssh.py")
        if not (self.bin / "ssh.exe").is_file():
            raise RuntimeError(f"distlib made no ssh.exe: {sorted(os.listdir(self.bin))}")
