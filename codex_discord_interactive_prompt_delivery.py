from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Awaitable, Callable
from typing import Protocol, TypeAlias, TypeVar

LogLengthValue: TypeAlias = int | str

ChannelT = TypeVar("ChannelT")
ChannelT_contra = TypeVar("ChannelT_contra", contravariant=True)
SendResultT = TypeVar("SendResultT")
SendResultT_co = TypeVar("SendResultT_co", covariant=True)
WatchResultT = TypeVar("WatchResultT")
WatchResultT_contra = TypeVar("WatchResultT_contra", contravariant=True)


class InteractiveView(Protocol):
    pass


class SendMessageTrackedFunc(Protocol[ChannelT_contra, SendResultT_co]):
    def __call__(
        self,
        target: ChannelT_contra,
        content: str,
        *,
        view: InteractiveView,
        context: str,
    ) -> Awaitable[SendResultT_co]: ...


class SendChunksFunc(Protocol[ChannelT_contra, SendResultT_co]):
    def __call__(self, target: ChannelT_contra, text: str) -> Awaitable[SendResultT_co]: ...


class StreamApprovalResultFunc(Protocol[ChannelT_contra, WatchResultT_contra]):
    def __call__(
        self,
        channel: ChannelT_contra,
        watch_result: WatchResultT_contra,
        target_thread_id: str | None,
    ) -> Awaitable[bool]: ...


def send_interactive_prompt_lines(target_thread_id: str, target_ref: str, heading: str, prompt: str) -> list[str]:
    lines = [heading, f"thread: {target_ref or target_thread_id}", ""]
    if prompt:
        lines.extend([prompt, ""])
    return lines


async def send_interactive_prompt(
    channel: ChannelT,
    target_thread_id: str,
    target_ref: str,
    state: str,
    prompt: str,
    options: list[tuple[str, str]],
    *,
    state_approval: str,
    state_input: str,
    approval_view_factory: Callable[[str], InteractiveView],
    input_choice_view_factory: Callable[[str, list[tuple[str, str]]], InteractiveView],
    send_message_tracked_func: SendMessageTrackedFunc[ChannelT, SendResultT],
    send_chunks_func: SendChunksFunc[ChannelT, SendResultT],
    fit_single_message_func: Callable[[str], str],
) -> None:
    if state == state_approval:
        lines = send_interactive_prompt_lines(target_thread_id, target_ref, "Waiting approval", prompt)
        _ = await send_message_tracked_func(
            channel,
            fit_single_message_func("\n".join(lines)),
            view=approval_view_factory(target_thread_id),
            context="interactive_approval",
        )
        return

    if state == state_input:
        lines = send_interactive_prompt_lines(target_thread_id, target_ref, "Waiting input", prompt)
        if options:
            _ = await send_message_tracked_func(
                channel,
                fit_single_message_func("\n".join(lines)),
                view=input_choice_view_factory(target_thread_id, options),
                context="interactive_input_choice",
            )
        else:
            lines.append("Reply with plain text to answer this prompt.")
            _ = await send_chunks_func(channel, "\n".join(lines))


async def submit_interactive_reply(
    channel: ChannelT,
    target_thread_id: str,
    state: str,
    answer: str,
    *,
    state_approval: str,
    state_input: str,
    make_post_approval_watch_result: Callable[[str], WatchResultT],
    submit_approval_reply_func: Callable[[str, str], tuple[int, str]],
    submit_input_reply_func: Callable[[str, str], tuple[int, str]],
    stream_post_approval_result_func: StreamApprovalResultFunc[ChannelT, WatchResultT],
    send_chunks_func: SendChunksFunc[ChannelT, SendResultT],
    format_log_text_len_func: Callable[[str], LogLengthValue],
    log_func: Callable[[str], None],
) -> None:
    if state == state_approval:
        watch_result = make_post_approval_watch_result(target_thread_id)
        exit_code, output = await asyncio.to_thread(submit_approval_reply_func, target_thread_id, answer)
        log_func(
            f"approval_reply_done exit={exit_code} target={target_thread_id} "
            + f"answer_len={format_log_text_len_func(answer)} "
            + f"output_len={format_log_text_len_func(output)}"
        )
        title = "Approval submitted" if exit_code == 0 else f"Approval failed (exit {exit_code})"
        _ = await send_chunks_func(channel, f"{title}\n\n{output or '(no output)'}")
        if exit_code == 0:
            _ = await stream_post_approval_result_func(channel, watch_result, target_thread_id)
        return

    if state == state_input:
        exit_code, output = await asyncio.to_thread(submit_input_reply_func, target_thread_id, answer)
        log_func(
            f"input_reply_done exit={exit_code} target={target_thread_id} "
            + f"answer_len={format_log_text_len_func(answer)} "
            + f"output_len={format_log_text_len_func(output)}"
        )
        title = "Input submitted" if exit_code == 0 else f"Input failed (exit {exit_code})"
        _ = await send_chunks_func(channel, f"{title}\n\n{output or '(no output)'}")


__all__ = [
    "InteractiveView",
    "SendChunksFunc",
    "SendMessageTrackedFunc",
    "StreamApprovalResultFunc",
    "send_interactive_prompt",
    "send_interactive_prompt_lines",
    "submit_interactive_reply",
]
