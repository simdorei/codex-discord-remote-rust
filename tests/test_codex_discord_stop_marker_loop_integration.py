from __future__ import annotations

from collections.abc import Coroutine
import importlib
from pathlib import Path
from typing import Protocol, runtime_checkable
import asyncio  # noqa: ANYIO_OK
import re
import tempfile
import unittest
from unittest import mock

from codex_process_identity import current_process_identity


class RequestedExit(Exception):
    pass


class StopMarkerClient:
    def __init__(self, *, close_delay: float = 0.0, mark_closed: bool = True) -> None:
        self.closed: bool = False
        self.close_calls: int = 0
        self._close_delay: float = close_delay
        self._mark_closed: bool = mark_closed

    def is_closed(self) -> bool:
        return self.closed

    async def close(self) -> None:
        self.close_calls += 1
        if self._close_delay:
            await asyncio.sleep(self._close_delay)
        if self._mark_closed:
            self.closed = True


class StopMarkerLoop(Protocol):
    def __call__(self, client: StopMarkerClient) -> Coroutine[None, None, None]: ...


class StopMarkerBotType(Protocol):
    def stop_marker_loop(
        self,
        client: StopMarkerClient,
    ) -> Coroutine[None, None, None]: ...


@runtime_checkable
class BotModule(Protocol):
    CodexDiscordBot: StopMarkerBotType
    ACTIVE_DISCORD_DELIVERIES: set[str]
    SCRIPT_DIR: Path
    STOP_MARKER_CLOSE_TIMEOUT_SECONDS: float
    STOP_MARKER_DRAIN_TIMEOUT_SECONDS: float

    def begin_discord_delivery(self, label: str) -> str: ...

    def end_discord_delivery(self, token: str) -> None: ...

    def clear_discord_delivery_stopping(self) -> None: ...


def _load_bot_module() -> BotModule:
    module = importlib.import_module("codex_discord_bot")
    if not isinstance(module, BotModule):
        raise AssertionError("codex_discord_bot compatibility exports are incomplete")
    return module


bot = _load_bot_module()


def _stop_marker_loop() -> StopMarkerLoop:
    return bot.CodexDiscordBot.stop_marker_loop


class DiscordStopMarkerLoopIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_ready_stop_watcher_updates_event_loop_heartbeat(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            marker_path = Path(temp_dir) / ".codex_discord_bot.stop"
            heartbeat_path = Path(temp_dir) / ".codex_discord_bot.heartbeat"
            client = StopMarkerClient()

            with (
                mock.patch.object(bot, "STOP_REQUEST_PATH", marker_path),
                mock.patch.object(bot, "STOP_MARKER_POLL_SECONDS", 0.01),
            ):
                task = asyncio.create_task(_stop_marker_loop()(client))
                await asyncio.sleep(0.05)
                client.closed = True
                await asyncio.wait_for(task, timeout=1)

            self.assertTrue(heartbeat_path.exists())

    async def test_stop_marker_waits_for_discord_delivery_drain(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            marker_path = Path(temp_dir) / ".codex_discord_bot.stop"
            _write_current_process_marker(marker_path)
            client = StopMarkerClient()
            exit_calls: list[tuple[int, str]] = []

            def exit_bot_process(exit_code: int, *, reason: str) -> None:
                exit_calls.append((exit_code, reason))

            try:
                bot.ACTIVE_DISCORD_DELIVERIES.clear()
                delivery_token = bot.begin_discord_delivery("test")
                with (
                    mock.patch.object(bot, "STOP_REQUEST_PATH", marker_path),
                    mock.patch.object(bot, "STOP_MARKER_POLL_SECONDS", 0.01),
                    mock.patch.object(bot, "STOP_MARKER_DRAIN_TIMEOUT_SECONDS", 1.0),
                    mock.patch.object(bot, "STOP_MARKER_CLOSE_TIMEOUT_SECONDS", 1.0),
                    mock.patch.object(bot, "exit_bot_process", exit_bot_process),
                ):
                    task = asyncio.create_task(_stop_marker_loop()(client))
                    await asyncio.sleep(0.05)

                    self.assertFalse(client.closed)

                    bot.end_discord_delivery(delivery_token)
                    await asyncio.wait_for(task, timeout=1.0)
            finally:
                bot.ACTIVE_DISCORD_DELIVERIES.clear()
                bot.clear_discord_delivery_stopping()

            self.assertTrue(client.closed)
            self.assertEqual(exit_calls, [])

    async def test_stop_marker_loop_closes_bot_and_removes_marker(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            marker_path = Path(temp_dir) / ".codex_discord_bot.stop"
            _write_current_process_marker(marker_path)
            client = StopMarkerClient()
            exit_calls: list[tuple[int, str]] = []

            def exit_bot_process(exit_code: int, *, reason: str) -> None:
                exit_calls.append((exit_code, reason))

            try:
                with (
                    mock.patch.object(bot, "STOP_REQUEST_PATH", marker_path),
                    mock.patch.object(bot, "STOP_MARKER_POLL_SECONDS", 0.01),
                    mock.patch.object(bot, "exit_bot_process", exit_bot_process),
                ):
                    await asyncio.wait_for(_stop_marker_loop()(client), timeout=1)
            finally:
                bot.clear_discord_delivery_stopping()

            self.assertEqual(client.close_calls, 1)
            self.assertEqual(exit_calls, [])
            self.assertFalse(marker_path.exists())

    async def test_stop_marker_rejects_a_stale_process_identity(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            marker_path = Path(temp_dir) / ".codex_discord_bot.stop"
            _ = marker_path.write_text("identity=999999|1", encoding="utf-8")
            client = StopMarkerClient()

            with (
                mock.patch.object(bot, "STOP_REQUEST_PATH", marker_path),
                mock.patch.object(bot, "STOP_MARKER_POLL_SECONDS", 0.01),
            ):
                task = asyncio.create_task(_stop_marker_loop()(client))
                for _ in range(100):
                    if not marker_path.exists():
                        break
                    await asyncio.sleep(0.01)
                client.closed = True
                await asyncio.wait_for(task, timeout=1)

            self.assertFalse(marker_path.exists())
            self.assertEqual(client.close_calls, 0)

    async def test_stop_marker_close_timeout_requests_process_exit(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            marker_path = Path(temp_dir) / ".codex_discord_bot.stop"
            _write_current_process_marker(marker_path)
            client = StopMarkerClient(close_delay=1.0, mark_closed=False)
            exit_calls: list[tuple[int, str]] = []

            def exit_bot_process(exit_code: int, *, reason: str) -> None:
                exit_calls.append((exit_code, reason))
                raise RequestedExit()

            try:
                with (
                    mock.patch.object(bot, "STOP_REQUEST_PATH", marker_path),
                    mock.patch.object(bot, "STOP_MARKER_POLL_SECONDS", 0.01),
                    mock.patch.object(bot, "STOP_MARKER_DRAIN_TIMEOUT_SECONDS", 0.01),
                    mock.patch.object(bot, "STOP_MARKER_CLOSE_TIMEOUT_SECONDS", 0.01),
                    mock.patch.object(bot, "exit_bot_process", exit_bot_process),
                    self.assertRaises(RequestedExit),
                ):
                    await _stop_marker_loop()(client)
            finally:
                bot.clear_discord_delivery_stopping()

            self.assertEqual(client.close_calls, 1)
            self.assertEqual(exit_calls, [(0, "stop_marker_close_timeout")])
            self.assertFalse(marker_path.exists())

    def test_watchdog_graceful_stop_wait_exceeds_bot_drain_timeout(self) -> None:
        watchdog_text = "\n".join(
            [
                (bot.SCRIPT_DIR / "codex-discord-watchdog.ps1").read_text(
                    encoding="utf-8"
                ),
                (bot.SCRIPT_DIR / "codex-discord-watchdog-runtime.ps1").read_text(
                    encoding="utf-8"
                ),
                (
                    bot.SCRIPT_DIR / "codex-discord-watchdog-restart-runtime.ps1"
                ).read_text(encoding="utf-8"),
            ]
        )
        match = re.search(r"\$GracefulStopTimeoutSeconds\s*=\s*(\d+)", watchdog_text)

        if match is None:
            self.fail("GracefulStopTimeoutSeconds setting missing")
        timeout_seconds = int(match.group(1))
        expected_minimum = int(
            bot.STOP_MARKER_DRAIN_TIMEOUT_SECONDS
            + bot.STOP_MARKER_CLOSE_TIMEOUT_SECONDS
            + 5
        )
        self.assertGreaterEqual(timeout_seconds, expected_minimum)
        self.assertIn(
            "-ExpectedIdentity $ExpectedIdentity",
            watchdog_text,
        )


def _write_current_process_marker(path: Path) -> None:
    _ = path.write_text(
        f"identity={current_process_identity()}",
        encoding="utf-8",
    )


if __name__ == "__main__":
    _ = unittest.main()
