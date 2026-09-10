from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
import hashlib
import time
from dataclasses import dataclass, field
from typing import Protocol

from codex_discord_text import (
    DISCORD_MAX_LEN,
    env_flag,
    format_discord_command_label,
    split_message,
)

DISCORD_RESTARTING_ERROR = "Discord bot is restarting; refusing new Discord delivery."
DISCORD_RESTARTING_NOTICE = (
    "Discord bot is restarting; this request was not accepted.\n"
    "Retry after the bot is online again."
)
DISCORD_SEND_RETRY_DELAYS_SECONDS = (0.75, 2.0)
DISCORD_CHUNK_MARKERS_ENABLED = env_flag("DISCORD_CHUNK_MARKERS", True)
DISCORD_CROSS_PATH_DEDUPE_SECONDS = 600.0
DIRECT_PROMPT_DELIVERY_CONTEXT = "send_chunks"
SESSION_MIRROR_DELIVERY_CONTEXT_PREFIX = "session_mirror:"


class DiscordDeliveryRejected(RuntimeError):
    pass


class LogFunc(Protocol):
    def __call__(self, message: str, /) -> None: ...


DiscordIdValue = int | str | bytes | bytearray | None


class MessageSendResult(Protocol):
    @property
    def id(self) -> DiscordIdValue: ...


class MessageableIdentityLike(Protocol):
    @property
    def id(self) -> DiscordIdValue: ...


class Messageable(Protocol):
    @property
    def id(self) -> DiscordIdValue: ...

    async def send(self, content: str) -> MessageSendResult | None: ...


class InteractionResponse(Protocol):
    async def send_message(self, content: str, *, ephemeral: bool = False) -> None: ...


class InteractionCommandLike(Protocol):
    @property
    def name(self) -> str: ...


class InteractionCommandSource(Protocol):
    @property
    def command(self) -> InteractionCommandLike | None: ...


class InteractionLike(Protocol):
    @property
    def command(self) -> InteractionCommandLike | None: ...

    @property
    def response(self) -> InteractionResponse: ...

    @property
    def channel_id(self) -> DiscordIdValue: ...


@dataclass(frozen=True, slots=True)
class RecentCrossPathDelivery:
    source: str
    delivered_at: float


@dataclass(frozen=True, slots=True)
class CrossPathDeliveryClaim:
    key: tuple[str, str]
    source: str
    completion: asyncio.Future[bool]


@dataclass(frozen=True, slots=True)
class CrossPathDeliveryStart:
    claim: CrossPathDeliveryClaim | None = None
    duplicate_source: str | None = None


@dataclass(slots=True)  # noqa: MUTABLE_OK
class DiscordDeliveryState:
    active_deliveries: set[str] = field(default_factory=set)
    active_cross_path_deliveries: dict[tuple[str, str], list[CrossPathDeliveryClaim]] = (
        field(default_factory=dict)
    )
    recent_cross_path_deliveries: dict[tuple[str, str], list[RecentCrossPathDelivery]] = (
        field(default_factory=dict)
    )
    stopping: bool = False
    retry_delays_seconds: tuple[float, ...] = DISCORD_SEND_RETRY_DELAYS_SECONDS
    chunk_markers_enabled: bool = DISCORD_CHUNK_MARKERS_ENABLED
    cross_path_dedupe_seconds: float = DISCORD_CROSS_PATH_DEDUPE_SECONDS


def get_messageable_id(target: MessageableIdentityLike | InteractionLike) -> str:
    return str(getattr(target, "id", None) or getattr(target, "channel_id", None) or "-")


def build_delivery_id(text: str) -> str:
    return hashlib.blake2s(
        str(text or "").encode("utf-8", errors="replace"),
        digest_size=4,
    ).hexdigest()


async def begin_cross_path_delivery(
    state: DiscordDeliveryState,
    *,
    target_id: str,
    text: str,
    context: str,
) -> CrossPathDeliveryStart:
    source = _cross_path_delivery_source(context)
    if source is None or state.cross_path_dedupe_seconds <= 0:
        return CrossPathDeliveryStart()
    key = (target_id, text)
    while True:
        _prune_recent_cross_path_deliveries(state, now=time.monotonic())
        previous_source = _consume_recent_cross_path_delivery(
            state,
            key=key,
            source=source,
        )
        if previous_source is not None:
            return CrossPathDeliveryStart(duplicate_source=previous_source)
        active_counterpart = next(
            (
                claim
                for claim in state.active_cross_path_deliveries.get(key, [])
                if claim.source != source
            ),
            None,
        )
        if active_counterpart is not None:
            _ = await asyncio.shield(active_counterpart.completion)
            continue
        claim = CrossPathDeliveryClaim(
            key=key,
            source=source,
            completion=asyncio.get_running_loop().create_future(),
        )
        state.active_cross_path_deliveries.setdefault(key, []).append(claim)
        return CrossPathDeliveryStart(claim=claim)


