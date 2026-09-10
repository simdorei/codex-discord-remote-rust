from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Protocol

import codex_discord_message_content as message_content
import codex_discord_message_intake_gate as message_intake_gate
import codex_discord_message_target as message_target


FormatLogTextLenFunc = Callable[[str], int | str]
MirrorChannelPersister = Callable[[str, int], None]
ProjectChannelDescriber = Callable[[int], str | None]


class DispatchAuthor(Protocol):
    @property
    def id(self) -> int: ...


class DispatchChannel(Protocol):
    @property
    def id(self) -> int: ...


class DispatchMessage(Protocol):
    @property
    def author(self) -> DispatchAuthor: ...

    @property
    def channel(self) -> DispatchChannel: ...


class InboundMessageChannel(Protocol):
    @property
    def id(self) -> int: ...


class InboundMessage(Protocol):
    @property
    def author(self) -> DispatchAuthor: ...

    @property
    def channel(self) -> InboundMessageChannel: ...

    @property
    def content(self) -> str: ...


class PrefixCommandHandler(Protocol):
    def __call__(self, message: DispatchMessage, command: str) -> Awaitable[None]: ...


class PlainAskHandler(Protocol):
    def __call__(
        self,
        message: DispatchMessage,
        content: str,
        *,
        target_thread_id: str | None = None,
    ) -> Awaitable[None]: ...


class ChunkSender(Protocol):
    def __call__(self, target: DispatchChannel, text: str) -> Awaitable[int]: ...


class MessageableChannelResolver(Protocol):
    def __call__(self, channel: InboundMessageChannel) -> DispatchChannel: ...


class BotBridgeMentionPredicate(Protocol):
    def __call__(self, message: InboundMessage) -> bool: ...


class RestartNoticeSender(Protocol):
    def __call__(self, target: DispatchChannel) -> Awaitable[None]: ...


class PlainAskContentPreparer(Protocol):
    def __call__(
        self,
        message: InboundMessage,
        content: str,
        target_thread_id: str | None,
        *,
        has_attachments: bool,
    ) -> Awaitable[str | None]: ...


@dataclass(frozen=True, slots=True)
class InboundDiscordMessageProcessDeps:
    require_messageable_channel: MessageableChannelResolver
    is_allowed_message_channel: message_intake_gate.AllowedChannelPredicate[DispatchChannel]
    is_bot_authored_bridge_mention: BotBridgeMentionPredicate
    is_allowed_user: Callable[[int], bool]
    is_stopping: Callable[[], bool]
    send_restarting_notice: RestartNoticeSender
    get_mirrored_codex_thread_id: message_target.MirrorThreadLookup
    get_bridge_mention_user_ids: Callable[[], set[int]]
    maybe_send_empty_content_notice: Callable[[InboundMessage], Awaitable[None]]
    prepare_plain_ask_message_content: PlainAskContentPreparer
    persist_inbound_mirror_thread_channel: MirrorChannelPersister
    handle_prefix_command: PrefixCommandHandler
    describe_mirrored_project_channel: ProjectChannelDescriber
    send_chunks: ChunkSender
    handle_plain_ask: PlainAskHandler
    format_log_text_len: FormatLogTextLenFunc
    log: Callable[[str], None]


@dataclass(frozen=True, slots=True)
class PreparedMessageDispatchDeps:
    format_log_text_len: FormatLogTextLenFunc
    persist_inbound_mirror_thread_channel: MirrorChannelPersister
    handle_prefix_command: PrefixCommandHandler
    describe_mirrored_project_channel: ProjectChannelDescriber
    send_chunks: ChunkSender
    handle_plain_ask: PlainAskHandler
    log: Callable[[str], None]


async def dispatch_prepared_message(
    message: DispatchMessage,
    content: str,
    target: message_target.DiscordMessageTarget,
    *,
    deps: PreparedMessageDispatchDeps,
) -> None:
    target_thread_id = target.target_thread_id
    target_source = target.target_source
    channel_id = message.channel.id
    deps.log(
        f"message chat={channel_id} user={message.author.id} "
        + f"prefix={content.startswith('!')} "
        + f"target_source={target_source} target={target_thread_id or '-'} "
        + f"text_len={deps.format_log_text_len(content)}"
    )
    if (
        target_source == "mirror"
        and target_thread_id is not None
        and target.persist_mirror_channel
    ):
        deps.persist_inbound_mirror_thread_channel(target_thread_id, int(channel_id))
        deps.log(f"inbound_mirror_channel_persisted target={target_thread_id} channel={channel_id}")
    if content.startswith("!"):
        await deps.handle_prefix_command(message, content[1:].strip())
        return
    if target_thread_id is None:
        project_message = deps.describe_mirrored_project_channel(channel_id)
        if project_message:
            _ = await deps.send_chunks(message.channel, project_message)
            return
    await deps.handle_plain_ask(message, content, target_thread_id=target_thread_id)


