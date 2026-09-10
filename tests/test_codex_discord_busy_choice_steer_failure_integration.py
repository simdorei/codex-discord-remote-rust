from __future__ import annotations

from pathlib import Path
import tempfile
import unittest

import codex_discord_bot as bot

from tests.test_codex_discord_bot import EnvPatch, FailingFollowup, FakeInteraction, FakeTarget
from tests.test_codex_discord_busy_choice_steer_callback_integration import (
    BusyMessage,
    find_steer_button,
)


class DiscordBusyChoiceSteerFailureIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_steer_now_busy_failure_reports_status_without_new_busy_view(self) -> None:
        original_run_steering_prompt = bot.run_steering_prompt
        original_build_context_warning = bot.build_context_warning
        original_get_interactive_state = bot.get_interactive_state_for_thread
        original_resolve_target_ref = bot.resolve_target_ref
        old_handoffs = dict(bot.get_runtime_state().steering_handoffs)
        try:
            bot.get_runtime_state().steering_handoffs.clear()

            def no_interactive_state(target_thread_id: str | None) -> tuple[str, str | None, str]:
                _ = target_thread_id
                return "", None, ""

            def resolve_target_ref(target_thread_id: str | None) -> tuple[str | None, str]:
                return target_thread_id, "taxlab:1"

            def fake_run_steering_prompt(
                prompt: str,
                target_thread_id: str | None,
            ) -> bot.SteeringPromptResult:
                _ = prompt, target_thread_id
                return bot.SteeringPromptResult(
                    1,
                    "ERROR: The selected thread is still busy. "
                    + "Wait, switch to another thread, or pass --force-while-busy.",
                )

            def empty_context_warning(target_thread_id: str | None) -> str:
                _ = target_thread_id
                return ""

            bot.get_interactive_state_for_thread = no_interactive_state
            bot.resolve_target_ref = resolve_target_ref
            bot.run_steering_prompt = fake_run_steering_prompt
            bot.build_context_warning = empty_context_warning

            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                message = BusyMessage()
                view = bot.BusyChoiceView(message, "please steer", target_thread_id="thread-1")
                button = find_steer_button(view)
                interaction = FakeInteraction()

                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    await button.callback(interaction)
                log_text = log_path.read_text(encoding="utf-8")

            self.assertTrue(interaction.response.deferred)
            self.assertEqual(interaction.message.edits, [None])
            self.assertEqual(
                message.channel.messages,
                [("Discord steering submitted.\nmessage: please steer", None)],
            )
            self.assertEqual(len(interaction.followup.messages), 1)
            content = str(interaction.followup.messages[0])
            self.assertIn("Codex app did not accept this steering message yet.", content)
            self.assertIn("target: `taxlab:1`", content)
            self.assertNotIn("selected thread is still busy", content.lower())
            self.assertIn("steer_busy_status_sent reason=steer_busy_failure exit=1 target=thread-1", log_text)
            self.assertNotIn("busy_choice_sent reason=steer_busy_failure", log_text)
            self.assertIn("component_message_components_cleared context=busy_choice_steer", log_text)
            self.assertIn("steering_start_ack_sent target=thread-1", log_text)
            self.assertNotIn("prompt=please steer", log_text)
            self.assertEqual(bot.get_runtime_state().steering_handoffs, {})
        finally:
            bot.get_runtime_state().steering_handoffs.clear()
            bot.get_runtime_state().steering_handoffs.update(old_handoffs)
            bot.run_steering_prompt = original_run_steering_prompt
            bot.build_context_warning = original_build_context_warning
            bot.get_interactive_state_for_thread = original_get_interactive_state
            bot.resolve_target_ref = original_resolve_target_ref

    async def test_steer_now_sends_prompt_after_stale_busy_notice(self) -> None:
        original_run_steering_prompt = bot.run_steering_prompt
        original_get_stale_info = bot.get_stale_busy_steer_block_info
        try:
            run_calls: list[tuple[str, str | None]] = []

            def fake_run_steering_prompt(
                prompt: str,
                target_thread_id: str | None,
            ) -> bot.SteeringPromptResult:
                run_calls.append((prompt, target_thread_id))
                return bot.SteeringPromptResult(
                    0,
                    "sent",
                    target_thread_id=target_thread_id,
                    target_ref="taxlab:1",
                )

            def stale_info(target_thread_id: str | None) -> tuple[str, str, float]:
                _ = target_thread_id
                return "thread-1", "taxlab:1", 660.0

            bot.run_steering_prompt = fake_run_steering_prompt
            bot.get_stale_busy_steer_block_info = stale_info

            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                message = BusyMessage()
                view = bot.BusyChoiceView(message, "please steer", target_thread_id="thread-1")
                button = find_steer_button(view)
                interaction = FakeInteraction(command_name="ask", channel_id=222)

                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    await button.callback(interaction)
                log_text = log_path.read_text(encoding="utf-8")

            self.assertTrue(interaction.response.deferred)
            self.assertEqual(interaction.message.edits, [None])
            channel_texts = [str(content) for content, _view in message.channel.messages]
            self.assertTrue(
                any("has not produced new output recently" in text for text in channel_texts),
                channel_texts,
            )
            self.assertTrue(
                any("steering" in text.lower() for text in channel_texts),
                channel_texts,
            )
            self.assertEqual(len(interaction.followup.messages), 1)
            self.assertIn("Steering sent", str(interaction.followup.messages[0]))
            self.assertEqual(interaction.followup.kwargs, [{"ephemeral": True}])
            self.assertEqual(run_calls, [("please steer", "thread-1")])
            self.assertIn("stale_busy_steer_blocked reason=steer_now target=thread-1", log_text)
            self.assertIn("steering_start_ack_sent target=thread-1", log_text)
        finally:
            bot.run_steering_prompt = original_run_steering_prompt
            bot.get_stale_busy_steer_block_info = original_get_stale_info

    async def test_steer_now_waiting_input_failure_resends_app_menu(self) -> None:
        original_run_steering_prompt = bot.run_steering_prompt
        original_build_context_warning = bot.build_context_warning
        original_get_interactive_state = bot.get_interactive_state_for_thread
        original_resolve_target_ref = bot.resolve_target_ref
        try:
            def no_interactive_state(target_thread_id: str | None) -> tuple[str, str | None, str]:
                _ = target_thread_id
                return "", None, ""

            def resolve_target_ref(target_thread_id: str | None) -> tuple[str | None, str]:
                return target_thread_id, "taxlab:1"

            def fake_run_steering_prompt(
                prompt: str,
                target_thread_id: str | None,
            ) -> bot.SteeringPromptResult:
                _ = prompt, target_thread_id
                return bot.SteeringPromptResult(
                    1,
                    "ERROR: The selected thread is waiting on a follow-up choice or input in Codex Desktop. "
                    + "Open the thread in the app and respond there first.",
                )

            def empty_context_warning(target_thread_id: str | None) -> str:
                _ = target_thread_id
                return ""

            bot.get_interactive_state_for_thread = no_interactive_state
            bot.resolve_target_ref = resolve_target_ref
            bot.run_steering_prompt = fake_run_steering_prompt
            bot.build_context_warning = empty_context_warning

            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                message = BusyMessage()
                view = bot.BusyChoiceView(message, "please steer", target_thread_id="thread-1")
                button = find_steer_button(view)
                interaction = FakeInteraction()

                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    await button.callback(interaction)
                log_text = log_path.read_text(encoding="utf-8")

            self.assertTrue(interaction.response.deferred)
            self.assertEqual(interaction.message.edits, [None])
            self.assertEqual(len(interaction.followup.messages), 1)
            self.assertIn("Codex app menu was refreshed", str(interaction.followup.messages[0]))
            self.assertEqual(len(message.channel.messages), 2)
            start_content, start_view = message.channel.messages[0]
            menu_content, menu_view = message.channel.messages[1]
            self.assertIn("Discord steering submitted", str(start_content))
            self.assertIsNone(start_view)
            self.assertIn("Waiting input", str(menu_content))
            self.assertIn("Pending input", str(menu_content))
            self.assertIsNone(menu_view)
            self.assertIn("codex_app_menu_sent reason=steer_busy_failure target=thread-1 state=waiting-input", log_text)
            self.assertNotIn("busy_choice_sent reason=steer_busy_failure", log_text)
            self.assertIn("component_message_components_cleared context=busy_choice_steer", log_text)
        finally:
            bot.run_steering_prompt = original_run_steering_prompt
            bot.build_context_warning = original_build_context_warning
            bot.get_interactive_state_for_thread = original_get_interactive_state
            bot.resolve_target_ref = original_resolve_target_ref

    async def test_steer_now_busy_status_surfaces_followup_failure(self) -> None:
        original_run_steering_prompt = bot.run_steering_prompt
        original_build_context_warning = bot.build_context_warning
        original_get_interactive_state = bot.get_interactive_state_for_thread
        original_resolve_target_ref = bot.resolve_target_ref
        try:
            def no_interactive_state(target_thread_id: str | None) -> tuple[str, str | None, str]:
                _ = target_thread_id
                return "", None, ""

            def resolve_target_ref(target_thread_id: str | None) -> tuple[str | None, str]:
                return target_thread_id, "taxlab:1"

            def fake_run_steering_prompt(
                prompt: str,
                target_thread_id: str | None,
            ) -> bot.SteeringPromptResult:
                _ = prompt, target_thread_id
                return bot.SteeringPromptResult(
                    1,
                    "ERROR: The selected thread is still busy. Wait, switch to another thread.",
                )

            def empty_context_warning(target_thread_id: str | None) -> str:
                _ = target_thread_id
                return ""

            bot.get_interactive_state_for_thread = no_interactive_state
            bot.resolve_target_ref = resolve_target_ref
            bot.run_steering_prompt = fake_run_steering_prompt
            bot.build_context_warning = empty_context_warning

            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                message = BusyMessage()
                view = bot.BusyChoiceView(message, "please steer", target_thread_id="thread-1")
                button = find_steer_button(view)
                interaction = FakeInteraction(command_name="ask", channel_id=222)
                failing_followup = FailingFollowup()
                fallback_channel = FakeTarget()
                setattr(interaction, "followup", failing_followup)
                setattr(interaction, "channel", fallback_channel)

                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    with self.assertRaisesRegex(RuntimeError, "followup unavailable"):
                        await button.callback(interaction)
                log_text = log_path.read_text(encoding="utf-8")

            self.assertEqual(failing_followup.messages, [])
            self.assertEqual(fallback_channel.messages, [])
            self.assertIn("button_response_failed command=ask title='Steering'", log_text)
            self.assertIn("error=followup unavailable", log_text)
            self.assertNotIn("button_response_fallback_sent", log_text)
            self.assertNotIn("steer_busy_status_sent reason=steer_busy_failure", log_text)
            self.assertNotIn("busy_choice_sent reason=steer_busy_failure", log_text)
        finally:
            bot.run_steering_prompt = original_run_steering_prompt
            bot.build_context_warning = original_build_context_warning
            bot.get_interactive_state_for_thread = original_get_interactive_state
            bot.resolve_target_ref = original_resolve_target_ref


if __name__ == "__main__":
    _ = unittest.main()
