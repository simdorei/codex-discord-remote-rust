from __future__ import annotations

from dataclasses import dataclass
import subprocess
from typing import Protocol, final, runtime_checkable

from codex_remote_mcp_computer_errors import ComputerControlError
from codex_remote_mcp_windows_windows import ResolvedWindow
from codex_windows_job import WindowsKillOnCloseJob


class OwnedProcess(Protocol):
    @property
    def pid(self) -> int: ...

    def poll(self) -> int | None: ...

    def wait(self, timeout: float | None = None) -> int: ...

    def terminate(self) -> None: ...

    def kill(self) -> None: ...


@runtime_checkable
class OwnedProcessTree(Protocol):
    def terminate_tree_and_close(self, *, timeout_seconds: float = 5.0) -> None: ...


@final
class JobOwnedProcess:
    """Retain a Popen handle together with its kill-on-close process tree."""

    __slots__ = ("_job", "_process")

    def __init__(
        self,
        process: subprocess.Popen[bytes],
        job: WindowsKillOnCloseJob,
    ) -> None:
        self._process = process
        self._job = job

    @property
    def pid(self) -> int:
        return self._process.pid

    def poll(self) -> int | None:
        return self._process.poll()

    def wait(self, timeout: float | None = None) -> int:
        return self._process.wait(timeout=timeout)

    def terminate(self) -> None:
        self._process.terminate()

    def kill(self) -> None:
        self._process.kill()

    def terminate_tree_and_close(self, *, timeout_seconds: float = 5.0) -> None:
        self._job.terminate_and_close(timeout_seconds=timeout_seconds)


@dataclass(frozen=True, slots=True)
class LaunchedApplication:
    window: ResolvedWindow
    process: OwnedProcess
    temporary_profile: str | None


@dataclass(frozen=True, slots=True)
class FailedLaunchCleanup:
    app: str
    process: OwnedProcess | None
    temporary_profile: str | None


@dataclass(slots=True, init=False)
class ApplicationLaunchCleanupError(ComputerControlError):
    cleanup: FailedLaunchCleanup

    def __init__(self, cleanup: FailedLaunchCleanup) -> None:
        ComputerControlError.__init__(
            self,
            "Failed application cleanup must be retried.",
        )
        self.cleanup = cleanup
