from __future__ import annotations

import os
import tempfile
import unittest
from collections.abc import Awaitable, Callable, Mapping
from pathlib import Path
from typing import Protocol, cast
from unittest import mock

import codex_discord_bot as bot
import codex_discord_session_mirror as session_mirror


SessionMirrorTarget = Mapping[str, session_mirror.SessionMirrorTargetValue]


class SessionMirrorTargetLoadError(RuntimeError):
    pass


class BadSessionMirrorTargetDependencyError(TypeError):
    pass


class SessionMirrorLoopClient(Protocol):
    session_mirror_poll_seconds: float

    def is_closed(self) -> bool: ...

    async def mirror_session_target(self, target: SessionMirrorTarget) -> None: ...


class SessionMirrorLoopFunc(Protocol):
    def __call__(self, client: SessionMirrorLoopClient, /) -> Awaitable[None]: ...


SESSION_MIRROR_LOOP = cast(SessionMirrorLoopFunc, bot.CodexDiscordBot.session_mirror_loop)


class FakeSessionMirrorClient:
    def __init__(self, *, close_after_checks: int) -> None:
        self.session_mirror_poll_seconds: float = 0.01
        self._session_mirror_last_at: str = "-"
        self._closed_checks: int = 0
        self._close_after_checks: int = close_after_checks
        self.mirror_calls: list[SessionMirrorTarget] = []

    def is_closed(self) -> bool:
        self._closed_checks += 1
        return self._closed_checks > self._close_after_checks

    async def mirror_session_target(self, target: SessionMirrorTarget) -> None:
        self.mirror_calls.append(target)


class DiscordSessionMirrorLoopIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def _run_with_log(self, action: Awaitable[None]) -> str:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with mock.patch.dict(os.environ, {"CODEX_DISCORD_LOG_PATH": str(log_path)}):
                await action
            return log_path.read_text(encoding="utf-8") if log_path.exists() else ""

    async def test_session_mirror_loop_continues_after_runtime_cycle_error(self) -> None:
        calls: list[str] = []
        client = FakeSessionMirrorClient(close_after_checks=2)

        async def fake_to_thread(
            func: Callable[[Path], list[SessionMirrorTarget]],
            mirror_db_path: Path,
            *,
            limit: int,
        ) -> list[SessionMirrorTarget]:
            _ = func, mirror_db_path, limit
            calls.append("to_thread")
            if len(calls) == 1:
                raise SessionMirrorTargetLoadError("session mirror target load unavailable")
            return []

        async def fake_sleep(seconds: float) -> None:
            _ = seconds

        with (
            mock.patch("codex_discord_bot.asyncio.to_thread", fake_to_thread),
            mock.patch("codex_discord_bot.asyncio.sleep", fake_sleep),
        ):
            log_text = await self._run_with_log(SESSION_MIRROR_LOOP(client))

        self.assertEqual(calls, ["to_thread", "to_thread"])
        self.assertIn("session_mirror_cycle_failed", log_text)
        self.assertIn("SessionMirrorTargetLoadError: session mirror target load unavailable", log_text)

    async def test_session_mirror_loop_continues_after_unexpected_cycle_error(self) -> None:
        calls: list[str] = []
        sleeps: list[float] = []
        client = FakeSessionMirrorClient(close_after_checks=2)

        async def fake_to_thread(
            func: Callable[[Path], list[SessionMirrorTarget]],
            mirror_db_path: Path,
            *,
            limit: int,
        ) -> list[SessionMirrorTarget]:
            _ = func, mirror_db_path, limit
            calls.append("to_thread")
            if len(calls) == 1:
                raise BadSessionMirrorTargetDependencyError("bad session mirror target dependency")
            return []

        async def fake_sleep(seconds: float) -> None:
            sleeps.append(seconds)

        with (
            mock.patch("codex_discord_bot.asyncio.to_thread", fake_to_thread),
            mock.patch("codex_discord_bot.asyncio.sleep", fake_sleep),
        ):
            log_text = await self._run_with_log(SESSION_MIRROR_LOOP(client))

        self.assertEqual(calls, ["to_thread", "to_thread"])
        self.assertEqual(sleeps, [0.01, 0.01])
        self.assertIn("session_mirror_unexpected_error", log_text)
        self.assertIn("BadSessionMirrorTargetDependencyError: bad session mirror target dependency", log_text)


if __name__ == "__main__":
    _ = unittest.main()
