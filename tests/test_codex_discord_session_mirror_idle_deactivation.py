from __future__ import annotations

import tempfile
import unittest
from collections.abc import Callable, Mapping
from pathlib import Path
from typing import final

from codex_app_server_transport_goal import GoalAbsent, GoalPresent, GoalTransportError, ThreadGoalLookup, ThreadGoalStatus
import codex_discord_session_mirror as session_mirror
import codex_discord_session_mirror_item_sender as item_sender
import codex_discord_session_mirror_target as session_mirror_target
from codex_session_events import JsonEvent


@final
class _Thread:
    def __init__(self, rollout_path: str) -> None:
        self.rollout_path: str = rollout_path


class SessionMirrorIdleDeactivationTests(unittest.IsolatedAsyncioTestCase):
    async def test_active_native_turn_keeps_output_target_alive(self) -> None:
        deactivated, logs = await self._run_idle_cycle(
            get_active_turn_id=lambda thread_id: "turn-2",
            get_goal_lookup=lambda thread_id: GoalAbsent(),
        )

        self.assertEqual(deactivated, [])
        self.assertNotIn("session_mirror_output_deactivated_idle", "\n".join(logs))

    async def test_active_goal_keeps_output_target_alive_without_active_turn(self) -> None:
        deactivated, _logs = await self._run_idle_cycle(
            get_active_turn_id=lambda thread_id: None,
            get_goal_lookup=lambda thread_id: GoalPresent(ThreadGoalStatus.ACTIVE),
        )

        self.assertEqual(deactivated, [])

    async def test_blocked_goal_without_active_turn_deactivates_output_target(self) -> None:
        deactivated, logs = await self._run_idle_cycle(
            get_active_turn_id=lambda thread_id: None,
            get_goal_lookup=lambda thread_id: GoalPresent(ThreadGoalStatus.BLOCKED),
        )

        self.assertEqual(deactivated, ["thread-1"])
        self.assertIn("session_mirror_output_deactivated_idle target=thread-1", logs)

    async def test_goal_lookup_error_keeps_output_target_alive(self) -> None:
        deactivated, logs = await self._run_idle_cycle(
            get_active_turn_id=lambda thread_id: None,
            get_goal_lookup=lambda thread_id: GoalTransportError("resident unavailable"),
        )

        self.assertEqual(deactivated, [])
        self.assertIn("session_mirror_goal_lookup_failed target=thread-1", logs)

    async def test_absent_goal_and_no_active_turn_deactivates_output_target(self) -> None:
        deactivated, logs = await self._run_idle_cycle(
            get_active_turn_id=lambda thread_id: None,
            get_goal_lookup=lambda thread_id: GoalAbsent(),
        )

        self.assertEqual(deactivated, ["thread-1"])
        self.assertIn("session_mirror_output_deactivated_idle target=thread-1", logs)

    async def test_failed_subscription_release_keeps_output_target_for_retry(self) -> None:
        deactivated, logs = await self._run_idle_cycle(
            get_active_turn_id=lambda thread_id: None,
            get_goal_lookup=lambda thread_id: GoalAbsent(),
            release_succeeds=False,
        )

        self.assertEqual(deactivated, [])
        self.assertNotIn("session_mirror_output_deactivated_idle target=thread-1", logs)

    async def _run_idle_cycle(
        self,
        *,
        get_active_turn_id: Callable[[str], str | None],
        get_goal_lookup: Callable[[str], ThreadGoalLookup],
        release_succeeds: bool = True,
    ) -> tuple[list[str], list[str]]:
        deactivated: list[str] = []
        logs: list[str] = []
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            session_path = Path(temp_dir) / "session.jsonl"
            _ = session_path.write_text("", encoding="utf-8")

            async def resolve_channel(discord_thread_id: int) -> None:
                _ = discord_thread_id
                return None

            def collect_items(
                codex_thread_id: str,
                events: list[JsonEvent],
                *,
                seen_agent_messages: dict[str, float],
                seen_user_messages: dict[str, float],
            ) -> list[item_sender.SessionMirrorItem]:
                _ = codex_thread_id, events, seen_agent_messages, seen_user_messages
                return []

            async def send_item(
                channel: object,
                item: Mapping[str, str],
                *,
                target_thread_id: str,
                target_ref: str,
            ) -> None:
                _ = channel, item, target_thread_id, target_ref

            async def release_target(thread_id: str) -> bool:
                if release_succeeds:
                    deactivated.append(thread_id)
                return release_succeeds

            await session_mirror_target.mirror_session_target(
                {"codex_thread_id": "thread-1", "discord_thread_id": 222},
                deps=session_mirror_target.SessionMirrorTargetDeps(
                    parse_session_mirror_target=session_mirror.parse_session_mirror_target,
                    choose_thread=lambda thread_id, cwd: _Thread(str(session_path)),
                    get_thread_context_usage=lambda thread: object(),
                    should_recommend_archive=lambda thread, usage: False,
                    get_thread_rollout_path=lambda thread: thread.rollout_path,
                    is_active_output_target=lambda thread_id: True,
                    archive_skip_logged=set(),
                    is_pending_cursor_target=lambda thread_id: False,
                    clear_pending_cursor_target=lambda thread_id: None,
                    update_session_mirror_cursor=lambda thread_id, rollout_path, cursor: None,
                    get_or_init_session_mirror_cursor=lambda thread_id, rollout_path, initial_cursor: 0,
                    read_new_session_events=lambda session_path, cursor, max_events=None: ([], cursor),
                    get_archive_backlog_max_events=lambda: 10,
                    collect_session_mirror_items=collect_items,
                    get_seen_agent_messages=lambda thread_id: {},
                    get_seen_user_messages=lambda thread_id: {},
                    resolve_session_mirror_channel=resolve_channel,
                    resolve_target_ref=lambda thread_id: (thread_id, thread_id),
                    has_session_mirror_event=lambda digest, thread_id: False,
                    send_session_mirror_item=send_item,
                    claim_session_mirror_event=lambda digest, thread_id: True,
                    release_session_mirror_output_target=release_target,
                    is_thread_busy=lambda path: False,
                    get_active_turn_id=get_active_turn_id,
                    get_thread_goal_lookup=get_goal_lookup,
                    log=logs.append,
                ),
            )
        return deactivated, logs


if __name__ == "__main__":
    _ = unittest.main()
