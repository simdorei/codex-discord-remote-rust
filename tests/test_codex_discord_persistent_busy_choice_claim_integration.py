from __future__ import annotations

from collections.abc import Awaitable
from pathlib import Path
from types import SimpleNamespace
from typing import Never, Protocol, cast, override
import os
import tempfile
import unittest
from unittest import mock

import codex_discord_bot as bot
from codex_discord_components import parse_busy_choice_custom_id
from codex_discord_steering import SteeringPromptResult
from tests.test_codex_discord_busy_choice_view_integration import FakeMessage, MakeBusyChoiceView
from tests.test_codex_discord_persistent_busy_choice_queue_channel_integration import FakeTarget
from tests.test_codex_discord_persistent_choice_integration import FakeFollowup, FakeInteractionMessage, FakeResponse


class FakeBusyInteraction:
    def __init__(self, *, channel: FakeTarget | None = None, user_id: int = 242286902982606848) -> None:
        self.command: SimpleNamespace = SimpleNamespace(name="-")
        self.channel_id: int = 222
        self.followup: FakeFollowup = FakeFollowup()
        self.response: FakeResponse = FakeResponse()
        self.user: SimpleNamespace = SimpleNamespace(id=user_id)
        self.channel: FakeTarget | None = channel
        self.message: FakeInteractionMessage = FakeInteractionMessage()
        self.data: dict[str, str] = {}


class SteeringRunner(Protocol):
    def __call__(self, prompt: str, target_thread_id: str | None) -> SteeringPromptResult: ...


class SteeringStreamer(Protocol):
    def __call__(
        self,
        channel: FakeTarget,
        steering_result: SteeringPromptResult,
        target_thread_id: str | None,
        **kwargs: bool | None,
    ) -> Awaitable[bool]: ...


class BusyChoiceHandler(Protocol):
    def __call__(
        self,
        interaction: FakeBusyInteraction,
        custom_id: str,
        *,
        steering_runner: SteeringRunner | None = None,
        steering_streamer: SteeringStreamer | None = None,
    ) -> Awaitable[bool]: ...


class ReportUnhandled(Protocol):
    def __call__(
        self,
        interaction: FakeBusyInteraction,
        *,
        delay_sec: float = 0.75,
    ) -> Awaitable[None]: ...


def _busy_choice_handler() -> BusyChoiceHandler:
    return cast(BusyChoiceHandler, bot.handle_persistent_busy_choice_interaction)


def _report_unhandled() -> ReportUnhandled:
    return cast(ReportUnhandled, bot.report_unhandled_component_interaction)


def _make_busy_choice_view() -> MakeBusyChoiceView:
    return cast(MakeBusyChoiceView, bot.make_busy_choice_view)


def _custom_ids(prompt: str, *, allow_steer: bool) -> dict[str, str]:
    view = _make_busy_choice_view()(
        FakeMessage(),
        prompt,
        target_thread_id="thread-1",
        allow_steer=allow_steer,
    )
    return {
        getattr(item, "label", ""): getattr(item, "custom_id", "")
        for item in view.children
    }


def _choice_id(custom_id: str) -> str:
    parsed = parse_busy_choice_custom_id(custom_id)
    if parsed is None:
        raise AssertionError(f"invalid busy choice custom id: {custom_id}")
    choice_id, _action = parsed
    return choice_id


