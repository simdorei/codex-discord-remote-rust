from __future__ import annotations

from dataclasses import dataclass

from websockets.exceptions import ConnectionClosed

from codex_remote_mcp_redaction import redact


@dataclass(frozen=True, slots=True)
class BridgeCloseDetails:
    code: int | None
    reason: str

    @property
    def was_replaced(self) -> bool:
        return self.code == 1012 and self.reason == "bridge replaced"

    @property
    def was_rejected(self) -> bool:
        return self.code in {1002, 1008}

    def log_fields(self) -> str:
        safe_reason = redact(self.reason)[:256]
        return f"close_code={self.code} close_reason={safe_reason!r}"


def bridge_close_details(exc: BaseException) -> BridgeCloseDetails | None:
    if not isinstance(exc, ConnectionClosed):
        return None
    close = exc.rcvd or exc.sent
    if close is None:
        return BridgeCloseDetails(code=None, reason="")
    return BridgeCloseDetails(code=close.code, reason=close.reason)


__all__ = ["BridgeCloseDetails", "bridge_close_details"]
