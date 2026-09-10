from __future__ import annotations

from dataclasses import dataclass, field
import unittest

import codex_discord_prompt_retry_exhausted as retry_exhausted


@dataclass(slots=True)
class FakeChannel:
    messages: list[str] = field(default_factory=list)


@dataclass(slots=True)
class RetryExhaustedFixture:
    busy: bool = False
    logs: list[str] = field(default_factory=list)
    builder_calls: list[tuple[str, int]] = field(default_factory=list)

    async def send_chunks(
        self,
        channel: FakeChannel,
        content: str,
        *,
        context: str | None = None,
    ) -> None:
        _ = context
        channel.messages.append(content)

    def build_message(self, target_ref: str, retry_attempts: int) -> str:
        self.builder_calls.append((target_ref, retry_attempts))
        return f"busy target={target_ref} attempts={retry_attempts}"

    def build(self) -> retry_exhausted.RetryExhaustedDeps[FakeChannel]:
        return retry_exhausted.RetryExhaustedDeps(
            is_selected_thread_busy_error=lambda exit_code, output: self.busy,
            build_codex_app_busy_retry_message=self.build_message,
            send_chunks=self.send_chunks,
            format_log_text_len=lambda output: len(output or ""),
            log=self.logs.append,
        )


class RetryExhaustedTests(unittest.IsolatedAsyncioTestCase):
    async def test_non_busy_output_returns_not_handled_without_send_or_log(self) -> None:
        fixture = RetryExhaustedFixture(busy=False)
        channel = FakeChannel()

        handled = await retry_exhausted.handle_retry_exhausted_status(
            channel,
            exit_code=0,
            output="final answer",
            target_thread_id="thread-1",
            target_ref="taxlab:1",
            retry_attempts=1,
            deps=fixture.build(),
        )

        self.assertFalse(handled)
        self.assertEqual(channel.messages, [])
        self.assertEqual(fixture.logs, [])
        self.assertEqual(fixture.builder_calls, [])

    async def test_busy_output_logs_and_sends_zero_attempt_status(self) -> None:
        fixture = RetryExhaustedFixture(busy=True)
        channel = FakeChannel()

        handled = await retry_exhausted.handle_retry_exhausted_status(
            channel,
            exit_code=1,
            output="busy",
            target_thread_id="thread-1",
            target_ref="taxlab:1",
            retry_attempts=0,
            deps=fixture.build(),
        )

        self.assertTrue(handled)
        self.assertEqual(channel.messages, ["busy target=taxlab:1 attempts=0"])
        self.assertEqual(fixture.builder_calls, [("taxlab:1", 0)])
        self.assertIn("ask_stream_busy_retry_exhausted target=thread-1 attempts=0 output_len=4", "\n".join(fixture.logs))

    async def test_busy_output_forwards_positive_retry_attempts(self) -> None:
        fixture = RetryExhaustedFixture(busy=True)
        channel = FakeChannel()

        handled = await retry_exhausted.handle_retry_exhausted_status(
            channel,
            exit_code=1,
            output="busy again",
            target_thread_id="thread-1",
            target_ref="taxlab:1",
            retry_attempts=2,
            deps=fixture.build(),
        )

        self.assertTrue(handled)
        self.assertEqual(fixture.builder_calls, [("taxlab:1", 2)])
        self.assertEqual(channel.messages, ["busy target=taxlab:1 attempts=2"])

    async def test_missing_target_logs_dash(self) -> None:
        fixture = RetryExhaustedFixture(busy=True)
        channel = FakeChannel()

        handled = await retry_exhausted.handle_retry_exhausted_status(
            channel,
            exit_code=1,
            output="busy",
            target_thread_id=None,
            target_ref="-",
            retry_attempts=1,
            deps=fixture.build(),
        )

        self.assertTrue(handled)
        self.assertIn("target=-", "\n".join(fixture.logs))
