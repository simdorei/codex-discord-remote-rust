from __future__ import annotations

from collections.abc import Awaitable, Callable
from pathlib import Path
from types import SimpleNamespace, TracebackType
from typing import TypeAlias, cast
import os
import tempfile
import unittest
from unittest import mock

import codex_discord_delivery as discord_delivery
import codex_discord_bot as bot


class FakeFollowup:
    def __init__(self) -> None:
        self.messages: list[str] = []
        self.kwargs: list[dict[str, bool]] = []

    async def send(self, content: str, **kwargs: bool) -> None:
        self.messages.append(content)
        self.kwargs.append(kwargs)


class FakeInteraction:
    def __init__(self, command_name: str = "ask", channel_id: int = 222) -> None:
        self.command: SimpleNamespace = SimpleNamespace(name=command_name)
        self.channel_id: int = channel_id
        self.followup: FakeFollowup = FakeFollowup()
        self.user: SimpleNamespace = SimpleNamespace(id=242286902982606848)
        self.channel: FakeTarget | None = None


class FakeTarget:
    def __init__(self, channel_id: int = 222) -> None:
        self.messages: list[tuple[str, None]] = []
        self.id: int = channel_id

    async def send(self, content: str, view: None = None) -> None:
        _ = view
        self.messages.append((content, None))


class EnvPatch:
    def __init__(self, key: str, value: str) -> None:
        self.key: str = key
        self.value: str = value
        self.original: str | None = None

    def __enter__(self) -> None:
        self.original = os.environ.get(self.key)
        os.environ[self.key] = self.value

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        tb: TracebackType | None,
    ) -> None:
        if self.original is None:
            _ = os.environ.pop(self.key, None)
        else:
            os.environ[self.key] = self.original


SlashAskHandler: TypeAlias = Callable[[FakeInteraction, str], Awaitable[None]]


def _slash_ask_handler() -> SlashAskHandler:
    return cast(SlashAskHandler, bot.handle_slash_ask)


class DiscordSlashAskIntegrationTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self) -> None:
        original_app_server_transport_enabled = bot.app_server_transport_enabled

        def restore_app_server_transport_enabled() -> None:
            bot.app_server_transport_enabled = original_app_server_transport_enabled

        self.addCleanup(restore_app_server_transport_enabled)
        bot.app_server_transport_enabled = lambda: False

    async def test_slash_ask_forwards_source_message_to_plain_ask(self) -> None:
        interaction = FakeInteraction(command_name="ask", channel_id=222)
        interaction.channel = FakeTarget(channel_id=222)
        asks: list[tuple[bot.SlashAskSourceMessage, str, str | None]] = []
        followups: list[tuple[str, bool, str, str]] = []

        async def send_direct_followup(
            sent_interaction: FakeInteraction,
            text: str,
            *,
            ephemeral: bool,
            log_prefix: str,
            context: str,
        ) -> None:
            self.assertIs(sent_interaction, interaction)
            followups.append((text, ephemeral, log_prefix, context))

        async def handle_plain_ask(
            source_message: bot.SlashAskSourceMessage,
            prompt: str,
            *,
            target_thread_id: str | None = None,
        ) -> None:
            asks.append((source_message, prompt, target_thread_id))

        with (
            mock.patch.object(bot, "send_direct_followup", send_direct_followup),
            mock.patch.object(bot, "handle_plain_ask", handle_plain_ask),
            mock.patch.object(bot, "get_mirrored_codex_thread_id", return_value="thread-1"),
            mock.patch.object(bot, "describe_mirrored_project_channel", return_value=""),
            mock.patch.object(discord_delivery, "get_interaction_command_name", return_value="ask"),
        ):
            await _slash_ask_handler()(interaction, "hello")

        self.assertEqual(
            followups,
            [("Ask handling posted in this channel.", True, "slash_ack", "ask_posted")],
        )
        self.assertEqual(len(asks), 1)
        source_message, prompt, target_thread_id = asks[0]
        self.assertIs(cast(object, source_message.channel), interaction.channel)
        self.assertIs(source_message.author, interaction.user)
        self.assertEqual(prompt, "hello")
        self.assertEqual(target_thread_id, "thread-1")

    async def test_slash_ask_routes_to_existing_ask_flow(self) -> None:
        calls: list[tuple[bot.SlashAskSourceMessage, str, str | None]] = []

        async def fake_handle_plain_ask(
            message: bot.SlashAskSourceMessage,
            prompt: str,
            *,
            target_thread_id: str | None = None,
        ) -> None:
            calls.append((message, prompt, target_thread_id))

        interaction = FakeInteraction(command_name="ask", channel_id=222)
        interaction.channel = FakeTarget()

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with (
                EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)),
                mock.patch.object(bot, "get_mirrored_codex_thread_id", return_value="thread-1"),
                mock.patch.object(bot, "handle_plain_ask", fake_handle_plain_ask),
            ):
                await _slash_ask_handler()(interaction, "please run")
            log_text = log_path.read_text(encoding="utf-8")

        self.assertEqual(interaction.followup.messages, ["Ask handling posted in this channel."])
        self.assertEqual(interaction.followup.kwargs, [{"ephemeral": True}])
        self.assertEqual(len(calls), 1)
        source_message, prompt, target_thread_id = calls[0]
        self.assertEqual(prompt, "please run")
        self.assertEqual(target_thread_id, "thread-1")
        self.assertIs(cast(object, source_message.channel), interaction.channel)
        self.assertIs(source_message.author, interaction.user)
        self.assertIn("slash_ask_dispatch command=ask channel=222", log_text)
        self.assertIn("target_source=mirror target=thread-1", log_text)
        self.assertIn("prompt_len=10", log_text)
        self.assertIn("slash_ask_ack_sent command=ask channel=222", log_text)

    async def test_slash_ask_blocks_project_parent_fallback(self) -> None:
        async def fail_handle_plain_ask(
            message: bot.SlashAskSourceMessage,
            prompt: str,
            *,
            target_thread_id: str | None = None,
        ) -> None:
            _ = (message, prompt, target_thread_id)
            raise AssertionError("project parent slash ask must not fall back to selected thread")

        interaction = FakeInteraction(command_name="ask", channel_id=333)
        interaction.channel = FakeTarget()

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with (
                EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)),
                mock.patch.object(bot, "get_mirrored_codex_thread_id", return_value=None),
                mock.patch.object(
                    bot,
                    "describe_mirrored_project_channel",
                    return_value="`taxlab` project channel has multiple Codex threads.",
                ),
                mock.patch.object(bot, "handle_plain_ask", fail_handle_plain_ask),
            ):
                await _slash_ask_handler()(interaction, "please run")
            log_text = log_path.read_text(encoding="utf-8")

        self.assertEqual(
            interaction.followup.messages,
            ["`taxlab` project channel has multiple Codex threads."],
        )
        self.assertIn("slash_ask_blocked command=ask channel=333", log_text)
        self.assertIn("reason=project_parent", log_text)
        self.assertNotIn("slash_ask_dispatch", log_text)

    async def test_slash_ask_delegates_without_busy_preflight(self) -> None:
        calls: list[tuple[FakeTarget, str, str | None, bot.SlashAskSourceMessage | None]] = []

        async def runner_idle(target_thread_id: str | None) -> bool:
            _ = target_thread_id
            return False

        async def fake_run_prompt_flow(
            channel: FakeTarget,
            prompt: str,
            *,
            source_message: bot.SlashAskSourceMessage | None = None,
            target_thread_id: str | None = None,
        ) -> None:
            calls.append((channel, prompt, target_thread_id, source_message))

        interaction = FakeInteraction(command_name="ask", channel_id=222)
        interaction.channel = FakeTarget()

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with (
                EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)),
                mock.patch.object(bot, "get_mirrored_codex_thread_id", return_value="thread-1"),
                mock.patch.object(bot, "get_interactive_state_for_thread", return_value=("", None, "")),
                mock.patch.object(bot, "get_busy_state_for_thread", return_value=("busy", "thread-1", "taxlab:1")),
                mock.patch.object(bot, "build_context_warning", return_value=""),
                mock.patch.object(bot, "is_thread_runner_busy", runner_idle),
                mock.patch.object(bot, "run_prompt_flow", fake_run_prompt_flow),
            ):
                await _slash_ask_handler()(interaction, "please steer")
            log_text = log_path.read_text(encoding="utf-8")

        self.assertEqual(interaction.followup.messages, ["Ask handling posted in this channel."])
        self.assertEqual(interaction.channel.messages, [])
        self.assertEqual(len(calls), 1)
        channel, prompt, target_thread_id, source_message = calls[0]
        self.assertIs(channel, interaction.channel)
        self.assertEqual(prompt, "please steer")
        self.assertEqual(target_thread_id, "thread-1")
        if source_message is None:
            self.fail("source_message was not forwarded")
        self.assertIs(cast(object, source_message.channel), interaction.channel)
        self.assertIn("slash_ask_dispatch command=ask channel=222", log_text)
        self.assertNotIn("target_busy", log_text)


if __name__ == "__main__":
    _ = unittest.main()
