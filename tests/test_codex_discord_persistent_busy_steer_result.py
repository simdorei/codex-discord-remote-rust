from __future__ import annotations

import unittest

import codex_discord_persistent_busy_steer as persistent_busy_steer
from codex_discord_steering import SteeringPromptResult


FollowupCall = tuple[str, str, int, str, bool]
StreamCall = tuple[object, SteeringPromptResult, str | None, bool | None, bool]


class PersistentBusySteerResultTests(unittest.IsolatedAsyncioTestCase):
    async def test_result_success_streams_with_default_flags(self) -> None:
        followups: list[FollowupCall] = []
        streams: list[StreamCall] = []
        logs: list[str] = []
        interaction = object()
        channel = object()
        result = SteeringPromptResult(0, "sent", target_thread_id="thread-1")

        async def send_followup_chunks(
            interaction: persistent_busy_steer.PersistentBusyInteraction,
            content: str,
            *,
            title: str,
            exit_code: int,
            log_prefix: str,
            ephemeral: bool,
        ) -> None:
            _ = interaction
            followups.append((content, title, exit_code, log_prefix, ephemeral))

        async def steering_streamer(
            channel: object,
            steering_result: SteeringPromptResult,
            target_thread_id: str | None,
            *,
            send_commentary_blocks: bool | None,
            send_final_blocks: bool,
        ) -> None:
            streams.append((channel, steering_result, target_thread_id, send_commentary_blocks, send_final_blocks))

        handled = await persistent_busy_steer.handle_persistent_busy_steer_result(
            interaction,
            channel,
            result,
            "thread-1",
            delegate_to_session_mirror=False,
            deps=persistent_busy_steer.PersistentBusySteerResultDeps(
                send_followup_chunks=send_followup_chunks,
                steering_streamer=steering_streamer,
                log=logs.append,
            ),
        )

        self.assertTrue(handled)
        self.assertEqual(followups, [("Steering sent\n\nsent", "Steering", 0, "button_response", True)])
        self.assertEqual(streams, [(channel, result, "thread-1", None, True)])
        self.assertEqual(logs, [])

    async def test_result_failure_does_not_stream_and_uses_no_output(self) -> None:
        followups: list[FollowupCall] = []
        streams: list[StreamCall] = []
        logs: list[str] = []

        async def send_followup_chunks(
            interaction: persistent_busy_steer.PersistentBusyInteraction,
            content: str,
            *,
            title: str,
            exit_code: int,
            log_prefix: str,
            ephemeral: bool,
        ) -> None:
            _ = interaction
            followups.append((content, title, exit_code, log_prefix, ephemeral))

        async def steering_streamer(
            channel: object,
            steering_result: SteeringPromptResult,
            target_thread_id: str | None,
            *,
            send_commentary_blocks: bool | None,
            send_final_blocks: bool,
        ) -> None:
            streams.append((channel, steering_result, target_thread_id, send_commentary_blocks, send_final_blocks))

        handled = await persistent_busy_steer.handle_persistent_busy_steer_result(
            object(),
            object(),
            SteeringPromptResult(7, ""),
            "thread-1",
            delegate_to_session_mirror=False,
            deps=persistent_busy_steer.PersistentBusySteerResultDeps(
                send_followup_chunks=send_followup_chunks,
                steering_streamer=steering_streamer,
                log=logs.append,
            ),
        )

        self.assertTrue(handled)
        self.assertEqual(followups, [("Steering failed (exit 7)\n\n(no output)", "Steering", 7, "button_response", True)])
        self.assertEqual(streams, [])
        self.assertEqual(logs, [])

    async def test_result_delegated_success_logs_and_suppresses_final_blocks_for_missing_target(self) -> None:
        followups: list[FollowupCall] = []
        streams: list[StreamCall] = []
        logs: list[str] = []
        channel = object()
        result = SteeringPromptResult(0, "sent")

        async def send_followup_chunks(
            interaction: persistent_busy_steer.PersistentBusyInteraction,
            content: str,
            *,
            title: str,
            exit_code: int,
            log_prefix: str,
            ephemeral: bool,
        ) -> None:
            _ = interaction
            followups.append((content, title, exit_code, log_prefix, ephemeral))

        async def steering_streamer(
            channel: object,
            steering_result: SteeringPromptResult,
            target_thread_id: str | None,
            *,
            send_commentary_blocks: bool | None,
            send_final_blocks: bool,
        ) -> None:
            streams.append((channel, steering_result, target_thread_id, send_commentary_blocks, send_final_blocks))

        handled = await persistent_busy_steer.handle_persistent_busy_steer_result(
            object(),
            channel,
            result,
            None,
            delegate_to_session_mirror=True,
            deps=persistent_busy_steer.PersistentBusySteerResultDeps(
                send_followup_chunks=send_followup_chunks,
                steering_streamer=steering_streamer,
                log=logs.append,
            ),
        )

        self.assertTrue(handled)
        self.assertEqual(followups, [("Steering sent\n\nsent", "Steering", 0, "button_response", True)])
        self.assertEqual(streams, [(channel, result, None, False, False)])
        self.assertEqual(logs, ["busy_choice_persistent_steer_delegated_to_session_mirror target=-"])
