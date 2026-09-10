from __future__ import annotations

import threading
from contextlib import AbstractContextManager
from pathlib import Path
from types import TracebackType
from typing import Self, final

from codex_remote_mcp_bridge import RemoteMcpBridge
from codex_remote_mcp_bridge_config import RemoteMcpBridgeConfig
from codex_remote_mcp_bridge_connection import RemoteMcpBridgeError


@final
class BlockingHandshakeSocket:
    def __init__(self) -> None:
        self.hello_sent: threading.Event = threading.Event()
        self.release: threading.Event = threading.Event()
        self.exited: threading.Event = threading.Event()

    def __enter__(self) -> Self:
        return self

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        _ = exc_type, exc, traceback
        self.exited.set()

    def send(self, message: str) -> None:
        if '"type":"hello"' in message:
            self.hello_sent.set()

    def recv(self, timeout: float | None = None) -> str:
        _ = timeout
        if not self.release.wait(timeout=5):
            raise TimeoutError
        raise OSError("socket closed")

    def close(self) -> None:
        self.release.set()


@final
class BlockingHandshakeConnector:
    def __init__(self, socket: BlockingHandshakeSocket) -> None:
        self.socket: BlockingHandshakeSocket = socket

    def __call__(
        self,
        config: RemoteMcpBridgeConfig,
    ) -> AbstractContextManager[BlockingHandshakeSocket]:
        _ = config
        return self.socket


def test_close_interrupts_a_bridge_blocked_during_handshake(tmp_path: Path) -> None:
    socket = BlockingHandshakeSocket()
    bridge = RemoteMcpBridge(
        RemoteMcpBridgeConfig(
            bridge_url="wss://example.test/bridge",
            device_id="device-1",
            device_token="secret-token",
            binding_ttl_seconds=600,
            binding_ack_timeout_seconds=2,
            reconnect_delay_seconds=0.01,
        ),
        connector=BlockingHandshakeConnector(socket),
        log=lambda _: None,
    )
    registration_errors: list[RemoteMcpBridgeError] = []

    def register() -> None:
        try:
            _ = bridge.register_project(
                "thread-1",
                "codex-pro-project-1",
                tmp_path,
            )
        except RemoteMcpBridgeError as exc:
            registration_errors.append(exc)

    worker = threading.Thread(target=register)
    worker.start()
    assert socket.hello_sent.wait(timeout=2)

    bridge.close()
    worker.join(timeout=3)

    assert socket.exited.wait(timeout=0.5)
    assert not worker.is_alive()
    assert registration_errors
