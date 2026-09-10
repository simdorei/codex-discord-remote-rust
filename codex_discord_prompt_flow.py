from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Generic, Protocol, TypeVar

import codex_discord_prompt_pending_delivery as pending_delivery
import codex_discord_prompt_stream_attempt as stream_attempt
import codex_discord_prompt_stream_suppression as stream_suppression


ChannelContraT = TypeVar("ChannelContraT", contravariant=True)
ChannelT = TypeVar("ChannelT")
ContextWarningBuilder = Callable[[str | None], str]


class InitialStreamRelay(
    stream_attempt.StreamRelay,
    stream_suppression.StreamSuppressionRelay,
    Protocol,
):
    pass


RelayT = TypeVar("RelayT", bound=InitialStreamRelay)


class QueuedAskStartBuilder(Protocol):
    def __call__(self, prompt: str, *, queued: bool = False) -> str: ...


class ChunkSender(Protocol[ChannelContraT]):
    def __call__(
        self,
        channel: ChannelContraT,
        content: str,
        *,
        context: str | None = None,
    ) -> Awaitable[None]: ...


@dataclass(frozen=True, slots=True)
class PromptFlowPreambleDeps(Generic[ChannelT]):
    build_context_warning: ContextWarningBuilder
    build_ask_start_message: QueuedAskStartBuilder
    send_chunks: ChunkSender[ChannelT]


@dataclass(frozen=True, slots=True)
class InitialStreamFlowDeps(Generic[ChannelT, RelayT]):
    stream_attempt_deps: stream_attempt.StreamAttemptDeps[ChannelT, RelayT]
    stream_suppression_deps: stream_suppression.StreamSuppressionDeps
    pending_delivery_deps: pending_delivery.AskStreamPendingDeliveryDeps[ChannelT]


@dataclass(frozen=True, slots=True)
class InitialStreamFlowResult(Generic[RelayT]):
    exit_code: int
    output: str
    relay: RelayT
    started_at: float
    handled: bool


def make_initial_stream_flow_deps(
    *,
    monotonic: stream_attempt.MonotonicClock,
    make_relay: stream_attempt.StreamRelayFactory[ChannelT, RelayT],
    channel_typing: stream_attempt.ChannelTypingFactory[ChannelT],
    run_ask_stream: stream_attempt.AskStreamRunner[RelayT],
    is_discord_relay_stale: stream_suppression.RelayStalePredicate,
    pending_delivery_deps: pending_delivery.AskStreamPendingDeliveryDeps[ChannelT],
    format_log_text_len: stream_attempt.TextLenFunc,
    log: stream_attempt.LogFunc,
) -> InitialStreamFlowDeps[ChannelT, RelayT]:
    return InitialStreamFlowDeps(
        stream_attempt_deps=stream_attempt.StreamAttemptDeps(
            monotonic=monotonic,
            make_relay=make_relay,
            channel_typing=channel_typing,
            run_ask_stream=run_ask_stream,
            format_log_text_len=format_log_text_len,
            log=log,
        ),
        stream_suppression_deps=stream_suppression.StreamSuppressionDeps(
            is_discord_relay_stale=is_discord_relay_stale,
            format_log_text_len=format_log_text_len,
            log=log,
        ),
        pending_delivery_deps=pending_delivery_deps,
    )


async def send_prompt_flow_preamble(
    channel: ChannelT,
    prompt: str,
    target_thread_id: str | None,
    *,
    queued: bool = False,
    deps: PromptFlowPreambleDeps[ChannelT],
) -> None:
    warning = deps.build_context_warning(target_thread_id)
    if warning:
        await deps.send_chunks(channel, warning)
    await deps.send_chunks(
        channel,
        deps.build_ask_start_message(prompt, queued=queued),
        context="ask_start",
    )


async def run_initial_stream_flow(
    channel: ChannelT,
    *,
    prompt: str,
    target_thread_id: str | None,
    target_ref: str,
    delegate_to_session_mirror: bool,
    deps: InitialStreamFlowDeps[ChannelT, RelayT],
) -> InitialStreamFlowResult[RelayT]:
    stream_result = await stream_attempt.run_stream_attempt(
        channel,
        prompt=prompt,
        target_thread_id=target_thread_id,
        target_ref=target_ref,
        delegate_to_session_mirror=delegate_to_session_mirror,
        deps=deps.stream_attempt_deps,
    )
    exit_code, output, relay, started_at = (
        stream_result.exit_code,
        stream_result.output,
        stream_result.relay,
        stream_result.started_at,
    )
    if stream_suppression.handle_stream_suppressed_after_steering(
        relay=relay,
        target_thread_id=target_thread_id,
        output=output,
        deps=deps.stream_suppression_deps,
    ):
        return InitialStreamFlowResult(exit_code, output, relay, started_at, True)
    if await pending_delivery.handle_ask_stream_delivery_pending(
        channel,
        exit_code=exit_code,
        output=output,
        relay=relay,
        target_thread_id=target_thread_id,
        log_pending=True,
        deps=deps.pending_delivery_deps,
    ):
        return InitialStreamFlowResult(exit_code, output, relay, started_at, True)
    return InitialStreamFlowResult(exit_code, output, relay, started_at, False)
