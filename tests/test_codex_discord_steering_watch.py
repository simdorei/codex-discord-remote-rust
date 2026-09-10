from __future__ import annotations

import unittest
from dataclasses import dataclass
from types import TracebackType

import codex_discord_steering_watch as steering_watch


@dataclass(frozen=True, slots=True)
class FakeWatchResult:
    session_path: str | None = "session.jsonl"
    start_offset: int | None = 0
    target_thread_id: str | None = "thread-1"
    target_ref: str | None = "project:1"
    delivery_pending: bool = False


@dataclass(frozen=True, slots=True)
class FakeRelay:
    sent_live: bool = False
    saw_final: bool = False
    saw_aborted: bool = False
    saw_timeout: bool = False
    suppressed_after_steering: bool = False


class FakeTyping:
    async def __aenter__(self) -> None:
        return None

    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        tb: TracebackType | None,
    ) -> None:
        _ = (exc_type, exc, tb)


class SteeringWatchTests(unittest.IsolatedAsyncioTestCase):
    def make_deps(
        self,
        relay: FakeRelay,
        *,
        exit_code: int = 0,
        output: str = "done",
    ) -> tuple[steering_watch.SteeringWatchDeps, list[str], list[str]]:
        sent: list[str] = []
        logs: list[str] = []

        def make_relay(
            loop: steering_watch.SteeringWatchLoop,
            channel: steering_watch.SteeringWatchChannel,
            target_thread_id: str,
            target_ref: str,
            *,
            started_at: float,
            send_commentary_blocks: bool | None,
            send_final_blocks: bool,
        ) -> FakeRelay:
            _ = (loop, channel, target_thread_id, target_ref, started_at, send_commentary_blocks, send_final_blocks)
            return relay

        def channel_typing(
            channel: steering_watch.SteeringWatchChannel,
            *,
            context: str,
        ) -> FakeTyping:
            _ = (channel, context)
            return FakeTyping()

        def run_watch_stream(
            watch_result: steering_watch.SteeringWatchResult,
            relay: steering_watch.SteeringWatchRelay,
            *,
            timeout_sec: float,
        ) -> tuple[int, str]:
            _ = (watch_result, relay, timeout_sec)
            return exit_code, output

        async def send_chunks(channel: steering_watch.SteeringWatchChannel, content: str) -> int:
            _ = channel
            sent.append(content)
            return 1

        deps = steering_watch.SteeringWatchDeps(
            monotonic=lambda: 100.0,
            make_relay=make_relay,
            get_watch_timeout=lambda: 3.0,
            channel_typing=channel_typing,
            run_watch_stream=run_watch_stream,
            send_chunks=send_chunks,
            log_line=logs.append,
            format_log_text_len=len,
        )
        return deps, sent, logs

    async def test_missing_session_returns_false_and_logs(self) -> None:
        deps, sent, logs = self.make_deps(FakeRelay())

        handled = await steering_watch.stream_steering_prompt_result_to_channel(
            "channel",
            FakeWatchResult(session_path=None),
            "thread-1",
            deps=deps,
        )

        self.assertFalse(handled)
        self.assertEqual(sent, [])
        self.assertEqual(logs, ["steer_watch_unavailable target=thread-1"])

    async def test_live_without_final_sends_fallback(self) -> None:
        deps, sent, logs = self.make_deps(FakeRelay(sent_live=True), output="captured")

        handled = await steering_watch.stream_steering_prompt_result_to_channel(
            "channel",
            FakeWatchResult(),
            "thread-1",
            deps=deps,
        )

        self.assertTrue(handled)
        self.assertEqual(sent, ["Steering finished\n\ncaptured"])
        self.assertTrue(any("steer_watch_no_final_fallback" in line for line in logs))

    async def test_timeout_without_live_sends_running_notice(self) -> None:
        deps, sent, logs = self.make_deps(FakeRelay(saw_timeout=True), exit_code=1, output="still running")

        handled = await steering_watch.stream_steering_prompt_result_to_channel(
            "channel",
            FakeWatchResult(delivery_pending=True),
            "thread-1",
            deps=deps,
        )

        self.assertTrue(handled)
        self.assertIn("Steering is still running in Codex.", sent[0])
        self.assertTrue(any("steer_watch_timeout_reported" in line for line in logs))

    async def test_delegated_public_output_returns_true_without_send(self) -> None:
        deps, sent, logs = self.make_deps(FakeRelay(sent_live=False, saw_final=True))

        handled = await steering_watch.stream_steering_prompt_result_to_channel(
            "channel",
            FakeWatchResult(),
            "thread-1",
            send_commentary_blocks=False,
            send_final_blocks=False,
            deps=deps,
        )

        self.assertTrue(handled)
        self.assertEqual(sent, [])
        self.assertTrue(any("steer_watch_public_output_delegated_to_session_mirror" in line for line in logs))


if __name__ == "__main__":
    _ = unittest.main()
