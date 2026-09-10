from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Generic, Protocol, TypeVar


ChannelT = TypeVar("ChannelT")
ChannelContraT = TypeVar("ChannelContraT", contravariant=True)
MessageT = TypeVar("MessageT", bound="BusyPromptMessage")
MessageContraT = TypeVar("MessageContraT", bound="BusyPromptMessage", contravariant=True)
SendResultT = TypeVar("SendResultT")
SendResultCoT = TypeVar("SendResultCoT", covariant=True)
FormatLogTextLen = Callable[[str | None], int | str]


class BusyPromptAuthor(Protocol):
    @property
    def bot(self) -> bool: ...


class BusyPromptMessage(Protocol):
    @property
    def author(self) -> BusyPromptAuthor: ...


class EnqueueThreadAsk(Protocol[ChannelContraT, MessageContraT]):
    def __call__(
        self,
        channel: ChannelContraT,
        prompt: str,
        target_thread_id: str | None,
        *,
        queued: bool = False,
        ack_sent: bool = False,
        source_message: MessageContraT | None = None,
    ) -> Awaitable[int]: ...


class SendBusyChoiceMessage(Protocol[ChannelContraT, MessageContraT]):
    def __call__(
        self,
        channel: ChannelContraT,
        source_message: MessageContraT,
        prompt: str,
        *,
        target_thread_id: str | None,
        allow_steer: bool,
        reason: str,
    ) -> Awaitable[bool]: ...


class SendChunks(Protocol[ChannelContraT, SendResultCoT]):
    def __call__(
        self,
        target: ChannelContraT,
        text: str,
        *,
        context: str = "send_chunks",
    ) -> Awaitable[SendResultCoT]: ...


@dataclass(frozen=True, slots=True)
class BusyPromptDeps(Generic[ChannelT, MessageT, SendResultT]):
    enqueue_thread_ask: EnqueueThreadAsk[ChannelT, MessageT]
    send_busy_choice_message: SendBusyChoiceMessage[ChannelT, MessageT]
    send_chunks: SendChunks[ChannelT, SendResultT]
    format_log_text_len: FormatLogTextLen
    log: Callable[[str], None]


def is_bot_authored_message(message: BusyPromptMessage) -> bool:
    return bool(message.author.bot)


async def handle_busy_prompt(
    channel: ChannelT,
    source_message: MessageT,
    prompt: str,
    *,
    target_thread_id: str | None,
    allow_steer: bool,
    reason: str,
    deps: BusyPromptDeps[ChannelT, MessageT, SendResultT],
) -> None:
    if not is_bot_authored_message(source_message):
        _ = await deps.send_busy_choice_message(
            channel,
            source_message,
            prompt,
            target_thread_id=target_thread_id,
            allow_steer=allow_steer,
            reason=reason,
        )
        return

    position = await deps.enqueue_thread_ask(
        channel,
        prompt,
        target_thread_id,
        queued=True,
        ack_sent=False,
        source_message=source_message,
    )
    safe_reason = reason.replace("\n", " ")[:80]
    deps.log(
        f"bot_busy_prompt_auto_queued reason={safe_reason} "
        + f"target={target_thread_id or '-'} position={position} "
        + f"prompt_len={deps.format_log_text_len(prompt)}"
    )
    _ = await deps.send_chunks(
        channel,
        "Queued bot-authored message after the current Codex turn.",
        context="bot_busy_prompt_auto_queued",
    )
