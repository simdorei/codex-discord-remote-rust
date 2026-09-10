from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Protocol

LogFunc = Callable[[str], None]
LogTextLenFormatter = Callable[[str | None], int]
InputSubmitter = Callable[[str, str], tuple[int, str]]


class InputUser(Protocol):
    @property
    def id(self) -> int | str | None: ...


class InputInteraction(Protocol):
    @property
    def user(self) -> InputUser: ...


class FollowupChunkSender(Protocol):
    def __call__(
        self,
        interaction: InputInteraction,
        content: str,
        *,
        title: str,
        exit_code: int,
        log_prefix: str,
    ) -> Awaitable[None]: ...


@dataclass(frozen=True, slots=True)
class InputChoiceButtonActionDeps:
    submit_input_reply: InputSubmitter
    send_followup_chunks: FollowupChunkSender
    format_log_text_len: LogTextLenFormatter
    log: LogFunc


async def handle_input_choice_button_submit(
    interaction: InputInteraction,
    target_thread_id: str,
    value: str,
    *,
    deps: InputChoiceButtonActionDeps,
) -> None:
    user_id = interaction.user.id or "-"
    value_len = deps.format_log_text_len(value)
    deps.log(f"input_choice_button user={user_id} value_len={value_len}")
    exit_code, output = await asyncio.to_thread(deps.submit_input_reply, target_thread_id, value)
    deps.log(f"input_choice_button_done exit={exit_code} target={target_thread_id} value_len={value_len}")
    title = "Input submitted" if exit_code == 0 else f"Input failed (exit {exit_code})"
    await deps.send_followup_chunks(
        interaction,
        f"{title}\n\n{output or '(no output)'}",
        title="Input",
        exit_code=exit_code,
        log_prefix="button_response",
    )
    deps.log(f"input_choice_button_sent exit={exit_code} target={target_thread_id}")
