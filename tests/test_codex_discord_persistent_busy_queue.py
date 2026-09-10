from __future__ import annotations

import unittest
from contextlib import asynccontextmanager
from dataclasses import dataclass

import codex_discord_persistent_busy_choice as persistent_busy_choice
import codex_discord_persistent_busy_queue as persistent_busy_queue

USER_ID = 242286902982606848
CHOICE_ID = "0123456789abcdef01234567"
PROMPT = "please queue"
TARGET_THREAD_ID = "thread-1"


@dataclass(frozen=True, slots=True)
class FakeInteraction:
    marker: str = "interaction"


@dataclass(frozen=True, slots=True)
class FakeChannel:
    channel_id: int = 222


@dataclass(frozen=True, slots=True)
class FakeSourceMessage:
    author_id: int = 242286902982606848


@dataclass(frozen=True, slots=True)
class FollowupCall:
    interaction: persistent_busy_choice.PersistentBusyInteraction
    content: str
    log_prefix: str
    context: str


@dataclass(frozen=True, slots=True)
class EnqueueCall:
    channel: persistent_busy_queue.QueueChannel
    prompt: str
    target_thread_id: str | None
    queued: bool
    ack_sent: bool
    source_message: persistent_busy_queue.QueueSourceMessage | None


@dataclass(frozen=True, slots=True)
class QueueHarness:
    deps: persistent_busy_queue.PersistentBusyQueueActionDeps
    busy_state_calls: list[str | None]
    runner_busy_calls: list[str | None]
    followups: list[FollowupCall]
    enqueues: list[EnqueueCall]
    queue_followups: list[tuple[persistent_busy_choice.PersistentBusyInteraction, int, str, int, str | None, str]]
    logs: list[str]


