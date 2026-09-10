from __future__ import annotations

import ctypes
import os

_DATETIME_FILETIME_OFFSET = 504_911_232_000_000_000


def current_process_identity() -> str:
    """Return the identity format used by the Windows restart watchdog."""
    process_id = os.getpid()
    if os.name != "nt":
        return f"{process_id}|0"
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.GetCurrentProcess.restype = ctypes.c_void_p
    kernel32.GetProcessTimes.argtypes = [
        ctypes.c_void_p,
        ctypes.POINTER(ctypes.c_uint64),
        ctypes.POINTER(ctypes.c_uint64),
        ctypes.POINTER(ctypes.c_uint64),
        ctypes.POINTER(ctypes.c_uint64),
    ]
    kernel32.GetProcessTimes.restype = ctypes.c_int
    created = ctypes.c_uint64()
    exited = ctypes.c_uint64()
    kernel = ctypes.c_uint64()
    user = ctypes.c_uint64()
    if not kernel32.GetProcessTimes(
        kernel32.GetCurrentProcess(),
        ctypes.byref(created),
        ctypes.byref(exited),
        ctypes.byref(kernel),
        ctypes.byref(user),
    ):
        raise OSError(ctypes.get_last_error(), "GetProcessTimes failed")
    ticks = created.value + _DATETIME_FILETIME_OFFSET
    return f"{process_id}|{ticks - (ticks % 10)}"


__all__ = ["current_process_identity"]
