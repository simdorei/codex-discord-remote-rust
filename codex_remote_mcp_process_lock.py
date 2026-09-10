from __future__ import annotations

import ctypes
import hashlib
import os
from collections.abc import Generator
from contextlib import contextmanager
from typing import Final, cast


ERROR_ALREADY_EXISTS: Final = 183
MUTEX_PREFIX: Final = r"Local\SimdoreiRemoteMcpBridge_"


@contextmanager
def acquire_remote_mcp_process_lock(
    device_id: str,
) -> Generator[bool, None, None]:
    """Allow only one Windows process to own a device bridge."""
    if os.name != "nt":
        yield True
        return

    kernel32 = ctypes.windll.kernel32
    kernel32.CreateMutexW.argtypes = [
        ctypes.c_void_p,
        ctypes.c_bool,
        ctypes.c_wchar_p,
    ]
    kernel32.CreateMutexW.restype = ctypes.c_void_p
    kernel32.GetLastError.restype = ctypes.c_ulong
    kernel32.ReleaseMutex.argtypes = [ctypes.c_void_p]
    kernel32.ReleaseMutex.restype = ctypes.c_bool
    kernel32.CloseHandle.argtypes = [ctypes.c_void_p]
    kernel32.CloseHandle.restype = ctypes.c_bool

    device_key = hashlib.sha256(device_id.encode("utf-8")).hexdigest()[:32]
    mutex_name = f"{MUTEX_PREFIX}{device_key}"
    mutex = cast(
        ctypes.c_void_p,
        kernel32.CreateMutexW(None, True, mutex_name),
    )
    if not mutex:
        raise ctypes.WinError()
    if kernel32.GetLastError() == ERROR_ALREADY_EXISTS:
        kernel32.CloseHandle(mutex)
        yield False
        return

    try:
        yield True
    finally:
        kernel32.ReleaseMutex(mutex)
        kernel32.CloseHandle(mutex)


__all__ = ["acquire_remote_mcp_process_lock"]
