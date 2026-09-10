from __future__ import annotations

from contextlib import AbstractAsyncContextManager
from dataclasses import dataclass
from functools import partial
from typing import Generic, cast

import codex_discord_busy as discord_busy
from codex_discord_bot_plain_ask_types import (
    BotPlainAskRuntimeDeps,
    MessageT,
    MessageableT,
    SendResultT,
    ViewT,
)
import codex_discord_plain_ask as discord_plain_ask
import codex_discord_plain_ask_runtime as discord_plain_ask_runtime
import codex_discord_prompt_flow as discord_prompt_flow


@dataclass(frozen=True, slots=True)
class BotPlainAskRuntime(Generic[MessageableT, MessageT, ViewT, SendResultT]):
    deps: BotPlainAskRuntimeDeps[MessageableT, MessageT, ViewT, SendResultT]

    async def run_prompt_flow(
        self,
        channel: MessageableT,
        prompt: str,
        *,
        queued: bool = False,
        source_message: MessageT | None = None,
        target_thread_id: str | None = None,
    ) -> None:
        async with self.deps.prompt_admission(channel, source_message, None) as accepted:
            if not accepted:
                return
            await discord_prompt_flow.send_prompt_flow_preamble(
                channel,
                prompt,
                target_thread_id,
                queued=queued,
                deps=discord_prompt_flow.PromptFlowPreambleDeps(
                    build_context_warning=self.deps.build_context_warning,
                    build_ask_start_message=self.deps.build_ask_start_message,
                    send_chunks=self.deps.send_prompt_chunks,
                ),
            )
            await self.deps.run_prompt_and_send(
                channel,
                prompt,
                queued=queued,
                ack_sent=True,
                source_message=source_message,
                target_thread_id=target_thread_id,
            )

    def make_busy_choice_payload(
        self,
        source_message: MessageT,
        prompt: str,
        *,
        target_thread_id: str | None = None,
        allow_steer: bool = True,
    ) -> tuple[str, ViewT]:
        build_busy_choice_message = partial(
            discord_busy.build_busy_choice_message,
            discord_max_len=self.deps.discord_max_len,
            fit_single_message_func=self.deps.fit_single_message,
        )
        return discord_busy.make_busy_choice_payload(
            source_message,
            prompt,
            target_thread_id=target_thread_id,
            allow_steer=allow_steer,
            build_busy_choice_message_func=build_busy_choice_message,
            make_busy_choice_view_func=self.deps.make_busy_choice_view,
        )

    async def send_busy_choice_message(
        self,
        channel: MessageableT,
        source_message: MessageT,
        prompt: str,
        *,
        target_thread_id: str | None,
        allow_steer: bool,
        reason: str,
    ) -> bool:
        async def send_error_chunks(target: MessageableT, text: str) -> SendResultT:
            return await self.deps.send_chunks(target, text)

        log_busy_choice_sent = partial(
            discord_busy.log_busy_choice_sent,
            log_func=self.deps.log,
            format_log_text_len_func=self.deps.format_log_text_len_as_text,
        )
        return await discord_busy.send_busy_choice_payload_message(
            channel,
            source_message,
            prompt,
            reason=reason,
            target_thread_id=target_thread_id,
            allow_steer=allow_steer,
            make_busy_choice_payload_func=self.make_busy_choice_payload,
            send_message_tracked_func=self.deps.send_message_tracked,
            send_chunks_func=send_error_chunks,
            log_busy_choice_sent_func=log_busy_choice_sent,
            format_log_text_len_func=self.deps.format_log_text_len_as_text,
            log_func=self.deps.log,
        )

    async def enqueue_plain_thread_ask(
        self,
        channel: discord_plain_ask.PlainAskChannel,
        prompt: str,
        target_thread_id: str | None,
        *,
        queued: bool = False,
        ack_sent: bool = False,
        source_message: MessageT | None = None,
    ) -> int:
        return await self.deps.enqueue_thread_ask(
            self.deps.require_messageable_channel(channel),
            prompt,
            target_thread_id,
            queued=queued,
            ack_sent=ack_sent,
            source_message=source_message,
        )

    async def send_plain_busy_choice_message(
        self,
        channel: discord_plain_ask.PlainAskChannel,
        source_message: MessageT,
        prompt: str,
        *,
        target_thread_id: str | None,
        allow_steer: bool,
        reason: str,
    ) -> bool:
        return await self.send_busy_choice_message(
            self.deps.require_messageable_channel(channel),
            source_message,
            prompt,
            target_thread_id=target_thread_id,
            allow_steer=allow_steer,
            reason=reason,
        )

    async def send_plain_ask_chunks(
        self,
        target: discord_plain_ask.PlainAskChannel,
        text: str,
        *,
        context: str = "send_chunks",
    ) -> int:
        result = await self.deps.send_chunks(
            self.deps.require_messageable_channel(target),
            text,
            context=context,
        )
        if isinstance(result, int):
            return result
        return 0

    async def handle_busy_plain_ask(
        self,
        message: MessageT,
        prompt: str,
        target_thread_id: str | None,
    ) -> None:
        await discord_plain_ask_runtime.handle_busy_plain_ask(
            message,
            prompt,
            target_thread_id,
            deps=self._make_plain_ask_runtime_deps(),
        )

    async def handle_plain_ask(
        self,
        message: MessageT,
        prompt: str,
        *,
        target_thread_id: str | None = None,
    ) -> None:
        await discord_plain_ask_runtime.handle_plain_ask(
            message,
            prompt,
            target_thread_id=target_thread_id,
            deps=self._make_plain_ask_runtime_deps(),
        )

    def _prompt_admission_for_plain_ask(
        self,
        channel: discord_plain_ask.PlainAskChannel,
        source_message: object | None,
        expected_generation: int | None,
        /,
    ) -> AbstractAsyncContextManager[bool]:
        return self.deps.prompt_admission(
            self.deps.require_messageable_channel(channel),
            source_message,
            expected_generation,
        )

    def _make_plain_ask_runtime_deps(
        self,
    ) -> discord_plain_ask_runtime.PlainAskRuntimeDeps[MessageT, int]:
        return discord_plain_ask_runtime.PlainAskRuntimeDeps(
            get_interactive_state_for_thread=self.deps.get_interactive_state_for_thread,
            send_interactive_prompt=self.deps.send_interactive_prompt,
            submit_interactive_reply=self.deps.submit_interactive_reply,
            state_input=self.deps.state_input,
            state_approval=self.deps.state_approval,
            has_recent_codex_app_user_prompt=self.deps.has_recent_codex_app_user_prompt,
            is_thread_runner_busy=self.deps.is_thread_runner_busy,
            mark_recent_discord_origin_prompt=self.deps.mark_recent_discord_origin_prompt,
            handle_busy_plain_ask=self.handle_busy_plain_ask,
            claim_direct_ask_target=self.deps.claim_direct_ask_target,
            release_direct_ask_target=self.deps.release_direct_ask_target,
            run_prompt_flow=cast(
                discord_plain_ask.RunPromptFlowFunc[MessageT],
                self.deps.run_plain_prompt_flow,
            ),
            prompt_admission=self._prompt_admission_for_plain_ask,
            enqueue_thread_ask=self.enqueue_plain_thread_ask,
            send_busy_choice_message=self.send_plain_busy_choice_message,
            send_chunks=self.send_plain_ask_chunks,
            format_log_text_len=self.deps.format_log_text_len,
            log=self.deps.log,
        )
