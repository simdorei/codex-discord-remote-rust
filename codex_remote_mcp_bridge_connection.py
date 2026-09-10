from __future__ import annotations

import threading
from collections.abc import Callable, Generator
from contextlib import AbstractContextManager, contextmanager
from dataclasses import dataclass
from time import monotonic
from types import TracebackType
from typing import Protocol, final, override

from websockets.sync.client import connect

from codex_remote_mcp_bridge_config import RemoteMcpBridgeConfig
from codex_remote_mcp_computer_errors import ComputerControlError
from codex_remote_mcp_dispatch import LocalProjectDispatcher
from simdorei_mcp_common.messages import (
    GatewayCommand,
    GatewayHello,
    ProjectAck,
    parse_gateway_message,
)


@dataclass(frozen=True, slots=True)
class RemoteMcpBridgeError(Exception):
    reason: str

    @override
    def __str__(self) -> str:
        return self.reason


class BridgeSocket(Protocol):
    def send(self, message: str) -> None: ...

    def recv(self, timeout: float | None = None) -> str | bytes: ...

    def close(self) -> None: ...


@dataclass(frozen=True, slots=True)
class SocketCloseAttempt:
    completed: bool
    error: BaseException | None


@final
class SerializedBridgeSocket:
    """Prevents a local shutdown from closing during a bridge send."""

    def __init__(self, socket: BridgeSocket) -> None:
        self._socket = socket
        self._send_close_lock = threading.Lock()

    def send(self, message: str) -> None:
        with self._send_close_lock:
            self._socket.send(message)

    def recv(self, timeout: float | None = None) -> str | bytes:
        return self._socket.recv(timeout=timeout)

    def close(self) -> None:
        with self._send_close_lock:
            self._socket.close()

    def serialize_context_exit(
        self, exit_context: Callable[[], bool | None]
    ) -> bool | None:
        with self._send_close_lock:
            return exit_context()


@final
class SerializedBridgeConnection:
    """Keeps connector teardown on the socket's send/close lock."""

    def __init__(self, context: AbstractContextManager[BridgeSocket]) -> None:
        self._context = context
        self._socket: SerializedBridgeSocket | None = None

    def __enter__(self) -> SerializedBridgeSocket:
        socket = SerializedBridgeSocket(self._context.__enter__())
        self._socket = socket
        return socket

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        traceback: TracebackType | None,
    ) -> bool | None:
        socket = self._socket
        if socket is None:
            raise RemoteMcpBridgeError(
                "The serialized bridge connection was not entered."
            )
        return socket.serialize_context_exit(
            lambda: self._context.__exit__(exc_type, exc, traceback)
        )


def close_socket_before_deadline(
    socket: BridgeSocket,
    *,
    deadline_monotonic: float,
) -> SocketCloseAttempt:
    completed = threading.Event()
    errors: list[BaseException] = []

    def close_socket() -> None:
        try:
            socket.close()
        except BaseException as exc:  # noqa: BLE001 - returned to owner thread.
            errors.append(exc)
        finally:
            completed.set()

    closer = threading.Thread(
        target=close_socket,
        name="codex-remote-mcp-socket-close",
        daemon=True,
    )
    closer.start()
    finished = completed.wait(max(0.0, deadline_monotonic - monotonic()))
    return SocketCloseAttempt(
        completed=finished,
        error=errors[0] if errors else None,
    )


BridgeConnector = Callable[
    [RemoteMcpBridgeConfig],
    AbstractContextManager[BridgeSocket],
]


@contextmanager
def connect_bridge(config: RemoteMcpBridgeConfig) -> Generator[BridgeSocket]:
    with connect(
        config.bridge_url,
        additional_headers={"Authorization": f"Bearer {config.device_token}"},
        open_timeout=10,
        ping_interval=20,
        ping_timeout=20,
        max_size=12_000_000,
    ) as socket:
        yield socket


def receive_message(
    socket: BridgeSocket,
    *,
    timeout: float | None = None,
) -> GatewayHello | ProjectAck | GatewayCommand:
    raw = socket.recv(timeout=timeout)
    if not isinstance(raw, str):
        raise RemoteMcpBridgeError("The gateway sent an unsupported binary message.")
    return parse_gateway_message(raw)


def invalidate_sessions(
    dispatcher: LocalProjectDispatcher,
    log: Callable[[str], None],
    *,
    deadline_monotonic: float | None = None,
) -> bool:
    try:
        dispatcher.invalidate_computer_sessions(
            deadline_monotonic=deadline_monotonic,
        )
    except (ComputerControlError, TimeoutError) as exc:
        log("remote_mcp_computer_cleanup_failed " + f"error={type(exc).__name__}")
        return False
    return True
