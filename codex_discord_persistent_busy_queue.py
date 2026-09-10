from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Protocol

import codex_discord_persistent_busy_choice as discord_persistent_busy_choice
from codex_discord_bot_prompt_delivery_types import PromptAdmission

BusyState = tuple[str, str | None, str | None]
SyncBusyStateGetter = Callable[[str | None], BusyState]
BusyStateGetter = Callable[[str | None], Awaitable[BusyState]]
ThreadRunnerBusyChecker = Callable[[str | None], Awaitable[bool]]
LogFunc = Callable[[str], None]
LogTextLenFormatter = Callable[[str], int]


class QueueChannel(Protocol): ...


class QueueSourceMessage(Protocol): ...


class BusyDirectFollowupSender(Protocol):
    def __call__(
        self,
        interaction: discord_persistent_busy_choice.PersistentBusyInteraction,
        content: str,
        *,
        log_prefix: str,
        context: str,
    ) -> Awaitable[None]: ...


class ThreadAskEnqueuer(Protocol):
    def __call__(
        self,
        channel: QueueChannel,
        prompt: str,
        target_thread_id: str | None,
        *,
        queued: bool = False,
        ack_sent: bool = False,
        source_message: QueueSourceMessage | None = None,
    ) -> Awaitable[int]: ...


class QueueFollowupHandler(Protocol):
    def __call__(
        self,
        interaction: discord_persistent_busy_choice.PersistentBusyInteraction,
        *,
        user_id: int,
        choice_id: str,
        position: int,
        target_thread_id: str | None,
        prompt: str,
        deps: discord_persistent_busy_choice.PersistentBusyQueueFollowupDeps,
    ) -> Awaitable[bool]: ...


@dataclass(frozen=True, slots=True)
class PersistentBusyQueueActionDeps:
    get_busy_state_for_thread: BusyStateGetter
    is_thread_runner_busy: ThreadRunnerBusyChecker
    send_followup: BusyDirectFollowupSender
    enqueue_thread_ask: ThreadAskEnqueuer
    prompt_admission: PromptAdmission[QueueChannel]
    handle_queue_followup: QueueFollowupHandler
    format_log_text_len: LogTextLenFormatter
    log: LogFunc


def make_persistent_busy_queue_action_deps(
    *,
    get_busy_state_for_thread: SyncBusyStateGetter,
    is_thread_runner_busy: ThreadRunnerBusyChecker,
    send_followup: BusyDirectFollowupSender,
    enqueue_thread_ask: ThreadAskEnqueuer,
    prompt_admission: PromptAdmission[QueueChannel],
    handle_queue_followup: QueueFollowupHandler,
    format_log_text_len: LogTextLenFormatter,
    log: LogFunc,
) -> PersistentBusyQueueActionDeps:
    async def get_busy_state(target_thread_id: str | None) -> BusyState:
        return await asyncio.to_thread(get_busy_state_for_thread, target_thread_id)

    return PersistentBusyQueueActionDeps(
        get_busy_state_for_thread=get_busy_state,
        is_thread_runner_busy=is_thread_runner_busy,
        send_followup=send_followup,
        enqueue_thread_ask=enqueue_thread_ask,
        prompt_admission=prompt_admission,
        handle_queue_followup=handle_queue_followup,
        format_log_text_len=format_log_text_len,
        log=log,
    )


async def handle_persistent_busy_queue_action(
    interaction: discord_persistent_busy_choice.PersistentBusyInteraction,
    channel: QueueChannel,
    source_message: QueueSourceMessage,
    *,
    user_id: int,
    choice_id: str,
    target_thread_id: str | None,
    prompt: str,
    deps: PersistentBusyQueueActionDeps,
) -> bool:
    async with deps.prompt_admission(channel, source_message, None) as accepted:
        if not accepted:
            return False
        busy_state, _busy_thread_id, _busy_ref = await deps.get_busy_state_for_thread(target_thread_id)
        if busy_state == "idle" and not await deps.is_thread_runner_busy(target_thread_id):
            position = await deps.enqueue_thread_ask(
                channel,
                prompt,
                target_thread_id,
                queued=False,
                ack_sent=False,
                source_message=source_message,
            )
            prompt_len = deps.format_log_text_len(prompt)
            deps.log(f"busy_choice_persistent_queue_immediate user={user_id} choice={choice_id} position={position} target={target_thread_id or '-'} prompt_len={prompt_len}")
            return True

        position = await deps.enqueue_thread_ask(
            channel,
            prompt,
            target_thread_id,
            queued=True,
            source_message=source_message,
        )
        return await deps.handle_queue_followup(
            interaction,
            user_id=user_id,
            choice_id=choice_id,
            position=position,
            target_thread_id=target_thread_id,
            prompt=prompt,
            deps=discord_persistent_busy_choice.PersistentBusyQueueFollowupDeps(
                send_followup=deps.send_followup,
                log=deps.log,
            ),
        )
