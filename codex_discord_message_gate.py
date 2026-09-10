from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from enum import StrEnum, unique
from typing import Final, Generic, Protocol, TypeVar, cast

from codex_discord_message_gate_context import (
    DEFAULT_PLAIN_ASK_CONTEXT_KEYWORDS as DEFAULT_PLAIN_ASK_CONTEXT_KEYWORDS,
    get_plain_ask_context_keywords as get_plain_ask_context_keywords,
    plain_ask_context_fallback_enabled as plain_ask_context_fallback_enabled,
    plain_ask_context_matches as plain_ask_context_matches,
    should_accept_plain_ask_without_required_mention as should_accept_plain_ask_without_required_mention,
)
from codex_discord_message_gate_packets import (
    BOT_BRIDGE_OPERATIONAL_PACKET_PREFIXES as BOT_BRIDGE_OPERATIONAL_PACKET_PREFIXES,
    is_bot_bridge_operational_packet as is_bot_bridge_operational_packet,
    is_bot_bridge_restart_check_handoff_packet as is_bot_bridge_restart_check_handoff_packet,
    normalize_bot_bridge_packet_content as normalize_bot_bridge_packet_content,
)
from codex_discord_message_mentions import (
    DISCORD_USER_MENTION_RE as DISCORD_USER_MENTION_RE,
    BotMessageWithMentions as BotMessageWithMentions,
    DiscordClientWithMentions as DiscordClientWithMentions,
    DiscordIdValue as DiscordIdValue,
    MessageWithMentions as MessageWithMentions,
    get_bridge_mention_user_ids as get_bridge_mention_user_ids,
    get_discord_message_mention_ids as get_discord_message_mention_ids,
    is_bot_authored_bridge_mention as is_bot_authored_bridge_mention,
    message_mentions_bridge_user as message_mentions_bridge_user,
    message_mentions_other_bot as message_mentions_other_bot,
    message_mentions_required_plain_ask_user as message_mentions_required_plain_ask_user,
    strip_required_plain_ask_mentions as strip_required_plain_ask_mentions,
)

ATTACHMENT_INSPECTION_PROMPT: Final = "Please inspect the attached Discord file(s)."
MessageT = TypeVar("MessageT")
MessageContraT = TypeVar("MessageContraT", contravariant=True)


class GatewayMessageProcessor(Protocol[MessageContraT]):
    def __call__(self, message: MessageContraT, *, source: str) -> Awaitable[None]: ...


@dataclass(frozen=True, slots=True)
class GatewayMessageDeps(Generic[MessageT]):
    discord_client: DiscordClientWithMentions
    claim_message: Callable[[MessageT], bool]
    get_message_id: Callable[[MessageT], DiscordIdValue]
    process_message: GatewayMessageProcessor[MessageT]
    mark_processed: Callable[[MessageT], None]
    log: Callable[[str], None]


@unique
class PlainAskGateAction(StrEnum):
    ACCEPT = "accept"
    REQUIRED_MENTION_MISSING = "required_mention_missing"
    MENTION_ONLY_CONTENT = "mention_only_content"
    OTHER_BOT_MENTION_IN_MIRRORED_THREAD = "other_bot_mention_in_mirrored_thread"


@dataclass(frozen=True, slots=True)
class PlainAskGateResult:
    content: str
    matched_mention: bool
    action: PlainAskGateAction
    context_fallback: bool = False


@dataclass(frozen=True, slots=True)
class BotBridgePrefixContentResult:
    content: str
    stripped_mention: bool


def prepare_bot_bridge_prefix_content(
    content: str,
    bridge_user_ids: set[int],
) -> BotBridgePrefixContentResult:
    if not content.startswith("!"):
        return BotBridgePrefixContentResult(content=content, stripped_mention=False)
    stripped_content, matched_bridge_mention = strip_required_plain_ask_mentions(
        content,
        bridge_user_ids,
    )
    if matched_bridge_mention and stripped_content:
        return BotBridgePrefixContentResult(content=stripped_content, stripped_mention=True)
    return BotBridgePrefixContentResult(content=content, stripped_mention=False)


