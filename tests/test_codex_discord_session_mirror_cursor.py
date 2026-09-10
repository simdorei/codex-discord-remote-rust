from __future__ import annotations

import unittest

import codex_discord_session_mirror_cursor as cursor_init


class SessionMirrorCursorInitializationTests(unittest.IsolatedAsyncioTestCase):
    async def test_active_pending_cursor_persists_zero_and_clears_marker(self) -> None:
        updates: list[tuple[str, str, int]] = []
        get_calls: list[tuple[str, str, int]] = []
        cleared: list[str] = []
        logs: list[str] = []

        async def update_cursor(codex_thread_id: str, rollout_path: str, cursor: int) -> None:
            updates.append((codex_thread_id, rollout_path, cursor))

        async def get_or_init_cursor(codex_thread_id: str, rollout_path: str, initial_cursor: int) -> int:
            get_calls.append((codex_thread_id, rollout_path, initial_cursor))
            return 999

        cursor = await cursor_init.initialize_session_mirror_cursor(
            "thread-1",
            "session.jsonl",
            session_size=123,
            active_output_target=True,
            deps=cursor_init.SessionMirrorCursorInitDeps(
                is_pending_cursor_target=lambda target: target == "thread-1",
                clear_pending_cursor_target=cleared.append,
                update_session_mirror_cursor=update_cursor,
                get_or_init_session_mirror_cursor=get_or_init_cursor,
                log=logs.append,
            ),
        )

        self.assertEqual(cursor, 0)
        self.assertEqual(updates, [("thread-1", "session.jsonl", 0)])
        self.assertEqual(get_calls, [])
        self.assertEqual(cleared, ["thread-1"])
        self.assertEqual(logs, ["session_mirror_pending_cursor_initialized target=thread-1 cursor=0"])

    async def test_pending_without_active_uses_session_size_and_clears_marker(self) -> None:
        updates: list[tuple[str, str, int]] = []
        get_calls: list[tuple[str, str, int]] = []
        cleared: list[str] = []
        logs: list[str] = []

        async def update_cursor(codex_thread_id: str, rollout_path: str, cursor: int) -> None:
            updates.append((codex_thread_id, rollout_path, cursor))

        async def get_or_init_cursor(codex_thread_id: str, rollout_path: str, initial_cursor: int) -> int:
            get_calls.append((codex_thread_id, rollout_path, initial_cursor))
            return 77

        cursor = await cursor_init.initialize_session_mirror_cursor(
            "thread-1",
            "session.jsonl",
            session_size=123,
            active_output_target=False,
            deps=cursor_init.SessionMirrorCursorInitDeps(
                is_pending_cursor_target=lambda target: target == "thread-1",
                clear_pending_cursor_target=cleared.append,
                update_session_mirror_cursor=update_cursor,
                get_or_init_session_mirror_cursor=get_or_init_cursor,
                log=logs.append,
            ),
        )

        self.assertEqual(cursor, 77)
        self.assertEqual(updates, [])
        self.assertEqual(get_calls, [("thread-1", "session.jsonl", 123)])
        self.assertEqual(cleared, ["thread-1"])
        self.assertEqual(logs, ["session_mirror_pending_cursor_initialized target=thread-1 cursor=123"])

    async def test_non_pending_active_output_uses_session_size_without_clear_or_log(self) -> None:
        get_calls: list[tuple[str, str, int]] = []
        cleared: list[str] = []
        logs: list[str] = []

        async def update_cursor(codex_thread_id: str, rollout_path: str, cursor: int) -> None:
            _ = (codex_thread_id, rollout_path, cursor)
            raise AssertionError("non-pending cursor should not update directly")

        async def get_or_init_cursor(codex_thread_id: str, rollout_path: str, initial_cursor: int) -> int:
            get_calls.append((codex_thread_id, rollout_path, initial_cursor))
            return 456

        cursor = await cursor_init.initialize_session_mirror_cursor(
            "thread-1",
            "session.jsonl",
            session_size=123,
            active_output_target=True,
            deps=cursor_init.SessionMirrorCursorInitDeps(
                is_pending_cursor_target=lambda target: False,
                clear_pending_cursor_target=cleared.append,
                update_session_mirror_cursor=update_cursor,
                get_or_init_session_mirror_cursor=get_or_init_cursor,
                log=logs.append,
            ),
        )

        self.assertEqual(cursor, 456)
        self.assertEqual(get_calls, [("thread-1", "session.jsonl", 123)])
        self.assertEqual(cleared, [])
        self.assertEqual(logs, [])
