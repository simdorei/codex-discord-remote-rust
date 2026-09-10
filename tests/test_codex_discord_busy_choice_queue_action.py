from __future__ import annotations

import unittest
from contextlib import asynccontextmanager
from dataclasses import dataclass
from typing import Literal

import codex_discord_busy_choice_queue_action as queue_action

USER_ID = 242286902982606848
PROMPT = "please queue"
TARGET_THREAD_ID = "thread-1"
POSITION = 4
QUEUED_CONTENT = f"Queued at position {POSITION}."
EventName = Literal["busy_state", "runner_busy", "followup", "enqueue", "log"]


@dataclass(frozen=True, slots=True)
class FakeInteraction:
    marker: str = "interaction"


@dataclass(frozen=True, slots=True)
class FakeChannel:
    channel_id: int = 222


@dataclass(frozen=True, slots=True)
class FakeSourceMessage:
    message_id: int = 333


@dataclass(frozen=True, slots=True)
class FollowupEvent:
    interaction: queue_action.QueueInteraction
    content: str
    log_prefix: str
    context: str


@dataclass(frozen=True, slots=True)
class EnqueueEvent:
    channel: queue_action.QueueChannel
    prompt: str
    target_thread_id: str | None
    queued: bool
    ack_sent: bool
    source_message: queue_action.QueueSourceMessage | None


@dataclass(frozen=True, slots=True)
class QueueHarness:
    deps: queue_action.BusyChoiceQueueActionDeps
    order: list[EventName]
    followups: list[FollowupEvent]
    enqueues: list[EnqueueEvent]
    logs: list[str]
    busy_state_calls: list[str | None]
    runner_busy_calls: list[str | None]


