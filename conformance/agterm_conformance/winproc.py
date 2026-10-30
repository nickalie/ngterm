"""Windows processes for the harness, through the Win32 API (`ctypes`): the process table, a process's command line and
creation time, its descendants, a kill by pid that re-checks the process is the one seen, and the polite quit a
Windows GUI app gets.

Windows has no `/proc` and no signals. A process is identified by its pid together with its creation time, so a pid
the system handed to a new process is never taken for the old one; a descendant is a process whose parent pid is an
ancestor's and which was created after it (a parent pid is never cleared, and may be reused once its process is gone).
The quit is the Restart Manager's: `WM_QUERYENDSESSION` then `WM_ENDSESSION` with `ENDSESSION_CLOSEAPP`, sent to a
top-level window of the process (a GUI app with no console takes no Ctrl event). Importable anywhere; every function
needs Windows.
"""

from __future__ import annotations

import ctypes
import sys
from collections.abc import Iterable
from dataclasses import dataclass

if sys.platform == "win32":
    from ctypes import wintypes

    _kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    _user32 = ctypes.WinDLL("user32", use_last_error=True)
    _shell32 = ctypes.WinDLL("shell32", use_last_error=True)
    _ntdll = ctypes.WinDLL("ntdll")

    class _ProcessEntry(ctypes.Structure):
        _fields_ = [
            ("dwSize", wintypes.DWORD),
            ("cntUsage", wintypes.DWORD),
            ("th32ProcessID", wintypes.DWORD),
            ("th32DefaultHeapID", ctypes.c_size_t),
            ("th32ModuleID", wintypes.DWORD),
            ("cntThreads", wintypes.DWORD),
            ("th32ParentProcessID", wintypes.DWORD),
            ("pcPriClassBase", wintypes.LONG),
            ("dwFlags", wintypes.DWORD),
            ("szExeFile", wintypes.WCHAR * 260),
        ]

    class _UnicodeString(ctypes.Structure):
        _fields_ = [("Length", wintypes.USHORT), ("MaximumLength", wintypes.USHORT), ("Buffer", ctypes.c_void_p)]

    _kernel32.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
    _kernel32.CreateToolhelp32Snapshot.argtypes = [wintypes.DWORD, wintypes.DWORD]
    _kernel32.Process32FirstW.argtypes = [wintypes.HANDLE, ctypes.POINTER(_ProcessEntry)]
    _kernel32.Process32NextW.argtypes = [wintypes.HANDLE, ctypes.POINTER(_ProcessEntry)]
    _kernel32.OpenProcess.restype = wintypes.HANDLE
    _kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    _kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    _kernel32.GetProcessTimes.argtypes = [wintypes.HANDLE, *([ctypes.POINTER(wintypes.FILETIME)] * 4)]
    _kernel32.TerminateProcess.argtypes = [wintypes.HANDLE, wintypes.UINT]
    _kernel32.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
    _kernel32.WaitForSingleObject.restype = wintypes.DWORD
    _kernel32.GetExitCodeProcess.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)]
    _kernel32.QueryFullProcessImageNameW.argtypes = [
        wintypes.HANDLE,
        wintypes.DWORD,
        wintypes.LPWSTR,
        ctypes.POINTER(wintypes.DWORD),
    ]
    _kernel32.LocalFree.argtypes = [ctypes.c_void_p]
    _ntdll.NtQueryInformationProcess.restype = wintypes.LONG
    _ntdll.NtQueryInformationProcess.argtypes = [
        wintypes.HANDLE,
        ctypes.c_int,
        ctypes.c_void_p,
        wintypes.ULONG,
        ctypes.POINTER(wintypes.ULONG),
    ]
    _shell32.CommandLineToArgvW.restype = ctypes.POINTER(wintypes.LPWSTR)
    _shell32.CommandLineToArgvW.argtypes = [wintypes.LPCWSTR, ctypes.POINTER(ctypes.c_int)]
    _EnumWindowsProc = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    _user32.EnumWindows.argtypes = [_EnumWindowsProc, wintypes.LPARAM]
    _user32.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
    _user32.GetClassNameW.argtypes = [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int]
    _user32.SendMessageTimeoutW.restype = wintypes.LPARAM
    _user32.SendMessageTimeoutW.argtypes = [
        wintypes.HWND,
        wintypes.UINT,
        wintypes.WPARAM,
        wintypes.LPARAM,
        wintypes.UINT,
        wintypes.UINT,
        ctypes.POINTER(ctypes.c_size_t),
    ]
    _user32.PostMessageW.argtypes = [wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM]

