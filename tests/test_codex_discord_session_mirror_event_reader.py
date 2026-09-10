from __future__ import annotations

import unittest
from pathlib import Path

import codex_discord_session_mirror_event_reader as event_reader


class SessionMirrorEventReaderTests(unittest.IsolatedAsyncioTestCase):
    async def test_archive_tail_read_uses_max_events_and_logs_nonempty_batch(self) -> None:
        calls: list[tuple[Path, int, int | None]] = []
        logs: list[str] = []

        async def read_events(session_path: Path, cursor: int, max_events: int | None) -> tuple[list[str], int]:
            calls.append((session_path, cursor, max_events))
            return ["one", "two"], 99

        deps: event_reader.SessionMirrorEventReaderDeps[str] = event_reader.SessionMirrorEventReaderDeps(
            read_events=read_events,
            get_archive_backlog_max_events=lambda: 200,
            log=logs.append,
        )
        result = await event_reader.read_session_mirror_events(
            "thread-1",
            Path("session.jsonl"),
            12,
            archive_tail_only=True,
            deps=deps,
        )

        self.assertEqual(result.events, ["one", "two"])
        self.assertEqual(result.next_cursor, 99)
        self.assertEqual(calls, [(Path("session.jsonl"), 12, 200)])
        self.assertEqual(
            logs,
            ["session_mirror_archive_backlog_batch target=thread-1 events=2 max_events=200"],
        )

    async def test_normal_read_uses_no_max_events_and_no_archive_log(self) -> None:
        calls: list[tuple[Path, int, int | None]] = []
        logs: list[str] = []

        async def read_events(session_path: Path, cursor: int, max_events: int | None) -> tuple[list[str], int]:
            calls.append((session_path, cursor, max_events))
            return ["one"], 42

        deps: event_reader.SessionMirrorEventReaderDeps[str] = event_reader.SessionMirrorEventReaderDeps(
            read_events=read_events,
            get_archive_backlog_max_events=lambda: 200,
            log=logs.append,
        )
        result = await event_reader.read_session_mirror_events(
            "thread-1",
            Path("session.jsonl"),
            12,
            archive_tail_only=False,
            deps=deps,
        )

        self.assertEqual(result.events, ["one"])
        self.assertEqual(result.next_cursor, 42)
        self.assertEqual(calls, [(Path("session.jsonl"), 12, None)])
        self.assertEqual(logs, [])

    async def test_archive_tail_unlimited_logs_unlimited_for_nonempty_batch(self) -> None:
        logs: list[str] = []

        async def read_events(session_path: Path, cursor: int, max_events: int | None) -> tuple[list[str], int]:
            self.assertEqual((session_path, cursor, max_events), (Path("session.jsonl"), 12, None))
            return ["one"], 42

        deps: event_reader.SessionMirrorEventReaderDeps[str] = event_reader.SessionMirrorEventReaderDeps(
            read_events=read_events,
            get_archive_backlog_max_events=lambda: 0,
            log=logs.append,
        )
        _ = await event_reader.read_session_mirror_events(
            "thread-1",
            Path("session.jsonl"),
            12,
            archive_tail_only=True,
            deps=deps,
        )

        self.assertEqual(
            logs,
            ["session_mirror_archive_backlog_batch target=thread-1 events=1 max_events=unlimited"],
        )

    async def test_archive_tail_empty_batch_does_not_log(self) -> None:
        logs: list[str] = []

        async def read_events(session_path: Path, cursor: int, max_events: int | None) -> tuple[list[str], int]:
            self.assertEqual((session_path, cursor, max_events), (Path("session.jsonl"), 12, 200))
            return [], 12

        deps: event_reader.SessionMirrorEventReaderDeps[str] = event_reader.SessionMirrorEventReaderDeps(
            read_events=read_events,
            get_archive_backlog_max_events=lambda: 200,
            log=logs.append,
        )
        result = await event_reader.read_session_mirror_events(
            "thread-1",
            Path("session.jsonl"),
            12,
            archive_tail_only=True,
            deps=deps,
        )

        self.assertEqual(result.events, [])
        self.assertEqual(result.next_cursor, 12)
        self.assertEqual(logs, [])
