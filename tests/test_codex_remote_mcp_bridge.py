from __future__ import annotations

import queue
from collections.abc import Callable
from contextlib import AbstractContextManager
from dataclasses import replace
from datetime import UTC, datetime, timedelta
from pathlib import Path
from types import TracebackType
from typing import Self, final

from codex_remote_mcp_bridge import RemoteMcpBridge
from codex_remote_mcp_bridge_config import RemoteMcpBridgeConfig
from codex_remote_mcp_dispatch import LocalProjectDispatcher
from simdorei_mcp_common.messages import (
    BridgeHello,
    GatewayHello,
    ProjectAck,
    ProjectInfoCommand,
    ProjectInfoResult,
    ProjectOperationCommand,
    ProjectOperationResult,
    ProjectSessionCommand,
    ProjectSessionResult,
    ProjectUpsert,
    RequestId,
)
from simdorei_mcp_common.operation_outputs import ComputerScreenshotOutput
from simdorei_mcp_common.operation_requests import (
    ComputerClickRequest,
    ComputerScreenshotRequest,
)
from tests.remote_mcp_computer_fakes import (
    FakeComputerPlatform,
    computer_window,
    make_controller,
)


class FakeSocket:
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
        if '"type":"project_upsert"' in message:
            project = ProjectUpsert.model_validate_json(message)
            self.inbound.put(
                ProjectAck(
                    project_scope=project.project_scope,
                    binding_id=project.binding_id,
                ).model_dump_json()
            )

    def recv(self, timeout: float | None = None) -> str:
        try:
            return self.inbound.get(timeout=timeout)
        except queue.Empty as exc:
            raise TimeoutError from exc

    def close(self) -> None:
        return None


@final
class FakeConnector:
    def __init__(self, socket: FakeSocket) -> None:
        self.socket: FakeSocket = socket

    def __call__(
        self,
        config: RemoteMcpBridgeConfig,
    ) -> AbstractContextManager[FakeSocket]:
        _ = config
        return self.socket


@final
class SequenceConnector:
    def __init__(self, sockets: list[FakeSocket]) -> None:
        self.sockets: list[FakeSocket] = sockets
        self.calls: int = 0

    def __call__(
        self,
        config: RemoteMcpBridgeConfig,
    ) -> AbstractContextManager[FakeSocket]:
        _ = config
        index = min(self.calls, len(self.sockets) - 1)
        self.calls += 1
        return self.sockets[index]


def _config() -> RemoteMcpBridgeConfig:
    return RemoteMcpBridgeConfig(
        bridge_url="wss://example.test/bridge",
        device_id="device-1",
        device_token="secret-token",
        binding_ttl_seconds=600,
        binding_ack_timeout_seconds=2,
        reconnect_delay_seconds=0.01,
    )


def test_bridge_registers_project_and_dispatches_project_info(tmp_path: Path) -> None:
    # Given
    socket = FakeSocket()
    bridge = RemoteMcpBridge(
        _config(),
        connector=FakeConnector(socket),
        log=lambda _: None,
    )

    # When
    ticket = bridge.register_project("thread-1", "codex-pro-project-1", tmp_path)
    session_id = "project-session-generation-1"
    socket.inbound.put(
        ProjectSessionCommand(
            request_id=RequestId("activate-1"),
            thread_id="thread-1",
            computer_session_id=session_id,
            computer_session_generation=1,
        ).model_dump_json()
    )
    socket.inbound.put(
        ProjectInfoCommand(
            request_id=RequestId("request-1"),
            thread_id="thread-1",
            computer_session_id=session_id,
        ).model_dump_json()
    )
    result_json = _wait_for_sent(
        socket.sent,
        lambda raw: '"type":"project_info_result"' in raw,
    )
    bridge.close()

    # Then
    assert ticket.project_scope == "codex-pro-project-1"
    project = next(
        ProjectUpsert.model_validate_json(raw)
        for raw in socket.sent
        if '"type":"project_upsert"' in raw
    )
    assert project.thread_id == "thread-1"
    result = ProjectInfoResult.model_validate_json(result_json)
    assert Path(result.output.root) == tmp_path.resolve()


