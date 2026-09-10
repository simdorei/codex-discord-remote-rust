from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from dataclasses import dataclass
from typing import override
import unittest

import codex_discord_runner as discord_runner
import codex_discord_runner_queue as runner_queue
from codex_discord_runtime import normalize_runner_key


@dataclass(frozen=True, slots=True)
class FakeChannel:
    id: int


@dataclass(frozen=True, slots=True)
class FakeAuthor:
    id: int


@dataclass(frozen=True, slots=True)
class FakeMessage:
    author: FakeAuthor


class RunnerQueueTests(unittest.IsolatedAsyncioTestCase):
    @override
    async def asyncTearDown(self) -> None:
        async with runner_queue.THREAD_RUNNERS_LOCK:
            runner_queue.THREAD_RUNNERS.clear()

    async def test_enqueue_creates_runner_and_reports_busy(self) -> None:
        calls: list[str | None] = []

        async def fake_loop(target_thread_id: str | None) -> None:
            calls.append(target_thread_id)

        channel = FakeChannel(id=222)
        source_message = FakeMessage(author=FakeAuthor(id=7))

        size = await runner_queue.enqueue_thread_ask(
            channel,
            "please queue",
            "thread-1",
            queued=True,
            ack_sent=True,
            source_message=source_message,
            thread_runner_loop_func=fake_loop,
        )
        runner = await runner_queue.get_thread_runner("thread-1")
        task = runner["task"]
        if task is not None:
            await task

        self.assertEqual(size, 1)
        self.assertEqual(calls, ["thread-1"])
        self.assertTrue(await runner_queue.is_thread_runner_busy("thread-1"))

    async def test_retract_removes_latest_matching_queued_job(self) -> None:
        channel = FakeChannel(id=222)
        other_channel = FakeChannel(id=333)
        owner_message = FakeMessage(author=FakeAuthor(id=7))
        other_owner_message = FakeMessage(author=FakeAuthor(id=9))
        runner = await runner_queue.get_thread_runner("thread-1")
        queue = runner["queue"]
        jobs: list[runner_queue.QueueJob] = [
            {"channel": channel, "prompt": "first matching", "source_message": owner_message},
            {"channel": channel, "prompt": "other owner", "source_message": other_owner_message},
            {"channel": channel, "prompt": "latest matching", "source_message": owner_message},
            {"channel": other_channel, "prompt": "other channel", "source_message": owner_message},
        ]
        for job in jobs:
            await queue.put(job)

        result = await runner_queue.retract_thread_ask(
            "thread-1",
            channel_id=222,
            owner_user_id=7,
        )

        remaining_prompts: list[str] = []
        while not queue.empty():
            job = queue.get_nowait()
            remaining_prompts.append(str(job.get("prompt")))
            queue.task_done()

        self.assertEqual(result["removed"], 1)
        self.assertEqual(result["remaining"], 3)
        self.assertEqual(
            remaining_prompts,
            ["first matching", "other owner", "other channel"],
        )

    async def test_retract_missing_runner_reports_normalized_key(self) -> None:
        result = await runner_queue.retract_thread_ask("thread-1")

        self.assertEqual(
            result,
            {
                "removed": 0,
                "remaining": 0,
                "active": False,
                "target_key": normalize_runner_key("thread-1"),
            },
        )

    async def test_generation_expiry_drain_retains_only_current_generation_jobs(self) -> None:
        queue: asyncio.Queue[runner_queue.QueueItem] = asyncio.Queue()
        for job_id, generation in (("old", 2), ("current", 3), ("legacy", 0)):
            await queue.put(
                {
                    "job_id": job_id,
                    "app_server_generation": generation,
                    "prompt": job_id,
                }
            )
        runner: runner_queue.ThreadRunner = {
            "queue": queue,
            "task": None,
            "active": True,
            "target_thread_id": "thread-1",
            "queued_job_ids": {"old", "current", "legacy"},
        }

        discarded = discord_runner._drain_generation_expired_queue(
            queue,
            runner,
            current_generation=3,
        )
        retained = queue.get_nowait()
        queue.task_done()

        self.assertEqual(discarded, 2)
        self.assertEqual(retained.get("job_id"), "current")
        self.assertEqual(runner["queued_job_ids"], {"current"})
