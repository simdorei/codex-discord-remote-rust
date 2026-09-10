import unittest

import codex_discord_queue_messages as queue_messages


class QueueMessageTests(unittest.TestCase):
    def test_build_retract_message_for_removed_item(self) -> None:
        self.assertEqual(
            queue_messages.build_retract_message({"removed": 1, "remaining": 2}, "thread-1"),
            "Retracted your latest queued ask for `thread-1`. remaining_queued: 2",
        )

    def test_build_retract_message_for_active_item(self) -> None:
        self.assertEqual(
            queue_messages.build_retract_message({"removed": 0, "active": True}, "thread-1"),
            "No queued ask from you for `thread-1`. The active ask cannot be retracted from Discord.",
        )

    def test_build_retract_message_for_missing_item(self) -> None:
        self.assertEqual(
            queue_messages.build_retract_message({"removed": 0}, "thread-1"),
            "No queued ask from you for `thread-1`.",
        )


if __name__ == "__main__":
    _ = unittest.main()
