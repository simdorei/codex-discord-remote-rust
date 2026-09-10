from __future__ import annotations

import queue
import threading
from collections.abc import Callable, Generator
from contextlib import AbstractContextManager, contextmanager
from pathlib import Path
from types import TracebackType
from typing import Protocol, Self, final

import pytest
from websockets.exceptions import ConnectionClosedError
from websockets.frames import Close

import codex_remote_mcp_bridge as bridge_module
from codex_remote_mcp_bridge import RemoteMcpBridge
from codex_remote_mcp_bridge_config import RemoteMcpBridgeConfig
from codex_remote_mcp_bridge_connection import RemoteMcpBridgeError
from simdorei_mcp_common.messages import (
    BridgeHello,
    GatewayHello,
    ProjectAck,
    ProjectUpsert,
)


class BridgeTestSocket(Protocol):
    def __enter__(self) -> Self: ...

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        traceback: TracebackType | None,
    ) -> None: ...

    def send(self, message: str) -> None: ...

    def recv(self, timeout: float | None = None) -> str: ...

    def close(self) -> None: ...


@final
class ReplacedSocket:
    def __init__(self, close_reason: str = "bridge replaced") -> None:
        self.inbound: queue.Queue[str] = queue.Queue()
        self.replace = threading.Event()
        self.close_reason = close_reason

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
            return self.inbound.get_nowait()
        except queue.Empty:
            if not self.replace.wait(timeout=timeout):
                raise TimeoutError from None
            raise ConnectionClosedError(
                Close(1012, self.close_reason),
                None,
            )

    def close(self) -> None:
        return None


@final
class CountingConnector:
    def __init__(self, socket_factory: Callable[[], BridgeTestSocket]) -> None:
        self._socket_factory = socket_factory
        self.calls = 0
        self.second_call = threading.Event()

    def __call__(
        self,
        config: RemoteMcpBridgeConfig,
    ) -> AbstractContextManager[BridgeTestSocket]:
        _ = config
        self.calls += 1
        if self.calls == 2:
            self.second_call.set()
        return self._socket_factory()


@final
class RejectedHandshakeSocket:
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
        _ = BridgeHello.model_validate_json(message)

    def recv(self, timeout: float | None = None) -> str:
        _ = timeout
        raise ConnectionClosedError(Close(1008, "unauthorized"), None)

    def close(self) -> None:
        return None


def test_bridge_replacement_is_terminal_and_logs_close_details(
    tmp_path: Path,
) -> None:
    socket = ReplacedSocket()
    connector = CountingConnector(lambda: socket)
    logs: list[str] = []
    bridge = RemoteMcpBridge(
        _config(),
        connector=connector,
        log=logs.append,
    )

    try:
        _ = bridge.register_project(
            "thread-1",
            "codex-pro-project-1",
            tmp_path,
        )
        socket.replace.set()

        assert not connector.second_call.wait(timeout=0.1)
        assert (
            "remote_mcp_bridge_displaced close_code=1012 close_reason='bridge replaced'"
        ) in logs
    finally:
        bridge.close()


def test_service_restart_close_still_reconnects(tmp_path: Path) -> None:
    first = ReplacedSocket("service restart")
    second = ReplacedSocket()
    sockets = iter((first, second))
    connector = CountingConnector(lambda: next(sockets))
    logs: list[str] = []
    bridge = RemoteMcpBridge(
        _config(),
        connector=connector,
        log=logs.append,
    )

    try:
        _ = bridge.register_project(
            "thread-1",
            "codex-pro-project-1",
            tmp_path,
        )
        first.replace.set()

        assert connector.second_call.wait(timeout=0.2)
        assert any(value.startswith("remote_mcp_bridge_disconnected") for value in logs)
    finally:
        bridge.close()


def test_authentication_rejection_is_terminal_without_reconnect(tmp_path: Path) -> None:
    connector = CountingConnector(RejectedHandshakeSocket)
    bridge = RemoteMcpBridge(
        _config(),
        connector=connector,
        log=lambda _: None,
    )

    try:
        with pytest.raises(RemoteMcpBridgeError, match="rejected"):
            _ = bridge.register_project(
                "thread-1",
                "codex-pro-project-1",
                tmp_path,
            )
        assert connector.calls == 1
        assert not connector.second_call.wait(timeout=0.1)
    finally:
        bridge.close()


def test_second_process_owner_cannot_connect(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    connector = CountingConnector(ReplacedSocket)

    @contextmanager
    def deny_process_lock(device_id: str) -> Generator[bool, None, None]:
        _ = device_id
        yield False

    monkeypatch.setattr(
        bridge_module,
        "acquire_remote_mcp_process_lock",
        deny_process_lock,
        raising=False,
    )
    bridge = RemoteMcpBridge(
        _config(),
        connector=connector,
        log=lambda _: None,
    )

    try:
        with pytest.raises(
            RemoteMcpBridgeError,
            match="already owns the remote MCP connection",
        ):
            _ = bridge.register_project(
                "thread-1",
                "codex-pro-project-1",
                tmp_path,
            )
    finally:
        bridge.close()

    assert connector.calls == 0


def test_new_registration_retries_after_process_owner_is_released(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    ownership = iter((False, True))
    connector = CountingConnector(ReplacedSocket)

    @contextmanager
    def changing_process_lock(device_id: str) -> Generator[bool, None, None]:
        _ = device_id
        yield next(ownership)

    monkeypatch.setattr(
        bridge_module,
        "acquire_remote_mcp_process_lock",
        changing_process_lock,
        raising=False,
    )
    bridge = RemoteMcpBridge(_config(), connector=connector, log=lambda _: None)

    try:
        with pytest.raises(RemoteMcpBridgeError, match="already owns"):
            _ = bridge.register_project("thread-1", "project-scope-1", tmp_path)

        ticket = bridge.register_project("thread-1", "project-scope-2", tmp_path)

        assert ticket.project_scope == "project-scope-2"
        assert connector.calls == 1
    finally:
        bridge.close()


def _config() -> RemoteMcpBridgeConfig:
    return RemoteMcpBridgeConfig(
        bridge_url="wss://example.test/bridge",
        device_id="device-1",
        device_token="secret-token",
        binding_ttl_seconds=600,
        binding_ack_timeout_seconds=0.2,
        reconnect_delay_seconds=0.01,
    )
