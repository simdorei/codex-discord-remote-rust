from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Protocol

from codex_discord_steering import SteeringPromptResult

LogFunc = Callable[[str], None]


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


class PersistentBusySteerStreamer(Protocol):
    def __call__(
        self,
        channel: PersistentBusyChannel,
        steering_result: SteeringPromptResult,
        target_thread_id: str | None,
        *,
        send_commentary_blocks: bool | None,
        send_final_blocks: bool,
    ) -> Awaitable[None]: ...


@dataclass(frozen=True, slots=True)
class PersistentBusySteerResultDeps:
    send_followup_chunks: BusyFollowupChunkSender
    steering_streamer: PersistentBusySteerStreamer
    log: LogFunc


async def handle_persistent_busy_steer_result(
    interaction: PersistentBusyInteraction,
    channel: PersistentBusyChannel,
    steering_result: SteeringPromptResult,
    target_thread_id: str | None,
    *,
    delegate_to_session_mirror: bool,
    deps: PersistentBusySteerResultDeps,
) -> bool:
    exit_code = steering_result.exit_code
    output = steering_result.output
    title = "Steering sent" if exit_code == 0 else f"Steering failed (exit {exit_code})"
    await deps.send_followup_chunks(
        interaction,
        f"{title}\n\n{output or '(no output)'}",
        title="Steering",
        exit_code=exit_code,
        log_prefix="button_response",
        ephemeral=True,
    )
    if exit_code != 0:
        return True
    if delegate_to_session_mirror:
        deps.log(f"busy_choice_persistent_steer_delegated_to_session_mirror target={target_thread_id or '-'}")
    await deps.steering_streamer(
        channel,
        steering_result,
        target_thread_id,
        send_commentary_blocks=False if delegate_to_session_mirror else None,
        send_final_blocks=not delegate_to_session_mirror,
    )
    return True
