from __future__ import annotations

from asyncio import CancelledError  # noqa: ANYIO_OK
from collections.abc import AsyncIterator, Awaitable, Callable
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Generic, Protocol, TypeVar, cast

import codex_discord_diagnostics_history as discord_diagnostics_history


MessageT = TypeVar("MessageT")
HistoryMessageT_co = TypeVar("HistoryMessageT_co", covariant=True)
HistoryMessageWatermark = tuple[datetime, int]
GetCachedHistoryChannel = Callable[[int], tuple[object | None, str]]
FetchHistoryChannel = Callable[[int], Awaitable[object | None]]
ClaimHistoryMessage = Callable[[MessageT], bool]
ProcessHistoryPollMessage = Callable[[MessageT, int], Awaitable[None]]
LogFunc = Callable[[str], None]
FormatLogTextLenFunc = Callable[[str], int | str]
PollHistoryChannelRunner = Callable[[str, int], Awaitable[None]]
SleepFunc = Callable[[float], Awaitable[None]]
SetLastAtFunc = Callable[[str], None]
NowIsoFunc = Callable[[], str]
TracebackFormatter = Callable[[], str]


class HistoryPollChannel(Protocol):
    @property
    def id(self) -> int | str | None: ...


class HistoryPollMessage(discord_diagnostics_history.DiscordHistoryMessage, Protocol):
    @property
    def channel(self) -> HistoryPollChannel | None: ...


class HistorySource(Protocol[HistoryMessageT_co]):
    def history(self, *, limit: int) -> AsyncIterator[HistoryMessageT_co]: ...


class HistoryPollTargetsGetter(Protocol):
    def __call__(
        self,
        allowed_channel_ids: set[int],
        startup_channel_id: int | None,
        *,
        limit: int = 50,
    ) -> list[tuple[str, int]]: ...


@dataclass(frozen=True, slots=True)
class PollHistoryChannelDeps(Generic[MessageT]):
    get_cached_channel_or_thread: GetCachedHistoryChannel
    fetch_channel: FetchHistoryChannel
    delivery_exceptions: tuple[type[BaseException], ...]
    history_limit: int
    is_primed_channel: Callable[[int], bool]
    mark_primed_channel: Callable[[int], None]
    get_channel_watermark: Callable[[int], HistoryMessageWatermark | None]
    set_channel_watermark: Callable[[int, HistoryMessageWatermark], None]
    now: Callable[[], datetime]
    claim_message: ClaimHistoryMessage[MessageT]
    process_history_poll_message: ProcessHistoryPollMessage[MessageT]
    log: LogFunc


@dataclass(frozen=True, slots=True)
class HistoryPollLoopDeps:
    allowed_channel_ids: set[int]
    startup_channel_id: int | None
    poll_seconds: float
    target_limit: int
    is_closed: Callable[[], bool]
    set_last_at: SetLastAtFunc
    now_iso: NowIsoFunc
    get_targets: HistoryPollTargetsGetter
    poll_history_channel: PollHistoryChannelRunner
    delivery_exceptions: tuple[type[BaseException], ...]
    format_traceback: TracebackFormatter
    sleep: SleepFunc
    log: LogFunc


def should_process_history_poll_message(message: discord_diagnostics_history.DiscordHistoryMessage) -> bool:
    author = message.author
    return author is None or not author.bot


