"""The command lines a test's hooks and custom commands run, in the dialect the build runs them in: POSIX `sh`
(capability `posix-command-line`, upstream's) or Windows `cmd.exe` (`cmd-command-line`, the Rust build on Windows,
which runs a hook's and a custom command's line as `%ComSpec% /D /S /C "<line>"`). A build claiming neither skips the
test as unsupported.

Every line built here writes only the paths it is given, each quoted for its dialect; a file meant to be read whole
is written to a temp name and renamed, so a reader never sees it half written."""

from __future__ import annotations

import os
import shlex
from dataclasses import dataclass

import pytest

POSIX = "posix-command-line"
CMD = "cmd-command-line"


@dataclass(frozen=True)
class CommandLines:
    cmd: bool

    @classmethod
    def of(cls, adapter, what: str = "the hooks and custom commands") -> CommandLines:
        """The adapter's dialect; skips the test as unsupported when it claims neither."""
        if adapter.has(POSIX):
            return cls(cmd=False)
        if adapter.has(CMD):
            return cls(cmd=True)
        pytest.skip(f"unsupported: {adapter.name} lacks {POSIX} and {CMD} ({what} are command lines)")

    def quote(self, path) -> str:
        path = os.fspath(path)
        return f'"{path}"' if self.cmd else shlex.quote(path)

    def var(self, name: str) -> str:
        """A variable's value as a word of its own, quoted where the dialect needs it."""
        return f"%{name}%" if self.cmd else f'"${name}"'

    def bare_var(self, name: str) -> str:
        """A variable's value inside a word: a path's name (`cmd` expands it inside quotes, `sh` outside them)."""
        return f"%{name}%" if self.cmd else f"${name}"

    def then(self, *lines: str) -> str:
        """`lines` one after the other, whatever each exits with."""
        return (" & " if self.cmd else "; ").join(lines)

    def drain_stdin(self) -> str:
        return "more > nul" if self.cmd else "cat > /dev/null"

    def save_stdin(self, path) -> str:
        """Stdin into `path` (`cmd`'s `more` ends it with an empty line, which a JSON reader ignores)."""
        return f"more > {self.quote(path)}" if self.cmd else f"cat > {self.quote(path)}"

    def append_stdin_line(self, path) -> str:
        """Stdin appended to `path`, then a newline in case it ended without one."""
        quoted = self.quote(path)
        if self.cmd:
            return f"more >> {quoted} & echo.>> {quoted}"
        return f"cat >> {quoted}; echo >> {quoted}"

    def write(self, path, *words: str, sep: str = "|", atomic: bool = True) -> str:
        """`words` (literal text or [`var`] values) joined by `sep`, without a newline, into `path`; through a temp
        file renamed into place when `atomic`. A literal word holds no quote, `%` or `$`."""
        target = os.fspath(path)
        written = f"{target}.tmp" if atomic else target
        if self.cmd:
            # `set /p` prints its prompt without a newline; its exit status is 1 with no input, so `&`, not `&&`
            text = sep.join(words)
            line = f'>{self.quote(written)} <nul set /p "={text}"'
            return f"{line} & move /y {self.quote(written)} {self.quote(target)} >nul" if atomic else line
        formats = sep.join("%s" for _ in words)
        line = f"printf '{formats}' {' '.join(words)} > {self.quote(written)}"
        return f"{line} && mv {self.quote(written)} {self.quote(target)}" if atomic else line