class PersistentBusyQueueActionTests(unittest.IsolatedAsyncioTestCase):
    async def test_idle_target_starts_immediate_enqueue_and_logs(self) -> None:
        interaction = FakeInteraction()
        channel = FakeChannel()
        source_message = FakeSourceMessage()
        harness = self._make_harness(busy_state="idle", runner_busy=False)

        handled = await self._run_action(
            harness,
            interaction,
            channel,
            source_message,
            user_id=USER_ID,
        )

        self.assertTrue(handled)
        self.assertEqual(harness.busy_state_calls, [TARGET_THREAD_ID])
        self.assertEqual(harness.runner_busy_calls, [TARGET_THREAD_ID])
        self.assertEqual(harness.followups, [])
        self.assertEqual(
            harness.enqueues,
            [
                EnqueueCall(
                    channel=channel,
                    prompt=PROMPT,
                    target_thread_id=TARGET_THREAD_ID,
                    queued=False,
                    ack_sent=False,
                    source_message=source_message,
                )
            ],
        )
        self.assertEqual(harness.queue_followups, [])
        self.assertEqual(
            harness.logs,
            [f"busy_choice_persistent_queue_immediate user={USER_ID} choice={CHOICE_ID} position=4 target={TARGET_THREAD_ID} prompt_len=12"],
        )

    async def test_busy_target_enqueues_queued_and_uses_queue_followup(self) -> None:
        interaction = FakeInteraction()
        channel = FakeChannel()
        source_message = FakeSourceMessage()
        harness = self._make_harness(busy_state="busy", runner_busy=False)

        handled = await self._run_action(harness, interaction, channel, source_message)

        self.assertTrue(handled)
        self.assertEqual(harness.busy_state_calls, [TARGET_THREAD_ID])
        self.assertEqual(harness.runner_busy_calls, [])
        self.assertEqual(harness.followups, [])
        self.assertEqual(
            harness.enqueues,
            [
                EnqueueCall(
                    channel=channel,
                    prompt=PROMPT,
                    target_thread_id=TARGET_THREAD_ID,
                    queued=True,
                    ack_sent=False,
                    source_message=source_message,
                )
            ],
        )
        self.assertEqual(
            harness.queue_followups,
            [(interaction, 1, CHOICE_ID, 4, TARGET_THREAD_ID, PROMPT)],
        )

    async def test_idle_target_with_busy_runner_falls_back_to_queued_path(self) -> None:
        harness = self._make_harness(busy_state="idle", runner_busy=True)

        handled = await self._run_action(harness, FakeInteraction(), FakeChannel(), FakeSourceMessage())

        self.assertTrue(handled)
        self.assertEqual(harness.runner_busy_calls, [TARGET_THREAD_ID])
        self.assertEqual(harness.followups, [])
        self.assertEqual(harness.enqueues[0].queued, True)
        self.assertEqual(len(harness.queue_followups), 1)

    async def test_none_target_is_forwarded_and_logged_as_dash(self) -> None:
        harness = self._make_harness(busy_state="idle", runner_busy=False)

        handled = await self._run_action(
            harness,
            FakeInteraction(),
            FakeChannel(),
            FakeSourceMessage(),
            target_thread_id=None,
            prompt="",
        )

        self.assertTrue(handled)
        self.assertEqual(harness.busy_state_calls, [None])
        self.assertEqual(harness.runner_busy_calls, [None])
        self.assertIsNone(harness.enqueues[0].target_thread_id)
        self.assertEqual(
            harness.logs,
            [f"busy_choice_persistent_queue_immediate user=1 choice={CHOICE_ID} position=4 target=- prompt_len=0"],
        )

    async def test_enqueue_exception_propagates(self) -> None:
        harness = self._make_harness(busy_state="busy", runner_busy=False, enqueue_error=True)

        with self.assertRaisesRegex(RuntimeError, "enqueue failed"):
            _ = await self._run_action(harness, FakeInteraction(), FakeChannel(), FakeSourceMessage())

        self.assertEqual(harness.queue_followups, [])

    async def test_immediate_enqueue_failure_does_not_send_starting_followup(self) -> None:
        harness = self._make_harness(busy_state="idle", runner_busy=False, enqueue_error=True)

        with self.assertRaisesRegex(RuntimeError, "enqueue failed"):
            _ = await self._run_action(harness, FakeInteraction(), FakeChannel(), FakeSourceMessage())

        self.assertEqual(harness.followups, [])
        self.assertEqual(len(harness.enqueues), 1)

    async def test_rejected_admission_stops_before_busy_ui_or_enqueue(self) -> None:
        harness = self._make_harness(busy_state="idle", runner_busy=False, admitted=False)

        handled = await self._run_action(harness, FakeInteraction(), FakeChannel(), FakeSourceMessage())

        self.assertFalse(handled)
        self.assertEqual(harness.busy_state_calls, [])
        self.assertEqual(harness.runner_busy_calls, [])
        self.assertEqual(harness.followups, [])
        self.assertEqual(harness.enqueues, [])
        self.assertEqual(harness.queue_followups, [])
        self.assertEqual(harness.logs, [])

    async def _run_action(
        self,
        harness: QueueHarness,
        interaction: FakeInteraction,
        channel: FakeChannel,
        source_message: FakeSourceMessage,
        *,
        user_id: int = 1,
        target_thread_id: str | None = TARGET_THREAD_ID,
        prompt: str = PROMPT,
    ) -> bool:
        return await persistent_busy_queue.handle_persistent_busy_queue_action(
            interaction,
            channel,
            source_message,
            user_id=user_id,
            choice_id=CHOICE_ID,
            target_thread_id=target_thread_id,
            prompt=prompt,
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
        busy_state_calls: list[str | None] = []
        runner_busy_calls: list[str | None] = []
        followups: list[FollowupCall] = []
        enqueues: list[EnqueueCall] = []
        queue_followups: list[tuple[persistent_busy_choice.PersistentBusyInteraction, int, str, int, str | None, str]] = []
        logs: list[str] = []

        async def get_busy_state(target_thread_id: str | None) -> persistent_busy_queue.BusyState:
            busy_state_calls.append(target_thread_id)
            return (busy_state, target_thread_id, "project:1")

        async def is_thread_runner_busy(target_thread_id: str | None) -> bool:
            runner_busy_calls.append(target_thread_id)
            return runner_busy

        async def send_followup(
            interaction: persistent_busy_choice.PersistentBusyInteraction,
            content: str,
            *,
            log_prefix: str,
            context: str,
        ) -> None:
            followups.append(FollowupCall(interaction, content, log_prefix, context))

        async def enqueue_thread_ask(
            channel: persistent_busy_queue.QueueChannel,
            prompt: str,
            target_thread_id: str | None,
            *,
            queued: bool = False,
            ack_sent: bool = False,
            source_message: persistent_busy_queue.QueueSourceMessage | None = None,
        ) -> int:
            enqueues.append(EnqueueCall(channel, prompt, target_thread_id, queued, ack_sent, source_message))
            if enqueue_error:
                raise RuntimeError("enqueue failed")
            return 4

        async def handle_queue_followup(
            interaction: persistent_busy_choice.PersistentBusyInteraction,
            *,
            user_id: int,
            choice_id: str,
            position: int,
            target_thread_id: str | None,
            prompt: str,
            deps: persistent_busy_choice.PersistentBusyQueueFollowupDeps,
        ) -> bool:
            _ = deps
            queue_followups.append((interaction, user_id, choice_id, position, target_thread_id, prompt))
            return True

        @asynccontextmanager
        async def prompt_admission(
            _channel: persistent_busy_queue.QueueChannel,
            _source_message: persistent_busy_queue.QueueSourceMessage,
            _expected_generation: int | None,
        ):
            yield admitted

        deps = persistent_busy_queue.PersistentBusyQueueActionDeps(
            get_busy_state_for_thread=get_busy_state,
            is_thread_runner_busy=is_thread_runner_busy,
            send_followup=send_followup,
            enqueue_thread_ask=enqueue_thread_ask,
            prompt_admission=prompt_admission,
            handle_queue_followup=handle_queue_followup,
            format_log_text_len=len,
            log=logs.append,
        )
        return QueueHarness(
            deps=deps,
            busy_state_calls=busy_state_calls,
            runner_busy_calls=runner_busy_calls,
            followups=followups,
            enqueues=enqueues,
            queue_followups=queue_followups,
            logs=logs,
        )
