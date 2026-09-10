from __future__ import annotations

import threading
from time import monotonic
from types import TracebackType
from typing import Self, final

from codex_remote_mcp_bridge_connection import (
    SerializedBridgeConnection,
    SerializedBridgeSocket,
    close_socket_before_deadline,
)


@final
class _BlockingSendSocket:
    def __init__(self) -> None:
        self.send_started = threading.Event()
        self.release_send = threading.Event()
        self.close_called = threading.Event()
        self.sending = False
        self.close_overlapped_send = False

    def send(self, message: str) -> None:
        _ = message
        self.sending = True
        self.send_started.set()
        assert self.release_send.wait(timeout=2)
        self.sending = False

    def recv(self, timeout: float | None = None) -> str:
        _ = timeout
        return "message"

    def close(self) -> None:
        self.close_overlapped_send = self.sending
        self.close_called.set()


@final
class _ClosingSocketContext:
    def __init__(self, socket: _BlockingSendSocket) -> None:
        self.socket = socket

    def __enter__(self) -> Self:
        return self

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        _ = exc_type, exc, traceback
        self.socket.close()

    def send(self, message: str) -> None:
        self.socket.send(message)

    def recv(self, timeout: float | None = None) -> str:
        return self.socket.recv(timeout)

    def close(self) -> None:
        self.socket.close()


def test_serialized_bridge_socket_never_closes_during_a_send() -> None:
    raw = _BlockingSendSocket()
    socket = SerializedBridgeSocket(raw)
    sender = threading.Thread(target=socket.send, args=("hello",))
    closer = threading.Thread(target=socket.close)

    sender.start()
    assert raw.send_started.wait(timeout=1)
    closer.start()
    assert not raw.close_called.wait(timeout=0.05)

    raw.release_send.set()
    sender.join(timeout=1)
    closer.join(timeout=1)

    assert not sender.is_alive()
    assert not closer.is_alive()
    assert raw.close_called.is_set()
    assert raw.close_overlapped_send is False


def test_connector_context_exit_never_closes_during_a_send() -> None:
    raw = _BlockingSendSocket()
    context = _ClosingSocketContext(raw)
    connection = SerializedBridgeConnection(context)
    socket = connection.__enter__()
    sender = threading.Thread(target=socket.send, args=("hello",))
    exiter = threading.Thread(target=connection.__exit__, args=(None, None, None))

    sender.start()
    assert raw.send_started.wait(timeout=1)
    exiter.start()
    assert not raw.close_called.wait(timeout=0.05)

    raw.release_send.set()
    sender.join(timeout=1)
    exiter.join(timeout=1)

    assert not sender.is_alive()
    assert not exiter.is_alive()
    assert raw.close_called.is_set()
    assert raw.close_overlapped_send is False


def test_socket_close_returns_at_deadline_when_send_never_finishes() -> None:
    raw = _BlockingSendSocket()
    socket = SerializedBridgeSocket(raw)
    sender = threading.Thread(target=socket.send, args=("hello",))
    sender.start()
    assert raw.send_started.wait(timeout=1)

    started = monotonic()
    attempt = close_socket_before_deadline(
        socket,
        deadline_monotonic=started + 0.05,
    )

    assert not attempt.completed
    assert attempt.error is None
    assert monotonic() - started < 0.5
    raw.release_send.set()
    sender.join(timeout=1)
    assert not sender.is_alive()
