from __future__ import annotations

# pyright: reportUnknownMemberType=false, reportUnknownVariableType=false
import asyncio  # noqa: ANYIO_OK
from contextlib import asynccontextmanager
from dataclasses import replace
import unittest

import discord

import codex_discord_bot as bot
import codex_discord_bot_shapes as discord_bot_shapes
from codex_discord_runner_queue import QueueItem, QueueJobValue, ThreadRunner
import codex_discord_runtime as discord_runtime
from codex_discord_text import DISCORD_MAX_LEN

from tests.test_codex_discord_bot import FakeTarget


class DiscordRunPromptFlowIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_rejected_admission_sends_no_prompt_preamble(self) -> None:
        calls: list[str] = []
        discord_client = discord.Client(intents=discord.Intents.none())
        self.addAsyncCleanup(discord_client.close)
        source_message = discord_bot_shapes.RuntimeBusyChoiceSourceMessage(
            author=discord_bot_shapes.RuntimeBusyChoiceAuthor(
                id=242286902982606848,
                bot=False,
            ),
            channel=discord_client.get_partial_messageable(222),
        )

        @asynccontextmanager
        async def reject_admission(
            _channel: object,
            _source_message: object | None,
            _expected_generation: int | None,
        ):
            yield False

        async def fail_if_run(*_args: object, **_kwargs: object) -> None:
            calls.append("run")

        runtime = type(bot.PLAIN_ASK_RUNTIME)(
            replace(
                bot.PLAIN_ASK_RUNTIME.deps,
                prompt_admission=reject_admission,
                run_prompt_and_send=fail_if_run,
            )
        )
        channel = FakeTarget()

        await runtime.run_prompt_flow(
            channel,
            "must not start",
            source_message=source_message,
            target_thread_id="thread-1",
        )

        self.assertEqual(channel.messages, [])
        self.assertEqual(calls, [])

    async def test_run_prompt_flow_sends_ask_start_repeatback(self) -> None:
        original_get_thread_runner = bot.get_thread_runner
        original_build_context_warning = bot.build_context_warning
        original_enqueue_thread_ask = bot.enqueue_thread_ask
        original_run_prompt_and_send = bot.run_prompt_and_send
        calls: list[tuple[str, str | None, bool]] = []

        async def fake_get_thread_runner(target_thread_id: str | None) -> ThreadRunner:
            _ = target_thread_id
            raise AssertionError("run_prompt_flow should not inspect runner busy state")

        def fake_build_context_warning(target_thread_id: str | None) -> str:
            _ = target_thread_id
            return ""

        async def fake_enqueue_thread_ask(
            channel: QueueJobValue,
            prompt: str,
            target_thread_id: str | None,
            *,
            queued: bool = False,
            ack_sent: bool = False,
            source_message: QueueJobValue = None,
        ) -> int:
            _ = (channel, prompt, target_thread_id, queued, ack_sent, source_message)
            raise AssertionError("run_prompt_flow should not enqueue general asks")

        async def fake_run_prompt_and_send(
            channel: QueueJobValue,
            prompt: str,
            *,
            queued: bool = False,
            ack_sent: bool = False,
            source_message: QueueJobValue = None,
            target_thread_id: str | None = None,
        ) -> None:
            _ = (channel, queued, source_message)
            calls.append((prompt, target_thread_id, ack_sent))

        try:
            bot.get_thread_runner = fake_get_thread_runner
            bot.build_context_warning = fake_build_context_warning
            bot.enqueue_thread_ask = fake_enqueue_thread_ask
            bot.run_prompt_and_send = fake_run_prompt_and_send
            channel = FakeTarget()

            await bot.run_prompt_flow(
                channel,
                "First sentence. second sentence\nthird line",
                target_thread_id="thread-1",
            )

            self.assertEqual(
                channel.messages,
                [("In progress\nmessage: First sentence.", None)],
            )
            self.assertEqual(calls, [("First sentence. second sentence\nthird line", "thread-1", True)])
        finally:
            bot.get_thread_runner = original_get_thread_runner
            bot.build_context_warning = original_build_context_warning
            bot.enqueue_thread_ask = original_enqueue_thread_ask
            bot.run_prompt_and_send = original_run_prompt_and_send

    async def test_run_prompt_flow_starts_distinct_target_threads_independently(self) -> None:
        original_run_prompt_and_send = bot.run_prompt_and_send
        original_build_context_warning = bot.build_context_warning
        target_ids = ["thread-a", "thread-b"]
        started: list[tuple[str | None, str, QueueJobValue]] = []
        both_started = asyncio.Event()
        release = asyncio.Event()

        def fake_build_context_warning(target_thread_id: str | None) -> str:
            _ = target_thread_id
            return ""

        async def fake_run_prompt_and_send(
            channel: QueueJobValue,
            prompt: str,
            *,
            queued: bool = False,
            ack_sent: bool = False,
            source_message: QueueJobValue = None,
            target_thread_id: str | None = None,
        ) -> None:
            _ = (queued, ack_sent, source_message)
            started.append((target_thread_id, prompt, channel))
            if len(started) == len(target_ids):
                _ = both_started.set()
            _ = await both_started.wait()
            _ = await release.wait()

        try:
            async with bot.THREAD_RUNNERS_LOCK:
                for target_id in target_ids:
                    _ = bot.THREAD_RUNNERS.pop(discord_runtime.normalize_runner_key(target_id), None)

            bot.run_prompt_and_send = fake_run_prompt_and_send
            bot.build_context_warning = fake_build_context_warning
            channel_a = FakeTarget(channel_id=101)
            channel_b = FakeTarget(channel_id=102)

            tasks = [
                asyncio.create_task(bot.run_prompt_flow(channel_a, "first", target_thread_id="thread-a")),
                asyncio.create_task(bot.run_prompt_flow(channel_b, "second", target_thread_id="thread-b")),
            ]
            _ = await asyncio.wait_for(both_started.wait(), timeout=1)

            self.assertCountEqual([target_thread_id for target_thread_id, _prompt, _channel in started], target_ids)
            self.assertEqual(channel_a.messages, [("In progress\nmessage: first", None)])
            self.assertEqual(channel_b.messages, [("In progress\nmessage: second", None)])
            _ = release.set()
            _ = await asyncio.wait_for(asyncio.gather(*tasks), timeout=1)
        finally:
            _ = release.set()
            for target_id in target_ids:
                runner = await bot.get_thread_runner(target_id)
                try:
                    await asyncio.wait_for(runner["queue"].join(), timeout=1)
                except asyncio.TimeoutError as exc:
                    _ = exc
                task = runner["task"]
                if task is not None and not task.done():
                    _ = task.cancel()
                    try:
                        await task
                    except asyncio.CancelledError as exc:
                        _ = exc
            async with bot.THREAD_RUNNERS_LOCK:
                for target_id in target_ids:
                    _ = bot.THREAD_RUNNERS.pop(discord_runtime.normalize_runner_key(target_id), None)
            bot.run_prompt_and_send = original_run_prompt_and_send
            bot.build_context_warning = original_build_context_warning

    async def test_run_prompt_flow_sends_directly_without_runner_queue(self) -> None:
        original_get_thread_runner = bot.get_thread_runner
        original_build_context_warning = bot.build_context_warning
        original_enqueue_thread_ask = bot.enqueue_thread_ask
        original_run_prompt_and_send = bot.run_prompt_and_send
        calls: list[tuple[str, str | None, bool, QueueJobValue]] = []

        async def fake_get_thread_runner(target_thread_id: str | None) -> ThreadRunner:
            _ = target_thread_id
            raise AssertionError("run_prompt_flow should not inspect runner busy state")

        def fake_build_context_warning(target_thread_id: str | None) -> str:
            _ = target_thread_id
            return ""

        async def fake_enqueue_thread_ask(
            channel: QueueJobValue,
            prompt: str,
            target_thread_id: str | None,
            *,
            queued: bool = False,
            ack_sent: bool = False,
            source_message: QueueJobValue = None,
        ) -> int:
            _ = (channel, prompt, target_thread_id, queued, ack_sent, source_message)
            raise AssertionError("run_prompt_flow should not enqueue general asks")

        async def fake_run_prompt_and_send(
            channel: QueueJobValue,
            prompt: str,
            *,
            queued: bool = False,
            ack_sent: bool = False,
            source_message: QueueJobValue = None,
            target_thread_id: str | None = None,
        ) -> None:
            _ = (channel, queued)
            calls.append((prompt, target_thread_id, ack_sent, source_message))

        try:
            bot.get_thread_runner = fake_get_thread_runner
            bot.build_context_warning = fake_build_context_warning
            bot.enqueue_thread_ask = fake_enqueue_thread_ask
            bot.run_prompt_and_send = fake_run_prompt_and_send
            channel = FakeTarget()
            discord_client = discord.Client(intents=discord.Intents.none())
            self.addAsyncCleanup(discord_client.close)
            source_message = discord_bot_shapes.RuntimeBusyChoiceSourceMessage(
                author=discord_bot_shapes.RuntimeBusyChoiceAuthor(
                    id=242286902982606848,
                    bot=False,
                ),
                channel=discord_client.get_partial_messageable(222),
            )

            @asynccontextmanager
            async def accept_admission(
                _channel: object,
                _source_message: object | None,
                _expected_generation: int | None,
            ):
                yield True

            runtime = type(bot.PLAIN_ASK_RUNTIME)(
                replace(bot.PLAIN_ASK_RUNTIME.deps, prompt_admission=accept_admission)
            )

            await runtime.run_prompt_flow(
                channel,
                "please queue",
                source_message=source_message,
                target_thread_id="thread-1",
            )

            self.assertEqual(channel.messages, [("In progress\nmessage: please queue", None)])
            self.assertEqual(calls, [("please queue", "thread-1", True, source_message)])
        finally:
            bot.get_thread_runner = original_get_thread_runner
            bot.build_context_warning = original_build_context_warning
            bot.enqueue_thread_ask = original_enqueue_thread_ask
            bot.run_prompt_and_send = original_run_prompt_and_send

    async def test_run_prompt_flow_chunks_long_context_warning(self) -> None:
        original_get_thread_runner = bot.get_thread_runner
        original_build_context_warning = bot.build_context_warning
        original_enqueue_thread_ask = bot.enqueue_thread_ask

        async def fake_get_thread_runner(target_thread_id: str | None) -> ThreadRunner:
            return {
                "active": False,
                "queue": asyncio.Queue[QueueItem](),
                "task": None,
                "target_thread_id": target_thread_id,
            }

        def fake_build_context_warning(target_thread_id: str | None) -> str:
            _ = target_thread_id
            return "x" * 4100

        async def fake_enqueue_thread_ask(
            channel: QueueJobValue,
            prompt: str,
            target_thread_id: str | None,
            *,
            queued: bool = False,
            ack_sent: bool = False,
            source_message: QueueJobValue = None,
        ) -> int:
            _ = (channel, prompt, target_thread_id, queued, ack_sent, source_message)
            return 1

        try:
            bot.get_thread_runner = fake_get_thread_runner
            bot.build_context_warning = fake_build_context_warning
            bot.enqueue_thread_ask = fake_enqueue_thread_ask
            channel = FakeTarget()

            await bot.run_prompt_flow(channel, "please run", target_thread_id="thread-1")

            sent = [content for content, _view in channel.messages]
            self.assertGreater(len(sent), 1)
            self.assertTrue(all(len(content) <= DISCORD_MAX_LEN for content in sent))
            first_warning_chunk = sent[0].split("\n", 1)[1] if sent[0].startswith("[1/") else sent[0]
            self.assertTrue(first_warning_chunk.startswith("x"))
            self.assertNotIn("Ask received. Sending to Codex.", "\n".join(sent))
        finally:
            bot.get_thread_runner = original_get_thread_runner
            bot.build_context_warning = original_build_context_warning
            bot.enqueue_thread_ask = original_enqueue_thread_ask


if __name__ == "__main__":
    _ = unittest.main()
