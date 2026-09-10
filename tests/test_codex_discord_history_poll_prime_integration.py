from __future__ import annotations

import datetime as dt
import os
import tempfile
import unittest
from collections.abc import AsyncIterator, Awaitable, Callable
from pathlib import Path
from typing import override
from unittest import mock

import codex_discord_bot as bot
import codex_discord_logging as discord_logging
import codex_discord_message_gate as discord_message_gate
from codex_discord_seen_cache import SeenCacheMap


class FakeAuthor:
    id: int = 242286902982606848
    bot: bool = False


class AppServerUnavailableError(RuntimeError):
    pass


class FakeMessage:
    type: None = None

    def __init__(
        self,
        content: str,
        *,
        channel_id: int = 333,
        message_id: int | None = None,
        created_at: dt.datetime | None = None,
    ) -> None:
        self.id: int | None = message_id
        self.author: FakeAuthor = FakeAuthor()
        self.content: str = content
        self.raw_mentions: list[int] = []
        self.mentions: list[FakeAuthor] = []
        self.attachments: list[str] = []
        self.embeds: list[str] = []
        self.stickers: list[str] = []
        self.created_at: dt.datetime = created_at or dt.datetime.now(dt.timezone.utc)
        self.channel: FakeHistoryChannel = FakeHistoryChannel(channel_id)


class FakeHistoryChannel:
    def __init__(self, channel_id: int = 333) -> None:
        self.id: int = channel_id
        self.history_messages: list[FakeMessage] = []
        self.messages: list[str] = []

    async def send(self, content: str) -> None:
        self.messages.append(content)

    def history(self, *, limit: int) -> AsyncIterator[FakeMessage]:
        async def iterator() -> AsyncIterator[FakeMessage]:
            for message in self.history_messages[:limit]:
                yield message

        return iterator()


class FakePollClient:
    def __init__(self, channel: FakeHistoryChannel) -> None:
        self._processed_message_ids: SeenCacheMap = {}
        self._history_poll_primed_channels: set[int] = set()
        self.allowed_channel_ids: set[int] = {channel.id}
        self.startup_channel_id: int | None = None
        self.history_poll_seconds: float = 1.0
        self.enable_prefix_commands: bool = True
        self.plain_ask_mention_user_ids: list[int] = []
        self.user: None = None
        self.channel: FakeHistoryChannel = channel

    def is_closed(self) -> bool:
        return False

    def get_cached_channel_or_thread(self, channel_id: int) -> tuple[object | None, str]:
        _ = channel_id
        return self.channel, "test_cache"

    async def fetch_channel(self, channel_id: int) -> object | None:
        _ = channel_id
        raise AssertionError("fetch not expected")

    async def history_poll_loop(self) -> None:
        return

    async def poll_history_channel(self, label: str, channel_id: int) -> None:
        await bot.HISTORY_RUNTIME.poll_history_channel(self, label, channel_id)

    def is_allowed_message_channel(self, channel: object) -> bool:
        _ = channel
        return True

    def is_allowed_user(self, user_id: int | None) -> bool:
        _ = user_id
        return True

    async def process_discord_message(self, message: object, *, source: str) -> None:
        if not isinstance(message, FakeMessage):
            raise TypeError(f"expected FakeMessage, got {type(message).__name__}")
        await bot.MESSAGE_RUNTIME.process_discord_message(self, message, source=source)


class RecordingHistoryRuntime:
    def __init__(self) -> None:
        self.calls: list[tuple[object, str, int]] = []

    async def poll_history_channel(self, owner: object, label: str, channel_id: int) -> None:
        self.calls.append((owner, label, channel_id))


class RecordingMessageRuntime:
    def __init__(self) -> None:
        self.calls: list[tuple[object, object, str]] = []

    async def process_discord_message(self, owner: object, message: object, *, source: str) -> None:
        self.calls.append((owner, message, source))


LogAction = Callable[[Path], Awaitable[None]]


