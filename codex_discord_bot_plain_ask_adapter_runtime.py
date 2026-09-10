from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from types import ModuleType
from typing import cast

import codex_discord_bot_plain_ask_busy_view_runtime as discord_bot_plain_ask_busy_view_runtime
import codex_discord_bot_plain_ask_runtime as discord_bot_plain_ask_runtime
import codex_discord_bot_plain_ask_types as discord_bot_plain_ask_types
import codex_discord_app_server_admission_factory as discord_app_server_admission_factory
from codex_discord_bot_plain_ask_adapter_types import (
    BusyChoiceMessage,
    BusyChoiceViewValue,
    MessageableChannel,
    RuntimeInteractivePromptSender,
    SendResult,
)
import codex_discord_busy as discord_busy
import codex_discord_plain_ask as discord_plain_ask
import codex_discord_prompt_flow as discord_prompt_flow
from codex_discord_bot_plain_ask_adapter_state import BotPlainAskAdapterStateMixin


@dataclass(frozen=True, slots=True)
class BotPlainAskAdapterRuntime(BotPlainAskAdapterStateMixin):
    module: ModuleType

    def make_plain_ask_runtime(
        self,
    ) -> discord_bot_plain_ask_runtime.BotPlainAskRuntime[
        MessageableChannel,
        BusyChoiceMessage,
        BusyChoiceViewValue,
        SendResult,
    ]:
        busy_view_runtime = discord_bot_plain_ask_busy_view_runtime.BotPlainAskBusyViewRuntime(self.module)
        return discord_bot_plain_ask_runtime.BotPlainAskRuntime(
            discord_bot_plain_ask_types.BotPlainAskRuntimeDeps(
                discord_max_len=cast(int, getattr(self.module, "DISCORD_MAX_LEN")),
                fit_single_message=cast(discord_busy.FitSingleMessage, self._module_func("fit_single_message")),
                require_messageable_channel=self.require_messageable_channel,
                make_busy_choice_view=busy_view_runtime.make_busy_choice_view,
                send_message_tracked=self.send_message_tracked,
                send_chunks=self.send_chunks,
                send_prompt_chunks=self.send_prompt_chunks,
                enqueue_thread_ask=self.enqueue_thread_ask,
                run_prompt_and_send=self.run_prompt_and_send,
                prompt_admission=discord_app_server_admission_factory.make_prompt_admission(
                    self.module,
                    self.send_chunks,
                ),
                run_plain_prompt_flow=self.run_plain_prompt_flow,
                build_context_warning=self.build_context_warning,
                build_ask_start_message=self.build_ask_start_message,
                get_interactive_state_for_thread=self.get_interactive_state_for_thread,
                send_interactive_prompt=self.send_interactive_prompt,
                submit_interactive_reply=self.submit_interactive_reply,
                state_input=cast(str, getattr(self.module, "INTERACTIVE_STATE_INPUT")),
                state_approval=cast(str, getattr(self.module, "INTERACTIVE_STATE_APPROVAL")),
                has_recent_codex_app_user_prompt=self.has_recent_codex_app_user_prompt,
                is_thread_runner_busy=self.is_thread_runner_busy,
                mark_recent_discord_origin_prompt=self.mark_recent_discord_origin_prompt,
                claim_direct_ask_target=self.claim_direct_ask_target,
                release_direct_ask_target=self.release_direct_ask_target,
                format_log_text_len=cast(Callable[[str | None], int | str], self._module_func("format_log_text_len")),
                format_log_text_len_as_text=cast(
                    discord_busy.FormatLogTextLen,
                    self._module_func("format_log_text_len_as_text"),
                ),
                log=cast(Callable[[str], None], self._module_func("log_line")),
            )
        )

    def require_messageable_channel(self, channel: discord_plain_ask.PlainAskChannel) -> MessageableChannel:
        return cast(
            Callable[[discord_plain_ask.PlainAskChannel], MessageableChannel],
            self._module_func("require_discord_messageable_channel"),
        )(channel)

    async def send_message_tracked(
        self,
        target: MessageableChannel,
        content: str,
        *,
        view: BusyChoiceViewValue,
        context: str,
    ) -> SendResult:
        return await cast(
            discord_bot_plain_ask_types.MessageSender[
                MessageableChannel,
                BusyChoiceViewValue,
                SendResult,
            ],
            self._module_func("send_message_tracked"),
        )(target, content, view=view, context=context)

    async def send_chunks(
        self,
        target: MessageableChannel,
        text: str,
        *,
        context: str | None = None,
    ) -> SendResult:
        return await cast(
            discord_bot_plain_ask_types.ChunkSender[MessageableChannel, SendResult],
            self._module_func("send_chunks"),
        )(target, text, context=context or "send_chunks")

    async def send_prompt_chunks(
        self,
        channel: MessageableChannel,
        content: str,
        *,
        context: str | None = None,
    ) -> None:
        await cast(
            discord_prompt_flow.ChunkSender[MessageableChannel],
            self._module_func("send_prompt_chunks"),
        )(channel, content, context=context)

    async def enqueue_thread_ask(
        self,
        channel: MessageableChannel,
        prompt: str,
        target_thread_id: str | None,
        *,
        queued: bool = False,
        ack_sent: bool = False,
        source_message: BusyChoiceMessage | None = None,
    ) -> int:
        return await cast(
            discord_bot_plain_ask_types.ThreadAskEnqueuer[MessageableChannel, BusyChoiceMessage],
            self._module_func("enqueue_thread_ask"),
        )(
            channel,
            prompt,
            target_thread_id,
            queued=queued,
            ack_sent=ack_sent,
            source_message=source_message,
        )

    async def run_prompt_and_send(
        self,
        channel: MessageableChannel,
        prompt: str,
        *,
        queued: bool = False,
        ack_sent: bool = False,
        source_message: BusyChoiceMessage | None = None,
        target_thread_id: str | None = None,
    ) -> None:
        await cast(
            discord_bot_plain_ask_types.PromptSender[MessageableChannel, BusyChoiceMessage],
            self._module_func("run_prompt_and_send"),
        )(
            channel,
            prompt,
            queued=queued,
            ack_sent=ack_sent,
            source_message=source_message,
            target_thread_id=target_thread_id,
        )

    async def run_plain_prompt_flow(
        self,
        channel: MessageableChannel,
        prompt: str,
        *,
        source_message: BusyChoiceMessage,
        target_thread_id: str | None,
    ) -> None:
        await cast(
            discord_bot_plain_ask_types.PlainPromptFlowRunner[MessageableChannel, BusyChoiceMessage],
            self._module_func("run_prompt_flow"),
        )(
            channel,
            prompt,
            source_message=source_message,
            target_thread_id=target_thread_id,
        )

    def build_context_warning(self, target_thread_id: str | None) -> str:
        return cast(Callable[[str | None], str], self._module_func("build_context_warning"))(target_thread_id)

    def build_ask_start_message(self, prompt: str, *, queued: bool = False) -> str:
        return cast(discord_prompt_flow.QueuedAskStartBuilder, self._module_func("build_ask_start_message"))(
            prompt,
            queued=queued,
        )

    def get_interactive_state_for_thread(self, target_thread_id: str | None) -> discord_plain_ask.InteractiveStateResult:
        return cast(
            discord_plain_ask.GetInteractiveStateFunc,
            self._module_func("get_interactive_state_for_thread"),
        )(target_thread_id)

    async def send_interactive_prompt(
        self,
        channel: discord_plain_ask.PlainAskChannel,
        target_thread_id: str,
        target_ref: str,
        state: str,
        prompt: str,
        options: list[str],
    ) -> None:
        await cast(RuntimeInteractivePromptSender, self._module_func("send_interactive_prompt"))(
            self.require_messageable_channel(channel),
            target_thread_id,
            target_ref,
            state,
            prompt,
            cast(list[tuple[str, str]], cast(object, options)),
        )

    async def submit_interactive_reply(
        self,
        channel: discord_plain_ask.PlainAskChannel,
        target_thread_id: str,
        target_ref: str,
        state: str,
        answer: str,
    ) -> None:
        await cast(discord_plain_ask.SubmitInteractiveReplyFunc, self._module_func("submit_interactive_reply"))(
            self.require_messageable_channel(channel),
            target_thread_id,
            target_ref,
            state,
            answer,
        )