TH32CS_SNAPPROCESS = 0x2
INVALID_HANDLE_VALUE = ctypes.c_void_p(-1).value
PROCESS_TERMINATE = 0x0001
PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
SYNCHRONIZE = 0x00100000
# NtQueryInformationProcess's ProcessCommandLineInformation (Windows 8.1 and later).
PROCESS_COMMAND_LINE_INFORMATION = 60
STATUS_INFO_LENGTH_MISMATCH = ctypes.c_long(0xC0000004).value
WAIT_OBJECT_0 = 0
STILL_ACTIVE = 259
WM_QUERYENDSESSION = 0x0011
WM_ENDSESSION = 0x0016
ENDSESSION_CLOSEAPP = 0x1
SMTO_ABORTIFHUNG = 0x2
# How long a window gets to answer `WM_QUERYENDSESSION`.
QUERY_TIMEOUT_MS = 5000


@dataclass(frozen=True)
class Process:
    """One process as the snapshot saw it: its creation time (a FILETIME count) is None when it could not be read
    (gone already, or another user's)."""

    pid: int
    ppid: int
    name: str
    created: int | None


def processes() -> list[Process]:
    """Every process in the system's process table now."""
    snapshot = _kernel32.CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)
    if snapshot in (None, INVALID_HANDLE_VALUE):
        raise ctypes.WinError(ctypes.get_last_error())
    seen: list[tuple[int, int, str]] = []
    try:
        entry = _ProcessEntry()
        entry.dwSize = ctypes.sizeof(_ProcessEntry)
        more = _kernel32.Process32FirstW(snapshot, ctypes.byref(entry))
        while more:
            seen.append((entry.th32ProcessID, entry.th32ParentProcessID, entry.szExeFile))
            more = _kernel32.Process32NextW(snapshot, ctypes.byref(entry))
    finally:
        _kernel32.CloseHandle(snapshot)
    return [Process(pid, ppid, name, creation_time(pid)) for pid, ppid, name in seen if pid]


def _open(pid: int, access: int) -> int | None:
    handle = _kernel32.OpenProcess(access, False, pid)
    return handle or None


def creation_time(pid: int) -> int | None:
    handle = _open(pid, PROCESS_QUERY_LIMITED_INFORMATION)
    if handle is None:
        return None
    try:
        return _creation_time(handle)
    finally:
        _kernel32.CloseHandle(handle)


def _creation_time(handle: int) -> int | None:
    times = [wintypes.FILETIME() for _ in range(4)]
    if not _kernel32.GetProcessTimes(handle, *(ctypes.byref(time) for time in times)):
        return None
    return (times[0].dwHighDateTime << 32) | times[0].dwLowDateTime


def image_path(pid: int) -> str | None:
    handle = _open(pid, PROCESS_QUERY_LIMITED_INFORMATION)
    if handle is None:
        return None
    try:
        size = wintypes.DWORD(32768)
        buffer = ctypes.create_unicode_buffer(size.value)
        if not _kernel32.QueryFullProcessImageNameW(handle, 0, buffer, ctypes.byref(size)):
            return None
        return buffer.value
    finally:
        _kernel32.CloseHandle(handle)


