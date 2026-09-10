from __future__ import annotations

import traceback
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Protocol, TypeAlias

MessageTrackedResult: TypeAlias = object


class SteeringAckChannel(Protocol):
    pass


class SteeringAckSender(Protocol):
    def __call__(
        self,
        channel: SteeringAckChannel,
        content: str,
        *,
        context: str,
    ) -> Awaitable[MessageTrackedResult]: ...


@dataclass(frozen=True, slots=True)
class BotSteeringAckRuntime:
    send_message_tracked: SteeringAckSender
    build_steering_start_message: Callable[[str], str]
    delivery_exceptions: tuple[type[BaseException], ...]
    log: Callable[[str], None]
    format_log_text_len: Callable[[str], int | str]

    async def send_steering_start_ack(
        self,
        channel: SteeringAckChannel,
        prompt: str,
        target_thread_id: str | None,
    ) -> bool:
        try:
            _ = await self.send_message_tracked(
                channel,
                self.build_steering_start_message(prompt),
                context="steering_start_ack",
            )
            self.log(
                f"steering_start_ack_sent target={target_thread_id or '-'} "
                + f"prompt_len={self.format_log_text_len(prompt)}"
            )
            return True
        except self.delivery_exceptions:
            self.log(
                f"steering_start_ack_failed target={target_thread_id or '-'} "
                + f"prompt_len={self.format_log_text_len(prompt)}\n"
                + traceback.format_exc()
            )
            return False
