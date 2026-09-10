from __future__ import annotations

from collections.abc import Awaitable
from pathlib import Path
from types import SimpleNamespace
from typing import Protocol, TypeAlias, cast, override
import os
import tempfile
import unittest
from unittest import mock

import codex_discord_bot as bot


class InteractionAlreadyAcknowledgedError(RuntimeError):
    pass


class MessageEditUnavailableError(RuntimeError):
    pass


class BadEditSignatureError(TypeError):
    pass


class BadPersistentHandlerError(TypeError):
    pass


class BadResponseSenderError(TypeError):
    pass


class FakeResponse:
    def __init__(self, *, already_acknowledged: bool = False) -> None:
        self.messages: list[str] = []
        self.send_message_kwargs: list[dict[str, bool]] = []
        self.done: bool = False
        self.already_acknowledged: bool = already_acknowledged

    async def send_message(self, content: str, ephemeral: bool = False) -> None:
        if self.already_acknowledged:
            raise InteractionAlreadyAcknowledgedError("Interaction has already been acknowledged.")
        self.messages.append(content)
        self.send_message_kwargs.append({"ephemeral": ephemeral})
        self.done = True

    def is_done(self) -> bool:
        return self.done


class FakeInteractionMessage:
    def __init__(self) -> None:
        self.edits: list[None] = []

    async def edit(self, view: None = None) -> None:
        self.edits.append(view)


class RuntimeFailingMessage:
    async def edit(self, view: None = None) -> None:
        _ = view
        raise MessageEditUnavailableError("edit unavailable")


class TypeErrorMessage:
    async def edit(self, view: None = None) -> None:
        _ = view
        raise BadEditSignatureError("bad edit signature")


class FakeInteraction:
    def __init__(self, command_name: str = "-", channel_id: int = 222) -> None:
        self.command: SimpleNamespace = SimpleNamespace(name=command_name)
        self.channel_id: int = channel_id
        self.response: FakeResponse = FakeResponse()
        self.user: SimpleNamespace = SimpleNamespace(id=242286902982606848)
        self.message: FakeInteractionMessage | RuntimeFailingMessage | TypeErrorMessage = (
            FakeInteractionMessage()
        )
        self.data: dict[str, str] = {"custom_id": "codex-busy-choice-old-button"}


class ReportUnhandled(Protocol):
    def __call__(
        self,
        interaction: FakeInteraction,
        *,
        delay_sec: float = 0.75,
    ) -> Awaitable[None]: ...


class ClearComponents(Protocol):
    def __call__(
        self,
        interaction: FakeInteraction,
        *,
        context: str,
    ) -> Awaitable[None]: ...


ReportResponse: TypeAlias = list[str]


def _report_unhandled() -> ReportUnhandled:
    return cast(ReportUnhandled, bot.report_unhandled_component_interaction)


def _clear_components() -> ClearComponents:
    return cast(ClearComponents, bot.clear_interaction_message_components)


class DiscordUnhandledComponentIntegrationTests(unittest.IsolatedAsyncioTestCase):
    _old_discord_log_path: str | None = None
    _temp_dir: tempfile.TemporaryDirectory[str] | None = None

    @override
    def setUp(self) -> None:
        self._old_discord_log_path = os.environ.get("CODEX_DISCORD_LOG_PATH")
        temp_dir = tempfile.TemporaryDirectory(ignore_cleanup_errors=True)
        self._temp_dir = temp_dir
        os.environ["CODEX_DISCORD_LOG_PATH"] = str(Path(temp_dir.name) / "discord-smoke.log")
        bot.ACTIVE_DISCORD_DELIVERIES.clear()
        bot.clear_discord_delivery_stopping()

    @override
    def tearDown(self) -> None:
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
        log_path = Path(os.environ["CODEX_DISCORD_LOG_PATH"])
        return log_path.read_text(encoding="utf-8") if log_path.exists() else ""

    async def test_unhandled_component_interaction_gets_stale_button_notice(self) -> None:
        interaction = FakeInteraction()
        message = FakeInteractionMessage()
        interaction.message = message

        await _report_unhandled()(interaction, delay_sec=0)

        self.assertEqual(
            interaction.response.messages,
            ["This Discord button is no longer active. Send the message again to get fresh controls."],
        )
        self.assertEqual(message.edits, [None])
        log_text = self._log_text()
        self.assertIn("component_interaction_unhandled_reported", log_text)
        self.assertIn("component_message_components_cleared context=unhandled_component", log_text)
        self.assertIn("custom_id=codex-busy-choice-old-button", log_text)

    async def test_clear_interaction_message_components_runtime_failure_logs(self) -> None:
        interaction = FakeInteraction()
        interaction.message = RuntimeFailingMessage()

        await _clear_components()(interaction, context="runtime-boundary")

        log_text = self._log_text()
        self.assertIn("component_message_components_clear_failed context=runtime-boundary", log_text)
        self.assertIn("MessageEditUnavailableError: edit unavailable", log_text)

    async def test_clear_interaction_message_components_type_error_is_not_component_failure(self) -> None:
        interaction = FakeInteraction()
        interaction.message = TypeErrorMessage()

        with self.assertRaisesRegex(TypeError, "bad edit signature"):
            await _clear_components()(interaction, context="type-boundary")

        self.assertNotIn("component_message_components_clear_failed", self._log_text())

    async def test_report_unhandled_component_handler_type_error_is_not_persistent_failure(self) -> None:
        async def fail_handler(interaction: FakeInteraction, custom_id: str) -> bool:
            _ = (interaction, custom_id)
            raise BadPersistentHandlerError("bad persistent handler")

        interaction = FakeInteraction()

        with mock.patch.object(bot, "handle_persistent_approval_interaction", fail_handler):
            with self.assertRaisesRegex(TypeError, "bad persistent handler"):
                await _report_unhandled()(interaction, delay_sec=0)

        self.assertNotIn("component_interaction_persistent_handler_failed", self._log_text())

    async def test_report_unhandled_component_response_type_error_is_not_report_failure(self) -> None:
        async def fail_response(
            interaction: FakeInteraction,
            content: str,
            *,
            ephemeral: bool = False,
            context: str = "interaction_response",
        ) -> None:
            _ = (interaction, content, ephemeral, context)
            raise BadResponseSenderError("bad response sender")

        interaction = FakeInteraction()

        with mock.patch.object(bot, "send_interaction_response_tracked", fail_response):
            with self.assertRaisesRegex(TypeError, "bad response sender"):
                await _report_unhandled()(interaction, delay_sec=0)

        log_text = self._log_text()
        self.assertIn("component_message_components_cleared context=unhandled_component", log_text)
        self.assertNotIn("component_interaction_unhandled_report_failed", log_text)

    async def test_unhandled_component_already_acknowledged_response_logs_concise_marker(self) -> None:
        interaction = FakeInteraction()
        interaction.response = FakeResponse(already_acknowledged=True)
        message = FakeInteractionMessage()
        interaction.message = message

        await _report_unhandled()(interaction, delay_sec=0)

        self.assertEqual(message.edits, [None])
        log_text = self._log_text()
        self.assertIn("component_interaction_unhandled_report_already_acknowledged", log_text)
        self.assertIn("custom_id=codex-busy-choice-old-button", log_text)
        self.assertNotIn("Traceback", log_text)


if __name__ == "__main__":
    _ = unittest.main()
