from __future__ import annotations

# pyright: reportAttributeAccessIssue=false, reportUnknownMemberType=false, reportUnknownVariableType=false
import asyncio  # noqa: ANYIO_OK
from dataclasses import dataclass
from pathlib import Path
import sqlite3
import tempfile
import unittest

import codex_discord_bot as bot
from codex_discord_runner_queue import QueueJob, ThreadRunner
import codex_discord_runtime as discord_runtime

from tests.test_codex_discord_bot import EnvPatch, FakeBot, FakeTarget


@dataclass(frozen=True, slots=True)
class QueueAuthor:
    id: int
    bot: bool = False


@dataclass(frozen=True, slots=True)
class QueueSourceMessage:
    channel: FakeTarget
    author: QueueAuthor


def make_source_message(
    *,
    channel_id: int = 222,
    author_id: int = 242286902982606848,
) -> QueueSourceMessage:
    return QueueSourceMessage(FakeTarget(channel_id=channel_id), QueueAuthor(author_id))


class DiscordRunnerQueueIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_thread_runner_job_failure_reports_short_channel_message(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            channel = FakeTarget()
            job: QueueJob = {"channel": channel}
            with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                await bot.report_thread_runner_job_failed(job, "thread-1")
            log_text = log_path.read_text(encoding="utf-8")

        self.assertEqual(
            channel.messages,
            [("Queued ask failed. Check codex_discord_bot.log.", None)],
        )
        self.assertIn("context=thread_runner_job_failed", log_text)
        self.assertIn("discord_delivery_sent", log_text)
        self.assertIn("thread_runner_job_failure_reported target=thread-1", log_text)

    async def test_build_runners_message_uses_thread_runner_snapshot(self) -> None:
        key = discord_runtime.normalize_runner_key("thread-1")
        queue: asyncio.Queue[QueueJob] = asyncio.Queue()
        original_resolve_target_ref = bot.resolve_target_ref

        def fake_resolve_target_ref(target_thread_id: str | None) -> tuple[str | None, str]:
            return target_thread_id, "thread:1"

        try:
            await queue.put({"prompt": "queued prompt"})
            runner: ThreadRunner = {
                "queue": queue,
                "task": None,
                "active": True,
                "target_thread_id": "thread-1",
            }
            async with bot.THREAD_RUNNERS_LOCK:
                bot.THREAD_RUNNERS[key] = runner
            bot.resolve_target_ref = fake_resolve_target_ref

            output = await bot.build_runners_message()

            self.assertEqual(
                output,
                f"Discord runner queues\n- thread:1: active=True queued=1 key={key[:8]}",
            )
        finally:
            bot.resolve_target_ref = original_resolve_target_ref
            async with bot.THREAD_RUNNERS_LOCK:
                _ = bot.THREAD_RUNNERS.pop(key, None)

    async def test_prefix_retract_uses_mapped_thread_queue(self) -> None:
        key = discord_runtime.normalize_runner_key("thread-1")
        command_message = make_source_message(channel_id=222)
        queued_source = make_source_message(channel_id=222)
        queue: asyncio.Queue[QueueJob] = asyncio.Queue()
        original_mirror_db_path = bot.MIRROR_DB_PATH
        try:
            with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
                bot.MIRROR_DB_PATH = Path(temp_dir) / "mirror.sqlite"
                bot.init_mirror_db()
                with sqlite3.connect(bot.MIRROR_DB_PATH) as conn:
                    _ = conn.execute(
                        """
                        INSERT INTO mirror_threads (
                            codex_thread_id, project_key, thread_title,
                            discord_channel_id, discord_thread_id, updated_at
                        ) VALUES (?, ?, ?, ?, ?, ?)
                        """,
                        ("thread-1", "project", "title", 111, 222, 1.0),
                    )
                queued_job: QueueJob = {
                    "channel": queued_source.channel,
                    "prompt": "queued prompt",
                    "target_thread_id": "thread-1",
                    "source_message": queued_source,
                }
                await queue.put(queued_job)
                runner: ThreadRunner = {
                    "queue": queue,
                    "task": None,
                    "active": False,
                    "target_thread_id": "thread-1",
                }
                async with bot.THREAD_RUNNERS_LOCK:
                    bot.THREAD_RUNNERS[key] = runner

                await bot.handle_prefix_command(FakeBot(), command_message, "retract")

            self.assertEqual(
                command_message.channel.messages,
                [("Retracted your latest queued ask for `thread-1`. remaining_queued: 0", None)],
            )
            self.assertEqual(queue.qsize(), 0)
        finally:
            bot.MIRROR_DB_PATH = original_mirror_db_path
            async with bot.THREAD_RUNNERS_LOCK:
                _ = bot.THREAD_RUNNERS.pop(key, None)

    async def test_retract_thread_ask_removes_latest_matching_queued_job(self) -> None:
        key = discord_runtime.normalize_runner_key("thread-1")
        owner_message = make_source_message()
        other_owner_message = make_source_message(author_id=999)
        other_channel_message = make_source_message(channel_id=333)
        queue: asyncio.Queue[QueueJob] = asyncio.Queue()
        jobs: list[QueueJob] = [
            {
                "channel": owner_message.channel,
                "prompt": "first matching",
                "target_thread_id": "thread-1",
                "source_message": owner_message,
            },
            {
                "channel": owner_message.channel,
                "prompt": "other owner",
                "target_thread_id": "thread-1",
                "source_message": other_owner_message,
            },
            {
                "channel": owner_message.channel,
                "prompt": "latest matching",
                "target_thread_id": "thread-1",
                "source_message": owner_message,
            },
            {
                "channel": other_channel_message.channel,
                "prompt": "other channel",
                "target_thread_id": "thread-1",
                "source_message": other_channel_message,
            },
        ]
        try:
            for job in jobs:
                await queue.put(job)
            runner: ThreadRunner = {
                "queue": queue,
                "task": None,
                "active": False,
                "target_thread_id": "thread-1",
            }
            async with bot.THREAD_RUNNERS_LOCK:
                bot.THREAD_RUNNERS[key] = runner

            result = await bot.retract_thread_ask(
                "thread-1",
                channel_id=222,
                owner_user_id=owner_message.author.id,
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
        finally:
            async with bot.THREAD_RUNNERS_LOCK:
                _ = bot.THREAD_RUNNERS.pop(key, None)


if __name__ == "__main__":
    _ = unittest.main()
