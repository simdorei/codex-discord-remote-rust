from __future__ import annotations

import unittest

import codex_discord_persistent_busy_steer as persistent_busy_steer


class PersistentBusyChoiceSteerTests(unittest.IsolatedAsyncioTestCase):
    async def test_stale_steer_block_returns_false_without_followup(self) -> None:
        checks: list[tuple[object, str, str | None, str]] = []
        followups: list[str] = []
        interaction = object()
        channel = object()

        async def send_stale_block_message(
            channel: object,
            prompt: str,
            target_thread_id: str | None,
            *,
            reason: str,
        ) -> bool:
            checks.append((channel, prompt, target_thread_id, reason))
            return False

        async def send_followup_chunks(
            interaction: persistent_busy_steer.PersistentBusyInteraction,
            content: str,
            *,
            title: str,
            exit_code: int,
            log_prefix: str,
            ephemeral: bool,
        ) -> None:
            _ = (interaction, title, exit_code, log_prefix, ephemeral)
            followups.append(content)

        handled = await persistent_busy_steer.handle_persistent_busy_stale_steer_block(
            interaction,
            channel,
            "please steer",
            "thread-1",
            reason="persistent_steer_now",
            deps=persistent_busy_steer.PersistentBusyStaleSteerBlockDeps(
                send_stale_block_message=send_stale_block_message,
                send_followup_chunks=send_followup_chunks,
            ),
        )

        self.assertFalse(handled)
        self.assertEqual(checks, [(channel, "please steer", "thread-1", "persistent_steer_now")])
        self.assertEqual(followups, [])

    async def test_stale_steer_block_sends_ephemeral_followup_for_block_reasons(self) -> None:
        followups: list[tuple[str, str, int, str, bool]] = []

        async def send_stale_block_message(
            channel: object,
            prompt: str,
            target_thread_id: str | None,
            *,
            reason: str,
        ) -> bool:
            _ = (channel, prompt, target_thread_id, reason)
            return True

        async def send_followup_chunks(
            interaction: persistent_busy_steer.PersistentBusyInteraction,
            content: str,
            *,
            title: str,
            exit_code: int,
            log_prefix: str,
            ephemeral: bool,
        ) -> None:
            _ = interaction
            followups.append((content, title, exit_code, log_prefix, ephemeral))

        for reason in ("persistent_steer_now", "persistent_steer_busy_failure"):
            handled = await persistent_busy_steer.handle_persistent_busy_stale_steer_block(
                object(),
                object(),
                "please steer",
                None,
                reason=reason,
                deps=persistent_busy_steer.PersistentBusyStaleSteerBlockDeps(
                    send_stale_block_message=send_stale_block_message,
                    send_followup_chunks=send_followup_chunks,
                ),
            )
            self.assertTrue(handled)

        expected_content = persistent_busy_steer.STEER_STALE_BLOCK_FOLLOWUP_MESSAGE
        self.assertEqual(
            followups,
            [
                (expected_content, "Steering", 0, "button_response", True),
                (expected_content, "Steering", 0, "button_response", True),
            ],
        )

    async def test_session_mirror_delegation_prefers_mapped_output(self) -> None:
        calls: list[str] = []

        async def prepare_mapped_session_mirror_output(
            channel: object,
            target_thread_id: str | None,
        ) -> bool:
            _ = (channel, target_thread_id)
            calls.append("mapped")
            return True

        async def prepare_session_mirror_delegation(
            channel: object,
            target_thread_id: str | None,
        ) -> bool:
            _ = (channel, target_thread_id)
            calls.append("fallback")
            return True

        delegated = await persistent_busy_steer.prepare_persistent_busy_steer_session_mirror(
            object(),
            "thread-1",
            deps=persistent_busy_steer.PersistentBusySteerSessionMirrorDeps(
                prepare_mapped_session_mirror_output=prepare_mapped_session_mirror_output,
                prepare_session_mirror_delegation=prepare_session_mirror_delegation,
            ),
        )

        self.assertTrue(delegated)
        self.assertEqual(calls, ["mapped"])

    async def test_session_mirror_delegation_falls_back_after_mapped_declines(self) -> None:
        calls: list[str] = []

        async def prepare_mapped_session_mirror_output(
            channel: object,
            target_thread_id: str | None,
        ) -> bool:
            _ = (channel, target_thread_id)
            calls.append("mapped")
            return False

        async def prepare_session_mirror_delegation(
            channel: object,
            target_thread_id: str | None,
        ) -> bool:
            _ = (channel, target_thread_id)
            calls.append("fallback")
            return True

        delegated = await persistent_busy_steer.prepare_persistent_busy_steer_session_mirror(
            object(),
            None,
            deps=persistent_busy_steer.PersistentBusySteerSessionMirrorDeps(
                prepare_mapped_session_mirror_output=prepare_mapped_session_mirror_output,
                prepare_session_mirror_delegation=prepare_session_mirror_delegation,
            ),
        )

        self.assertTrue(delegated)
        self.assertEqual(calls, ["mapped", "fallback"])

    async def test_session_mirror_delegation_returns_false_when_both_decline(self) -> None:
        calls: list[str] = []

        async def prepare_mapped_session_mirror_output(
            channel: object,
            target_thread_id: str | None,
        ) -> bool:
            _ = (channel, target_thread_id)
            calls.append("mapped")
            return False

        async def prepare_session_mirror_delegation(
            channel: object,
            target_thread_id: str | None,
        ) -> bool:
            _ = (channel, target_thread_id)
            calls.append("fallback")
            return False

        delegated = await persistent_busy_steer.prepare_persistent_busy_steer_session_mirror(
            object(),
            "thread-2",
            deps=persistent_busy_steer.PersistentBusySteerSessionMirrorDeps(
                prepare_mapped_session_mirror_output=prepare_mapped_session_mirror_output,
                prepare_session_mirror_delegation=prepare_session_mirror_delegation,
            ),
        )

        self.assertFalse(delegated)
        self.assertEqual(calls, ["mapped", "fallback"])