def finish_cross_path_delivery(
    state: DiscordDeliveryState,
    *,
    claim: CrossPathDeliveryClaim | None,
    succeeded: bool,
) -> None:
    if claim is None:
        return
    active_claims = state.active_cross_path_deliveries.get(claim.key, [])
    remaining_claims = [
        active_claim for active_claim in active_claims if active_claim is not claim
    ]
    if remaining_claims:
        state.active_cross_path_deliveries[claim.key] = remaining_claims
    else:
        state.active_cross_path_deliveries.pop(claim.key, None)
    if succeeded:
        now = time.monotonic()
        _prune_recent_cross_path_deliveries(state, now=now)
        state.recent_cross_path_deliveries.setdefault(claim.key, []).append(
            RecentCrossPathDelivery(source=claim.source, delivered_at=now)
        )
    if not claim.completion.done():
        claim.completion.set_result(succeeded)


def _cross_path_delivery_source(context: str) -> str | None:
    if context == DIRECT_PROMPT_DELIVERY_CONTEXT:
        return "direct_prompt"
    if context.startswith(SESSION_MIRROR_DELIVERY_CONTEXT_PREFIX):
        return "session_mirror"
    return None


def _consume_recent_cross_path_delivery(
    state: DiscordDeliveryState,
    *,
    key: tuple[str, str],
    source: str,
) -> str | None:
    recent_deliveries = state.recent_cross_path_deliveries.get(key, [])
    for index, delivery in enumerate(recent_deliveries):
        if delivery.source == source:
            continue
        previous_source = delivery.source
        del recent_deliveries[index]
        if not recent_deliveries:
            del state.recent_cross_path_deliveries[key]
        return previous_source
    return None


def _prune_recent_cross_path_deliveries(
    state: DiscordDeliveryState,
    *,
    now: float,
) -> None:
    cutoff = now - state.cross_path_dedupe_seconds
    for key, deliveries in list(state.recent_cross_path_deliveries.items()):
        retained = [delivery for delivery in deliveries if delivery.delivered_at >= cutoff]
        if retained:
            state.recent_cross_path_deliveries[key] = retained
        else:
            del state.recent_cross_path_deliveries[key]


def set_discord_delivery_stopping(
    state: DiscordDeliveryState,
    reason: str,
    *,
    log_func: LogFunc,
) -> None:
    if state.stopping:
        return
    state.stopping = True
    log_func(
        f"discord_delivery_stopping reason={format_discord_command_label(reason, limit=80)} "
        + f"active={len(state.active_deliveries)}"
    )


def clear_discord_delivery_stopping(state: DiscordDeliveryState) -> None:
    state.stopping = False


def is_discord_delivery_stopping(state: DiscordDeliveryState) -> bool:
    return state.stopping


def begin_discord_delivery(
    state: DiscordDeliveryState,
    label: str,
    *,
    log_func: LogFunc,
    allow_during_stop: bool = False,
) -> str:
    if state.stopping and not allow_during_stop:
        safe_label = format_discord_command_label(label, limit=120)
        log_func(f"discord_delivery_rejected context={safe_label or '-'} reason=stopping")
        raise DiscordDeliveryRejected(DISCORD_RESTARTING_ERROR)
    token = f"{label}:{time.monotonic_ns()}"
    state.active_deliveries.add(token)
    return token


def end_discord_delivery(state: DiscordDeliveryState, token: str) -> None:
    state.active_deliveries.discard(token)


async def wait_for_discord_delivery_drain(
    state: DiscordDeliveryState,
    *,
    timeout_seconds: float,
    reason: str,
    log_func: LogFunc,
) -> bool:
    deadline = time.monotonic() + max(0.0, timeout_seconds)
    while state.active_deliveries:
        if time.monotonic() >= deadline:
            log_func(
                f"discord_delivery_drain_timeout reason={reason} "
                + f"active={len(state.active_deliveries)}"
            )
            return False
        await asyncio.sleep(0.1)
    log_func(f"discord_delivery_drain_done reason={reason}")
    return True


async def sleep_discord_delivery_retry(delay_seconds: float) -> None:
    await asyncio.sleep(delay_seconds)


def split_delivery_chunks(text: str, *, state: DiscordDeliveryState) -> list[str]:
    text = str(text or "")
    if not state.chunk_markers_enabled:
        return split_message(text)
    marker_budget = 32
    chunks = split_message(text, limit=max(1, DISCORD_MAX_LEN - marker_budget))
    if len(chunks) <= 1:
        return chunks
    total = len(chunks)
    return [f"[{index}/{total}]\n{chunk}" for index, chunk in enumerate(chunks, start=1)]


def get_interaction_command_name(interaction: InteractionCommandSource) -> str:
    command = interaction.command
    return "-" if command is None else str(command.name or "-")
