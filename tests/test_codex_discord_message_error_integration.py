from __future__ import annotations

from collections.abc import Awaitable
from pathlib import Path
from typing import Protocol, cast
import tempfile
import unittest
from unittest import mock

import codex_discord_bot as bot

from tests.test_codex_discord_bot import EnvPatch, FakeMessage, FakeTarget


class PrefixHandlerUnavailable(RuntimeError):
    pass


class BadPrefixDependency(TypeError):
    pass


class MessageReportUnavailable(RuntimeError):
    pass


class BadMessageReportDependency(TypeError):
    pass


class ProcessDiscordMessage(Protocol):
    def __call__(
        self,
        client: bot.CodexDiscordBot,
        message: FakeMessage,
        *,
        source: str,
    ) -> Awaitable[None]:
        ...


class MessageErrorClient:
    enable_prefix_commands: bool = True

    def is_allowed_message_channel(self, channel: FakeTarget) -> bool:
        _ = channel
        return True

    def is_allowed_user(self, user_id: int | None) -> bool:
        _ = user_id
        return True


def _client() -> bot.CodexDiscordBot:
    return cast(bot.CodexDiscordBot, cast(object, MessageErrorClient()))


def _message_for_bot(message: FakeMessage) -> FakeMessage:
    return message


def _process_message() -> ProcessDiscordMessage:
    return cast(ProcessDiscordMessage, bot.CodexDiscordBot.process_discord_message)


class DiscordMessageErrorIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_process_message_runtime_error_logs_and_reports(self) -> None:
        async def fail_prefix_command(
            client: bot.CodexDiscordBot,
            message: FakeMessage,
            command: str,
        ) -> None:
            _ = (client, message, command)
            raise PrefixHandlerUnavailable("prefix handler unavailable")

        fake_message = FakeMessage(content="!status", channel_id=333)

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                with mock.patch.object(bot, "handle_prefix_command", fail_prefix_command):
                    await _process_message()(
                        _client(),
                        _message_for_bot(fake_message),
                        source="test",
                    )
            log_text = log_path.read_text(encoding="utf-8")

        self.assertEqual(
            fake_message.channel.messages,
            [("Discord bot error. Check codex_discord_bot.log.", None)],
        )
        self.assertIn("on_message_error", log_text)
        self.assertIn("prefix handler unavailable", log_text)

    async def test_process_message_type_error_is_not_message_error(self) -> None:
        async def fail_prefix_command(
            client: bot.CodexDiscordBot,
            message: FakeMessage,
            command: str,
        ) -> None:
            _ = (client, message, command)
            raise BadPrefixDependency("bad prefix dependency")

        fake_message = FakeMessage(content="!status", channel_id=333)

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                with mock.patch.object(bot, "handle_prefix_command", fail_prefix_command):
                    with self.assertRaisesRegex(TypeError, "bad prefix dependency"):
                        await _process_message()(
                            _client(),
                            _message_for_bot(fake_message),
                            source="test",
                        )
            log_text = log_path.read_text(encoding="utf-8") if log_path.exists() else ""

        self.assertEqual(fake_message.channel.messages, [])
        self.assertNotIn("on_message_error", log_text)

    async def test_process_message_error_report_runtime_failure_logs_report_failed(self) -> None:
        async def fail_prefix_command(
            client: bot.CodexDiscordBot,
            message: FakeMessage,
            command: str,
        ) -> None:
            _ = (client, message, command)
            raise PrefixHandlerUnavailable("prefix handler unavailable")

        async def fail_send_chunks(channel: FakeTarget, text: str) -> None:
            _ = (channel, text)
            raise MessageReportUnavailable("message report unavailable")

        fake_message = FakeMessage(content="!status", channel_id=333)

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                with (
                    mock.patch.object(bot, "handle_prefix_command", fail_prefix_command),
                    mock.patch.object(bot, "send_chunks", fail_send_chunks),
                ):
                    await _process_message()(
                        _client(),
                        _message_for_bot(fake_message),
                        source="test",
                    )
            log_text = log_path.read_text(encoding="utf-8")

        self.assertIn("on_message_error", log_text)
        self.assertIn("on_message_error_report_failed", log_text)
        self.assertIn("message report unavailable", log_text)

    async def test_process_message_error_report_type_error_is_not_report_failed(self) -> None:
        async def fail_prefix_command(
            client: bot.CodexDiscordBot,
            message: FakeMessage,
            command: str,
        ) -> None:
            _ = (client, message, command)
            raise PrefixHandlerUnavailable("prefix handler unavailable")

        async def fail_send_chunks(channel: FakeTarget, text: str) -> None:
            _ = (channel, text)
            raise BadMessageReportDependency("bad message report dependency")

        fake_message = FakeMessage(content="!status", channel_id=333)

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                with (
                    mock.patch.object(bot, "handle_prefix_command", fail_prefix_command),
                    mock.patch.object(bot, "send_chunks", fail_send_chunks),
                ):
                    with self.assertRaisesRegex(TypeError, "bad message report dependency"):
                        await _process_message()(
                            _client(),
                            _message_for_bot(fake_message),
                            source="test",
                        )
            log_text = log_path.read_text(encoding="utf-8") if log_path.exists() else ""

        self.assertIn("on_message_error", log_text)
        self.assertNotIn("on_message_error_report_failed", log_text)


if __name__ == "__main__":
    _ = unittest.main()
