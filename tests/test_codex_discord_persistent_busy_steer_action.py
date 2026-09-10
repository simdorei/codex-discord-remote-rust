from __future__ import annotations

import unittest
from contextlib import AbstractAsyncContextManager
from dataclasses import dataclass
from types import TracebackType
from typing import override

import codex_discord_persistent_busy_steer as persistent_busy_steer
import codex_discord_persistent_busy_steer_action as steer_action
from codex_discord_steering import SteeringPromptResult

USER_ID = 242286902982606848
CHOICE_ID = "0123456789abcdef01234567"
PROMPT = "please steer"
TARGET_THREAD_ID = "thread-1"


@dataclass(frozen=True, slots=True)
class FakeInteraction:
    marker: str = "interaction"


@dataclass(frozen=True, slots=True)
class FakeChannel:
    channel_id: int = 222


class NullTypingContext(AbstractAsyncContextManager[None]):
    @override
    async def __aenter__(self) -> None:
        return None

    @override
    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc_value: BaseException | None,
        traceback: TracebackType | None,
    ) -> bool:
        _ = (exc_type, exc_value, traceback)
        return False


@dataclass(frozen=True, slots=True)
class SteerHarness:
    deps: steer_action.PersistentBusySteerActionDeps
    events: list[tuple[str, str | None, str]]
    logs: list[str]


