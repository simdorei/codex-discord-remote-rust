from __future__ import annotations

# pyright: reportAny=false

import ctypes
import os
from typing import Final, final


_JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: Final = 9
_JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: Final = 0x00002000
_PROCESS_TERMINATE: Final = 0x0001
_PROCESS_SET_QUOTA: Final = 0x0100
_PROCESS_SUSPEND_RESUME: Final = 0x0800
_WAIT_OBJECT_0: Final = 0
_WAIT_TIMEOUT: Final = 0x00000102
_WAIT_FAILED: Final = 0xFFFFFFFF
_JOB_TERMINATION_TIMEOUT_MS: Final = 5000
WINDOWS_CREATE_SUSPENDED: Final = 0x00000004


@final
class _IoCounters(ctypes.Structure):
    _fields_ = [
        ("ReadOperationCount", ctypes.c_uint64),
        ("WriteOperationCount", ctypes.c_uint64),
        ("OtherOperationCount", ctypes.c_uint64),
        ("ReadTransferCount", ctypes.c_uint64),
        ("WriteTransferCount", ctypes.c_uint64),
        ("OtherTransferCount", ctypes.c_uint64),
    ]


@final
class _JobObjectBasicLimitInformation(ctypes.Structure):
    _fields_ = [
        ("PerProcessUserTimeLimit", ctypes.c_int64),
        ("PerJobUserTimeLimit", ctypes.c_int64),
        ("LimitFlags", ctypes.c_uint32),
        ("MinimumWorkingSetSize", ctypes.c_size_t),
        ("MaximumWorkingSetSize", ctypes.c_size_t),
        ("ActiveProcessLimit", ctypes.c_uint32),
        ("Affinity", ctypes.c_size_t),
        ("PriorityClass", ctypes.c_uint32),
        ("SchedulingClass", ctypes.c_uint32),
    ]


@final
class _JobObjectExtendedLimitInformation(ctypes.Structure):
    _fields_ = [
        ("BasicLimitInformation", _JobObjectBasicLimitInformation),
        ("IoInfo", _IoCounters),
        ("ProcessMemoryLimit", ctypes.c_size_t),
        ("JobMemoryLimit", ctypes.c_size_t),
        ("PeakProcessMemoryUsed", ctypes.c_size_t),
        ("PeakJobMemoryUsed", ctypes.c_size_t),
    ]


@final
class WindowsKillOnCloseJob:
    """Own one Windows process tree until termination is confirmed."""

    __slots__ = ("_handle",)

    def __init__(self, handle: int) -> None:
        self._handle: int | None = handle

    @property
    def closed(self) -> bool:
        return self._handle is None

    def terminate_and_close(self, *, timeout_seconds: float = 5.0) -> None:
        handle = self._handle
        if handle is None:
            return
        kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel32.TerminateJobObject.argtypes = [ctypes.c_void_p, ctypes.c_uint32]
        kernel32.TerminateJobObject.restype = ctypes.c_int
        kernel32.WaitForSingleObject.argtypes = [ctypes.c_void_p, ctypes.c_uint32]
        kernel32.WaitForSingleObject.restype = ctypes.c_uint32
        kernel32.CloseHandle.argtypes = [ctypes.c_void_p]
        kernel32.CloseHandle.restype = ctypes.c_int
        primary: BaseException | None = None
        try:
            if not kernel32.TerminateJobObject(handle, 1):
                raise _windows_error("TerminateJobObject failed")
            wait_result = int(
                kernel32.WaitForSingleObject(
                    handle,
                    min(
                        _JOB_TERMINATION_TIMEOUT_MS,
                        max(0, int(timeout_seconds * 1_000)),
                    ),
                )
            )
            if wait_result == _WAIT_TIMEOUT:
                raise TimeoutError(
                    "Timed out waiting for the owned Windows Job Object to empty."
                )
            if wait_result == _WAIT_FAILED:
                raise _windows_error("WaitForSingleObject failed for owned job")
            if wait_result != _WAIT_OBJECT_0:
                raise OSError(f"Unexpected owned job wait result: {wait_result}")
        except BaseException as exc:
            primary = exc
        finally:
            close_error = (
                None
                if kernel32.CloseHandle(handle)
                else _windows_error("CloseHandle failed for owned job")
            )
            self._handle = None
        if primary is not None:
            if close_error is not None:
                primary.add_note(
                    f"secondary handle-close failure: {type(close_error).__name__}"
                )
            raise primary
        if close_error is not None:
            raise close_error


