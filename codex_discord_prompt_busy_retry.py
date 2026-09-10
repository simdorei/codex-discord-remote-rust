from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Generic, Protocol, TypeAlias, TypeVar

import codex_discord_prompt_busy_result as busy_result
import codex_discord_prompt_pending_delivery as pending_delivery
import codex_discord_prompt_retry_attempt as retry_attempt
import codex_discord_prompt_retry_exhausted as retry_exhausted
import codex_discord_prompt_retry_suppression as retry_suppression


ChannelT = TypeVar("ChannelT")
SendResultT = TypeVar("SendResultT")
ChannelContraT = TypeVar("ChannelContraT", contravariant=True)
SendResultCoT = TypeVar("SendResultCoT", covariant=True)
RecentOffsets: TypeAlias = busy_result.RecentOffsets
BusyPredicate: TypeAlias = Callable[[int, str], bool]
LogFunc: TypeAlias = Callable[[str], None]


class BusyRetryRelay(retry_attempt.RetryRelay, retry_suppression.RetryRelay, Protocol):
    pass


RelayT = TypeVar("RelayT", bound=BusyRetryRelay)


class ChunkSender(Protocol[ChannelContraT, SendResultCoT]):
    def __call__(self, channel: ChannelContraT, text: str) -> Awaitable[SendResultCoT]: ...


@dataclass(frozen=True, slots=True)
class BusyRetryFlowDeps(Generic[ChannelT, RelayT, SendResultT]):
    is_selected_thread_busy_error: BusyPredicate
    retry_attempt_deps: retry_attempt.RetryAttemptDeps[ChannelT, RelayT]
    retry_suppression_deps: retry_suppression.RetrySuppressionDeps
    pending_delivery_deps: pending_delivery.AskStreamPendingDeliveryDeps[ChannelT]
    busy_result_deps: busy_result.AskStreamBusyResultDeps[ChannelT]
    retry_exhausted_deps: retry_exhausted.RetryExhaustedDeps[ChannelT]
    send_chunks: ChunkSender[ChannelT, SendResultT]
    log: LogFunc


@dataclass(frozen=True, slots=True)
class BusyRetryFlowResult(Generic[RelayT]):
    exit_code: int
    output: str
    relay: RelayT
    handled: bool


def make_busy_retry_flow_deps(
    *,
    is_selected_thread_busy_error: BusyPredicate,
    sleep: retry_attempt.AsyncSleeper,
    make_retry_relay: retry_attempt.RetryRelayFactory[ChannelT, RelayT],
    channel_typing: retry_attempt.ChannelTypingFactory[ChannelT],
    run_ask_stream: retry_attempt.AskStreamRunner[RelayT],
    is_discord_relay_stale: retry_suppression.RelayStalePredicate,
    pending_delivery_deps: pending_delivery.AskStreamPendingDeliveryDeps[ChannelT],
    busy_result_deps: busy_result.AskStreamBusyResultDeps[ChannelT],
    send_retry_notice_chunks: ChunkSender[ChannelT, SendResultT],
    send_status_chunks: retry_exhausted.ChunkSender[ChannelT],
    format_log_text_len: retry_attempt.TextLenFunc,
    log: LogFunc,
    build_codex_app_busy_retry_message: retry_exhausted.BusyRetryMessageBuilder = (
        busy_result.build_codex_app_busy_retry_message
    ),
) -> BusyRetryFlowDeps[ChannelT, RelayT, SendResultT]:
    return BusyRetryFlowDeps(
        is_selected_thread_busy_error=is_selected_thread_busy_error,
        retry_attempt_deps=retry_attempt.RetryAttemptDeps(
            sleep=sleep,
            make_retry_relay=make_retry_relay,
            channel_typing=channel_typing,
            run_ask_stream=run_ask_stream,
            format_log_text_len=format_log_text_len,
            log=log,
        ),
        retry_suppression_deps=retry_suppression.RetrySuppressionDeps(
            is_discord_relay_stale=is_discord_relay_stale,
            format_log_text_len=format_log_text_len,
            log=log,
        ),
        pending_delivery_deps=pending_delivery_deps,
        busy_result_deps=busy_result_deps,
        retry_exhausted_deps=retry_exhausted.RetryExhaustedDeps(
            is_selected_thread_busy_error=is_selected_thread_busy_error,
            build_codex_app_busy_retry_message=build_codex_app_busy_retry_message,
            send_chunks=send_status_chunks,
            format_log_text_len=format_log_text_len,
            log=log,
        ),
        send_chunks=send_retry_notice_chunks,
        log=log,
    )


