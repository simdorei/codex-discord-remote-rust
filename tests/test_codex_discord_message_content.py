from __future__ import annotations

import unittest

import codex_discord_message_content as message_content
import codex_discord_message_gate as message_gate
import codex_discord_message_target as message_target


class DiscordMessageContentTests(unittest.TestCase):
    def test_flags_bot_bridge_operational_packet_before_other_preparation(self) -> None:
        target = message_target.DiscordMessageTarget(None, "selected")

        prepared = message_content.prepare_discord_message_content(
            "<@123> PROGRESS: still running",
            target,
            bot_bridge_mention=True,
            bridge_user_ids={123},
            has_attachments=False,
        )

        self.assertTrue(prepared.bot_bridge_operational_packet)
        self.assertEqual(prepared.content, "<@123> PROGRESS: still running")
        self.assertIs(prepared.target, target)

    def test_applies_attachment_prompt_for_empty_attachment_message(self) -> None:
        prepared = message_content.prepare_discord_message_content(
            "   ",
            message_target.DiscordMessageTarget(None, "selected"),
            bot_bridge_mention=False,
            bridge_user_ids=set(),
            has_attachments=True,
        )

        self.assertEqual(prepared.content, message_gate.ATTACHMENT_INSPECTION_PROMPT)

    def test_bot_bridge_mention_allows_explicit_target_resolution(self) -> None:
        target = message_target.DiscordMessageTarget(None, "selected")

        prepared = message_content.prepare_discord_message_content(
            "Work thread: 019eeaac-6170-7133-86ac-bef0f1c6e865",
            target,
            bot_bridge_mention=True,
            bridge_user_ids={123},
            has_attachments=False,
        )

        self.assertEqual(prepared.target.target_source, "explicit")
        self.assertEqual(prepared.target.target_thread_id, "019eeaac-6170-7133-86ac-bef0f1c6e865")


if __name__ == "__main__":
    _ = unittest.main()
