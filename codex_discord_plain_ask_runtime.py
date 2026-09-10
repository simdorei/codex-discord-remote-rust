from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from typing import Generic, Protocol, TypeVar

import codex_discord_busy_prompt as discord_busy_prompt
from codex_discord_bot_prompt_delivery_types import PromptAdmission
import codex_discord_interactive as discord_interactive
import codex_discord_plain_ask as discord_plain_ask
import codex_discord_plain_ask_handler as discord_plain_ask_handler


class PlainBusyAskMessage(
    discord_plain_ask.PlainAskMessage,
    discord_busy_prompt.BusyPromptMessage,
    Protocol,
):
    pass


MessageT = TypeVar("MessageT", bound=PlainBusyAskMessage)
SendResultT = TypeVar("SendResultT")


@dataclass(frozen=True, slots=True)
class PlainAskRuntimeDeps(Generic[MessageT, SendResultT]):
    get_interactive_state_for_thread: discord_plain_ask.GetInteractiveStateFunc
    send_interactive_prompt: discord_plain_ask.SendInteractivePromptFunc
    submit_interactive_reply: discord_plain_ask.SubmitInteractiveReplyFunc
    state_input: str
    state_approval: str
    has_recent_codex_app_user_prompt: discord_plain_ask_handler.HasRecentPromptSyncFunc
    is_thread_runner_busy: discord_plain_ask.IsRunnerBusyFunc
    mark_recent_discord_origin_prompt: discord_plain_ask.MarkRecentPromptFunc
    handle_busy_plain_ask: discord_plain_ask.HandleBusyPlainAskFunc[MessageT]
    claim_direct_ask_target: discord_plain_ask.ClaimDirectAskTargetFunc
    release_direct_ask_target: discord_plain_ask.ReleaseDirectAskTargetFunc
    run_prompt_flow: discord_plain_ask.RunPromptFlowFunc[MessageT]
    prompt_admission: PromptAdmission[discord_plain_ask.PlainAskChannel]
    enqueue_thread_ask: discord_busy_prompt.EnqueueThreadAsk[discord_plain_ask.PlainAskChannel, MessageT]
    send_busy_choice_message: discord_busy_prompt.SendBusyChoiceMessage[
        discord_plain_ask.PlainAskChannel,
        MessageT,
    ]
    send_chunks: discord_busy_prompt.SendChunks[discord_plain_ask.PlainAskChannel, SendResultT]
    format_log_text_len: Callable[[str | None], int | str]
    log: Callable[[str], None]


async def handle_busy_plain_ask(
    message: MessageT,
    prompt: str,
    target_thread_id: str | None,
    *,
    deps: PlainAskRuntimeDeps[MessageT, SendResultT],
) -> None:
    await discord_busy_prompt.handle_busy_prompt(
        message.channel,
        message,
        prompt,
        target_thread_id=target_thread_id,
        allow_steer=True,
        reason="same_thread_runner_busy",
        deps=discord_busy_prompt.BusyPromptDeps(
            enqueue_thread_ask=deps.enqueue_thread_ask,
            send_busy_choice_message=deps.send_busy_choice_message,
            send_chunks=deps.send_chunks,
            format_log_text_len=deps.format_log_text_len,
            log=deps.log,
        ),
    )


async def handle_plain_ask(
    message: MessageT,
    prompt: str,
    *,
    target_thread_id: str | None = None,
    deps: PlainAskRuntimeDeps[MessageT, SendResultT],
) -> None:
    async def send_plain_chunks(
        channel: discord_plain_ask.PlainAskChannel,
        text: str,
        *,
        context: str = "send_chunks",
    ) -> SendResultT:
        return await deps.send_chunks(channel, text, context=context)

    async with deps.prompt_admission(message.channel, message, None) as accepted:
        if not accepted:
            return
        await discord_plain_ask_handler.handle_plain_ask_message(
            message,
            prompt,
            target_thread_id=target_thread_id,
            deps=discord_plain_ask_handler.PlainAskHandlerDeps(
                get_interactive_state_for_thread=deps.get_interactive_state_for_thread,
                normalize_interactive_text_reply=lambda state, text: discord_interactive.normalize_interactive_text_reply(
                    state,
                    text,
                    state_input=deps.state_input,
                    state_approval=deps.state_approval,
                ),
                send_interactive_prompt=deps.send_interactive_prompt,
                submit_interactive_reply=deps.submit_interactive_reply,
                state_input=deps.state_input,
                state_approval=deps.state_approval,
                has_recent_codex_app_user_prompt=deps.has_recent_codex_app_user_prompt,
                is_thread_runner_busy=deps.is_thread_runner_busy,
                mark_recent_discord_origin_prompt=deps.mark_recent_discord_origin_prompt,
                handle_busy_plain_ask=deps.handle_busy_plain_ask,
                claim_direct_ask_target=deps.claim_direct_ask_target,
                release_direct_ask_target=deps.release_direct_ask_target,
                run_prompt_flow=deps.run_prompt_flow,
                send_chunks=send_plain_chunks,
                format_log_text_len=deps.format_log_text_len,
                log=deps.log,
            ),
        )