def create_kill_on_close_job_for_suspended_process(
    process_id: int,
) -> WindowsKillOnCloseJob:
    if os.name != "nt":
        raise OSError("Windows Job Objects are only available on Windows.")
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.CreateJobObjectW.argtypes = [ctypes.c_void_p, ctypes.c_wchar_p]
    kernel32.CreateJobObjectW.restype = ctypes.c_void_p
    kernel32.SetInformationJobObject.argtypes = [
        ctypes.c_void_p,
        ctypes.c_int,
        ctypes.c_void_p,
        ctypes.c_uint32,
    ]
    kernel32.SetInformationJobObject.restype = ctypes.c_int
    kernel32.OpenProcess.argtypes = [ctypes.c_uint32, ctypes.c_int, ctypes.c_uint32]
    kernel32.OpenProcess.restype = ctypes.c_void_p
    kernel32.AssignProcessToJobObject.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
    kernel32.AssignProcessToJobObject.restype = ctypes.c_int
    kernel32.CloseHandle.argtypes = [ctypes.c_void_p]
    kernel32.CloseHandle.restype = ctypes.c_int

    raw_job = kernel32.CreateJobObjectW(None, None)
    if not raw_job:
        raise _windows_error("CreateJobObjectW failed")
    job_handle = int(raw_job)
    assigned = False
    try:
        limits = _JobObjectExtendedLimitInformation()
        limits.BasicLimitInformation.LimitFlags = _JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        if not kernel32.SetInformationJobObject(
            job_handle,
            _JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
            ctypes.byref(limits),
            ctypes.sizeof(limits),
        ):
            raise _windows_error("SetInformationJobObject failed")
        process_handle = kernel32.OpenProcess(
            _PROCESS_TERMINATE | _PROCESS_SET_QUOTA | _PROCESS_SUSPEND_RESUME,
            False,
            process_id,
        )
        if not process_handle:
            raise _windows_error(
                f"OpenProcess failed for suspended owned PID {process_id}"
            )
        try:
            if not kernel32.AssignProcessToJobObject(job_handle, process_handle):
                raise _windows_error(
                    f"AssignProcessToJobObject failed for owned PID {process_id}"
                )
            assigned = True
            _resume_suspended_process(process_handle, process_id)
        finally:
            _ = kernel32.CloseHandle(process_handle)
        return WindowsKillOnCloseJob(job_handle)
    except (OSError, TimeoutError):
        if assigned:
            kernel32.TerminateJobObject.argtypes = [ctypes.c_void_p, ctypes.c_uint32]
            kernel32.TerminateJobObject.restype = ctypes.c_int
            _ = kernel32.TerminateJobObject(job_handle, 1)
        _ = kernel32.CloseHandle(job_handle)
        raise


def _resume_suspended_process(process_handle: int, process_id: int) -> None:
    ntdll = ctypes.WinDLL("ntdll", use_last_error=True)
    ntdll.NtResumeProcess.argtypes = [ctypes.c_void_p]
    ntdll.NtResumeProcess.restype = ctypes.c_long
    status = int(ntdll.NtResumeProcess(process_handle))
    if status != 0:
        unsigned_status = status & 0xFFFFFFFF
        message = f"NtResumeProcess failed for suspended PID {process_id}; "
        message += f"NTSTATUS=0x{unsigned_status:08x}"
        raise OSError(message)


def _windows_error(message: str) -> OSError:
    return OSError(ctypes.get_last_error(), message)
