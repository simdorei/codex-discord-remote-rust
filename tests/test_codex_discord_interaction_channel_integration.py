from __future__ import annotations

from collections.abc import Awaitable, Callable
from pathlib import Path
from typing import Protocol, TypeAlias, cast, override
import os
import tempfile
import unittest

import codex_discord_bot as bot


class ChannelFetchUnavailableError(RuntimeError):
    pass


class BadFetchSignatureError(TypeError):
    pass


class FakeTarget:
    def __init__(self, channel_id: int = 333) -> None:
        self.id: int = channel_id
        self.messages: list[str] = []

    async def send(self, content: str) -> None:
        self.messages.append(content)


FetchChannel: TypeAlias = Callable[[int], Awaitable[FakeTarget]]


class FakeClient:
    def __init__(self, fetch_channel: FetchChannel) -> None:
        self.fetch_channel: FetchChannel = fetch_channel


class FakeInteraction:
    def __init__(self, channel_id: int = 222) -> None:
        self.channel_id: int = channel_id
        self.channel: FakeTarget | None = None
        self.client: FakeClient | None = None


class ResolveInteractionChannel(Protocol):
    def __call__(
        self,
        interaction: FakeInteraction,
        channel_id: int,
    ) -> Awaitable[FakeTarget | None]: ...


def _resolve_interaction_channel() -> ResolveInteractionChannel:
    return cast(ResolveInteractionChannel, bot.resolve_interaction_channel)


class DiscordInteractionChannelIntegrationTests(unittest.IsolatedAsyncioTestCase):
    _old_discord_log_path: str | None = None
    _temp_dir: tempfile.TemporaryDirectory[str] | None = None

    @override
    def setUp(self) -> None:
        self._old_discord_log_path = os.environ.get("CODEX_DISCORD_LOG_PATH")
        temp_dir = tempfile.TemporaryDirectory(ignore_cleanup_errors=True)
        self._temp_dir = temp_dir
        os.environ["CODEX_DISCORD_LOG_PATH"] = str(Path(temp_dir.name) / "discord-smoke.log")

    @override
    def tearDown(self) -> None:
        if self._old_discord_log_path is None:
            _ = os.environ.pop("CODEX_DISCORD_LOG_PATH", None)
        else:
            os.environ["CODEX_DISCORD_LOG_PATH"] = self._old_discord_log_path
        if self._temp_dir is not None:
            self._temp_dir.cleanup()
            self._temp_dir = None

    def _log_text(self) -> str:
        log_path = Path(os.environ["CODEX_DISCORD_LOG_PATH"])
        return log_path.read_text(encoding="utf-8") if log_path.exists() else ""

    async def test_resolve_interaction_channel_fetches_messageable_client_channel(self) -> None:
        fetched_channel = FakeTarget(channel_id=333)

        async def fetch_channel(channel_id: int) -> FakeTarget:
            self.assertEqual(channel_id, 333)
            return fetched_channel

        interaction = FakeInteraction()
        interaction.client = FakeClient(fetch_channel)

        resolved = await _resolve_interaction_channel()(interaction, 333)

        self.assertIs(resolved, fetched_channel)

    async def test_resolve_interaction_channel_fetch_runtime_failure_logs_and_returns_none(self) -> None:
        async def fetch_channel(channel_id: int) -> FakeTarget:
            self.assertEqual(channel_id, 333)
            raise ChannelFetchUnavailableError("fetch unavailable")

        interaction = FakeInteraction()
        interaction.client = FakeClient(fetch_channel)

        resolved = await _resolve_interaction_channel()(interaction, 333)

        self.assertIsNone(resolved)
        self.assertIn(
            (
                "busy_choice_persistent_channel_fetch_failed channel=333 "
                "error_type=ChannelFetchUnavailableError"
            ),
            self._log_text(),
        )

    async def test_resolve_interaction_channel_fetch_type_error_is_not_fetch_failure(self) -> None:
        async def fetch_channel(channel_id: int) -> FakeTarget:
            self.assertEqual(channel_id, 333)
            raise BadFetchSignatureError("bad fetch signature")

        interaction = FakeInteraction()
        interaction.client = FakeClient(fetch_channel)

        with self.assertRaisesRegex(TypeError, "bad fetch signature"):
            _ = await _resolve_interaction_channel()(interaction, 333)

        self.assertNotIn("busy_choice_persistent_channel_fetch_failed", self._log_text())


if __name__ == "__main__":
    _ = unittest.main()
