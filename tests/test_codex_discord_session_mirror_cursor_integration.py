from __future__ import annotations

import os
import sqlite3
import tempfile
import time
import unittest
from pathlib import Path
from types import SimpleNamespace
from typing import cast, final, override
from unittest import mock

import codex_desktop_bridge as bridge
import codex_discord_bot as bot


class ThreadUnavailableError(RuntimeError):
    pass


@final
class SessionMirrorCursorIntegrationTests(unittest.TestCase):
    @override
    def __init__(self, methodName: str = "runTest") -> None:
        super().__init__(methodName)
        self._old_mirror_db_path = bot.MIRROR_DB_PATH
        self._old_discord_log_path: str | None = None
        self._old_active_targets: dict[str, float] = {}
        self._old_pending_targets: set[str] = set()
        self._mirror_db_temp_dir: tempfile.TemporaryDirectory[str] | None = None

    @override
    def setUp(self) -> None:
        self._old_mirror_db_path = bot.MIRROR_DB_PATH
        self._old_discord_log_path = os.environ.get("CODEX_DISCORD_LOG_PATH")
        self._old_active_targets = dict(bot.get_session_mirror_state().active_output_targets)
        self._old_pending_targets = set(bot.get_session_mirror_state().pending_cursor_targets)
        mirror_db_temp_dir = tempfile.TemporaryDirectory(ignore_cleanup_errors=True)
        self._mirror_db_temp_dir = mirror_db_temp_dir

        bot.MIRROR_DB_PATH = Path(mirror_db_temp_dir.name) / "mirror.sqlite"
        os.environ["CODEX_DISCORD_LOG_PATH"] = str(Path(mirror_db_temp_dir.name) / "test.log")
        bot.get_session_mirror_state().active_output_targets.clear()
        bot.get_session_mirror_state().pending_cursor_targets.clear()
        bot.init_mirror_db()

    @override
    def tearDown(self) -> None:
        bot.MIRROR_DB_PATH = self._old_mirror_db_path
        if self._old_discord_log_path is None:
            _ = os.environ.pop("CODEX_DISCORD_LOG_PATH", None)
        else:
            os.environ["CODEX_DISCORD_LOG_PATH"] = self._old_discord_log_path
        bot.get_session_mirror_state().active_output_targets.clear()
        bot.get_session_mirror_state().active_output_targets.update(self._old_active_targets)
        bot.get_session_mirror_state().pending_cursor_targets.clear()
        bot.get_session_mirror_state().pending_cursor_targets.update(self._old_pending_targets)
        mirror_db_temp_dir = self._mirror_db_temp_dir
        if mirror_db_temp_dir is not None:
            mirror_db_temp_dir.cleanup()

    def test_prime_session_mirror_cursor_advances_existing_cursor_to_session_end(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            session_path = Path(temp_dir) / "session.jsonl"
            _ = session_path.write_text("older\nlatest\n", encoding="utf-8")
            current_cursor = session_path.stat().st_size
            get_calls: list[tuple[str, str, int]] = []
            updates: list[tuple[str, str, int]] = []

            def fake_get_cursor(codex_thread_id: str, rollout_path: str, initial_cursor: int) -> int:
                get_calls.append((codex_thread_id, rollout_path, initial_cursor))
                return 3

            def fake_update_cursor(codex_thread_id: str, rollout_path: str, cursor: int) -> None:
                updates.append((codex_thread_id, rollout_path, cursor))

            with (
                mock.patch.object(bridge, "choose_thread", return_value=SimpleNamespace(rollout_path=str(session_path))),
                mock.patch.object(bot, "get_or_init_session_mirror_cursor", side_effect=fake_get_cursor),
                mock.patch.object(bot, "update_session_mirror_cursor", side_effect=fake_update_cursor),
                mock.patch.dict(os.environ, {"DISCORD_SESSION_MIRROR": "1"}),
            ):
                result = bot.prime_session_mirror_cursor_for_target("thread-1")

        self.assertEqual(result, current_cursor)
        self.assertEqual(get_calls, [("thread-1", str(session_path), current_cursor)])
        self.assertEqual(updates, [("thread-1", str(session_path), current_cursor)])

    def test_prime_session_mirror_cursor_preserves_active_output_cursor(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            session_path = Path(temp_dir) / "session.jsonl"
            _ = session_path.write_text("older\npending answer\n", encoding="utf-8")
            current_cursor = session_path.stat().st_size
            updates: list[tuple[str, str, int]] = []

            def fake_update_cursor(codex_thread_id: str, rollout_path: str, cursor: int) -> None:
                updates.append((codex_thread_id, rollout_path, cursor))

            bot.activate_session_mirror_output_target("thread-1")
            with (
                mock.patch.object(bridge, "choose_thread", return_value=SimpleNamespace(rollout_path=str(session_path))),
                mock.patch.object(bot, "get_or_init_session_mirror_cursor", return_value=6),
                mock.patch.object(bot, "update_session_mirror_cursor", side_effect=fake_update_cursor),
                mock.patch.dict(os.environ, {"DISCORD_SESSION_MIRROR": "1"}),
            ):
                result = bot.prime_session_mirror_cursor_for_target("thread-1")

        self.assertIsNotNone(result)
        assert result is not None
        self.assertLess(result, current_cursor)
        self.assertEqual(result, 6)
        self.assertEqual(updates, [])

    def test_prime_session_mirror_cursor_preserves_recent_cursor(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            session_path = Path(temp_dir) / "session.jsonl"
            _ = session_path.write_text("older\npending answer\n", encoding="utf-8")
            current_cursor = session_path.stat().st_size
            rollout_path = str(session_path)
            with sqlite3.connect(bot.MIRROR_DB_PATH) as conn:
                _ = conn.execute(
                    """
                    INSERT OR REPLACE INTO codex_session_mirror_offsets (
                        codex_thread_id, rollout_path, cursor, updated_at
                    )
                    VALUES (?, ?, ?, ?)
                    """,
                    ("thread-1", rollout_path, 6, time.time()),
                )

            with (
                mock.patch.object(bridge, "choose_thread", return_value=SimpleNamespace(rollout_path=rollout_path)),
                mock.patch.dict(os.environ, {"DISCORD_SESSION_MIRROR": "1"}),
            ):
                result = bot.prime_session_mirror_cursor_for_target("thread-1")

            with sqlite3.connect(bot.MIRROR_DB_PATH) as conn:
                row = cast(
                    tuple[int] | None,
                    conn.execute(
                        "SELECT cursor FROM codex_session_mirror_offsets WHERE codex_thread_id = ?",
                        ("thread-1",),
                    ).fetchone(),
                )

        self.assertIsNotNone(result)
        assert result is not None
        self.assertLess(result, current_cursor)
        self.assertEqual(result, 6)
        self.assertEqual(row, (6,))

    def test_prime_session_mirror_cursor_returns_none_when_thread_unavailable(self) -> None:
        def raise_unavailable(thread_id: str, cwd: str | None = None) -> SimpleNamespace:
            _ = thread_id, cwd
            raise ThreadUnavailableError("thread unavailable")

        with (
            mock.patch.object(bridge, "choose_thread", side_effect=raise_unavailable),
            mock.patch.dict(os.environ, {"DISCORD_SESSION_MIRROR": "1"}),
        ):
            result = bot.prime_session_mirror_cursor_for_target("thread-1")

        log_text = Path(os.environ["CODEX_DISCORD_LOG_PATH"]).read_text(encoding="utf-8")
        self.assertIsNone(result)
        self.assertIn("session_mirror_cursor_prime_failed target=thread-1", log_text)
        self.assertIn("ThreadUnavailableError: thread unavailable", log_text)

    def test_session_mirror_rollout_path_missing_returns_false_when_thread_unavailable(self) -> None:
        def raise_unavailable(thread_id: str, cwd: str | None = None) -> SimpleNamespace:
            _ = thread_id, cwd
            raise ThreadUnavailableError("thread unavailable")

        with mock.patch.object(bridge, "choose_thread", side_effect=raise_unavailable):
            result = bot.session_mirror_rollout_path_missing("thread-1")

        log_text = Path(os.environ["CODEX_DISCORD_LOG_PATH"]).read_text(encoding="utf-8")
        self.assertFalse(result)
        self.assertIn(
            "session_mirror_output_prepare_failed target=thread-1 "
            + "reason=thread_unavailable error_type=ThreadUnavailableError",
            log_text,
        )


if __name__ == "__main__":
    _ = unittest.main()
