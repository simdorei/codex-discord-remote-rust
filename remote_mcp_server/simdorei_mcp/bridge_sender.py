from __future__ import annotations

from typing import Protocol, final, override

import anyio

from remote_mcp_server.simdorei_mcp.broker_models import BridgeSender
from simdorei_mcp_common.messages import GatewayCommand, GatewayHello, ProjectAck


class GatewayWebSocket(Protocol):
    async def send_text(self, data: str) -> None: ...

    async def close(self, code: int = 1000, reason: str | None = None) -> None: ...


@final
class WebSocketBridgeSender(BridgeSender):
    """Serializes every send and close on one device socket."""

    def __init__(self, socket: GatewayWebSocket) -> None:
        self._socket = socket
        self._send_close_lock = anyio.Lock()

    @override
    async def send(self, command: GatewayCommand) -> None:
        await self._send_text(command.model_dump_json())

    async def send_control(self, message: GatewayHello | ProjectAck) -> None:
        await self._send_text(message.model_dump_json())

    @override
    async def close(self) -> None:
        await self.reject(1012, "bridge replaced")

    async def reject(self, code: int, reason: str) -> None:
        async with self._send_close_lock:
            await self._socket.close(code=code, reason=reason)

    async def _send_text(self, payload: str) -> None:
        async with self._send_close_lock:
            await self._socket.send_text(payload)
