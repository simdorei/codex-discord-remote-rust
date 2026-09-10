from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Awaitable, Callable
from contextlib import AbstractAsyncContextManager
from dataclasses import dataclass
from typing import Protocol

import codex_discord_busy as discord_busy
import codex_discord_busy_choice_steer_failure as discord_busy_choice_steer_failure
import codex_discord_busy_choice_steer_result as discord_busy_choice_steer_result
from codex_discord_steering import SteeringPromptResult

LogFunc = Callable[[str], None]
LogTextLenFormatter = Callable[[str | None], int]
TimeNowFunc = Callable[[], float]
SteeringRunner = Callable[[str, str | None], SteeringPromptResult]
SteeringHandoffMarker = Callable[[str | None], float]


class BusyChoiceInteraction(Protocol):
    pass


class BusyChoiceChannel(Protocol):
    pass


class FollowupChunkSender(Protocol):
    def __call__(
        self,
        interaction: BusyChoiceInteraction,
        content: str,
        *,
        title: str,
        exit_code: int,
        log_prefix: str,
        ephemeral: bool,
    ) -> Awaitable[None]: ...


class StaleBusySteerBlockSender(Protocol):
    def __call__(
        self,
        channel: BusyChoiceChannel,
        prompt: str,
        target_thread_id: str | None,
        *,
        reason: str,
    ) -> Awaitable[bool]: ...


class SessionMirrorOutputPreparer(Protocol):
    def __call__(self, channel: BusyChoiceChannel, target_thread_id: str | None) -> Awaitable[bool]: ...


class SteeringStartAckSender(Protocol):
    def __call__(
        self,
        channel: BusyChoiceChannel,
        prompt: str,
        target_thread_id: str | None,
    ) -> Awaitable[None]: ...


class ChannelTypingFactory(Protocol):
    def __call__(self, channel: BusyChoiceChannel, *, context: str) -> AbstractAsyncContextManager[None]: ...


@dataclass(frozen=True, slots=True)
class BusyChoiceSteerActionDeps:
    send_stale_block_message: StaleBusySteerBlockSender
    prepare_mapped_session_mirror_output: SessionMirrorOutputPreparer
    prepare_session_mirror_delegation: SessionMirrorOutputPreparer
    send_steering_start_ack: SteeringStartAckSender
    send_followup_chunks: FollowupChunkSender
    channel_typing: ChannelTypingFactory
    run_steering_prompt: SteeringRunner
    mark_steering_handoff: SteeringHandoffMarker
    format_log_text_len: LogTextLenFormatter
    log: LogFunc
    time_monotonic: TimeNowFunc
    steer_failure_deps: discord_busy_choice_steer_failure.BusyChoiceSteerFailureDeps
    steer_result_deps: discord_busy_choice_steer_result.BusyChoiceSteerResultDeps


async def handle_busy_choice_steer_action(
    interaction: BusyChoiceInteraction,
    channel: BusyChoiceChannel,
    prompt: str,
    target_thread_id: str | None,
    *,
    user_id: int,
    deps: BusyChoiceSteerActionDeps,
) -> None:
    target_log = target_thread_id or "-"
    prompt_len = deps.format_log_text_len(prompt)
    deps.log(f"steer_now user={user_id} target={target_log} prompt_len={prompt_len}")
    _ = await deps.send_stale_block_message(
        channel,
        prompt,
        target_thread_id,
        reason="steer_now",
    )

    delegate_to_session_mirror = await deps.prepare_mapped_session_mirror_output(
        channel,
        target_thread_id,
    )
    if not delegate_to_session_mirror:
        delegate_to_session_mirror = await deps.prepare_session_mirror_delegation(
            channel,
            target_thread_id,
        )

    await deps.send_steering_start_ack(channel, prompt, target_thread_id)
    started_at = deps.time_monotonic()
    async with deps.channel_typing(channel, context="steer_now"):
        steering_result = await asyncio.to_thread(
            deps.run_steering_prompt,
            prompt,
            target_thread_id,
        )

    exit_code = steering_result.exit_code
    output = steering_result.output
    if exit_code == 0:
        _ = deps.mark_steering_handoff(target_thread_id)
    elapsed_sec = deps.time_monotonic() - started_at
    output_len = deps.format_log_text_len(output)
    deps.log(f"steer_now_done exit={exit_code} target={target_log} elapsed_sec={elapsed_sec:.2f} output_len={output_len}")

    if discord_busy.is_selected_thread_busy_error(exit_code, output):
        _ = await discord_busy_choice_steer_failure.handle_busy_choice_steer_busy_failure(
            interaction,
            channel,
            prompt,
            target_thread_id,
            exit_code=exit_code,
            output=output,
            deps=deps.steer_failure_deps,
        )
        return

    await discord_busy_choice_steer_result.handle_busy_choice_steer_result(
        interaction,
        channel,
        steering_result,
        target_thread_id,
        delegate_to_session_mirror=delegate_to_session_mirror,
        deps=deps.steer_result_deps,
    )
