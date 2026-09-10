from __future__ import annotations

import os
import tempfile
import unittest
from collections.abc import Coroutine
from pathlib import Path
from types import SimpleNamespace, TracebackType
from typing import Protocol, cast, final

import codex_discord_bot as bot
from codex_discord_text import DISCORD_MAX_LEN


@final
class EnvPatch:
    def __init__(self, key: str, value: str) -> None:
        self.key = key
        self.value = value
        self.original: str | None = None

    def __enter__(self) -> None:
        self.original = os.environ.get(self.key)
        os.environ[self.key] = self.value

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        tb: TracebackType | None,
    ) -> None:
        _ = exc_type, exc, tb
        if self.original is None:
            _ = os.environ.pop(self.key, None)
        else:
            os.environ[self.key] = self.original


@final
class FakeFollowup:
    def __init__(self) -> None:
        self.messages: list[str] = []

    async def send(self, content: str, **kwargs: bool) -> None:
        _ = kwargs
        self.messages.append(content)


@final
class FakeInteraction:
    def __init__(self, command_name: str = "where", channel_id: int = 222) -> None:
        self.command = SimpleNamespace(name=command_name)
        self.channel_id = channel_id
        self.followup = FakeFollowup()


class ChunkTarget(Protocol):
    id: int

    async def send(self, content: str) -> None:
        ...


@final
class FakeTarget:
    def __init__(self, channel_id: int = 456) -> None:
        self.id = channel_id
        self.messages: list[str] = []

    async def send(self, content: str) -> None:
        self.messages.append(content)


@final
class TransientSendError(RuntimeError):
    pass


@final
class TransientFailingTarget:
    def __init__(self, channel_id: int = 789, *, failures: int = 1) -> None:
        self.id = channel_id
        self.failures = failures
        self.messages: list[str] = []

    async def send(self, content: str) -> None:
        if self.failures > 0:
            self.failures -= 1
            raise TransientSendError("transient send failure")
        self.messages.append(content)


class SendInteractionChunksFunc(Protocol):
    def __call__(
        self,
        interaction: FakeInteraction,
        text: str,
        *,
        title: str,
        exit_code: int | None = None,
    ) -> Coroutine[None, None, None]:
        ...


class SendChunksFunc(Protocol):
    def __call__(
        self,
        target: ChunkTarget,
        text: str,
        *,
        context: str = "send_chunks",
        allow_during_stop: bool = False,
    ) -> Coroutine[None, None, int]:
        ...


async def send_interaction_chunks(interaction: FakeInteraction, text: str, *, title: str) -> None:
    sender = cast(SendInteractionChunksFunc, bot.send_interaction_chunks)
    await sender(interaction, text, title=title)


async def send_chunks(target: ChunkTarget, text: str, *, context: str) -> int:
    sender = cast(SendChunksFunc, bot.send_chunks)
    return await sender(target, text, context=context)


@final
class DiscordChunkDeliveryIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_send_interaction_chunks_logs_and_sends(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            interaction = FakeInteraction()
            with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                await send_interaction_chunks(interaction, "hello", title="Where")

            self.assertEqual(interaction.followup.messages, ["hello"])
            log_text = log_path.read_text(encoding="utf-8")
            self.assertIn("slash_response_start command=where", log_text)
            self.assertIn("slash_response_sent command=where", log_text)

    async def test_send_chunks_marks_and_logs_multi_chunk_delivery(self) -> None:
        original_chunk_markers = bot.DISCORD_CHUNK_MARKERS_ENABLED
        try:
            bot.DISCORD_CHUNK_MARKERS_ENABLED = True
            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                target = FakeTarget()
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    chunks_sent = await send_chunks(
                        target,
                        "x" * (DISCORD_MAX_LEN + 200),
                        context="unit_long_delivery",
                    )
                log_text = log_path.read_text(encoding="utf-8")
        finally:
            bot.DISCORD_CHUNK_MARKERS_ENABLED = original_chunk_markers

        self.assertEqual(chunks_sent, len(target.messages))
        self.assertGreater(len(target.messages), 1)
        self.assertTrue(target.messages[0].startswith("[1/"))
        self.assertTrue(all(len(content) <= DISCORD_MAX_LEN for content in target.messages))
        self.assertIn("discord_delivery_start", log_text)
        self.assertIn("context=unit_long_delivery", log_text)
        self.assertIn("discord_delivery_chunk_sent", log_text)
        self.assertIn("discord_delivery_sent", log_text)

    async def test_send_chunks_retries_transient_delivery_failure(self) -> None:
        original_retry_delays = bot.DISCORD_SEND_RETRY_DELAYS_SECONDS
        try:
            bot.DISCORD_SEND_RETRY_DELAYS_SECONDS = (0.0,)
            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                target = TransientFailingTarget(failures=1)
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    chunks_sent = await send_chunks(target, "retry me", context="unit_retry")
                log_text = log_path.read_text(encoding="utf-8")
        finally:
            bot.DISCORD_SEND_RETRY_DELAYS_SECONDS = original_retry_delays

        self.assertEqual(chunks_sent, 1)
        self.assertEqual(target.messages, ["retry me"])
        self.assertIn("discord_delivery_retry", log_text)
        self.assertIn("context=unit_retry", log_text)
        self.assertIn("error_type=TransientSendError", log_text)
        self.assertIn("discord_delivery_sent", log_text)


if __name__ == "__main__":
    _ = unittest.main()
