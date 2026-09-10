from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from typing import Generic, TypeVar

import codex_discord_interactive_prompt_delivery as interactive_delivery


ChannelT = TypeVar("ChannelT")
WatchResultT = TypeVar("WatchResultT")
SendResultT = TypeVar("SendResultT")


@dataclass(frozen=True, slots=True)
class BotInteractiveRuntimeDeps(Generic[ChannelT, WatchResultT, SendResultT]):
    state_approval: str
    state_input: str
    approval_view_factory: Callable[[str], interactive_delivery.InteractiveView]
    input_choice_view_factory: Callable[
        [str, list[tuple[str, str]]],
        interactive_delivery.InteractiveView,
    ]
    send_message_tracked: interactive_delivery.SendMessageTrackedFunc[ChannelT, SendResultT]
    send_chunks: interactive_delivery.SendChunksFunc[ChannelT, SendResultT]
    fit_single_message: Callable[[str], str]
    make_post_approval_watch_result: Callable[[str], WatchResultT]
    submit_approval_reply: Callable[[str, str], tuple[int, str]]
    submit_input_reply: Callable[[str, str], tuple[int, str]]
    stream_post_approval_result: interactive_delivery.StreamApprovalResultFunc[ChannelT, WatchResultT]
    format_log_text_len: Callable[[str], interactive_delivery.LogLengthValue]
    log: Callable[[str], None]


@dataclass(frozen=True, slots=True)
class BotInteractiveRuntime(Generic[ChannelT, WatchResultT, SendResultT]):
    deps: BotInteractiveRuntimeDeps[ChannelT, WatchResultT, SendResultT]

    async def send_interactive_prompt(
        self,
        channel: ChannelT,
        target_thread_id: str,
        target_ref: str,
        state: str,
        prompt: str,
        options: list[tuple[str, str]],
    ) -> None:
        await interactive_delivery.send_interactive_prompt(
            channel,
            target_thread_id,
            target_ref,
            state,
            prompt,
            options,
            state_approval=self.deps.state_approval,
            state_input=self.deps.state_input,
            approval_view_factory=self.deps.approval_view_factory,
            input_choice_view_factory=self.deps.input_choice_view_factory,
            send_message_tracked_func=self.deps.send_message_tracked,
            send_chunks_func=self.deps.send_chunks,
            fit_single_message_func=self.deps.fit_single_message,
        )

    async def submit_interactive_reply(
        self,
        channel: ChannelT,
        target_thread_id: str,
        target_ref: str,
        state: str,
        answer: str,
    ) -> None:
        _ = target_ref
        await interactive_delivery.submit_interactive_reply(
            channel,
            target_thread_id,
            state,
            answer,
            state_approval=self.deps.state_approval,
            state_input=self.deps.state_input,
            make_post_approval_watch_result=self.deps.make_post_approval_watch_result,
            submit_approval_reply_func=self.deps.submit_approval_reply,
            submit_input_reply_func=self.deps.submit_input_reply,
            stream_post_approval_result_func=self.deps.stream_post_approval_result,
            send_chunks_func=self.deps.send_chunks,
            format_log_text_len_func=self.deps.format_log_text_len,
            log_func=self.deps.log,
        )
