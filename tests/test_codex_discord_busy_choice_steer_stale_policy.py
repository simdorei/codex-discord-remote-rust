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


class BusyChoiceSteerStalePolicyTests(unittest.TestCase):
    def test_stale_busy_notice_does_not_block_steering_delivery(self) -> None:
        asyncio.run(self._run_stale_busy_notice_does_not_block_steering_delivery())

    async def _run_stale_busy_notice_does_not_block_steering_delivery(self) -> None:
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
            return True

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
            _ = (channel, target_thread_id)
            raise AssertionError("mapped mirror path should be used")

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
            first_line = content.splitlines()[0] if content else ""
            events.append(f"followup:{title}:{exit_code}:{ephemeral}:{first_line}")

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

        times = iter([10.0, 11.0])

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
            time_monotonic=lambda: next(times),
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

        self.assertIn("stale:prompt:thread-1:steer_now", events)
        self.assertIn("run:prompt:thread-1", events)
        self.assertIn("stream:thread-1:False:False", events)
        self.assertNotIn(
            "followup:Steering:0:True:Steering was not sent because this Codex thread appears stuck. See the public channel notice.",
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