async def poll_history_channel(
    label: str,
    channel_id: int,
    *,
    deps: PollHistoryChannelDeps[MessageT],
) -> None:
    poll_started_watermark = (_normalize_datetime(deps.now()), 0)
    channel, source = deps.get_cached_channel_or_thread(channel_id)
    if channel is None:
        try:
            channel = await deps.fetch_channel(channel_id)
            source = "fetch"
        except deps.delivery_exceptions as exc:
            deps.log(
                f"history_poll_channel_failed label={label} channel={channel_id} "
                + f"error_type={type(exc).__name__}"
            )
            return
    if not callable(getattr(channel, "history", None)):
        deps.log(f"history_poll_channel_skipped label={label} channel={channel_id} reason=no_history")
        return

    history_channel = cast(HistorySource[MessageT], channel)
    is_primed = deps.is_primed_channel(int(channel_id))
    history_messages: list[MessageT] = []
    claimed_messages: list[MessageT] = []
    try:
        async for message in history_channel.history(limit=deps.history_limit):
            history_messages.append(message)
            if deps.claim_message(message):
                claimed_messages.append(message)
    except deps.delivery_exceptions as exc:
        deps.log(
            f"history_poll_channel_failed label={label} channel={channel_id} "
            + f"source={source} error_type={type(exc).__name__}"
        )
        return
    latest_watermark = _latest_message_watermark(history_messages)
    if not is_primed:
        deps.mark_primed_channel(int(channel_id))
        eligible_messages = _messages_after_watermark(claimed_messages, poll_started_watermark)
        prime_watermark = poll_started_watermark
        if latest_watermark is not None:
            prime_watermark = max(prime_watermark, latest_watermark)
        deps.set_channel_watermark(int(channel_id), prime_watermark)
        deps.log(
            f"history_poll_primed label={label} channel={channel_id} "
            + f"source={source} fetched_messages={len(history_messages)} "
            + f"claimed_messages={len(claimed_messages)} "
            + f"eligible_messages={len(eligible_messages)} "
            + f"discarded_messages={len(claimed_messages) - len(eligible_messages)}"
        )
        for message in eligible_messages:
            await deps.process_history_poll_message(message, channel_id)
        return

    watermark = deps.get_channel_watermark(int(channel_id))
    if watermark is None:
        reprime_watermark = poll_started_watermark
        if latest_watermark is not None:
            reprime_watermark = max(reprime_watermark, latest_watermark)
        deps.set_channel_watermark(int(channel_id), reprime_watermark)
        deps.log(
            f"history_poll_reprimed label={label} channel={channel_id} "
            + f"source={source} reason=missing_watermark "
            + f"fetched_messages={len(history_messages)} "
            + f"claimed_messages={len(claimed_messages)} "
            + f"discarded_messages={len(claimed_messages)}"
        )
        return
    if latest_watermark is not None and latest_watermark > watermark:
        deps.set_channel_watermark(int(channel_id), latest_watermark)
    eligible_messages = _messages_after_watermark(claimed_messages, watermark)
    for message in eligible_messages:
        await deps.process_history_poll_message(message, channel_id)


def _latest_message_watermark(messages: list[MessageT]) -> HistoryMessageWatermark | None:
    watermarks = [value for message in messages if (value := message_watermark(message))]
    return max(watermarks, default=None)


def _is_message_after_watermark(message: object, watermark: HistoryMessageWatermark | None) -> bool:
    current_watermark = message_watermark(message)
    if current_watermark is None:
        return False
    return watermark is None or current_watermark > watermark


def _messages_after_watermark(
    messages: list[MessageT],
    watermark: HistoryMessageWatermark,
) -> list[MessageT]:
    return [
        message
        for message in reversed(messages)
        if _is_message_after_watermark(message, watermark)
    ]


def message_watermark(message: object) -> HistoryMessageWatermark | None:
    created_at = _message_created_at(message)
    raw_id = getattr(message, "id", None)
    if raw_id is None:
        return None
    try:
        message_id = int(raw_id)
    except (TypeError, ValueError):
        return None
    if created_at is None:
        return None
    return created_at, message_id


def _message_created_at(message: object) -> datetime | None:
    created_at = getattr(message, "created_at", None)
    if not isinstance(created_at, datetime):
        return None
    return _normalize_datetime(created_at)


def _normalize_datetime(value: datetime) -> datetime:
    created_at = value
    if created_at.tzinfo is None:
        return created_at.replace(tzinfo=timezone.utc)
    return created_at.astimezone(timezone.utc)


async def history_poll_loop(deps: HistoryPollLoopDeps) -> None:
    while not deps.is_closed():
        try:
            deps.set_last_at(deps.now_iso())
            targets = deps.get_targets(
                deps.allowed_channel_ids,
                deps.startup_channel_id,
                limit=deps.target_limit,
            )
            for label, channel_id in targets:
                await deps.poll_history_channel(label, channel_id)
        except CancelledError:
            raise
        except deps.delivery_exceptions:
            deps.log("history_poll_cycle_failed\n" + deps.format_traceback())
        await deps.sleep(deps.poll_seconds)


def format_history_poll_message_log(
    message: HistoryPollMessage,
    channel_id: int,
    *,
    format_log_text_len: FormatLogTextLenFunc,
) -> str:
    channel = message.channel
    author = message.author
    resolved_channel_id = channel_id if channel is None or channel.id is None else channel.id
    author_id = "-" if author is None else author.id
    content = message.content or ""
    return (
        f"history_poll_message channel={resolved_channel_id} "
        f"user={author_id} content_len={format_log_text_len(content)}"
    )