def should_process_gateway_message_author(
    message: BotMessageWithMentions,
    discord_client: DiscordClientWithMentions,
    log: Callable[[str], None],
) -> bool:
    author = getattr(message, "author", None)
    if not bool(getattr(author, "bot", False)):
        return True
    author_id = cast(DiscordIdValue, getattr(author, "id", None))
    author_id_text = "" if author_id is None else str(author_id)
    self_user_id = cast(DiscordIdValue, getattr(getattr(discord_client, "user", None), "id", None))
    if self_user_id is not None and author_id_text == str(self_user_id):
        return False
    if message_mentions_bridge_user(message, discord_client):
        return True
    channel_id = getattr(getattr(message, "channel", None), "id", "-")
    content = str(getattr(message, "content", "") or "")
    if is_bot_bridge_restart_check_handoff_packet(normalize_bot_bridge_packet_content(content)):
        log(
            f"bot_bridge_unmentioned_restart_check_handoff_accepted "
            + f"chat={channel_id} user={author_id or '-'}"
        )
        return True
    log(
        f"ignored_message reason=bot_author_without_bridge_mention chat={channel_id} user={author_id or '-'}"
    )
    return False


async def process_gateway_message(message: MessageT, *, deps: GatewayMessageDeps[MessageT]) -> None:
    if not should_process_gateway_message_author(
        cast(BotMessageWithMentions, message),
        deps.discord_client,
        deps.log,
    ):
        return
    if not deps.claim_message(message):
        deps.log(
            f"duplicate_message_skipped source=gateway "
            + f"chat={getattr(getattr(message, 'channel', None), 'id', '-')} "
            + f"message={deps.get_message_id(message) or '-'}"
        )
        return
    await deps.process_message(message, source="gateway")
    deps.mark_processed(message)


def prepare_plain_ask_content(
    message: MessageWithMentions,
    content: str,
    required_user_ids: set[int],
    target_thread_id: str | None,
    *,
    has_attachments: bool,
) -> PlainAskGateResult:
    prepared_content = content
    matched_mention = False
    context_fallback = False

    if required_user_ids:
        stripped_content, matched_mention = strip_required_plain_ask_mentions(
            prepared_content,
            required_user_ids,
        )
        if matched_mention:
            prepared_content = stripped_content
        if not matched_mention and target_thread_id is None:
            matched_mention = message_mentions_required_plain_ask_user(
                message,
                required_user_ids,
            )
        if (
            not matched_mention
            and target_thread_id is None
            and should_accept_plain_ask_without_required_mention(prepared_content)
        ):
            context_fallback = True
            matched_mention = True
        if not matched_mention and target_thread_id is None:
            return PlainAskGateResult(
                content=prepared_content,
                matched_mention=False,
                action=PlainAskGateAction.REQUIRED_MENTION_MISSING,
            )
        if matched_mention and not prepared_content:
            if has_attachments:
                prepared_content = ATTACHMENT_INSPECTION_PROMPT
            else:
                return PlainAskGateResult(
                    content=prepared_content,
                    matched_mention=True,
                    action=PlainAskGateAction.MENTION_ONLY_CONTENT,
                )

    if (
        target_thread_id is not None
        and not matched_mention
        and message_mentions_other_bot(message, required_user_ids)
    ):
        return PlainAskGateResult(
            content=prepared_content,
            matched_mention=False,
            action=PlainAskGateAction.OTHER_BOT_MENTION_IN_MIRRORED_THREAD,
        )

    return PlainAskGateResult(
        content=prepared_content,
        matched_mention=matched_mention,
        action=PlainAskGateAction.ACCEPT,
        context_fallback=context_fallback,
    )
