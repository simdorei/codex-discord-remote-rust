from __future__ import annotations

from dataclasses import dataclass
from types import TracebackType
from typing import cast
import unittest

import codex_discord_approval_followup as approval_followup


@dataclass(frozen=True, slots=True)
class FakeWatchResult:
    target_thread_id: str | None = "thread-real"
    target_ref: str | None = "project:1"
    session_path: str | None = "session.jsonl"
    start_offset: int | None = 10


class FakeRelay:
    def __init__(
        self,
        *,
        sent_live: bool = False,
        saw_final: bool = False,
        saw_aborted: bool = False,
        saw_timeout: bool = False,
    ) -> None:
        self.sent_live = sent_live
        self.saw_final = saw_final
        self.saw_aborted = saw_aborted
        self.saw_timeout = saw_timeout


class FakeChannel:
    def __init__(self) -> None:
        self.messages: list[str] = []


class FakeTypingContext:
    def __init__(self) -> None:
        self.entered = False
        self.exited = False

    async def __aenter__(self) -> None:
        self.entered = True

    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        _ = (exc_type, exc, traceback)
        self.exited = True


class FollowupFixture:
    def __init__(
        self,
        *,
        relay: FakeRelay | None = None,
        watch_exit_code: int = 0,
        watch_output: str = "",
    ) -> None:
        self.relay = relay or FakeRelay()
        self.typing_context = FakeTypingContext()
        self.watch_exit_code = watch_exit_code
        self.watch_output = watch_output
        self.logs: list[str] = []
        self.watch_calls: list[tuple[str | None, str | None, float]] = []

    def make_relay(
        self,
        loop: approval_followup.ApprovalFollowupLoop,
        channel: approval_followup.ApprovalFollowupChannel,
        target_thread_id: str,
        target_ref: str,
    ) -> FakeRelay:
        _ = (loop, channel)
        self.watch_calls.append((target_thread_id, target_ref, -1.0))
        return self.relay

    def channel_typing(
        self,
        channel: approval_followup.ApprovalFollowupChannel,
        *,
        context: str,
    ) -> FakeTypingContext:
        _ = (channel, context)
        return self.typing_context

    def run_watch_stream(
        self,
        watch_result: approval_followup.ApprovalFollowupWatchResult,
        relay: approval_followup.ApprovalFollowupRelay,
        *,
        timeout_sec: float,
    ) -> tuple[int, str]:
        self.watch_calls.append((watch_result.target_thread_id, watch_result.target_ref, timeout_sec))
        self.relay.sent_live = relay.sent_live
        return self.watch_exit_code, self.watch_output

    async def send_chunks(self, channel: approval_followup.ApprovalFollowupChannel, content: str) -> int:
        cast(FakeChannel, channel).messages.append(content)
        return len(content)

    def build(self) -> approval_followup.ApprovalFollowupDeps:
        return approval_followup.ApprovalFollowupDeps(
            make_relay=self.make_relay,
            get_watch_timeout=lambda: 7.0,
            channel_typing=self.channel_typing,
            run_watch_stream=self.run_watch_stream,
            send_chunks=self.send_chunks,
            log_line=self.logs.append,
            format_log_text_len=lambda text: len(text or ""),
        )


class ApprovalFollowupTests(unittest.IsolatedAsyncioTestCase):
    async def test_returns_false_when_watch_result_missing_or_has_no_session(self) -> None:
        fixture = FollowupFixture()
        channel = FakeChannel()

        self.assertFalse(
            await approval_followup.stream_post_approval_result_to_channel(
                channel,
                None,
                "thread-request",
                deps=fixture.build(),
            )
        )
        self.assertFalse(
            await approval_followup.stream_post_approval_result_to_channel(
                channel,
                FakeWatchResult(session_path=None),
                "thread-request",
                deps=fixture.build(),
            )
        )

        self.assertEqual(channel.messages, [])
        self.assertEqual(fixture.logs, ["approval_followup_watch_unavailable target=thread-request reason=no_session"])

    async def test_sent_live_without_final_sends_no_final_fallback(self) -> None:
        fixture = FollowupFixture(relay=FakeRelay(sent_live=True), watch_output="")
        channel = FakeChannel()

        handled = await approval_followup.stream_post_approval_result_to_channel(
            channel,
            cast(approval_followup.ApprovalFollowupWatchResult, FakeWatchResult()),
            "thread-request",
            deps=fixture.build(),
        )

        self.assertTrue(handled)
        self.assertTrue(fixture.typing_context.entered)
        self.assertTrue(fixture.typing_context.exited)
        self.assertEqual(
            channel.messages,
            ["Approval follow-up finished\n\n(no final answer captured)"],
        )
        self.assertEqual(
            fixture.watch_calls,
            [("thread-real", "project:1", -1.0), ("thread-real", "project:1", 7.0)],
        )
        self.assertIn(
            "approval_followup_watch_done exit=0 target=thread-request "
            "sent_live=True final=False aborted=False timeout=False output_len=0",
            fixture.logs,
        )
        self.assertIn(
            "approval_followup_watch_no_final_fallback target=thread-request output_len=0",
            fixture.logs,
        )


if __name__ == "__main__":
    _ = unittest.main()
