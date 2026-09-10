from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Protocol

from codex_discord_bot_prompt_delivery_types import PromptAdmission

BusyState = tuple[str, str | None, str]
BusyStateGetter = Callable[[str | None], Awaitable[BusyState]]
ThreadRunnerBusyChecker = Callable[[str | None], Awaitable[bool]]
LogFunc = Callable[[str], None]
LogTextLenFormatter = Callable[[str], int]


class QueueInteraction(Protocol): ...


class QueueChannel(Protocol): ...


class QueueSourceMessage(Protocol): ...


class BusyDirectFollowupSender(Protocol):
    def __call__(
        self,
        interaction: QueueInteraction,
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


@dataclass(frozen=True, slots=True)
class BusyChoiceQueueActionDeps:
    get_busy_state_for_thread: BusyStateGetter
    is_thread_runner_busy: ThreadRunnerBusyChecker
    send_followup: BusyDirectFollowupSender
    enqueue_thread_ask: ThreadAskEnqueuer
    prompt_admission: PromptAdmission[QueueChannel]
    format_log_text_len: LogTextLenFormatter
    log: LogFunc


async def handle_busy_choice_queue_action(
    interaction: QueueInteraction,
    channel: QueueChannel,
    source_message: QueueSourceMessage,
    *,
    prompt: str,
    target_thread_id: str | None,
    user_id: int,
    deps: BusyChoiceQueueActionDeps,
) -> None:
    async with deps.prompt_admission(channel, source_message, None) as accepted:
        if not accepted:
            return
        busy_state, _busy_thread_id, _busy_ref = await deps.get_busy_state_for_thread(
            target_thread_id,
        )
        target_log = target_thread_id or "-"
        prompt_len = deps.format_log_text_len(prompt)
        if busy_state == "idle" and not await deps.is_thread_runner_busy(target_thread_id):
            immediate_log = f"queue_next_immediate user={user_id} target={target_log} prompt_len={prompt_len}"
            deps.log(immediate_log)
            position = await deps.enqueue_thread_ask(
                channel,
                prompt,
                target_thread_id,
                queued=False,
                ack_sent=False,
                source_message=source_message,
            )
            immediate_enqueued_log = f"queue_next_immediate_enqueued user={user_id} position={position} target={target_log}"
            deps.log(immediate_enqueued_log)
            return

        position = await deps.enqueue_thread_ask(
            channel,
            prompt,
            target_thread_id,
            queued=True,
            source_message=source_message,
        )
        queued_log = f"queue_next user={user_id} position={position} target={target_log} prompt_len={prompt_len}"
        deps.log(queued_log)
        await deps.send_followup(
            interaction,
            f"Queued at position {position}.",
            log_prefix="button_followup",
            context="queue_next",
        )
        deps.log(f"queue_next_sent user={user_id} position={position} target={target_log}")
