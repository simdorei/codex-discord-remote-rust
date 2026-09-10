from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from types import ModuleType
from typing import Protocol, TypeAlias, cast

import codex_discord_bot_interactive_runtime as discord_bot_interactive_runtime
import codex_discord_interactive_prompt_delivery as interactive_delivery
ModuleValue: TypeAlias = object


InteractiveChannel: TypeAlias = object
WatchResult: TypeAlias = object
SendResult: TypeAlias = object


class TrackedMessageSender(Protocol):
    def __call__(
        self,
        target: InteractiveChannel,
        content: str,
        *,
        view: interactive_delivery.InteractiveView,
        context: str,
    ) -> Awaitable[SendResult]: ...


@dataclass(frozen=True, slots=True)
class BotInteractiveAdapterRuntime:
    module: ModuleType

    def make_interactive_runtime(
        self,
    ) -> discord_bot_interactive_runtime.BotInteractiveRuntime[
        InteractiveChannel,
        WatchResult,
        SendResult,
    ]:
        return discord_bot_interactive_runtime.BotInteractiveRuntime(
            discord_bot_interactive_runtime.BotInteractiveRuntimeDeps(
                state_approval=cast(str, getattr(self.module, "INTERACTIVE_STATE_APPROVAL")),
                state_input=cast(str, getattr(self.module, "INTERACTIVE_STATE_INPUT")),
                approval_view_factory=self.make_approval_view,
                input_choice_view_factory=self.make_input_choice_view,
                send_message_tracked=self.send_message_tracked,
                send_chunks=self.send_chunks,
                fit_single_message=self.fit_single_message,
                make_post_approval_watch_result=self.make_post_approval_watch_result,
                submit_approval_reply=self.submit_approval_reply,
                submit_input_reply=self.submit_input_reply,
                stream_post_approval_result=self.stream_post_approval_result,
                format_log_text_len=self.format_log_text_len,
                log=self.log,
            )
        )

    def make_approval_view(self, target_thread_id: str) -> interactive_delivery.InteractiveView:
        return cast(
            interactive_delivery.InteractiveView,
            cast(Callable[[str], object], self._module_func("ApprovalView"))(target_thread_id),
        )

    def make_input_choice_view(
        self,
        target_thread_id: str,
        options: list[tuple[str, str]],
    ) -> interactive_delivery.InteractiveView:
        return cast(
            interactive_delivery.InteractiveView,
            cast(Callable[[str, list[tuple[str, str]]], object], self._module_func("InputChoiceView"))(
                target_thread_id,
                options,
            ),
        )

    async def send_message_tracked(
        self,
        target: InteractiveChannel,
        content: str,
        *,
        view: interactive_delivery.InteractiveView,
        context: str,
    ) -> SendResult:
        return await cast(
            TrackedMessageSender,
            self._module_func("send_message_tracked"),
        )(target, content, view=view, context=context)

    async def send_chunks(self, target: InteractiveChannel, text: str) -> SendResult:
        return await cast(
            Callable[[InteractiveChannel, str], Awaitable[SendResult]],
            self._module_func("send_chunks"),
        )(target, text)

    async def stream_post_approval_result(
        self,
        channel: InteractiveChannel,
        watch_result: WatchResult,
        target_thread_id: str | None,
    ) -> bool:
        return await cast(
            interactive_delivery.StreamApprovalResultFunc[InteractiveChannel, WatchResult],
            self._module_func("stream_post_approval_result_to_channel"),
        )(channel, watch_result, target_thread_id)

    def fit_single_message(self, text: str) -> str:
        return cast(Callable[[str], str], self._module_func("fit_single_message"))(text)

    def make_post_approval_watch_result(self, target_thread_id: str) -> WatchResult:
        return cast(
            Callable[[str], WatchResult],
            self._module_func("make_post_approval_watch_result"),
        )(target_thread_id)

    def submit_approval_reply(self, target_thread_id: str, answer: str) -> tuple[int, str]:
        return cast(
            Callable[[str, str], tuple[int, str]],
            self._module_func("submit_approval_reply"),
        )(target_thread_id, answer)

    def submit_input_reply(self, target_thread_id: str, answer: str) -> tuple[int, str]:
        return cast(
            Callable[[str, str], tuple[int, str]],
            self._module_func("submit_input_reply"),
        )(target_thread_id, answer)

    def format_log_text_len(self, text: str) -> interactive_delivery.LogLengthValue:
        return cast(
            Callable[[str], interactive_delivery.LogLengthValue],
            self._module_func("format_log_text_len"),
        )(text)

    def log(self, message: str) -> None:
        cast(Callable[[str], None], self._module_func("log_line"))(message)

    def _module_func(self, name: str) -> ModuleValue:
        return cast(object, getattr(self.module, name))
