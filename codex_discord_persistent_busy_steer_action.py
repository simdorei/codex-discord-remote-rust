from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Protocol

import codex_discord_persistent_busy_steer as discord_persistent_busy_steer
from codex_discord_steering import SteeringPromptResult

BusyErrorDetector = Callable[[int, str], bool]
LogFunc = Callable[[str], None]
TextLenFormatter = Callable[[str | None], int]


class SteerChannel(Protocol): ...


class PersistentBusyInteraction(Protocol): ...


class StaleSteerBlockHandler(Protocol):
    def __call__(
        self,
        interaction: PersistentBusyInteraction,
        channel: SteerChannel,
        prompt: str,
        target_thread_id: str | None,
        *,
        reason: str,
        deps: discord_persistent_busy_steer.PersistentBusyStaleSteerBlockDeps,
    ) -> Awaitable[bool]: ...


class SessionMirrorPreparer(Protocol):
    def __call__(
        self,
        channel: SteerChannel,
        target_thread_id: str | None,
        *,
        deps: discord_persistent_busy_steer.PersistentBusySteerSessionMirrorDeps,
    ) -> Awaitable[bool]: ...


class SteeringStartAckSender(Protocol):
    def __call__(self, channel: SteerChannel, prompt: str, target_thread_id: str | None) -> Awaitable[None]: ...


class SteerPromptRunner(Protocol):
    def __call__(
        self,
        channel: SteerChannel,
        prompt: str,
        target_thread_id: str | None,
        *,
        choice_id: str,
        deps: discord_persistent_busy_steer.PersistentBusySteerRunDeps,
    ) -> Awaitable[SteeringPromptResult]: ...


class SteerBusyFailureHandler(Protocol):
    def __call__(
        self,
        interaction: PersistentBusyInteraction,
        channel: SteerChannel,
        prompt: str,
        target_thread_id: str | None,
        output: str,
        *,
        deps: discord_persistent_busy_steer.PersistentBusySteerBusyFailureDeps,
    ) -> Awaitable[bool]: ...


class SteerResultHandler(Protocol):
    def __call__(
        self,
        interaction: PersistentBusyInteraction,
        channel: SteerChannel,
        steering_result: SteeringPromptResult,
        target_thread_id: str | None,
        *,
        delegate_to_session_mirror: bool,
        deps: discord_persistent_busy_steer.PersistentBusySteerResultDeps,
    ) -> Awaitable[bool]: ...


@dataclass(frozen=True, slots=True)
class PersistentBusySteerActionDeps:
    handle_stale_steer_block: StaleSteerBlockHandler
    stale_steer_block_deps: discord_persistent_busy_steer.PersistentBusyStaleSteerBlockDeps
    prepare_session_mirror: SessionMirrorPreparer
    session_mirror_deps: discord_persistent_busy_steer.PersistentBusySteerSessionMirrorDeps
    send_steering_start_ack: SteeringStartAckSender
    run_steer_prompt: SteerPromptRunner
    steer_run_deps: discord_persistent_busy_steer.PersistentBusySteerRunDeps
    is_selected_thread_busy_error: BusyErrorDetector
    handle_busy_failure: SteerBusyFailureHandler
    busy_failure_deps: discord_persistent_busy_steer.PersistentBusySteerBusyFailureDeps
    handle_steer_result: SteerResultHandler
    steer_result_deps: discord_persistent_busy_steer.PersistentBusySteerResultDeps
    format_log_text_len: TextLenFormatter
    log: LogFunc


