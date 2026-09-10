from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from types import ModuleType
from typing import TypeAlias, cast

import codex_discord_bot_relay_runtime as discord_bot_relay_runtime
import codex_discord_prompt_relay_channels as discord_prompt_relay_channels
import codex_discord_runtime_config as discord_runtime_config
import codex_discord_stream as discord_stream
import codex_discord_stream_relay as discord_stream_relay
ModuleValue: TypeAlias = object


PromptRelayChannel: TypeAlias = object
MessageableChannel: TypeAlias = object


@dataclass(frozen=True, slots=True)
class BotPromptRelayAdapterRuntime:
    module: ModuleType

    def make_prompt_relay_channels(self) -> discord_prompt_relay_channels.PromptRelayChannelRuntime:
        return discord_prompt_relay_channels.PromptRelayChannelRuntime(
            discord_prompt_relay_channels.PromptRelayChannelDeps(
                send_chunks=self.send_prompt_relay_chunks,
                send_interactive_prompt=self.send_interactive_prompt,
                format_log_text_len=cast(
                    Callable[[str | None], int | str],
                    self._module_func("format_log_text_len"),
                ),
            )
        )

    def make_discord_ask_relay_class(self) -> type[discord_stream.DiscordAskRelay]:
        return discord_bot_relay_runtime.make_discord_ask_relay_class(
            discord_bot_relay_runtime.DiscordAskRelayClassDeps(
                commentary_enabled=discord_runtime_config.discord_stream_commentary_enabled,
                send_chunks=self.dispatch_send_relay_chunks,
                parse_interactive_notice=cast(
                    discord_stream_relay.ParseInteractiveNoticeFunc,
                    self._module_func("parse_interactive_notice"),
                ),
                send_interactive_prompt=self.dispatch_send_relay_interactive_prompt,
                register_discord_relay=cast(
                    discord_stream_relay.RegisterDiscordRelayFunc,
                    self._module_func("register_discord_relay"),
                ),
                is_discord_relay_stale=cast(
                    discord_stream_relay.IsDiscordRelayStaleFunc,
                    self._module_func("is_discord_relay_stale"),
                ),
                had_steering_handoff_since=cast(
                    discord_stream_relay.HadSteeringHandoffSinceFunc,
                    self._module_func("had_steering_handoff_since"),
                ),
                log=cast(discord_stream_relay.LogFunc, self._module_func("log_line")),
                format_log_text_len=self.dispatch_format_relay_log_text_len,
            ),
            quiet_notice_delay_seconds=cast(
                float,
                getattr(self.module, "QUIET_PROGRESS_NOTICE_DELAY_SECONDS"),
            ),
        )

    def require_discord_messageable_channel(self, channel: PromptRelayChannel) -> MessageableChannel:
        return channel

    async def send_relay_chunks(
        self,
        channel: discord_stream_relay.RelayChannel,
        text: str,
    ) -> None:
        messageable = cast(
            Callable[[PromptRelayChannel], MessageableChannel],
            self._module_func("require_discord_messageable_channel"),
        )(channel)
        _ = await cast(
            Callable[[MessageableChannel, str], Awaitable[int | None]],
            self._module_func("send_chunks"),
        )(messageable, text)

    async def send_relay_interactive_prompt(
        self,
        channel: discord_stream_relay.RelayChannel,
        target_thread_id: str,
        target_ref: str,
        state: str,
        prompt: str,
        options: discord_stream_relay.InteractiveNoticeOptions,
    ) -> None:
        await self._prompt_relay_channels().send_relay_interactive_prompt(
            channel,
            target_thread_id,
            target_ref,
            state,
            prompt,
            options,
        )

    def format_relay_log_text_len(self, text: str) -> str:
        return self._prompt_relay_channels().format_relay_log_text_len(text)

    def format_log_text_len_as_text(self, value: ModuleValue) -> str:
        text = str(value) if value is not None else None
        return self._prompt_relay_channels().format_log_text_len_as_text(text)

    async def send_prompt_chunks(
        self,
        channel: PromptRelayChannel,
        content: str,
        *,
        context: str | None = None,
    ) -> None:
        await self._prompt_relay_channels().send_prompt_chunks(
            channel,
            content,
            context=context,
        )

    async def send_prompt_relay_chunks(
        self,
        channel: PromptRelayChannel,
        text: str,
        *,
        context: str = "send_chunks",
    ) -> int | None:
        return await cast(
            discord_prompt_relay_channels.PromptChunkSender,
            self._module_func("send_chunks"),
        )(channel, text, context=context)

    async def send_interactive_prompt(
        self,
        channel: PromptRelayChannel,
        target_thread_id: str,
        target_ref: str,
        state: str,
        prompt: str,
        options: list[tuple[str, str]],
    ) -> None:
        await cast(
            discord_prompt_relay_channels.PromptInteractiveSender,
            self._module_func("send_interactive_prompt"),
        )(
            channel,
            target_thread_id,
            target_ref,
            state,
            prompt,
            options,
        )

    async def dispatch_send_relay_chunks(
        self,
        channel: discord_stream_relay.RelayChannel,
        text: str,
    ) -> None:
        await cast(
            discord_stream_relay.SendChunksFunc,
            self._module_func("send_relay_chunks"),
        )(channel, text)

    async def dispatch_send_relay_interactive_prompt(
        self,
        channel: discord_stream_relay.RelayChannel,
        target_thread_id: str,
        target_ref: str,
        state: str,
        prompt: str,
        options: discord_stream_relay.InteractiveNoticeOptions,
    ) -> None:
        await cast(
            discord_stream_relay.SendInteractivePromptFunc,
            self._module_func("send_relay_interactive_prompt"),
        )(channel, target_thread_id, target_ref, state, prompt, options)

    def dispatch_format_relay_log_text_len(self, text: str) -> str:
        return cast(
            discord_stream_relay.FormatLogTextLenFunc,
            self._module_func("format_relay_log_text_len"),
        )(text)

    def _prompt_relay_channels(self) -> discord_prompt_relay_channels.PromptRelayChannelRuntime:
        return cast(
            discord_prompt_relay_channels.PromptRelayChannelRuntime,
            getattr(self.module, "PROMPT_RELAY_CHANNELS"),
        )

    def _module_func(self, name: str) -> ModuleValue:
        return cast(object, getattr(self.module, name))
