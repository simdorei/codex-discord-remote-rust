from __future__ import annotations

import hashlib
import threading
from collections import OrderedDict
from collections.abc import Callable
from dataclasses import dataclass
from typing import Final, TypeAlias, final

from simdorei_mcp_common.messages import (
    BridgeResult,
    GatewayCommand,
    OperationErrorResult,
)
from simdorei_mcp_common.canonical_json import canonical_command_json
from simdorei_mcp_common.request_deadlines import RequestBudget


CacheKey: TypeAlias = tuple[str, str | None, str]
MAX_CACHED_RESULTS: Final = 256


@dataclass(frozen=True, slots=True)
class CachedResult:
    fingerprint: str
    result: BridgeResult


@final
class IdempotentResultCache:
    """Shares in-flight requests and remembers successful bridge results."""

    def __init__(self) -> None:
        self._condition = threading.Condition()
        self._in_flight: dict[CacheKey, str] = {}
        self._completed: OrderedDict[CacheKey, CachedResult] = OrderedDict()

    def execute_once(
        self,
        command: GatewayCommand,
        execute: Callable[[], BridgeResult],
        *,
        budget: RequestBudget,
    ) -> BridgeResult:
        key = _cache_key(command)
        fingerprint = _fingerprint(command)
        with self._condition:
            while key in self._in_flight:
                if self._in_flight[key] != fingerprint:
                    return _conflict(command)
                wait_seconds = budget.remaining(0.1)
                _ = self._condition.wait(timeout=wait_seconds)
            cached = self._completed.get(key)
            if cached is not None:
                if cached.fingerprint != fingerprint:
                    return _conflict(command)
                self._completed.move_to_end(key)
                return cached.result
            self._in_flight[key] = fingerprint

        result: BridgeResult | None = None
        try:
            result = execute()
            return result
        finally:
            with self._condition:
                _ = self._in_flight.pop(key, None)
                if result is not None and not isinstance(
                    result,
                    OperationErrorResult,
                ):
                    self._completed[key] = CachedResult(fingerprint, result)
                    self._completed.move_to_end(key)
                    while len(self._completed) > MAX_CACHED_RESULTS:
                        _ = self._completed.popitem(last=False)
                self._condition.notify_all()


def _cache_key(command: GatewayCommand) -> CacheKey:
    return (
        command.thread_id,
        command.computer_session_id,
        str(command.request_id),
    )


def _fingerprint(command: GatewayCommand) -> str:
    payload = canonical_command_json(
        command,
        exclude={"deadline_at", "request_id"},
    )
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()


def _conflict(command: GatewayCommand) -> OperationErrorResult:
    return OperationErrorResult(
        request_id=command.request_id,
        error_code="request_id_conflict",
        message="The request ID was reused with different command content.",
    )


__all__ = ["IdempotentResultCache"]
