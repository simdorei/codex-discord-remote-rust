from __future__ import annotations

from collections.abc import AsyncIterator
from datetime import datetime, timezone
from pathlib import Path
from typing import cast
import tempfile
import unittest
from unittest import mock

import codex_discord_bot as bot
from codex_discord_diagnostics_history import DiagnosticsHistoryBot, DiscordChannelLike


class FakeAuthor:
    def __init__(self, author_id: int, *, is_bot: bool) -> None:
        self.id: int = author_id
        self.bot: bool = is_bot


class FakeMessageType:
    def __init__(self, name: str) -> None:
        self.name: str = name


class FakeMessage:
    def __init__(self, *, author: FakeAuthor, content: str, created_at: datetime) -> None:
        self.author: FakeAuthor = author
        self.content: str = content
        self.created_at: datetime = created_at
        self.type: FakeMessageType = FakeMessageType("default")


class FakeHistoryChannel:
    def __init__(self, channel_id: int, messages: list[FakeMessage]) -> None:
        self.id: int = channel_id
        self.messages: list[FakeMessage] = messages

    def history(self, *, limit: int) -> AsyncIterator[FakeMessage]:
        async def iterator() -> AsyncIterator[FakeMessage]:
            for message in self.messages[:limit]:
                yield message

        return iterator()


class FakeBot:
    allowed_channel_ids: set[int] = {222}
    startup_channel_id: int = 111

    def __init__(self, bot_message: FakeMessage, user_message: FakeMessage) -> None:
        self._bot_message: FakeMessage = bot_message
        self._user_message: FakeMessage = user_message

    def get_cached_channel_or_thread(self, channel_id: int) -> tuple[DiscordChannelLike | None, str]:
        channels: dict[int, DiscordChannelLike] = {
            111: cast(DiscordChannelLike, FakeHistoryChannel(111, [self._bot_message])),
            222: cast(DiscordChannelLike, FakeHistoryChannel(222, [self._bot_message, self._user_message])),
        }
        return channels.get(channel_id), "fake_cache"


class DiscordHistoryDiagnosticsIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_discord_channel_history_sanitizes_message_content(self) -> None:
        message = FakeMessage(
            author=FakeAuthor(242, is_bot=False),
            content="sensitive prompt",
            created_at=datetime(2026, 6, 3, 15, 12, tzinfo=timezone.utc),
        )
        output = "\n".join(
            await bot.build_discord_channel_history_lines(cast(DiscordChannelLike, FakeHistoryChannel(222, [message])))
        )

        self.assertIn("Recent channel history:", output)
        self.assertIn("2026-06-03T15:12:00+00:00 bot=False content_len=16 type=default", output)
        self.assertNotIn("sensitive prompt", output)

    async def test_discord_tracked_target_history_sanitizes_message_content(self) -> None:
        user_message = FakeMessage(
            author=FakeAuthor(242, is_bot=False),
            content="sensitive prompt",
            created_at=datetime(2026, 6, 3, 15, 12, tzinfo=timezone.utc),
        )
        bot_message = FakeMessage(
            author=FakeAuthor(151, is_bot=True),
            content="bot startup",
            created_at=datetime(2026, 6, 3, 15, 13, tzinfo=timezone.utc),
        )

        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            mirror_path = Path(temp_dir) / "mirror.sqlite"
            with mock.patch.object(bot, "MIRROR_DB_PATH", mirror_path):
                output = "\n".join(
                    await bot.build_discord_tracked_target_user_history_lines(
                        cast(DiagnosticsHistoryBot, FakeBot(bot_message, user_message))
                    )
                )

        self.assertIn("Recent tracked target user history:", output)
        self.assertIn("startup channel=111 source=fake_cache latest_user=-", output)
        self.assertIn(
            "allowed channel=222 source=fake_cache "
            + "latest_user_at=2026-06-03T15:12:00+00:00 user=242 content_len=16 type=default",
            output,
        )
        self.assertNotIn("sensitive prompt", output)


if __name__ == "__main__":
    _ = unittest.main()
