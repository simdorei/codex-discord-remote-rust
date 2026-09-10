from __future__ import annotations

from collections.abc import Awaitable
from pathlib import Path
from typing import Protocol, TypeAlias, cast, override
import os
import tempfile
import unittest
from unittest import mock

import codex_desktop_bridge as bridge
import codex_discord_bot as bot
from codex_thread_models import ThreadInfo


class DeliveryWaitUnavailableError(RuntimeError):
    pass


class BadDeliveryDependencyError(TypeError):
    pass


class MirrorDeliveryUnavailableError(RuntimeError):
    pass


class BadMirrorDeliveryDependencyError(TypeError):
    pass


class FakeTarget:
    def __init__(self, channel_id: int = 888) -> None:
        self.id: int = channel_id
        self.messages: list[str] = []

    async def send(self, content: str) -> None:
        self.messages.append(content)


RecentOffsets: TypeAlias = dict[str, tuple[ThreadInfo, Path, int]]


class RecordedBusyHandler(Protocol):
    def __call__(
        self,
        channel: FakeTarget,
        prompt: str,
        *,
        target_thread_id: str | None,
        target_ref: str | None,
        recent_offsets: RecentOffsets,
        transport_output: str,
        delegate_to_session_mirror: bool,
    ) -> Awaitable[bool]: ...


class MirroredBusyWaiter(Protocol):
    def __call__(
        self,
        prompt: str,
        *,
        target_thread_id: str | None,
        recent_offsets: RecentOffsets,
    ) -> Awaitable[None]: ...


def _handle_recorded_busy_transport_prompt() -> RecordedBusyHandler:
    return cast(RecordedBusyHandler, bot.handle_recorded_busy_transport_prompt)


def _wait_for_mirrored_busy_delegation_settle() -> MirroredBusyWaiter:
    return cast(MirroredBusyWaiter, bot.wait_for_mirrored_busy_delegation_settle)


def _recent_offsets() -> RecentOffsets:
    thread = ThreadInfo(
        id="thread-1",
        title="Thread One",
        cwd="C:/repo",
        updated_at=1,
        rollout_path="C:/repo/session.jsonl",
        model="gpt",
        reasoning_effort="high",
        tokens_used=0,
    )
    return {"thread-1": (thread, Path("C:/repo/session.jsonl"), 0)}


async def _fake_wait_for_idle(
    target_thread_id: str | None,
    *,
    timeout_sec: float = 3600.0,
    poll_sec: float = 5.0,
) -> tuple[str, str | None, str]:
    _ = (timeout_sec, poll_sec)
    return "idle", target_thread_id, "Thread One"


class DiscordBusyDeliveryWaitIntegrationTests(unittest.IsolatedAsyncioTestCase):
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

    async def test_handle_recorded_busy_transport_prompt_runtime_failure_logs_and_returns_false(self) -> None:
        with mock.patch.object(
            bridge,
            "wait_for_prompt_delivery",
            side_effect=DeliveryWaitUnavailableError("delivery wait unavailable"),
        ):
            handled = await _handle_recorded_busy_transport_prompt()(
                FakeTarget(),
                "hello",
                target_thread_id="thread-1",
                target_ref="Thread One",
                recent_offsets=_recent_offsets(),
                transport_output="busy",
                delegate_to_session_mirror=False,
            )

        self.assertFalse(handled)
        self.assertIn(
            "ask_busy_delivery_verify_failed target=thread-1 error=delivery wait unavailable",
            self._log_text(),
        )

    async def test_handle_recorded_busy_transport_prompt_type_error_is_not_verify_failed(self) -> None:
        with mock.patch.object(
            bridge,
            "wait_for_prompt_delivery",
            side_effect=BadDeliveryDependencyError("bad delivery dependency"),
        ):
            with self.assertRaisesRegex(TypeError, "bad delivery dependency"):
                _ = await _handle_recorded_busy_transport_prompt()(
                    FakeTarget(),
                    "hello",
                    target_thread_id="thread-1",
                    target_ref="Thread One",
                    recent_offsets=_recent_offsets(),
                    transport_output="busy",
                    delegate_to_session_mirror=False,
                )

        self.assertNotIn("ask_busy_delivery_verify_failed", self._log_text())

    async def test_wait_for_mirrored_busy_delegation_settle_runtime_failure_logs_and_continues(self) -> None:
        with (
            mock.patch.object(bot, "get_steering_pending_watch_timeout", return_value=0.01),
            mock.patch.object(bot, "wait_for_codex_thread_idle", _fake_wait_for_idle),
            mock.patch.object(
                bridge,
                "wait_for_prompt_delivery",
                side_effect=MirrorDeliveryUnavailableError("mirror delivery unavailable"),
            ),
        ):
            await _wait_for_mirrored_busy_delegation_settle()(
                "hello",
                target_thread_id="thread-1",
                recent_offsets=_recent_offsets(),
            )

        log_text = self._log_text()
        self.assertIn(
            "ask_busy_mirror_delivery_wait_failed target=thread-1 error=mirror delivery unavailable",
            log_text,
        )
        self.assertIn("ask_busy_mirror_delivery_pending_timeout target=thread-1 timeout=0.01", log_text)
        self.assertIn("ask_busy_mirror_idle_wait_done target=thread-1", log_text)

    async def test_wait_for_mirrored_busy_delegation_settle_type_error_is_not_wait_failed(self) -> None:
        with (
            mock.patch.object(bot, "get_steering_pending_watch_timeout", return_value=0.01),
            mock.patch.object(bot, "wait_for_codex_thread_idle", _fake_wait_for_idle),
            mock.patch.object(
                bridge,
                "wait_for_prompt_delivery",
                side_effect=BadMirrorDeliveryDependencyError("bad mirror delivery dependency"),
            ),
        ):
            with self.assertRaisesRegex(TypeError, "bad mirror delivery dependency"):
                _ = await _wait_for_mirrored_busy_delegation_settle()(
                    "hello",
                    target_thread_id="thread-1",
                    recent_offsets=_recent_offsets(),
                )

        self.assertNotIn("ask_busy_mirror_delivery_wait_failed", self._log_text())


if __name__ == "__main__":
    _ = unittest.main()
