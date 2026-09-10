from __future__ import annotations

import unittest

import codex_discord_busy_messages as busy_messages


class DiscordBusyMessagesTests(unittest.TestCase):
    def test_stale_busy_steer_message_is_warning_not_block_notice(self) -> None:
        message = busy_messages.build_stale_busy_steer_block_message(
            "please steer",
            target_ref="repo:1",
            age_seconds=620,
            fit_single_message_func=lambda text: text,
        )

        self.assertIn("Steering will still be sent", message)
        self.assertIn("`!stop`", message)
        self.assertIn("Stop reply", message)
        self.assertNotIn("was not sent", message)
        self.assertNotIn("!open_abort", message)


if __name__ == "__main__":
    _ = unittest.main()
