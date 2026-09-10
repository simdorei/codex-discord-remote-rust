from __future__ import annotations

# pyright: reportAny=false

import ctypes
import ctypes.wintypes as wt
import os
import subprocess
import sys
from typing import Final

_CREATE_SUSPENDED: Final = 0x00000004
_PROCESS_SUSPEND_RESUME: Final = 0x0800
_TITLE_ENVIRONMENT: Final = "SIMDOREI_MCP_TERMINAL_WINDOW_TITLE"


def main(arguments: list[str]) -> int:
    if os.name != "nt" or arguments:
        return 64
    title = os.environ.get(_TITLE_ENVIRONMENT)
    if title is None or not title:
        return 64

    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.SetConsoleCtrlHandler.argtypes = [ctypes.c_void_p, wt.BOOL]
    kernel32.SetConsoleCtrlHandler.restype = wt.BOOL
    if not kernel32.SetConsoleCtrlHandler(None, False):
        return 2

    process: subprocess.Popen[bytes] | None = None
    try:
        process = subprocess.Popen(
            [
                "powershell.exe",
                "-NoLogo",
                "-NoProfile",
                "-NoExit",
                "-Command",
                f"$Host.UI.RawUI.WindowTitle = $env:{_TITLE_ENVIRONMENT}",
            ],
            close_fds=True,
            creationflags=_CREATE_SUSPENDED,
        )
        if not kernel32.SetConsoleCtrlHandler(None, True):
            return 3
        if not _resume_process(kernel32, process.pid):
            return 4
        return process.wait()
    except OSError:
        return 5
    finally:
        if process is not None and process.poll() is None:
            process.kill()
            _ = process.wait()


def _resume_process(kernel32: ctypes.WinDLL, process_id: int) -> bool:
    kernel32.OpenProcess.argtypes = [wt.DWORD, wt.BOOL, wt.DWORD]
    kernel32.OpenProcess.restype = wt.HANDLE
    kernel32.CloseHandle.argtypes = [wt.HANDLE]
    kernel32.CloseHandle.restype = wt.BOOL
    process_handle = kernel32.OpenProcess(
        _PROCESS_SUSPEND_RESUME,
        False,
        process_id,
    )
    if not process_handle:
        return False
    try:
        ntdll = ctypes.WinDLL("ntdll", use_last_error=True)
        ntdll.NtResumeProcess.argtypes = [wt.HANDLE]
        ntdll.NtResumeProcess.restype = ctypes.c_long
        return int(ntdll.NtResumeProcess(process_handle)) == 0
    finally:
        _ = kernel32.CloseHandle(process_handle)


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
