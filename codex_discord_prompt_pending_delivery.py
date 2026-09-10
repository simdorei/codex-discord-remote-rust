from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Generic, Protocol, TypeVar


ChannelContraT = TypeVar("ChannelContraT", contravariant=True)
ChannelT = TypeVar("ChannelT")
DeliveryPendingPredicate = Callable[[str], bool]
LogFunc = Callable[[str], None]
TextLenFunc = Callable[[str | None], int]


class AskStreamRelay(Protocol):
    @property
    def sent_live(self) -> bool: ...


class ChunkSender(Protocol[ChannelContraT]):
    def __call__(
        self,
        channel: ChannelContraT,
        content: str,
        *,
        context: str | None = None,
    ) -> Awaitable[None]: ...


@dataclass(frozen=True, slots=True)
class AskStreamPendingDeliveryDeps(Generic[ChannelT]):
    is_delivery_confirmation_timeout: DeliveryPendingPredicate
    send_chunks: ChunkSender[ChannelT]
    format_log_text_len: TextLenFunc
    log: LogFunc


async def handle_ask_stream_delivery_pending(
    channel: ChannelT,
    *,
    exit_code: int,
    output: str,
    relay: AskStreamRelay,
    target_thread_id: str | None,
    log_pending: bool,
    deps: AskStreamPendingDeliveryDeps[ChannelT],
) -> bool:
    if not deps.is_delivery_confirmation_timeout(output):
        return False
    if log_pending:
        target_ref = target_thread_id or "-"
        deps.log(
            f"ask_stream_delivery_pending exit={exit_code} target={target_ref} sent_live={relay.sent_live} "
            + f"output_len={deps.format_log_text_len(output)}"
        )
    await deps.send_chunks(channel, format_pending_ask_delivery_output(output))
    return True


def format_pending_ask_delivery_output(output: str) -> str:
    _ = output
    return "\n".join(
        [
            "[delivery_pending] Codex accepted the message, but local recording is delayed.",
            "Wait for the mirrored reply before resending.",
        ]
    )