def make_persistent_busy_steer_action_deps(
    *,
    steering_runner: discord_persistent_busy_steer.SteeringRunner,
    steering_streamer: discord_persistent_busy_steer.PersistentBusySteerStreamer,
    send_stale_block_message: discord_persistent_busy_steer.BusyStaleSteerBlockSender,
    send_followup_chunks: discord_persistent_busy_steer.BusyFollowupChunkSender,
    prepare_mapped_session_mirror_output: discord_persistent_busy_steer.BusySteerSessionMirrorPreparer,
    prepare_session_mirror_delegation: discord_persistent_busy_steer.BusySteerSessionMirrorPreparer,
    send_steering_start_ack: SteeringStartAckSender,
    channel_typing: discord_persistent_busy_steer.BusyChannelTypingFactory,
    mark_steering_handoff: discord_persistent_busy_steer.SteeringHandoffMarker,
    is_selected_thread_busy_error: BusyErrorDetector,
    send_codex_app_menu_if_available: discord_persistent_busy_steer.BusyCodexAppMenuSender,
    resolve_target_ref: discord_persistent_busy_steer.TargetRefResolver,
    build_not_accepted_message: discord_persistent_busy_steer.NotAcceptedMessageBuilder,
    format_log_text_len: TextLenFormatter,
    monotonic: discord_persistent_busy_steer.MonotonicFunc,
    log: LogFunc,
) -> PersistentBusySteerActionDeps:
    async def stream_persistent_busy_result(
        channel: SteerChannel,
        steering_result: SteeringPromptResult,
        target_thread_id: str | None,
        *,
        send_commentary_blocks: bool | None,
        send_final_blocks: bool,
    ) -> None:
        _ = await steering_streamer(
            channel,
            steering_result,
            target_thread_id,
            send_commentary_blocks=send_commentary_blocks,
            send_final_blocks=send_final_blocks,
        )

    return PersistentBusySteerActionDeps(
        handle_stale_steer_block=discord_persistent_busy_steer.handle_persistent_busy_stale_steer_block,
        stale_steer_block_deps=discord_persistent_busy_steer.PersistentBusyStaleSteerBlockDeps(
            send_stale_block_message=send_stale_block_message,
            send_followup_chunks=send_followup_chunks,
        ),
        prepare_session_mirror=discord_persistent_busy_steer.prepare_persistent_busy_steer_session_mirror,
        session_mirror_deps=discord_persistent_busy_steer.PersistentBusySteerSessionMirrorDeps(
            prepare_mapped_session_mirror_output=prepare_mapped_session_mirror_output,
            prepare_session_mirror_delegation=prepare_session_mirror_delegation,
        ),
        send_steering_start_ack=send_steering_start_ack,
        run_steer_prompt=discord_persistent_busy_steer.run_persistent_busy_steer_prompt,
        steer_run_deps=discord_persistent_busy_steer.PersistentBusySteerRunDeps(
            run_steering_prompt=steering_runner,
            channel_typing=channel_typing,
            mark_steering_handoff=mark_steering_handoff,
            format_log_text_len=format_log_text_len,
            monotonic=monotonic,
            log=log,
        ),
        is_selected_thread_busy_error=is_selected_thread_busy_error,
        handle_busy_failure=discord_persistent_busy_steer.handle_persistent_busy_steer_busy_failure,
        busy_failure_deps=discord_persistent_busy_steer.PersistentBusySteerBusyFailureDeps(
            send_codex_app_menu_if_available=send_codex_app_menu_if_available,
            send_stale_block_message=send_stale_block_message,
            send_followup_chunks=send_followup_chunks,
            resolve_target_ref=resolve_target_ref,
            build_not_accepted_message=build_not_accepted_message,
            log=log,
        ),
        handle_steer_result=discord_persistent_busy_steer.handle_persistent_busy_steer_result,
        steer_result_deps=discord_persistent_busy_steer.PersistentBusySteerResultDeps(
            send_followup_chunks=send_followup_chunks,
            steering_streamer=stream_persistent_busy_result,
            log=log,
        ),
        format_log_text_len=format_log_text_len,
        log=log,
    )


async def handle_persistent_busy_steer_action(
    interaction: PersistentBusyInteraction,
    channel: SteerChannel,
    *,
    user_id: int,
    choice_id: str,
    target_thread_id: str | None,
    prompt: str,
    deps: PersistentBusySteerActionDeps,
) -> bool:
    prompt_len = deps.format_log_text_len(prompt)
    deps.log(f"busy_choice_persistent_steer user={user_id} choice={choice_id} target={target_thread_id or '-'} prompt_len={prompt_len}")
    if await deps.handle_stale_steer_block(
        interaction,
        channel,
        prompt,
        target_thread_id,
        reason="persistent_steer_now",
        deps=deps.stale_steer_block_deps,
    ):
        return True
    delegate_to_session_mirror = await deps.prepare_session_mirror(
        channel,
        target_thread_id,
        deps=deps.session_mirror_deps,
    )
    await deps.send_steering_start_ack(channel, prompt, target_thread_id)
    steering_result = await deps.run_steer_prompt(
        channel,
        prompt,
        target_thread_id,
        choice_id=choice_id,
        deps=deps.steer_run_deps,
    )
    exit_code = steering_result.exit_code
    output = steering_result.output
    if deps.is_selected_thread_busy_error(exit_code, output):
        return await deps.handle_busy_failure(
            interaction,
            channel,
            prompt,
            target_thread_id,
            output,
            deps=deps.busy_failure_deps,
        )
    return await deps.handle_steer_result(
        interaction,
        channel,
        steering_result,
        target_thread_id,
        delegate_to_session_mirror=delegate_to_session_mirror,
        deps=deps.steer_result_deps,
    )
