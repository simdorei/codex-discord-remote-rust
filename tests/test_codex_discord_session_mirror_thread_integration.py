from __future__ import annotations

from pathlib import Path
from collections.abc import Awaitable
from typing import Protocol, cast, override
import os
import tempfile
import unittest
from unittest import mock

import codex_discord_bot as bot
import codex_desktop_bridge as bridge


class ThreadLookupUnavailableError(RuntimeError):
    pass


class BadThreadLookupError(TypeError):
    pass


class SnapshotUnavailableError(RuntimeError):
    pass


class BadSnapshotDependencyError(TypeError):
    pass


class FakeChannel:
    def __init__(self, channel_id: int) -> None:
        self.id: int = channel_id
        self.messages: list[str] = []

    async def send(self, content: str) -> None:
        self.messages.append(content)


class ContextNoticeSender(Protocol):
    def __call__(
        self,
        channel: FakeChannel,
        target_thread_id: str | None,
        target_ref: str,
    ) -> Awaitable[bool]: ...


def _send_context_notice() -> ContextNoticeSender:
    return cast(ContextNoticeSender, bot.send_context_exhausted_prompt_notice_if_needed)


class DiscordSessionMirrorThreadIntegrationTests(unittest.IsolatedAsyncioTestCase):
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

    def test_should_delegate_output_to_session_mirror_thread_runtime_failure_logs(self) -> None:
        channel = FakeChannel(channel_id=444)

        with mock.patch.object(bot, "discord_session_mirror_enabled", return_value=True):
            with mock.patch.object(bot, "get_mirrored_codex_thread_id", return_value="thread-1"):
                with mock.patch.object(
                    bridge,
                    "choose_thread",
                    side_effect=ThreadLookupUnavailableError("thread unavailable"),
                ):
                    result = bot.should_delegate_output_to_session_mirror(channel, "thread-1")

        self.assertFalse(result)
        self.assertIn(
            (
                "session_mirror_delegate_disabled target=thread-1 "
                "reason=thread_unavailable error_type=ThreadLookupUnavailableError"
            ),
            self._log_text(),
        )

    def test_should_delegate_output_to_session_mirror_thread_type_error_is_not_disabled(self) -> None:
        channel = FakeChannel(channel_id=444)

        with mock.patch.object(bot, "discord_session_mirror_enabled", return_value=True):
            with mock.patch.object(bot, "get_mirrored_codex_thread_id", return_value="thread-1"):
                with mock.patch.object(
                    bridge,
                    "choose_thread",
                    side_effect=BadThreadLookupError("bad thread lookup"),
                ):
                    with self.assertRaisesRegex(TypeError, "bad thread lookup"):
                        _ = bot.should_delegate_output_to_session_mirror(channel, "thread-1")

        self.assertNotIn("session_mirror_delegate_disabled", self._log_text())

    async def test_send_context_exhausted_prompt_notice_thread_runtime_failure_logs(self) -> None:
        channel = FakeChannel(channel_id=777)

        with mock.patch.object(
            bridge,
            "choose_thread",
            side_effect=ThreadLookupUnavailableError("thread unavailable"),
        ):
            sent = await _send_context_notice()(
                channel,
                "thread-1",
                "Thread One",
            )

        self.assertFalse(sent)
        self.assertEqual(channel.messages, [])
        self.assertIn(
            "ask_context_guard_unavailable target=thread-1 error_type=ThreadLookupUnavailableError",
            self._log_text(),
        )

    async def test_send_context_exhausted_prompt_notice_thread_type_error_is_not_guard_unavailable(self) -> None:
        channel = FakeChannel(channel_id=777)

        with mock.patch.object(
            bridge,
            "choose_thread",
            side_effect=BadThreadLookupError("bad thread lookup"),
        ):
            with self.assertRaisesRegex(TypeError, "bad thread lookup"):
                _ = await _send_context_notice()(
                    channel,
                    "thread-1",
                    "Thread One",
                )

        self.assertEqual(channel.messages, [])
        self.assertNotIn("ask_context_guard_unavailable", self._log_text())

    def test_snapshot_ask_prompt_delivery_state_runtime_failure_logs_and_returns_empty(self) -> None:
        with mock.patch.object(
            bridge,
            "choose_thread",
            side_effect=SnapshotUnavailableError("snapshot unavailable"),
        ):
            target_thread, recent_offsets = bot.snapshot_ask_prompt_delivery_state("thread-1")

        self.assertIsNone(target_thread)
        self.assertEqual(recent_offsets, {})
        self.assertIn(
            "ask_delivery_snapshot_unavailable target=thread-1 error=snapshot unavailable",
            self._log_text(),
        )

    def test_snapshot_ask_prompt_delivery_state_type_error_is_not_snapshot_unavailable(self) -> None:
        with mock.patch.object(
            bridge,
            "choose_thread",
            side_effect=BadSnapshotDependencyError("bad snapshot dependency"),
        ):
            with self.assertRaisesRegex(TypeError, "bad snapshot dependency"):
                _ = bot.snapshot_ask_prompt_delivery_state("thread-1")

        self.assertNotIn("ask_delivery_snapshot_unavailable", self._log_text())


if __name__ == "__main__":
    _ = unittest.main()
