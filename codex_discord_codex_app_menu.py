from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Protocol

import discord

import codex_discord_interactive as discord_interactive

LogFunc = Callable[[str], None]
InteractiveStateGetter = Callable[[str | None], tuple[str, str | None, str]]
TargetRefResolver = Callable[[str | None], tuple[str | None, str]]


class InteractivePromptSender(Protocol):
    def __call__(
        self,
        channel: discord.abc.Messageable,
        target_thread_id: str,
        target_ref: str,
        state: str,
        prompt: str,
        options: list[tuple[str, str]],
    ) -> Awaitable[None]: ...


@dataclass(frozen=True, slots=True)
class CodexAppMenuDeps:
    get_interactive_state_for_thread: InteractiveStateGetter
    resolve_target_ref: TargetRefResolver
    send_interactive_prompt: InteractivePromptSender
    state_none: str
    state_input: str
    state_approval: str
    log: LogFunc


async def send_codex_app_menu_if_available(
    channel: discord.abc.Messageable,
    target_thread_id: str | None,
    output: str,
    *,
    reason: str,
    deps: CodexAppMenuDeps,
) -> bool:
    state, resolved_thread_id, target_ref = deps.get_interactive_state_for_thread(target_thread_id)
    if not state:
        state = discord_interactive.infer_interactive_state_from_error(
            output,
            state_none=deps.state_none,
            state_input=deps.state_input,
            state_approval=deps.state_approval,
        )
        if state:
            resolved_thread_id, target_ref = deps.resolve_target_ref(target_thread_id)
    if not state or not resolved_thread_id:
        return False

    prompt_text = "Pending approval" if state == deps.state_approval else "Pending input"
    await deps.send_interactive_prompt(
        channel,
        resolved_thread_id,
        target_ref,
        state,
        prompt_text,
        [],
    )
    deps.log(
        f"codex_app_menu_sent reason={reason} target={resolved_thread_id or '-'} "
        + f"state={state}"
    )
    return True
