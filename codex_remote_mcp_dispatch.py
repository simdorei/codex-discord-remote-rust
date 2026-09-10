"""Cohesive local bridge command dispatcher. (# noqa: SIZE_OK)"""

# pyright: reportUnnecessaryComparison=false
from __future__ import annotations

from collections.abc import Callable
from datetime import UTC, datetime
from pathlib import Path
import threading
from time import monotonic
from typing import assert_never, final, overload

from codex_remote_mcp_computer import (
    ComputerController,
    is_computer_operation,
    new_computer_controller,
    new_device_computer_controller,
)
from codex_remote_mcp_computer_contracts import ComputerAccessMode
from codex_remote_mcp_command_policy import requires_execution_lock
from codex_remote_mcp_dispatch_commands import execute_bound_project_command
from codex_remote_mcp_dispatch_errors import project_error_code
from codex_remote_mcp_dispatch_state import ActiveProject, ProjectDispatchState
from codex_remote_mcp_files import ProjectFileError
from codex_remote_mcp_redaction import redact
from codex_remote_mcp_terminal_engine import (
    TerminalExecutionEngine,
)
from codex_remote_mcp_terminal_sessions import TerminalSessionRegistry
from codex_remote_mcp_terminal_windows import TerminalWindowManager
from simdorei_mcp_common.messages import (
    BridgeResult,
    DeviceSessionCommand,
    GatewayCommand,
    ListFilesCommand,
    ListFilesResult,
    OperationErrorResult,
    ProjectInfoCommand,
    ProjectInfoResult,
    ProjectOperationCommand,
    ProjectOperationResult,
    ProjectSessionCommand,
    ProjectSessionResult,
    ReadFileCommand,
    ReadFileResult,
    WriteFileCommand,
    WriteFileResult,
)
from simdorei_mcp_common.operation_outputs import ComputerStopOutput
from simdorei_mcp_common.operation_requests import ComputerStopRequest
from simdorei_mcp_common.terminal_protocol import TerminalExecRequest
from simdorei_mcp_common.terminal_window_interaction_protocol import (
    is_terminal_window_interaction_request,
)
from simdorei_mcp_common.terminal_window_protocol import is_terminal_window_request
from simdorei_mcp_common.request_deadlines import (
    RequestBudget,
    RequestDeadlineExpired,
)


