from __future__ import annotations

from dataclasses import dataclass, field
import unittest

import codex_discord_prompt_stream_result as stream_result


@dataclass(frozen=True, slots=True)
class FakeRelay:
    sent_live: bool = False
    saw_final: bool = False
    saw_aborted: bool = False
    saw_timeout: bool = False


@dataclass(frozen=True, slots=True)
class FakeChannel:
    messages: list[str] = field(default_factory=list)


@dataclass(frozen=True, slots=True)
class StreamResultFixture:
    handoff: bool = False
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

    def build(self) -> stream_result.AskStreamResultDeps[FakeChannel]:
        return stream_result.make_ask_stream_result_deps(
            send_chunks=self.send_chunks,
            had_steering_handoff_since=lambda target_thread_id, started_at: self.handoff,
            format_log_text_len=lambda output: len(output or ""),
            log=self.logs.append,
        )


class AskStreamResultTests(unittest.IsolatedAsyncioTestCase):
    async def test_delegated_success_suppresses_discord_send(self) -> None:
        fixture = StreamResultFixture()
        channel = FakeChannel()

        await stream_result.handle_ask_stream_result(
            channel,
            exit_code=0,
            output="final mirrored",
            relay=FakeRelay(sent_live=False, saw_final=False),
            target_thread_id="thread-1",
            started_at=10.0,
            delegate_to_session_mirror=True,
            deps=fixture.build(),
        )

        self.assertEqual(channel.messages, [])
        self.assertIn("ask_stream_delegated_to_session_mirror target=thread-1", "\n".join(fixture.logs))

    async def test_no_final_without_live_sends_failure(self) -> None:
        fixture = StreamResultFixture()
        channel = FakeChannel()

        await stream_result.handle_ask_stream_result(
            channel,
            exit_code=0,
            output="[ready]",
            relay=FakeRelay(sent_live=False, saw_final=False),
            target_thread_id="thread-1",
            started_at=10.0,
            delegate_to_session_mirror=False,
            deps=fixture.build(),
        )

        self.assertEqual(len(channel.messages), 1)
        self.assertIn("Ask failed", channel.messages[0])
        self.assertIn("ERROR: Codex stream completed without a final answer.", channel.messages[0])
        self.assertIn("ask_stream_no_final_error target=thread-1", "\n".join(fixture.logs))

    async def test_no_final_live_sends_failure(self) -> None:
        fixture = StreamResultFixture()
        channel = FakeChannel()

        await stream_result.handle_ask_stream_result(
            channel,
            exit_code=0,
            output="[commentary]\nworking\n\n[ready]",
            relay=FakeRelay(sent_live=True, saw_final=False),
            target_thread_id="thread-1",
            started_at=10.0,
            delegate_to_session_mirror=False,
            deps=fixture.build(),
        )

        self.assertEqual(len(channel.messages), 1)
        self.assertIn("Ask failed", channel.messages[0])
        self.assertIn("ERROR: Codex stream completed without a final answer.", channel.messages[0])

    async def test_handoff_suppresses_no_final_fallback(self) -> None:
        fixture = StreamResultFixture(handoff=True)
        channel = FakeChannel()

        await stream_result.handle_ask_stream_result(
            channel,
            exit_code=0,
            output="[delivery_verified] taxlab:1",
            relay=FakeRelay(sent_live=False, saw_final=False),
            target_thread_id="thread-1",
            started_at=10.0,
            delegate_to_session_mirror=False,
            deps=fixture.build(),
        )

        self.assertEqual(channel.messages, [])
        self.assertIn("ask_stream_suppressed_after_steering target=thread-1", "\n".join(fixture.logs))

    async def test_sent_live_abort_or_timeout_skips_extra_send(self) -> None:
        for relay in (
            FakeRelay(sent_live=True, saw_aborted=True),
            FakeRelay(sent_live=True, saw_timeout=True),
        ):
            fixture = StreamResultFixture()
            channel = FakeChannel()

            await stream_result.handle_ask_stream_result(
                channel,
                exit_code=1,
                output="partial",
                relay=relay,
                target_thread_id="thread-1",
                started_at=10.0,
                delegate_to_session_mirror=False,
                deps=fixture.build(),
            )

            self.assertEqual(channel.messages, [])

    async def test_nonzero_without_live_sends_failure(self) -> None:
        fixture = StreamResultFixture()
        channel = FakeChannel()

        await stream_result.handle_ask_stream_result(
            channel,
            exit_code=7,
            output="boom",
            relay=FakeRelay(sent_live=False),
            target_thread_id="thread-1",
            started_at=10.0,
            delegate_to_session_mirror=False,
            deps=fixture.build(),
        )

        self.assertEqual(channel.messages, ["Ask failed (exit 7)\n\nboom"])
