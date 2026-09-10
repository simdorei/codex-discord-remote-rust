from __future__ import annotations

from dataclasses import dataclass, field
from types import TracebackType
import unittest

import codex_discord_prompt_stream_attempt as stream_attempt


@dataclass(slots=True)
class FakeChannel:
    name: str = "channel"


@dataclass(slots=True)
class FakeRelay:
    sent_live: bool = False
    saw_final: bool = False
    saw_aborted: bool = False
    saw_timeout: bool = False


@dataclass(slots=True)
class FakeTypingContext:
    events: list[str]
    context: str

    async def __aenter__(self) -> None:
        self.events.append(f"enter:{self.context}")

    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        _ = exc_type, exc, traceback
        self.events.append(f"exit:{self.context}")


@dataclass(slots=True)
class StreamAttemptFixture:
    relay: FakeRelay = field(default_factory=FakeRelay)
    monotonic_value: float = 10.0
    relay_calls: list[tuple[str | None, str, float, bool]] = field(default_factory=list)
    run_calls: list[tuple[str, FakeRelay, str | None]] = field(default_factory=list)
    typing_events: list[str] = field(default_factory=list)
    logs: list[str] = field(default_factory=list)

    def monotonic(self) -> float:
        return self.monotonic_value

    def make_relay(
        self,
        channel: FakeChannel,
        *,
        target_thread_id: str | None,
        target_ref: str,
        started_at: float,
        delegate_to_session_mirror: bool,
    ) -> FakeRelay:
        _ = channel
        self.relay_calls.append((target_thread_id, target_ref, started_at, delegate_to_session_mirror))
        return self.relay

    def channel_typing(self, channel: FakeChannel, *, context: str) -> FakeTypingContext:
        _ = channel
        return FakeTypingContext(self.typing_events, context)

    async def run_stream(self, prompt: str, relay: FakeRelay, *, target_thread_id: str | None) -> tuple[int, str]:
        self.run_calls.append((prompt, relay, target_thread_id))
        return 7, "stream output"

    def build(self) -> stream_attempt.StreamAttemptDeps[FakeChannel, FakeRelay]:
        return stream_attempt.StreamAttemptDeps(
            monotonic=self.monotonic,
            make_relay=self.make_relay,
            channel_typing=self.channel_typing,
            run_ask_stream=self.run_stream,
            format_log_text_len=lambda output: len(output or ""),
            log=self.logs.append,
        )


class StreamAttemptTests(unittest.IsolatedAsyncioTestCase):
    async def test_stream_attempt_runs_inside_typing_and_logs_result(self) -> None:
        fixture = StreamAttemptFixture(FakeRelay(sent_live=True, saw_final=True), monotonic_value=15.0)
        channel = FakeChannel()

        result = await stream_attempt.run_stream_attempt(
            channel,
            prompt="please run",
            target_thread_id="thread-1",
            target_ref="taxlab:1",
            delegate_to_session_mirror=False,
            deps=fixture.build(),
        )

        self.assertEqual(fixture.typing_events, ["enter:ask_stream", "exit:ask_stream"])
        self.assertEqual(fixture.relay_calls, [("thread-1", "taxlab:1", 15.0, False)])
        self.assertEqual(fixture.run_calls, [("please run", fixture.relay, "thread-1")])
        self.assertIs(result.relay, fixture.relay)
        self.assertEqual((result.exit_code, result.output, result.started_at), (7, "stream output", 15.0))
        self.assertIn(
            "ask_stream_done exit=7 target=thread-1 sent_live=True final=True aborted=False timeout=False output_len=13",
            "\n".join(fixture.logs),
        )

    async def test_delegate_flag_is_forwarded_to_relay_factory(self) -> None:
        fixture = StreamAttemptFixture(monotonic_value=16.0)

        _ = await stream_attempt.run_stream_attempt(
            FakeChannel(),
            prompt="please run",
            target_thread_id="thread-1",
            target_ref="taxlab:1",
            delegate_to_session_mirror=True,
            deps=fixture.build(),
        )

        self.assertEqual(fixture.relay_calls, [("thread-1", "taxlab:1", 16.0, True)])

    async def test_missing_target_logs_dash(self) -> None:
        fixture = StreamAttemptFixture()

        _ = await stream_attempt.run_stream_attempt(
            FakeChannel(),
            prompt="please run",
            target_thread_id=None,
            target_ref="-",
            delegate_to_session_mirror=False,
            deps=fixture.build(),
        )

        self.assertIn("target=-", "\n".join(fixture.logs))
