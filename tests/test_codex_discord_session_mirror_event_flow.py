from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from pathlib import Path
import tempfile
import unittest

import codex_discord_session_mirror_event_flow as event_flow
import codex_discord_session_mirror_cursor as mirror_cursor

Event = dict[str, str]
Item = dict[str, str]


@dataclass(frozen=True, slots=True)
class FakeThread:
    rollout_path: str


class SessionMirrorEventFlowTests(unittest.IsolatedAsyncioTestCase):
    async def test_prepare_session_mirror_delivery_items_returns_batch(self) -> None:
        logs: list[str] = []
        cursor_gets: list[tuple[str, str, int]] = []
        reads: list[tuple[Path, int, int | None]] = []
        collector_calls: list[tuple[str, list[Event], dict[str, float], dict[str, float]]] = []
        seen_agent_messages: dict[str, float] = {"agent": 1.0}
        seen_user_messages: dict[str, float] = {"user": 2.0}
        events: list[Event] = [{"type": "response"}]
        items: list[Item] = [{"kind": "final", "text": "done", "digest": "digest-1"}]

        with tempfile.TemporaryDirectory() as temp_dir:
            session_path = Path(temp_dir) / "session.jsonl"
            _ = session_path.write_text("abc", encoding="utf-8")
            thread = FakeThread(rollout_path=str(session_path))

            async def choose_thread(codex_thread_id: str) -> FakeThread:
                self.assertEqual(codex_thread_id, "thread-1")
                return thread

            async def get_context_usage(codex_thread: FakeThread) -> str:
                self.assertEqual(codex_thread, thread)
                return "usage"

            def should_recommend_archive(codex_thread: FakeThread, context_usage: str) -> bool:
                self.assertEqual((codex_thread, context_usage), (thread, "usage"))
                return True

            async def get_cursor(codex_thread_id: str, rollout_path: str, initial_cursor: int) -> int:
                cursor_gets.append((codex_thread_id, rollout_path, initial_cursor))
                return 1

            async def update_cursor(codex_thread_id: str, rollout_path: str, cursor: int) -> None:
                raise AssertionError(f"cursor should not update yet: {codex_thread_id} {rollout_path} {cursor}")

            async def read_events(path: Path, cursor: int, max_events: int | None) -> tuple[list[Event], int]:
                reads.append((path, cursor, max_events))
                return events, 42

            def collect_items(
                codex_thread_id: str,
                events: list[Event],
                *,
                seen_agent_messages: dict[str, float],
                seen_user_messages: dict[str, float],
            ) -> list[Item]:
                collector_calls.append((codex_thread_id, events, seen_agent_messages, seen_user_messages))
                return items

            prepared = await event_flow.prepare_session_mirror_delivery_items(
                "thread-1",
                deps=event_flow.SessionMirrorEventFlowDeps(
                    choose_thread=choose_thread,
                    get_thread_context_usage=get_context_usage,
                    should_recommend_archive=should_recommend_archive,
                    get_thread_rollout_path=lambda codex_thread: codex_thread.rollout_path,
                    is_active_output_target=lambda codex_thread_id: False,
                    archive_skip_logged=set(),
                    is_pending_cursor_target=lambda codex_thread_id: False,
                    clear_pending_cursor_target=lambda codex_thread_id: None,
                    update_session_mirror_cursor=update_cursor,
                    get_or_init_session_mirror_cursor=get_cursor,
                    read_events=read_events,
                    get_archive_backlog_max_events=lambda: 50,
                    collect_session_mirror_items=collect_items,
                    get_seen_agent_messages=lambda codex_thread_id: seen_agent_messages,
                    get_seen_user_messages=lambda codex_thread_id: seen_user_messages,
                    log=logs.append,
                ),
            )

        self.assertEqual(
            prepared,
            event_flow.SessionMirrorPreparedItems(
                rollout_path=str(session_path),
                events=events,
                items=items,
                next_cursor=42,
            ),
        )
        self.assertEqual(cursor_gets, [("thread-1", str(session_path), 3)])
        self.assertEqual(reads, [(session_path, 1, 50)])
        self.assertEqual(collector_calls, [("thread-1", events, seen_agent_messages, seen_user_messages)])
        self.assertEqual(
            logs,
            [
                "session_mirror_archive_tail_only target=thread-1 reason=archive_recommended",
                "session_mirror_archive_backlog_batch target=thread-1 events=1 max_events=50",
            ],
        )

    async def test_unavailable_thread_logs_and_returns_none(self) -> None:
        logs: list[str] = []

        async def choose_thread(codex_thread_id: str) -> FakeThread:
            self.assertEqual(codex_thread_id, "thread-1")
            raise RuntimeError("missing")

        prepared = await event_flow.prepare_session_mirror_delivery_items(
            "thread-1",
            deps=_edge_deps(choose_thread=choose_thread, logs=logs),
        )

        self.assertIsNone(prepared)
        self.assertEqual(
            logs,
            ["session_mirror_thread_unavailable target=thread-1 error_type=RuntimeError"],
        )

    async def test_missing_session_path_returns_before_cursor_or_read(self) -> None:
        calls: list[str] = []
        missing_path = Path("missing-session.jsonl")
        thread = FakeThread(rollout_path=str(missing_path))

        async def choose_thread(codex_thread_id: str) -> FakeThread:
            self.assertEqual(codex_thread_id, "thread-1")
            return thread

        async def update_cursor(codex_thread_id: str, rollout_path: str, cursor: int) -> None:
            calls.append(f"update:{codex_thread_id}:{rollout_path}:{cursor}")

        async def get_cursor(codex_thread_id: str, rollout_path: str, initial_cursor: int) -> int:
            calls.append(f"get:{codex_thread_id}:{rollout_path}:{initial_cursor}")
            return 0

        prepared = await event_flow.prepare_session_mirror_delivery_items(
            "thread-1",
            deps=_edge_deps(
                choose_thread=choose_thread,
                update_session_mirror_cursor=update_cursor,
                get_or_init_session_mirror_cursor=get_cursor,
            ),
        )

        self.assertIsNone(prepared)
        self.assertEqual(calls, [])

    async def test_empty_events_return_without_collection(self) -> None:
        reads: list[tuple[Path, int, int | None]] = []
        with tempfile.TemporaryDirectory() as temp_dir:
            session_path = Path(temp_dir) / "session.jsonl"
            _ = session_path.write_text("abc", encoding="utf-8")

            async def read_events(path: Path, cursor: int, max_events: int | None) -> tuple[list[Event], int]:
                reads.append((path, cursor, max_events))
                return [], 42

            prepared = await event_flow.prepare_session_mirror_delivery_items(
                "thread-1",
                deps=_edge_deps(session_path=session_path, read_events=read_events),
            )

        self.assertIsNone(prepared)
        self.assertEqual(reads, [(session_path, 3, None)])

    async def test_empty_items_commit_cursor_and_return_none(self) -> None:
        updates: list[tuple[str, str, int]] = []
        with tempfile.TemporaryDirectory() as temp_dir:
            session_path = Path(temp_dir) / "session.jsonl"
            _ = session_path.write_text("abc", encoding="utf-8")

            async def update_cursor(codex_thread_id: str, rollout_path: str, cursor: int) -> None:
                updates.append((codex_thread_id, rollout_path, cursor))

            prepared = await event_flow.prepare_session_mirror_delivery_items(
                "thread-1",
                deps=_edge_deps(
                    session_path=session_path,
                    update_session_mirror_cursor=update_cursor,
                    collected_items=[],
                ),
            )

        self.assertIsNone(prepared)
        self.assertEqual(updates, [("thread-1", str(session_path), 42)])


