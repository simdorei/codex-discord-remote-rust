from __future__ import annotations

import os
import tempfile
import unittest
from collections.abc import Coroutine
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol, TypeAlias, cast, final, override

import codex_discord_bot as bot


@final
class FakeView:
    pass


TargetMessage: TypeAlias = tuple[str, FakeView | None]


@final
class RecordingTarget:
    def __init__(self, channel_id: int = 456) -> None:
        self.id = channel_id
        self.messages: list[TargetMessage] = []

    async def send(self, content: str, view: FakeView | None = None) -> None:
        self.messages.append((content, view))


class MessageTarget(Protocol):
    async def send(self, content: str, view: FakeView | None = None) -> None:
        ...


@dataclass(frozen=True, slots=True)
class FakeCommand:
    name: str


@final
class FakeResponse:
    def __init__(self) -> None:
        self.messages: list[str] = []
        self.send_message_kwargs: list[dict[str, bool]] = []

    async def send_message(self, content: str, ephemeral: bool = False) -> None:
        self.messages.append(content)
        self.send_message_kwargs.append({"ephemeral": ephemeral})


@final
class FakeInteraction:
    def __init__(self, command_name: str = "ask", channel_id: int = 456) -> None:
        self.command = FakeCommand(command_name)
        self.channel_id = channel_id
        self.response = FakeResponse()


class SendChunksFunc(Protocol):
    def __call__(
        self,
        target: MessageTarget,
        text: str,
        *,
        context: str = "send_chunks",
        allow_during_stop: bool = False,
    ) -> Coroutine[None, None, int]:
        ...


class SendRestartNoticeFunc(Protocol):
    def __call__(self, target: MessageTarget) -> Coroutine[None, None, None]:
        ...


class SendMessageTrackedFunc(Protocol):
    def __call__(
        self,
        target: MessageTarget,
        content: str,
        *,
        view: FakeView | None = None,
        context: str = "send_message",
        allow_during_stop: bool = False,
    ) -> Coroutine[None, None, None]:
        ...


class SendInteractionResponseFunc(Protocol):
    def __call__(
        self,
        interaction: FakeInteraction,
        content: str,
        *,
        ephemeral: bool = False,
        context: str = "interaction_response",
        allow_during_stop: bool = False,
    ) -> Coroutine[None, None, None]:
        ...


async def send_chunks(target: MessageTarget, text: str, *, context: str) -> int:
    sender = cast(SendChunksFunc, bot.send_chunks)
    return await sender(target, text, context=context)


async def send_discord_restarting_notice(target: MessageTarget) -> None:
    sender = cast(SendRestartNoticeFunc, bot.send_discord_restarting_notice)
    await sender(target)


async def send_message_tracked(target: MessageTarget, content: str, *, context: str) -> None:
    sender = cast(SendMessageTrackedFunc, bot.send_message_tracked)
    await sender(target, content, context=context)


async def send_interaction_response_tracked(
    target_interaction: FakeInteraction,
    content: str,
    *,
    ephemeral: bool,
    context: str,
) -> None:
    sender = cast(SendInteractionResponseFunc, bot.send_interaction_response_tracked)
    await sender(target_interaction, content, ephemeral=ephemeral, context=context)


@final
class DiscordDeliveryStoppingIntegrationTests(unittest.IsolatedAsyncioTestCase):
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
        return Path(os.environ["CODEX_DISCORD_LOG_PATH"]).read_text(encoding="utf-8")

    async def test_send_chunks_rejects_new_delivery_while_stopping(self) -> None:
        target = RecordingTarget()

        bot.set_discord_delivery_stopping("unit")
        with self.assertRaisesRegex(
            bot.DiscordDeliveryRejected,
            "Discord bot is restarting",
        ):
            _ = await send_chunks(target, "blocked", context="unit_blocked")

        log_text = self._log_text()
        self.assertEqual(target.messages, [])
        self.assertIn("discord_delivery_rejected", log_text)
        self.assertIn("context=chunks:", log_text)

    async def test_restart_notice_is_allowed_while_stopping(self) -> None:
        target = RecordingTarget()

        bot.set_discord_delivery_stopping("unit")
        await send_discord_restarting_notice(target)

        log_text = self._log_text()
        self.assertEqual(len(target.messages), 1)
        self.assertIn("Discord bot is restarting", target.messages[0][0])
        self.assertIn("context=restart_notice", log_text)
        self.assertIn("discord_delivery_sent", log_text)

    async def test_send_message_tracked_rejects_new_delivery_while_stopping(self) -> None:
        target = RecordingTarget()

        bot.set_discord_delivery_stopping("unit")
        with self.assertRaisesRegex(
            bot.DiscordDeliveryRejected,
            "Discord bot is restarting",
        ):
            await send_message_tracked(target, "blocked", context="unit_message")

        log_text = self._log_text()
        self.assertEqual(target.messages, [])
        self.assertIn("discord_delivery_rejected", log_text)
        self.assertIn("context=message:", log_text)

    async def test_interaction_response_tracked_rejects_new_delivery_while_stopping(self) -> None:
        fake_interaction = FakeInteraction()

        bot.set_discord_delivery_stopping("unit")
        with self.assertRaisesRegex(
            bot.DiscordDeliveryRejected,
            "Discord bot is restarting",
        ):
            await send_interaction_response_tracked(
                fake_interaction,
                "blocked",
                ephemeral=True,
                context="unit_response",
            )

        log_text = self._log_text()
        self.assertEqual(fake_interaction.response.messages, [])
        self.assertIn("discord_delivery_rejected", log_text)
        self.assertIn("context=response:", log_text)


if __name__ == "__main__":
    _ = unittest.main()
