from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from typing import cast, final
from unittest import mock

import codex_discord_session_mirror as session_mirror
import codex_discord_session_mirror_target as session_mirror_target
import codex_pro_session_mirror_gate as pro_session_mirror_gate


@final
class FakeThread:
    def __init__(self, rollout_path: str) -> None:
        self.rollout_path = rollout_path


class ProSessionMirrorOutputTests(unittest.IsolatedAsyncioTestCase):
    def tearDown(self) -> None:
        pro_session_mirror_gate.reset_for_tests()

    async def test_pro_output_hold_returns_before_reading_session_events(self) -> None:
        deps = mock.Mock()
        deps.parse_session_mirror_target = session_mirror.parse_session_mirror_target
        pro_session_mirror_gate.hold("thread-1")

        with mock.patch.object(
            session_mirror_target.event_flow,
            "prepare_session_mirror_delivery_items",
            new_callable=mock.AsyncMock,
        ) as prepare_items:
            await session_mirror_target.mirror_session_target(
                {"codex_thread_id": "thread-1", "discord_thread_id": 222},
                deps=cast(
                    session_mirror_target.SessionMirrorTargetDeps[
                        object,
                        object,
                        object,
                        object,
                    ],
                    deps,
                ),
            )

        prepare_items.assert_not_awaited()

    async def test_rejected_pro_output_advances_cursor_without_delivery(self) -> None:
        updates: list[tuple[str, str, int]] = []
        logs: list[str] = []
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            session_path = Path(temp_dir) / "session.jsonl"
            _ = session_path.write_text("rejected output", encoding="utf-8")
            deps = mock.Mock()
            deps.parse_session_mirror_target = session_mirror.parse_session_mirror_target
            deps.choose_thread = lambda thread_id, cwd: FakeThread(str(session_path))
            deps.get_thread_rollout_path = lambda thread: thread.rollout_path
            deps.update_session_mirror_cursor = (
                lambda thread_id, rollout_path, cursor: updates.append(
                    (thread_id, rollout_path, cursor)
                )
            )
            deps.log = logs.append
            typed_deps = cast(
                session_mirror_target.SessionMirrorTargetDeps[
                    object,
                    object,
                    object,
                    object,
                ],
                deps,
            )
            target = {"codex_thread_id": "thread-1", "discord_thread_id": 222}
            pro_session_mirror_gate.reject("thread-1")

            await session_mirror_target.mirror_session_target(target, deps=typed_deps)
            await session_mirror_target.mirror_session_target(target, deps=typed_deps)

        self.assertEqual(
            updates,
            [("thread-1", str(session_path), len("rejected output"))],
        )
        self.assertEqual(
            logs,
            [
                "pro_session_mirror_output_discarded "
                + f"target=thread-1 cursor={len('rejected output')}"
            ],
        )
        self.assertEqual(
            pro_session_mirror_gate.mode("thread-1"),
            pro_session_mirror_gate.GateMode.OPEN,
        )


if __name__ == "__main__":
    _ = unittest.main()