def _edge_deps(
    *,
    choose_thread: Callable[[str], Awaitable[FakeThread]] | None = None,
    session_path: Path | None = None,
    read_events: Callable[[Path, int, int | None], Awaitable[tuple[list[Event], int]]] | None = None,
    update_session_mirror_cursor: mirror_cursor.SessionMirrorCursorUpdater | None = None,
    get_or_init_session_mirror_cursor: mirror_cursor.SessionMirrorCursorGetter | None = None,
    collected_items: list[Item] | None = None,
    logs: list[str] | None = None,
) -> event_flow.SessionMirrorEventFlowDeps[FakeThread, str, Event, Item]:
    resolved_path = session_path or Path("missing-session.jsonl")
    thread = FakeThread(rollout_path=str(resolved_path))
    events: list[Event] = [{"type": "response"}]
    items = [{"kind": "final", "text": "done", "digest": "digest-1"}] if collected_items is None else collected_items
    log_lines = [] if logs is None else logs

    async def default_choose_thread(codex_thread_id: str) -> FakeThread:
        self_test_thread_id = "thread-1"
        if codex_thread_id != self_test_thread_id:
            raise AssertionError(f"unexpected thread: {codex_thread_id}")
        return thread

    async def get_context_usage(codex_thread: FakeThread) -> str:
        _ = codex_thread
        return "usage"

    async def default_get_cursor(codex_thread_id: str, rollout_path: str, initial_cursor: int) -> int:
        _ = (codex_thread_id, rollout_path)
        return initial_cursor

    async def default_update_cursor(codex_thread_id: str, rollout_path: str, cursor: int) -> None:
        _ = (codex_thread_id, rollout_path, cursor)

    async def default_read_events(path: Path, cursor: int, max_events: int | None) -> tuple[list[Event], int]:
        _ = (path, cursor, max_events)
        return events, 42

    def collect_items(
        codex_thread_id: str,
        events: list[Event],
        *,
        seen_agent_messages: dict[str, float],
        seen_user_messages: dict[str, float],
    ) -> list[Item]:
        _ = (codex_thread_id, events, seen_agent_messages, seen_user_messages)
        return items

    return event_flow.SessionMirrorEventFlowDeps(
        choose_thread=default_choose_thread if choose_thread is None else choose_thread,
        get_thread_context_usage=get_context_usage,
        should_recommend_archive=lambda codex_thread, context_usage: False,
        get_thread_rollout_path=lambda codex_thread: codex_thread.rollout_path,
        is_active_output_target=lambda codex_thread_id: False,
        archive_skip_logged=set(),
        is_pending_cursor_target=lambda codex_thread_id: False,
        clear_pending_cursor_target=lambda codex_thread_id: None,
        update_session_mirror_cursor=default_update_cursor
        if update_session_mirror_cursor is None
        else update_session_mirror_cursor,
        get_or_init_session_mirror_cursor=default_get_cursor
        if get_or_init_session_mirror_cursor is None
        else get_or_init_session_mirror_cursor,
        read_events=default_read_events if read_events is None else read_events,
        get_archive_backlog_max_events=lambda: 50,
        collect_session_mirror_items=collect_items,
        get_seen_agent_messages=lambda codex_thread_id: {},
        get_seen_user_messages=lambda codex_thread_id: {},
        log=log_lines.append,
    )