class BusyChoiceQueueActionTests(unittest.IsolatedAsyncioTestCase):
    async def test_immediate_path_logs_followup_and_enqueue_in_order(self) -> None:
        interaction = FakeInteraction()
        channel = FakeChannel()
        source_message = FakeSourceMessage()
        harness = self._make_harness(busy_state="idle", runner_busy=False)

        await self._run_action(harness, interaction, channel, source_message)

        self.assertEqual(harness.order, ["busy_state", "runner_busy", "log", "enqueue", "log"])
        self.assertEqual(
            harness.logs,
            [
                f"queue_next_immediate user={USER_ID} target={TARGET_THREAD_ID} prompt_len=12",
                f"queue_next_immediate_enqueued user={USER_ID} position={POSITION} target={TARGET_THREAD_ID}",
            ],
        )
        self.assertEqual(harness.followups, [])
        self.assertEqual(
            harness.enqueues,
            [EnqueueEvent(channel, PROMPT, TARGET_THREAD_ID, False, False, source_message)],
        )

    async def test_queued_path_enqueues_follows_up_and_logs_in_order(self) -> None:
        interaction = FakeInteraction()
        channel = FakeChannel()
        source_message = FakeSourceMessage()
        harness = self._make_harness(busy_state="busy", runner_busy=False)

        await self._run_action(harness, interaction, channel, source_message)

        self.assertEqual(harness.runner_busy_calls, [])
        self.assertEqual(harness.order, ["busy_state", "enqueue", "log", "followup", "log"])
        self.assertEqual(
            harness.logs,
            [
                f"queue_next user={USER_ID} position={POSITION} target={TARGET_THREAD_ID} prompt_len=12",
                f"queue_next_sent user={USER_ID} position={POSITION} target={TARGET_THREAD_ID}",
            ],
        )
        self.assertEqual(
            harness.followups,
            [FollowupEvent(interaction, QUEUED_CONTENT, "button_followup", "queue_next")],
        )
        self.assertEqual(
            harness.enqueues,
            [EnqueueEvent(channel, PROMPT, TARGET_THREAD_ID, True, False, source_message)],
        )

    async def test_idle_target_with_busy_runner_falls_back_to_queued_path(self) -> None:
        harness = self._make_harness(busy_state="idle", runner_busy=True)

        await self._run_action(harness, FakeInteraction(), FakeChannel(), FakeSourceMessage())

        self.assertEqual(harness.runner_busy_calls, [TARGET_THREAD_ID])
        self.assertEqual(harness.enqueues[0].queued, True)
        self.assertEqual(harness.followups[0].content, QUEUED_CONTENT)
        self.assertEqual(harness.followups[0].context, "queue_next")

    async def test_none_target_is_forwarded_and_logged_as_dash(self) -> None:
        harness = self._make_harness(busy_state="idle", runner_busy=False)

        await self._run_action(harness, FakeInteraction(), FakeChannel(), FakeSourceMessage(), target_thread_id=None, prompt="")

        self.assertEqual(harness.busy_state_calls, [None])
        self.assertEqual(harness.runner_busy_calls, [None])
        self.assertEqual(harness.enqueues[0].target_thread_id, None)
        self.assertEqual(
            harness.logs,
            [
                f"queue_next_immediate user={USER_ID} target=- prompt_len=0",
                f"queue_next_immediate_enqueued user={USER_ID} position={POSITION} target=-",
            ],
        )

    async def test_enqueue_exception_propagates_without_followup_substitution(self) -> None:
        harness = self._make_harness(busy_state="busy", runner_busy=False, enqueue_error=True)

        with self.assertRaisesRegex(RuntimeError, "enqueue failed"):
            await self._run_action(harness, FakeInteraction(), FakeChannel(), FakeSourceMessage())

        self.assertEqual(harness.followups, [])
        self.assertEqual(harness.logs, [])

    async def test_immediate_path_leaves_start_ack_to_queue_runner(self) -> None:
        harness = self._make_harness(busy_state="idle", runner_busy=False)

        await self._run_action(harness, FakeInteraction(), FakeChannel(), FakeSourceMessage())

        self.assertEqual(harness.followups, [])
        self.assertFalse(harness.enqueues[0].ack_sent)

    async def test_immediate_enqueue_failure_does_not_send_starting_followup(self) -> None:
        harness = self._make_harness(busy_state="idle", runner_busy=False, enqueue_error=True)

        with self.assertRaisesRegex(RuntimeError, "enqueue failed"):
            await self._run_action(harness, FakeInteraction(), FakeChannel(), FakeSourceMessage())

        self.assertEqual(harness.followups, [])
        self.assertEqual(len(harness.enqueues), 1)

    async def test_rejected_admission_stops_before_busy_ui_or_enqueue(self) -> None:
        harness = self._make_harness(busy_state="idle", runner_busy=False, admitted=False)

        await self._run_action(harness, FakeInteraction(), FakeChannel(), FakeSourceMessage())

        self.assertEqual(harness.order, [])
        self.assertEqual(harness.followups, [])
        self.assertEqual(harness.enqueues, [])

    async def _run_action(
        self,
        harness: QueueHarness,
        interaction: FakeInteraction,
        channel: FakeChannel,
        source_message: FakeSourceMessage,
        *,
        target_thread_id: str | None = TARGET_THREAD_ID,
        prompt: str = PROMPT,
    ) -> None:
        await queue_action.handle_busy_choice_queue_action(
            interaction,
            channel,
            source_message,
            prompt=prompt,
            target_thread_id=target_thread_id,
            user_id=USER_ID,
            deps=harness.deps,
        )

    def _make_harness(
        self,
        *,
        busy_state: str,
        runner_busy: bool,
        enqueue_error: bool = False,
        admitted: bool = True,
    ) -> QueueHarness:
        order: list[EventName] = []
        followups: list[FollowupEvent] = []
        enqueues: list[EnqueueEvent] = []
        logs: list[str] = []
        busy_state_calls: list[str | None] = []
        runner_busy_calls: list[str | None] = []

        async def get_busy_state(target_thread_id: str | None) -> queue_action.BusyState:
            busy_state_calls.append(target_thread_id)
            order.append("busy_state")
            return (busy_state, target_thread_id, "project:1")

        async def is_thread_runner_busy(target_thread_id: str | None) -> bool:
            runner_busy_calls.append(target_thread_id)
            order.append("runner_busy")
            return runner_busy

        async def send_followup(
            interaction: queue_action.QueueInteraction,
            content: str,
            *,
            log_prefix: str,
            context: str,
        ) -> None:
            followups.append(FollowupEvent(interaction, content, log_prefix, context))
            order.append("followup")

        async def enqueue_thread_ask(
            channel: queue_action.QueueChannel,
            prompt: str,
            target_thread_id: str | None,
            *,
            queued: bool = False,
            ack_sent: bool = False,
            source_message: queue_action.QueueSourceMessage | None = None,
        ) -> int:
            enqueues.append(EnqueueEvent(channel, prompt, target_thread_id, queued, ack_sent, source_message))
            order.append("enqueue")
            if enqueue_error:
                raise RuntimeError("enqueue failed")
            return POSITION

        def log(message: str) -> None:
            logs.append(message)
            order.append("log")

        @asynccontextmanager
        async def prompt_admission(
            _channel: queue_action.QueueChannel,
            _source_message: queue_action.QueueSourceMessage,
            _expected_generation: int | None,
        ):
            yield admitted

        deps = queue_action.BusyChoiceQueueActionDeps(
            get_busy_state_for_thread=get_busy_state,
            is_thread_runner_busy=is_thread_runner_busy,
            send_followup=send_followup,
            enqueue_thread_ask=enqueue_thread_ask,
            prompt_admission=prompt_admission,
            format_log_text_len=len,
            log=log,
        )
        return QueueHarness(deps, order, followups, enqueues, logs, busy_state_calls, runner_busy_calls)
