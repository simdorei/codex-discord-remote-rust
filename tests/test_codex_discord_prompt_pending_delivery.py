from __future__ import annotations

from dataclasses import dataclass, field
import unittest

import codex_discord_prompt_pending_delivery as pending_delivery


@dataclass(frozen=True, slots=True)
class FakeRelay:
    sent_live: bool = False


@dataclass(slots=True)
class FakeChannel:
    messages: list[str] = field(default_factory=list)


@dataclass(slots=True)
class PendingDeliveryFixture:
    pending: bool = False
    logs: list[str] = field(default_factory=list)

    async def send_chunks(
        self,
        channel: FakeChannel,
        content: str,
        *,
        context: str | None = None,
    ) -> None:
        _ = context
        channel.messages.append(content)

    def build(self) -> pending_delivery.AskStreamPendingDeliveryDeps[FakeChannel]:
        return pending_delivery.AskStreamPendingDeliveryDeps(
            is_delivery_confirmation_timeout=lambda output: self.pending,
            send_chunks=self.send_chunks,
            format_log_text_len=lambda output: len(output or ""),
            log=self.logs.append,
        )


class AskStreamPendingDeliveryTests(unittest.IsolatedAsyncioTestCase):
    async def test_non_pending_returns_false_without_send_or_log(self) -> None:
        fixture = PendingDeliveryFixture(pending=False)
        channel = FakeChannel()

        handled = await pending_delivery.handle_ask_stream_delivery_pending(
            channel,
            exit_code=1,
            output="ordinary failure",
            relay=FakeRelay(sent_live=True),
            target_thread_id="thread-1",
            log_pending=True,
            deps=fixture.build(),
        )

        self.assertFalse(handled)
        self.assertEqual(channel.messages, [])
        self.assertEqual(fixture.logs, [])

    async def test_initial_pending_logs_and_sends_formatted_output(self) -> None:
        fixture = PendingDeliveryFixture(pending=True)
        channel = FakeChannel()
        output = "\n".join(
            [
                "target_thread: thread-1",
                "ERROR: Prompt delivery could not be confirmed in any recent Codex thread after IPC delivery.",
                "The transport reported success, but no matching user message was recorded.",
            ]
        )

        handled = await pending_delivery.handle_ask_stream_delivery_pending(
            channel,
            exit_code=1,
            output=output,
            relay=FakeRelay(sent_live=True),
            target_thread_id="thread-1",
            log_pending=True,
            deps=fixture.build(),
        )

        self.assertTrue(handled)
        self.assertEqual(len(channel.messages), 1)
        self.assertIn("[delivery_pending]", channel.messages[0])
        self.assertNotIn("target_thread: thread-1", channel.messages[0])
        self.assertNotIn("ERROR:", channel.messages[0])
        self.assertIn("ask_stream_delivery_pending exit=1 target=thread-1 sent_live=True", "\n".join(fixture.logs))

    async def test_retry_pending_sends_without_initial_log_marker(self) -> None:
        fixture = PendingDeliveryFixture(pending=True)
        channel = FakeChannel()

        handled = await pending_delivery.handle_ask_stream_delivery_pending(
            channel,
            exit_code=1,
            output="ERROR: IPC start-turn failed: thread-follower-start-turn-timeout",
            relay=FakeRelay(sent_live=False),
            target_thread_id="thread-1",
            log_pending=False,
            deps=fixture.build(),
        )

        self.assertTrue(handled)
        self.assertEqual(len(channel.messages), 1)
        self.assertIn("Wait for the mirrored reply before resending.", channel.messages[0])
        self.assertNotIn("thread-follower-start-turn-timeout", channel.messages[0])
        self.assertEqual(fixture.logs, [])

    def test_formatter_hides_transport_metadata(self) -> None:
        output = "\n".join(
            [
                "target_thread: thread-1",
                "ERROR: Prompt delivery could not be confirmed in any recent Codex thread after IPC delivery.",
                "The transport reported success, but no matching user message was recorded.",
                "ui_activation: ipc-thread-follower-start-turn",
                "thread_ref: codex-discord-remote:2",
            ]
        )

        formatted = pending_delivery.format_pending_ask_delivery_output(output)

        self.assertIn("[delivery_pending]", formatted)
        self.assertIn("Wait for the mirrored reply before resending.", formatted)
        self.assertNotIn("target_thread: thread-1", formatted)
        self.assertNotIn("ui_activation: ipc-thread-follower-start-turn", formatted)
        self.assertNotIn("thread_ref:", formatted)
        self.assertNotIn("ERROR:", formatted)
        self.assertNotIn("transport reported success", formatted)
