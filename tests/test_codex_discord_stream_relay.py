from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from pathlib import Path
from typing import Protocol, cast
import unittest

from codex_discord_steering import NativeExactWatchTarget, SteeringPromptResult
import codex_discord_stream as stream
import codex_discord_stream_relay as stream_relay
import codex_discord_stream_relay_types as stream_relay_types


class _ChannelLike(Protocol):
    pass


class _InteractiveState(Protocol):
    pass


class _InteractiveOptions(Protocol):
    pass


class _RelayProbe:
    def __init__(self) -> None:
        self.lines: list[str] = []
        self.finish_count: int = 0

    def feed_line(self, line: str) -> None:
        self.lines.append(line)

    def finish(self) -> None:
        self.finish_count += 1


class WatchFailedError(Exception):
    pass


class _RelayDeps:
    def __init__(self) -> None:
        self.sent: list[str] = []
        self.prompts: list[
            tuple[_ChannelLike, str, str, _InteractiveState, str, _InteractiveOptions]
        ] = []
        self.logs: list[str] = []
        self.stale: bool = False
        self.handoff: bool = False
        self.generation: int = 7

    async def send_chunks(self, _channel: _ChannelLike, text: str) -> None:
        self.sent.append(text)

    def parse_interactive_notice(
        self,
        text: str,
    ) -> tuple[str | None, str, stream_relay.InteractiveNoticeOptions]:
        if text == "interactive":
            return "state", "prompt", [("yes", "yes"), ("no", "no")]
        return None, "", []

    async def send_interactive_prompt(
        self,
        channel: _ChannelLike,
        target_thread_id: str,
        target_ref: str,
        state: _InteractiveState,
        prompt: str,
        options: _InteractiveOptions,
    ) -> None:
        self.prompts.append((channel, target_thread_id, target_ref, state, prompt, options))

    def register_discord_relay(self, _target_thread_id: str | None) -> int:
        return self.generation

    def is_discord_relay_stale(self, _target_thread_id: str | None, generation: int) -> bool:
        return self.stale or generation != self.generation

    def had_steering_handoff_since(self, _target_thread_id: str | None, _since: float) -> bool:
        return self.handoff

    def log(self, line: str) -> None:
        self.logs.append(line)

    def format_log_text_len(self, text: str) -> str:
        return str(len(text))


