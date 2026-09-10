from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from typing import Protocol, TypeVar

import codex_discord_prompt_busy_result as discord_prompt_busy_result
import codex_discord_prompt_pending_delivery as discord_prompt_pending_delivery


ChannelContraT = TypeVar("ChannelContraT", contravariant=True)
RelayContraT = TypeVar("RelayContraT", contravariant=True)
RelayCoT = TypeVar("RelayCoT", covariant=True)
ChannelT = TypeVar("ChannelT")
RelayT = TypeVar("RelayT")


class DiscordAskRelayFactory(Protocol[ChannelContraT, RelayCoT]):
    def __call__(
        self,
        loop: asyncio.AbstractEventLoop,
        channel: ChannelContraT,
        target_thread_id: str | None,
        target_ref: str,
        *,
        suppress_after_steering_since: float,
        send_commentary_blocks: bool | None,
        send_final_blocks: bool,
    ) -> RelayCoT: ...


class RunAskStreamFunc(Protocol[RelayContraT]):
    def __call__(
        self,
        prompt: str,
        relay: RelayContraT,
        *,
        target_thread_id: str | None,
    ) -> tuple[int, str]: ...


def make_ask_stream_pending_delivery_deps(
    *,
    is_delivery_confirmation_timeout: discord_prompt_pending_delivery.DeliveryPendingPredicate,
    send_chunks: discord_prompt_pending_delivery.ChunkSender[ChannelT],
    format_log_text_len: discord_prompt_pending_delivery.TextLenFunc,
    log: discord_prompt_pending_delivery.LogFunc,
) -> discord_prompt_pending_delivery.AskStreamPendingDeliveryDeps[ChannelT]:
    return discord_prompt_pending_delivery.AskStreamPendingDeliveryDeps(
        is_delivery_confirmation_timeout=is_delivery_confirmation_timeout,
        send_chunks=send_chunks,
        format_log_text_len=format_log_text_len,
        log=log,
    )


def make_discord_ask_relay(
    relay_factory: DiscordAskRelayFactory[ChannelT, RelayT],
    channel: ChannelT,
    *,
    target_thread_id: str | None,
    target_ref: str,
    started_at: float,
    delegate_to_session_mirror: bool,
) -> RelayT:
    return relay_factory(
        asyncio.get_running_loop(),
        channel,
        target_thread_id,
        target_ref,
        suppress_after_steering_since=started_at,
        send_commentary_blocks=False if delegate_to_session_mirror else None,
        send_final_blocks=not delegate_to_session_mirror,
    )


async def run_ask_stream_in_thread(
    run_ask_stream_func: RunAskStreamFunc[RelayT],
    prompt: str,
    relay: RelayT,
    *,
    target_thread_id: str | None,
) -> tuple[int, str]:
    return await asyncio.to_thread(
        run_ask_stream_func,
        prompt,
        relay,
        target_thread_id=target_thread_id,
    )


def make_ask_stream_busy_result_deps(
    *,
    handle_recorded_busy_transport_prompt: discord_prompt_busy_result.RecordedBusyHandler[ChannelT],
    wait_for_mirrored_busy_delegation_settle: discord_prompt_busy_result.BusySettleWaiter,
    mark_steering_handoff: discord_prompt_busy_result.SteeringHandoffMarker,
    send_codex_app_menu_if_available: discord_prompt_busy_result.AppMenuSender[ChannelT],
    format_log_text_len: discord_prompt_busy_result.TextLenFunc,
    log: discord_prompt_busy_result.LogFunc,
) -> discord_prompt_busy_result.AskStreamBusyResultDeps[ChannelT]:
    return discord_prompt_busy_result.AskStreamBusyResultDeps(
        handle_recorded_busy_transport_prompt=handle_recorded_busy_transport_prompt,
        wait_for_mirrored_busy_delegation_settle=wait_for_mirrored_busy_delegation_settle,
        mark_steering_handoff=mark_steering_handoff,
        send_codex_app_menu_if_available=send_codex_app_menu_if_available,
        format_log_text_len=format_log_text_len,
        log=log,
    )
