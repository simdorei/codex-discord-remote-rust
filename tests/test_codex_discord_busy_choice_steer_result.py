from __future__ import annotations

import unittest

import codex_discord_busy_choice_steer_result as busy_steer_result
from codex_discord_steering import SteeringPromptResult


class FollowupFailure(RuntimeError): ...


class StreamFailure(RuntimeError): ...


class FakeInteraction: ...


class FakeChannel: ...


FollowupCall = tuple[str, str, int, str, bool]
StreamCall = tuple[busy_steer_result.BusyChoiceChannel, SteeringPromptResult, str | None, bool | None, bool]


class Recorder:
    def __init__(self) -> None:
        self.followups: list[FollowupCall] = []
        self.streams: list[StreamCall] = []
        self.logs: list[str] = []
        self.fail_followup: bool = False
        self.fail_stream: bool = False

    async def send_followup_chunks(
        self,
        interaction: busy_steer_result.BusyChoiceInteraction,
        content: str,
        *,
        title: str,
        exit_code: int,
        log_prefix: str,
        ephemeral: bool,
    ) -> None:
        _ = interaction
        if self.fail_followup:
            raise FollowupFailure("followup failed")
        self.followups.append((content, title, exit_code, log_prefix, ephemeral))

    async def stream_result(
        self,
        channel: busy_steer_result.BusyChoiceChannel,
        steering_result: SteeringPromptResult,
        target_thread_id: str | None,
        *,
        send_commentary_blocks: bool | None,
        send_final_blocks: bool,
    ) -> bool:
        if self.fail_stream:
            raise StreamFailure("stream failed")
        self.streams.append((channel, steering_result, target_thread_id, send_commentary_blocks, send_final_blocks))
        return True

    def deps(self) -> busy_steer_result.BusyChoiceSteerResultDeps:
        return busy_steer_result.BusyChoiceSteerResultDeps(
            send_followup_chunks=self.send_followup_chunks,
            steering_streamer=self.stream_result,
            log=self.logs.append,
        )


class BusyChoiceSteerResultTests(unittest.IsolatedAsyncioTestCase):
    async def test_success_streams_with_default_flags_and_logs_sent(self) -> None:
        recorder = Recorder()
        interaction = FakeInteraction()
        channel = FakeChannel()
        result = SteeringPromptResult(0, "sent", target_thread_id="thread-1")

        await busy_steer_result.handle_busy_choice_steer_result(
            interaction,
            channel,
            result,
            "thread-1",
            delegate_to_session_mirror=False,
            deps=recorder.deps(),
        )

        self.assertEqual(recorder.followups, [("Steering sent\n\nsent", "Steering", 0, "button_response", True)])
        self.assertEqual(recorder.streams, [(channel, result, "thread-1", None, True)])
        self.assertEqual(recorder.logs, ["steer_now_sent exit=0 target=thread-1"])

    async def test_delegated_success_logs_and_suppresses_public_blocks(self) -> None:
        recorder = Recorder()
        channel = FakeChannel()
        result = SteeringPromptResult(0, "sent", target_thread_id="thread-1")

        await busy_steer_result.handle_busy_choice_steer_result(
            FakeInteraction(),
            channel,
            result,
            "thread-1",
            delegate_to_session_mirror=True,
            deps=recorder.deps(),
        )

        self.assertEqual(recorder.followups, [("Steering sent\n\nsent", "Steering", 0, "button_response", True)])
        self.assertEqual(recorder.streams, [(channel, result, "thread-1", False, False)])
        self.assertEqual(
            recorder.logs,
            [
                "steer_now_sent exit=0 target=thread-1",
                "steer_now_delegated_to_session_mirror target=thread-1",
            ],
        )

    async def test_failure_sends_failed_followup_and_does_not_stream(self) -> None:
        recorder = Recorder()

        await busy_steer_result.handle_busy_choice_steer_result(
            FakeInteraction(),
            FakeChannel(),
            SteeringPromptResult(7, "failed"),
            "thread-1",
            delegate_to_session_mirror=False,
            deps=recorder.deps(),
        )

        self.assertEqual(
            recorder.followups,
            [("Steering failed (exit 7)\n\nfailed", "Steering", 7, "button_response", True)],
        )
        self.assertEqual(recorder.streams, [])
        self.assertEqual(recorder.logs, ["steer_now_sent exit=7 target=thread-1"])

    async def test_empty_output_uses_no_output_fallback(self) -> None:
        recorder = Recorder()

        await busy_steer_result.handle_busy_choice_steer_result(
            FakeInteraction(),
            FakeChannel(),
            SteeringPromptResult(9, ""),
            "thread-1",
            delegate_to_session_mirror=False,
            deps=recorder.deps(),
        )

        self.assertEqual(
            recorder.followups,
            [("Steering failed (exit 9)\n\n(no output)", "Steering", 9, "button_response", True)],
        )

    async def test_none_target_logs_dash_and_forwards_none_to_streamer(self) -> None:
        recorder = Recorder()
        channel = FakeChannel()
        result = SteeringPromptResult(0, "sent")

        await busy_steer_result.handle_busy_choice_steer_result(
            FakeInteraction(),
            channel,
            result,
            None,
            delegate_to_session_mirror=True,
            deps=recorder.deps(),
        )

        self.assertEqual(recorder.streams, [(channel, result, None, False, False)])
        self.assertEqual(
            recorder.logs,
            ["steer_now_sent exit=0 target=-", "steer_now_delegated_to_session_mirror target=-"],
        )

    async def test_followup_exception_propagates_without_logging_or_streaming(self) -> None:
        recorder = Recorder()
        recorder.fail_followup = True

        with self.assertRaises(FollowupFailure):
            await busy_steer_result.handle_busy_choice_steer_result(
                FakeInteraction(),
                FakeChannel(),
                SteeringPromptResult(0, "sent"),
                "thread-1",
                delegate_to_session_mirror=False,
                deps=recorder.deps(),
            )

        self.assertEqual(recorder.logs, [])
        self.assertEqual(recorder.streams, [])

    async def test_stream_exception_propagates_after_sent_log(self) -> None:
        recorder = Recorder()
        recorder.fail_stream = True

        with self.assertRaises(StreamFailure):
            await busy_steer_result.handle_busy_choice_steer_result(
                FakeInteraction(),
                FakeChannel(),
                SteeringPromptResult(0, "sent"),
                "thread-1",
                delegate_to_session_mirror=False,
                deps=recorder.deps(),
            )

        self.assertEqual(recorder.logs, ["steer_now_sent exit=0 target=thread-1"])
