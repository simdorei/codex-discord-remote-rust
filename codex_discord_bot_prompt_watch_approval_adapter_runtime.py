from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from types import ModuleType
from typing import Protocol, cast, TypeAlias

import codex_discord_approval_followup as discord_approval_followup
ModuleValue: TypeAlias = object


class ChannelFetcher(Protocol):
    async def fetch_channel(self, channel_id: int) -> ModuleValue: ...


@dataclass(frozen=True, slots=True)
class BotPromptWatchApprovalAdapterRuntime:
    module: ModuleType

    async def stream_post_approval_result_to_channel(
        self,
        channel: discord_approval_followup.ApprovalFollowupChannel,
        watch_result: discord_approval_followup.ApprovalFollowupWatchResult | None,
        target_thread_id: str | None,
    ) -> bool:
        if target_thread_id is None:
            self.log_line("approval_followup_watch_unavailable target=- reason=no_target")
            return False
        return await discord_approval_followup.stream_post_approval_result_to_channel(
            channel,
            watch_result,
            target_thread_id,
            deps=discord_approval_followup.ApprovalFollowupDeps(
                make_relay=cast(discord_approval_followup.MakeRelayFunc, self._module_func("make_approval_followup_relay")),
                get_watch_timeout=self.get_watch_timeout,
                channel_typing=cast(
                    discord_approval_followup.ChannelTypingFunc,
                    self._module_func("approval_followup_channel_typing"),
                ),
                run_watch_stream=cast(
                    discord_approval_followup.WatchStreamFunc,
                    self._module_func("run_approval_followup_watch_stream"),
                ),
                send_chunks=cast(discord_approval_followup.SendChunksFunc, self._module_func("send_chunks")),
                log_line=self.log_line,
                format_log_text_len=cast(
                    Callable[[str], int | str],
                    self._module_func("format_log_text_len"),
                ),
            ),
        )

    async def resolve_approval_followup_channel(
        self,
        interaction: ModuleValue,
    ) -> discord_approval_followup.ApprovalFollowupChannel | None:
        channel = cast(object | None, getattr(interaction, "channel", None))
        if channel is not None and callable(getattr(channel, "send", None)):
            return self.require_approval_channel(channel)
        message = cast(object | None, getattr(interaction, "message", None))
        message_channel = None if message is None else cast(object | None, getattr(message, "channel", None))
        if message_channel is not None and callable(getattr(message_channel, "send", None)):
            return self.require_approval_channel(message_channel)
        channel_id = int(getattr(interaction, "channel_id", 0) or 0)
        client = cast(object | None, getattr(interaction, "client", None))
        if channel_id and client is not None:
            try:
                fetched = await cast(ChannelFetcher, client).fetch_channel(channel_id)
                if callable(getattr(fetched, "send", None)):
                    return self.require_approval_channel(fetched)
            except self.delivery_exceptions() as exc:
                self.log_line(
                    f"approval_followup_channel_fetch_failed target_channel={channel_id} "
                    + f"error_type={type(exc).__name__}"
                )
        return None

    async def stream_post_approval_result_for_interaction(
        self,
        interaction: ModuleValue,
        watch_result: discord_approval_followup.ApprovalFollowupWatchResult | None,
        target_thread_id: str,
    ) -> bool:
        channel = await self.resolve_approval_followup_channel(interaction)
        if channel is None:
            self.log_line(f"approval_followup_watch_channel_unavailable target={target_thread_id}")
            return False
        return await self.stream_post_approval_result_to_channel(channel, watch_result, target_thread_id)

    def run_approval_followup_watch_stream(
        self,
        watch_result: discord_approval_followup.ApprovalFollowupWatchResult,
        relay: discord_approval_followup.ApprovalFollowupRelay,
        *,
        timeout_sec: float,
    ) -> tuple[int, str]:
        return cast(
            discord_approval_followup.WatchStreamFunc,
            self._module_func("run_steering_watch_stream"),
        )(watch_result, relay, timeout_sec=timeout_sec)

    def require_approval_channel(self, channel: ModuleValue) -> discord_approval_followup.ApprovalFollowupChannel:
        return cast(
            discord_approval_followup.ApprovalFollowupChannel,
            cast(Callable[[object], object], self._module_func("require_discord_messageable_channel"))(channel),
        )

    def get_watch_timeout(self) -> float:
        return cast(Callable[[], float], self._module_func("get_steering_pending_watch_timeout"))()

    def delivery_exceptions(self) -> tuple[type[BaseException], ...]:
        return cast(tuple[type[BaseException], ...], getattr(self.module, "DISCORD_DELIVERY_EXCEPTIONS"))

    def log_line(self, message: str) -> None:
        cast(Callable[[str], None], self._module_func("log_line"))(message)

    def _module_func(self, name: str) -> ModuleValue:
        return cast(object, getattr(self.module, name))
