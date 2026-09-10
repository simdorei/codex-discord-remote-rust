from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import TypeAlias

import codex_discord_message_dispatch as message_dispatch
import codex_discord_message_intake_gate as message_intake_gate
import codex_discord_message_target as message_target

ExceptionTypes: TypeAlias = tuple[type[BaseException], ...]


@dataclass(frozen=True, slots=True)
class InboundMessageRuntimeDeps:
    require_messageable_channel: message_dispatch.MessageableChannelResolver
    is_allowed_message_channel: message_intake_gate.AllowedChannelPredicate[
        message_dispatch.DispatchChannel
    ]
    is_bot_authored_bridge_mention: message_dispatch.BotBridgeMentionPredicate
    is_allowed_user: Callable[[int], bool]
    is_stopping: Callable[[], bool]
    send_restarting_notice: message_dispatch.RestartNoticeSender
    get_mirrored_codex_thread_id: message_target.MirrorThreadLookup
    get_bridge_mention_user_ids: Callable[[], set[int]]
    maybe_send_empty_content_notice: Callable[[message_dispatch.InboundMessage], Awaitable[None]]
    prepare_plain_ask_message_content: message_dispatch.PlainAskContentPreparer
    persist_inbound_mirror_thread_channel: message_dispatch.MirrorChannelPersister
    handle_prefix_command: message_dispatch.PrefixCommandHandler
    describe_mirrored_project_channel: message_dispatch.ProjectChannelDescriber
    send_chunks: message_dispatch.ChunkSender
    handle_plain_ask: message_dispatch.PlainAskHandler
    format_log_text_len: message_dispatch.FormatLogTextLenFunc
    delivery_rejected_type: type[BaseException]
    delivery_exceptions: ExceptionTypes
    format_exception: Callable[[], str]
    log: Callable[[str], None]


async def process_inbound_discord_message_safely(
    message: message_dispatch.InboundMessage,
    *,
    source: str,
    enable_prefix_commands: bool,
    deps: InboundMessageRuntimeDeps,
) -> None:
    try:
        await message_dispatch.process_inbound_discord_message(
            message,
            source=source,
            enable_prefix_commands=enable_prefix_commands,
            deps=_make_process_deps(deps),
        )
    except deps.delivery_rejected_type:
        deps.log("on_message_delivery_rejected\n" + deps.format_exception())
    except deps.delivery_exceptions:
        deps.log("on_message_error\n" + deps.format_exception())
        try:
            _ = await deps.send_chunks(
                deps.require_messageable_channel(message.channel),
                "Discord bot error. Check codex_discord_bot.log.",
            )
        except deps.delivery_exceptions:
            deps.log("on_message_error_report_failed\n" + deps.format_exception())


def _make_process_deps(deps: InboundMessageRuntimeDeps) -> message_dispatch.InboundDiscordMessageProcessDeps:
    return message_dispatch.InboundDiscordMessageProcessDeps(
        require_messageable_channel=deps.require_messageable_channel,
        is_allowed_message_channel=deps.is_allowed_message_channel,
        is_bot_authored_bridge_mention=deps.is_bot_authored_bridge_mention,
        is_allowed_user=deps.is_allowed_user,
        is_stopping=deps.is_stopping,
        send_restarting_notice=deps.send_restarting_notice,
        get_mirrored_codex_thread_id=deps.get_mirrored_codex_thread_id,
        get_bridge_mention_user_ids=deps.get_bridge_mention_user_ids,
        maybe_send_empty_content_notice=deps.maybe_send_empty_content_notice,
        prepare_plain_ask_message_content=deps.prepare_plain_ask_message_content,
        persist_inbound_mirror_thread_channel=deps.persist_inbound_mirror_thread_channel,
        handle_prefix_command=deps.handle_prefix_command,
        describe_mirrored_project_channel=deps.describe_mirrored_project_channel,
        send_chunks=deps.send_chunks,
        handle_plain_ask=deps.handle_plain_ask,
        format_log_text_len=deps.format_log_text_len,
        log=deps.log,
    )
