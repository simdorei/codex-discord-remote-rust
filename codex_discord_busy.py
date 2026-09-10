"""Busy-thread helper logic for Discord ask flows."""

from __future__ import annotations

from collections.abc import Awaitable, Callable
import traceback
from typing import Protocol, TypeVar

from codex_discord_busy_messages import (
    BusyChoiceViewFactory,
    FitSingleMessage,
    FormatLogTextLen,
    LogLine,
    build_busy_choice_message,
    build_stale_busy_steer_block_message,
    log_busy_choice_sent,
    make_busy_choice_payload,
)

SourceMessageT = TypeVar("SourceMessageT")
SourceMessageT_contra = TypeVar("SourceMessageT_contra", contravariant=True)
ViewT = TypeVar("ViewT")
ChannelT = TypeVar("ChannelT")
ChannelT_contra = TypeVar("ChannelT_contra", contravariant=True)
SendResultT = TypeVar("SendResultT")
SendResultT_co = TypeVar("SendResultT_co", covariant=True)
ViewT_contra = TypeVar("ViewT_contra", contravariant=True)
ViewT_co = TypeVar("ViewT_co", covariant=True)


class SendMessageTrackedFunc(Protocol[ChannelT_contra, ViewT_contra, SendResultT_co]):
    def __call__(
        self,
        target: ChannelT_contra,
        content: str,
        *,
        view: ViewT_contra,
        context: str,
    ) -> Awaitable[SendResultT_co]: ...


class SendChunksFunc(Protocol[ChannelT_contra, SendResultT_co]):
    def __call__(self, target: ChannelT_contra, text: str) -> Awaitable[SendResultT_co]: ...


class MakeBusyChoicePayloadFunc(Protocol[SourceMessageT_contra, ViewT_co]):
    def __call__(
        self,
        source_message: SourceMessageT_contra,
        prompt: str,
        *,
        target_thread_id: str | None,
        allow_steer: bool,
    ) -> tuple[str, ViewT_co]: ...


class BusyChoiceSourceValue(Protocol):
    pass


class BusyChoiceSource(Protocol):
    @property
    def author(self) -> BusyChoiceSourceValue | None: ...

    @property
    def channel(self) -> BusyChoiceSourceValue | None: ...


def is_selected_thread_busy_error(exit_code: int, output: str) -> bool:
    if exit_code == 0:
        return False
    text = (output or "").lower()
    return (
        "selected thread is still busy" in text
        or "target thread is still busy" in text
        or "--force-while-busy" in text and "still busy" in text
        or "selected thread is waiting on a follow-up choice or input" in text
        or "selected thread is waiting on an approval prompt" in text
        or "timed out waiting for ipc data" in text and "codex-ipc" in text
    )


def has_busy_choice_source(source_message: BusyChoiceSource | None) -> bool:
    return bool(
        source_message is not None
        and getattr(source_message, "author", None) is not None
        and getattr(source_message, "channel", None) is not None
    )


async def send_busy_choice_message(
    channel: ChannelT,
    content: str,
    view: ViewT,
    *,
    reason: str,
    target_thread_id: str | None,
    prompt: str,
    send_message_tracked_func: SendMessageTrackedFunc[ChannelT, ViewT, SendResultT_co],
    send_chunks_func: SendChunksFunc[ChannelT, SendResultT_co],
    log_busy_choice_sent_func: Callable[[str, str | None, str], None],
    format_log_text_len_func: FormatLogTextLen,
    log_func: LogLine,
) -> bool:
    failure: Exception | None = None
    try:
        _ = await send_message_tracked_func(
            channel,
            content,
            view=view,
            context=f"busy_choice:{reason}",
        )
        log_busy_choice_sent_func(reason, target_thread_id, prompt)
        return True
    except Exception as exc:  # noqa: BROAD_EXCEPT_OK
        failure = exc
        safe_reason = reason.replace("\n", " ")[:80]
        log_func(
            f"busy_choice_send_failed reason={safe_reason} "
            + f"target={target_thread_id or '-'} prompt_len={format_log_text_len_func(prompt)}\n"
            + traceback.format_exc()
        )
    try:
        _ = await send_chunks_func(
            channel,
            "\n\n".join(
                [
                    "Busy choice failed",
                    f"ERROR: {type(failure).__name__}: {failure}",
                ]
            ),
        )
        safe_reason = reason.replace("\n", " ")[:80]
        log_func(f"busy_choice_error_sent reason={safe_reason} target={target_thread_id or '-'}")
        return False
    except Exception:  # noqa: BROAD_EXCEPT_OK
        safe_reason = reason.replace("\n", " ")[:80]
        log_func(
            f"busy_choice_error_send_failed reason={safe_reason} "
            + f"target={target_thread_id or '-'}\n"
            + traceback.format_exc()
        )
        raise


async def send_busy_choice_payload_message(
    channel: ChannelT,
    source_message: SourceMessageT,
    prompt: str,
    *,
    target_thread_id: str | None,
    allow_steer: bool,
    reason: str,
    make_busy_choice_payload_func: MakeBusyChoicePayloadFunc[SourceMessageT, ViewT],
    send_message_tracked_func: SendMessageTrackedFunc[ChannelT, ViewT, SendResultT],
    send_chunks_func: SendChunksFunc[ChannelT, SendResultT],
    log_busy_choice_sent_func: Callable[[str, str | None, str], None],
    format_log_text_len_func: FormatLogTextLen,
    log_func: LogLine,
) -> bool:
    content, view = make_busy_choice_payload_func(
        source_message,
        prompt,
        target_thread_id=target_thread_id,
        allow_steer=allow_steer,
    )
    return await send_busy_choice_message(
        channel,
        content,
        view,
        reason=reason,
        target_thread_id=target_thread_id,
        prompt=prompt,
        send_message_tracked_func=send_message_tracked_func,
        send_chunks_func=send_chunks_func,
        log_busy_choice_sent_func=log_busy_choice_sent_func,
        format_log_text_len_func=format_log_text_len_func,
        log_func=log_func,
    )


__all__ = [
    "BusyChoiceSource",
    "BusyChoiceSourceValue",
    "BusyChoiceViewFactory",
    "FitSingleMessage",
    "FormatLogTextLen",
    "LogLine",
    "SendChunksFunc",
    "SendMessageTrackedFunc",
    "SourceMessageT",
    "SourceMessageT_contra",
    "ViewT",
    "build_busy_choice_message",
    "build_stale_busy_steer_block_message",
    "has_busy_choice_source",
    "is_selected_thread_busy_error",
    "log_busy_choice_sent",
    "send_busy_choice_payload_message",
    "make_busy_choice_payload",
    "send_busy_choice_message",
]
