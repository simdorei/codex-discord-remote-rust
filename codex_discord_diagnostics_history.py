from __future__ import annotations

from collections.abc import AsyncIterable, Awaitable, Callable, Sequence
from typing import Protocol, TypeAlias, cast, runtime_checkable


class DiagnosticsHistoryBot(Protocol):
    pass


class DiscordChannelLike(Protocol):
    @property
    def id(self) -> int | str: ...


class DiscordAuthorLike(Protocol):
    bot: bool
    id: int | str


class DiscordMessageCreatedAtLike(Protocol):
    def isoformat(self) -> str: ...


class DiscordMessageTypeLike(Protocol):
    name: str


DiscordMessageTypeValue: TypeAlias = DiscordMessageTypeLike | str | int | None
DiscordMessageContent: TypeAlias = str | None
DiscordChannelSourceValue: TypeAlias = DiscordChannelLike | str | int | None


class DiscordHistoryMessage(Protocol):
    author: DiscordAuthorLike | None
    content: DiscordMessageContent
    created_at: DiscordMessageCreatedAtLike | None
    type: DiscordMessageTypeValue


@runtime_checkable
class DiscordHistoryChannel(Protocol):
    def history(self, *, limit: int) -> AsyncIterable[DiscordHistoryMessage]: ...


LogTextLengthFormatter: TypeAlias = Callable[[str | None], int]
CachedChannelGetter: TypeAlias = Callable[
    [int], tuple[DiscordChannelLike | None, DiscordChannelSourceValue] | DiscordChannelLike | None
]
ClientChannelGetter: TypeAlias = Callable[[int], DiscordChannelLike | None]
FetchChannelGetter: TypeAlias = Callable[
    [int], DiscordChannelLike | Awaitable[DiscordChannelLike | None] | None
]


class StartupProbeTargetsGetter(Protocol):
    def __call__(
        self,
        allowed_channel_ids: set[int],
        startup_channel_id: int | None,
        *,
        limit: int = 30,
    ) -> Sequence[tuple[str, int]]: ...


def _bot_int_set_attr(bot: DiagnosticsHistoryBot, name: str) -> set[int]:
    return cast(set[int], getattr(bot, name, set[int]()))


def format_discord_message_type(message: DiscordHistoryMessage) -> str:
    message_type = getattr(message, "type", "-")
    return str(getattr(message_type, "name", message_type) or "-")


def format_discord_message_created_at(message: DiscordHistoryMessage) -> str:
    created_at = getattr(message, "created_at", None)
    isoformat = getattr(created_at, "isoformat", None)
    if callable(isoformat):
        return str(isoformat())
    return "-"


async def build_discord_channel_history_lines(
    channel: DiscordChannelLike | None,
    *,
    limit: int = 5,
    format_log_text_len_func: LogTextLengthFormatter,
) -> list[str]:
    lines = ["Recent channel history:"]
    if not isinstance(channel, DiscordHistoryChannel):
        return [*lines, "history_unavailable: no_channel"]
    try:
        messages: list[str] = []
        async for message in channel.history(limit=limit):
            author = getattr(message, "author", None)
            content = cast(str | None, getattr(message, "content", "") or "")
            messages.append(
                f"{format_discord_message_created_at(message)} "
                + f"bot={bool(getattr(author, 'bot', False))} "
                + f"content_len={format_log_text_len_func(content)} "
                + f"type={format_discord_message_type(message)}"
            )
    except Exception as exc:  # noqa: BROAD_EXCEPT_OK
        return [*lines, f"history_error: {type(exc).__name__}"]
    return [*lines, *(messages or ["-"])]


async def resolve_discord_history_channel(
    bot: DiagnosticsHistoryBot, channel_id: int
) -> tuple[DiscordChannelLike | None, str]:
    getter = getattr(bot, "get_cached_channel_or_thread", None)
    if callable(getter):
        cached_result = cast(CachedChannelGetter, getter)(channel_id)
        if isinstance(cached_result, tuple):
            try:
                channel_candidate, source_value = cached_result
            except ValueError:
                channel = None
                source = "-"
            else:
                channel = channel_candidate
                source = str(source_value)
        else:
            channel = None
            source = "-"
    else:
        channel = None
        source = "-"
        get_channel = getattr(bot, "get_channel", None)
        if callable(get_channel):
            channel = cast(ClientChannelGetter, get_channel)(channel_id)
            source = "client_channel_cache" if channel is not None else "-"
    if channel is None:
        fetch_channel = getattr(bot, "fetch_channel", None)
        if callable(fetch_channel):
            try:
                fetched = cast(FetchChannelGetter, fetch_channel)(channel_id)
                if isinstance(fetched, Awaitable):
                    channel = await fetched
                else:
                    channel = fetched
                source = "fetch"
            except Exception as exc:  # noqa: BROAD_EXCEPT_OK
                return None, f"fetch_error:{type(exc).__name__}"
    return channel, source


async def build_discord_tracked_target_user_history_lines(
    bot: DiagnosticsHistoryBot,
    *,
    get_startup_probe_targets_func: StartupProbeTargetsGetter,
    format_log_text_len_func: LogTextLengthFormatter,
    per_target_limit: int = 5,
    target_limit: int = 50,
) -> list[str]:
    lines = ["Recent tracked target user history:"]
    targets = get_startup_probe_targets_func(
        _bot_int_set_attr(bot, "allowed_channel_ids"),
        cast(int | None, getattr(bot, "startup_channel_id", None)),
        limit=target_limit,
    )
    if not targets:
        return [*lines, "-"]
    for label, channel_id in targets:
        channel, source = await resolve_discord_history_channel(bot, channel_id)
        prefix = f"{label} channel={channel_id} source={source}"
        if not isinstance(channel, DiscordHistoryChannel):
            lines.append(f"{prefix} latest_user=-")
            continue
        latest_user_message = None
        try:
            async for message in channel.history(limit=per_target_limit):
                if getattr(getattr(message, "author", None), "bot", False):
                    continue
                latest_user_message = message
                break
        except Exception as exc:  # noqa: BROAD_EXCEPT_OK
            lines.append(f"{prefix} latest_user=history_error:{type(exc).__name__}")
            continue
        if latest_user_message is None:
            lines.append(f"{prefix} latest_user=-")
            continue
        author = getattr(latest_user_message, "author", None)
        content = cast(str | None, getattr(latest_user_message, "content", "") or "")
        lines.append(
            f"{prefix} latest_user_at={format_discord_message_created_at(latest_user_message)} "
            + f"user={getattr(author, 'id', '-')} "
            + f"content_len={format_log_text_len_func(content)} "
            + f"type={format_discord_message_type(latest_user_message)}"
        )
    return lines
