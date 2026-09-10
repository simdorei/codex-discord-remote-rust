from __future__ import annotations

from collections.abc import Awaitable
from pathlib import Path
from types import TracebackType
from typing import Never, Protocol, cast, override
import os
import tempfile
import unittest
from unittest import mock

import codex_discord_bot as bot
from codex_discord_components import parse_busy_choice_custom_id
from tests.test_codex_discord_busy_choice_view_integration import FakeMessage, MakeBusyChoiceView
from tests.test_codex_discord_persistent_choice_integration import FakeInteraction


class FakeTyping:
    async def __aenter__(self) -> None:
        return None

    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        _ = (exc_type, exc, traceback)


class FakeTarget:
    def __init__(self, channel_id: int = 222, parent_id: int | None = None) -> None:
        self.messages: list[tuple[str, None]] = []
        self.id: int = channel_id
        self.parent_id: int | None = parent_id

    async def send(self, content: str, view: None = None) -> None:
        self.messages.append((content, view))

    def typing(self) -> FakeTyping:
        return FakeTyping()


class ReportUnhandled(Protocol):
    def __call__(
        self,
        interaction: FakeInteraction,
        *,
        delay_sec: float = 0.75,
    ) -> Awaitable[None]: ...


def _report_unhandled() -> ReportUnhandled:
    return cast(ReportUnhandled, bot.report_unhandled_component_interaction)


def _make_busy_choice_view() -> MakeBusyChoiceView:
    return cast(MakeBusyChoiceView, bot.make_busy_choice_view)


def _custom_id_for_label(prompt: str, label: str) -> str:
    view = _make_busy_choice_view()(
        FakeMessage(),
        prompt,
        target_thread_id="thread-1",
        allow_steer=True,
    )
    custom_id = next(
        getattr(item, "custom_id", "")
        for item in view.children
        if getattr(item, "label", "") == label
    )
    if not custom_id:
        raise AssertionError(f"missing busy choice button: {label}")
    return custom_id


class DiscordPersistentBusyChoiceQueueChannelIntegrationTests(unittest.IsolatedAsyncioTestCase):
    _old_mirror_db_path: Path | None = None
    _old_discord_log_path: str | None = None
    _temp_dir: tempfile.TemporaryDirectory[str] | None = None

    @override
    def setUp(self) -> None:
        original_app_server_transport_enabled = bot.app_server_transport_enabled

        def restore_app_server_transport_enabled() -> None:
            bot.app_server_transport_enabled = original_app_server_transport_enabled

        self.addCleanup(restore_app_server_transport_enabled)
        bot.app_server_transport_enabled = lambda: False
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
        return Path(os.environ["CODEX_DISCORD_LOG_PATH"]).read_text(encoding="utf-8")

    async def test_persistent_busy_choice_ignore_handles_restart_stale_view(self) -> None:
        ignore_id = _custom_id_for_label("please ignore", "Ignore")
        parsed = parse_busy_choice_custom_id(ignore_id)
        if parsed is None:
            raise AssertionError(f"invalid busy choice custom id: {ignore_id}")
        choice_id, _action = parsed
        interaction = FakeInteraction()
        interaction.data = {"custom_id": ignore_id}

        await _report_unhandled()(interaction, delay_sec=0)

        log_text = self._log_text()
        remaining = bot.get_busy_choice_record(choice_id)
        self.assertEqual(interaction.response.messages, ["Ignored."])
        self.assertEqual(interaction.message.edits, [None])
        self.assertIsNone(remaining)
        self.assertIn("busy_choice_persistent_ignore", log_text)
        self.assertIn("component_message_components_cleared context=busy_choice_ignore", log_text)
        self.assertNotIn("please ignore", log_text)

    async def test_persistent_busy_choice_defers_before_channel_resolution(self) -> None:
        queue_id = _custom_id_for_label("please queue", "Queue next")
        observed_deferred: list[bool] = []

        async def fake_resolve(interaction: FakeInteraction, channel_id: int) -> FakeTarget:
            observed_deferred.append(interaction.response.deferred)
            return FakeTarget(channel_id=channel_id)

        def fake_busy_state(target_thread_id: str) -> tuple[str, str, str]:
            return ("busy", target_thread_id, "project:1")

        async def fake_enqueue(*_args: Never, **_kwargs: Never) -> int:
            return 1

        interaction = FakeInteraction()
        interaction.data = {"custom_id": queue_id}

        with (
            mock.patch.object(bot, "resolve_interaction_channel", fake_resolve),
            mock.patch.object(bot, "get_busy_state_for_thread", fake_busy_state),
            mock.patch.object(bot, "enqueue_thread_ask", fake_enqueue),
        ):
            await _report_unhandled()(interaction, delay_sec=0)

        log_text = self._log_text()
        self.assertEqual(observed_deferred, [True])
        self.assertEqual(interaction.message.edits, [None])
        self.assertIn("component_message_components_cleared context=busy_choice_queue", log_text)

    async def test_persistent_busy_choice_channel_unavailable_sends_followup(self) -> None:
        queue_id = _custom_id_for_label("please queue", "Queue next")

        async def fake_resolve(interaction: FakeInteraction, channel_id: int) -> None:
            _ = (interaction, channel_id)
            return None

        interaction = FakeInteraction()
        interaction.data = {"custom_id": queue_id}

        with mock.patch.object(bot, "resolve_interaction_channel", fake_resolve):
            await _report_unhandled()(interaction, delay_sec=0)

        log_text = self._log_text()
        self.assertEqual(interaction.response.defer_kwargs, [{"thinking": False, "ephemeral": False}])
        self.assertEqual(interaction.message.edits, [None])
        self.assertEqual(
            interaction.followup.messages,
            ["Discord channel is unavailable. Send the message again to get fresh controls."],
        )
        self.assertEqual(interaction.followup.kwargs, [{}])
        self.assertIn("button_followup_sent command=- context=persistent_channel_unavailable", log_text)
        self.assertIn("busy_choice_persistent_channel_unavailable action=queue", log_text)


if __name__ == "__main__":
    _ = unittest.main()
