from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass

import codex_discord_message_gate as message_gate
import codex_discord_message_target as message_target


@dataclass(frozen=True, slots=True)
class PreparedDiscordMessageContent:
    content: str
    target: message_target.DiscordMessageTarget
    bot_bridge_operational_packet: bool = False
    stripped_bot_bridge_prefix_mention: bool = False


@dataclass(frozen=True, slots=True)
class PreparedInboundMessageContent:
    content: str
    target: message_target.DiscordMessageTarget
    handled: bool = False
    empty_content: bool = False


def prepare_discord_message_content(
    content: str,
    target: message_target.DiscordMessageTarget,
    *,
    bot_bridge_mention: bool,
    bridge_user_ids: set[int],
    has_attachments: bool,
) -> PreparedDiscordMessageContent:
    prepared_content = content.strip()
    restart_check_handoff = message_gate.is_bot_bridge_restart_check_handoff_packet(
        message_gate.normalize_bot_bridge_packet_content(prepared_content)
    )
    if bot_bridge_mention and message_gate.is_bot_bridge_operational_packet(
        prepared_content,
        bridge_user_ids,
    ):
        return PreparedDiscordMessageContent(
            content=prepared_content,
            target=target,
            bot_bridge_operational_packet=True,
        )
    stripped_bot_bridge_prefix_mention = False
    if bot_bridge_mention:
        bot_bridge_prefix = message_gate.prepare_bot_bridge_prefix_content(
            prepared_content,
            bridge_user_ids,
        )
        if bot_bridge_prefix.stripped_mention:
            prepared_content = bot_bridge_prefix.content
            stripped_bot_bridge_prefix_mention = True
    target = target.with_explicit_target(
        prepared_content,
        bot_bridge_mention=bot_bridge_mention or restart_check_handoff,
    )
    if not prepared_content and has_attachments:
        prepared_content = message_gate.ATTACHMENT_INSPECTION_PROMPT
    return PreparedDiscordMessageContent(
        content=prepared_content,
        target=target,
        stripped_bot_bridge_prefix_mention=stripped_bot_bridge_prefix_mention,
    )


def prepare_inbound_message_content(
    content: str,
    target: message_target.DiscordMessageTarget,
    *,
    bot_bridge_mention: bool,
    bridge_user_ids: set[int],
    has_attachments: bool,
    channel_id: int | str | None,
    user_id: int | str | None,
    log: Callable[[str], None],
) -> PreparedInboundMessageContent:
    prepared_message_content = prepare_discord_message_content(
        content,
        target,
        bot_bridge_mention=bot_bridge_mention,
        bridge_user_ids=bridge_user_ids,
        has_attachments=has_attachments,
    )
    if prepared_message_content.bot_bridge_operational_packet:
        log(
            f"ignored_message reason=bot_bridge_operational_packet "
            + f"chat={channel_id or '-'} user={user_id or '-'}"
        )
        return PreparedInboundMessageContent(
            content=prepared_message_content.content,
            target=prepared_message_content.target,
            handled=True,
        )
    if prepared_message_content.stripped_bot_bridge_prefix_mention:
        log(f"bot_bridge_prefix_mention_stripped chat={channel_id or '-'} user={user_id or '-'}")
    if not prepared_message_content.content:
        log(f"ignored_message reason=empty_content chat={channel_id or '-'} user={user_id or '-'}")
        return PreparedInboundMessageContent(
            content=prepared_message_content.content,
            target=prepared_message_content.target,
            handled=True,
            empty_content=True,
        )
    return PreparedInboundMessageContent(
        content=prepared_message_content.content,
        target=prepared_message_content.target,
    )
