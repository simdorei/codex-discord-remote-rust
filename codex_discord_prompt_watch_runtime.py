from __future__ import annotations

from collections.abc import Callable
from contextlib import AbstractAsyncContextManager
from dataclasses import dataclass
from typing import cast

import codex_discord_approval_followup as discord_approval_followup
import codex_discord_steering as discord_steering
import codex_discord_steering_watch as discord_steering_watch
import codex_discord_steering_watch_runtime as discord_steering_watch_runtime
import codex_discord_stream as discord_stream
import codex_discord_watch_relay_factory as discord_watch_relay_factory


SteeringPromptResult = discord_steering.SteeringPromptResult
SteeringRelayBuilder = Callable[
    [
        discord_steering_watch.SteeringWatchLoop,
        discord_steering_watch.SteeringWatchChannel,
        str,
        str,
        float,
        bool,
        bool | None,
        bool,
    ],
    discord_steering_watch.SteeringWatchRelay,
]
ApprovalRelayBuilder = Callable[
    [
        discord_approval_followup.ApprovalFollowupLoop,
        discord_approval_followup.ApprovalFollowupChannel,
        str,
        str,
        bool,
    ],
    discord_approval_followup.ApprovalFollowupRelay,
]


@dataclass(frozen=True, slots=True)
class PromptWatchRuntimeDeps:
    make_steering_relay: SteeringRelayBuilder
    make_approval_relay: ApprovalRelayBuilder
    get_watch_timeout: Callable[[], float]
    channel_typing: discord_steering_watch.ChannelTypingFunc
    run_watch_stream: discord_steering_watch.WatchStreamFunc
    send_chunks: discord_steering_watch.SendChunksFunc
    watch_for_final_answer: discord_stream.WatchForFinalAnswerFunc
    make_post_approval_watch_result: Callable[[str], SteeringPromptResult | None]
    log_line: Callable[[str], None]
    format_log_text_len: Callable[[str | None], int | str]


@dataclass(frozen=True, slots=True)
class PromptWatchRuntime:
    deps: PromptWatchRuntimeDeps

    def make_steering_watch_relay(
        self,
        loop: discord_steering_watch.SteeringWatchLoop,
        channel: discord_steering_watch.SteeringWatchChannel,
        target_thread_id: str,
        target_ref: str,
        *,
        started_at: float,
        send_commentary_blocks: bool | None,
        send_final_blocks: bool,
    ) -> discord_steering_watch.SteeringWatchRelay:
        return discord_watch_relay_factory.make_steering_watch_relay(
            self._build_steering_watch_relay,
            loop,
            channel,
            target_thread_id,
            target_ref,
            started_at=started_at,
            send_commentary_blocks=send_commentary_blocks,
            send_final_blocks=send_final_blocks,
        )

    def _build_steering_watch_relay(
        self,
        loop: discord_steering_watch.SteeringWatchLoop,
        channel: discord_steering_watch.SteeringWatchChannel,
        target_thread_id: str,
        target_ref: str,
        *,
        suppress_after_steering_since: float,
        send_timeout_blocks: bool,
        send_commentary_blocks: bool | None,
        send_final_blocks: bool,
    ) -> discord_steering_watch.SteeringWatchRelay:
        return self.deps.make_steering_relay(
            loop,
            channel,
            target_thread_id,
            target_ref,
            suppress_after_steering_since,
            send_timeout_blocks,
            send_commentary_blocks,
            send_final_blocks,
        )

    def run_steering_watch_stream(
        self,
        watch_result: discord_steering_watch.SteeringWatchResult,
        relay: discord_steering_watch.SteeringWatchRelay,
        *,
        timeout_sec: float = 0,
    ) -> tuple[int, str]:
        return discord_stream.run_steering_watch_stream(
            cast(SteeringPromptResult, watch_result),
            cast(discord_stream.DiscordAskRelay, relay),
            timeout_sec=timeout_sec,
            watch_for_final_answer_func=self.deps.watch_for_final_answer,
        )

    def steering_watch_channel_typing(
        self,
        channel: discord_steering_watch.SteeringWatchChannel,
        *,
        context: str,
    ) -> AbstractAsyncContextManager[None]:
        return self.deps.channel_typing(channel, context=context)

    async def stream_steering_prompt_result_to_channel(
        self,
        channel: discord_steering_watch.SteeringWatchChannel,
        steering_result: SteeringPromptResult | None,
        target_thread_id: str | None,
        *,
        label: str = "Steering",
        send_commentary_blocks: bool | None = None,
        send_final_blocks: bool = True,
    ) -> bool:
        return await discord_steering_watch_runtime.stream_steering_prompt_result_to_channel(
            channel,
            steering_result,
            target_thread_id,
            label=label,
            send_commentary_blocks=send_commentary_blocks,
            send_final_blocks=send_final_blocks,
            deps=discord_steering_watch_runtime.make_steering_watch_runtime_deps(
                make_relay=self.make_steering_watch_relay,
                get_watch_timeout=self.deps.get_watch_timeout,
                channel_typing=self.steering_watch_channel_typing,
                run_watch_stream=self.deps.run_watch_stream,
                send_chunks=self.deps.send_chunks,
                log_line=self.deps.log_line,
                format_log_text_len=self.deps.format_log_text_len,
            ),
        )

    def make_post_approval_watch_result(self, target_thread_id: str) -> SteeringPromptResult | None:
        return self.deps.make_post_approval_watch_result(target_thread_id)

    def make_approval_followup_relay(
        self,
        loop: discord_approval_followup.ApprovalFollowupLoop,
        channel: discord_approval_followup.ApprovalFollowupChannel,
        target_thread_id: str,
        target_ref: str,
    ) -> discord_approval_followup.ApprovalFollowupRelay:
        return discord_watch_relay_factory.make_approval_followup_relay(
            self._build_approval_followup_relay,
            loop,
            channel,
            target_thread_id,
            target_ref,
        )

    def _build_approval_followup_relay(
        self,
        loop: discord_approval_followup.ApprovalFollowupLoop,
        channel: discord_approval_followup.ApprovalFollowupChannel,
        target_thread_id: str,
        target_ref: str,
        *,
        send_timeout_blocks: bool,
    ) -> discord_approval_followup.ApprovalFollowupRelay:
        return self.deps.make_approval_relay(
            loop,
            channel,
            target_thread_id,
            target_ref,
            send_timeout_blocks,
        )
