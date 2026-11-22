"""The command lines a test's hooks, custom commands, overlay programs and scratch commands run, in the dialect the
build runs them in: POSIX `sh` (capability `posix-command-line`, upstream's) or Windows `cmd.exe` (`cmd-command-line`,
the Rust build on Windows, which runs such a line as `%ComSpec% /D /S /C "<line>"`). A build claiming neither skips the
test as unsupported.

Every line built here writes only the paths it is given, each quoted for its dialect; a file meant to be read whole
is written to a temp name and renamed, so a reader never sees it half written; [`read_written`] reads such a
file."""

from __future__ import annotations

import os
import shlex
import subprocess
import sys
from dataclasses import dataclass

import pytest

if sys.platform == "win32":
    import ctypes
    import msvcrt
    from ctypes import wintypes

    _kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    _kernel32.CreateFileW.restype = wintypes.HANDLE
    _kernel32.CreateFileW.argtypes = [
        wintypes.LPCWSTR,
        wintypes.DWORD,
        wintypes.DWORD,
        wintypes.LPVOID,
        wintypes.DWORD,
        wintypes.DWORD,
        wintypes.HANDLE,
    ]
    _GENERIC_READ = 0x80000000
    _SHARE_ALL = 0x1 | 0x2 | 0x4  # FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE
    _OPEN_EXISTING = 3
    _INVALID_HANDLE = wintypes.HANDLE(-1).value

POSIX = "posix-command-line"
CMD = "cmd-command-line"


def env_has_argv0(env: str = "/usr/bin/env") -> bool:
    """Whether `env` takes `--argv0` (GNU env does, macOS's does not)."""
    try:
        run = subprocess.run([env, "--argv0", "sh", "/bin/sh", "-c", ":"], capture_output=True, check=False)
    except OSError:
        return False
    return run.returncode == 0


def read_written(path) -> str:
    """The text of a file that [`CommandLines.write`] renamed into place.

    On Windows, `move` renames through a handle with delete access and closes that handle only after the rename, so
    the file is already at its name while the handle is open. A plain `open` does not share delete access and fails
    with a sharing violation (`PermissionError`) in that moment; this open shares it."""
    if sys.platform != "win32":
        with open(path, encoding="utf-8") as file:
            return file.read()
    handle = _kernel32.CreateFileW(os.fspath(path), _GENERIC_READ, _SHARE_ALL, None, _OPEN_EXISTING, 0, None)
    if handle == _INVALID_HANDLE:
        error = ctypes.get_last_error()
        raise OSError(None, ctypes.FormatError(error).strip(), os.fspath(path), error)
    with open(msvcrt.open_osfhandle(handle, os.O_RDONLY), encoding="utf-8") as file:
        return file.read()


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

    def hold(self) -> str:
        """A program that prints nothing and stays up reading its terminal until the terminal goes: `cat`, or `more`
        under `cmd` (it reads the console until Ctrl+Z, and erases the line the cursor is on as it starts, so what
        comes before it ends with a newline)."""
        return "more" if self.cmd else "exec cat"

    def pwd(self) -> str:
        """Prints the working directory (`cmd`'s bare `cd`)."""
        return "cd" if self.cmd else "pwd"

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