class DiscordAskRelayTests(unittest.IsolatedAsyncioTestCase):
    def test_facade_reexports_interactive_notice_options(self) -> None:
        self.assertIs(
            stream_relay.InteractiveNoticeOptions,
            stream_relay_types.InteractiveNoticeOptions,
        )

    def _relay(
        self,
        deps: _RelayDeps,
        *,
        quiet_notice_delay_sec: float = -1.0,
        send_commentary_blocks: bool = True,
        suppress_after_steering_since: float | None = None,
    ) -> stream.DiscordAskRelay:
        return stream.DiscordAskRelay(
            asyncio.get_running_loop(),
            object(),
            "thread-1",
            "project:1",
            quiet_notice_delay_sec=quiet_notice_delay_sec,
            suppress_after_steering_since=suppress_after_steering_since,
            send_commentary_blocks=send_commentary_blocks,
            send_chunks_func=deps.send_chunks,
            parse_interactive_notice_func=deps.parse_interactive_notice,
            send_interactive_prompt_func=deps.send_interactive_prompt,
            register_discord_relay_func=deps.register_discord_relay,
            is_discord_relay_stale_func=deps.is_discord_relay_stale,
            had_steering_handoff_since_func=deps.had_steering_handoff_since,
            log_func=deps.log,
            format_log_text_len_func=deps.format_log_text_len,
        )

    async def test_commentary_and_final_send_in_order(self) -> None:
        deps = _RelayDeps()
        relay = self._relay(deps)

        relay.feed_line("[commentary]")
        relay.feed_line("checking")
        relay.feed_line("[final_answer]")
        relay.feed_line("done")
        await asyncio.to_thread(relay.finish)

        self.assertEqual(deps.sent, ["In progress\n\nchecking", "Final\n\ndone"])
        self.assertTrue(relay.sent_live)
        self.assertTrue(relay.saw_final)

    async def test_quiet_notice_sends_after_waiting_marker(self) -> None:
        deps = _RelayDeps()
        relay = self._relay(deps, quiet_notice_delay_sec=0.01)

        relay.feed_line("[waiting_for_final_answer]")
        await asyncio.sleep(0.05)
        await asyncio.to_thread(relay.finish)

        self.assertEqual(len(deps.sent), 1)
        self.assertIn("Codex is still working.", deps.sent[0])
        self.assertTrue(relay.quiet_notice_sent)
        self.assertFalse(relay.sent_live)

    async def test_quiet_notice_cancels_after_final(self) -> None:
        deps = _RelayDeps()
        relay = self._relay(deps, quiet_notice_delay_sec=0.05)

        relay.feed_line("[waiting_for_final_answer]")
        relay.feed_line("[final_answer]")
        relay.feed_line("done")
        await asyncio.to_thread(relay.finish)
        await asyncio.sleep(0.08)

        self.assertEqual(deps.sent, ["Final\n\ndone"])
        self.assertFalse(relay.quiet_notice_sent)

    async def test_stale_final_is_suppressed_and_logged(self) -> None:
        deps = _RelayDeps()
        deps.stale = True
        relay = self._relay(deps)

        relay.feed_line("[final_answer]")
        relay.feed_line("done")
        await asyncio.to_thread(relay.finish)

        self.assertEqual(deps.sent, [])
        self.assertTrue(relay.suppressed_after_steering)
        self.assertTrue(any("discord_relay_suppressed_after_steering" in line for line in deps.logs))

    async def test_interactive_notice_delegates_to_prompt_sender(self) -> None:
        deps = _RelayDeps()
        relay = self._relay(deps)

        relay.feed_line("[final_answer]")
        relay.feed_line("interactive")
        await asyncio.to_thread(relay.finish)

        self.assertEqual(deps.sent, [])
        self.assertEqual(len(deps.prompts), 1)
        self.assertEqual(
            deps.prompts[0][1:],
            ("thread-1", "project:1", "state", "prompt", [("yes", "yes"), ("no", "no")]),
        )
        self.assertTrue(relay.sent_live)

    async def test_aborted_marker_sends_aborted_notice(self) -> None:
        deps = _RelayDeps()
        relay = self._relay(deps)

        relay.feed_line("[aborted]")
        await asyncio.to_thread(relay.finish)

        self.assertEqual(deps.sent, ["Aborted."])
        self.assertTrue(relay.saw_aborted)
        self.assertTrue(relay.sent_live)

    def test_steering_watch_stream_finishes_relay_when_watch_raises(self) -> None:
        relay = _RelayProbe()
        steering_result = SteeringPromptResult(
            0,
            "",
            session_path="session.jsonl",
            start_offset=7,
        )

        def fail_watch(
            *,
            session_path: Path,
            start_offset: int,
            timeout_sec: float,
            include_commentary: bool,
            stream_live: bool = False,
            stream_label: str = "",
            stream_callback: stream.LineStreamFunc | None = None,
            expected_turn_id: str | None = None,
        ) -> stream.WatchForFinalAnswerResult:
            _ = (
                session_path,
                start_offset,
                timeout_sec,
                include_commentary,
                stream_live,
                stream_label,
                stream_callback,
                expected_turn_id,
            )
            raise WatchFailedError("watch failed")

        with self.assertRaisesRegex(WatchFailedError, "watch failed"):
            _ = stream.run_steering_watch_stream(
                steering_result,
                cast(stream.DiscordAskRelay, cast(object, relay)),
                watch_for_final_answer_func=fail_watch,
            )

        self.assertEqual(relay.finish_count, 1)
        self.assertEqual(
            relay.lines,
            [
                "[waiting_for_final_answer]",
                "Use Ctrl+C to stop waiting after the prompt is sent.",
            ],
        )

    def test_native_watch_target_passes_exact_turn_id(self) -> None:
        relay = _RelayProbe()
        captured_turn_ids: list[str | None] = []
        steering_result = SteeringPromptResult(
            0,
            "",
            session_path="session.jsonl",
            start_offset=7,
            watch_target=NativeExactWatchTarget("turn-42"),
        )

        def watch(**kwargs: object) -> stream.WatchForFinalAnswerResult:
            captured_turn_ids.append(cast(str | None, kwargs.get("expected_turn_id")))
            return {
                "status": "aborted",
                "commentary": [],
                "final_answer": "",
                "streamed_live": False,
                "final_streamed_live": False,
            }

        exit_code, _output = stream.run_steering_watch_stream(
            steering_result,
            cast(stream.DiscordAskRelay, cast(object, relay)),
            watch_for_final_answer_func=watch,
        )

        self.assertEqual(exit_code, 0)
        self.assertEqual(captured_turn_ids, ["turn-42"])
        self.assertIn("[aborted]", relay.lines)

    def test_failed_native_watch_uses_failure_marker_and_nonzero_exit(self) -> None:
        relay = _RelayProbe()
        steering_result = SteeringPromptResult(0, "", session_path="session.jsonl", start_offset=7)

        def watch(**kwargs: object) -> stream.WatchForFinalAnswerResult:
            _ = kwargs
            return {
                "status": "failed",
                "commentary": [],
                "final_answer": "",
                "streamed_live": False,
                "final_streamed_live": False,
                "error_message": "worker crashed",
            }

        exit_code, output = stream.run_steering_watch_stream(
            steering_result,
            cast(stream.DiscordAskRelay, cast(object, relay)),
            watch_for_final_answer_func=watch,
        )

        self.assertEqual(exit_code, 1)
        self.assertIn("[failed]", relay.lines)
        self.assertIn("worker crashed", output)


if __name__ == "__main__":
    _ = unittest.main()
