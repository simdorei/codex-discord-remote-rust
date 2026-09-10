from __future__ import annotations

import queue
import threading
from contextlib import AbstractContextManager
from dataclasses import replace
from datetime import UTC, datetime
from pathlib import Path
from types import TracebackType
from typing import Self, final

import pytest

from codex_remote_mcp_bridge import RemoteMcpBridge
from codex_remote_mcp_bridge_config import ProjectTicket, RemoteMcpBridgeConfig
from codex_remote_mcp_bridge_connection import RemoteMcpBridgeError
from codex_remote_mcp_dispatch import LocalProjectDispatcher
from simdorei_mcp_common.messages import (
    BridgeHello,
    GatewayHello,
    ProjectAck,
    ProjectInfoCommand,
    ProjectInfoResult,
    ProjectUpsert,
    RequestId,
)
from tests.remote_mcp_dispatch_support import (
    TEST_PROJECT_SESSION_ID,
    activate_test_session,
)


class ManualAckSocket:
    def __init__(self) -> None:
        self.inbound: queue.Queue[str] = queue.Queue()
        self.sent: list[str] = []

    def __enter__(self) -> Self:
        return self

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        _ = exc_type, exc, traceback

    def send(self, message: str) -> None:
        self.sent.append(message)
        if '"type":"hello"' in message:
            _ = BridgeHello.model_validate_json(message)
            self.inbound.put(GatewayHello().model_dump_json())

    def recv(self, timeout: float | None = None) -> str:
        try:
            return self.inbound.get(timeout=timeout)
        except queue.Empty as exc:
            raise TimeoutError from exc

    def close(self) -> None:
        return None


@final
class ManualAckConnector:
    def __init__(self, socket: ManualAckSocket) -> None:
        self.socket: ManualAckSocket = socket

    def __call__(
        self,
        config: RemoteMcpBridgeConfig,
    ) -> AbstractContextManager[ManualAckSocket]:
        _ = config
        return self.socket


def test_connect_device_starts_bridge_without_registering_a_project() -> None:
    socket = ManualAckSocket()
    bridge = RemoteMcpBridge(
        _config(),
        connector=ManualAckConnector(socket),
        log=lambda _: None,
    )

    bridge.connect_device()
    bridge.close()

    assert any('"type":"hello"' in message for message in socket.sent)
    assert not any('"type":"project_upsert"' in message for message in socket.sent)


def test_delayed_ack_cannot_confirm_a_refreshed_binding(tmp_path: Path) -> None:
    socket = ManualAckSocket()
    bridge = RemoteMcpBridge(
        _config(),
        connector=ManualAckConnector(socket),
        log=lambda _: None,
    )
    tickets: list[ProjectTicket] = []

    def register() -> None:
        tickets.append(
            bridge.register_project(
                "thread-1",
                "codex-pro-project-1",
                tmp_path,
            )
        )

    first_worker = threading.Thread(target=register)
    first_worker.start()
    first = _wait_for_project(socket.sent)
    _ack(socket, first)
    first_worker.join(timeout=2)
    assert not first_worker.is_alive()

    second_worker = threading.Thread(target=register)
    second_worker.start()
    second = _wait_for_project(socket.sent, excluding=first.binding_id)
    _ack(socket, first)
    second_worker.join(timeout=0.05)
    assert second_worker.is_alive()

    _ack(socket, second)
    second_worker.join(timeout=2)
    bridge.close()

    assert not second_worker.is_alive()
    assert len(tickets) == 2


def test_failed_registration_preserves_the_previous_local_binding(
    tmp_path: Path,
) -> None:
    first_root = tmp_path / "first"
    second_root = tmp_path / "second"
    first_root.mkdir()
    second_root.mkdir()
    socket = ManualAckSocket()
    dispatcher = LocalProjectDispatcher()
    bridge = RemoteMcpBridge(
        replace(_config(), binding_ack_timeout_seconds=0.05),
        connector=ManualAckConnector(socket),
        dispatcher=dispatcher,
        log=lambda _: None,
    )
    tickets: list[ProjectTicket] = []

    def register_first() -> None:
        tickets.append(
            bridge.register_project(
                "thread-1",
                "codex-pro-project-old",
                first_root,
            )
        )

    first_worker = threading.Thread(target=register_first)
    first_worker.start()
    first = _wait_for_project(socket.sent)
    _ack(socket, first)
    first_worker.join(timeout=2)
    assert not first_worker.is_alive()
    activate_test_session(dispatcher, "thread-1")

    with pytest.raises(RemoteMcpBridgeError, match="did not acknowledge"):
        _ = bridge.register_project(
            "thread-1",
            "codex-pro-project-new",
            second_root,
        )
    result = dispatcher.execute(
        ProjectInfoCommand(
            request_id=RequestId("inspect-old-binding"),
            thread_id="thread-1",
            computer_session_id=TEST_PROJECT_SESSION_ID,
        )
    )
    _wait_for_project_send_count(socket.sent, first.binding_id, expected=2)
    bridge.close()

    assert isinstance(result, ProjectInfoResult)
    assert Path(result.output.root) == first_root.resolve()


def _ack(socket: ManualAckSocket, project: ProjectUpsert) -> None:
    socket.inbound.put(
        ProjectAck(
            project_scope=project.project_scope,
            binding_id=project.binding_id,
        ).model_dump_json()
    )


def _wait_for_project(
    messages: list[str],
    *,
    excluding: str | None = None,
) -> ProjectUpsert:
    deadline = datetime.now(UTC).timestamp() + 2
    while datetime.now(UTC).timestamp() < deadline:
        for message in messages:
            if '"type":"project_upsert"' not in message:
                continue
            project = ProjectUpsert.model_validate_json(message)
            if project.binding_id != excluding:
                return project
    raise AssertionError("expected project registration was not sent")


def _wait_for_project_send_count(
    messages: list[str],
    binding_id: str,
    *,
    expected: int,
) -> None:
    deadline = datetime.now(UTC).timestamp() + 2
    while datetime.now(UTC).timestamp() < deadline:
        count = sum(
            '"type":"project_upsert"' in message
            and ProjectUpsert.model_validate_json(message).binding_id == binding_id
            for message in messages
        )
        if count >= expected:
            return
    raise AssertionError("expected project registration was not restored")


def _config() -> RemoteMcpBridgeConfig:
    return RemoteMcpBridgeConfig(
        bridge_url="wss://example.test/bridge",
        device_id="device-1",
        device_token="secret-token",
        binding_ttl_seconds=600,
        binding_ack_timeout_seconds=2,
        reconnect_delay_seconds=0.01,
    )
