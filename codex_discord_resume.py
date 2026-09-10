from __future__ import annotations

import time
from collections.abc import Callable
from dataclasses import dataclass
from enum import StrEnum
from typing import Final, Protocol, assert_never, override

from codex_app_server_transport_replies import (
    CodexAppServerTransportError,
    JsonObject,
)
from codex_app_server_transport_threads import get_thread_status_type

MANUAL_RESUME_TIMEOUT_SEC: Final = 60.0
THREAD_READ_TIMEOUT_SEC: Final = 8.0
THREAD_READ_RECOVERY_TIMEOUT_SEC: Final = 25.0
THREAD_READ_RESTART_OVERHEAD_SEC: Final = 15.0


class ResidentThreadClient(Protocol):
    def read_thread(
        self,
        thread_id: str,
        *,
        include_turns: bool = False,
        timeout_sec: float = THREAD_READ_TIMEOUT_SEC,
        recovery_timeout_sec: float | None = None,
    ) -> JsonObject: ...

    def resume_thread(self, thread_id: str, *, timeout_sec: float = 10.0) -> JsonObject: ...


class ResumeRecoveryState(StrEnum):
    ALREADY_LOADED = "already_loaded"
    RECOVERED = "recovered"


@dataclass(frozen=True, slots=True)
class ResumeRecoveryResult:
    thread_id: str
    state: ResumeRecoveryState


@dataclass(frozen=True, slots=True)
class ResidentThreadRecoveryError(CodexAppServerTransportError):
    thread_id: str
    reason: str

    @override
    def __str__(self) -> str:
        return f"Codex thread {self.thread_id} {self.reason}"


ResolveQueueTargetFunc = Callable[[int | None, str | None], tuple[str | None, str]]
ResolveSelectedTargetFunc = Callable[[], tuple[str | None, str]]


def recover_resident_thread_for_request(
    client: ResidentThreadClient,
    channel_id: int | None,
    ref: str | None,
    *,
    resolve_queue_command_target: ResolveQueueTargetFunc,
    resolve_selected_target: ResolveSelectedTargetFunc,
    timeout_sec: float = MANUAL_RESUME_TIMEOUT_SEC,
    monotonic_func: Callable[[], float] = time.monotonic,
) -> str:
    target_thread_id, target_ref = resolve_queue_command_target(channel_id, ref)
    if target_thread_id is None:
        target_thread_id, selected_ref = resolve_selected_target()
        target_ref = selected_ref or target_ref
    if target_thread_id is None:
        raise ResidentThreadRecoveryError(target_ref or "selected", "has no mapped or selected target")
    result = recover_resident_thread(
        client,
        target_thread_id,
        timeout_sec=timeout_sec,
        monotonic_func=monotonic_func,
    )
    return format_resume_recovery_message(result, target_ref)


def recover_resident_thread(
    client: ResidentThreadClient,
    thread_id: str,
    *,
    timeout_sec: float = MANUAL_RESUME_TIMEOUT_SEC,
    monotonic_func: Callable[[], float] = time.monotonic,
) -> ResumeRecoveryResult:
    deadline = monotonic_func() + max(timeout_sec, 0.0)
    thread = _read_thread_with_deadline(client, thread_id, deadline, monotonic_func)
    if get_thread_status_type(thread) != "notLoaded":
        return ResumeRecoveryResult(thread_id, ResumeRecoveryState.ALREADY_LOADED)

    remaining = _remaining_or_raise(thread_id, deadline, monotonic_func)
    _ = client.resume_thread(thread_id, timeout_sec=remaining)
    confirmed = _read_thread_with_deadline(client, thread_id, deadline, monotonic_func)
    if get_thread_status_type(confirmed) == "notLoaded":
        raise ResidentThreadRecoveryError(thread_id, "is still not loaded after resume")
    return ResumeRecoveryResult(thread_id, ResumeRecoveryState.RECOVERED)


def format_resume_recovery_message(result: ResumeRecoveryResult, target_ref: str) -> str:
    label = target_ref or result.thread_id
    status = _format_resume_recovery_status(result.state)
    return "\n".join(
        [
            "Codex thread resume check complete.",
            f"thread: {label}",
            f"status: {status}",
            "No prompt was resent. Resend the original message when ready.",
        ]
    )


def _format_resume_recovery_status(state: ResumeRecoveryState) -> str:
    match state:
        case ResumeRecoveryState.ALREADY_LOADED:
            return "already loaded"
        case ResumeRecoveryState.RECOVERED:
            return "recovered"
    assert_never(state)


def _read_thread_with_deadline(
    client: ResidentThreadClient,
    thread_id: str,
    deadline: float,
    monotonic_func: Callable[[], float],
) -> JsonObject:
    remaining = _remaining_or_raise(thread_id, deadline, monotonic_func)
    first_timeout = min(THREAD_READ_TIMEOUT_SEC, remaining)
    response = client.read_thread(
        thread_id,
        include_turns=False,
        timeout_sec=first_timeout,
        recovery_timeout_sec=min(
            THREAD_READ_RECOVERY_TIMEOUT_SEC,
            max(
                0.0,
                remaining - first_timeout - THREAD_READ_RESTART_OVERHEAD_SEC,
            ),
        ),
    )
    thread = response.get("thread")
    if not isinstance(thread, dict):
        raise ResidentThreadRecoveryError(thread_id, "returned no thread payload")
    return thread


def _remaining_or_raise(
    thread_id: str,
    deadline: float,
    monotonic_func: Callable[[], float],
) -> float:
    remaining = max(0.0, deadline - monotonic_func())
    if remaining <= 0:
        raise ResidentThreadRecoveryError(thread_id, "resume check timed out")
    return remaining
