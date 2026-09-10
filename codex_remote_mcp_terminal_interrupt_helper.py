from __future__ import annotations

# pyright: reportAny=false

import ctypes
import ctypes.wintypes as wt
import os
import sys
import time
from typing import Final

_CTRL_C_EVENT: Final = 0


def main(arguments: list[str]) -> int:
    if os.name != "nt" or len(arguments) != 1 or not arguments[0].isdigit():
        return 64
    process_id = int(arguments[0])
    if process_id <= 0:
        return 64
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.FreeConsole.restype = wt.BOOL
    kernel32.AttachConsole.argtypes = [wt.DWORD]
    kernel32.AttachConsole.restype = wt.BOOL
    kernel32.SetConsoleCtrlHandler.argtypes = [ctypes.c_void_p, wt.BOOL]
    kernel32.SetConsoleCtrlHandler.restype = wt.BOOL
    kernel32.GenerateConsoleCtrlEvent.argtypes = [wt.DWORD, wt.DWORD]
    kernel32.GenerateConsoleCtrlEvent.restype = wt.BOOL

    _ = kernel32.FreeConsole()
    if not kernel32.AttachConsole(process_id):
        return 2
    try:
        if not kernel32.SetConsoleCtrlHandler(None, True):
            return 3
        if not kernel32.GenerateConsoleCtrlEvent(_CTRL_C_EVENT, 0):
            return 4
        time.sleep(0.1)
        return 0
    finally:
        _ = kernel32.FreeConsole()


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
