from __future__ import annotations

from dataclasses import dataclass
from typing import Generic, Protocol, TypeVar

import codex_discord_prompt_busy_result as busy_result
import codex_discord_prompt_busy_retry as busy_retry
import codex_discord_prompt_flow as prompt_flow
import codex_discord_prompt_pending_delivery as pending_delivery
import codex_discord_prompt_retry_attempt as retry_attempt
import codex_discord_prompt_retry_exhausted as retry_exhausted
import codex_discord_prompt_stream_attempt as stream_attempt
import codex_discord_prompt_stream_result as stream_result
import codex_discord_prompt_stream_suppression as stream_suppression

ChannelT = TypeVar("ChannelT")
SendResultT = TypeVar("SendResultT")


class PromptDeliveryRelay(
    prompt_flow.InitialStreamRelay,
    busy_retry.BusyRetryRelay,
    stream_result.AskStreamRelay,
    Protocol,
):
    pass


RelayT = TypeVar("RelayT", bound=PromptDeliveryRelay)


@dataclass(frozen=True, slots=True)
class PromptDeliveryFlowDeps(Generic[ChannelT, RelayT, SendResultT]):
    initial_stream_deps: prompt_flow.InitialStreamFlowDeps[ChannelT, RelayT]
    busy_retry_deps: busy_retry.BusyRetryFlowDeps[ChannelT, RelayT, SendResultT]
    stream_result_deps: stream_result.AskStreamResultDeps[ChannelT]


def make_prompt_delivery_flow_deps(
    *,
    monotonic: stream_attempt.MonotonicClock,
    make_relay: stream_attempt.StreamRelayFactory[ChannelT, RelayT],
    channel_typing: stream_attempt.ChannelTypingFactory[ChannelT],
    run_ask_stream: stream_attempt.AskStreamRunner[RelayT],
    is_discord_relay_stale: stream_suppression.RelayStalePredicate,
    pending_delivery_deps: pending_delivery.AskStreamPendingDeliveryDeps[ChannelT],
    is_selected_thread_busy_error: busy_retry.BusyPredicate,
    sleep: retry_attempt.AsyncSleeper,
    busy_result_deps: busy_result.AskStreamBusyResultDeps[ChannelT],
    send_retry_notice_chunks: busy_retry.ChunkSender[ChannelT, SendResultT],
    send_status_chunks: retry_exhausted.ChunkSender[ChannelT],
    build_codex_app_busy_retry_message: retry_exhausted.BusyRetryMessageBuilder,
    send_result_chunks: stream_result.ChunkSender[ChannelT],
    had_steering_handoff_since: stream_result.SteeringHandoffPredicate,
    format_log_text_len: stream_attempt.TextLenFunc,
    log: stream_attempt.LogFunc,
) -> PromptDeliveryFlowDeps[ChannelT, RelayT, SendResultT]:
    return PromptDeliveryFlowDeps(
        initial_stream_deps=prompt_flow.make_initial_stream_flow_deps(
            monotonic=monotonic,
            make_relay=make_relay,
            channel_typing=channel_typing,
            run_ask_stream=run_ask_stream,
            is_discord_relay_stale=is_discord_relay_stale,
            pending_delivery_deps=pending_delivery_deps,
            format_log_text_len=format_log_text_len,
            log=log,
        ),
        busy_retry_deps=busy_retry.make_busy_retry_flow_deps(
            is_selected_thread_busy_error=is_selected_thread_busy_error,
            sleep=sleep,
            make_retry_relay=make_relay,
            channel_typing=channel_typing,
            run_ask_stream=run_ask_stream,
            is_discord_relay_stale=is_discord_relay_stale,
            pending_delivery_deps=pending_delivery_deps,
            busy_result_deps=busy_result_deps,
            send_retry_notice_chunks=send_retry_notice_chunks,
            send_status_chunks=send_status_chunks,
            build_codex_app_busy_retry_message=build_codex_app_busy_retry_message,
            format_log_text_len=format_log_text_len,
            log=log,
        ),
        stream_result_deps=stream_result.make_ask_stream_result_deps(
            send_chunks=send_result_chunks,
            had_steering_handoff_since=had_steering_handoff_since,
            format_log_text_len=format_log_text_len,
            log=log,
        ),
    )


async def run_prompt_delivery_flow(
    channel: ChannelT,
    *,
    prompt: str,
    target_thread_id: str | None,
    target_ref: str,
    recent_offsets: busy_retry.RecentOffsets,
    delegate_to_session_mirror: bool,
    retry_attempts: int,
    retry_delay: float,
    source_message_available: bool,
    deps: PromptDeliveryFlowDeps[ChannelT, RelayT, SendResultT],
) -> None:
    initial_stream = await prompt_flow.run_initial_stream_flow(
        channel,
        prompt=prompt,
        target_thread_id=target_thread_id,
        target_ref=target_ref,
        delegate_to_session_mirror=delegate_to_session_mirror,
        deps=deps.initial_stream_deps,
    )
    exit_code, output, relay, started_at = (
        initial_stream.exit_code,
        initial_stream.output,
        initial_stream.relay,
        initial_stream.started_at,
    )
    if initial_stream.handled:
        return
    busy_retry_result = await busy_retry.handle_busy_retry_flow(
        channel,
        prompt=prompt,
        exit_code=exit_code,
        output=output,
        relay=relay,
        target_thread_id=target_thread_id,
        target_ref=target_ref,
        recent_offsets=recent_offsets,
        delegate_to_session_mirror=delegate_to_session_mirror,
        started_at=started_at,
        retry_attempts=retry_attempts,
        retry_delay=retry_delay,
        source_message_available=source_message_available,
        deps=deps.busy_retry_deps,
    )
    exit_code, output, relay = (
        busy_retry_result.exit_code,
        busy_retry_result.output,
        busy_retry_result.relay,
    )
    if busy_retry_result.handled:
        return
    await stream_result.handle_ask_stream_result(
        channel,
        exit_code=exit_code,
        output=output,
        relay=relay,
        target_thread_id=target_thread_id,
        started_at=started_at,
        delegate_to_session_mirror=delegate_to_session_mirror,
        deps=deps.stream_result_deps,
    )
