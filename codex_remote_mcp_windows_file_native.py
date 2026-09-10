# pyright: reportAny=false
from __future__ import annotations

import ctypes
import os
from collections.abc import Generator
from contextlib import contextmanager
from functools import cache
from pathlib import Path
from typing import BinaryIO, Final, final

GENERIC_READ: Final = 0x80000000
GENERIC_WRITE: Final = 0x40000000
DELETE_ACCESS: Final = 0x00010000
FILE_READ_ATTRIBUTES: Final = 0x00000080
FILE_ATTRIBUTE_DIRECTORY: Final = 0x00000010
FILE_ATTRIBUTE_REPARSE_POINT: Final = 0x00000400
CREATE_NEW: Final = 1
OPEN_EXISTING: Final = 3
FILE_DISPOSITION_INFO_CLASS: Final = 4

_FILE_SHARE_READ: Final = 0x00000001
_FILE_SHARE_WRITE: Final = 0x00000002
_FILE_SHARE_DELETE: Final = 0x00000004
_FILE_RENAME_INFO_EX_CLASS: Final = 22
_FILE_RENAME_FLAG_REPLACE_IF_EXISTS: Final = 0x00000001
_FILE_RENAME_FLAG_POSIX_SEMANTICS: Final = 0x00000002
_FILE_FLAG_BACKUP_SEMANTICS: Final = 0x02000000
_FILE_FLAG_OPEN_REPARSE_POINT: Final = 0x00200000
_ERROR_FILE_NOT_FOUND: Final = 2
_ERROR_PATH_NOT_FOUND: Final = 3
_ERROR_FILE_EXISTS: Final = 80
_ERROR_ALREADY_EXISTS: Final = 183
_INVALID_HANDLE_VALUE: Final = ctypes.c_void_p(-1).value


@final
class ByHandleFileInformation(ctypes.Structure):
    _fields_ = [
        ("file_attributes", ctypes.c_uint32),
        ("creation_time_low", ctypes.c_uint32),
        ("creation_time_high", ctypes.c_uint32),
        ("last_access_time_low", ctypes.c_uint32),
        ("last_access_time_high", ctypes.c_uint32),
        ("last_write_time_low", ctypes.c_uint32),
        ("last_write_time_high", ctypes.c_uint32),
        ("volume_serial_number", ctypes.c_uint32),
        ("file_size_high", ctypes.c_uint32),
        ("file_size_low", ctypes.c_uint32),
        ("number_of_links", ctypes.c_uint32),
        ("file_index_high", ctypes.c_uint32),
        ("file_index_low", ctypes.c_uint32),
    ]


@final
class FileDispositionInformation(ctypes.Structure):
    _fields_ = [("delete_file", ctypes.c_int)]


@final
class FileRenameInformationEx(ctypes.Structure):
    _fields_ = [
        ("flags", ctypes.c_uint32),
        ("root_directory", ctypes.c_void_p),
        ("file_name_length", ctypes.c_uint32),
        ("file_name", ctypes.c_wchar * 1),
    ]


class WindowsNativeFileError(OSError):
    """Raised when a Win32 handle operation fails."""


def open_handle(
    path: Path,
    access: int,
    disposition: int,
    *,
    directory: bool = False,
    share_write: bool = True,
    share_delete: bool = False,
) -> int:
    flags = _FILE_FLAG_OPEN_REPARSE_POINT | (
        _FILE_FLAG_BACKUP_SEMANTICS if directory else 0
    )
    share_mode = _FILE_SHARE_READ
    if share_write:
        share_mode |= _FILE_SHARE_WRITE
    if share_delete:
        share_mode |= _FILE_SHARE_DELETE
    raw = _kernel32().CreateFileW(
        str(path),
        access,
        share_mode,
        None,
        disposition,
        flags,
        None,
    )
    if raw in (None, 0, _INVALID_HANDLE_VALUE):
        error = ctypes.get_last_error()
        if error in {_ERROR_FILE_NOT_FOUND, _ERROR_PATH_NOT_FOUND}:
            raise FileNotFoundError(error, "path was not found", str(path))
        if error in {_ERROR_FILE_EXISTS, _ERROR_ALREADY_EXISTS}:
            raise FileExistsError(error, "path already exists", str(path))
        raise windows_error(f"CreateFileW failed for {path}")
    return int(raw)


def file_information(handle: int) -> ByHandleFileInformation:
    information = ByHandleFileInformation()
    if not _kernel32().GetFileInformationByHandle(handle, ctypes.byref(information)):
        raise windows_error("GetFileInformationByHandle failed")
    return information


def file_identity(information: ByHandleFileInformation) -> tuple[int, int, int]:
    return (
        information.volume_serial_number,
        information.file_index_high,
        information.file_index_low,
    )


