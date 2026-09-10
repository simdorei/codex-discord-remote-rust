from __future__ import annotations

import unittest

import codex_discord_persistent_busy_choice as persistent_busy_choice


class PersistentBusyChoiceQueueFollowupTests(unittest.IsolatedAsyncioTestCase):
    async def test_queue_followup_sends_position_and_logs_target(self) -> None:
        followups: list[tuple[str, str, str]] = []
        logs: list[str] = []
        target = object()

        async def send_followup(
            interaction: persistent_busy_choice.PersistentBusyInteraction,
            content: str,
            *,
            log_prefix: str,
            context: str,
        ) -> None:
            self.assertIs(interaction, target)
            followups.append((content, log_prefix, context))

        handled = await persistent_busy_choice.handle_persistent_busy_queue_followup(
            target,
            user_id=242286902982606848,
            choice_id="0123456789abcdef01234567",
            position=3,
            target_thread_id="thread-1",
            prompt="please queue",
            deps=persistent_busy_choice.PersistentBusyQueueFollowupDeps(
                send_followup=send_followup,
                log=logs.append,
            ),
        )

        expected_log = (
            "busy_choice_persistent_queue "
            "user=242286902982606848 choice=0123456789abcdef01234567 "
            "position=3 target=thread-1 prompt_len=12"
        )
        self.assertTrue(handled)
        self.assertEqual(followups, [("Queued at position 3.", "button_followup", "persistent_queue_next")])
        self.assertEqual(logs, [expected_log])

    async def test_queue_followup_logs_dash_for_missing_target(self) -> None:
        logs: list[str] = []

        async def send_followup(
            interaction: persistent_busy_choice.PersistentBusyInteraction,
            content: str,
            *,
            log_prefix: str,
            context: str,
        ) -> None:
            _ = (interaction, content, log_prefix, context)

        _ = await persistent_busy_choice.handle_persistent_busy_queue_followup(
            object(),
            user_id=1,
            choice_id="0123456789abcdef01234567",
            position=9,
            target_thread_id=None,
            prompt="",
            deps=persistent_busy_choice.PersistentBusyQueueFollowupDeps(
                send_followup=send_followup,
                log=logs.append,
            ),
        )

        expected_log = "busy_choice_persistent_queue user=1 choice=0123456789abcdef01234567 position=9 target=- prompt_len=0"
        self.assertEqual(logs, [expected_log])
