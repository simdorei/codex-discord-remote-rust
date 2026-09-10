from __future__ import annotations

import unittest

from codex_app_server_transport_children import ChildLifecycleTracker
from codex_app_server_transport_replies import JsonObject


class ChildLifecycleTrackerTests(unittest.TestCase):
    def test_official_spawn_item_is_idempotent_and_resolves_nested_root(self) -> None:
        tracker = ChildLifecycleTracker()
        logs: list[str] = []
        tracker.reset(3)
        child = self._spawn("root", "child")
        nested = self._spawn("child", "grandchild")

        tracker.record_notification(child, generation=3, log=logs.append)
        tracker.record_notification(child, generation=3, log=logs.append)
        tracker.record_notification(nested, generation=3, log=logs.append)

        snapshot = tracker.snapshot(3)
        self.assertTrue(snapshot.cleanup_pending)
        self.assertEqual(snapshot.root_thread_ids, ("root",))
        self.assertEqual(snapshot.child_thread_ids, ("child", "grandchild"))
        self.assertEqual(len(logs), 2)

    def test_stale_generation_and_malformed_notifications_are_ignored(self) -> None:
        tracker = ChildLifecycleTracker()
        tracker.reset(8)

        tracker.record_notification(self._spawn("root", "child"), generation=7, log=lambda _line: None)
        tracker.record_notification(
            {"method": "item/completed", "params": {"item": {"type": "collabAgentToolCall"}}},
            generation=8,
            log=lambda _line: None,
        )

        self.assertFalse(tracker.snapshot(8).cleanup_pending)
        self.assertEqual(tracker.snapshot(7).child_thread_ids, ())

    @staticmethod
    def _spawn(parent_id: str, child_id: str) -> JsonObject:
        return {
            "method": "item/completed",
            "params": {
                "threadId": parent_id,
                "item": {
                    "type": "collabAgentToolCall",
                    "id": f"spawn-{child_id}",
                    "tool": "spawnAgent",
                    "status": "completed",
                    "senderThreadId": parent_id,
                    "receiverThreadIds": [child_id],
                    "agentsStates": {},
                },
            },
        }


if __name__ == "__main__":
    _ = unittest.main()
