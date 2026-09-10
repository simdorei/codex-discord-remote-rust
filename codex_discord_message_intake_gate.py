from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Generic, Protocol, TypeVar


ChannelT = TypeVar("ChannelT")
ChannelContraT = TypeVar("ChannelContraT", contravariant=True)
MessageContraT = TypeVar("MessageContraT", contravariant=True)
FormatLogTextLenFunc = Callable[[str], int | str]


class MessageAuthor(Protocol):
    @property
    def id(self) -> int: ...


class DiscordMessageLike(Protocol):
    @property
    def author(self) -> MessageAuthor: ...


MessageT = TypeVar("MessageT", bound=DiscordMessageLike)


class AllowedChannelPredicate(Protocol[ChannelContraT]):
    def __call__(self, channel: ChannelContraT) -> bool: ...


class BotBridgeMentionPredicate(Protocol[MessageContraT]):
    def __call__(self, message: MessageContraT) -> bool: ...


class RestartNoticeSender(Protocol[ChannelContraT]):
    def __call__(self, target: ChannelContraT) -> Awaitable[None]: ...


@dataclass(frozen=True, slots=True)
class MessageIntakeGateDeps(Generic[ChannelT, MessageT]):
    is_allowed_message_channel: AllowedChannelPredicate[ChannelT]
    is_bot_authored_bridge_mention: BotBridgeMentionPredicate[MessageT]
    is_allowed_user: Callable[[int], bool]
    is_stopping: Callable[[], bool]
    send_restarting_notice: RestartNoticeSender[ChannelT]
    log: Callable[[str], None]


@dataclass(frozen=True, slots=True)
class MessageIntakeGateResult:
    handled: bool
    bot_bridge_mention: bool


async def gate_discord_message(
    message: MessageT,
    *,
    message_channel: ChannelT,
    deps: MessageIntakeGateDeps[ChannelT, MessageT],
) -> MessageIntakeGateResult:
    if not deps.is_allowed_message_channel(message_channel):
        parent = getattr(message_channel, "parent", None)
        category = getattr(message_channel, "category", None) or getattr(parent, "category", None)
        deps.log(
            f"ignored_message reason=channel_not_allowed chat={getattr(message_channel, 'id', '-')} "
            + f"parent={getattr(message_channel, 'parent_id', '-')} "
            + f"category={getattr(category, 'name', '-')}"
        )
        return MessageIntakeGateResult(handled=True, bot_bridge_mention=False)
    bot_bridge_mention = deps.is_bot_authored_bridge_mention(message)
    if not deps.is_allowed_user(message.author.id):
        deps.log(f"ignored_message reason=user_not_allowed user={message.author.id}")
        return MessageIntakeGateResult(handled=True, bot_bridge_mention=bot_bridge_mention)
    if deps.is_stopping():
        deps.log(
            f"message_rejected reason=bot_stopping chat={getattr(message_channel, 'id', '-')} "
            + f"user={message.author.id}"
        )
        await deps.send_restarting_notice(message_channel)
        return MessageIntakeGateResult(handled=True, bot_bridge_mention=bot_bridge_mention)
    return MessageIntakeGateResult(handled=False, bot_bridge_mention=bot_bridge_mention)


async def gate_inbound_discord_message(
    message: MessageT,
    *,
    message_channel: ChannelT,
    source: str,
    enable_prefix_commands: bool,
    deps: MessageIntakeGateDeps[ChannelT, MessageT],
    format_log_text_len: FormatLogTextLenFunc,
    log: Callable[[str], None],
) -> MessageIntakeGateResult:
    content = str(getattr(message, "content", "") or "")
    log(
        f"message_received chat={getattr(message_channel, 'id', '-')} "
        + f"parent={getattr(message_channel, 'parent_id', '-')} "
        + f"user={message.author.id} content_len={format_log_text_len(content)} source={source}"
    )
    if not enable_prefix_commands:
        log("ignored_message reason=message_content_disabled")
        return MessageIntakeGateResult(handled=True, bot_bridge_mention=False)
    return await gate_discord_message(
        message,
        message_channel=message_channel,
        deps=deps,
    )
