from __future__ import annotations

from collections.abc import Awaitable
from pathlib import Path
from typing import Protocol, cast, override
import os
import tempfile
import unittest

import codex_discord_bot as bot
from tests.test_codex_discord_persistent_choice_integration import FakeInteraction, FakeResponse


class AlreadyAcknowledgedError(RuntimeError):
    pass


class AlreadyRespondedError(RuntimeError):
    pass


class FailingResponse(FakeResponse):
    def __init__(self, exc: RuntimeError) -> None:
        super().__init__()
        self.exc: RuntimeError = exc

    @override
    async def send_message(self, content: str, ephemeral: bool = False) -> None:
        _ = (content, ephemeral)
        raise self.exc

    @override
    async def defer(self, thinking: bool = False, **kwargs: bool) -> None:
        _ = (thinking, kwargs)
        raise self.exc

    @override
    def is_done(self) -> bool:
        return False


class ReportUnhandled(Protocol):
    def __call__(
        self,
        interaction: FakeInteraction,
        *,
        delay_sec: float = 0.75,
    ) -> Awaitable[None]: ...


class BusyChoiceHandler(Protocol):
    def __call__(
        self,
        interaction: FakeInteraction,
        custom_id: str,
    ) -> Awaitable[bool]: ...


def _report_unhandled() -> ReportUnhandled:
    return cast(ReportUnhandled, bot.report_unhandled_component_interaction)


def _busy_choice_handler() -> BusyChoiceHandler:
    return cast(BusyChoiceHandler, bot.handle_persistent_busy_choice_interaction)


class DiscordPersistentBusyChoiceBoundaryIntegrationTests(unittest.IsolatedAsyncioTestCase):
    _old_mirror_db_path: Path | None = None
    _old_discord_log_path: str | None = None
    _temp_dir: tempfile.TemporaryDirectory[str] | None = None

    @override
    def setUp(self) -> None:
        self._old_mirror_db_path = bot.MIRROR_DB_PATH
        self._old_discord_log_path = os.environ.get("CODEX_DISCORD_LOG_PATH")
        temp_dir = tempfile.TemporaryDirectory(ignore_cleanup_errors=True)
        self._temp_dir = temp_dir
        temp_path = Path(temp_dir.name)
        bot.MIRROR_DB_PATH = temp_path / "mirror.sqlite"
        os.environ["CODEX_DISCORD_LOG_PATH"] = str(temp_path / "discord-smoke.log")
        bot.init_mirror_db()
        bot.ACTIVE_DISCORD_DELIVERIES.clear()
        bot.clear_discord_delivery_stopping()

    @override
    def tearDown(self) -> None:
        if self._old_mirror_db_path is not None:
            bot.MIRROR_DB_PATH = self._old_mirror_db_path
        if self._old_discord_log_path is None:
            _ = os.environ.pop("CODEX_DISCORD_LOG_PATH", None)
        else:
            os.environ["CODEX_DISCORD_LOG_PATH"] = self._old_discord_log_path
        bot.ACTIVE_DISCORD_DELIVERIES.clear()
        bot.clear_discord_delivery_stopping()
        if self._temp_dir is not None:
            self._temp_dir.cleanup()
            self._temp_dir = None

    def _log_text(self) -> str:
        log_path = Path(os.environ["CODEX_DISCORD_LOG_PATH"])
        return log_path.read_text(encoding="utf-8") if log_path.exists() else ""

    async def test_unhandled_component_interaction_skips_already_handled_response(self) -> None:
        interaction = FakeInteraction()
        interaction.data = {"custom_id": "codex-busy-choice-active-button"}
        await interaction.response.defer(thinking=True)

        await _report_unhandled()(interaction, delay_sec=0)

        self.assertEqual(interaction.response.messages, [])
        self.assertFalse(Path(os.environ["CODEX_DISCORD_LOG_PATH"]).exists())

    async def test_persistent_busy_choice_missing_record_clears_buttons(self) -> None:
        custom_id = "codex_busy:0123456789abcdef01234567:steer"
        interaction = FakeInteraction()
        interaction.data = {"custom_id": custom_id}

        handled = await _busy_choice_handler()(interaction, custom_id)

        self.assertTrue(handled)
        self.assertEqual(
            interaction.response.messages,
            ["This Discord button is no longer active. Send the message again to get fresh controls."],
        )
        self.assertEqual(interaction.message.edits, [None])
        log_text = self._log_text()
        self.assertIn("busy_choice_persistent_missing action=steer", log_text)
        self.assertIn("component_message_components_cleared context=busy_choice_missing", log_text)

    async def test_persistent_busy_choice_already_acknowledged_logs_concise_marker(self) -> None:
        interaction = FakeInteraction()
        custom_id = "codex_busy:0123456789abcdef01234567:steer"
        interaction.data = {"custom_id": custom_id}
        interaction.response = FailingResponse(AlreadyAcknowledgedError(
            "Interaction has already been acknowledged."
        ))

        await _report_unhandled()(interaction, delay_sec=0)

        self.assertEqual(interaction.message.edits, [None])
        log_text = self._log_text()
        self.assertIn("component_interaction_persistent_handler_already_acknowledged", log_text)
        self.assertIn(f"custom_id={custom_id}", log_text)
        self.assertNotIn("Traceback", log_text)

    async def test_persistent_busy_choice_already_responded_logs_concise_marker(self) -> None:
        interaction = FakeInteraction()
        custom_id = "codex_busy:0123456789abcdef01234567:steer"
        interaction.data = {"custom_id": custom_id}
        interaction.response = FailingResponse(AlreadyRespondedError(
            "This interaction has already been responded to before."
        ))

        await _report_unhandled()(interaction, delay_sec=0)

        self.assertEqual(interaction.message.edits, [None])
        log_text = self._log_text()
        self.assertIn("component_interaction_persistent_handler_already_acknowledged", log_text)
        self.assertIn(f"custom_id={custom_id}", log_text)
        self.assertNotIn("Traceback", log_text)


if __name__ == "__main__":
    _ = unittest.main()
