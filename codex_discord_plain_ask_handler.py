from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Callable
from dataclasses import dataclass
from typing import Generic, TypeVar

import codex_discord_plain_ask as plain_ask

MessageT = TypeVar("MessageT", bound=plain_ask.PlainAskMessage)
SendResultT = TypeVar("SendResultT")
HasRecentPromptSyncFunc = Callable[[str | None, str], bool]


@dataclass(frozen=True, slots=True)
class PlainAskHandlerDeps(Generic[MessageT, SendResultT]):
    get_interactive_state_for_thread: plain_ask.GetInteractiveStateFunc
    normalize_interactive_text_reply: plain_ask.NormalizeInteractiveReplyFunc
    send_interactive_prompt: plain_ask.SendInteractivePromptFunc
    submit_interactive_reply: plain_ask.SubmitInteractiveReplyFunc
    state_input: str
    state_approval: str
    has_recent_codex_app_user_prompt: HasRecentPromptSyncFunc
    is_thread_runner_busy: plain_ask.IsRunnerBusyFunc
    mark_recent_discord_origin_prompt: plain_ask.MarkRecentPromptFunc
    handle_busy_plain_ask: plain_ask.HandleBusyPlainAskFunc[MessageT]
    claim_direct_ask_target: plain_ask.ClaimDirectAskTargetFunc
    release_direct_ask_target: plain_ask.ReleaseDirectAskTargetFunc
    run_prompt_flow: plain_ask.RunPromptFlowFunc[MessageT]
    send_chunks: plain_ask.SendChunksFunc[SendResultT]
    format_log_text_len: plain_ask.FormatLogTextLenFunc
    log: Callable[[str], None]


async def _has_recent_codex_app_user_prompt(
    deps: PlainAskHandlerDeps[MessageT, SendResultT],
    target_thread_id: str | None,
    prompt: str,
) -> bool:
    return await asyncio.to_thread(
        deps.has_recent_codex_app_user_prompt,
        target_thread_id,
        prompt,
    )


async def handle_plain_ask_message(
    message: MessageT,
    prompt: str,
    *,
    target_thread_id: str | None,
    deps: PlainAskHandlerDeps[MessageT, SendResultT],
) -> None:
    interactive_result = await plain_ask.handle_interactive_plain_ask(
        message,
        prompt,
        target_thread_id,
        deps=plain_ask.PlainAskInteractiveDeps(
            get_interactive_state_for_thread=deps.get_interactive_state_for_thread,
            normalize_interactive_text_reply=deps.normalize_interactive_text_reply,
            send_interactive_prompt=deps.send_interactive_prompt,
            submit_interactive_reply=deps.submit_interactive_reply,
            state_input=deps.state_input,
            state_approval=deps.state_approval,
        ),
    )
    ask_target_thread_id = interactive_result.ask_target_thread_id
    if interactive_result.handled:
        return

    await plain_ask.handle_direct_plain_ask(
        message,
        prompt,
        ask_target_thread_id,
        deps=plain_ask.PlainAskDirectDeps(
            has_recent_codex_app_user_prompt=lambda target, text: _has_recent_codex_app_user_prompt(
                deps,
                target,
                text,
            ),
            is_thread_runner_busy=deps.is_thread_runner_busy,
            mark_recent_discord_origin_prompt=deps.mark_recent_discord_origin_prompt,
            handle_busy_plain_ask=deps.handle_busy_plain_ask,
            claim_direct_ask_target=deps.claim_direct_ask_target,
            release_direct_ask_target=deps.release_direct_ask_target,
            run_prompt_flow=deps.run_prompt_flow,
            send_chunks=deps.send_chunks,
            format_log_text_len=deps.format_log_text_len,
            log=deps.log,
        ),
    )
