from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Generic, Protocol, TypeVar


ChannelContraT = TypeVar("ChannelContraT", contravariant=True)
ChannelT = TypeVar("ChannelT")
BusyPredicate = Callable[[int, str], bool]
BusyRetryMessageBuilder = Callable[[str, int], str]
LogFunc = Callable[[str], None]
TextLenFunc = Callable[[str | None], int]


class ChunkSender(Protocol[ChannelContraT]):
    def __call__(
        self,
        channel: ChannelContraT,
        content: str,
        *,
        context: str | None = None,
    ) -> Awaitable[None]: ...


@dataclass(frozen=True, slots=True)
class RetryExhaustedDeps(Generic[ChannelT]):
    is_selected_thread_busy_error: BusyPredicate
    build_codex_app_busy_retry_message: BusyRetryMessageBuilder
    send_chunks: ChunkSender[ChannelT]
    format_log_text_len: TextLenFunc
    log: LogFunc


async def handle_retry_exhausted_status(
    channel: ChannelT,
    *,
    exit_code: int,
    output: str,
    target_thread_id: str | None,
    target_ref: str,
    retry_attempts: int,
    deps: RetryExhaustedDeps[ChannelT],
) -> bool:
    if not deps.is_selected_thread_busy_error(exit_code, output):
        return False
    deps.log(
        f"ask_stream_busy_retry_exhausted target={target_thread_id or '-'} attempts={retry_attempts} "
        + f"output_len={deps.format_log_text_len(output)}"
    )
    await deps.send_chunks(channel, deps.build_codex_app_busy_retry_message(target_ref, retry_attempts))
    return True
