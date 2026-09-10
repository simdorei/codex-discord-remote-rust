"""Cohesive synchronized project and session state. (# noqa: SIZE_OK)"""

from __future__ import annotations

import threading
from collections.abc import Callable
from dataclasses import dataclass, field
from datetime import datetime
from pathlib import Path
from time import monotonic
from typing import Protocol, final

from codex_remote_mcp_computer import ComputerController
from codex_remote_mcp_computer_contracts import ComputerAccessMode
from codex_remote_mcp_computer_errors import ComputerControlError
from codex_remote_mcp_files import ProjectFileAccess
from codex_remote_mcp_idempotency import IdempotentResultCache
from simdorei_mcp_common.leases import RenewableExpiry


@dataclass(frozen=True, slots=True)
class ActiveProject:
    access: ProjectFileAccess
    lease: RenewableExpiry = field(repr=False)
    computer_access_mode: ComputerAccessMode = ComputerAccessMode.PROJECT
    result_cache: IdempotentResultCache = field(
        default_factory=IdempotentResultCache,
        compare=False,
        repr=False,
    )
    execution_lock: threading.RLock = field(
        default_factory=threading.RLock,
        compare=False,
        repr=False,
    )

    @property
    def expires_at(self) -> datetime:
        return self.lease.value

    def renew(self, expires_at: datetime) -> None:
        self.lease.extend(expires_at)


@dataclass(frozen=True, slots=True)
class SessionActivation:
    connection_generation: int | None
    session_generation: int
    session_id: str
    committed: bool


class LockHandle(Protocol):
    def acquire(self, blocking: bool = True, timeout: float = -1) -> bool: ...

    def release(self) -> None: ...