async def handle_busy_retry_flow(
    channel: ChannelT,
    *,
    prompt: str,
    exit_code: int,
    output: str,
    relay: RelayT,
    target_thread_id: str | None,
    target_ref: str,
    recent_offsets: RecentOffsets,
    delegate_to_session_mirror: bool,
    started_at: float,
    retry_attempts: int,
    retry_delay: float,
    source_message_available: bool,
    deps: BusyRetryFlowDeps[ChannelT, RelayT, SendResultT],
) -> BusyRetryFlowResult[RelayT]:
    if not deps.is_selected_thread_busy_error(exit_code, output):
        return BusyRetryFlowResult(exit_code, output, relay, False)

    deps.log(
        f"ask_stream_busy_transport_failure kind=target target={target_thread_id or '-'} "
        f"source_message={'yes' if source_message_available else 'no'}"
    )
    if await _handle_busy_result(
        channel,
        prompt=prompt,
        target_thread_id=target_thread_id,
        target_ref=target_ref,
        recent_offsets=recent_offsets,
        transport_output=output,
        delegate_to_session_mirror=delegate_to_session_mirror,
        retry_index=None,
        deps=deps,
    ):
        return BusyRetryFlowResult(exit_code, output, relay, True)

    if retry_attempts > 0:
        await deps.send_chunks(
            channel,
            f"Codex app did not accept this Discord message yet. Retrying mapped-thread delivery up to {retry_attempts} time(s).",
        )
    for retry_index in range(1, retry_attempts + 1):
        retry_result = await retry_attempt.run_retry_attempt(
            channel,
            prompt=prompt,
            retry_index=retry_index,
            retry_delay=retry_delay,
            target_thread_id=target_thread_id,
            target_ref=target_ref,
            started_at=started_at,
            delegate_to_session_mirror=delegate_to_session_mirror,
            deps=deps.retry_attempt_deps,
        )
        exit_code, output, relay = retry_result.exit_code, retry_result.output, retry_result.relay
        if retry_suppression.handle_retry_suppressed_after_steering(
            relay=relay,
            retry_index=retry_index,
            target_thread_id=target_thread_id,
            output=output,
            deps=deps.retry_suppression_deps,
        ):
            return BusyRetryFlowResult(exit_code, output, relay, True)
        if await pending_delivery.handle_ask_stream_delivery_pending(
            channel,
            exit_code=exit_code,
            output=output,
            relay=relay,
            target_thread_id=target_thread_id,
            log_pending=False,
            deps=deps.pending_delivery_deps,
        ):
            return BusyRetryFlowResult(exit_code, output, relay, True)
        if not deps.is_selected_thread_busy_error(exit_code, output):
            break
        if await _handle_busy_result(
            channel,
            prompt=prompt,
            target_thread_id=target_thread_id,
            target_ref=target_ref,
            recent_offsets=recent_offsets,
            transport_output=output,
            delegate_to_session_mirror=delegate_to_session_mirror,
            retry_index=retry_index,
            deps=deps,
        ):
            return BusyRetryFlowResult(exit_code, output, relay, True)

    handled = await retry_exhausted.handle_retry_exhausted_status(
        channel,
        exit_code=exit_code,
        output=output,
        target_thread_id=target_thread_id,
        target_ref=target_ref,
        retry_attempts=retry_attempts,
        deps=deps.retry_exhausted_deps,
    )
    return BusyRetryFlowResult(exit_code, output, relay, handled)


async def _handle_busy_result(
    channel: ChannelT,
    *,
    prompt: str,
    target_thread_id: str | None,
    target_ref: str,
    recent_offsets: RecentOffsets,
    transport_output: str,
    delegate_to_session_mirror: bool,
    retry_index: int | None,
    deps: BusyRetryFlowDeps[ChannelT, RelayT, SendResultT],
) -> bool:
    return await busy_result.handle_ask_stream_busy_result(
        channel,
        prompt,
        target_thread_id=target_thread_id,
        target_ref=target_ref,
        recent_offsets=recent_offsets,
        transport_output=transport_output,
        delegate_to_session_mirror=delegate_to_session_mirror,
        retry_index=retry_index,
        deps=deps.busy_result_deps,
    )