def command_line(pid: int) -> list[str] | None:
    """The process's argv, its command line split as the C runtime splits it (`CommandLineToArgvW`); None when it
    cannot be read."""
    handle = _open(pid, PROCESS_QUERY_LIMITED_INFORMATION)
    if handle is None:
        return None
    try:
        size = wintypes.ULONG(0)
        status = _ntdll.NtQueryInformationProcess(handle, PROCESS_COMMAND_LINE_INFORMATION, None, 0, ctypes.byref(size))
        if status != STATUS_INFO_LENGTH_MISMATCH or not size.value:
            return None
        buffer = ctypes.create_string_buffer(size.value)
        status = _ntdll.NtQueryInformationProcess(
            handle, PROCESS_COMMAND_LINE_INFORMATION, buffer, size, ctypes.byref(size)
        )
        if status < 0:
            return None
        text = _UnicodeString.from_buffer(buffer)
        line = ctypes.wstring_at(text.Buffer, text.Length // 2) if text.Buffer else ""
    finally:
        _kernel32.CloseHandle(handle)
    return split_command_line(line)


def split_command_line(line: str) -> list[str]:
    if not line:
        return []
    count = ctypes.c_int(0)
    words = _shell32.CommandLineToArgvW(line, ctypes.byref(count))
    if not words:
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        return [words[index] for index in range(count.value)]
    finally:
        _kernel32.LocalFree(words)


def descendants(roots: Iterable[Process], table: list[Process] | None = None) -> list[Process]:
    """Every process descending from one of `roots` (the roots left out): a child names its parent's pid and was
    created after it, so a pid reused since is not mistaken for the parent."""
    table = processes() if table is None else table
    children: dict[int, list[Process]] = {}
    for process in table:
        children.setdefault(process.ppid, []).append(process)
    found: dict[tuple[int, int | None], Process] = {}
    pending = list(roots)
    seen = {(root.pid, root.created) for root in pending}
    while pending:
        parent = pending.pop()
        for child in children.get(parent.pid, []):
            key = (child.pid, child.created)
            if key in seen or child.pid == parent.pid:
                continue
            if parent.created is not None and (child.created is None or child.created < parent.created):
                continue
            seen.add(key)
            found[key] = child
            pending.append(child)
    return list(found.values())


def alive(process: Process) -> bool:
    """Whether the process the snapshot saw is still running (the same pid, created at the same time)."""
    handle = _open(process.pid, PROCESS_QUERY_LIMITED_INFORMATION)
    if handle is None:
        return False
    try:
        if process.created is not None and _creation_time(handle) != process.created:
            return False
        code = wintypes.DWORD(0)
        return bool(_kernel32.GetExitCodeProcess(handle, ctypes.byref(code))) and code.value == STILL_ACTIVE
    finally:
        _kernel32.CloseHandle(handle)


def terminate(process: Process, code: int = 1, wait_ms: int = 5000) -> bool:
    """Ends `process` by its pid once its creation time shows it is the one seen; True when it ended here."""
    handle = _open(process.pid, PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE)
    if handle is None:
        return False
    try:
        if process.created is not None and _creation_time(handle) != process.created:
            return False
        if not _kernel32.TerminateProcess(handle, code):
            return False
        _kernel32.WaitForSingleObject(handle, wait_ms)
        return True
    finally:
        _kernel32.CloseHandle(handle)


def top_level_windows(pid: int, class_name: str | None = None) -> list[int]:
    """The process's top-level windows, hidden ones included, of `class_name` when given."""
    found: list[int] = []

    def visit(window: int, _: int) -> bool:
        owner = wintypes.DWORD(0)
        _user32.GetWindowThreadProcessId(window, ctypes.byref(owner))
        if owner.value == pid:
            if class_name is None:
                found.append(window)
            else:
                name = ctypes.create_unicode_buffer(256)
                if _user32.GetClassNameW(window, name, 256) and name.value == class_name:
                    found.append(window)
        return True

    _user32.EnumWindows(_EnumWindowsProc(visit), 0)
    return found


def ask_to_end(pid: int, class_name: str) -> bool:
    """The Restart Manager's quit, sent to the process's top-level windows of `class_name`: `WM_QUERYENDSESSION`, then
    `WM_ENDSESSION` posted (its handler may hold the window until the process ends). False without such a window."""
    windows = top_level_windows(pid, class_name)
    for window in windows:
        answer = ctypes.c_size_t(0)
        _user32.SendMessageTimeoutW(
            window, WM_QUERYENDSESSION, 0, ENDSESSION_CLOSEAPP, SMTO_ABORTIFHUNG, QUERY_TIMEOUT_MS, ctypes.byref(answer)
        )
        _user32.PostMessageW(window, WM_ENDSESSION, 1, ENDSESSION_CLOSEAPP)
    return bool(windows)
