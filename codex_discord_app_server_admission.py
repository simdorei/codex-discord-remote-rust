from __future__ import annotations

from collections.abc import AsyncGenerator, Awaitable, Callable
from contextlib import AbstractContextManager, asynccontextmanager, nullcontext
from contextvars import ContextVar, Token
from datetime import datetime, timezone
from typing import Protocol, TypeVar, cast

from codex_app_server_transport_lifecycle import (
    AppServerGenerationExpiredError,
    AppServerGenerationMismatch,
    AppServerLifecycleSnapshot,
)


ChannelT = TypeVar("ChannelT")
_EXPECTED_GENERATION: ContextVar[int | None] = ContextVar(
    "codex_discord_expected_app_server_generation",
    default=None,
)


class LifecycleClient(Protocol):
    def lifecycle_snapshot(self) -> AppServerLifecycleSnapshot: ...


PromptAdmissionSender = Callable[[ChannelT, str], Awaitable[object]]
LogFunc = Callable[[str], None]
DeliveryAdmission = Callable[[int | None], AbstractContextManager[AppServerLifecycleSnapshot]]


def current_expected_app_server_generation() -> int | None:
    return _EXPECTED_GENERATION.get()


def _message_created_timestamp(source_message: object) -> float | None:
    created_at = getattr(source_message, "created_at", None)
    if isinstance(created_at, int | float):
        return float(created_at)
    if not isinstance(created_at, datetime):
        return None
    if created_at.tzinfo is None:
        created_at = created_at.replace(tzinfo=timezone.utc)
    return created_at.timestamp()


def _has_discord_message_id(source_message: object | None) -> bool:
    try:
        return int(getattr(source_message, "id", 0) or 0) > 0
    except (TypeError, ValueError):
        return False


def _expired_error(expected_generation: int, snapshot: AppServerLifecycleSnapshot) -> AppServerGenerationExpiredError:
    return AppServerGenerationMismatch(
        expected_generation=expected_generation,
        actual_generation=snapshot.generation,
        healthy=snapshot.healthy,
    )


def _set_expected_generation(generation: int | None) -> Token[int | None]:
    return _EXPECTED_GENERATION.set(generation)


def _delivery_admission(
    client: LifecycleClient,
    expected_generation: int | None,
) -> AbstractContextManager[AppServerLifecycleSnapshot]:
    method = getattr(client, "delivery_admission", None)
    if callable(method):
        return cast(DeliveryAdmission, method)(expected_generation)
    return nullcontext(client.lifecycle_snapshot())


@asynccontextmanager
async def admit_prompt_delivery(
    channel: ChannelT,
    source_message: object | None,
    *,
    expected_generation: int | None,
    transport_enabled: bool,
    client: LifecycleClient,
    send_notice: PromptAdmissionSender[ChannelT],
    log: LogFunc,
) -> AsyncGenerator[bool]:
    if not transport_enabled:
        token = _set_expected_generation(None)
        try:
            yield True
        finally:
            _EXPECTED_GENERATION.reset(token)
        return

    inherited_generation = current_expected_app_server_generation()
    if expected_generation is None and inherited_generation is not None:
        expected_generation = inherited_generation
    # Discord message, slash, and component entry points all supply a source object.
    # Keep source-less calls only as the legacy internal/test auto-start path.
    if source_message is None and expected_generation is None:
        snapshot = client.lifecycle_snapshot()
        token = _set_expected_generation(snapshot.generation if snapshot.healthy else None)
        try:
            yield True
        finally:
            _EXPECTED_GENERATION.reset(token)
        return
    with _delivery_admission(client, expected_generation) as snapshot:
        if expected_generation is not None and (
            not snapshot.healthy or snapshot.generation != expected_generation
        ):
            raise _expired_error(expected_generation, snapshot)

        reason = ""
        if not snapshot.healthy:
            reason = "app_server_unavailable"
        else:
            created_at = _message_created_timestamp(source_message)
            if created_at is not None:
                if snapshot.accepting_since is None or created_at < snapshot.accepting_since:
                    reason = "source_predates_generation"
            elif _has_discord_message_id(source_message):
                reason = "source_timestamp_missing"

        if reason:
            message_id = getattr(source_message, "id", None)
            log(
                f"app_server_prompt_discarded reason={reason} "
                + f"message={message_id or '-'} generation={snapshot.generation}"
            )
            if expected_generation is not None:
                raise _expired_error(expected_generation, snapshot)
            _ = await send_notice(
                channel,
                "Codex was unavailable when this message was received. "
                + "It was not queued or replayed. Please resend it after Codex is available.",
            )
            yield False
            return

        token = _set_expected_generation(snapshot.generation)
        try:
            yield True
        finally:
            _EXPECTED_GENERATION.reset(token)
