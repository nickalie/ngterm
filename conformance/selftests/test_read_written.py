"""`read_written`: the text of a file a command line wrote and renamed into place, read in the moment the rename's own
handle still holds it (the `windows` CI job runs the Windows cases)."""

import os
import subprocess
import sys

import pytest
from agterm_conformance.command_line import CommandLines, read_written

windows = pytest.mark.skipif(sys.platform != "win32", reason="the Windows sharing modes")

RENAMES = 300


def test_it_reads_the_whole_text(tmp_path):
    path = tmp_path / "marker"
    path.write_text("id|left|1", encoding="utf-8")
    assert read_written(path) == "id|left|1"


def test_a_missing_file_is_not_found(tmp_path):
    with pytest.raises(FileNotFoundError):
        read_written(tmp_path / "missing")


@windows
def test_it_reads_a_file_a_handle_with_delete_access_holds(tmp_path):
    import ctypes
    from ctypes import wintypes

    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.CreateFileW.restype = wintypes.HANDLE
    kernel32.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, wintypes.LPVOID,
                                     wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE]
    kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    path = tmp_path / "marker"
    path.write_text("text", encoding="utf-8")
    delete, share_all, open_existing = 0x10000, 0x7, 3
    handle = kernel32.CreateFileW(os.fspath(path), delete, share_all, None, open_existing, 0, None)
    assert handle != wintypes.HANDLE(-1).value, ctypes.get_last_error()
    try:
        with pytest.raises(PermissionError):
            path.read_text(encoding="utf-8")
        assert read_written(path) == "text"
    finally:
        kernel32.CloseHandle(handle)


def _renames(tmp_path, read):
    """Runs the `cmd` line `write` makes `RENAMES` times and reads each file as soon as it exists; the errors seen."""
    lines = CommandLines(cmd=True)
    errors = []
    for index in range(RENAMES):
        path = tmp_path / f"marker-{index}"
        command = lines.write(path, "id", "left", str(index))
        process = subprocess.Popen(f'{os.environ["ComSpec"]} /D /S /C "{command}"')
        try:
            while True:
                if path.exists():
                    try:
                        assert read(path) == f"id|left|{index}"
                        break
                    except PermissionError as error:
                        errors.append(error)
                elif process.poll() is not None and not path.exists():
                    pytest.fail(f"{path.name} was never written")
        finally:
            process.wait()
    return errors


@windows
def test_a_file_cmd_move_renames_into_place_reads_as_soon_as_it_exists(tmp_path):
    assert _renames(tmp_path, read_written) == []

