from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol, cast, final, override
import os
import sqlite3
import tempfile
import unittest

import codex_discord_bot as bot
from codex_discord_logging import get_log_path

from tests.test_codex_discord_bot import FakeTarget


INSERT_PROJECT_SQL = "\n".join(
    (
        "INSERT INTO mirror_projects",
        "    (project_key, project_name, discord_channel_id, updated_at)",
        "VALUES (?, ?, ?, ?)",
    )
)
INSERT_THREAD_SQL = "\n".join(
    (
        "INSERT INTO mirror_threads (",
        "    codex_thread_id, project_key, thread_title,",
        "    discord_channel_id, discord_thread_id, updated_at",
        ") VALUES (?, ?, ?, ?, ?, ?)",
    )
)
INSERT_OFFSET_SQL = "\n".join(
    (
        "INSERT INTO codex_session_mirror_offsets",
        "    (codex_thread_id, rollout_path, cursor, updated_at)",
        "VALUES (?, ?, ?, ?)",
    )
)
INSERT_EVENT_SQL = "\n".join(
    (
        "INSERT INTO codex_session_mirror_events",
        "    (event_digest, codex_thread_id, created_at)",
        "VALUES (?, ?, ?)",
    )
)


@dataclass(frozen=True, slots=True)
class ArchiveRows:
    mirror_rows: list[tuple[str]]
    offset_rows: list[tuple[str]]
    project_rows: list[tuple[str]]
    event_rows: list[tuple[str]]


@final
class ArchiveCleanupOwnerFake:
    def __init__(self) -> None:
        self._session_mirror_archive_skip_logged: set[str] = {"thread-1"}
        self._session_mirror_seen_agent_messages: dict[str, dict[str, float]] = {
            "thread-1": {"agent": 1.0}
        }
        self._session_mirror_seen_user_messages: dict[str, dict[str, float]] = {
            "thread-1": {"user": 1.0}
        }

    @property
    def skip_logged(self) -> set[str]:
        return self._session_mirror_archive_skip_logged

    @property
    def seen_agent_messages(self) -> dict[str, dict[str, float]]:
        return self._session_mirror_seen_agent_messages

    @property
    def seen_user_messages(self) -> dict[str, dict[str, float]]:
        return self._session_mirror_seen_user_messages


@final
class EmptyArchiveCleanupOwner:
    pass


class CleanupUnavailable(RuntimeError):
    pass


class RunArchiveBridgeAndSend(Protocol):
    def __call__(
        self,
        target: FakeTarget,
        argv: list[str],
        title: str,
        failure_title: str | None = None,
        archive_cleanup_owner: ArchiveCleanupOwnerFake | EmptyArchiveCleanupOwner | None = None,
    ) -> Awaitable[tuple[int, str]]:
        ...


def run_archive_bridge_and_send() -> RunArchiveBridgeAndSend:
    return cast(RunArchiveBridgeAndSend, bot.run_bridge_and_send)


def bridge_command_result(exit_code: int, output: str) -> Callable[[list[str]], tuple[int, str]]:
    def run_bridge_command(argv: list[str]) -> tuple[int, str]:
        del argv
        return exit_code, output

    return run_bridge_command


def insert_archive_mirror_state(
    *,
    include_project: bool = False,
    include_event: bool = False,
) -> None:
    with sqlite3.connect(bot.MIRROR_DB_PATH) as conn:
        if include_project:
            _ = conn.execute(
                INSERT_PROJECT_SQL,
                ("project", "Project", 111, 1.0),
            )
        _ = conn.execute(
            INSERT_THREAD_SQL,
            ("thread-1", "project", "Archived", 111, 222, 1.0),
        )
        _ = conn.execute(
            INSERT_OFFSET_SQL,
            ("thread-1", "thread-1.jsonl", 42, 1.0),
        )
        if include_event:
            _ = conn.execute(
                INSERT_EVENT_SQL,
                ("digest-1", "thread-1", 1.0),
            )