class PersistentBusySteerActionTests(unittest.IsolatedAsyncioTestCase):
    async def test_success_path_runs_in_order_and_streams_result(self) -> None:
        harness = self._make_harness(session_delegate=True)

        handled = await self._run_action(harness)

        self.assertTrue(handled)
        self.assertEqual(
            harness.events,
            [
                ("stale", TARGET_THREAD_ID, "persistent_steer_now"),
                ("prepare", TARGET_THREAD_ID, "-"),
                ("ack", TARGET_THREAD_ID, PROMPT),
                ("run", TARGET_THREAD_ID, CHOICE_ID),
                ("busy_check", TARGET_THREAD_ID, "ok"),
                ("result", TARGET_THREAD_ID, "delegate=True"),
            ],
        )
        self.assertEqual(
            harness.logs,
            [f"busy_choice_persistent_steer user={USER_ID} choice={CHOICE_ID} target={TARGET_THREAD_ID} prompt_len=12"],
        )

    async def test_stale_block_returns_before_ack_or_run(self) -> None:
        harness = self._make_harness(stale_blocked=True)

        handled = await self._run_action(harness)

        self.assertTrue(handled)
        self.assertEqual(harness.events, [("stale", TARGET_THREAD_ID, "persistent_steer_now")])

    async def test_busy_failure_routes_output_and_skips_result(self) -> None:
        harness = self._make_harness(busy_error=True, run_result=SteeringPromptResult(1, "busy now"))

        handled = await self._run_action(harness)

        self.assertTrue(handled)
        self.assertIn(("busy_failure", TARGET_THREAD_ID, "busy now"), harness.events)
        self.assertNotIn(("result", TARGET_THREAD_ID, "delegate=False"), harness.events)

    async def test_run_exception_propagates(self) -> None:
        harness = self._make_harness(run_error=RuntimeError("steer failed"))

        with self.assertRaisesRegex(RuntimeError, "steer failed"):
            _ = await self._run_action(harness)

        self.assertNotIn(("busy_check", TARGET_THREAD_ID, "ok"), harness.events)
        self.assertNotIn(("result", TARGET_THREAD_ID, "delegate=False"), harness.events)

    async def test_none_target_is_forwarded_and_logged_as_dash(self) -> None:
        harness = self._make_harness()

        handled = await self._run_action(harness, target_thread_id=None)

        self.assertTrue(handled)
        self.assertIn(("prepare", None, "-"), harness.events)
        self.assertIn(("run", None, CHOICE_ID), harness.events)
        self.assertEqual(
            harness.logs,
            [f"busy_choice_persistent_steer user={USER_ID} choice={CHOICE_ID} target=- prompt_len=12"],
        )

    async def _run_action(
        self,
        harness: SteerHarness,
        *,
        target_thread_id: str | None = TARGET_THREAD_ID,
    ) -> bool:
        return await steer_action.handle_persistent_busy_steer_action(
            FakeInteraction(),
            FakeChannel(),
            user_id=USER_ID,
            choice_id=CHOICE_ID,
            target_thread_id=target_thread_id,
            prompt=PROMPT,
            deps=harness.deps,
        )

    def _make_harness(
        self,
        *,
        stale_blocked: bool = False,
        session_delegate: bool = False,
        busy_error: bool = False,
        run_result: SteeringPromptResult | None = None,
        run_error: RuntimeError | None = None,
    ) -> SteerHarness:
        events: list[tuple[str, str | None, str]] = []
        logs: list[str] = []
        steering_result = run_result or SteeringPromptResult(0, "ok")

        async def handle_stale(
            interaction: steer_action.PersistentBusyInteraction,
            channel: steer_action.SteerChannel,
            prompt: str,
            target_thread_id: str | None,
            *,
            reason: str,
            deps: persistent_busy_steer.PersistentBusyStaleSteerBlockDeps,
        ) -> bool:
            _ = (interaction, channel, prompt, deps)
            events.append(("stale", target_thread_id, reason))
            return stale_blocked

        async def prepare(
            channel: steer_action.SteerChannel,
            target_thread_id: str | None,
            *,
            deps: persistent_busy_steer.PersistentBusySteerSessionMirrorDeps,
        ) -> bool:
            _ = (channel, deps)
            events.append(("prepare", target_thread_id, "-"))
            return session_delegate

        async def send_ack(channel: steer_action.SteerChannel, prompt: str, target_thread_id: str | None) -> None:
            _ = channel
            events.append(("ack", target_thread_id, prompt))

        async def run_prompt(
            channel: steer_action.SteerChannel,
            prompt: str,
            target_thread_id: str | None,
            *,
            choice_id: str,
            deps: persistent_busy_steer.PersistentBusySteerRunDeps,
        ) -> SteeringPromptResult:
            _ = (channel, prompt, deps)
            events.append(("run", target_thread_id, choice_id))
            if run_error is not None:
                raise run_error
            return steering_result

        def is_busy_error(exit_code: int, output: str) -> bool:
            _ = exit_code
            events.append(("busy_check", TARGET_THREAD_ID, output))
            return busy_error

        async def handle_busy_failure(
            interaction: steer_action.PersistentBusyInteraction,
            channel: steer_action.SteerChannel,
            prompt: str,
            target_thread_id: str | None,
            output: str,
            *,
            deps: persistent_busy_steer.PersistentBusySteerBusyFailureDeps,
        ) -> bool:
            _ = (interaction, channel, prompt, deps)
            events.append(("busy_failure", target_thread_id, output))
            return True

        async def handle_result(
            interaction: steer_action.PersistentBusyInteraction,
            channel: steer_action.SteerChannel,
            steering_result: SteeringPromptResult,
            target_thread_id: str | None,
            *,
            delegate_to_session_mirror: bool,
            deps: persistent_busy_steer.PersistentBusySteerResultDeps,
        ) -> bool:
            _ = (interaction, channel, steering_result, deps)
            events.append(("result", target_thread_id, f"delegate={delegate_to_session_mirror}"))
            return True

        def format_log_text_len(value: str | None) -> int:
            return len(value or "")

        return SteerHarness(
            deps=steer_action.PersistentBusySteerActionDeps(
                handle_stale_steer_block=handle_stale,
                stale_steer_block_deps=self._stale_deps(),
                prepare_session_mirror=prepare,
                session_mirror_deps=self._session_deps(),
                send_steering_start_ack=send_ack,
                run_steer_prompt=run_prompt,
                steer_run_deps=self._run_deps(),
                is_selected_thread_busy_error=is_busy_error,
                handle_busy_failure=handle_busy_failure,
                busy_failure_deps=self._busy_failure_deps(),
                handle_steer_result=handle_result,
                steer_result_deps=self._result_deps(),
                format_log_text_len=format_log_text_len,
                log=logs.append,
            ),
            events=events,
            logs=logs,
        )

    def _stale_deps(self) -> persistent_busy_steer.PersistentBusyStaleSteerBlockDeps:
        async def stale_sender(channel: object, prompt: str, target_thread_id: str | None, *, reason: str) -> bool:
            _ = (channel, prompt, target_thread_id, reason)
            return False

        async def followup(*args: object, **kwargs: object) -> None:
            _ = (args, kwargs)

        return persistent_busy_steer.PersistentBusyStaleSteerBlockDeps(stale_sender, followup)

    def _session_deps(self) -> persistent_busy_steer.PersistentBusySteerSessionMirrorDeps:
        async def prepare(channel: object, target_thread_id: str | None) -> bool:
            _ = (channel, target_thread_id)
            return False

        return persistent_busy_steer.PersistentBusySteerSessionMirrorDeps(prepare, prepare)

    def _run_deps(self) -> persistent_busy_steer.PersistentBusySteerRunDeps:
        def run(prompt: str, target_thread_id: str | None) -> SteeringPromptResult:
            _ = (prompt, target_thread_id)
            return SteeringPromptResult(0, "ok")

        def channel_typing(channel: object, *, context: str) -> AbstractAsyncContextManager[None]:
            _ = (channel, context)
            return NullTypingContext()

        def text_len(value: str | None) -> int:
            return len(value or "")

        return persistent_busy_steer.PersistentBusySteerRunDeps(run, channel_typing, lambda target: None, text_len, lambda: 1.0, lambda line: None)

    def _busy_failure_deps(self) -> persistent_busy_steer.PersistentBusySteerBusyFailureDeps:
        async def app_menu(channel: object, target_thread_id: str | None, output: str, *, reason: str) -> bool:
            _ = (channel, target_thread_id, output, reason)
            return False

        def resolve_target(target_thread_id: str | None) -> tuple[str | None, str]:
            return (target_thread_id, target_thread_id or "-")

        return persistent_busy_steer.PersistentBusySteerBusyFailureDeps(
            app_menu,
            self._stale_deps().send_stale_block_message,
            self._stale_deps().send_followup_chunks,
            resolve_target,
            lambda target_ref: target_ref,
            lambda line: None,
        )

    def _result_deps(self) -> persistent_busy_steer.PersistentBusySteerResultDeps:
        async def followup(*args: object, **kwargs: object) -> None:
            _ = (args, kwargs)

        async def streamer(*args: object, **kwargs: object) -> None:
            _ = (args, kwargs)

        return persistent_busy_steer.PersistentBusySteerResultDeps(followup, streamer, lambda line: None)
