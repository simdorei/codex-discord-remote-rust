from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Protocol

from codex_discord_persistent_busy_steer import (
    STEER_APP_MENU_REFRESHED_FOLLOWUP_MESSAGE,
    STEER_STALE_BLOCK_FOLLOWUP_MESSAGE,
)

LogFunc = Callable[[str], None]
TargetRefResolver = Callable[[str | None], tuple[str | None, str]]
NotAcceptedMessageBuilder = Callable[[str], str]


class BusyChoiceInteraction(Protocol): ...


class BusyChoiceChannel(Protocol): ...


class CodexAppMenuSender(Protocol):
    def __call__(
        self,
        channel: BusyChoiceChannel,
        target_thread_id: str | None,
        output: str,
        *,
        reason: str,
    ) -> Awaitable[bool]: ...


class StaleBusySteerBlockSender(Protocol):
    def __call__(
        self,
        channel: BusyChoiceChannel,
        prompt: str,
        target_thread_id: str | None,
        *,
        reason: str,
    ) -> Awaitable[bool]: ...


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


@dataclass(frozen=True, slots=True)
class BusyChoiceSteerFailureDeps:
    send_codex_app_menu_if_available: CodexAppMenuSender
    send_stale_block_message: StaleBusySteerBlockSender
    send_followup_chunks: FollowupChunkSender
    resolve_target_ref: TargetRefResolver
    build_not_accepted_message: NotAcceptedMessageBuilder
    log: LogFunc


async def handle_busy_choice_steer_busy_failure(
    interaction: BusyChoiceInteraction,
    channel: BusyChoiceChannel,
    prompt: str,
    target_thread_id: str | None,
    *,
    exit_code: int,
    output: str,
    deps: BusyChoiceSteerFailureDeps,
) -> bool:
    if await deps.send_codex_app_menu_if_available(
        channel,
        target_thread_id,
        output,
        reason="steer_busy_failure",
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
    if await deps.send_stale_block_message(
        channel,
        prompt,
        target_thread_id,
        reason="steer_busy_failure",
    ):
        await deps.send_followup_chunks(
            interaction,
            STEER_STALE_BLOCK_FOLLOWUP_MESSAGE,
            title="Steering",
            exit_code=0,
            log_prefix="button_response",
            ephemeral=True,
        )
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
    deps.log(f"steer_busy_status_sent reason=steer_busy_failure exit={exit_code} target={target_thread_id or '-'}")
    return True
