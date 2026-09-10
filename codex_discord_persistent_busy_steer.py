from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Awaitable, Callable
from contextlib import AbstractAsyncContextManager
from dataclasses import dataclass
from typing import Protocol

from codex_discord_persistent_busy_steer_result import (
    PersistentBusySteerResultDeps as PersistentBusySteerResultDeps,
    PersistentBusySteerStreamer as PersistentBusySteerStreamer,
    handle_persistent_busy_steer_result as handle_persistent_busy_steer_result,
)
from codex_discord_steering import SteeringPromptResult

STEER_STALE_BLOCK_FOLLOWUP_MESSAGE = "Steering was not sent because this Codex thread appears stuck. See the public channel notice."
STEER_APP_MENU_REFRESHED_FOLLOWUP_MESSAGE = "Codex app menu was refreshed in this Discord thread."
LogFunc = Callable[[str], None]
TextLenFormatter = Callable[[str | None], int]
MonotonicFunc = Callable[[], float]
SteeringHandoffMarker = Callable[[str | None], None]
SteeringRunner = Callable[[str, str | None], SteeringPromptResult]


class PersistentBusyInteraction(Protocol): ...


class PersistentBusyChannel(Protocol): ...


class BusyFollowupChunkSender(Protocol):
    def __call__(
        self,
        interaction: PersistentBusyInteraction,
        content: str,
        *,
        title: str,
        exit_code: int,
        log_prefix: str,
        ephemeral: bool,
    ) -> Awaitable[None]: ...


class BusyCodexAppMenuSender(Protocol):
    def __call__(
        self,
        channel: PersistentBusyChannel,
        target_thread_id: str | None,
        output: str,
        *,
        reason: str,
    ) -> Awaitable[bool]: ...


class BusyStaleSteerBlockSender(Protocol):
    def __call__(
        self,
        channel: PersistentBusyChannel,
        prompt: str,
        target_thread_id: str | None,
        *,
        reason: str,
    ) -> Awaitable[bool]: ...


class BusySteerSessionMirrorPreparer(Protocol):
    def __call__(self, channel: PersistentBusyChannel, target_thread_id: str | None) -> Awaitable[bool]: ...


class BusyChannelTypingFactory(Protocol):
    def __call__(self, channel: PersistentBusyChannel, *, context: str) -> AbstractAsyncContextManager[None]: ...


class TargetRefResolver(Protocol):
    def __call__(self, target_thread_id: str | None) -> tuple[str | None, str]: ...


class NotAcceptedMessageBuilder(Protocol):
    def __call__(self, target_ref: str) -> str: ...


@dataclass(frozen=True, slots=True)
class PersistentBusyStaleSteerBlockDeps:
    send_stale_block_message: BusyStaleSteerBlockSender
    send_followup_chunks: BusyFollowupChunkSender


@dataclass(frozen=True, slots=True)
class PersistentBusySteerBusyFailureDeps:
    send_codex_app_menu_if_available: BusyCodexAppMenuSender
    send_stale_block_message: BusyStaleSteerBlockSender
    send_followup_chunks: BusyFollowupChunkSender
    resolve_target_ref: TargetRefResolver
    build_not_accepted_message: NotAcceptedMessageBuilder
    log: LogFunc


@dataclass(frozen=True, slots=True)
class PersistentBusySteerSessionMirrorDeps:
    prepare_mapped_session_mirror_output: BusySteerSessionMirrorPreparer
    prepare_session_mirror_delegation: BusySteerSessionMirrorPreparer


@dataclass(frozen=True, slots=True)
class PersistentBusySteerRunDeps:
    run_steering_prompt: SteeringRunner
    channel_typing: BusyChannelTypingFactory
    mark_steering_handoff: SteeringHandoffMarker
    format_log_text_len: TextLenFormatter
    monotonic: MonotonicFunc
    log: LogFunc


async def handle_persistent_busy_stale_steer_block(
    interaction: PersistentBusyInteraction,
    channel: PersistentBusyChannel,
    prompt: str,
    target_thread_id: str | None,
    *,
    reason: str,
    deps: PersistentBusyStaleSteerBlockDeps,
) -> bool:
    blocked = await deps.send_stale_block_message(
        channel,
        prompt,
        target_thread_id,
        reason=reason,
    )
    if not blocked:
        return False
    await deps.send_followup_chunks(
        interaction,
        STEER_STALE_BLOCK_FOLLOWUP_MESSAGE,
        title="Steering",
        exit_code=0,
        log_prefix="button_response",
        ephemeral=True,
    )
    return True


async def handle_persistent_busy_steer_busy_failure(
    interaction: PersistentBusyInteraction,
    channel: PersistentBusyChannel,
    prompt: str,
    target_thread_id: str | None,
    output: str,
    *,
    deps: PersistentBusySteerBusyFailureDeps,
) -> bool:
    if await deps.send_codex_app_menu_if_available(
        channel,
        target_thread_id,
        output,
        reason="persistent_steer_busy_failure",
    ):
        await deps.send_followup_chunks(
            interaction,
            STEER_APP_MENU_REFRESHED_FOLLOWUP_MESSAGE,
            title="Steering",
            exit_code=0,
            log_prefix="button_response",
            ephemeral=True,
        )
        return True
    if await handle_persistent_busy_stale_steer_block(
        interaction,
        channel,
        prompt,
        target_thread_id,
        reason="persistent_steer_busy_failure",
        deps=PersistentBusyStaleSteerBlockDeps(
            send_stale_block_message=deps.send_stale_block_message,
            send_followup_chunks=deps.send_followup_chunks,
        ),
    ):
        return True
    _resolved_thread_id, target_ref = deps.resolve_target_ref(target_thread_id)
    await deps.send_followup_chunks(
        interaction,
        deps.build_not_accepted_message(target_ref),
        title="Steering",
        exit_code=0,
        log_prefix="button_response",
        ephemeral=True,
    )
    deps.log(f"steer_busy_status_sent reason=persistent_steer_busy_failure target={target_thread_id or '-'}")
    return True


async def prepare_persistent_busy_steer_session_mirror(
    channel: PersistentBusyChannel,
    target_thread_id: str | None,
    *,
    deps: PersistentBusySteerSessionMirrorDeps,
) -> bool:
    if await deps.prepare_mapped_session_mirror_output(channel, target_thread_id):
        return True
    return await deps.prepare_session_mirror_delegation(channel, target_thread_id)


async def run_persistent_busy_steer_prompt(
    channel: PersistentBusyChannel,
    prompt: str,
    target_thread_id: str | None,
    *,
    choice_id: str,
    deps: PersistentBusySteerRunDeps,
) -> SteeringPromptResult:
    started_at = deps.monotonic()
    async with deps.channel_typing(channel, context="persistent_steer_now"):
        steering_result = await asyncio.to_thread(deps.run_steering_prompt, prompt, target_thread_id)
    exit_code = steering_result.exit_code
    output = steering_result.output
    if exit_code == 0:
        deps.mark_steering_handoff(target_thread_id)
    target = target_thread_id or "-"
    elapsed_sec = deps.monotonic() - started_at
    output_len = deps.format_log_text_len(output)
    deps.log(
        " ".join(
            (
                f"busy_choice_persistent_steer_done exit={exit_code}",
                f"choice={choice_id}",
                f"target={target}",
                f"elapsed_sec={elapsed_sec:.2f}",
                f"output_len={output_len}",
            )
        )
    )
    return steering_result
