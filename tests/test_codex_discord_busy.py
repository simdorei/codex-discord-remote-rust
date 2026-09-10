from __future__ import annotations

import unittest

import codex_discord_busy as busy


def fit_single_message(text: str) -> str:
    return text


class DiscordBusyMessageTests(unittest.TestCase):
    def test_busy_choice_message_truncates_prompt_and_keeps_footer(self) -> None:
        output = busy.build_busy_choice_message(
            "A" * 300,
            "thread-1",
            discord_max_len=180,
            fit_single_message_func=fit_single_message,
        )

        self.assertLessEqual(len(output), 180)
        self.assertIn("Codex app is still processing this mapped thread.", output)
        self.assertIn("[prompt truncated for Discord]", output)
        self.assertTrue(output.endswith("Choose the Discord action for this message."))

    def test_make_busy_choice_payload_uses_message_and_view_factories(self) -> None:
        calls: list[tuple[str, str, str | None, bool]] = []

        def make_view(
            source_message: str,
            prompt: str,
            *,
            target_thread_id: str | None,
            allow_steer: bool,
        ) -> str:
            calls.append((source_message, prompt, target_thread_id, allow_steer))
            return "view"

        content, view = busy.make_busy_choice_payload(
            "source-message",
            "run tests",
            target_thread_id="thread-1",
            allow_steer=True,
            build_busy_choice_message_func=lambda prompt, target_thread_id: (
                f"{target_thread_id}:{prompt}"
            ),
            make_busy_choice_view_func=make_view,
        )

        self.assertEqual(content, "thread-1:run tests")
        self.assertEqual(view, "view")
        self.assertEqual(calls, [("source-message", "run tests", "thread-1", True)])

    def test_stale_busy_steer_block_message_formats_thread_age_and_prompt(self) -> None:
        output = busy.build_stale_busy_steer_block_message(
            "  please continue  ",
            target_ref="codex-discord-remote:1",
            age_seconds=125.0,
            fit_single_message_func=fit_single_message,
        )

        self.assertIn("This Codex thread is busy but has not produced new output recently.", output)
        self.assertIn("thread: codex-discord-remote:1", output)
        self.assertIn("last Codex activity: about 2 min ago", output)
        self.assertIn("message: please continue", output)

    def test_stale_busy_steer_block_message_uses_selected_and_minimum_one_minute(self) -> None:
        output = busy.build_stale_busy_steer_block_message(
            "",
            target_ref="",
            age_seconds=1.0,
            fit_single_message_func=fit_single_message,
        )

        self.assertIn("thread: selected", output)
        self.assertIn("last Codex activity: about 1 min ago", output)
        self.assertIn("message: ", output)


class FakeBusyChannel:
    def __init__(self) -> None:
        self.sent: list[tuple[str, str | None, str]] = []


class DiscordBusyChoiceSendTests(unittest.IsolatedAsyncioTestCase):
    async def test_send_busy_choice_message_sends_payload_and_logs_success(self) -> None:
        channel = FakeBusyChannel()
        logs: list[str] = []

        async def send_message_tracked(
            target: FakeBusyChannel,
            content: str,
            *,
            view: str | None = None,
            context: str,
        ) -> None:
            target.sent.append((content, view, context))

        async def send_chunks(target: FakeBusyChannel, text: str) -> int:
            _ = target, text
            raise AssertionError("success path must not send fallback chunks")

        result = await busy.send_busy_choice_message(
            channel,
            "prompt",
            "view",
            reason="late_busy",
            target_thread_id="thread-1",
            prompt="run tests",
            send_message_tracked_func=send_message_tracked,
            send_chunks_func=send_chunks,
            log_busy_choice_sent_func=lambda reason, target_thread_id, prompt: logs.append(
                f"{reason}:{target_thread_id}:{prompt}"
            ),
            format_log_text_len_func=lambda text: str(len(text)),
            log_func=logs.append,
        )

        self.assertTrue(result)
        self.assertEqual(channel.sent, [("prompt", "view", "busy_choice:late_busy")])
        self.assertEqual(logs, ["late_busy:thread-1:run tests"])
