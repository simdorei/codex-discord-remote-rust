from __future__ import annotations

from collections.abc import Awaitable
from pathlib import Path
from types import SimpleNamespace
from typing import Protocol, cast, override
import os
import tempfile
import unittest
from unittest import mock

from codex_discord_components import format_input_choice_custom_id
import codex_discord_bot as bot


class FakeFollowup:
    def __init__(self) -> None:
        self.messages: list[str] = []
        self.kwargs: list[dict[str, bool]] = []

    async def send(self, content: str, view: None = None, **kwargs: bool) -> None:
        _ = view
        self.messages.append(content)
        self.kwargs.append(kwargs)


class FakeResponse:
    def __init__(self) -> None:
        self.messages: list[str] = []
        self.send_message_kwargs: list[dict[str, bool]] = []
        self.deferred: bool = False
        self.done: bool = False
        self.defer_kwargs: list[dict[str, bool]] = []

    async def send_message(self, content: str, ephemeral: bool = False) -> None:
        self.messages.append(content)
        self.send_message_kwargs.append({"ephemeral": ephemeral})
        self.done = True

    async def defer(self, thinking: bool = False, **kwargs: bool) -> None:
        self.deferred = True
        self.done = True
        self.defer_kwargs.append({"thinking": thinking, **kwargs})

    def is_done(self) -> bool:
        return self.done


class FakeInteractionMessage:
    _next_id: int = 1000

    def __init__(self) -> None:
        self.id: int = FakeInteractionMessage._next_id
        FakeInteractionMessage._next_id += 1
        self.edits: list[None] = []

    async def edit(self, view: None = None) -> None:
        self.edits.append(view)


class FakeInteraction:
    def __init__(self, channel_id: int = 222) -> None:
        self.command: SimpleNamespace = SimpleNamespace(name="-")
        self.channel_id: int = channel_id
        self.followup: FakeFollowup = FakeFollowup()
        self.response: FakeResponse = FakeResponse()
        self.user: SimpleNamespace = SimpleNamespace(id=242286902982606848)
        self.channel: None = None
        self.message: FakeInteractionMessage = FakeInteractionMessage()
        self.data: dict[str, str] = {}


class ReportUnhandled(Protocol):
    def __call__(
        self,
        interaction: FakeInteraction,
        *,
        delay_sec: float = 0.75,
    ) -> Awaitable[None]: ...


def _report_unhandled() -> ReportUnhandled:
    return cast(ReportUnhandled, bot.report_unhandled_component_interaction)


class DiscordPersistentChoiceIntegrationTests(unittest.IsolatedAsyncioTestCase):
    _old_mirror_db_path: Path | None = None
    _old_discord_log_path: str | None = None
    _temp_dir: tempfile.TemporaryDirectory[str] | None = None

    @override
    def setUp(self) -> None:
        self._old_discord_log_path = os.environ.get("CODEX_DISCORD_LOG_PATH")
        self._old_mirror_db_path = bot.MIRROR_DB_PATH
        temp_dir = tempfile.TemporaryDirectory(ignore_cleanup_errors=True)
        self._temp_dir = temp_dir
        temp_path = Path(temp_dir.name)
        os.environ["CODEX_DISCORD_LOG_PATH"] = str(temp_path / "discord-smoke.log")
        bot.MIRROR_DB_PATH = temp_path / "mirror.sqlite"
        bot.init_mirror_db()
        bot.ACTIVE_DISCORD_DELIVERIES.clear()
        bot.clear_discord_delivery_stopping()

    @override
    def tearDown(self) -> None:
        if self._old_discord_log_path is None:
            _ = os.environ.pop("CODEX_DISCORD_LOG_PATH", None)
        else:
            os.environ["CODEX_DISCORD_LOG_PATH"] = self._old_discord_log_path
        if self._old_mirror_db_path is not None:
            bot.MIRROR_DB_PATH = self._old_mirror_db_path
        bot.ACTIVE_DISCORD_DELIVERIES.clear()
        bot.clear_discord_delivery_stopping()
        if self._temp_dir is not None:
            self._temp_dir.cleanup()
            self._temp_dir = None

    def _log_text(self) -> str:
        return Path(os.environ["CODEX_DISCORD_LOG_PATH"]).read_text(encoding="utf-8")

    async def test_persistent_approval_handles_restart_stale_view(self) -> None:
        submitted: list[tuple[str, str]] = []

        def fake_submit(target_thread_id: str, answer: str) -> tuple[int, str]:
            submitted.append((target_thread_id, answer))
            return 0, "approved"

        interaction = FakeInteraction()
        interaction.data = {"custom_id": "codex_approval:thread-1:2"}

        with mock.patch.object(bot, "submit_approval_reply", fake_submit):
            await _report_unhandled()(interaction, delay_sec=0)

        log_text = self._log_text()
        self.assertEqual(submitted, [("thread-1", "2")])
        self.assertTrue(interaction.response.deferred)
        self.assertEqual(interaction.message.edits, [None])
        self.assertEqual(interaction.followup.messages, ["Approval submitted\n\napproved"])
        self.assertIn("approval_persistent user=242286902982606848 target=thread-1 answer_len=1", log_text)
        self.assertIn("approval_persistent_done exit=0 target=thread-1 answer_len=1", log_text)
        self.assertIn("component_message_components_cleared context=approval_persistent", log_text)
        self.assertNotIn("approved session", log_text)

    async def test_persistent_input_choice_handles_restart_stale_view(self) -> None:
        submitted: list[tuple[str, str]] = []
        self.assertIsNone(format_input_choice_custom_id("thread-1", "first choice"))
        custom_id = format_input_choice_custom_id("thread-1", "choice-1")
        if custom_id is None:
            self.fail("expected compact input choice custom id")

        def fake_submit(target_thread_id: str, value: str) -> tuple[int, str]:
            submitted.append((target_thread_id, value))
            return 0, "answered"

        interaction = FakeInteraction()
        interaction.data = {"custom_id": custom_id}

        with mock.patch.object(bot, "submit_input_reply", fake_submit):
            await _report_unhandled()(interaction, delay_sec=0)

        log_text = self._log_text()
        self.assertEqual(submitted, [("thread-1", "choice-1")])
        self.assertTrue(interaction.response.deferred)
        self.assertEqual(interaction.message.edits, [None])
        self.assertEqual(interaction.followup.messages, ["Input submitted\n\nanswered"])
        self.assertIn("input_choice_persistent user=242286902982606848 target=thread-1 value_len=8", log_text)
        self.assertIn("input_choice_persistent_done exit=0 target=thread-1 value_len=8", log_text)
        self.assertIn("component_message_components_cleared context=input_choice_persistent", log_text)
        self.assertNotIn("choice-1", log_text)


if __name__ == "__main__":
    _ = unittest.main()
