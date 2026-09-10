"""Cohesive remote bridge connection state machine. (# noqa: SIZE_OK)"""

from __future__ import annotations

import queue
import threading
from collections.abc import Callable
from datetime import UTC, datetime, timedelta
from pathlib import Path
from time import monotonic
from typing import assert_never, final
from uuid import uuid4

from pydantic import ValidationError
from websockets.exceptions import WebSocketException

from codex_remote_mcp_bridge_closure import bridge_close_details
from codex_remote_mcp_bridge_config import ProjectTicket, RemoteMcpBridgeConfig
from codex_remote_mcp_bridge_connection import (
    BridgeConnector,
    BridgeSocket,
    RemoteMcpBridgeError,
    SerializedBridgeConnection,
    close_socket_before_deadline,
    connect_bridge,
    invalidate_sessions,
    receive_message,
)
from codex_remote_mcp_bridge_workers import BridgeCommandWorkers
from codex_remote_mcp_process_lock import acquire_remote_mcp_process_lock
from codex_remote_mcp_restart_models import RestartProject
from codex_remote_mcp_bridge_projects import (
    prune_expired_projects,
    replace_thread_project,
)
from codex_remote_mcp_dispatch import LocalProjectDispatcher
from simdorei_mcp_common.messages import (
    BridgeHello,
    DeviceSessionCommand,
    DeviceId,
    GatewayHello,
    ListFilesCommand,
    ProjectAck,
    ProjectInfoCommand,
    ProjectOperationCommand,
    ProjectSessionCommand,
    ProjectUpsert,
    ReadFileCommand,
    WriteFileCommand,
)

LogFunc = Callable[[str], None]


NowFactory = Callable[[], datetime]


