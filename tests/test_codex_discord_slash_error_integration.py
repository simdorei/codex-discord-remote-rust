from __future__ import annotations

from collections.abc import Awaitable
from pathlib import Path
from typing import Protocol, cast
import tempfile
import unittest
from unittest import mock

import codex_discord_bot as bot

from tests.test_codex_discord_bot import EnvPatch, FakeInteraction


class SlashCommandFailure(Exception):
    pass


class SlashResponseUnavailable(RuntimeError):
    pass


class BadSlashResponseDependency(TypeError):
    pass


class CommandTreeStub:
    pass


class SlashErrorHandler(Protocol):
    def __call__(
        self,
        tree: CommandTreeStub,
        interaction: FakeInteraction,
        error: Exception,
        /,
    ) -> Awaitable[None]:
        ...


def _on_error() -> SlashErrorHandler:
    return cast(SlashErrorHandler, bot.LoggingCommandTree.on_error)


class DiscordSlashErrorIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_slash_error_handler_reports_before_initial_response(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            interaction = FakeInteraction(command_name="ask", channel_id=222)
            with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                await _on_error()(
                    CommandTreeStub(),
                    interaction,
                    SlashCommandFailure("boom"),
                )

            self.assertEqual(
                interaction.response.messages,
                ["Discord slash command error. Check codex_discord_bot.log."],
            )
            self.assertEqual(interaction.followup.messages, [])
            log_text = log_path.read_text(encoding="utf-8")
            self.assertIn("slash_command_error command=ask channel=222", log_text)
            self.assertIn("slash_command_error_sent command=ask response=initial", log_text)

    async def test_slash_error_handler_reports_after_defer(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            interaction = FakeInteraction(command_name="ask", channel_id=222)
            interaction.response.deferred = True
            interaction.response.done = True
            with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                await _on_error()(
                    CommandTreeStub(),
                    interaction,
                    SlashCommandFailure("boom"),
                )

            self.assertEqual(interaction.response.messages, [])
            self.assertEqual(
                interaction.followup.messages,
                ["Discord slash command error. Check codex_discord_bot.log."],
            )
            self.assertEqual(interaction.followup.kwargs, [{"ephemeral": True}])
            log_text = log_path.read_text(encoding="utf-8")
            self.assertIn("slash_command_error command=ask channel=222", log_text)
            self.assertIn("slash_command_error_sent command=ask response=followup", log_text)

    async def test_slash_error_handler_delivery_failure_logs_report_failure(self) -> None:
        async def fail_response(
            interaction: FakeInteraction,
            content: str,
            *,
            ephemeral: bool = False,
            context: str = "interaction_response",
            allow_during_stop: bool = False,
        ) -> None:
            _ = (interaction, content, ephemeral, context, allow_during_stop)
            raise SlashResponseUnavailable("slash response unavailable")

        interaction = FakeInteraction(command_name="ask", channel_id=222)
        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                with mock.patch.object(bot, "send_interaction_response_tracked", fail_response):
                    await _on_error()(
                        CommandTreeStub(),
                        interaction,
                        SlashCommandFailure("boom"),
                    )
            log_text = log_path.read_text(encoding="utf-8")

        self.assertIn("slash_command_error command=ask channel=222", log_text)
        self.assertIn("slash_command_error_report_failed", log_text)
        self.assertIn("slash response unavailable", log_text)

    async def test_slash_error_handler_type_error_is_not_report_failure(self) -> None:
        async def fail_response(
            interaction: FakeInteraction,
            content: str,
            *,
            ephemeral: bool = False,
            context: str = "interaction_response",
            allow_during_stop: bool = False,
        ) -> None:
            _ = (interaction, content, ephemeral, context, allow_during_stop)
            raise BadSlashResponseDependency("bad slash response dependency")

        interaction = FakeInteraction(command_name="ask", channel_id=222)
        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                with mock.patch.object(bot, "send_interaction_response_tracked", fail_response):
                    with self.assertRaisesRegex(TypeError, "bad slash response dependency"):
                        await _on_error()(
                            CommandTreeStub(),
                            interaction,
                            SlashCommandFailure("boom"),
                        )
            log_text = log_path.read_text(encoding="utf-8") if log_path.exists() else ""

        self.assertIn("slash_command_error command=ask channel=222", log_text)
        self.assertNotIn("slash_command_error_report_failed", log_text)


if __name__ == "__main__":
    _ = unittest.main()