@final
class LocalProjectDispatcher:  # MUTABLE_OK: owns synchronized project bindings.
    """Thread-safe local project registry and command executor."""

    def __init__(
        self,
        *,
        computer_factory: Callable[[], ComputerController] = new_computer_controller,
        device_computer_factory: Callable[
            [], ComputerController
        ] = new_device_computer_controller,
    ) -> None:
        self._state = ProjectDispatchState(computer_factory, device_computer_factory)
        self._terminal_lifecycle_lock = threading.RLock()
        self._terminals = TerminalSessionRegistry()

    def upsert(
        self,
        thread_id: str,
        root: Path,
        expires_at: datetime,
        *,
        computer_access_mode: ComputerAccessMode = ComputerAccessMode.PROJECT,
    ) -> None:
        with self._terminal_lifecycle_lock:
            self._state.upsert(
                thread_id,
                root,
                expires_at,
                computer_access_mode=computer_access_mode,
            )
            self._terminals.close_thread(thread_id)

    def renew(self, thread_id: str, expires_at: datetime) -> None:
        self._state.renew(thread_id, expires_at)

    @overload
    def execute(
        self,
        command: ProjectSessionCommand,
        *,
        connection_generation: int | None = None,
        cancel_event: threading.Event | None = None,
    ) -> ProjectSessionResult | OperationErrorResult: ...

    @overload
    def execute(
        self,
        command: DeviceSessionCommand,
        *,
        connection_generation: int | None = None,
        cancel_event: threading.Event | None = None,
    ) -> ProjectSessionResult | OperationErrorResult: ...

    @overload
    def execute(
        self,
        command: ProjectInfoCommand,
        *,
        connection_generation: int | None = None,
        cancel_event: threading.Event | None = None,
    ) -> ProjectInfoResult | OperationErrorResult: ...

    @overload
    def execute(
        self,
        command: ListFilesCommand,
        *,
        connection_generation: int | None = None,
        cancel_event: threading.Event | None = None,
    ) -> ListFilesResult | OperationErrorResult: ...

    @overload
    def execute(
        self,
        command: ReadFileCommand,
        *,
        connection_generation: int | None = None,
        cancel_event: threading.Event | None = None,
    ) -> ReadFileResult | OperationErrorResult: ...

    @overload
    def execute(
        self,
        command: WriteFileCommand,
        *,
        connection_generation: int | None = None,
        cancel_event: threading.Event | None = None,
    ) -> WriteFileResult | OperationErrorResult: ...

    @overload
    def execute(
        self,
        command: ProjectOperationCommand,
        *,
        connection_generation: int | None = None,
        cancel_event: threading.Event | None = None,
    ) -> ProjectOperationResult | OperationErrorResult: ...

    @overload
    def execute(
        self,
        command: GatewayCommand,
        *,
        connection_generation: int | None = None,
        cancel_event: threading.Event | None = None,
    ) -> BridgeResult: ...

    def execute(
        self,
        command: GatewayCommand,
        *,
        connection_generation: int | None = None,
        cancel_event: threading.Event | None = None,
    ) -> BridgeResult:
        match command:
            case DeviceSessionCommand():
                try:
                    self.upsert(
                        command.thread_id,
                        Path(command.working_directory),
                        command.expires_at,
                        computer_access_mode=ComputerAccessMode.DEVICE,
                    )
                except ProjectFileError as exc:
                    return OperationErrorResult(
                        request_id=command.request_id,
                        error_code=project_error_code(exc),
                        message=redact(str(exc)),
                    )
            case (
                ProjectSessionCommand()
                | ProjectInfoCommand()
                | ListFilesCommand()
                | ReadFileCommand()
                | WriteFileCommand()
                | ProjectOperationCommand()
            ):
                pass
            case unreachable:
                assert_never(unreachable)
        project = self._state.binding(command.thread_id)
        if project is None:
            return OperationErrorResult(
                request_id=command.request_id,
                error_code="binding_missing",
                message="The Codex thread is not bound on this device.",
            )
        budget = RequestBudget.from_deadline(
            command.deadline_at,
            cancel_event=cancel_event,
        )
        if _request_expired(budget):
            return _request_expired_result(command)
        if isinstance(command, (WriteFileCommand, ProjectOperationCommand)):
            try:
                return project.result_cache.execute_once(
                    command,
                    lambda: self._execute_admitted(
                        command,
                        project,
                        budget,
                        connection_generation,
                    ),
                    budget=budget,
                )
            except RequestDeadlineExpired:
                return _request_expired_result(command)
        return self._execute_admitted(
            command,
            project,
            budget,
            connection_generation,
        )

    def _execute_admitted(
        self,
        command: GatewayCommand,
        project: ActiveProject,
        budget: RequestBudget,
        connection_generation: int | None,
    ) -> BridgeResult:
        if _request_expired(budget):
            return _request_expired_result(command)
        if isinstance(command, ProjectOperationCommand) and isinstance(
            command.operation,
            ComputerStopRequest,
        ):
            return self._execute_computer_stop(
                command,
                project,
                budget,
                connection_generation,
            )
        if requires_execution_lock(command):
            with project.execution_lock:
                if _request_expired(budget):
                    return _request_expired_result(command)
                return self._execute_locked(
                    command,
                    project,
                    budget,
                    connection_generation,
                )
        return self._execute_locked(
            command,
            project,
            budget,
            connection_generation,
        )

    def _execute_computer_stop(
        self,
        command: ProjectOperationCommand,
        project: ActiveProject,
        budget: RequestBudget,
        connection_generation: int | None,
    ) -> ProjectOperationResult | OperationErrorResult:
        try:
            if project.expires_at <= datetime.now(UTC):
                self._state.stop_computer_if_bound(
                    command.thread_id,
                    project,
                    deadline_monotonic=monotonic() + budget.remaining(),
                )
                return OperationErrorResult(
                    request_id=command.request_id,
                    error_code="binding_expired",
                    message="The local project binding expired. Run !pro again.",
                )
            self._state.stop_computer_if_current(
                command.thread_id,
                project,
                command.computer_session_id,
                connection_generation,
                deadline_monotonic=monotonic() + budget.remaining(),
            )
        except ProjectFileError as exc:
            return OperationErrorResult(
                request_id=command.request_id,
                error_code=project_error_code(exc),
                message=redact(str(exc)),
            )
        return ProjectOperationResult(
            request_id=command.request_id,
            output=ComputerStopOutput(
                message="Computer control stopped until this project is bound again."
            ),
        )

    def _execute_locked(
        self,
        command: GatewayCommand,
        project: ActiveProject,
        budget: RequestBudget,
        connection_generation: int | None,
    ) -> BridgeResult:
        if _request_expired(budget):
            return _request_expired_result(command)
        computer: ComputerController | None = None
        terminal: TerminalExecutionEngine | None = None
        terminal_windows: TerminalWindowManager | None = None
        if project.expires_at <= datetime.now(UTC):
            self._stop_computer(
                command.thread_id,
                deadline_monotonic=monotonic() + budget.remaining(),
            )
            return OperationErrorResult(
                request_id=command.request_id,
                error_code="binding_expired",
                message="The local project binding expired. Run !pro again.",
            )
        match command:
            case ProjectSessionCommand() | DeviceSessionCommand():
                generation = command.computer_session_id
                if generation is None:
                    return OperationErrorResult(
                        request_id=command.request_id,
                        error_code="computer_session_missing",
                        message="The project session command has no generation.",
                    )
                try:
                    self._activate_project_session(
                        command.thread_id,
                        project,
                        generation,
                        command.computer_session_generation,
                        connection_generation,
                    )
                except ProjectFileError as exc:
                    return OperationErrorResult(
                        request_id=command.request_id,
                        error_code=project_error_code(exc),
                        message=redact(str(exc)),
                    )
                return ProjectSessionResult(request_id=command.request_id)
            case (
                ProjectInfoCommand()
                | ListFilesCommand()
                | ReadFileCommand()
                | WriteFileCommand()
                | ProjectOperationCommand()
            ):
                pass
            case unreachable:
                assert_never(unreachable)
        try:
            self._require_project_session(
                command.thread_id,
                project,
                command.computer_session_id,
                connection_generation,
            )
        except ProjectFileError as exc:
            return OperationErrorResult(
                request_id=command.request_id,
                error_code=project_error_code(exc),
                message=redact(str(exc)),
            )
        if isinstance(command, ProjectOperationCommand) and is_computer_operation(
            command.operation
        ):
            if self._state.is_computer_stopped(command.thread_id):
                return OperationErrorResult(
                    request_id=command.request_id,
                    error_code="computer_control_stopped",
                    message="Computer control is stopped. Run !pro again to renew it.",
                )
            try:
                computer = self._computer_for(
                    command.thread_id,
                    project,
                    command.computer_session_id,
                    connection_generation,
                )
            except ProjectFileError as exc:
                return OperationErrorResult(
                    request_id=command.request_id,
                    error_code=project_error_code(exc),
                    message=redact(str(exc)),
                )
            if computer is None:
                return OperationErrorResult(
                    request_id=command.request_id,
                    error_code="computer_control_stopped",
                    message="Computer control is stopped. Run !pro again to renew it.",
                )
        try:
            if isinstance(command, ProjectOperationCommand) and isinstance(
                command.operation,
                TerminalExecRequest,
            ):
                terminal = self._terminal_for(
                    command.thread_id,
                    project,
                    command.computer_session_id,
                    connection_generation,
                )
            if isinstance(command, ProjectOperationCommand) and (
                is_terminal_window_request(command.operation)
                or is_terminal_window_interaction_request(command.operation)
            ):
                terminal_windows = self._terminal_windows_for(
                    command.thread_id,
                    project,
                    command.computer_session_id,
                    connection_generation,
                )
            if terminal_windows is not None:
                return execute_bound_project_command(
                    command,
                    project.access,
                    computer,
                    terminal_windows=terminal_windows,
                    budget=budget,
                )
            if terminal is None:
                return execute_bound_project_command(
                    command,
                    project.access,
                    computer,
                    budget=budget,
                )
            return execute_bound_project_command(
                command,
                project.access,
                computer,
                terminal=terminal,
                budget=budget,
            )
        except RequestDeadlineExpired:
            return _request_expired_result(command)
        except ProjectFileError as exc:
            return OperationErrorResult(
                request_id=command.request_id,
                error_code=project_error_code(exc),
                message=redact(str(exc)),
            )

    def _computer_for(
        self,
        thread_id: str,
        expected_project: ActiveProject,
        computer_session_id: str | None,
        connection_generation: int | None,
    ) -> ComputerController | None:
        return self._state.computer_for(
            thread_id,
            expected_project,
            computer_session_id,
            connection_generation,
        )

    def _activate_project_session(
        self,
        thread_id: str,
        expected_project: ActiveProject,
        computer_session_id: str,
        computer_session_generation: int,
        connection_generation: int | None,
    ) -> None:
        with self._terminal_lifecycle_lock:
            self._state.activate(
                thread_id,
                expected_project,
                computer_session_id,
                computer_session_generation,
                connection_generation,
            )
            _ = self._terminals.for_session(
                thread_id,
                expected_project.access.root,
                computer_session_id,
            )

    def _terminal_for(
        self,
        thread_id: str,
        expected_project: ActiveProject,
        session_id: str | None,
        connection_generation: int | None,
    ) -> TerminalExecutionEngine:
        with self._terminal_lifecycle_lock:
            self._state.require(
                thread_id,
                expected_project,
                session_id,
                connection_generation,
            )
            if session_id is None:
                raise ProjectFileError(
                    "terminal",
                    "terminal session identity is missing",
                )
            return self._terminals.for_session(
                thread_id,
                expected_project.access.root,
                session_id,
            )

    def _terminal_windows_for(
        self,
        thread_id: str,
        expected_project: ActiveProject,
        session_id: str | None,
        connection_generation: int | None,
    ) -> TerminalWindowManager:
        with self._terminal_lifecycle_lock:
            self._state.require(
                thread_id,
                expected_project,
                session_id,
                connection_generation,
            )
            if session_id is None:
                raise ProjectFileError(
                    "terminal",
                    "terminal session identity is missing",
                )
            return self._terminals.windows_for_session(
                thread_id,
                expected_project.access.root,
                session_id,
            )

    def _require_project_session(
        self,
        thread_id: str,
        expected_project: ActiveProject,
        computer_session_id: str | None,
        connection_generation: int | None,
    ) -> None:
        self._state.require(
            thread_id,
            expected_project,
            computer_session_id,
            connection_generation,
        )

    def begin_connection(self, generation: int) -> None:
        with self._terminal_lifecycle_lock:
            self._state.begin_connection(generation)
            self._terminals.close_all()

    def retire_computer_sessions(self) -> None:
        with self._terminal_lifecycle_lock:
            self._state.retire_sessions()
            self._terminals.close_all()

    def invalidate_computer_sessions(
        self,
        *,
        deadline_monotonic: float | None = None,
    ) -> None:
        with self._terminal_lifecycle_lock:
            self._state.retire_sessions()
            self._terminals.close_all()
            self._state.invalidate_sessions(deadline_monotonic=deadline_monotonic)

    def _stop_computer(
        self,
        thread_id: str,
        *,
        deadline_monotonic: float | None = None,
    ) -> None:
        with self._terminal_lifecycle_lock:
            self._terminals.close_thread(thread_id)
            self._state.stop_computer(
                thread_id,
                deadline_monotonic=deadline_monotonic,
            )


def _request_expired(budget: RequestBudget) -> bool:
    try:
        budget.ensure_active()
    except RequestDeadlineExpired:
        return True
    return False


def _request_expired_result(command: GatewayCommand) -> OperationErrorResult:
    return OperationErrorResult(
        request_id=command.request_id,
        error_code="request_expired",
        message="The local project request expired before execution.",
    )
