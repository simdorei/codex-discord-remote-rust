# pyright: reportAny=false
from __future__ import annotations

import ctypes
from functools import cache
from pathlib import Path
from typing import Final, final

from codex_remote_mcp_windows_file_native import WindowsNativeFileError

_FILE_LINK_INFORMATION_CLASS: Final = 72
_ERROR_FILE_EXISTS: Final = 80
_ERROR_ALREADY_EXISTS: Final = 183


@final
class IoStatusValue(ctypes.Union):
    _fields_ = (
        ("status", ctypes.c_long),
        ("pointer", ctypes.c_void_p),
    )


@final
class IoStatusBlock(ctypes.Structure):
    _fields_ = [
        ("value", IoStatusValue),
        ("information", ctypes.c_size_t),
    ]


@final
class FileLinkInformation(ctypes.Structure):
    _fields_ = [
        ("replace_if_exists", ctypes.c_ubyte),
        ("root_directory", ctypes.c_void_p),
        ("file_name_length", ctypes.c_uint32),
        ("file_name", ctypes.c_wchar * 1),
    ]


def link_file_by_handle(handle: int, target: Path) -> None:
    name = _native_path(target).encode("utf-16-le")
    name_offset = FileLinkInformation.file_name.offset
    minimum_size = max(ctypes.sizeof(FileLinkInformation), name_offset + len(name))
    buffer_size = (minimum_size + 3) & ~3
    buffer = ctypes.create_string_buffer(buffer_size)
    information = ctypes.cast(buffer, ctypes.POINTER(FileLinkInformation)).contents
    information.replace_if_exists = 0
    information.root_directory = None
    information.file_name_length = len(name)
    _ = ctypes.memmove(ctypes.addressof(buffer) + name_offset, name, len(name))
    io_status = IoStatusBlock()
    status = int(
        _ntdll().NtSetInformationFile(
            handle,
            ctypes.byref(io_status),
            buffer,
            buffer_size,
            _FILE_LINK_INFORMATION_CLASS,
        )
    )
    if status >= 0:
        return
    error = int(_ntdll().RtlNtStatusToDosError(status))
    if error in {_ERROR_FILE_EXISTS, _ERROR_ALREADY_EXISTS}:
        raise FileExistsError(error, "path already exists", str(target))
    raise WindowsNativeFileError(error, "atomic file link failed", str(target))


def _native_path(path: Path) -> str:
    value = str(path)
    if value.startswith("\\\\"):
        return f"\\??\\UNC\\{value[2:]}"
    return f"\\??\\{value}"


@cache
def _ntdll():
    ntdll = ctypes.WinDLL("ntdll")
    ntdll.NtSetInformationFile.argtypes = [
        ctypes.c_void_p,
        ctypes.POINTER(IoStatusBlock),
        ctypes.c_void_p,
        ctypes.c_uint32,
        ctypes.c_int,
    ]
    ntdll.NtSetInformationFile.restype = ctypes.c_long
    ntdll.RtlNtStatusToDosError.argtypes = [ctypes.c_long]
    ntdll.RtlNtStatusToDosError.restype = ctypes.c_uint32
    return ntdll