def fetch_archive_rows() -> ArchiveRows:
    with sqlite3.connect(bot.MIRROR_DB_PATH) as conn:
        return ArchiveRows(
            mirror_rows=cast(
                list[tuple[str]],
                conn.execute("SELECT codex_thread_id FROM mirror_threads").fetchall(),
            ),
            offset_rows=cast(
                list[tuple[str]],
                conn.execute("SELECT codex_thread_id FROM codex_session_mirror_offsets").fetchall(),
            ),
            project_rows=cast(
                list[tuple[str]],
                conn.execute("SELECT project_key FROM mirror_projects").fetchall(),
            ),
            event_rows=cast(
                list[tuple[str]],
                conn.execute("SELECT event_digest FROM codex_session_mirror_events").fetchall(),
            ),
        )


class ArchiveBridgeCleanupTestCase(unittest.IsolatedAsyncioTestCase):
    def __init__(self, methodName: str = "runTest") -> None:
        super().__init__(methodName)
        self._old_mirror_db_path: Path = Path()
        self._old_discord_log_path: str | None = None
        self._old_app_server_transport: str | None = None
        self._old_active_session_mirror_output_targets: dict[str, float] = {}
        self._old_pending_session_mirror_cursor_targets: set[str] = set()
        self._mirror_db_temp_dir: tempfile.TemporaryDirectory[str] | None = None

    @override
    def setUp(self) -> None:
        self._old_mirror_db_path = bot.MIRROR_DB_PATH
        self._old_discord_log_path = os.environ.get("CODEX_DISCORD_LOG_PATH")
        self._old_app_server_transport = os.environ.get("CODEX_DISCORD_APP_SERVER_TRANSPORT")
        self._old_active_session_mirror_output_targets = dict(
            bot.get_session_mirror_state().active_output_targets
        )
        self._old_pending_session_mirror_cursor_targets = set(
            bot.get_session_mirror_state().pending_cursor_targets
        )
        bot.get_session_mirror_state().active_output_targets.clear()
        bot.get_session_mirror_state().pending_cursor_targets.clear()
        bot.ACTIVE_DISCORD_DELIVERIES.clear()
        bot.clear_discord_delivery_stopping()
        self._mirror_db_temp_dir = tempfile.TemporaryDirectory(ignore_cleanup_errors=True)
        bot.MIRROR_DB_PATH = Path(self._mirror_db_temp_dir.name) / "mirror.sqlite"
        os.environ["CODEX_DISCORD_LOG_PATH"] = str(
            Path(self._mirror_db_temp_dir.name) / "test_discord_bot.log"
        )
        os.environ["CODEX_DISCORD_APP_SERVER_TRANSPORT"] = "0"
        bot.init_mirror_db()

    @override
    def tearDown(self) -> None:
        bot.MIRROR_DB_PATH = self._old_mirror_db_path
        if self._old_discord_log_path is None:
            _ = os.environ.pop("CODEX_DISCORD_LOG_PATH", None)
        else:
            os.environ["CODEX_DISCORD_LOG_PATH"] = self._old_discord_log_path
        if self._old_app_server_transport is None:
            _ = os.environ.pop("CODEX_DISCORD_APP_SERVER_TRANSPORT", None)
        else:
            os.environ["CODEX_DISCORD_APP_SERVER_TRANSPORT"] = self._old_app_server_transport
        bot.get_session_mirror_state().active_output_targets.clear()
        bot.get_session_mirror_state().active_output_targets.update(
            self._old_active_session_mirror_output_targets
        )
        bot.get_session_mirror_state().pending_cursor_targets.clear()
        bot.get_session_mirror_state().pending_cursor_targets.update(
            self._old_pending_session_mirror_cursor_targets
        )
        bot.ACTIVE_DISCORD_DELIVERIES.clear()
        bot.clear_discord_delivery_stopping()
        if self._mirror_db_temp_dir is not None:
            self._mirror_db_temp_dir.cleanup()

    def log_text(self) -> str:
        return get_log_path().read_text(encoding="utf-8")