class DiscordHistoryPollPrimeIntegrationTests(unittest.IsolatedAsyncioTestCase):
    @override
    def setUp(self) -> None:
        old_mirror_db_path = bot.MIRROR_DB_PATH
        mirror_db_temp_dir = tempfile.TemporaryDirectory(ignore_cleanup_errors=True)
        self.addCleanup(mirror_db_temp_dir.cleanup)
        self.addCleanup(setattr, bot, "MIRROR_DB_PATH", old_mirror_db_path)
        bot.MIRROR_DB_PATH = Path(mirror_db_temp_dir.name) / "mirror.sqlite"
        bot.init_mirror_db()

    async def _run_with_log(self, action: LogAction) -> str:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with mock.patch.dict(os.environ, {"CODEX_DISCORD_LOG_PATH": str(log_path)}):
                await action(log_path)
            return log_path.read_text(encoding="utf-8")

    async def test_dynamic_client_poll_and_message_methods_delegate_to_runtimes(self) -> None:
        history_runtime = RecordingHistoryRuntime()
        message_runtime = RecordingMessageRuntime()
        client = bot.CodexDiscordBot.__new__(bot.CodexDiscordBot)
        message = FakeMessage("delegated", message_id=777)

        with (
            mock.patch.object(bot, "HISTORY_RUNTIME", history_runtime),
            mock.patch.object(bot, "MESSAGE_RUNTIME", message_runtime),
        ):
            await client.poll_history_channel("allowed", 333)
            await client.process_discord_message(message, source="adapter-test")

        self.assertEqual(history_runtime.calls, [(client, "allowed", 333)])
        self.assertEqual(message_runtime.calls, [(client, message, "adapter-test")])

    async def test_history_poll_primes_then_processes_new_user_message_once(self) -> None:
        handled: list[tuple[str, str | None]] = []
        channel = FakeHistoryChannel()
        old_message = FakeMessage(
            "old",
            message_id=100,
            created_at=dt.datetime.now(dt.timezone.utc) - dt.timedelta(seconds=1),
        )
        new_message = FakeMessage(
            "please hook",
            message_id=101,
            created_at=dt.datetime.now(dt.timezone.utc) + dt.timedelta(seconds=1),
        )
        old_message.channel = channel
        new_message.channel = channel
        client = FakePollClient(channel)

        async def runner_idle(target_thread_id: str | None) -> bool:
            _ = target_thread_id
            return False

        async def fake_handle_plain_ask(
            message: FakeMessage,
            prompt: str,
            *,
            target_thread_id: str | None = None,
        ) -> None:
            _ = message
            handled.append((prompt, target_thread_id))

        def mirror_thread_id(channel_id: int) -> str:
            _ = channel_id
            return "thread-1"

        def busy_state(target_thread_id: str | None) -> tuple[str, None, str]:
            _ = target_thread_id
            return "idle", None, ""

        async def run_poll(log_path: Path) -> None:
            _ = log_path
            channel.history_messages = [old_message]
            await client.poll_history_channel("allowed", 333)
            await client.poll_history_channel("allowed", 333)
            channel.history_messages = [new_message, old_message]
            await client.poll_history_channel("allowed", 333)
            await discord_message_gate.process_gateway_message(
                new_message,
                deps=discord_message_gate.GatewayMessageDeps(
                    discord_client=client,
                    claim_message=lambda message: bot.PROCESSED_MESSAGE_RUNTIME.claim_discord_message(
                        client,
                        message,
                    ),
                    get_message_id=bot.PROCESSED_MESSAGE_RUNTIME.get_discord_message_id,
                    process_message=client.process_discord_message,
                    mark_processed=lambda message: bot.PROCESSED_MESSAGE_RUNTIME.mark_discord_message_processed(
                        client,
                        message,
                    ),
                    log=discord_logging.log_line,
                ),
            )

        with (
            mock.patch.object(bot, "get_mirrored_codex_thread_id", mirror_thread_id),
            mock.patch.object(bot, "get_busy_state_for_thread", busy_state),
            mock.patch.object(bot, "is_thread_runner_busy", runner_idle),
            mock.patch.object(bot, "handle_plain_ask", fake_handle_plain_ask),
        ):
            log_text = await self._run_with_log(run_poll)

        self.assertEqual(handled, [("please hook", "thread-1")])
        self.assertIn("history_poll_primed label=allowed channel=333", log_text)
        self.assertIn("history_poll_message channel=333", log_text)
        self.assertIn("message_received chat=333", log_text)
        self.assertIn("source=history_poll", log_text)
        self.assertIn("duplicate_message_skipped source=gateway chat=333 message=101", log_text)

    async def test_history_poll_first_prime_discards_offline_backlog(self) -> None:
        handled: list[tuple[str, str | None]] = []
        channel = FakeHistoryChannel()
        cutoff = dt.datetime(2026, 6, 3, 15, 0, tzinfo=dt.timezone.utc)
        old_message = FakeMessage("old", message_id=100, created_at=cutoff - dt.timedelta(seconds=1))
        fresh_message = FakeMessage("bootstrap hook", message_id=101, created_at=cutoff + dt.timedelta(seconds=1))
        old_message.channel = channel
        fresh_message.channel = channel
        channel.history_messages = [fresh_message, old_message]
        client = FakePollClient(channel)

        async def runner_idle(target_thread_id: str | None) -> bool:
            _ = target_thread_id
            return False

        async def fake_handle_plain_ask(
            message: FakeMessage,
            prompt: str,
            *,
            target_thread_id: str | None = None,
        ) -> None:
            _ = message
            handled.append((prompt, target_thread_id))

        def mirror_thread_id(channel_id: int) -> str:
            _ = channel_id
            return "thread-1"

        def busy_state(target_thread_id: str | None) -> tuple[str, None, str]:
            _ = target_thread_id
            return "idle", None, ""

        async def run_poll(log_path: Path) -> None:
            _ = log_path
            await client.poll_history_channel("allowed", 333)

        with (
            mock.patch.object(bot, "get_mirrored_codex_thread_id", mirror_thread_id),
            mock.patch.object(bot, "get_busy_state_for_thread", busy_state),
            mock.patch.object(bot, "is_thread_runner_busy", runner_idle),
            mock.patch.object(bot, "handle_plain_ask", fake_handle_plain_ask),
        ):
            log_text = await self._run_with_log(run_poll)

        self.assertEqual(handled, [])
        self.assertIn("history_poll_primed label=allowed channel=333", log_text)
        self.assertIn("fetched_messages=2", log_text)
        self.assertIn("claimed_messages=2", log_text)
        self.assertIn("eligible_messages=0", log_text)
        self.assertIn("discarded_messages=2", log_text)
        self.assertNotIn("history_poll_message channel=333", log_text)
        self.assertNotIn("old", log_text)

    async def test_history_poll_watermark_blocks_old_message_after_processed_ids_are_lost(self) -> None:
        handled: list[str] = []
        channel = FakeHistoryChannel()
        old_message = FakeMessage(
            "old backlog",
            message_id=100,
            created_at=dt.datetime(2026, 6, 3, 15, 0, tzinfo=dt.timezone.utc),
        )
        old_message.channel = channel
        channel.history_messages = [old_message]
        client = FakePollClient(channel)

        async def fake_handle_plain_ask(
            message: FakeMessage,
            prompt: str,
            *,
            target_thread_id: str | None = None,
        ) -> None:
            _ = (message, target_thread_id)
            handled.append(prompt)

        async def run_poll(log_path: Path) -> None:
            _ = log_path
            await client.poll_history_channel("allowed", 333)
            client._processed_message_ids.clear()
            with bot.discord_store.connect_store(bot.MIRROR_DB_PATH) as conn:
                _ = conn.execute("DELETE FROM discord_processed_messages")
            await client.poll_history_channel("allowed", 333)

        with (
            mock.patch.object(bot, "get_mirrored_codex_thread_id", return_value="thread-1"),
            mock.patch.object(bot, "get_busy_state_for_thread", return_value=("idle", None, "")),
            mock.patch.object(bot, "is_thread_runner_busy", mock.AsyncMock(return_value=False)),
            mock.patch.object(bot, "handle_plain_ask", fake_handle_plain_ask),
        ):
            log_text = await self._run_with_log(run_poll)

        self.assertEqual(handled, [])
        self.assertEqual(log_text.count("history_poll_primed label=allowed channel=333"), 1)
        self.assertNotIn("history_poll_message channel=333", log_text)

    async def test_history_poll_accepts_message_created_after_empty_prime(self) -> None:
        handled: list[str] = []
        channel = FakeHistoryChannel()
        client = FakePollClient(channel)

        async def fake_handle_plain_ask(
            message: FakeMessage,
            prompt: str,
            *,
            target_thread_id: str | None = None,
        ) -> None:
            _ = (message, target_thread_id)
            handled.append(prompt)

        async def run_poll(log_path: Path) -> None:
            _ = log_path
            await client.poll_history_channel("allowed", 333)
            new_message = FakeMessage(
                "new after start",
                message_id=101,
                created_at=dt.datetime.now(dt.timezone.utc) + dt.timedelta(seconds=1),
            )
            new_message.channel = channel
            channel.history_messages = [new_message]
            await client.poll_history_channel("allowed", 333)

        with (
            mock.patch.object(bot, "get_mirrored_codex_thread_id", return_value="thread-1"),
            mock.patch.object(bot, "get_busy_state_for_thread", return_value=("idle", None, "")),
            mock.patch.object(bot, "is_thread_runner_busy", mock.AsyncMock(return_value=False)),
            mock.patch.object(bot, "handle_plain_ask", fake_handle_plain_ask),
        ):
            await self._run_with_log(run_poll)

        self.assertEqual(handled, ["new after start"])

    async def test_history_poll_empty_prime_still_blocks_old_message_seen_later(self) -> None:
        handled: list[str] = []
        channel = FakeHistoryChannel()
        client = FakePollClient(channel)

        async def fake_handle_plain_ask(
            message: FakeMessage,
            prompt: str,
            *,
            target_thread_id: str | None = None,
        ) -> None:
            _ = (message, target_thread_id)
            handled.append(prompt)

        async def run_poll(log_path: Path) -> None:
            _ = log_path
            await client.poll_history_channel("allowed", 333)
            delayed_old_message = FakeMessage(
                "delayed old backlog",
                message_id=101,
                created_at=dt.datetime.now(dt.timezone.utc) - dt.timedelta(days=1),
            )
            delayed_old_message.channel = channel
            channel.history_messages = [delayed_old_message]
            await client.poll_history_channel("allowed", 333)

        with (
            mock.patch.object(bot, "get_mirrored_codex_thread_id", return_value="thread-1"),
            mock.patch.object(bot, "get_busy_state_for_thread", return_value=("idle", None, "")),
            mock.patch.object(bot, "is_thread_runner_busy", mock.AsyncMock(return_value=False)),
            mock.patch.object(bot, "handle_plain_ask", fake_handle_plain_ask),
        ):
            log_text = await self._run_with_log(run_poll)

        self.assertEqual(handled, [])
        self.assertNotIn("history_poll_message channel=333", log_text)

    async def test_history_poll_app_server_failure_is_not_replayed(self) -> None:
        attempts: list[str] = []
        channel = FakeHistoryChannel()
        client = FakePollClient(channel)

        async def fail_handle_plain_ask(
            message: FakeMessage,
            prompt: str,
            *,
            target_thread_id: str | None = None,
        ) -> None:
            _ = (message, target_thread_id)
            attempts.append(prompt)
            raise AppServerUnavailableError("app server unavailable")

        async def run_poll(log_path: Path) -> None:
            _ = log_path
            await client.poll_history_channel("allowed", 333)
            offline_message = FakeMessage(
                "while app server offline",
                message_id=101,
                created_at=dt.datetime.now(dt.timezone.utc) + dt.timedelta(seconds=1),
            )
            offline_message.channel = channel
            channel.history_messages = [offline_message]
            await client.poll_history_channel("allowed", 333)
            await client.poll_history_channel("allowed", 333)

        with (
            mock.patch.object(bot, "get_mirrored_codex_thread_id", return_value="thread-1"),
            mock.patch.object(bot, "get_busy_state_for_thread", return_value=("idle", None, "")),
            mock.patch.object(bot, "is_thread_runner_busy", mock.AsyncMock(return_value=False)),
            mock.patch.object(bot, "handle_plain_ask", fail_handle_plain_ask),
        ):
            await self._run_with_log(run_poll)

        self.assertEqual(attempts, ["while app server offline"])


if __name__ == "__main__":
    _ = unittest.main()