def test_reconnect_prunes_expired_binding_and_requires_a_fresh_ack(
    tmp_path: Path,
) -> None:
    now = [datetime(2026, 7, 31, tzinfo=UTC)]
    first = FakeSocket()
    second = FakeSocket()
    connector = SequenceConnector([first, second])
    config = replace(_config(), binding_ttl_seconds=10)
    bridge = RemoteMcpBridge(
        config,
        connector=connector,
        log=lambda _: None,
        now=lambda: now[0],
    )
    _ = bridge.register_project("thread-1", "codex-pro-project-1", tmp_path)

    now[0] += timedelta(seconds=11)
    first.inbound.put(GatewayHello().model_dump_json())
    _ = _wait_for_sent(second.sent, lambda raw: '"type":"hello"' in raw)
    assert not any('"type":"project_upsert"' in raw for raw in second.sent)

    ticket = bridge.register_project(
        "thread-1",
        "codex-pro-project-1",
        tmp_path,
    )
    bridge.close()

    assert ticket.expires_at == now[0] + timedelta(seconds=10)
    assert sum('"type":"project_upsert"' in raw for raw in second.sent) == 1


def test_bridge_disconnect_invalidates_local_observation_before_reconnect(
    tmp_path: Path,
) -> None:
    first = FakeSocket()
    second = FakeSocket()
    connector = SequenceConnector([first, second])
    platform = FakeComputerPlatform(computer_window())
    dispatcher = LocalProjectDispatcher(
        computer_factory=lambda: make_controller(platform),
    )
    logs: list[str] = []
    bridge = RemoteMcpBridge(
        _config(),
        connector=connector,
        dispatcher=dispatcher,
        log=logs.append,
    )
    _ = bridge.register_project("thread-1", "codex-pro-project-1", tmp_path)
    activated = dispatcher.execute(
        ProjectSessionCommand(
            request_id=RequestId("activate-computer-session-a"),
            thread_id="thread-1",
            computer_session_id="computer-session-a",
            computer_session_generation=1,
        )
    )
    assert isinstance(activated, ProjectSessionResult)
    captured = dispatcher.execute(
        ProjectOperationCommand(
            request_id=RequestId("request-capture-before-disconnect"),
            thread_id="thread-1",
            computer_session_id="computer-session-a",
            operation=ComputerScreenshotRequest(window_id=42),
        )
    )
    assert isinstance(captured, ProjectOperationResult)
    assert isinstance(captured.output, ComputerScreenshotOutput)

    first.inbound.put(GatewayHello().model_dump_json())
    _ = _wait_for_value(
        logs,
        lambda value: value.startswith("remote_mcp_bridge_disconnected"),
    )
    stale = dispatcher.execute(
        ProjectOperationCommand(
            request_id=RequestId("request-click-after-disconnect"),
            thread_id="thread-1",
            computer_session_id="computer-session-a",
            operation=ComputerClickRequest(
                window_id=42,
                observation_id=captured.output.observation_id,
                x=10,
                y=10,
            ),
        )
    )
    bridge.close()

    assert not isinstance(stale, ProjectOperationResult)
    assert platform.clicks == []


def test_reconnect_advertises_only_latest_scope_for_a_thread(tmp_path: Path) -> None:
    first, second = FakeSocket(), FakeSocket()
    bridge = RemoteMcpBridge(
        _config(), connector=SequenceConnector([first, second]), log=lambda _: None
    )
    _ = bridge.register_project("thread-1", "codex-pro-project-old", tmp_path)
    _ = bridge.register_project("thread-1", "codex-pro-project-new", tmp_path)

    first.inbound.put(GatewayHello().model_dump_json())
    _ = _wait_for_sent(second.sent, lambda raw: '"type":"project_upsert"' in raw)
    bridge.close()

    assert tuple(
        ProjectUpsert.model_validate_json(raw).project_scope
        for raw in second.sent
        if '"type":"project_upsert"' in raw
    ) == ("codex-pro-project-new",)


def _wait_for_sent(messages: list[str], predicate: Callable[[str], bool]) -> str:
    deadline = datetime.now(UTC).timestamp() + 2
    while datetime.now(UTC).timestamp() < deadline:
        for message in messages:
            if predicate(message):
                return message
    raise AssertionError("expected bridge message was not sent")


def _wait_for_value(values: list[str], predicate: Callable[[str], bool]) -> str:
    deadline = datetime.now(UTC).timestamp() + 2
    while datetime.now(UTC).timestamp() < deadline:
        for value in values:
            if predicate(value):
                return value
    raise AssertionError("expected value was not observed")