class DiscordPersistentBusyChoiceClaimIntegrationTests(unittest.IsolatedAsyncioTestCase):
    _old_mirror_db_path: Path | None = None
    _old_discord_log_path: str | None = None
    _temp_dir: tempfile.TemporaryDirectory[str] | None = None

    @override
    def setUp(self) -> None:
        original_app_server_transport_enabled = bot.app_server_transport_enabled

        def restore_app_server_transport_enabled() -> None:
            bot.app_server_transport_enabled = original_app_server_transport_enabled

        self.addCleanup(restore_app_server_transport_enabled)
        bot.app_server_transport_enabled = lambda: False
        self._old_mirror_db_path = bot.MIRROR_DB_PATH
        self._old_discord_log_path = os.environ.get("CODEX_DISCORD_LOG_PATH")
        temp_dir = tempfile.TemporaryDirectory(ignore_cleanup_errors=True)
        self._temp_dir = temp_dir
        temp_path = Path(temp_dir.name)
        bot.MIRROR_DB_PATH = temp_path / "mirror.sqlite"
        os.environ["CODEX_DISCORD_LOG_PATH"] = str(temp_path / "discord-smoke.log")
        bot.init_mirror_db()
        bot.ACTIVE_DISCORD_DELIVERIES.clear()
        bot.clear_discord_delivery_stopping()

    @override
    def tearDown(self) -> None:
        if self._old_mirror_db_path is not None:
            bot.MIRROR_DB_PATH = self._old_mirror_db_path
        if self._old_discord_log_path is None:
            _ = os.environ.pop("CODEX_DISCORD_LOG_PATH", None)
        else:
            os.environ["CODEX_DISCORD_LOG_PATH"] = self._old_discord_log_path
        bot.ACTIVE_DISCORD_DELIVERIES.clear()
        bot.clear_discord_delivery_stopping()
        if self._temp_dir is not None:
            self._temp_dir.cleanup()
            self._temp_dir = None

    def _log_text(self) -> str:
        return Path(os.environ["CODEX_DISCORD_LOG_PATH"]).read_text(encoding="utf-8")

    async def test_persistent_busy_choice_denied_does_not_claim_record(self) -> None:
        queue_id = _custom_ids("please queue", allow_steer=True)["Queue next"]
        choice_id = _choice_id(queue_id)
        interaction = FakeBusyInteraction(user_id=999)
        interaction.data = {"custom_id": queue_id}

        await _report_unhandled()(interaction, delay_sec=0)

        remaining = bot.get_busy_choice_record(choice_id)
        log_text = self._log_text()
        self.assertEqual(interaction.response.messages, ["Only the original sender can choose this."])
        self.assertIsNotNone(remaining)
        self.assertIn("busy_choice_persistent_denied", log_text)
        self.assertNotIn("please queue", log_text)

    async def test_persistent_steer_not_allowed_does_not_claim_queue_choice(self) -> None:
        enqueue_calls: list[tuple[str, str | None, bool]] = []

        async def fake_enqueue(
            channel: FakeTarget, prompt: str, target_thread_id: str | None, *, queued: bool = False, **_kwargs: Never
        ) -> int:
            _ = channel
            enqueue_calls.append((prompt, target_thread_id, queued))
            return 1

        def fake_busy_state(target_thread_id: str) -> tuple[str, str, str]:
            return ("busy", target_thread_id, "project:1")

        custom_ids = _custom_ids("please queue", allow_steer=False)
        choice_id = _choice_id(custom_ids["Queue next"])
        channel = FakeTarget(channel_id=222)
        steer_interaction = FakeBusyInteraction(channel=channel)
        steer_interaction.data = {"custom_id": custom_ids["Steer now"]}
        queue_interaction = FakeBusyInteraction(channel=channel)
        queue_interaction.data = {"custom_id": custom_ids["Queue next"]}

        with (
            mock.patch.object(bot, "get_busy_state_for_thread", fake_busy_state),
            mock.patch.object(bot, "enqueue_thread_ask", fake_enqueue),
        ):
            steer_handled = await _busy_choice_handler()(steer_interaction, custom_ids["Steer now"])
            remaining_after_steer = bot.get_busy_choice_record(choice_id)
            queue_handled = await _busy_choice_handler()(queue_interaction, custom_ids["Queue next"])
            remaining_after_queue = bot.get_busy_choice_record(choice_id)

        log_text = self._log_text()
        self.assertTrue(steer_handled)
        self.assertEqual(steer_interaction.response.messages, ["This message targets a different Codex thread. Queue it instead."])
        self.assertIsNotNone(remaining_after_steer)
        self.assertTrue(queue_handled)
        self.assertEqual(queue_interaction.followup.messages, ["Queued at position 1."])
        self.assertEqual(enqueue_calls, [("please queue", "thread-1", True)])
        self.assertIsNone(remaining_after_queue)
        self.assertIn("busy_choice_persistent_steer_rejected", log_text)
        self.assertIn("busy_choice_persistent_queue", log_text)

    async def test_persistent_steer_duplicate_interaction_runs_once(self) -> None:
        steer_id = _custom_ids("please steer", allow_steer=True)["Steer now"]
        calls: list[tuple[str, str | None]] = []
        streamed: list[tuple[SteeringPromptResult, str | None, dict[str, bool | None]]] = []

        def fake_steer(prompt: str, target_thread_id: str | None) -> SteeringPromptResult:
            calls.append((prompt, target_thread_id))
            return SteeringPromptResult(
                0,
                "[qa_delivery_verified]",
                target_thread_id=target_thread_id,
                target_ref=target_thread_id or "-",
                session_path="qa-session.jsonl",
                start_offset=0,
            )

        async def fake_stream(channel: FakeTarget, steering_result: SteeringPromptResult, target_thread_id: str | None, **kwargs: bool | None) -> bool:
            _ = channel
            streamed.append((steering_result, target_thread_id, kwargs))
            return True

        channel = FakeTarget(channel_id=222)
        first = FakeBusyInteraction(channel=channel)
        first.data = {"custom_id": steer_id}
        second = FakeBusyInteraction(channel=channel)
        second.data = {"custom_id": steer_id}

        first_handled = await _busy_choice_handler()(first, steer_id, steering_runner=fake_steer, steering_streamer=fake_stream)
        second_handled = await _busy_choice_handler()(second, steer_id, steering_runner=fake_steer, steering_streamer=fake_stream)

        log_text = self._log_text()
        self.assertTrue(first_handled)
        self.assertTrue(second_handled)
        self.assertEqual(calls, [("please steer", "thread-1")])
        self.assertEqual(len(streamed), 1)
        self.assertEqual(streamed[0][2], {"send_commentary_blocks": None, "send_final_blocks": True})
        self.assertEqual(first.response.defer_kwargs, [{"thinking": True, "ephemeral": True}])
        self.assertTrue(first.followup.messages[0].startswith("Steering sent"))
        self.assertEqual(channel.messages, [("Discord steering submitted.\nmessage: please steer", None)])
        self.assertEqual(second.response.messages, ["This Discord button is no longer active. Send the message again to get fresh controls."])
        self.assertIn("steering_start_ack_sent target=thread-1", log_text)
        self.assertIn("busy_choice_persistent_steer_done exit=0", log_text)
        self.assertIn("busy_choice_persistent_missing action=steer", log_text)


if __name__ == "__main__":
    _ = unittest.main()