async def process_inbound_discord_message(
    message: InboundMessage,
    *,
    source: str,
    enable_prefix_commands: bool,
    deps: InboundDiscordMessageProcessDeps,
) -> None:
    message_channel = deps.require_messageable_channel(message.channel)
    intake_deps: message_intake_gate.MessageIntakeGateDeps[
        DispatchChannel,
        InboundMessage,
    ] = message_intake_gate.MessageIntakeGateDeps(
        is_allowed_message_channel=deps.is_allowed_message_channel,
        is_bot_authored_bridge_mention=deps.is_bot_authored_bridge_mention,
        is_allowed_user=deps.is_allowed_user,
        is_stopping=deps.is_stopping,
        send_restarting_notice=deps.send_restarting_notice,
        log=deps.log,
    )
    intake_gate = await message_intake_gate.gate_inbound_discord_message(
        message,
        message_channel=message_channel,
        source=source,
        enable_prefix_commands=enable_prefix_commands,
        deps=intake_deps,
        format_log_text_len=deps.format_log_text_len,
        log=deps.log,
    )
    if intake_gate.handled:
        return
    content = message.content or ""
    has_attachments = bool(getattr(message, "attachments", None))
    dispatch_deps = PreparedMessageDispatchDeps(
        format_log_text_len=deps.format_log_text_len,
        persist_inbound_mirror_thread_channel=deps.persist_inbound_mirror_thread_channel,
        handle_prefix_command=deps.handle_prefix_command,
        describe_mirrored_project_channel=deps.describe_mirrored_project_channel,
        send_chunks=deps.send_chunks,
        handle_plain_ask=deps.handle_plain_ask,
        log=deps.log,
    )
    if content.strip().startswith("!") or intake_gate.bot_bridge_mention:
        selected_target = message_target.DiscordMessageTarget(None, "selected")
        prepared_prefix_content = message_content.prepare_inbound_message_content(
            content,
            selected_target,
            bot_bridge_mention=intake_gate.bot_bridge_mention,
            bridge_user_ids=deps.get_bridge_mention_user_ids(),
            has_attachments=has_attachments,
            channel_id=message.channel.id,
            user_id=message.author.id,
            log=deps.log,
        )
        if prepared_prefix_content.handled:
            if prepared_prefix_content.empty_content:
                await deps.maybe_send_empty_content_notice(message)
            return
        if prepared_prefix_content.content.startswith("!"):
            await dispatch_prepared_message(
                message,
                prepared_prefix_content.content,
                prepared_prefix_content.target,
                deps=dispatch_deps,
            )
            return
    resolved_target = message_target.resolve_discord_message_target(
        deps.get_mirrored_codex_thread_id,
        message.channel.id,
        getattr(message.channel, "parent_id", None),
    )
    prepared_message_content = message_content.prepare_inbound_message_content(
        content,
        resolved_target,
        bot_bridge_mention=intake_gate.bot_bridge_mention,
        bridge_user_ids=deps.get_bridge_mention_user_ids(),
        has_attachments=has_attachments,
        channel_id=message.channel.id,
        user_id=message.author.id,
        log=deps.log,
    )
    if prepared_message_content.handled:
        if prepared_message_content.empty_content:
            await deps.maybe_send_empty_content_notice(message)
        return
    content = prepared_message_content.content
    resolved_target = prepared_message_content.target
    target_thread_id = resolved_target.target_thread_id
    if not content.startswith("!"):
        prepared_content = await deps.prepare_plain_ask_message_content(
            message,
            content,
            target_thread_id,
            has_attachments=has_attachments,
        )
        if prepared_content is None:
            return
        content = prepared_content
    await dispatch_prepared_message(
        message,
        content,
        resolved_target,
        deps=dispatch_deps,
    )
