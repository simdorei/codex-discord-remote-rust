from __future__ import annotations

from collections.abc import Awaitable, Sequence
from pathlib import Path
from typing import Protocol, cast, override
import tempfile
import unittest

import codex_discord_bot as bot
from codex_discord_components import parse_busy_choice_custom_id

from tests.test_codex_discord_bot import EnvPatch, ViewFailingTarget


class FakeAuthor:
    def __init__(self) -> None:
        self.id: int = 242286902982606848
        self.bot: bool = False


class FakeChannel:
    def __init__(self, channel_id: int) -> None:
        self.id: int = channel_id


class FakeMessage:
    def __init__(self, channel_id: int = 222) -> None:
        self.channel: FakeChannel | ViewFailingTarget = FakeChannel(channel_id)
        self.author: FakeAuthor = FakeAuthor()


class ChoiceItem(Protocol):
    label: str
    custom_id: str


class ChoiceView(Protocol):
    children: Sequence[ChoiceItem]


class MakeBusyChoiceView(Protocol):
    def __call__(
        self,
        source_message: FakeMessage,
        prompt: str,
        *,
        target_thread_id: str | None,
        allow_steer: bool = True,
    ) -> ChoiceView: ...


def _make_busy_choice_view() -> MakeBusyChoiceView:
    return cast(MakeBusyChoiceView, bot.make_busy_choice_view)


class SendBusyChoiceMessage(Protocol):
    def __call__(
        self,
        channel: ViewFailingTarget,
        source_message: FakeMessage,
        prompt: str,
        *,
        target_thread_id: str | None,
        allow_steer: bool,
        reason: str,
    ) -> Awaitable[bool]: ...


def _send_busy_choice_message() -> SendBusyChoiceMessage:
    return cast(SendBusyChoiceMessage, bot.send_busy_choice_message)


class DiscordBusyChoiceViewIntegrationTests(unittest.IsolatedAsyncioTestCase):
    _old_mirror_db_path: Path | None = None
    _temp_dir: tempfile.TemporaryDirectory[str] | None = None

    @override
    def setUp(self) -> None:
        self._old_mirror_db_path = bot.MIRROR_DB_PATH
        temp_dir = tempfile.TemporaryDirectory(ignore_cleanup_errors=True)
        self._temp_dir = temp_dir
        bot.MIRROR_DB_PATH = Path(temp_dir.name) / "mirror.sqlite"
        bot.init_mirror_db()

    @override
    def tearDown(self) -> None:
        if self._old_mirror_db_path is not None:
            bot.MIRROR_DB_PATH = self._old_mirror_db_path
        if self._temp_dir is not None:
            self._temp_dir.cleanup()
            self._temp_dir = None

    async def test_busy_choice_view_persists_custom_ids(self) -> None:
        view = _make_busy_choice_view()(
            FakeMessage(),
            "please steer",
            target_thread_id="thread-1",
            allow_steer=True,
        )
        custom_ids = {
            getattr(item, "label", ""): getattr(item, "custom_id", "")
            for item in view.children
        }

        self.assertRegex(custom_ids["Steer now"], r"^codex_busy:[0-9a-f]{24}:steer$")
        self.assertRegex(custom_ids["Queue next"], r"^codex_busy:[0-9a-f]{24}:queue$")
        self.assertRegex(custom_ids["Stop reply"], r"^codex_busy:[0-9a-f]{24}:stop$")
        self.assertRegex(custom_ids["Ignore"], r"^codex_busy:[0-9a-f]{24}:ignore$")
        self.assertIsNotNone(parse_busy_choice_custom_id(custom_ids["Stop reply"]))

    async def test_busy_choice_view_has_single_action_button_each(self) -> None:
        view = _make_busy_choice_view()(
            FakeMessage(),
            "please steer",
            target_thread_id="thread-1",
            allow_steer=True,
        )
        labels = [getattr(item, "label", "") for item in view.children]

        self.assertEqual(labels.count("Steer now"), 1)
        self.assertEqual(labels.count("Queue next"), 1)
        self.assertEqual(labels.count("Stop reply"), 1)
        self.assertEqual(labels.count("Ignore"), 1)

    async def test_busy_choice_send_reports_view_send_error(self) -> None:
        original_build_context_warning = bot.build_context_warning
        try:
            bot.build_context_warning = lambda target_thread_id: ""
            message = FakeMessage()
            channel = ViewFailingTarget()
            message.channel = channel

            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    sent_with_view = await _send_busy_choice_message()(
                        channel,
                        message,
                        "please steer",
                        target_thread_id="thread-1",
                        allow_steer=True,
                        reason="late_busy_failure",
                    )
                log_text = log_path.read_text(encoding="utf-8")

            self.assertFalse(sent_with_view)
            self.assertGreaterEqual(len(channel.messages), 1)
            error_text = "\n".join(content for content, view in channel.messages if view is None)
            self.assertIn("Busy choice failed", error_text)
            self.assertIn("ERROR: RuntimeError: view rejected", error_text)
            self.assertNotIn("Codex app is still processing this mapped thread.", error_text)
            self.assertNotIn("Discord could not attach steering buttons.", error_text)
            self.assertIn("busy_choice_send_failed reason=late_busy_failure", log_text)
            self.assertIn("busy_choice_error_sent reason=late_busy_failure", log_text)
            self.assertNotIn("busy_choice_sent reason=late_busy_failure", log_text)
        finally:
            bot.build_context_warning = original_build_context_warning


if __name__ == "__main__":
    _ = unittest.main()
