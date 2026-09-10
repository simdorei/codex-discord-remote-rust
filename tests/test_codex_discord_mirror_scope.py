from __future__ import annotations

import unittest

import codex_discord_mirror_scope as mirror_scope
from codex_thread_models import ThreadInfo


class FakeMirrorScopeBridge:
    def __init__(self) -> None:
        self.threads: list[ThreadInfo] = [
            ThreadInfo(
                id="thread-1",
                title="one",
                cwd="C:\\repo\\one",
                updated_at=1,
                rollout_path="thread-1.jsonl",
                model="gpt-5.5",
                reasoning_effort="xhigh",
                tokens_used=0,
            ),
            ThreadInfo(
                id="thread-2",
                title="two",
                cwd="C:\\repo\\two",
                updated_at=2,
                rollout_path="thread-2.jsonl",
                model="gpt-5.5",
                reasoning_effort="xhigh",
                tokens_used=0,
            ),
        ]
        self.calls: list[str] = []

    def load_user_root_threads(self) -> list[ThreadInfo]:
        self.calls.append("root")
        return list(self.threads)

    def load_recent_threads(self, limit: int = 20) -> list[ThreadInfo]:
        self.calls.append(f"recent:{limit}")
        return list(self.threads[:limit])

    def filter_thread_list_for_target(
        self,
        threads: list[ThreadInfo],
        target_thread_id: str,
        cwd: str | None,
    ) -> list[ThreadInfo]:
        self.calls.append(f"cwd:{cwd or '-'}")
        self.calls.append(f"target:{target_thread_id}")
        return [thread for thread in threads if thread.id == target_thread_id]


class MirrorScopeTests(unittest.TestCase):
    def test_load_mirror_scope_threads_keeps_db_root_default(self) -> None:
        bridge = FakeMirrorScopeBridge()

        threads = mirror_scope.load_mirror_scope_threads(bridge)

        self.assertEqual([thread.id for thread in threads], ["thread-1", "thread-2"])
        self.assertEqual(bridge.calls, ["root"])

    def test_load_mirror_scope_threads_bounds_recent_limit(self) -> None:
        bridge = FakeMirrorScopeBridge()

        _ = mirror_scope.load_mirror_scope_threads(bridge, 999)

        self.assertEqual(bridge.calls, ["recent:100"])

    def test_filter_threads_for_discord_channel_uses_mirror_target_first(self) -> None:
        bridge = FakeMirrorScopeBridge()

        threads = mirror_scope.filter_threads_for_discord_channel(
            list(bridge.threads),
            123,
            bridge_module=bridge,
            get_mirrored_codex_thread_id=lambda _channel_id: "thread-2",
            get_mirror_project_for_channel=lambda _channel_id: ("unused", "unused"),
            project_keys_match=lambda current, expected: current == expected,
            get_project_key=lambda thread: thread.cwd,
        )

        self.assertEqual([thread.id for thread in threads], ["thread-2"])
        self.assertEqual(bridge.calls, ["cwd:-", "target:thread-2"])

    def test_filter_threads_for_discord_channel_falls_back_to_project_scope(self) -> None:
        bridge = FakeMirrorScopeBridge()

        threads = mirror_scope.filter_threads_for_discord_channel(
            list(bridge.threads),
            123,
            bridge_module=bridge,
            get_mirrored_codex_thread_id=lambda _channel_id: None,
            get_mirror_project_for_channel=lambda _channel_id: ("C:\\repo\\one", "one"),
            project_keys_match=lambda current, expected: current == expected,
            get_project_key=lambda thread: thread.cwd,
        )

        self.assertEqual([thread.id for thread in threads], ["thread-1"])
        self.assertEqual(bridge.calls, [])


if __name__ == "__main__":
    _ = unittest.main()