@final
class ProjectDispatchState:  # MUTABLE_OK: synchronized project/session registry.
    def __init__(
        self,
        computer_factory: Callable[[], ComputerController],
        device_computer_factory: Callable[[], ComputerController] | None = None,
    ) -> None:
        self._lock = threading.Lock()
        self._lifecycle_lock = threading.Lock()
        self._projects: dict[str, ActiveProject] = {}
        self._computer_factory = computer_factory
        self._device_computer_factory = device_computer_factory or computer_factory
        self._computers: dict[str, ComputerController] = {}
        self._sessions: dict[str, SessionActivation] = {}
        self._connection_generation: int | None = None
        self._computer_stopped: set[str] = set()

    def begin_connection(self, generation: int) -> None:
        with self._lock:
            current = self._connection_generation
            if current is not None and generation <= current:
                raise ComputerControlError(
                    "The local bridge connection generation did not advance."
                )
            self._connection_generation = generation
            self._sessions.clear()

    def binding(self, thread_id: str) -> ActiveProject | None:
        with self._lock:
            return self._projects.get(thread_id)

    def upsert(
        self,
        thread_id: str,
        root: Path,
        expires_at: datetime,
        *,
        computer_access_mode: ComputerAccessMode = ComputerAccessMode.PROJECT,
    ) -> None:
        project = ActiveProject(
            access=ProjectFileAccess(root),
            lease=RenewableExpiry(expires_at),
            computer_access_mode=computer_access_mode,
        )
        with self._lifecycle_lock:
            previous_project = self.binding(thread_id)
            if previous_project is None:
                self._replace(thread_id, project)
            else:
                with previous_project.execution_lock:
                    self._replace(thread_id, project)

    def renew(self, thread_id: str, expires_at: datetime) -> None:
        with self._lock:
            project = self._projects.get(thread_id)
            if project is None:
                raise ComputerControlError("The project binding is no longer active.")
            project.renew(expires_at)

    def computer_for(
        self,
        thread_id: str,
        expected_project: ActiveProject,
        session_id: str | None,
        connection_generation: int | None,
    ) -> ComputerController | None:
        with self._lock:
            if thread_id in self._computer_stopped:
                return None
            self._require_locked(
                thread_id,
                expected_project,
                session_id,
                connection_generation,
            )
            current = self._computers.get(thread_id)
            if current is not None:
                return current
            factory = (
                self._device_computer_factory
                if expected_project.computer_access_mode is ComputerAccessMode.DEVICE
                else self._computer_factory
            )
            created = factory()
            self._computers[thread_id] = created
            return created

    def activate(
        self,
        thread_id: str,
        expected_project: ActiveProject,
        session_id: str,
        session_generation: int,
        connection_generation: int | None,
    ) -> None:
        with self._lock:
            self._require_project_locked(thread_id, expected_project)
            self._require_connection_locked(connection_generation)
            current = self._sessions.get(thread_id)
            if current is not None:
                if current.connection_generation != connection_generation:
                    raise ComputerControlError(
                        "The project session belongs to a stale bridge connection."
                    )
                if session_generation < current.session_generation:
                    raise ComputerControlError(
                        "The project session command is older than the active session."
                    )
                if (
                    session_generation == current.session_generation
                    and session_id != current.session_id
                ):
                    raise ComputerControlError(
                        "The project session generation conflicts with another session."
                    )
            if (
                current is not None
                and session_generation == current.session_generation
                and current.committed
            ):
                return
            pending = SessionActivation(
                connection_generation=connection_generation,
                session_generation=session_generation,
                session_id=session_id,
                committed=False,
            )
            self._sessions[thread_id] = pending
            previous = self._computers.get(thread_id)
        if previous is not None:
            previous.stop()
        with self._lock:
            self._require_project_locked(thread_id, expected_project)
            self._require_connection_locked(connection_generation)
            if self._sessions.get(thread_id) != pending:
                raise ComputerControlError(
                    "The project session changed while activation was completing."
                )
            if self._computers.get(thread_id) is previous:
                _ = self._computers.pop(thread_id, None)
            self._sessions[thread_id] = SessionActivation(
                connection_generation=connection_generation,
                session_generation=session_generation,
                session_id=session_id,
                committed=True,
            )

    def require(
        self,
        thread_id: str,
        expected_project: ActiveProject,
        session_id: str | None,
        connection_generation: int | None,
    ) -> None:
        if session_id is None:
            raise ComputerControlError(
                "The project command has no active ChatGPT session."
            )
        with self._lock:
            self._require_locked(
                thread_id,
                expected_project,
                session_id,
                connection_generation,
            )

    def is_computer_stopped(self, thread_id: str) -> bool:
        with self._lock:
            return thread_id in self._computer_stopped

    def stop_computer(
        self,
        thread_id: str,
        *,
        deadline_monotonic: float | None = None,
    ) -> None:
        with self._lock:
            self._computer_stopped.add(thread_id)
            controller = self._computers.get(thread_id)
        if controller is not None:
            controller.stop(deadline_monotonic=deadline_monotonic)

    def stop_computer_if_bound(
        self,
        thread_id: str,
        expected_project: ActiveProject,
        *,
        deadline_monotonic: float | None = None,
    ) -> None:
        with self._lock:
            if self._projects.get(thread_id) is not expected_project:
                raise ComputerControlError(
                    "The project binding changed while the command was starting."
                )
            self._computer_stopped.add(thread_id)
            controller = self._computers.get(thread_id)
        if controller is not None:
            controller.stop(deadline_monotonic=deadline_monotonic)

    def stop_computer_if_current(
        self,
        thread_id: str,
        expected_project: ActiveProject,
        session_id: str | None,
        connection_generation: int | None,
        *,
        deadline_monotonic: float | None = None,
    ) -> None:
        if session_id is None:
            raise ComputerControlError(
                "The project command has no active ChatGPT session."
            )
        with self._lock:
            self._require_locked(
                thread_id,
                expected_project,
                session_id,
                connection_generation,
            )
            self._computer_stopped.add(thread_id)
            controller = self._computers.get(thread_id)
        if controller is not None:
            controller.stop(deadline_monotonic=deadline_monotonic)

    def retire_sessions(self) -> None:
        with self._lock:
            self._connection_generation = None
            self._sessions.clear()

    def invalidate_sessions(
        self,
        *,
        deadline_monotonic: float | None = None,
    ) -> None:
        self.retire_sessions()
        if not _acquire_before(self._lifecycle_lock, deadline_monotonic):
            raise TimeoutError("Timed out waiting to start local session cleanup.")
        acquired_project_locks: list[LockHandle] = []
        try:
            with self._lock:
                project_locks = tuple(
                    project.execution_lock
                    for _, project in sorted(self._projects.items())
                )
            for project_lock in project_locks:
                if not _acquire_before(project_lock, deadline_monotonic):
                    raise TimeoutError(
                        "Timed out waiting for an active local project command."
                    )
                acquired_project_locks.append(project_lock)
            with self._lock:
                controllers = tuple(self._computers.items())
            failures: list[ComputerControlError] = []
            for thread_id, controller in controllers:
                if deadline_monotonic is not None and monotonic() >= deadline_monotonic:
                    raise TimeoutError(
                        "Timed out before local session cleanup completed."
                    )
                try:
                    controller.stop(deadline_monotonic=deadline_monotonic)
                except ComputerControlError as exc:
                    failures.append(exc)
                    continue
                with self._lock:
                    if self._computers.get(thread_id) is controller:
                        del self._computers[thread_id]
            if failures:
                raise failures[0]
        finally:
            for project_lock in reversed(acquired_project_locks):
                project_lock.release()
            self._lifecycle_lock.release()

    def _replace(self, thread_id: str, project: ActiveProject) -> None:
        with self._lock:
            previous = self._computers.get(thread_id)
        if previous is not None:
            previous.stop()
        with self._lock:
            self._projects[thread_id] = project
            if self._computers.get(thread_id) is previous:
                _ = self._computers.pop(thread_id, None)
            _ = self._sessions.pop(thread_id, None)
            self._computer_stopped.discard(thread_id)

    def _require_locked(
        self,
        thread_id: str,
        expected_project: ActiveProject,
        session_id: str | None,
        connection_generation: int | None,
    ) -> None:
        self._require_project_locked(thread_id, expected_project)
        self._require_connection_locked(connection_generation)
        session = self._sessions.get(thread_id)
        if (
            session is None
            or not session.committed
            or session.session_id != session_id
            or session.connection_generation != connection_generation
        ):
            raise ComputerControlError(
                "The ChatGPT session is stale or was not acknowledged locally."
            )

    def _require_project_locked(
        self,
        thread_id: str,
        expected_project: ActiveProject,
    ) -> None:
        if self._projects.get(thread_id) is not expected_project:
            raise ComputerControlError(
                "The project binding changed while the command was starting."
            )

    def _require_connection_locked(
        self,
        connection_generation: int | None,
    ) -> None:
        if (
            connection_generation is not None
            and self._connection_generation != connection_generation
        ):
            raise ComputerControlError(
                "The command belongs to a stale local bridge connection."
            )


def _acquire_before(
    lock: LockHandle,
    deadline_monotonic: float | None,
) -> bool:
    if deadline_monotonic is None:
        return lock.acquire()
    remaining_seconds = deadline_monotonic - monotonic()
    if remaining_seconds <= 0:
        return lock.acquire(False)
    return lock.acquire(True, remaining_seconds)
