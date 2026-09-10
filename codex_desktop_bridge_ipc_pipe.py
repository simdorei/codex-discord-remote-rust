# pyright: reportAny=false, reportUnknownMemberType=false
from __future__ import annotations

import ctypes
import ctypes.wintypes as wt
import json
import os
import time
from typing import cast

from codex_bridge_state import JsonObject

CODEX_IPC_PIPE = r"\\.\pipe\codex-ipc"
GENERIC_READ = 0x80000000
GENERIC_WRITE = 0x40000000
INVALID_HANDLE_VALUE = ctypes.c_void_p(-1).value
OPEN_EXISTING = 3
PIPE_PEEK_RETRY_SEC = 0.05


class IPCPipeError(RuntimeError):
    pass


class IPCPipeOpenError(IPCPipeError):
    def __init__(self, detail: str) -> None:
        self.detail: str = detail
        super().__init__(f"Could not open {CODEX_IPC_PIPE}: {detail}")


class IPCPipePeekError(IPCPipeError):
    def __init__(self, detail: str) -> None:
        self.detail: str = detail
        super().__init__(f"Could not peek {CODEX_IPC_PIPE}: {detail}")


class IPCPipeReadError(IPCPipeError):
    def __init__(self, detail: str) -> None:
        self.detail: str = detail
        super().__init__(f"Could not read from {CODEX_IPC_PIPE}: {detail}")


class IPCShortReadError(IPCPipeError):
    def __init__(self, expected_size: int, actual_size: int) -> None:
        self.expected_size: int = expected_size
        self.actual_size: int = actual_size
        super().__init__(
            f"Short IPC read from {CODEX_IPC_PIPE}: expected {expected_size}, got {actual_size}."
        )


class IPCInvalidPayloadLengthError(IPCPipeError):
    def __init__(self) -> None:
        super().__init__("IPC payload length was negative.")


class IPCInvalidMessageError(IPCPipeError):
    def __init__(self) -> None:
        super().__init__("IPC message was not a JSON object.")


class IPCPipeWriteError(IPCPipeError):
    def __init__(self, detail: str) -> None:
        self.detail: str = detail
        super().__init__(f"Could not write to {CODEX_IPC_PIPE}: {detail}")


class IPCShortWriteError(IPCPipeError):
    def __init__(self, expected_size: int, actual_size: int) -> None:
        self.expected_size: int = expected_size
        self.actual_size: int = actual_size
        super().__init__(
            f"Short IPC write to {CODEX_IPC_PIPE}: expected {expected_size}, got {actual_size}."
        )

kernel32 = ctypes.windll.kernel32 if os.name == "nt" else None

if kernel32 is not None:
    kernel32.CreateFileW.argtypes = [
        wt.LPCWSTR,
        wt.DWORD,
        wt.DWORD,
        wt.LPVOID,
        wt.DWORD,
        wt.DWORD,
        wt.HANDLE,
    ]
    kernel32.CreateFileW.restype = wt.HANDLE
    kernel32.ReadFile.argtypes = [wt.HANDLE, wt.LPVOID, wt.DWORD, ctypes.POINTER(wt.DWORD), wt.LPVOID]
    kernel32.ReadFile.restype = wt.BOOL
    kernel32.WriteFile.argtypes = [wt.HANDLE, wt.LPCVOID, wt.DWORD, ctypes.POINTER(wt.DWORD), wt.LPVOID]
    kernel32.WriteFile.restype = wt.BOOL
    kernel32.CloseHandle.argtypes = [wt.HANDLE]
    kernel32.CloseHandle.restype = wt.BOOL
    kernel32.PeekNamedPipe.argtypes = [
        wt.HANDLE,
        wt.LPVOID,
        wt.DWORD,
        ctypes.POINTER(wt.DWORD),
        ctypes.POINTER(wt.DWORD),
        ctypes.POINTER(wt.DWORD),
    ]
    kernel32.PeekNamedPipe.restype = wt.BOOL


def require_kernel32() -> ctypes.CDLL:
    if kernel32 is None:
        raise IPCPipeError("Codex IPC pipe is only implemented on Windows.")
    return kernel32


def get_last_win_error_message() -> str:
    code = int(require_kernel32().GetLastError())
    if not code:
        return "unknown Windows error"
    return f"{ctypes.WinError(code)}"


def open_codex_ipc_pipe() -> int:
    if os.name != "nt":
        raise IPCPipeOpenError("Windows named-pipe IPC is only implemented on Windows.")

    handle = require_kernel32().CreateFileW(
        CODEX_IPC_PIPE,
        GENERIC_READ | GENERIC_WRITE,
        0,
        None,
        OPEN_EXISTING,
        0,
        None,
    )
    if not handle or handle == INVALID_HANDLE_VALUE:
        raise IPCPipeOpenError(get_last_win_error_message())
    return int(handle)


def peek_pipe_bytes_available(handle: int) -> int:
    win32 = require_kernel32()
    total_available = wt.DWORD(0)
    ok = win32.PeekNamedPipe(handle, None, 0, None, ctypes.byref(total_available), None)
    if not ok:
        raise IPCPipePeekError(get_last_win_error_message())
    return int(total_available.value)


def wait_for_pipe_bytes(handle: int, min_bytes: int, timeout_sec: float) -> None:
    deadline = time.time() + timeout_sec
    while time.time() < deadline:
        if peek_pipe_bytes_available(handle) >= min_bytes:
            return
        time.sleep(PIPE_PEEK_RETRY_SEC)
    raise TimeoutError(f"Timed out waiting for IPC data from {CODEX_IPC_PIPE}.")


def read_pipe_exact(handle: int, size: int, timeout_sec: float) -> bytes:
    win32 = require_kernel32()
    wait_for_pipe_bytes(handle, size, timeout_sec)
    buffer = ctypes.create_string_buffer(size)
    bytes_read = wt.DWORD(0)
    ok = win32.ReadFile(handle, buffer, size, ctypes.byref(bytes_read), None)
    if not ok:
        raise IPCPipeReadError(get_last_win_error_message())
    if int(bytes_read.value) != size:
        raise IPCShortReadError(size, int(bytes_read.value))
    return buffer.raw[:size]


def read_ipc_message(handle: int, timeout_sec: float) -> JsonObject:
    header = read_pipe_exact(handle, 4, timeout_sec)
    payload_size = int.from_bytes(header, "little")
    if payload_size < 0:
        raise IPCInvalidPayloadLengthError()
    payload = read_pipe_exact(handle, payload_size, timeout_sec)
    value = json.loads(payload.decode("utf-8", errors="ignore"))
    if not isinstance(value, dict):
        raise IPCInvalidMessageError()
    return cast(JsonObject, value)


def write_ipc_message(handle: int, payload: JsonObject) -> None:
    win32 = require_kernel32()
    data = json.dumps(payload, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
    frame = len(data).to_bytes(4, "little") + data
    buffer = ctypes.create_string_buffer(frame)
    bytes_written = wt.DWORD(0)
    ok = win32.WriteFile(handle, buffer, len(frame), ctypes.byref(bytes_written), None)
    if not ok:
        raise IPCPipeWriteError(get_last_win_error_message())
    if int(bytes_written.value) != len(frame):
        raise IPCShortWriteError(len(frame), int(bytes_written.value))


def close_ipc_pipe(handle: int) -> None:
    _ = require_kernel32().CloseHandle(handle)
