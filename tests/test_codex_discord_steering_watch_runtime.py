from __future__ import annotations

import unittest
from types import TracebackType

import codex_discord_steering_watch as steering_watch
import codex_discord_steering_watch_runtime as steering_watch_runtime


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


class FakeRelay:
    @property
    def sent_live(self) -> bool:
        return False

    @property
    def saw_final(self) -> bool:
        return False

    @property
    def saw_aborted(self) -> bool:
        return False

    @property
    def saw_timeout(self) -> bool:
        return False

    @property
    def suppressed_after_steering(self) -> bool:
        return False


class SteeringWatchRuntimeTests(unittest.TestCase):
    def test_make_steering_watch_runtime_deps_preserves_callables(self) -> None:
        def make_relay(
            loop: steering_watch.SteeringWatchLoop,
            channel: steering_watch.SteeringWatchChannel,
            target_thread_id: str,
            target_ref: str,
            *,
            started_at: float,
            send_commentary_blocks: bool | None,
            send_final_blocks: bool,
        ) -> steering_watch.SteeringWatchRelay:
            _ = (loop, channel, target_thread_id, target_ref, started_at, send_commentary_blocks, send_final_blocks)
            return FakeRelay()

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
            return 0, "done"

        async def send_chunks(channel: steering_watch.SteeringWatchChannel, content: str) -> int:
            _ = (channel, content)
            return 1

        deps = steering_watch_runtime.make_steering_watch_runtime_deps(
            make_relay=make_relay,
            get_watch_timeout=lambda: 3.0,
            channel_typing=channel_typing,
            run_watch_stream=run_watch_stream,
            send_chunks=send_chunks,
            log_line=lambda line: None,
            format_log_text_len=len,
        )

        self.assertIs(deps.make_relay, make_relay)
        self.assertIs(deps.channel_typing, channel_typing)
        self.assertIs(deps.run_watch_stream, run_watch_stream)
        self.assertIs(deps.send_chunks, send_chunks)


if __name__ == "__main__":
    _ = unittest.main()
