from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Protocol

from codex_discord_steering import SteeringPromptResult

LogFunc = Callable[[str], None]


class BusyChoiceInteraction(Protocol): ...


class BusyChoiceChannel(Protocol): ...


class BusyChoiceFollowupChunkSender(Protocol):
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


class BusyChoiceSteerStreamer(Protocol):
    def __call__(
        self,
        channel: BusyChoiceChannel,
        steering_result: SteeringPromptResult,
        target_thread_id: str | None,
        *,
        send_commentary_blocks: bool | None,
        send_final_blocks: bool,
    ) -> Awaitable[bool]: ...


@dataclass(frozen=True, slots=True)
class BusyChoiceSteerResultDeps:
    send_followup_chunks: BusyChoiceFollowupChunkSender
    steering_streamer: BusyChoiceSteerStreamer
    log: LogFunc


async def handle_busy_choice_steer_result(
    interaction: BusyChoiceInteraction,
    channel: BusyChoiceChannel,
    steering_result: SteeringPromptResult,
    target_thread_id: str | None,
    *,
    delegate_to_session_mirror: bool,
    deps: BusyChoiceSteerResultDeps,
) -> None:
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
    target_log = target_thread_id or "-"
    deps.log(f"steer_now_sent exit={exit_code} target={target_log}")
    if exit_code != 0:
        return
    if delegate_to_session_mirror:
        deps.log(f"steer_now_delegated_to_session_mirror target={target_log}")
    _ = await deps.steering_streamer(
        channel,
        steering_result,
        target_thread_id,
        send_commentary_blocks=False if delegate_to_session_mirror else None,
        send_final_blocks=not delegate_to_session_mirror,
    )
