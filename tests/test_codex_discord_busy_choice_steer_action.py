from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import AsyncGenerator
from contextlib import asynccontextmanager
import unittest

import codex_discord_busy_choice_steer_action as steer_action
import codex_discord_busy_choice_steer_failure as steer_failure
import codex_discord_busy_choice_steer_result as steer_result
from codex_discord_steering import SteeringPromptResult


class FakeInteraction:
    pass


class FakeChannel:
    pass


class BusyChoiceSteerActionExportTests(unittest.TestCase):
    def test_steer_action_module_exports_handler_and_deps(self) -> None:
        self.assertTrue(hasattr(steer_action, "BusyChoiceSteerActionDeps"))
        self.assertTrue(hasattr(steer_action, "handle_busy_choice_steer_action"))

    def test_success_uses_mapped_session_mirror_and_marks_handoff(self) -> None:
        asyncio.run(self._run_success_uses_mapped_session_mirror_and_marks_handoff())

    async def _run_success_uses_mapped_session_mirror_and_marks_handoff(self) -> None:
        events: list[str] = []
        interaction = FakeInteraction()
        channel = FakeChannel()

        async def send_stale_block_message(
            channel: steer_action.BusyChoiceChannel,
            prompt: str,
            target_thread_id: str | None,
            *,
            reason: str,
        ) -> bool:
            _ = channel
            events.append(f"stale:{prompt}:{target_thread_id}:{reason}")
            return False

        async def prepare_mapped_session_mirror_output(
            channel: steer_action.BusyChoiceChannel,
            target_thread_id: str | None,
        ) -> bool:
            _ = channel
            events.append(f"mapped:{target_thread_id}")
            return True

        async def prepare_session_mirror_delegation(
            channel: steer_action.BusyChoiceChannel,
            target_thread_id: str | None,
        ) -> bool:
            _ = channel
            events.append(f"fallback:{target_thread_id}")
            return False

        async def send_steering_start_ack(
            channel: steer_action.BusyChoiceChannel,
            prompt: str,
            target_thread_id: str | None,
        ) -> None:
            _ = channel
            events.append(f"ack:{prompt}:{target_thread_id}")

        async def send_followup_chunks(
            interaction: steer_action.BusyChoiceInteraction,
            content: str,
            *,
            title: str,
            exit_code: int,
            log_prefix: str,
            ephemeral: bool,
        ) -> None:
            _ = (interaction, log_prefix)
            events.append(f"followup:{title}:{exit_code}:{ephemeral}:{content.splitlines()[0]}")

        @asynccontextmanager
        async def channel_typing(
            channel: steer_action.BusyChoiceChannel,
            *,
            context: str,
        ) -> AsyncGenerator[None]:
            _ = channel
            events.append(f"typing:start:{context}")
            yield
            events.append(f"typing:stop:{context}")

        def run_steering_prompt(prompt: str, target_thread_id: str | None) -> SteeringPromptResult:
            events.append(f"run:{prompt}:{target_thread_id}")
            return SteeringPromptResult(0, "sent", target_thread_id=target_thread_id)

        def mark_steering_handoff(target_thread_id: str | None) -> float:
            events.append(f"handoff:{target_thread_id}")
            return 123.0

        times = iter([10.0, 12.5])

        def time_monotonic() -> float:
            return next(times)

        async def steering_streamer(
            channel: steer_result.BusyChoiceChannel,
            steering_result: SteeringPromptResult,
            target_thread_id: str | None,
            *,
            send_commentary_blocks: bool | None,
            send_final_blocks: bool,
        ) -> bool:
            _ = (channel, steering_result)
            events.append(f"stream:{target_thread_id}:{send_commentary_blocks}:{send_final_blocks}")
            return True

        deps = steer_action.BusyChoiceSteerActionDeps(
            send_stale_block_message=send_stale_block_message,
            prepare_mapped_session_mirror_output=prepare_mapped_session_mirror_output,
            prepare_session_mirror_delegation=prepare_session_mirror_delegation,
            send_steering_start_ack=send_steering_start_ack,
            send_followup_chunks=send_followup_chunks,
            channel_typing=channel_typing,
            run_steering_prompt=run_steering_prompt,
            mark_steering_handoff=mark_steering_handoff,
            format_log_text_len=lambda text: len(str(text or "")),
            log=events.append,
            time_monotonic=time_monotonic,
            steer_failure_deps=_unused_failure_deps(),
            steer_result_deps=steer_result.BusyChoiceSteerResultDeps(
                send_followup_chunks=send_followup_chunks,
                steering_streamer=steering_streamer,
                log=events.append,
            ),
        )

        await steer_action.handle_busy_choice_steer_action(
            interaction,
            channel,
            "prompt",
            "thread-1",
            user_id=42,
            deps=deps,
        )

        self.assertEqual(
            [
                "steer_now user=42 target=thread-1 prompt_len=6",
                "stale:prompt:thread-1:steer_now",
                "mapped:thread-1",
                "ack:prompt:thread-1",
                "typing:start:steer_now",
                "run:prompt:thread-1",
                "typing:stop:steer_now",
                "handoff:thread-1",
                "steer_now_done exit=0 target=thread-1 elapsed_sec=2.50 output_len=4",
                "followup:Steering:0:True:Steering sent",
                "steer_now_sent exit=0 target=thread-1",
                "steer_now_delegated_to_session_mirror target=thread-1",
                "stream:thread-1:False:False",
            ],
            events,
        )


def _unused_failure_deps() -> steer_failure.BusyChoiceSteerFailureDeps:
    async def send_codex_app_menu_if_available(
        channel: steer_failure.BusyChoiceChannel,
        target_thread_id: str | None,
        output: str,
        *,
        reason: str,
    ) -> bool:
        _ = (channel, target_thread_id, output, reason)
        raise AssertionError("failure deps should not be used")

    async def send_stale_block_message(
        channel: steer_failure.BusyChoiceChannel,
        prompt: str,
        target_thread_id: str | None,
        *,
        reason: str,
    ) -> bool:
        _ = (channel, prompt, target_thread_id, reason)
        raise AssertionError("failure deps should not be used")

    async def send_followup_chunks(
        interaction: steer_failure.BusyChoiceInteraction,
        content: str,
        *,
        title: str,
        exit_code: int,
        log_prefix: str,
        ephemeral: bool,
    ) -> None:
        _ = (interaction, content, title, exit_code, log_prefix, ephemeral)
        raise AssertionError("failure deps should not be used")

    def ignore_log(message: str) -> None:
        _ = message

    return steer_failure.BusyChoiceSteerFailureDeps(
        send_codex_app_menu_if_available=send_codex_app_menu_if_available,
        send_stale_block_message=send_stale_block_message,
        send_followup_chunks=send_followup_chunks,
        resolve_target_ref=lambda target_thread_id: (target_thread_id, target_thread_id or "-"),
        build_not_accepted_message=lambda target_ref: target_ref,
        log=ignore_log,
    )
