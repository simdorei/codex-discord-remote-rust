from __future__ import annotations

import ctypes
import os
from collections.abc import Callable, Generator
from contextlib import contextmanager
from pathlib import Path
from typing import Final, Protocol, TypeAlias, cast


LogLine: TypeAlias = Callable[[str], None]
ERROR_ALREADY_EXISTS: Final = 183


class RemoveRuntimeLockFunc(Protocol):
    def __call__(self, *, reason: str) -> None: ...


def remove_runtime_lock_for_current_process(
    runtime_lock_path: Path,
    *,
    reason: str,
    log_func: LogLine,
) -> None:
    current_pid = str(os.getpid())
    try:
        if runtime_lock_path.read_text(encoding="ascii").strip() == current_pid:
            runtime_lock_path.unlink()
            log_func(f"runtime_lock_removed path={runtime_lock_path} pid={current_pid} reason={reason}")
    except OSError as exc:
        log_func(
            f"runtime_lock_remove_failed path={runtime_lock_path} pid={current_pid} "
            + f"reason={reason} error_type={type(exc).__name__} error={exc}"
        )


@contextmanager
def acquire_runtime_instance_lock(
    mutex_name: str,
    *,
    runtime_mutex_name: str,
    runtime_lock_path: Path,
    log_func: LogLine,
    remove_runtime_lock_func: RemoveRuntimeLockFunc,
) -> Generator[bool, None, None]:
    if os.name != "nt":
        yield True
        return

    kernel32 = ctypes.windll.kernel32
    kernel32.CreateMutexW.argtypes = [ctypes.c_void_p, ctypes.c_bool, ctypes.c_wchar_p]
    kernel32.CreateMutexW.restype = ctypes.c_void_p
    kernel32.GetLastError.restype = ctypes.c_ulong
    kernel32.ReleaseMutex.argtypes = [ctypes.c_void_p]
    kernel32.ReleaseMutex.restype = ctypes.c_bool
    kernel32.CloseHandle.argtypes = [ctypes.c_void_p]
    kernel32.CloseHandle.restype = ctypes.c_bool

    mutex = cast(ctypes.c_void_p, kernel32.CreateMutexW(None, True, mutex_name))
    if not mutex:
        raise ctypes.WinError()
    if kernel32.GetLastError() == ERROR_ALREADY_EXISTS:
        kernel32.CloseHandle(mutex)
        log_func(f"main_duplicate_instance_blocked mutex={mutex_name}")
        yield False
        return

    writes_runtime_lock = mutex_name == runtime_mutex_name
    current_pid = str(os.getpid())
    if writes_runtime_lock:
        try:
            _ = runtime_lock_path.write_text(current_pid, encoding="ascii")
            log_func(f"runtime_lock_written path={runtime_lock_path} pid={current_pid}")
        except OSError as exc:
            log_func(
                f"runtime_lock_write_failed path={runtime_lock_path} "
                + f"pid={current_pid} error_type={type(exc).__name__}"
            )

    try:
        yield True
    finally:
        if writes_runtime_lock:
            remove_runtime_lock_func(reason="normal_exit")
        kernel32.ReleaseMutex(mutex)
        kernel32.CloseHandle(mutex)


__all__ = [
    "RemoveRuntimeLockFunc",
    "acquire_runtime_instance_lock",
    "remove_runtime_lock_for_current_process",
]
