from __future__ import annotations

import os
import tempfile
import unittest
from collections.abc import Coroutine
from pathlib import Path
from types import SimpleNamespace
from typing import Protocol, cast, final, override
from unittest import mock

import codex_discord_bot as bot


@final
class FakeView:
    pass


@final
class FakeCommandTree:
    pass


@final
class FakeFollowup:
    def __init__(self) -> None:
        self.messages: list[str] = []
        self.kwargs: list[dict[str, bool]] = []

    async def send(self, content: str, **kwargs: bool) -> None:
        self.messages.append(content)
        self.kwargs.append(kwargs)


@final
class FakeResponse:
    def __init__(self) -> None:
        self.done = True

    def is_done(self) -> bool:
        return self.done


@final
class FakeInteraction:
    def __init__(self, command_name: str = "ask", channel_id: int = 456) -> None:
        self.command = SimpleNamespace(name=command_name)
        self.channel_id = channel_id
        self.followup = FakeFollowup()
        self.response = FakeResponse()
        self.user = SimpleNamespace(id=242286902982606848)


class MessageTarget(Protocol):
    async def send(self, content: str, view: FakeView | None = None) -> None:
        ...


@final
class RecordingChannel:
    def __init__(self, channel_id: int = 333) -> None:
        self.id = channel_id
        self.parent_id: int | None = None
        self.messages: list[tuple[str, FakeView | None]] = []

    async def send(self, content: str, view: FakeView | None = None) -> None:
        self.messages.append((content, view))


@final
class FakeMessage:
    def __init__(self, content: str = "please run", channel_id: int = 333) -> None:
        self.id: int | None = None
        self.channel = RecordingChannel(channel_id)
        self.author = SimpleNamespace(id=242286902982606848, bot=False)
        self.content = content
        self.raw_mentions: list[int] = []
        self.mentions: list[SimpleNamespace] = []
        self.attachments: list[str] = []
        self.embeds: list[str] = []
        self.stickers: list[str] = []


@final
class FakeClient:
    enable_prefix_commands = True

    def __init__(self, *, allowed_user_id: int | None = None) -> None:
        self._processed_message_ids: dict[int, float] = {}
        self.plain_ask_mention_user_ids: set[int] = set()
        self.user = SimpleNamespace(id=1511380398914142379)
        self._allowed_user_id = allowed_user_id

    def is_allowed_message_channel(self, channel: MessageTarget) -> bool:
        _ = channel
        return True

    def is_allowed_user(self, user_id: int | None) -> bool:
        if self._allowed_user_id is not None:
            return user_id == self._allowed_user_id
        return True


class SlashErrorReporter(Protocol):
    def __call__(
        self,
        tree: FakeCommandTree,
        interaction: FakeInteraction,
        error: BaseException,
    ) -> Coroutine[None, None, None]:
        ...


class MessageProcessor(Protocol):
    def __call__(
        self,
        client: FakeClient,
        message: FakeMessage,
        *,
        source: str,
    ) -> Coroutine[None, None, None]:
        ...


class MessageHandler(Protocol):
    def __call__(self, client: FakeClient, message: FakeMessage) -> Coroutine[None, None, None]:
        ...