def confined_information(
    handle: int,
    root_final: str,
) -> ByHandleFileInformation:
    information = file_information(handle)
    if information.file_attributes & FILE_ATTRIBUTE_REPARSE_POINT:
        raise WindowsNativeFileError("reparse points are not allowed")
    try:
        common = os.path.commonpath((root_final, final_path(handle)))
    except ValueError as exc:
        raise WindowsNativeFileError("path is outside the project root") from exc
    if os.path.normcase(common) != os.path.normcase(root_final):
        raise WindowsNativeFileError("path is outside the project root")
    return information


def final_path(handle: int) -> str:
    size = int(_kernel32().GetFinalPathNameByHandleW(handle, None, 0, 0))
    if size <= 0:
        raise windows_error("GetFinalPathNameByHandleW failed")
    buffer = ctypes.create_unicode_buffer(size + 1)
    if not _kernel32().GetFinalPathNameByHandleW(handle, buffer, len(buffer), 0):
        raise windows_error("GetFinalPathNameByHandleW failed")
    value = buffer.value
    if value.startswith("\\\\?\\UNC\\"):
        value = "\\\\" + value[8:]
    elif value.startswith("\\\\?\\"):
        value = value[4:]
    return os.path.normpath(value)


@contextmanager
def binary_stream(handle: int, *, writable: bool) -> Generator[BinaryIO]:
    import msvcrt

    flags = os.O_BINARY | (os.O_RDWR if writable else os.O_RDONLY)
    descriptor = msvcrt.open_osfhandle(handle, flags)
    mode = "r+b" if writable else "rb"
    with os.fdopen(descriptor, mode) as stream:
        yield stream


def os_handle(stream: BinaryIO) -> int:
    import msvcrt

    return int(msvcrt.get_osfhandle(stream.fileno()))


def set_delete(handle: int) -> None:
    disposition = FileDispositionInformation(1)
    if not _kernel32().SetFileInformationByHandle(
        handle,
        FILE_DISPOSITION_INFO_CLASS,
        ctypes.byref(disposition),
        ctypes.sizeof(disposition),
    ):
        raise windows_error("SetFileInformationByHandle failed")


def rename_file_by_handle(
    handle: int,
    target: Path,
    *,
    replace_existing: bool,
) -> None:
    name = str(target).encode("utf-16-le")
    name_offset = FileRenameInformationEx.file_name.offset
    buffer = ctypes.create_string_buffer(name_offset + len(name) + 2)
    information = ctypes.cast(
        buffer,
        ctypes.POINTER(FileRenameInformationEx),
    ).contents
    information.flags = (
        _FILE_RENAME_FLAG_REPLACE_IF_EXISTS | _FILE_RENAME_FLAG_POSIX_SEMANTICS
        if replace_existing
        else 0
    )
    information.root_directory = None
    information.file_name_length = len(name)
    _ = ctypes.memmove(ctypes.addressof(buffer) + name_offset, name, len(name))
    if not _kernel32().SetFileInformationByHandle(
        handle,
        _FILE_RENAME_INFO_EX_CLASS,
        buffer,
        len(buffer),
    ):
        error = ctypes.get_last_error()
        if error in {_ERROR_FILE_EXISTS, _ERROR_ALREADY_EXISTS}:
            raise FileExistsError(error, "path already exists", str(target))
        raise WindowsNativeFileError(error, "atomic file rename failed", str(target))


def replace_file_by_handle(handle: int, target: Path) -> None:
    rename_file_by_handle(handle, target, replace_existing=True)


def close_handle(handle: int) -> None:
    if not _kernel32().CloseHandle(handle):
        raise windows_error("CloseHandle failed")


def windows_error(message: str) -> WindowsNativeFileError:
    return WindowsNativeFileError(ctypes.get_last_error(), message)


@cache
def _kernel32():
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.CreateFileW.argtypes = [
        ctypes.c_wchar_p,
        ctypes.c_uint32,
        ctypes.c_uint32,
        ctypes.c_void_p,
        ctypes.c_uint32,
        ctypes.c_uint32,
        ctypes.c_void_p,
    ]
    kernel32.CreateFileW.restype = ctypes.c_void_p
    kernel32.GetFileInformationByHandle.argtypes = [
        ctypes.c_void_p,
        ctypes.POINTER(ByHandleFileInformation),
    ]
    kernel32.GetFileInformationByHandle.restype = ctypes.c_int
    kernel32.GetFinalPathNameByHandleW.argtypes = [
        ctypes.c_void_p,
        ctypes.c_wchar_p,
        ctypes.c_uint32,
        ctypes.c_uint32,
    ]
    kernel32.GetFinalPathNameByHandleW.restype = ctypes.c_uint32
    kernel32.SetFileInformationByHandle.argtypes = [
        ctypes.c_void_p,
        ctypes.c_int,
        ctypes.c_void_p,
        ctypes.c_uint32,
    ]
    kernel32.SetFileInformationByHandle.restype = ctypes.c_int
    kernel32.CloseHandle.argtypes = [ctypes.c_void_p]
    kernel32.CloseHandle.restype = ctypes.c_int
    return kernel32