@final
class RemoteMcpBridge:  # MUTABLE_OK: owns synchronized connection state.
    """Maintains one outbound bridge and dispatches bounded file operations."""

    def __init__(
        self,
        config: RemoteMcpBridgeConfig,
        *,
        connector: BridgeConnector | None = None,
        dispatcher: LocalProjectDispatcher | None = None,
        log: LogFunc,
        now: NowFactory = lambda: datetime.now(UTC),
        close_timeout_seconds: float = 12.0,
    ) -> None:
        if close_timeout_seconds <= 0:
            raise ValueError("close_timeout_seconds must be positive")
        self._config = config
        self._connector = connector or connect_bridge
        self._log = log
        self._now = now
        self._close_timeout_seconds = close_timeout_seconds
        self._dispatcher = dispatcher or LocalProjectDispatcher()
        self._workers = BridgeCommandWorkers(self._dispatcher, log=log)
        self._condition = threading.Condition()
        self._registration_lock = threading.Lock()
        self._outbound: queue.Queue[ProjectUpsert] = queue.Queue()
        self._active: dict[str, ProjectUpsert] = {}
        self._active_roots: dict[str, Path] = {}
        self._pending_projects: dict[str, ProjectUpsert] = {}
        self._acked: dict[str, str] = {}
        self._last_error = ""
        self._terminal_error: RemoteMcpBridgeError | None = None
        self._connected = False
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None
        self._socket_lock = threading.Lock()
        self._active_socket: BridgeSocket | None = None

    def connect_device(self) -> None:
        self._reset_terminal_for_retry()
        connection_error: RemoteMcpBridgeError | None = None
        with self._condition:
            self._ensure_started()
            connected = self._condition.wait_for(
                lambda: (
                    self._connected
                    or self._terminal_error is not None
                    or self._stop.is_set()
                ),
                timeout=self._config.binding_ack_timeout_seconds,
            )
            if self._terminal_error is not None:
                connection_error = self._terminal_error
            elif not connected or not self._connected:
                detail = (
                    f" Last connection error: {self._last_error}"
                    if self._last_error
                    else ""
                )
                connection_error = RemoteMcpBridgeError(
                    "The remote MCP device did not connect in time." + detail
                )
        if connection_error is not None:
            raise connection_error

    def register_project(
        self,
        thread_id: str,
        project_scope: str,
        root: Path,
    ) -> ProjectTicket:
        if not thread_id:
            raise RemoteMcpBridgeError(
                "A Codex thread is required for project registration."
            )
        if not project_scope:
            raise RemoteMcpBridgeError(
                "A project scope is required for project registration."
            )
        with self._registration_lock:
            return self._register_project(thread_id, project_scope, root)

    def _register_project(
        self,
        thread_id: str,
        project_scope: str,
        root: Path,
    ) -> ProjectTicket:
        self._reset_terminal_for_retry()
        expires_at = self._now() + timedelta(seconds=self._config.binding_ttl_seconds)
        project = ProjectUpsert(
            project_scope=project_scope,
            binding_id=uuid4().hex,
            thread_id=thread_id,
            project_name=root.resolve().name or "local-project",
            expires_at=expires_at,
        )
        return self._register_project_upsert(project, root)

    def restore_project(self, restart_project: RestartProject) -> ProjectTicket:
        project = restart_project.project
        if project.expires_at <= self._now():
            raise RemoteMcpBridgeError(
                "The restart project binding expired before it could be restored."
            )
        with self._registration_lock:
            return self._register_project_upsert(project, restart_project.root)

    def prepare_restart_projects(
        self,
        *,
        timeout_seconds: float = 10.0,
    ) -> tuple[RestartProject, ...]:
        if not self._workers.prepare_restart(timeout_seconds):
            raise RemoteMcpBridgeError(
                "The local project bridge still has running commands."
            )
        with self._condition:
            self._prune_expired_locked()
            return tuple(
                RestartProject(project=project, root=self._active_roots[scope])
                for scope, project in self._active.items()
            )

    def cancel_restart_preparation(self) -> None:
        self._workers.cancel_restart_preparation()

    def _register_project_upsert(
        self,
        project: ProjectUpsert,
        root: Path,
    ) -> ProjectTicket:
        registration_error: RemoteMcpBridgeError | None = None
        with self._condition:
            self._prune_expired_locked()
            self._pending_projects[project.project_scope] = project
            if self._connected:
                self._outbound.put(project)
            self._ensure_started()
            acknowledged = self._condition.wait_for(
                lambda: (
                    self._acked.get(project.project_scope) == project.binding_id
                    or self._terminal_error is not None
                    or self._stop.is_set()
                ),
                timeout=self._config.binding_ack_timeout_seconds,
            )
            if self._terminal_error is not None:
                registration_error = self._terminal_error
            elif (
                not acknowledged
                or self._acked.get(project.project_scope) != project.binding_id
            ):
                detail = (
                    f" Last connection error: {self._last_error}"
                    if self._last_error
                    else ""
                )
                registration_error = RemoteMcpBridgeError(
                    "The local project bridge did not acknowledge the binding in time."
                    + detail
                )
            if registration_error is not None:
                self._rollback_registration_locked(project)
        if registration_error is not None:
            raise registration_error

        committed = False
        try:
            self._dispatcher.upsert(project.thread_id, root, project.expires_at)
            with self._condition:
                pending = self._pending_projects.get(project.project_scope)
                if pending != project:
                    raise RemoteMcpBridgeError(
                        "The pending local project registration changed unexpectedly."
                    )
                _ = self._pending_projects.pop(project.project_scope, None)
                stale_scopes = tuple(
                    scope
                    for scope, active in self._active.items()
                    if active.thread_id == project.thread_id
                    and scope != project.project_scope
                )
                replace_thread_project(self._active, self._acked, project)
                for scope in stale_scopes:
                    _ = self._active_roots.pop(scope, None)
                self._active_roots[project.project_scope] = root.resolve()
                self._acked[project.project_scope] = project.binding_id
                committed = True
                self._condition.notify_all()
        finally:
            if not committed:
                with self._condition:
                    self._rollback_registration_locked(project)
        return ProjectTicket(
            project_scope=project.project_scope,
            expires_at=project.expires_at,
        )

    def _reset_terminal_for_retry(self) -> None:
        with self._condition:
            terminal_error = self._terminal_error
            thread = self._thread
        if terminal_error is None:
            return
        if thread is not None and thread.is_alive():
            thread.join(timeout=1)
        if thread is not None and thread.is_alive():
            raise RemoteMcpBridgeError(
                "The previous remote MCP bridge failure is still shutting down."
            )
        with self._condition:
            self._thread = None
            self._terminal_error = None
            self._last_error = ""
            self._stop.clear()

    def close(self) -> None:
        deadline_monotonic = monotonic() + self._close_timeout_seconds
        self._stop.set()
        self._workers.close()
        self._dispatcher.retire_computer_sessions()
        with self._condition:
            self._connected = False
            self._acked.clear()
            self._condition.notify_all()
        with self._socket_lock:
            socket = self._active_socket
        socket_close_completed = True
        if socket is not None:
            attempt = close_socket_before_deadline(
                socket,
                deadline_monotonic=deadline_monotonic,
            )
            socket_close_completed = attempt.completed
            if attempt.error is not None:
                self._log(
                    "remote_mcp_bridge_close_failed "
                    + f"error={type(attempt.error).__name__}"
                )
        thread = self._thread
        if thread is not None and thread.is_alive():
            thread.join(timeout=max(0.0, deadline_monotonic - monotonic()))
        thread_stopped = thread is None or not thread.is_alive()
        cleanup_complete = invalidate_sessions(
            self._dispatcher,
            self._log,
            deadline_monotonic=deadline_monotonic,
        )
        if not socket_close_completed:
            raise RemoteMcpBridgeError(
                "The local project bridge socket did not close before the deadline."
            )
        if not thread_stopped:
            raise RemoteMcpBridgeError(
                "The local project bridge did not stop before the close deadline."
            )
        if not cleanup_complete:
            raise RemoteMcpBridgeError(
                "The local project bridge close deadline expired before every "
                + "session-owned app was cleaned up."
            )

    def _ensure_started(self) -> None:
        if self._thread is not None and self._thread.is_alive():
            return
        self._stop.clear()
        self._thread = threading.Thread(
            target=self._run,
            name="codex-remote-mcp-bridge",
            daemon=True,
        )
        self._thread.start()

    def _run(self) -> None:
        with acquire_remote_mcp_process_lock(self._config.device_id) as owns_bridge:
            if not owns_bridge:
                self._fail_terminal(
                    RemoteMcpBridgeError(
                        "Another process already owns the remote MCP connection "
                        + "for this device."
                    ),
                    "remote_mcp_bridge_owner_conflict",
                )
                return
            self._run_owned()

    def _run_owned(self) -> None:
        while not self._stop.is_set():
            try:
                with SerializedBridgeConnection(
                    self._connector(self._config)
                ) as socket:
                    with self._socket_lock:
                        self._active_socket = socket
                    try:
                        self._serve_connection(socket)
                    finally:
                        with self._socket_lock:
                            if self._active_socket is socket:
                                self._active_socket = None
                        _ = invalidate_sessions(
                            self._dispatcher,
                            self._log,
                            deadline_monotonic=(
                                monotonic() + self._close_timeout_seconds
                            ),
                        )
            except (
                OSError,
                ValidationError,
                WebSocketException,
                RemoteMcpBridgeError,
            ) as exc:
                close = bridge_close_details(exc)
                if close is not None and close.was_replaced:
                    self._fail_terminal(
                        RemoteMcpBridgeError(
                            "The remote MCP bridge was replaced by another process."
                        ),
                        "remote_mcp_bridge_displaced " + close.log_fields(),
                    )
                    return
                if close is not None and close.was_rejected:
                    self._fail_terminal(
                        RemoteMcpBridgeError(
                            "The remote MCP gateway rejected the bridge connection."
                        ),
                        "remote_mcp_bridge_rejected " + close.log_fields(),
                    )
                    return
                with self._condition:
                    self._last_error = str(exc)
                    self._connected = False
                    self._acked.clear()
                    self._condition.notify_all()
                close_fields = f" {close.log_fields()}" if close is not None else ""
                self._log(
                    "remote_mcp_bridge_disconnected "
                    + f"error={type(exc).__name__}{close_fields}"
                )
            _ = self._stop.wait(self._config.reconnect_delay_seconds)

    def _fail_terminal(
        self,
        error: RemoteMcpBridgeError,
        log_message: str,
    ) -> None:
        with self._condition:
            self._last_error = str(error)
            self._terminal_error = error
            self._connected = False
            self._acked.clear()
            self._stop.set()
            self._condition.notify_all()
        self._log(log_message)

    def _serve_connection(self, socket: BridgeSocket) -> None:
        generation = self._workers.begin_connection()
        socket.send(
            BridgeHello(
                protocol_version=10,
                device_id=DeviceId(self._config.device_id),
            ).model_dump_json()
        )
        while not self._stop.is_set():
            try:
                first = receive_message(socket, timeout=0.25)
            except TimeoutError:
                continue
            break
        else:
            return
        if not isinstance(first, GatewayHello):
            raise RemoteMcpBridgeError(
                "The gateway did not acknowledge the bridge hello."
            )
        with self._condition:
            self._connected = True
            self._last_error = ""
            self._acked.clear()
            self._prune_expired_locked()
            projects_by_scope = {
                **self._active,
                **self._pending_projects,
            }
            projects = tuple(projects_by_scope.values())
            self._condition.notify_all()
        for project in projects:
            socket.send(project.model_dump_json())
        self._log("remote_mcp_bridge_connected")
        while not self._stop.is_set():
            self._send_worker_results(socket, generation)
            self._renew_active_projects()
            self._send_queued_projects(socket)
            try:
                message = receive_message(socket, timeout=0.25)
            except TimeoutError:
                continue
            match message:
                case ProjectAck(project_scope=project_scope, binding_id=binding_id):
                    with self._condition:
                        pending = self._pending_projects.get(project_scope)
                        active = self._active.get(project_scope)
                        pending_ack = (
                            pending is not None
                            and pending.binding_id == binding_id
                            and pending.expires_at > self._now()
                        )
                        active_ack = (
                            active is not None
                            and active.binding_id == binding_id
                            and active.expires_at > self._now()
                        )
                        if pending_ack or active_ack:
                            self._acked[project_scope] = binding_id
                        self._condition.notify_all()
                        if not pending_ack:
                            continue
                        committed = self._condition.wait_for(
                            lambda: (
                                self._active.get(project_scope) is not active
                                or self._pending_projects.get(project_scope) is not pending
                                or self._stop.is_set()
                            ),
                            timeout=self._config.binding_ack_timeout_seconds,
                        )
                        active = self._active.get(project_scope)
                        if (
                            not committed
                            or active is None
                            or active.binding_id != binding_id
                        ):
                            raise RemoteMcpBridgeError(
                                "The local project binding did not commit after acknowledgement."
                            )
                case (
                    ProjectInfoCommand()
                    | ListFilesCommand()
                    | ReadFileCommand()
                    | WriteFileCommand()
                    | ProjectOperationCommand()
                    | ProjectSessionCommand()
                    | DeviceSessionCommand()
                ):
                    rejected = self._workers.submit(generation, message)
                    if rejected is not None:
                        socket.send(rejected.model_dump_json())
                case GatewayHello():
                    raise RemoteMcpBridgeError("The gateway sent a duplicate hello.")
                case _:
                    assert_never(message)

    def _send_worker_results(
        self,
        socket: BridgeSocket,
        generation: int,
    ) -> None:
        for result in self._workers.drain(generation):
            socket.send(result.model_dump_json())

    def _renew_active_projects(self) -> None:
        ttl = timedelta(seconds=self._config.binding_ttl_seconds)
        with self._condition:
            now = self._now()
            self._prune_expired_locked()
            renewal_deadline = now + ttl / 2
            pending_threads = {
                project.thread_id for project in self._pending_projects.values()
            }
            renewals = tuple(
                project.model_copy(update={"expires_at": now + ttl})
                for project in self._active.values()
                if project.thread_id not in pending_threads
                and project.expires_at <= renewal_deadline
            )
            for project in renewals:
                self._active[project.project_scope] = project
                self._outbound.put(project)
        for project in renewals:
            self._dispatcher.renew(project.thread_id, project.expires_at)

    def _send_queued_projects(self, socket: BridgeSocket) -> None:
        while True:
            try:
                project = self._outbound.get_nowait()
            except queue.Empty:
                return
            with self._condition:
                self._prune_expired_locked()
                current = self._pending_projects.get(
                    project.project_scope
                ) or self._active.get(project.project_scope)
            if current == project:
                socket.send(project.model_dump_json())

    def _prune_expired_locked(self) -> None:
        prune_expired_projects(self._active, self._acked, self._now())
        stale_roots = self._active_roots.keys() - self._active.keys()
        for scope in stale_roots:
            del self._active_roots[scope]
        expired_pending = tuple(
            scope
            for scope, project in self._pending_projects.items()
            if project.expires_at <= self._now()
        )
        for scope in expired_pending:
            _ = self._pending_projects.pop(scope, None)
            _ = self._acked.pop(scope, None)

    def _rollback_registration_locked(self, project: ProjectUpsert) -> None:
        if self._pending_projects.get(project.project_scope) == project:
            _ = self._pending_projects.pop(project.project_scope, None)
        if self._acked.get(project.project_scope) == project.binding_id:
            _ = self._acked.pop(project.project_scope, None)
        self._condition.notify_all()
        if self._connected:
            for active in self._active.values():
                self._outbound.put(active)
