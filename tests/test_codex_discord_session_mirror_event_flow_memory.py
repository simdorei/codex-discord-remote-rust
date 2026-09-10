from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
import tempfile
import unittest

import codex_discord_session_mirror_event_flow as event_flow

Event = dict[str, str]
Item = dict[str, str]


@dataclass(frozen=True, slots=True)
class FakeThread:
    rollout_path: str


class SessionMirrorEventFlowMemoryTests(unittest.IsolatedAsyncioTestCase):
    async def test_context_usage_memory_error_tails_only_and_skips_future_scan(self) -> None:
        calls = 0
        logs: list[str] = []
        reads: list[tuple[Path, int, int | None]] = []

        async def choose_thread(codex_thread_id: str) -> FakeThread:
            self.assertEqual(codex_thread_id, "thread-1")
            return thread

        async def get_context_usage(codex_thread: FakeThread) -> str:
            nonlocal calls
            self.assertEqual(codex_thread, thread)
            calls += 1
            raise MemoryError

        def should_recommend_archive(codex_thread: FakeThread, context_usage: str) -> bool:
            raise AssertionError(f"archive recommendation should not run: {codex_thread} {context_usage}")

        async def update_cursor(codex_thread_id: str, rollout_path: str, cursor: int) -> None:
            self.assertEqual((codex_thread_id, rollout_path, cursor), ("thread-1", str(session_path), 42))

        async def get_cursor(codex_thread_id: str, rollout_path: str, initial_cursor: int) -> int:
            self.assertEqual((codex_thread_id, rollout_path, initial_cursor), ("thread-1", str(session_path), 3))
            return initial_cursor

        async def read_events(path: Path, cursor: int, max_events: int | None) -> tuple[list[Event], int]:
            reads.append((path, cursor, max_events))
            return [{"type": "response"}], 42

        def collect_items(
            codex_thread_id: str,
            events: list[Event],
            *,
            seen_agent_messages: dict[str, float],
            seen_user_messages: dict[str, float],
        ) -> list[Item]:
            self.assertEqual((codex_thread_id, events), ("thread-1", [{"type": "response"}]))
            self.assertEqual(seen_agent_messages, {})
            self.assertEqual(seen_user_messages, {})
            return [{"kind": "final", "text": "done", "digest": "digest-1"}]

        with tempfile.TemporaryDirectory() as temp_dir:
            session_path = Path(temp_dir) / "session.jsonl"
            _ = session_path.write_text("abc", encoding="utf-8")
            thread = FakeThread(rollout_path=str(session_path))
            deps = event_flow.SessionMirrorEventFlowDeps(
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
                get_seen_agent_messages=lambda codex_thread_id: {},
                get_seen_user_messages=lambda codex_thread_id: {},
                log=logs.append,
            )

            first = await event_flow.prepare_session_mirror_delivery_items("thread-1", deps=deps)
            second = await event_flow.prepare_session_mirror_delivery_items("thread-1", deps=deps)

        self.assertIsNotNone(first)
        self.assertIsNotNone(second)
        self.assertEqual(calls, 1)
        self.assertEqual(reads, [(session_path, 3, 50), (session_path, 3, 50)])
        self.assertEqual(
            logs,
            [
                "session_mirror_context_usage_failed target=thread-1 error_type=MemoryError action=archive_tail_only",
                "session_mirror_archive_tail_only target=thread-1 reason=archive_recommended",
                "session_mirror_archive_backlog_batch target=thread-1 events=1 max_events=50",
                "session_mirror_archive_backlog_batch target=thread-1 events=1 max_events=50",
            ],
        )
