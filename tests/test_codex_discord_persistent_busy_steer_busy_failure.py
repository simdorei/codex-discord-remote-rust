from __future__ import annotations

import unittest

import codex_discord_persistent_busy_steer as persistent_busy_steer


class PersistentBusySteerBusyFailureTests(unittest.IsolatedAsyncioTestCase):
    async def test_busy_failure_refreshes_app_menu_and_skips_fallbacks(self) -> None:
        menu_calls: list[tuple[object, str | None, str, str]] = []
        stale_calls: list[str] = []
        followups: list[tuple[str, str, int, str, bool]] = []
        logs: list[str] = []
        interaction = object()
        channel = object()

        async def send_codex_app_menu_if_available(
            channel: object,
            target_thread_id: str | None,
            output: str,
            *,
            reason: str,
        ) -> bool:
            menu_calls.append((channel, target_thread_id, output, reason))
            return True

        async def send_stale_block_message(
            channel: object,
            prompt: str,
            target_thread_id: str | None,
            *,
            reason: str,
        ) -> bool:
            _ = (channel, prompt, target_thread_id)
            stale_calls.append(reason)
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
            _ = interaction
            followups.append((content, title, exit_code, log_prefix, ephemeral))

        handled = await persistent_busy_steer.handle_persistent_busy_steer_busy_failure(
            interaction,
            channel,
            "please steer",
            "thread-1",
            "busy output",
            deps=persistent_busy_steer.PersistentBusySteerBusyFailureDeps(
                send_codex_app_menu_if_available=send_codex_app_menu_if_available,
                send_stale_block_message=send_stale_block_message,
                send_followup_chunks=send_followup_chunks,
                resolve_target_ref=lambda target_thread_id: (target_thread_id, "taxlab:1"),
                build_not_accepted_message=lambda target_ref: f"not accepted for {target_ref}",
                log=logs.append,
            ),
        )

        self.assertTrue(handled)
        self.assertEqual(
            menu_calls,
            [(channel, "thread-1", "busy output", "persistent_steer_busy_failure")],
        )
        self.assertEqual(
            followups,
            [
                (
                    persistent_busy_steer.STEER_APP_MENU_REFRESHED_FOLLOWUP_MESSAGE,
                    "Steering",
                    0,
                    "button_response",
                    True,
                )
            ],
        )
        self.assertEqual(stale_calls, [])
        self.assertEqual(logs, [])

    async def test_busy_failure_uses_stale_block_before_not_accepted_followup(self) -> None:
        stale_calls: list[tuple[object, str, str | None, str]] = []
        followups: list[str] = []
        logs: list[str] = []
        interaction = object()
        channel = object()

        async def send_codex_app_menu_if_available(
            channel: object,
            target_thread_id: str | None,
            output: str,
            *,
            reason: str,
        ) -> bool:
            _ = (channel, target_thread_id, output, reason)
            return False

        async def send_stale_block_message(
            channel: object,
            prompt: str,
            target_thread_id: str | None,
            *,
            reason: str,
        ) -> bool:
            stale_calls.append((channel, prompt, target_thread_id, reason))
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
            _ = (interaction, title, exit_code, log_prefix, ephemeral)
            followups.append(content)

        handled = await persistent_busy_steer.handle_persistent_busy_steer_busy_failure(
            interaction,
            channel,
            "please steer",
            "thread-2",
            "busy output",
            deps=persistent_busy_steer.PersistentBusySteerBusyFailureDeps(
                send_codex_app_menu_if_available=send_codex_app_menu_if_available,
                send_stale_block_message=send_stale_block_message,
                send_followup_chunks=send_followup_chunks,
                resolve_target_ref=lambda target_thread_id: (target_thread_id, "taxlab:2"),
                build_not_accepted_message=lambda target_ref: f"not accepted for {target_ref}",
                log=logs.append,
            ),
        )

        self.assertTrue(handled)
        self.assertEqual(stale_calls, [(channel, "please steer", "thread-2", "persistent_steer_busy_failure")])
        self.assertEqual(followups, [persistent_busy_steer.STEER_STALE_BLOCK_FOLLOWUP_MESSAGE])
        self.assertEqual(logs, [])

    async def test_busy_failure_sends_not_accepted_with_missing_target_log(self) -> None:
        followups: list[tuple[str, str, int, str, bool]] = []
        logs: list[str] = []

        async def send_codex_app_menu_if_available(
            channel: object,
            target_thread_id: str | None,
            output: str,
            *,
            reason: str,
        ) -> bool:
            _ = (channel, target_thread_id, output, reason)
            return False

        async def send_stale_block_message(
            channel: object,
            prompt: str,
            target_thread_id: str | None,
            *,
            reason: str,
        ) -> bool:
            _ = (channel, prompt, target_thread_id, reason)
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
            _ = interaction
            followups.append((content, title, exit_code, log_prefix, ephemeral))

        handled = await persistent_busy_steer.handle_persistent_busy_steer_busy_failure(
            object(),
            object(),
            "please steer",
            None,
            "busy output",
            deps=persistent_busy_steer.PersistentBusySteerBusyFailureDeps(
                send_codex_app_menu_if_available=send_codex_app_menu_if_available,
                send_stale_block_message=send_stale_block_message,
                send_followup_chunks=send_followup_chunks,
                resolve_target_ref=lambda target_thread_id: (target_thread_id, "selected"),
                build_not_accepted_message=lambda target_ref: f"not accepted for {target_ref}",
                log=logs.append,
            ),
        )

        self.assertTrue(handled)
        self.assertEqual(followups, [("not accepted for selected", "Steering", 0, "button_response", True)])
        self.assertEqual(logs, ["steer_busy_status_sent reason=persistent_steer_busy_failure target=-"])
