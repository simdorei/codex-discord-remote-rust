from __future__ import annotations

import threading
import time
from collections.abc import Callable
from dataclasses import dataclass, field
from datetime import UTC, datetime, timedelta
from typing import Final

from simdorei_mcp_common.operation_requests import (
    CommandRunRequest,
    GitCommitRequest,
    GitPushRequest,
    ProjectOperation,
    ProjectStatusRequest,
    RepoDiffRequest,
    RepoStatusRequest,
)
from simdorei_mcp_common.terminal_protocol import TerminalExecRequest


DEFAULT_REQUEST_LIFETIME_SECONDS: Final = 60
TRANSPORT_GRACE_SECONDS: Final = 15
MAX_COMMAND_RUN_SECONDS: Final = 300
MAX_TERMINAL_EXEC_SECONDS: Final = 3_600
COMMAND_REQUEST_LIFETIME_SECONDS: Final = (
    MAX_COMMAND_RUN_SECONDS + TRANSPORT_GRACE_SECONDS
)
GIT_REQUEST_LIFETIME_SECONDS: Final = 120 + TRANSPORT_GRACE_SECONDS
MAX_REQUEST_LIFETIME_SECONDS: Final = MAX_TERMINAL_EXEC_SECONDS + TRANSPORT_GRACE_SECONDS
GATEWAY_REQUEST_TIMEOUT_SECONDS: Final = MAX_REQUEST_LIFETIME_SECONDS + 15


class RequestDeadlineExpired(TimeoutError):
    """Raised before starting local work after the shared request deadline."""


class RequestExecutionCancelled(RequestDeadlineExpired):
    """Raised after the owning bridge connection cancels local execution."""


@dataclass(frozen=True, slots=True)
class RequestBudget:
    _deadline_monotonic: float
    _clock: Callable[[], float] = field(repr=False, compare=False)
    _cancel_event: threading.Event | None = field(
        default=None,
        repr=False,
        compare=False,
    )

    @classmethod
    def from_deadline(
        cls,
        deadline_at: datetime,
        *,
        cancel_event: threading.Event | None = None,
    ) -> RequestBudget:
        clock = time.monotonic
        remaining_seconds = (deadline_at - datetime.now(UTC)).total_seconds()
        return cls(
            _deadline_monotonic=clock() + max(0.0, remaining_seconds),
            _clock=clock,
            _cancel_event=cancel_event,
        )

    @property
    def cancel_event(self) -> threading.Event | None:
        return self._cancel_event

    def ensure_active(self) -> None:
        _ = self.remaining()

    def remaining(self, cap_seconds: float | None = None) -> float:
        if self._cancel_event is not None and self._cancel_event.is_set():
            raise RequestExecutionCancelled(
                "The local project request was cancelled with its bridge connection."
            )
        remaining_seconds = self._deadline_monotonic - self._clock()
        if remaining_seconds <= 0:
            raise RequestDeadlineExpired(
                "The local project request expired before execution."
            )
        if cap_seconds is None:
            return remaining_seconds
        return min(cap_seconds, remaining_seconds)


def default_request_deadline() -> datetime:
    return datetime.now(UTC) + timedelta(seconds=DEFAULT_REQUEST_LIFETIME_SECONDS)


def operation_request_deadline(operation: ProjectOperation) -> datetime:
    lifetime = operation_request_lifetime_seconds(operation)
    return datetime.now(UTC) + timedelta(seconds=lifetime)


def operation_request_lifetime_seconds(operation: ProjectOperation) -> int:
    if isinstance(operation, CommandRunRequest):
        return operation.timeout_seconds + TRANSPORT_GRACE_SECONDS
    if isinstance(operation, TerminalExecRequest):
        return operation.timeout_seconds + TRANSPORT_GRACE_SECONDS
    if isinstance(operation, GitPushRequest):
        return COMMAND_REQUEST_LIFETIME_SECONDS
    if isinstance(
        operation,
        (
            RepoStatusRequest,
            RepoDiffRequest,
            GitCommitRequest,
            ProjectStatusRequest,
        ),
    ):
        return GIT_REQUEST_LIFETIME_SECONDS
    return DEFAULT_REQUEST_LIFETIME_SECONDS


__all__ = [
    "DEFAULT_REQUEST_LIFETIME_SECONDS",
    "COMMAND_REQUEST_LIFETIME_SECONDS",
    "GATEWAY_REQUEST_TIMEOUT_SECONDS",
    "GIT_REQUEST_LIFETIME_SECONDS",
    "MAX_REQUEST_LIFETIME_SECONDS",
    "MAX_TERMINAL_EXEC_SECONDS",
    "RequestBudget",
    "RequestDeadlineExpired",
    "RequestExecutionCancelled",
    "default_request_deadline",
    "operation_request_deadline",
    "operation_request_lifetime_seconds",
]
