from __future__ import annotations

import unittest

import codex_discord_session_mirror_items as mirror_items


Event = dict[str, str]
Item = dict[str, str]


class SessionMirrorItemsTests(unittest.IsolatedAsyncioTestCase):
    async def test_nonempty_collection_returns_items_without_cursor_commit(self) -> None:
        updates: list[tuple[str, str, int]] = []
        seen_agent_messages: dict[str, float] = {"agent": 1.0}
        seen_user_messages: dict[str, float] = {"user": 2.0}
        events: list[Event] = [{"type": "response"}]
        expected_items: list[Item] = [{"kind": "final", "text": "done"}]
        collector_calls: list[tuple[str, list[Event], dict[str, float], dict[str, float]]] = []

        def collect_items(
            codex_thread_id: str,
            events: list[Event],
            *,
            seen_agent_messages: dict[str, float],
            seen_user_messages: dict[str, float],
        ) -> list[Item]:
            collector_calls.append((codex_thread_id, events, seen_agent_messages, seen_user_messages))
            return expected_items

        async def update_cursor(codex_thread_id: str, rollout_path: str, cursor: int) -> None:
            updates.append((codex_thread_id, rollout_path, cursor))

        result = await mirror_items.collect_session_mirror_delivery_items(
            "thread-1",
            events,
            "session.jsonl",
            42,
            seen_agent_messages=seen_agent_messages,
            seen_user_messages=seen_user_messages,
            deps=mirror_items.SessionMirrorItemsDeps(
                collect_session_mirror_items=collect_items,
                update_session_mirror_cursor=update_cursor,
            ),
        )

        self.assertEqual(result.items, expected_items)
        self.assertFalse(result.cursor_committed)
        self.assertEqual(updates, [])
        self.assertEqual(
            collector_calls,
            [("thread-1", events, seen_agent_messages, seen_user_messages)],
        )

    async def test_empty_collection_commits_next_cursor_once(self) -> None:
        updates: list[tuple[str, str, int]] = []
        seen_agent_messages: dict[str, float] = {}
        seen_user_messages: dict[str, float] = {}
        events: list[Event] = [{"type": "response"}]

        def collect_items(
            codex_thread_id: str,
            events: list[Event],
            *,
            seen_agent_messages: dict[str, float],
            seen_user_messages: dict[str, float],
        ) -> list[Item]:
            _ = (codex_thread_id, events, seen_agent_messages, seen_user_messages)
            return []

        async def update_cursor(codex_thread_id: str, rollout_path: str, cursor: int) -> None:
            updates.append((codex_thread_id, rollout_path, cursor))

        result = await mirror_items.collect_session_mirror_delivery_items(
            "thread-1",
            events,
            "session.jsonl",
            42,
            seen_agent_messages=seen_agent_messages,
            seen_user_messages=seen_user_messages,
            deps=mirror_items.SessionMirrorItemsDeps(
                collect_session_mirror_items=collect_items,
                update_session_mirror_cursor=update_cursor,
            ),
        )

        self.assertEqual(result.items, [])
        self.assertTrue(result.cursor_committed)
        self.assertEqual(updates, [("thread-1", "session.jsonl", 42)])
