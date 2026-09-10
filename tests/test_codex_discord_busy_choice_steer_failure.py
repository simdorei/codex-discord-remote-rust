from __future__ import annotations

import unittest
from dataclasses import dataclass

import codex_discord_busy_choice_steer_failure as steer_failure
from codex_discord_persistent_busy_steer import (
    STEER_APP_MENU_REFRESHED_FOLLOWUP_MESSAGE,
    STEER_STALE_BLOCK_FOLLOWUP_MESSAGE,
)

PROMPT = "please steer"
TARGET_THREAD_ID = "thread-1"


@dataclass(frozen=True, slots=True)
class FakeInteraction:
    marker: str = "interaction"


@dataclass(frozen=True, slots=True)
class FakeChannel:
    channel_id: int = 222


@dataclass(frozen=True, slots=True)
class FailureHarness:
    deps: steer_failure.BusyChoiceSteerFailureDeps
    events: list[tuple[str, str | None, str]]
    followups: list[tuple[str, str, int, str, bool]]
    logs: list[str]


class BusyChoiceSteerFailureTests(unittest.IsolatedAsyncioTestCase):
    async def test_app_menu_refresh_sends_refresh_followup_and_stops(self) -> None:
        harness = self._make_harness(app_menu_sent=True)

        handled = await self._run_failure(harness)

        self.assertTrue(handled)
        self.assertEqual(harness.events, [("app_menu", TARGET_THREAD_ID, "steer_busy_failure")])
        self.assertEqual(harness.followups, [(STEER_APP_MENU_REFRESHED_FOLLOWUP_MESSAGE, "Steering", 0, "button_response", True)])
        self.assertEqual(harness.logs, [])

    async def test_stale_block_sends_stale_followup_after_app_menu_miss(self) -> None:
        harness = self._make_harness(stale_blocked=True)

        handled = await self._run_failure(harness)

        self.assertTrue(handled)
        self.assertEqual(
            harness.events,
            [
                ("app_menu", TARGET_THREAD_ID, "steer_busy_failure"),
                ("stale", TARGET_THREAD_ID, "steer_busy_failure"),
            ],
        )
        self.assertEqual(harness.followups, [(STEER_STALE_BLOCK_FOLLOWUP_MESSAGE, "Steering", 0, "button_response", True)])
        self.assertEqual(harness.logs, [])

    async def test_not_accepted_followup_logs_exit_and_target(self) -> None:
        harness = self._make_harness()

        handled = await self._run_failure(harness)

        self.assertTrue(handled)
        self.assertEqual(harness.followups, [("not accepted: thread-1", "Steering", 0, "button_response", True)])
        self.assertEqual(harness.logs, ["steer_busy_status_sent reason=steer_busy_failure exit=7 target=thread-1"])

    async def test_none_target_uses_dash_ref_and_log_target(self) -> None:
        harness = self._make_harness()

        handled = await self._run_failure(harness, target_thread_id=None)

        self.assertTrue(handled)
        self.assertIn(("resolve", None, "-"), harness.events)
        self.assertEqual(harness.followups, [("not accepted: -", "Steering", 0, "button_response", True)])
        self.assertEqual(harness.logs, ["steer_busy_status_sent reason=steer_busy_failure exit=7 target=-"])

    async def test_app_menu_exception_propagates_without_fallback(self) -> None:
        harness = self._make_harness(app_menu_error=RuntimeError("menu failed"))

        with self.assertRaisesRegex(RuntimeError, "menu failed"):
            _ = await self._run_failure(harness)

        self.assertEqual(harness.events, [("app_menu", TARGET_THREAD_ID, "steer_busy_failure")])
        self.assertEqual(harness.followups, [])
        self.assertEqual(harness.logs, [])

    async def _run_failure(
        self,
        harness: FailureHarness,
        *,
        target_thread_id: str | None = TARGET_THREAD_ID,
    ) -> bool:
        return await steer_failure.handle_busy_choice_steer_busy_failure(
            FakeInteraction(),
            FakeChannel(),
            PROMPT,
            target_thread_id,
            exit_code=7,
            output="busy now",
            deps=harness.deps,
        )

    def _make_harness(
        self,
        *,
        app_menu_sent: bool = False,
        stale_blocked: bool = False,
        app_menu_error: RuntimeError | None = None,
    ) -> FailureHarness:
        events: list[tuple[str, str | None, str]] = []
        followups: list[tuple[str, str, int, str, bool]] = []
        logs: list[str] = []

        async def app_menu(
            channel: steer_failure.BusyChoiceChannel,
            target_thread_id: str | None,
            output: str,
            *,
            reason: str,
        ) -> bool:
            _ = (channel, output)
            events.append(("app_menu", target_thread_id, reason))
            if app_menu_error is not None:
                raise app_menu_error
            return app_menu_sent

        async def stale(
            channel: steer_failure.BusyChoiceChannel,
            prompt: str,
            target_thread_id: str | None,
            *,
            reason: str,
        ) -> bool:
            _ = (channel, prompt)
            events.append(("stale", target_thread_id, reason))
            return stale_blocked

        async def followup(
            interaction: steer_failure.BusyChoiceInteraction,
            content: str,
            *,
            title: str,
            exit_code: int,
            log_prefix: str,
            ephemeral: bool,
        ) -> None:
            _ = interaction
            followups.append((content, title, exit_code, log_prefix, ephemeral))

        def resolve_target(target_thread_id: str | None) -> tuple[str | None, str]:
            target_ref = target_thread_id or "-"
            events.append(("resolve", target_thread_id, target_ref))
            return (target_thread_id, target_ref)

        deps = steer_failure.BusyChoiceSteerFailureDeps(
            send_codex_app_menu_if_available=app_menu,
            send_stale_block_message=stale,
            send_followup_chunks=followup,
            resolve_target_ref=resolve_target,
            build_not_accepted_message=lambda target_ref: f"not accepted: {target_ref}",
            log=logs.append,
        )
        return FailureHarness(deps=deps, events=events, followups=followups, logs=logs)