@final
class DiscordRestartNoticeIntegrationTests(unittest.IsolatedAsyncioTestCase):
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

    async def test_slash_command_error_reports_restart_notice_while_stopping(self) -> None:
        interaction = FakeInteraction()
        reporter = cast(SlashErrorReporter, bot.LoggingCommandTree.on_error)

        bot.set_discord_delivery_stopping("unit")
        await reporter(
            FakeCommandTree(),
            interaction,
            bot.DiscordDeliveryRejected(bot.DISCORD_RESTARTING_ERROR),
        )

        self.assertEqual(len(interaction.followup.messages), 1)
        self.assertIn("Discord bot is restarting", interaction.followup.messages[0])
        self.assertEqual(interaction.followup.kwargs, [{"ephemeral": True}])
        self.assertIn("slash_command_error_sent command=ask response=followup", self._log_text())

    async def test_process_message_sends_restart_notice_while_stopping(self) -> None:
        message = FakeMessage()
        processor = cast(MessageProcessor, bot.CodexDiscordBot.process_discord_message)

        bot.set_discord_delivery_stopping("unit")
        await processor(FakeClient(), message, source="gateway")

        log_text = self._log_text()
        self.assertEqual(len(message.channel.messages), 1)
        self.assertIn("Discord bot is restarting", message.channel.messages[0][0])
        self.assertIn("message_rejected reason=bot_stopping", log_text)
        self.assertIn("context=restart_notice", log_text)

    async def test_on_message_routes_restart_check_handoff_to_explicit_codex_thread(self) -> None:
        calls: list[tuple[FakeMessage, str, str | None]] = []
        explicit_thread_id = "019ed8fb-2ead-7321-af85-6df41459df30"

        def no_mirrored_codex_thread(channel_id: int | None) -> None:
            _ = channel_id

        def fail_persist_inbound_mirror_thread_channel(target_thread_id: str, discord_thread_id: int) -> None:
            _ = (target_thread_id, discord_thread_id)
            raise AssertionError("explicit target must not rewrite mirror mapping")

        async def capture_handle_plain_ask(
            message: FakeMessage,
            prompt: str,
            *,
            target_thread_id: str | None = None,
        ) -> None:
            calls.append((message, prompt, target_thread_id))

        message = FakeMessage(
            content=(
                "<@1511380398914142379> <@242286902982606848>\n"
                "ACTION: RESTART-CHECK / HANDOFF\n\n"
                "Codex thread: `019ed8fb-2ead-7321-af85-6df41459df30`\n"
                "Post-restart checks:\n"
                "1. Run status.ps1\n"
                "Continuation rule: Continue refactoring until the user says to stop."
            ),
            channel_id=333,
        )
        message.author = SimpleNamespace(id=1500506752234422322, bot=True)
        message.raw_mentions = [1511380398914142379, 242286902982606848]
        message.mentions = [
            SimpleNamespace(id=1511380398914142379, bot=True),
            SimpleNamespace(id=242286902982606848, bot=False),
        ]
        on_message = cast(MessageHandler, bot.CodexDiscordBot.on_message)

        with (
            mock.patch.object(bot, "get_mirrored_codex_thread_id", side_effect=no_mirrored_codex_thread),
            mock.patch.object(bot, "handle_plain_ask", side_effect=capture_handle_plain_ask),
            mock.patch.object(
                bot,
                "persist_inbound_mirror_thread_channel",
                side_effect=fail_persist_inbound_mirror_thread_channel,
            ),
        ):
            await on_message(FakeClient(allowed_user_id=1500506752234422322), message)

        self.assertEqual(len(calls), 1)
        _, prompt, target_thread_id = calls[0]
        self.assertEqual(target_thread_id, explicit_thread_id)
        self.assertIn("ACTION: RESTART-CHECK / HANDOFF", prompt)
        self.assertIn(f"target_source=explicit target={explicit_thread_id}", self._log_text())

    async def test_on_message_accepts_bot_authored_restart_check_handoff_packet(self) -> None:
        calls: list[tuple[FakeMessage, str, str | None]] = []

        def mirrored_codex_thread(channel_id: int | None) -> str:
            _ = channel_id
            return "thread-1"

        async def capture_handle_plain_ask(
            message: FakeMessage,
            prompt: str,
            *,
            target_thread_id: str | None = None,
        ) -> None:
            calls.append((message, prompt, target_thread_id))

        message = FakeMessage(
            content=(
                "<@1511380398914142379> <@242286902982606848>\n"
                "ACTION: RESTART-CHECK / HANDOFF\n\n"
                "2-minute restart watch complete. Continue in the explicit route target only.\n\n"
                "Route target:\n"
                "- work_thread: 019eeaac-6170-7133-86ac-bef0f1c6e865\n"
                "- discord_thread: 1518267756510838855\n\n"
                "Post-restart checklist now required:\n"
                "1. verify startup notice still appears.\n"
                "2. continue refactoring until the user says to stop."
            ),
            channel_id=333,
        )
        message.author = SimpleNamespace(id=1500506752234422322, bot=True)
        message.raw_mentions = [1511380398914142379, 242286902982606848]
        message.mentions = [
            SimpleNamespace(id=1511380398914142379, bot=True),
            SimpleNamespace(id=242286902982606848, bot=False),
        ]
        on_message = cast(MessageHandler, bot.CodexDiscordBot.on_message)

        with (
            mock.patch.object(bot, "get_mirrored_codex_thread_id", side_effect=mirrored_codex_thread),
            mock.patch.object(bot, "handle_plain_ask", side_effect=capture_handle_plain_ask),
        ):
            await on_message(FakeClient(allowed_user_id=1500506752234422322), message)

        self.assertEqual(len(calls), 1)
        _, prompt, target_thread_id = calls[0]
        self.assertEqual(target_thread_id, "thread-1")
        self.assertIn("ACTION: RESTART-CHECK / HANDOFF", prompt)
        self.assertIn("Post-restart checklist", prompt)
        self.assertNotIn("ignored_message reason=bot_bridge_operational_packet", self._log_text())


if __name__ == "__main__":
    _ = unittest.main()
