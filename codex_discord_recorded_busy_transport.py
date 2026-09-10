from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
import sqlite3
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Protocol

import discord

import codex_discord_prompt_busy_result as discord_prompt_busy_result
import codex_discord_steering as discord_steering
from codex_thread_models import ThreadInfo

LogFunc = Callable[[str], None]
TextLenFormatter = Callable[[str | None], int]
TimeoutGetter = Callable[[], float]
SteeringHandoffMarker = Callable[[str | None], None]


class PromptDeliveryBridge(Protocol):
    def wait_for_prompt_delivery(
        self,
        recent_offsets: discord_prompt_busy_result.RecentOffsets,
        prompt: str,
        *,
        timeout_sec: float,
    ) -> ThreadInfo | None: ...

    def get_thread_workspace_ref(self, thread: ThreadInfo) -> str | None: ...

    def get_thread_label(self, thread: ThreadInfo) -> str: ...


class SteeringResultStreamer(Protocol):
    def __call__(
        self,
        channel: discord.abc.Messageable,
        steering_result: discord_steering.SteeringPromptResult,
        target_thread_id: str | None,
        *,
        label: str,
    ) -> Awaitable[bool]: ...


@dataclass(frozen=True, slots=True)
class RecordedBusyTransportDeps:
    bridge: PromptDeliveryBridge
    get_delivery_confirm_timeout: TimeoutGetter
    mark_steering_handoff: SteeringHandoffMarker
    stream_steering_prompt_result_to_channel: SteeringResultStreamer
    format_log_text_len: TextLenFormatter
    log: LogFunc


async def handle_recorded_busy_transport_prompt(
    channel: discord.abc.Messageable,
    prompt: str,
    *,
    target_thread_id: str | None,
    target_ref: str,
    recent_offsets: discord_prompt_busy_result.RecentOffsets,
    transport_output: str,
    delegate_to_session_mirror: bool,
    deps: RecordedBusyTransportDeps,
) -> bool:
    if not target_thread_id or not recent_offsets:
        return False
    try:
        delivered_thread = await asyncio.to_thread(
            deps.bridge.wait_for_prompt_delivery,
            recent_offsets,
            prompt,
            timeout_sec=deps.get_delivery_confirm_timeout(),
        )
    except (OSError, RuntimeError, sqlite3.Error) as exc:
        deps.log(
            f"ask_busy_delivery_verify_failed target={target_thread_id or '-'} "
            + f"error={str(exc)[:200]}"
        )
        return False
    if delivered_thread is None:
        return False
    if delivered_thread.id != target_thread_id:
        deps.log(
            f"ask_busy_delivery_wrong_thread target={target_thread_id or '-'} "
            + f"delivered={delivered_thread.id}"
        )
        return False

    delivered_ref = deps.bridge.get_thread_workspace_ref(delivered_thread) or target_ref
    deps.log(
        f"ask_busy_nonzero_but_delivered target={target_thread_id or '-'} "
        + f"delivered={delivered_thread.id} output_len={deps.format_log_text_len(transport_output)}"
    )
    deps.mark_steering_handoff(target_thread_id)
    if delegate_to_session_mirror:
        return True

    result = discord_steering.make_steering_prompt_result(
        0,
        "\n\n".join(
            part
            for part in [
                f"[delivery_verified] {deps.bridge.get_thread_label(delivered_thread)}",
                "Original transport returned a selected-thread busy error, but the Discord prompt was recorded in Codex.",
                transport_output,
            ]
            if part
        ),
        target_thread=delivered_thread,
        target_ref=delivered_ref,
        recent_offsets=recent_offsets,
    )
    _ = await deps.stream_steering_prompt_result_to_channel(
        channel,
        result,
        target_thread_id,
        label="Ask",
    )
    return True
