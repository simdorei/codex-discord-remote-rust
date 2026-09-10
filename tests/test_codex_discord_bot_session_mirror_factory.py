from __future__ import annotations

import asyncio
import unittest
from collections.abc import Sequence
from pathlib import Path
from typing import cast
from unittest import mock

import codex_app_server_transport as app_server_transport
import codex_discord_bot_session_mirror_factory as factory
import codex_discord_bot_session_mirror_runtime as bot_runtime
import codex_discord_session_mirror as session_mirror
import codex_discord_session_mirror_item_delivery as item_delivery
from codex_session_events import JsonEvent


class SessionMirrorSubscriptionReleaseWiringTests(unittest.IsolatedAsyncioTestCase):
    async def test_injected_release_callback_is_used(self) -> None:
        deactivated: list[str] = []
        runtime = _runtime(deactivated)

        result = await runtime.deps.release_session_mirror_output_target("thread-1")

        self.assertTrue(result)
        self.assertEqual(deactivated, ["thread-1"])

    async def test_deferred_release_keeps_output_target_active_for_retry(self) -> None:
        deactivated: list[str] = []
        runtime = _runtime(deactivated, release_result=False)

        result = await runtime.deps.release_session_mirror_output_target("thread-1")

        self.assertFalse(result)
        self.assertEqual(deactivated, [])

    async def test_unhealthy_app_server_does_not_restart_for_background_callbacks(self) -> None:
        deactivated: list[str] = []
        runtime = _runtime(deactivated)

        with (
            mock.patch.object(factory, "env_flag", return_value=True),
            mock.patch.object(
                app_server_transport.DEFAULT_CLIENT,
                "lifecycle_snapshot",
                return_value=app_server_transport.AppServerLifecycleSnapshot(7, False, None),
            ),
            mock.patch.object(
                app_server_transport.DEFAULT_CLIENT,
                "get_active_turn_id_or_raise",
            ) as active_turn,
            mock.patch.object(
                app_server_transport.DEFAULT_CLIENT,
                "get_thread_goal_lookup",
            ) as goal_lookup,
            mock.patch.object(
                app_server_transport.DEFAULT_CLIENT,
                "release_thread_subscription_if_terminal",
            ) as release,
        ):
            with self.assertRaises(app_server_transport.AppServerGenerationMismatch):
                _ = runtime.deps.get_active_turn_id("thread-1")
            goal = runtime.deps.get_thread_goal_lookup("thread-1")

        self.assertIsInstance(goal, app_server_transport.GoalTransportError)
        active_turn.assert_not_called()
        goal_lookup.assert_not_called()
        release.assert_not_called()
        self.assertEqual(deactivated, [])

    async def test_healthy_background_reads_use_the_captured_generation(self) -> None:
        deactivated: list[str] = []
        runtime = _runtime(deactivated)

        with (
            mock.patch.object(factory, "env_flag", return_value=True),
            mock.patch.object(
                app_server_transport.DEFAULT_CLIENT,
                "lifecycle_snapshot",
                return_value=app_server_transport.AppServerLifecycleSnapshot(7, True, 1.0),
            ),
            mock.patch.object(
                app_server_transport.DEFAULT_CLIENT,
                "get_active_turn_id_or_raise",
                return_value=None,
            ) as active_turn,
            mock.patch.object(
                app_server_transport.DEFAULT_CLIENT,
                "get_thread_goal_lookup",
                return_value=app_server_transport.GoalAbsent(),
            ) as goal_lookup,
        ):
            self.assertIsNone(runtime.deps.get_active_turn_id("thread-1"))
            goal = runtime.deps.get_thread_goal_lookup("thread-1")

        self.assertIsInstance(goal, app_server_transport.GoalAbsent)
        active_turn.assert_called_once_with("thread-1", expected_generation=7)
        goal_lookup.assert_called_once_with("thread-1", expected_generation=7)


def _runtime(
    deactivated: list[str],
    *,
    release_result: bool = True,
) -> bot_runtime.SessionMirrorRuntime[object]:
    async def load_targets(
        _db_path: Path,
        _limit: int,
    ) -> Sequence[session_mirror.SessionMirrorTargetMapping]:
        return []

    async def sleep(_seconds: float) -> None:
        return None

    def parse_interactive_notice(_text: str) -> tuple[str, str, item_delivery.InteractiveOptions]:
        return "", "", []

    async def send_interactive_prompt(
        channel: object,
        target_thread_id: str,
        target_ref: str,
        state: str,
        prompt: str,
        options: item_delivery.InteractiveOptions,
    ) -> None:
        _ = channel, target_thread_id, target_ref, state, prompt, options
        return None

    async def send_chunks(channel: object, content: str, *, context: str) -> None:
        _ = channel, content, context
        return None

    async def send_attachment(
        channel: object,
        content: str,
        attachment_url: str,
        filename: str,
        *,
        context: str,
    ) -> None:
        _ = channel, content, attachment_url, filename, context
        return None

    def collect_items(
        codex_thread_id: str,
        events: list[JsonEvent],
        *,
        seen_agent_messages: dict[str, float],
        seen_user_messages: dict[str, float],
    ) -> list[item_delivery.SessionMirrorItem]:
        _ = codex_thread_id, events, seen_agent_messages, seen_user_messages
        return []

    async def send_typing_pulse(_channel: object, _target_thread_id: str, _context: str) -> None:
        return None

    async def release_output_target(thread_id: str) -> bool:
        if release_result:
            deactivated.append(thread_id)
        return release_result

    return factory.make_session_mirror_runtime(
        target_limit=10,
        archive_backlog_max_events_default=100,
        delivery_exceptions=(RuntimeError,),
        fetch_failure_types=(RuntimeError,),
        get_db_path=lambda: Path("mirror.db"),
        load_targets_in_thread=load_targets,
        create_task=asyncio.create_task,
        sleep=sleep,
        is_messageable=lambda _channel: True,
        parse_interactive_notice=parse_interactive_notice,
        send_interactive_prompt=send_interactive_prompt,
        send_chunks=send_chunks,
        send_attachment=send_attachment,
        collect_session_mirror_items=collect_items,
        get_archive_skip_logged=lambda _owner: set(),
        resolve_target_ref=lambda thread_id: (thread_id, thread_id),
        is_active_output_target=lambda _thread_id: True,
        is_pending_cursor_target=lambda _thread_id: False,
        clear_pending_cursor_target=lambda _thread_id: None,
        update_session_mirror_cursor=lambda _thread_id, _path, _cursor: None,
        get_or_init_session_mirror_cursor=lambda _thread_id, _path, cursor: cursor,
        has_session_mirror_event=lambda _digest, _thread_id: False,
        claim_session_mirror_event=lambda _digest, _thread_id: True,
        release_session_mirror_output_target=release_output_target,
        events_bridge=cast(factory.SessionMirrorEventsBridge, mock.Mock()),
        log=lambda _line: None,
        send_typing_pulse=send_typing_pulse,
    )


if __name__ == "__main__":
    _ = unittest.main()
