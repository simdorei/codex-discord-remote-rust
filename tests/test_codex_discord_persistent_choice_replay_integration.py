from __future__ import annotations

from collections.abc import Awaitable, Callable
from pathlib import Path
from typing import Protocol, TypeAlias, cast, override
import os
import tempfile
import unittest

from codex_discord_components import format_input_choice_custom_id
import codex_discord_bot as bot
from tests.test_codex_discord_persistent_choice_integration import FakeInteraction, FakeInteractionMessage


Submitter: TypeAlias = Callable[[str, str], tuple[int, str]]


class ApprovalHandler(Protocol):
    def __call__(
        self,
        interaction: FakeInteraction,
        custom_id: str,
        *,
        approval_submitter: Submitter,
    ) -> Awaitable[bool]: ...


class InputChoiceHandler(Protocol):
    def __call__(
        self,
        interaction: FakeInteraction,
        custom_id: str,
        *,
        input_submitter: Submitter,
    ) -> Awaitable[bool]: ...


def _approval_handler() -> ApprovalHandler:
    return cast(ApprovalHandler, bot.handle_persistent_approval_interaction)


def _input_choice_handler() -> InputChoiceHandler:
    return cast(InputChoiceHandler, bot.handle_persistent_input_choice_interaction)


class DiscordPersistentChoiceReplayIntegrationTests(unittest.IsolatedAsyncioTestCase):
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

    @override
    def tearDown(self) -> None:
        if self._old_mirror_db_path is not None:
            bot.MIRROR_DB_PATH = self._old_mirror_db_path
        if self._old_discord_log_path is None:
            _ = os.environ.pop("CODEX_DISCORD_LOG_PATH", None)
        else:
            os.environ["CODEX_DISCORD_LOG_PATH"] = self._old_discord_log_path
        if self._temp_dir is not None:
            self._temp_dir.cleanup()
            self._temp_dir = None

    def _log_text(self) -> str:
        return Path(os.environ["CODEX_DISCORD_LOG_PATH"]).read_text(encoding="utf-8")

    async def test_persistent_approval_replay_is_single_use_per_message(self) -> None:
        submitted: list[tuple[str, str]] = []

        def fake_submit(target_thread_id: str, answer: str) -> tuple[int, str]:
            submitted.append((target_thread_id, answer))
            return 0, "approved"

        shared_message = FakeInteractionMessage()
        first = FakeInteraction()
        first.message = shared_message
        second = FakeInteraction()
        second.message = shared_message

        first_handled = await _approval_handler()(
            first,
            "codex_approval:thread-1:2",
            approval_submitter=fake_submit,
        )
        second_handled = await _approval_handler()(
            second,
            "codex_approval:thread-1:3",
            approval_submitter=fake_submit,
        )

        log_text = self._log_text()
        self.assertTrue(first_handled)
        self.assertTrue(second_handled)
        self.assertEqual(submitted, [("thread-1", "2")])
        self.assertEqual(second.response.messages, ["This approval choice was already handled."])
        self.assertIn("approval_persistent_already_handled user=242286902982606848 target=thread-1", log_text)
        self.assertIn("component_message_components_cleared context=approval_persistent_already_handled", log_text)

    async def test_persistent_input_choice_replay_is_single_use_per_message(self) -> None:
        submitted: list[tuple[str, str]] = []
        first_custom_id = format_input_choice_custom_id("thread-1", "choice-1")
        second_custom_id = format_input_choice_custom_id("thread-1", "choice-2")
        if first_custom_id is None or second_custom_id is None:
            self.fail("expected compact input choice custom ids")

        def fake_submit(target_thread_id: str, value: str) -> tuple[int, str]:
            submitted.append((target_thread_id, value))
            return 0, "answered"

        shared_message = FakeInteractionMessage()
        first = FakeInteraction()
        first.message = shared_message
        second = FakeInteraction()
        second.message = shared_message

        first_handled = await _input_choice_handler()(
            first,
            first_custom_id,
            input_submitter=fake_submit,
        )
        second_handled = await _input_choice_handler()(
            second,
            second_custom_id,
            input_submitter=fake_submit,
        )

        log_text = self._log_text()
        self.assertTrue(first_handled)
        self.assertTrue(second_handled)
        self.assertEqual(submitted, [("thread-1", "choice-1")])
        self.assertEqual(second.response.messages, ["This input choice was already handled."])
        self.assertIn("input_choice_persistent_already_handled user=242286902982606848 target=thread-1", log_text)
        self.assertIn("component_message_components_cleared context=input_choice_persistent_already_handled", log_text)


if __name__ == "__main__":
    _ = unittest.main()
