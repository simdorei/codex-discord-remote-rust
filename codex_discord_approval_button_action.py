from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Protocol

LogFunc = Callable[[str], None]
LogTextLenFormatter = Callable[[str | None], int]
ApprovalSubmitter = Callable[[str, str], tuple[int, str]]


class ApprovalUser(Protocol):
    @property
    def id(self) -> int | str | None: ...


class ApprovalInteraction(Protocol):
    @property
    def user(self) -> ApprovalUser: ...


class ApprovalWatchResult(Protocol):
    pass


class ApprovalWatchMaker(Protocol):
    def __call__(self, target_thread_id: str) -> ApprovalWatchResult: ...


class FollowupChunkSender(Protocol):
    def __call__(
        self,
        interaction: ApprovalInteraction,
        content: str,
        *,
        title: str,
        exit_code: int,
        log_prefix: str,
    ) -> Awaitable[None]: ...


class PostApprovalResultStreamer(Protocol):
    def __call__(
        self,
        interaction: ApprovalInteraction,
        watch_result: ApprovalWatchResult,
        target_thread_id: str,
    ) -> Awaitable[None]: ...


@dataclass(frozen=True, slots=True)
class ApprovalButtonActionDeps:
    make_post_approval_watch_result: ApprovalWatchMaker
    submit_approval_reply: ApprovalSubmitter
    send_followup_chunks: FollowupChunkSender
    stream_post_approval_result: PostApprovalResultStreamer
    format_log_text_len: LogTextLenFormatter
    log: LogFunc


async def handle_approval_button_submit(
    interaction: ApprovalInteraction,
    target_thread_id: str,
    answer: str,
    *,
    deps: ApprovalButtonActionDeps,
) -> None:
    user_id = interaction.user.id or "-"
    answer_len = deps.format_log_text_len(answer)
    deps.log(f"approval_button user={user_id} answer_len={answer_len}")
    watch_result = deps.make_post_approval_watch_result(target_thread_id)
    exit_code, output = await asyncio.to_thread(deps.submit_approval_reply, target_thread_id, answer)
    deps.log(f"approval_button_done exit={exit_code} target={target_thread_id} answer_len={answer_len}")
    title = "Approval submitted" if exit_code == 0 else f"Approval failed (exit {exit_code})"
    await deps.send_followup_chunks(
        interaction,
        f"{title}\n\n{output or '(no output)'}",
        title="Approval",
        exit_code=exit_code,
        log_prefix="button_response",
    )
    deps.log(f"approval_button_sent exit={exit_code} target={target_thread_id}")
    if exit_code == 0:
        await deps.stream_post_approval_result(
            interaction,
            watch_result,
            target_thread_id,
        )
