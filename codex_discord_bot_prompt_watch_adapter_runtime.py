from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Callable
from contextlib import AbstractAsyncContextManager
from dataclasses import dataclass
from pathlib import Path
from types import ModuleType
from typing import Protocol, cast, TypeAlias

import codex_discord_approval_followup as discord_approval_followup
import codex_discord_prompt_watch_runtime as discord_prompt_watch_runtime
import codex_discord_steering as discord_steering
import codex_discord_steering_watch as discord_steering_watch
import codex_discord_steering_watch_runtime as discord_steering_watch_runtime
import codex_discord_stream as discord_stream
import codex_discord_stream_relay as discord_stream_relay
ModuleValue: TypeAlias = object


class DiscordAskRelayFactory(Protocol):
    def __call__(
        self,
        loop: asyncio.AbstractEventLoop,
        channel: discord_stream_relay.RelayChannel,
        target_thread_id: str | None,
        target_ref: str,
        quiet_notice_delay_sec: float = 0,
        suppress_after_steering_since: float | None = None,
        send_timeout_blocks: bool = True,
        send_commentary_blocks: bool | None = None,
        send_final_blocks: bool = True,
    ) -> discord_stream.DiscordAskRelay: ...


@dataclass(frozen=True, slots=True)
class BotPromptWatchAdapterRuntime:
    module: ModuleType

    def make_prompt_watch_runtime(self) -> discord_prompt_watch_runtime.PromptWatchRuntime:
        return discord_prompt_watch_runtime.PromptWatchRuntime(
            discord_prompt_watch_runtime.PromptWatchRuntimeDeps(
                make_steering_relay=self.make_steering_relay,
                make_approval_relay=self.make_approval_relay,
                get_watch_timeout=self.get_watch_timeout,
                channel_typing=self.channel_typing,
                run_watch_stream=self.run_watch_stream,
                send_chunks=self.send_chunks,
                watch_for_final_answer=self.watch_for_final_answer,
                make_post_approval_watch_result=self.make_post_approval_watch_result,
                log_line=self.log_line,
                format_log_text_len=self.format_log_text_len,
            )
        )

    def make_steering_relay(
        self,
        loop: discord_steering_watch.SteeringWatchLoop,
        channel: discord_steering_watch.SteeringWatchChannel,
        target_thread_id: str,
        target_ref: str,
        suppress_after_steering_since: float,
        send_timeout_blocks: bool,
        send_commentary_blocks: bool | None,
        send_final_blocks: bool,
    ) -> discord_steering_watch.SteeringWatchRelay:
        relay_factory = cast(DiscordAskRelayFactory, getattr(self.module, "DiscordAskRelay"))
        return cast(
            discord_steering_watch.SteeringWatchRelay,
            relay_factory(
                loop,
                self.require_relay_channel(channel),
                target_thread_id,
                target_ref,
                suppress_after_steering_since=suppress_after_steering_since,
                send_timeout_blocks=send_timeout_blocks,
                send_commentary_blocks=send_commentary_blocks,
                send_final_blocks=send_final_blocks,
            ),
        )

    def make_approval_relay(
        self,
        loop: discord_approval_followup.ApprovalFollowupLoop,
        channel: discord_approval_followup.ApprovalFollowupChannel,
        target_thread_id: str,
        target_ref: str,
        send_timeout_blocks: bool,
    ) -> discord_approval_followup.ApprovalFollowupRelay:
        relay_factory = cast(DiscordAskRelayFactory, getattr(self.module, "DiscordAskRelay"))
        return cast(
            discord_approval_followup.ApprovalFollowupRelay,
            relay_factory(
                loop,
                self.require_relay_channel(channel),
                target_thread_id,
                target_ref,
                send_timeout_blocks=send_timeout_blocks,
            ),
        )

    def get_watch_timeout(self) -> float:
        return cast(Callable[[], float], self._module_func("get_steering_pending_watch_timeout"))()

    def channel_typing(
        self,
        channel: discord_steering_watch.SteeringWatchChannel,
        *,
        context: str,
    ) -> AbstractAsyncContextManager[None]:
        return cast(
            discord_steering_watch.ChannelTypingFunc,
            self._module_func("channel_typing"),
        )(self.require_relay_channel(channel), context=context)

    def run_watch_stream(
        self,
        watch_result: discord_steering_watch.SteeringWatchResult,
        relay: discord_steering_watch.SteeringWatchRelay,
        *,
        timeout_sec: float,
    ) -> tuple[int, str]:
        return cast(
            discord_steering_watch.WatchStreamFunc,
            self._module_func("run_steering_watch_stream"),
        )(watch_result, relay, timeout_sec=timeout_sec)

    async def send_chunks(
        self,
        channel: discord_steering_watch.SteeringWatchChannel,
        content: str,
    ) -> int | None:
        return await cast(
            discord_steering_watch.SendChunksFunc,
            self._module_func("send_chunks"),
        )(self.require_relay_channel(channel), content)

    def watch_for_final_answer(
        self,
        *,
        session_path: Path,
        start_offset: int,
        timeout_sec: float,
        include_commentary: bool,
        stream_live: bool = False,
        stream_label: str = "",
        stream_callback: discord_stream.LineStreamFunc | None = None,
        expected_turn_id: str | None = None,
    ) -> discord_stream.WatchForFinalAnswerResult:
        bridge = cast(object, getattr(self.module, "BRIDGE_FINAL_ANSWER"))
        watcher = cast(discord_stream.WatchForFinalAnswerFunc, getattr(bridge, "watch_for_final_answer"))
        if expected_turn_id is not None:
            return watcher(
                session_path=session_path,
                start_offset=start_offset,
                timeout_sec=timeout_sec,
                include_commentary=include_commentary,
                stream_live=stream_live,
                stream_label=stream_label,
                stream_callback=stream_callback,
                expected_turn_id=expected_turn_id,
            )
        return watcher(
            session_path=session_path,
            start_offset=start_offset,
            timeout_sec=timeout_sec,
            include_commentary=include_commentary,
            stream_live=stream_live,
            stream_label=stream_label,
            stream_callback=stream_callback,
        )

    def make_post_approval_watch_result(
        self,
        target_thread_id: str,
    ) -> discord_steering.SteeringPromptResult | None:
        sqlite_module = cast(ModuleType, getattr(self.module, "sqlite3"))
        sqlite_error_type = cast(type[Exception], getattr(sqlite_module, "Error"))
        return discord_steering_watch_runtime.make_post_approval_watch_result(
            target_thread_id,
            bridge=cast(discord_steering_watch_runtime.ApprovalWatchBridge, getattr(self.module, "BRIDGE_THREAD_STATE")),
            get_active_turn_id=self.get_active_turn_id,
            log_line=self.log_line,
            expected_exceptions=(OSError, RuntimeError, sqlite_error_type),
        )

    def get_active_turn_id(self, thread_id: str) -> str | None:
        transport_module = cast(ModuleType, getattr(self.module, "app_server_transport"))
        client = cast(object, getattr(transport_module, "DEFAULT_CLIENT"))
        return cast(Callable[[str], str | None], getattr(client, "get_active_turn_id"))(thread_id)

    def require_relay_channel(self, channel: ModuleValue) -> discord_stream_relay.RelayChannel:
        return cast(
            discord_stream_relay.RelayChannel,
            cast(Callable[[object], object], self._module_func("require_discord_messageable_channel"))(channel),
        )

    def log_line(self, message: str) -> None:
        cast(Callable[[str], None], self._module_func("log_line"))(message)

    def format_log_text_len(self, text: str | None) -> int | str:
        return cast(Callable[[str | None], int | str], self._module_func("format_log_text_len"))(text)

    def _module_func(self, name: str) -> ModuleValue:
        return cast(object, getattr(self.module, name))
