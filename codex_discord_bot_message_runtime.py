from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Protocol, cast

import codex_discord_message_dispatch as discord_message_dispatch
import codex_discord_message_dispatch_runtime as discord_message_dispatch_runtime
import codex_discord_message_gate as discord_message_gate
import codex_discord_plain_ask as discord_plain_ask


class MessageRuntimeOwner(discord_message_gate.DiscordClientWithMentions, Protocol):
    enable_prefix_commands: bool

    def is_allowed_message_channel(
        self,
        channel: discord_message_dispatch.DispatchChannel,
    ) -> bool: ...

    def is_allowed_user(self, user_id: int | None) -> bool: ...


class OwnerPrefixCommandHandler(Protocol):
    def __call__(
        self,
        owner: MessageRuntimeOwner,
        message: discord_message_dispatch.DispatchMessage,
        command: str,
    ) -> Awaitable[None]: ...


@dataclass(frozen=True, slots=True)
class BotMessageRuntimeDeps:
    require_messageable_channel: discord_message_dispatch.MessageableChannelResolver
    is_stopping: Callable[[], bool]
    send_restarting_notice: discord_message_dispatch.RestartNoticeSender
    get_mirrored_codex_thread_id: Callable[[int | None], str | None]
    maybe_send_empty_content_notice: Callable[
        [discord_message_dispatch.InboundMessage],
        Awaitable[None],
    ]
    make_plain_ask_message_content_deps: Callable[
        [],
        discord_plain_ask.PlainAskMessageContentDeps[
            discord_plain_ask.PlainAskPreparedMessage,
            int,
        ],
    ]
    persist_inbound_mirror_thread_channel: discord_message_dispatch.MirrorChannelPersister
    handle_prefix_command: OwnerPrefixCommandHandler
    describe_mirrored_project_channel: discord_message_dispatch.ProjectChannelDescriber
    send_chunks: discord_message_dispatch.ChunkSender
    handle_plain_ask: discord_message_dispatch.PlainAskHandler
    format_log_text_len: discord_message_dispatch.FormatLogTextLenFunc
    delivery_rejected_type: type[BaseException]
    delivery_exceptions: tuple[type[BaseException], ...]
    format_exception: Callable[[], str]
    log: Callable[[str], None]


@dataclass(frozen=True, slots=True)
class BotMessageRuntime:
    deps: BotMessageRuntimeDeps

    async def process_discord_message(
        self,
        owner: MessageRuntimeOwner,
        message: discord_message_dispatch.InboundMessage,
        *,
        source: str,
    ) -> None:
        def is_bot_authored_bridge_mention(
            message: discord_message_dispatch.InboundMessage,
        ) -> bool:
            return discord_message_gate.is_bot_authored_bridge_mention(
                cast(discord_message_gate.BotMessageWithMentions, cast(object, message)),
                owner,
            )

        async def prepare_plain_ask_message_content(
            message: discord_message_dispatch.InboundMessage,
            content: str,
            target_thread_id: str | None,
            *,
            has_attachments: bool,
        ) -> str | None:
            return await discord_plain_ask.prepare_plain_ask_message_content(
                owner,
                cast(discord_plain_ask.PlainAskPreparedMessage, cast(object, message)),
                content,
                target_thread_id,
                has_attachments=has_attachments,
                deps=self.deps.make_plain_ask_message_content_deps(),
            )

        async def handle_prefix_command(
            message: discord_message_dispatch.DispatchMessage,
            command: str,
        ) -> None:
            await self.deps.handle_prefix_command(owner, message, command)

        await discord_message_dispatch_runtime.process_inbound_discord_message_safely(
            message,
            source=source,
            enable_prefix_commands=owner.enable_prefix_commands,
            deps=discord_message_dispatch_runtime.InboundMessageRuntimeDeps(
                require_messageable_channel=self.deps.require_messageable_channel,
                is_allowed_message_channel=owner.is_allowed_message_channel,
                is_bot_authored_bridge_mention=is_bot_authored_bridge_mention,
                is_allowed_user=owner.is_allowed_user,
                is_stopping=self.deps.is_stopping,
                send_restarting_notice=self.deps.send_restarting_notice,
                get_mirrored_codex_thread_id=self.deps.get_mirrored_codex_thread_id,
                get_bridge_mention_user_ids=lambda: discord_message_gate.get_bridge_mention_user_ids(
                    owner,
                ),
                maybe_send_empty_content_notice=self.deps.maybe_send_empty_content_notice,
                prepare_plain_ask_message_content=prepare_plain_ask_message_content,
                persist_inbound_mirror_thread_channel=(
                    self.deps.persist_inbound_mirror_thread_channel
                ),
                handle_prefix_command=handle_prefix_command,
                describe_mirrored_project_channel=self.deps.describe_mirrored_project_channel,
                send_chunks=self.deps.send_chunks,
                handle_plain_ask=self.deps.handle_plain_ask,
                format_log_text_len=self.deps.format_log_text_len,
                delivery_rejected_type=self.deps.delivery_rejected_type,
                delivery_exceptions=self.deps.delivery_exceptions,
                format_exception=self.deps.format_exception,
                log=self.deps.log,
            ),
        )
