from __future__ import annotations

import traceback
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from types import ModuleType
from typing import cast, TypeAlias

import codex_discord_bot_message_runtime as discord_bot_message_runtime
import codex_discord_message_dispatch as discord_message_dispatch
import codex_discord_plain_ask as discord_plain_ask
import codex_discord_prefix_dispatch as discord_prefix_dispatch
ModuleValue: TypeAlias = object


@dataclass(frozen=True, slots=True)
class BotMessageAdapterRuntime:
    module: ModuleType

    def make_message_runtime(self) -> discord_bot_message_runtime.BotMessageRuntime:
        return discord_bot_message_runtime.BotMessageRuntime(
            discord_bot_message_runtime.BotMessageRuntimeDeps(
                require_messageable_channel=self.require_messageable_channel,
                is_stopping=self.is_discord_delivery_stopping,
                send_restarting_notice=self.send_discord_restarting_notice,
                get_mirrored_codex_thread_id=self.get_mirrored_codex_thread_id,
                maybe_send_empty_content_notice=self.maybe_send_empty_content_notice,
                make_plain_ask_message_content_deps=self.make_plain_ask_message_content_deps,
                persist_inbound_mirror_thread_channel=self.persist_inbound_mirror_thread_channel,
                handle_prefix_command=self.dispatch_handle_prefix_command,
                describe_mirrored_project_channel=self.describe_mirrored_project_channel,
                send_chunks=self.send_chunks,
                handle_plain_ask=self.handle_plain_ask,
                format_log_text_len=self.format_log_text_len,
                delivery_rejected_type=cast(type[BaseException], getattr(self.module, "DiscordDeliveryRejected")),
                delivery_exceptions=cast(tuple[type[BaseException], ...], getattr(self.module, "DISCORD_DELIVERY_EXCEPTIONS")),
                format_exception=traceback.format_exc,
                log=cast(Callable[[str], None], self._module_func("log_line")),
            )
        )

    async def handle_prefix_command(
        self,
        owner: discord_prefix_dispatch.PrefixDispatchBot,
        message: discord_message_dispatch.DispatchMessage,
        command: str,
    ) -> None:
        await discord_prefix_dispatch.handle_prefix_command(
            cast(discord_prefix_dispatch.PrefixDispatchMessage, message),
            command,
            deps=cast(
                Callable[[discord_prefix_dispatch.PrefixDispatchBot], discord_prefix_dispatch.PrefixDispatchDeps],
                self._module_func("_make_prefix_dispatch_deps"),
            )(owner),
        )

    async def dispatch_handle_prefix_command(
        self,
        owner: discord_bot_message_runtime.MessageRuntimeOwner,
        message: discord_message_dispatch.DispatchMessage,
        command: str,
    ) -> None:
        await cast(
            discord_bot_message_runtime.OwnerPrefixCommandHandler,
            self._module_func("handle_prefix_command"),
        )(owner, message, command)

    def is_discord_delivery_stopping(self) -> bool:
        return cast(Callable[[], bool], self._module_func("is_discord_delivery_stopping"))()

    def require_messageable_channel(
        self,
        channel: discord_message_dispatch.InboundMessageChannel,
    ) -> discord_message_dispatch.DispatchChannel:
        return cast(
            discord_message_dispatch.MessageableChannelResolver,
            self._module_func("require_discord_messageable_channel"),
        )(channel)

    async def send_discord_restarting_notice(
        self,
        target: discord_message_dispatch.DispatchChannel,
    ) -> None:
        await cast(
            discord_message_dispatch.RestartNoticeSender,
            self._module_func("send_discord_restarting_notice"),
        )(target)

    def get_mirrored_codex_thread_id(self, channel_id: int | None) -> str | None:
        return cast(Callable[[int | None], str | None], self._module_func("get_mirrored_codex_thread_id"))(
            channel_id
        )

    async def maybe_send_empty_content_notice(
        self,
        message: discord_message_dispatch.InboundMessage,
    ) -> None:
        await cast(
            Callable[[discord_message_dispatch.InboundMessage], Awaitable[None]],
            self._module_func("maybe_send_empty_content_notice"),
        )(message)

    def persist_inbound_mirror_thread_channel(self, target_thread_id: str, channel_id: int) -> None:
        cast(
            discord_message_dispatch.MirrorChannelPersister,
            self._module_func("persist_inbound_mirror_thread_channel"),
        )(target_thread_id, channel_id)

    def describe_mirrored_project_channel(self, channel_id: int) -> str | None:
        return cast(
            discord_message_dispatch.ProjectChannelDescriber,
            self._module_func("describe_mirrored_project_channel"),
        )(channel_id)

    async def send_chunks(
        self,
        target: discord_message_dispatch.DispatchChannel,
        text: str,
    ) -> int:
        return await cast(discord_message_dispatch.ChunkSender, self._module_func("send_chunks"))(target, text)

    async def handle_plain_ask(
        self,
        message: discord_message_dispatch.DispatchMessage,
        content: str,
        *,
        target_thread_id: str | None = None,
    ) -> None:
        await cast(discord_message_dispatch.PlainAskHandler, self._module_func("handle_plain_ask"))(
            message,
            content,
            target_thread_id=target_thread_id,
        )

    def format_log_text_len(self, text: str) -> int | str:
        return cast(discord_message_dispatch.FormatLogTextLenFunc, self._module_func("format_log_text_len"))(text)

    def make_plain_ask_message_content_deps(
        self,
    ) -> discord_plain_ask.PlainAskMessageContentDeps[
        discord_plain_ask.PlainAskPreparedMessage,
        int,
    ]:
        return cast(
            Callable[
                [],
                discord_plain_ask.PlainAskMessageContentDeps[
                    discord_plain_ask.PlainAskPreparedMessage,
                    int,
                ],
            ],
            self._module_func("_make_plain_ask_message_content_deps"),
        )()

    def _module_func(self, name: str) -> ModuleValue:
        return cast(object, getattr(self.module, name))
